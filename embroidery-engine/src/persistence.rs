//! Bounded source artwork and crash-safe project/recovery persistence.
use crate::model::Project;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub const MAX_PROJECT_BYTES: usize = 32_000_000;
pub const MAX_SOURCE_BYTES: usize = 12_000_000;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceObjectIdentity {
    pub object_id: u64,
    pub region_identity: u64,
    pub original_fingerprint: u64,
}

/// Schema v5: source-coordinate masks, physical scale, palette and exact import
/// identities. Kept separate from editable objects and the generated stitch plan.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RasterRecipe {
    pub options: crate::import::RasterOptions,
    pub width_mm: f64,
    pub palette: Vec<[u8; 3]>,
    pub objects: Vec<SourceObjectIdentity>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub issues: Vec<crate::model::PlanWarning>,
}

pub fn object_fingerprint(object: &crate::model::Object) -> u64 {
    // Stable across save/open, independent of randomized hash-map seeds.
    serde_json::to_vec(object)
        .unwrap_or_default()
        .into_iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        })
}

impl RasterRecipe {
    pub fn validate(&self, source: Option<&SourceImage>) -> Result<(), String> {
        self.options.validate()?;
        if !self.width_mm.is_finite() || self.width_mm <= 0. || self.width_mm > 9990. {
            return Err("Raster recipe requires a finite physical width".into());
        }
        if self.objects.len() > 10_000
            || self.palette.len() > 32
            || self.warnings.len() > 10_000
            || self.issues.len() > 10_000
            || self.warnings.iter().any(|s| s.len() > 4096)
            || self.issues.iter().any(|issue| {
                issue.message.len() > 4096
                    || issue.point.is_some_and(|p| {
                        !p.x.is_finite()
                            || !p.y.is_finite()
                            || p.x.abs() > 9990.
                            || p.y.abs() > 9990.
                    })
            })
        {
            return Err("Raster recipe exceeds project limits".into());
        }
        if let Some(cleanup) = &self.options.cleanup {
            cleanup.validate()?;
            let image = source
                .ok_or("Cleanup recipe requires embedded source artwork")?
                .decode()?;
            if [cleanup.width, cleanup.height] != [image.width(), image.height()] {
                return Err("Cleanup mask dimensions do not match embedded source artwork".into());
            }
        }
        Ok(())
    }

    pub fn capture(project: &crate::model::Project, options: crate::import::RasterOptions) -> Self {
        let mut palette: Vec<_> = project.objects.iter().map(|o| o.color).collect();
        palette.sort();
        palette.dedup();
        Self {
            options,
            width_mm: project.width_mm,
            palette,
            objects: project
                .objects
                .iter()
                .map(|o| SourceObjectIdentity {
                    object_id: o.id,
                    region_identity: crate::region_edit::region_identity(o),
                    original_fingerprint: object_fingerprint(o),
                })
                .collect(),
            warnings: vec![],
            issues: vec![],
        }
    }

    /// Only transfer an override when its original region identity is proven.
    /// Changed topology with a manual edit is a conflict, never a guessed match.
    pub fn preserve_overrides(
        &self,
        previous: &crate::model::Project,
        next: &mut crate::model::Project,
    ) -> Result<(), String> {
        // Classification changes editable geometry (Running has no filled
        // contours), but must not change the captured source-region identity.
        // Trust that capture only while its automatic baseline still matches;
        // arbitrary edits to a candidate cannot forge a topology match.
        let source_identity = |object: &crate::model::Object| {
            next.raster_recipe
                .as_ref()
                .and_then(|recipe| {
                    recipe
                        .objects
                        .iter()
                        .find(|identity| {
                            identity.object_id == object.id
                                && identity.original_fingerprint == object_fingerprint(object)
                        })
                        .map(|identity| identity.region_identity)
                })
                .unwrap_or_else(|| crate::region_edit::region_identity(object))
        };
        let mut source_regions: std::collections::BTreeMap<_, _> = next
            .objects
            .iter()
            .map(|object| (object.id, source_identity(object)))
            .collect();
        let previous_regions: std::collections::BTreeSet<_> =
            self.objects.iter().map(|o| o.region_identity).collect();
        let next_regions: std::collections::BTreeSet<_> =
            source_regions.values().copied().collect();
        if previous_regions != next_regions {
            // A merged/split manual object may depend on several original regions.
            // Without a complete dependency proof, never copy it across changed topology.
            let manual = self.objects.iter().find(|identity| {
                previous
                    .objects
                    .iter()
                    .find(|o| o.id == identity.object_id)
                    .is_none_or(|o| object_fingerprint(o) != identity.original_fingerprint)
            });
            if let Some(identity) = manual {
                let name = previous
                    .objects
                    .iter()
                    .find(|o| o.id == identity.object_id)
                    .map_or(previous.name.as_str(), |o| o.name.as_str());
                return Err(format!(
                    "{name}: topology changed; a manual override cannot be matched. Keep the existing project or explicitly reset import decisions."
                ));
            }
        }
        let mut next_id = previous
            .objects
            .iter()
            .map(|o| o.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("Object ID limit reached")?;
        let mut diagnostic_ids = std::collections::BTreeMap::new();
        let mut remapped_regions = std::collections::BTreeMap::new();
        for object in &mut next.objects {
            let previous_id = object.id;
            object.id = next_id;
            if let Some(graph) = &mut object.centerline_graph
                && graph.source_object_id == previous_id
            {
                graph.source_object_id = object.id;
            }
            diagnostic_ids.insert(previous_id, object.id);
            if let Some(&region) = source_regions.get(&previous_id) {
                remapped_regions.insert(object.id, region);
            }
            next_id = next_id.checked_add(1).ok_or("Object ID limit reached")?;
        }
        source_regions = remapped_regions;
        // Remap fresh diagnostics before restoring old IDs, avoiding chains
        // where a fresh ID happens to equal another object's previous ID.
        if let Some(recipe) = &mut next.raster_recipe {
            for identity in &mut recipe.objects {
                if let Some(&id) = diagnostic_ids.get(&identity.object_id) {
                    identity.object_id = id;
                    if let Some(object) = next.objects.iter().find(|object| object.id == id) {
                        identity.original_fingerprint = object_fingerprint(object);
                    }
                }
            }
            for issue in &mut recipe.issues {
                if let Some(&id) = diagnostic_ids.get(&issue.object_id) {
                    issue.object_id = id;
                }
            }
        }
        let mut retained_old_ids = std::collections::BTreeSet::new();
        for identity in &self.objects {
            let Some(old) = previous.objects.iter().find(|o| o.id == identity.object_id) else {
                // A deliberately deleted imported object must not reappear on retrace.
                next.objects
                    .retain(|o| source_regions.get(&o.id) != Some(&identity.region_identity));
                if let Some(recipe) = &mut next.raster_recipe {
                    recipe.issues.retain(|issue| {
                        source_regions.get(&issue.object_id) != Some(&identity.region_identity)
                    });
                    recipe
                        .objects
                        .retain(|i| i.region_identity != identity.region_identity);
                    recipe.objects.push(SourceObjectIdentity {
                        object_id: identity.object_id,
                        region_identity: identity.region_identity,
                        original_fingerprint: identity.original_fingerprint,
                    });
                }
                continue;
            };
            let matched = next
                .objects
                .iter_mut()
                .find(|o| source_regions.get(&o.id) == Some(&identity.region_identity));
            if let Some(matched) = matched {
                let fresh_id = matched.id;
                let manual = object_fingerprint(old) != identity.original_fingerprint;
                if manual {
                    *matched = old.clone();
                } else {
                    // Preserve proven identity/order, while allowing a new
                    // automatic profile/classification to improve the result.
                    matched.id = old.id;
                    if let Some(graph) = &mut matched.centerline_graph
                        && graph.source_object_id == fresh_id
                    {
                        graph.source_object_id = old.id;
                    }
                }
                if manual {
                    // Fresh automatic diagnostics are authoritative: changing
                    // a machine/profile setting may resolve an old warning.
                    // A restored manual object still keeps its prior issues.
                    retained_old_ids.insert(old.id);
                }
                if let Some(recipe) = &mut next.raster_recipe
                    && let Some(next_identity) = recipe
                        .objects
                        .iter_mut()
                        .find(|i| i.region_identity == identity.region_identity)
                {
                    next_identity.object_id = old.id;
                    next_identity.original_fingerprint = if manual {
                        identity.original_fingerprint
                    } else {
                        object_fingerprint(matched)
                    };
                }
                if let Some(recipe) = &mut next.raster_recipe {
                    for issue in &mut recipe.issues {
                        if issue.object_id == fresh_id {
                            issue.object_id = old.id;
                        }
                    }
                }
            } else if object_fingerprint(old) != identity.original_fingerprint {
                return Err(format!(
                    "{}: topology changed; a manual override cannot be matched. Keep the existing project or explicitly reset import decisions.",
                    old.name
                ));
            }
        }
        // Objects added by the user are never part of the source's segmentation.
        next.objects.extend(
            previous
                .objects
                .iter()
                .filter(|o| !self.objects.iter().any(|i| i.object_id == o.id))
                .cloned(),
        );
        if let Some(recipe) = &mut next.raster_recipe {
            for issue in &self.issues {
                if issue.object_id != 0
                    && retained_old_ids.contains(&issue.object_id)
                    && !recipe.issues.iter().any(|fresh| {
                        fresh.object_id == issue.object_id
                            && fresh.point == issue.point
                            && fresh.message == issue.message
                    })
                {
                    recipe.issues.push(issue.clone());
                }
            }
            if previous_regions == next_regions {
                for warning in &self.warnings {
                    if !recipe.warnings.contains(warning) {
                        recipe.warnings.push(warning.clone());
                    }
                }
            }
        }
        next.objects.sort_by_key(|o| {
            previous
                .objects
                .iter()
                .position(|old| old.id == o.id)
                .unwrap_or(usize::MAX)
        });
        next.layering = previous.layering.clone();
        Ok(())
    }
}

/// The original PNG/JPEG bytes, independent of the original file's location.
/// Hex serialization bounds encoded source data to 24MB even in pretty JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceImage {
    pub file_name: String,
    #[serde(with = "encoded_bytes")]
    pub encoded: Vec<u8>,
    pub width_mm: f64,
    pub height_mm: f64,
}

mod encoded_bytes {
    use super::MAX_SOURCE_BYTES;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        const HEX: &[u8] = b"0123456789abcdef";
        let mut encoded = String::with_capacity(bytes.len() * 2);
        for &byte in bytes {
            encoded.push(HEX[usize::from(byte >> 4)] as char);
            encoded.push(HEX[usize::from(byte & 15)] as char);
        }
        serializer.serialize_str(&encoded)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() > MAX_SOURCE_BYTES * 2 || encoded.len() % 2 != 0 {
            return Err(serde::de::Error::custom(
                "Embedded source bytes exceed the limit or have invalid encoding",
            ));
        }
        let digit = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        };
        encoded
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                digit(pair[0])
                    .zip(digit(pair[1]))
                    .map(|(hi, lo)| hi * 16 + lo)
                    .ok_or_else(|| {
                        serde::de::Error::custom("Invalid embedded source byte encoding")
                    })
            })
            .collect()
    }
}

impl SourceImage {
    /// Bounded lossless encoding: never silently change source coordinates.
    pub fn from_image(
        file_name: String,
        image: &image::DynamicImage,
        width_mm: f64,
    ) -> Result<Self, String> {
        validate_dimensions(image.width(), image.height())?;
        let mut bytes = Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        let encoded = bytes.into_inner();
        if encoded.len() > MAX_SOURCE_BYTES {
            return Err("Source image exceeds the lossless 12MB embedded artwork limit; use a smaller source image before cleanup".into());
        }
        Self::from_encoded(file_name, encoded, width_mm)
    }

    pub fn from_path(path: &Path, width_mm: f64) -> Result<Self, String> {
        if fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_SOURCE_BYTES as u64 {
            return Err("Source image exceeds the 12MB embedded artwork limit".into());
        }
        Self::from_encoded(
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            fs::read(path).map_err(|e| e.to_string())?,
            width_mm,
        )
    }

    pub fn from_encoded(
        file_name: String,
        encoded: Vec<u8>,
        width_mm: f64,
    ) -> Result<Self, String> {
        let mut source = Self {
            file_name,
            encoded,
            width_mm,
            height_mm: width_mm,
        };
        let image = source.decode()?;
        source.height_mm = width_mm * f64::from(image.height()) / f64::from(image.width());
        source.validate()?;
        Ok(source)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.file_name.len() > 4096 {
            return Err("Source image filename exceeds 4096 bytes".into());
        }
        if self.encoded.is_empty() || self.encoded.len() > MAX_SOURCE_BYTES {
            return Err("Source image must contain 1–12,000,000 encoded bytes".into());
        }
        if ![self.width_mm, self.height_mm]
            .into_iter()
            .all(|v| v.is_finite() && v > 0. && v <= 9990.)
        {
            return Err("Source image dimensions must be finite and within 0–9990mm".into());
        }
        let reader = self.reader()?;
        let (width, height) = reader.into_dimensions().map_err(|e| e.to_string())?;
        validate_dimensions(width, height)?;
        let aspect = self.width_mm * f64::from(height) / f64::from(width);
        if (aspect - self.height_mm).abs() > 1e-6 * self.height_mm.max(1.) {
            return Err("Source image physical dimensions do not match its aspect ratio".into());
        }
        Ok(())
    }

    fn reader(&self) -> Result<image::ImageReader<Cursor<&[u8]>>, String> {
        if self.encoded.is_empty() || self.encoded.len() > MAX_SOURCE_BYTES {
            return Err("Source image must contain 1–12,000,000 encoded bytes".into());
        }
        let mut reader = image::ImageReader::new(Cursor::new(self.encoded.as_slice()))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        if !matches!(
            reader.format(),
            Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
        ) {
            return Err("Embedded source artwork must be PNG or JPEG".into());
        }
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(128_000_000);
        reader.limits(limits);
        Ok(reader)
    }

    pub fn decode(&self) -> Result<image::DynamicImage, String> {
        let (width, height) = self
            .reader()?
            .into_dimensions()
            .map_err(|e| e.to_string())?;
        validate_dimensions(width, height)?;
        self.reader()?.decode().map_err(|e| e.to_string())
    }
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > 32_000_000
    {
        return Err("Source image dimensions exceed 16384px per axis or 32 million pixels".into());
    }
    Ok(())
}

/// Writes a prepared project (including its chosen schema version) atomically.
pub fn save_project(project: &Project, path: &Path) -> Result<(), String> {
    project.validate_limits()?;
    if let Some(source) = &project.source_image {
        source.validate()?;
    }
    let bytes = serde_json::to_vec_pretty(project).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_PROJECT_BYTES {
        return Err("Project file exceeds 32MB".into());
    }
    atomic_write(path, &bytes)
}

/// The temporary file lives beside the destination, so replacement stays on
/// the same volume. Failure leaves the last saved destination untouched.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let file_name = path
        .file_name()
        .ok_or("Save destination needs a filename")?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(
        ".{}.{}.{}.tmp",
        file_name.to_string_lossy(),
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination).map_err(|e| e.to_string())
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH.
    let success = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0x1 | 0x8) };
    if success == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

pub fn recovery_path(project_path: Option<&Path>) -> PathBuf {
    match project_path {
        Some(path) => path.with_file_name(format!(
            ".{}.recovery.elab",
            path.file_name().unwrap_or_default().to_string_lossy()
        )),
        None => std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("EmbroideryLab")
            .join("Recovery")
            .join("unsaved.elab"),
    }
}

pub fn write_recovery(project: &Project, project_path: Option<&Path>) -> Result<PathBuf, String> {
    let path = recovery_path(project_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Use the public save path to apply all schema upgrades consistently.
    project.save(&path)?;
    Ok(path)
}

pub fn load_recovery(project_path: Option<&Path>) -> Result<Option<Project>, String> {
    let path = recovery_path(project_path);
    match fs::metadata(&path) {
        Ok(_) => Project::load(&path).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub fn clear_recovery(project_path: Option<&Path>) -> Result<(), String> {
    match fs::remove_file(recovery_path(project_path)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

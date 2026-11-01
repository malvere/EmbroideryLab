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
pub const MAX_SOURCE_BYTES: usize = 4_000_000;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// The original PNG/JPEG bytes, independent of the original file's location.
/// Hex serialization keeps the embedded bytes below 8MB even in pretty JSON.
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
    /// A bounded, explicitly re-encoded fallback for unusually large originals.
    pub fn from_image(
        file_name: String,
        image: &image::DynamicImage,
        width_mm: f64,
    ) -> Result<Self, String> {
        validate_dimensions(image.width(), image.height())?;
        let mut edge = 1800;
        loop {
            let resized = image.thumbnail(edge, edge);
            let mut bytes = Cursor::new(Vec::new());
            resized
                .write_to(&mut bytes, image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            let encoded = bytes.into_inner();
            if encoded.len() <= MAX_SOURCE_BYTES {
                return Self::from_encoded(file_name, encoded, width_mm);
            }
            if edge <= 450 {
                return Err("Source image could not fit the 4MB embedded artwork limit".into());
            }
            edge = edge * 3 / 4;
        }
    }

    pub fn from_path(path: &Path, width_mm: f64) -> Result<Self, String> {
        if fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_SOURCE_BYTES as u64 {
            return Err("Source image exceeds the 4MB embedded artwork limit".into());
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
            return Err("Source image must contain 1–4,000,000 encoded bytes".into());
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
            return Err("Source image must contain 1–4,000,000 encoded bytes".into());
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

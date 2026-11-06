use embroidery_lab::{
    Project,
    persistence::{self, SourceImage},
};
use std::{io::Cursor, path::PathBuf};

fn folder(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "embroidery-persistence-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}
fn source() -> SourceImage {
    let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        16,
        8,
        image::Rgba([10, 30, 200, 255]),
    ));
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    SourceImage::from_encoded("original.png".into(), bytes.into_inner(), 80.).unwrap()
}

#[test]
fn source_survives_independent_project_reopen_with_exact_original_bytes() {
    let source = source();
    assert_eq!(source.height_mm, 40.);
    let project = Project {
        source_image: Some(source.clone()),
        ..Project::demo()
    };
    let path = folder("source").join("embedded.elab");
    project.save(&path).unwrap();
    let reopened = Project::load(&path).unwrap();
    let persisted = reopened.source_image.unwrap();
    assert_eq!(persisted, source);
    assert_eq!(
        persisted.decode().unwrap().to_rgba8().get_pixel(0, 0).0,
        [10, 30, 200, 255]
    );
    assert!(std::fs::metadata(path).unwrap().len() < 10_000);
}

#[test]
fn raster_import_embeds_source_for_gui_and_cli_without_original_file() {
    let dir = folder("raster-import");
    let image_path = dir.join("raster.png");
    let project_path = dir.join("raster.elab");
    let original = source();
    std::fs::write(&image_path, &original.encoded).unwrap();
    let project = embroidery_lab::import::load_image_with_options(
        &image_path,
        80.,
        &embroidery_lab::import::RasterOptions::default(),
    )
    .unwrap();
    assert_eq!(project.version, 5);
    assert!(project.raster_recipe.is_some());
    assert_eq!(
        project.source_image.as_ref().unwrap().encoded,
        original.encoded
    );
    project.save(&project_path).unwrap();
    std::fs::remove_file(image_path).unwrap();
    assert!(
        Project::load(&project_path)
            .unwrap()
            .source_image
            .unwrap()
            .decode()
            .is_ok()
    );
}

#[test]
fn invalid_or_oversized_source_is_rejected_before_unbounded_decode() {
    assert!(SourceImage::from_encoded("bad.png".into(), vec![1, 2, 3], 10.).is_err());
    assert!(
        SourceImage::from_encoded(
            "big.png".into(),
            vec![0; persistence::MAX_SOURCE_BYTES + 1],
            10.
        )
        .is_err()
    );
    let mut invalid = source();
    invalid.height_mm = 12.;
    assert!(invalid.validate().is_err());
    assert!(
        serde_json::from_str::<SourceImage>(
            r#"{"file_name":"x","encoded":"zz","width_mm":1,"height_mm":1}"#
        )
        .is_err()
    );
}

#[test]
fn atomic_replace_and_failed_save_preserve_previous_document() {
    let dir = folder("replace");
    let path = dir.join("design.elab");
    persistence::atomic_write(&path, b"first").unwrap();
    persistence::atomic_write(&path, b"second").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"second");
    let project = Project {
        source_image: Some(SourceImage {
            encoded: vec![],
            ..source()
        }),
        ..Project::demo()
    };
    assert!(persistence::save_project(&project, &path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"second");
    assert!(persistence::atomic_write(&dir, b"cannot replace directory").is_err());
    assert!(dir.is_dir());
    assert!(
        !std::fs::read_dir(dir).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp"))
    );
}

#[test]
fn recovery_is_separate_from_last_saved_project_and_can_be_cleared() {
    let path = folder("recovery").join("design.elab");
    let original = Project::demo();
    original.save(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut edited = original.clone();
    edited.name = "Recovered edits".into();
    let recovery = persistence::write_recovery(&edited, Some(&path)).unwrap();
    assert_ne!(recovery, path);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        persistence::load_recovery(Some(&path))
            .unwrap()
            .unwrap()
            .name,
        "Recovered edits"
    );
    persistence::clear_recovery(Some(&path)).unwrap();
    assert!(persistence::load_recovery(Some(&path)).unwrap().is_none());
    assert_ne!(persistence::recovery_path(None), path);
}

#[test]
fn independent_underlay_angle_uses_schema_five_and_legacy_none_is_omitted() {
    let path = folder("underlay-angle").join("direction.elab");
    let mut project = Project::demo();
    assert!(project.objects[0].settings.underlay_angle_deg.is_none());
    let fingerprint = persistence::object_fingerprint(&project.objects[0]);
    project.save(&path).unwrap();
    let legacy = Project::load(&path).unwrap();
    assert_eq!(legacy.version, 1);
    assert_eq!(
        persistence::object_fingerprint(&legacy.objects[0]),
        fingerprint
    );
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("underlay_angle_deg")
    );
    project.objects[0].settings.underlay_angle_deg = Some(31.25);
    let before = embroidery_lab::engine::generate(&project).unwrap();
    project.save(&path).unwrap();
    let reopened = Project::load(&path).unwrap();
    assert_eq!(reopened.version, 5);
    assert_eq!(reopened.objects[0].settings.underlay_angle_deg, Some(31.25));
    let after = embroidery_lab::engine::generate(&reopened).unwrap();
    assert_eq!(
        embroidery_lab::dst::encode(&before, "angle").unwrap(),
        embroidery_lab::dst::encode(&after, "angle").unwrap()
    );
    let saved = std::fs::read(&path).unwrap();
    for invalid in [f64::NAN, f64::INFINITY, -180.1, 180.1] {
        project.objects[0].settings.underlay_angle_deg = Some(invalid);
        assert!(
            project
                .validate_limits()
                .unwrap_err()
                .contains("underlay angle")
        );
        assert!(embroidery_lab::engine::generate(&project).is_err());
        assert!(project.save(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), saved);
    }
}

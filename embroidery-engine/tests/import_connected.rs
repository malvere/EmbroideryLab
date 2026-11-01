use embroidery_lab::{
    Object, Point, Project,
    import::{self, BackgroundMode, RasterOptions},
};
use image::{Rgba, RgbaImage};
use std::path::PathBuf;

const WHITE: [u8; 4] = [255, 255, 255, 255];
const BLUE: [u8; 4] = [20, 60, 180, 255];

fn path(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!(
        "embroidery-connected-import-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&folder).unwrap();
    folder.join(name)
}
fn options(background: BackgroundMode) -> RasterOptions {
    RasterOptions {
        max_dimension: 512,
        palette_size: 2,
        min_region_pixels: 1,
        simplify_tolerance_mm: 0.,
        background,
        ..RasterOptions::default()
    }
}
fn import_image(name: &str, image: &RgbaImage, background: BackgroundMode) -> Project {
    let file = path(name);
    image.save(&file).unwrap();
    import::load_image_with_options(&file, f64::from(image.width()), &options(background)).unwrap()
}
fn contains(object: &Object, x: f64, y: f64) -> bool {
    let point = Point::new(x, y);
    let mut inside = false;
    for ring in &object.contours {
        for (a, b) in ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
        {
            if (a.y > point.y) != (b.y > point.y)
                && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
            {
                inside = !inside;
            }
        }
    }
    inside
}
fn colored_at(project: &Project, color: [u8; 3], x: f64, y: f64) -> bool {
    project
        .objects
        .iter()
        .any(|object| object.color == color && contains(object, x, y))
}
fn eye_image() -> RgbaImage {
    let mut image = RgbaImage::from_pixel(64, 64, Rgba(WHITE));
    for y in 8..56 {
        for x in 8..56 {
            image.put_pixel(x, y, Rgba(BLUE));
        }
    }
    for y in 24..40 {
        for x in 24..40 {
            image.put_pixel(x, y, Rgba(WHITE));
        }
    }
    image
}

#[test]
fn connected_color_preserves_enclosed_white_detail_and_removes_exterior() {
    let image = eye_image();
    let connected = import_image(
        "enclosed-eye.png",
        &image,
        BackgroundMode::ConnectedColors {
            rgb: vec![[255; 3]],
            tolerance: 0.,
        },
    );
    let global = import_image(
        "enclosed-eye.png",
        &image,
        BackgroundMode::Colors {
            rgb: vec![[255; 3]],
            tolerance: 0.,
        },
    );
    assert_eq!(connected.objects.len(), 2);
    assert_eq!(global.objects.len(), 1);
    assert!(colored_at(&connected, [255; 3], 32., 32.));
    assert!(!colored_at(&global, [255; 3], 32., 32.));
    assert!(
        !connected
            .objects
            .iter()
            .any(|object| contains(object, 2., 2.))
    );
    assert!(colored_at(&connected, [20, 60, 180], 16., 16.));
    assert!(!colored_at(&connected, [20, 60, 180], 32., 32.));
    let repeat = import_image(
        "enclosed-eye.png",
        &image,
        BackgroundMode::ConnectedColors {
            rgb: vec![[255; 3]],
            tolerance: 0.,
        },
    );
    assert_eq!(
        serde_json::to_string(&connected).unwrap(),
        serde_json::to_string(&repeat).unwrap()
    );
}

#[test]
fn connected_border_tolerates_small_boundary_color_noise_without_erasing_eye() {
    let mut image = eye_image();
    for y in 0..64 {
        for x in 0..64 {
            if x < 8 || y < 8 || x >= 56 || y >= 56 {
                let shade = 250 + ((x + y) % 6) as u8;
                image.put_pixel(x, y, Rgba([shade, shade, shade, 255]));
            }
        }
    }
    let project = import_image(
        "noisy-perimeter.png",
        &image,
        BackgroundMode::ConnectedBorder { tolerance: 12. },
    );
    assert_eq!(project.objects.len(), 2);
    assert!(colored_at(&project, [255; 3], 32., 32.));
    assert!(
        !project
            .objects
            .iter()
            .any(|object| contains(object, 3., 3.))
    );
}

#[test]
fn connected_mask_has_no_diagonal_leaks_through_opaque_barrier() {
    let mut image = RgbaImage::from_pixel(32, 32, Rgba(BLUE));
    for (x, y) in [(0, 0), (1, 1), (2, 2)] {
        image.put_pixel(x, y, Rgba(WHITE));
    }
    let project = import_image(
        "diagonal.png",
        &image,
        BackgroundMode::ConnectedColors {
            rgb: vec![[255; 3]],
            tolerance: 0.,
        },
    );
    assert!(!colored_at(&project, [255; 3], 0.5, 0.5));
    assert!(colored_at(&project, [255; 3], 1.5, 1.5));
    assert!(colored_at(&project, [255; 3], 2.5, 2.5));
}

#[test]
fn transparency_connects_exterior_but_isolated_alpha_hole_does_not_seed_color_mask() {
    let mut image = RgbaImage::from_pixel(64, 64, Rgba(BLUE));
    for x in 0..20 {
        image.put_pixel(x, 10, Rgba([0; 4]));
    }
    for y in 8..14 {
        for x in 20..28 {
            image.put_pixel(x, y, Rgba(WHITE));
        }
    }
    image.put_pixel(35, 40, Rgba([0; 4]));
    for y in 39..43 {
        for x in 36..41 {
            image.put_pixel(x, y, Rgba(WHITE));
        }
    }
    let project = import_image(
        "alpha-path.png",
        &image,
        BackgroundMode::ConnectedColors {
            rgb: vec![[255; 3]],
            tolerance: 0.,
        },
    );
    assert!(!colored_at(&project, [255; 3], 24., 11.));
    assert!(colored_at(&project, [255; 3], 38., 41.));
    assert!(
        !project
            .objects
            .iter()
            .any(|object| contains(object, 35.5, 40.5))
    );
    assert!(
        !project
            .objects
            .iter()
            .any(|object| contains(object, 10.5, 10.5))
    );
}

#[test]
fn multiple_selected_colors_share_exterior_connectivity_without_implicit_white() {
    let gold = [200, 160, 60, 255];
    let mut image = eye_image();
    for y in 0..64 {
        for x in 0..8 {
            image.put_pixel(x, y, Rgba(gold));
        }
    }
    let project = import_image(
        "two-background-colors.png",
        &image,
        BackgroundMode::ConnectedColors {
            rgb: vec![[200, 160, 60], [255; 3]],
            tolerance: 0.,
        },
    );
    assert_eq!(project.objects.len(), 2);
    assert!(colored_at(&project, [255; 3], 32., 32.));
    assert!(
        !project
            .objects
            .iter()
            .any(|object| contains(object, 3., 3.))
    );
    assert!(
        !project
            .objects
            .iter()
            .any(|object| contains(object, 32., 3.))
    );
    let only_gold = import_image(
        "only-gold.png",
        &image,
        BackgroundMode::ConnectedColors {
            rgb: vec![[200, 160, 60]],
            tolerance: 0.,
        },
    );
    assert!(colored_at(&only_gold, [255; 3], 32., 3.));
}

#[test]
fn connected_modes_validate_and_serialize_without_changing_legacy_defaults() {
    assert_eq!(
        RasterOptions::default().background,
        BackgroundMode::NearWhite
    );
    for background in [
        BackgroundMode::ConnectedColors {
            rgb: vec![],
            tolerance: 0.,
        },
        BackgroundMode::ConnectedColors {
            rgb: vec![[0; 3]; 17],
            tolerance: 0.,
        },
        BackgroundMode::ConnectedColors {
            rgb: vec![[0; 3]],
            tolerance: f64::NAN,
        },
        BackgroundMode::ConnectedBorder { tolerance: -1. },
        BackgroundMode::ConnectedBorder {
            tolerance: f64::INFINITY,
        },
    ] {
        assert!(options(background).validate().is_err());
    }
    for background in [
        BackgroundMode::ConnectedColors {
            rgb: vec![[200, 160, 60], [255; 3]],
            tolerance: 24.,
        },
        BackgroundMode::ConnectedBorder { tolerance: 24. },
    ] {
        let original = options(background);
        let json = serde_json::to_string(&original).unwrap();
        let loaded: RasterOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded, original);
        assert!(loaded.validate().is_ok());
    }
}

#[test]
fn connected_border_without_opaque_samples_has_clear_error() {
    let file = path("transparent-perimeter-error.png");
    let mut image = RgbaImage::new(32, 32);
    for y in 8..24 {
        for x in 8..24 {
            image.put_pixel(x, y, Rgba(BLUE));
        }
    }
    image.save(&file).unwrap();
    let error = import::load_image_with_options(
        &file,
        32.,
        &options(BackgroundMode::ConnectedBorder { tolerance: 24. }),
    )
    .unwrap_err();
    assert!(error.contains("opaque perimeter"));
}

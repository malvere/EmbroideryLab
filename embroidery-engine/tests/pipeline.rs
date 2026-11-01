use embroidery_lab::{Command, Project, dst, engine, import};
use std::path::PathBuf;
fn temp(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("embroidery-lab-tests-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    folder.join(name)
}

#[test]
fn svg_to_editable_project_to_valid_dst() {
    let path = temp("transformed-hole.svg");
    std::fs::write(&path,r##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="100"><g transform="translate(10 5)"><path fill="#257ac2" fill-rule="evenodd" d="M0 0H80V60H0Z M20 20H60V40H20Z"/></g></svg>"##).unwrap();
    let project = import::load_svg(&path, 120.).unwrap();
    assert_eq!(project.objects.len(), 1);
    assert_eq!(project.objects[0].contours.len(), 2);
    assert_eq!(project.objects[0].contours[0][0].x, 10.);
    let saved = temp("project.elab");
    project.save(&saved).unwrap();
    let project = Project::load(&saved).unwrap();
    let stitches = engine::generate(&project).unwrap();
    let bytes = dst::encode(&stitches, "pipeline").unwrap();
    let decoded = dst::decode(&bytes).unwrap();
    assert!(
        decoded
            .iter()
            .filter(|s| s.command == Command::Stitch)
            .count()
            > 100
    );
    assert_eq!(decoded.last().unwrap().command, Command::End);
}
#[test]
fn png_to_contours_preserves_topology() {
    let path = temp("logo.png");
    let mut image = image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]));
    for y in 8..56 {
        for x in 8..56 {
            if !(24..40).contains(&x) || !(24..40).contains(&y) {
                image.put_pixel(x, y, image::Rgba([30, 80, 180, 255]));
            }
        }
    }
    image.save(&path).unwrap();
    let p = import::load_image(&path, 64.).unwrap();
    assert_eq!(p.objects.len(), 1);
    assert_eq!(p.objects[0].contours.len(), 2);
    let decoded =
        dst::decode(&dst::encode(&engine::generate(&p).unwrap(), "PNG").unwrap()).unwrap();
    assert!(dst::stats(&decoded).stitches > 100);
}
#[test]
fn jpg_import_and_generation() {
    let path = temp("logo.jpg");
    let mut image = image::RgbImage::from_pixel(64, 64, image::Rgb([255, 255, 255]));
    for y in 8..56 {
        for x in 8..56 {
            image.put_pixel(x, y, image::Rgb([35, 85, 190]));
        }
    }
    image.save(&path).unwrap();
    let p = import::load_image(&path, 64.).unwrap();
    assert!(!p.objects.is_empty());
    assert!(dst::decode(&dst::encode(&engine::generate(&p).unwrap(), "JPEG").unwrap()).is_ok());
}
#[test]
fn unsupported_svg_is_explicit() {
    let path = temp("gradient.svg");
    std::fs::write(&path,r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><defs><linearGradient id="g"><stop stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient></defs><rect width="20" height="20" fill="url(#g)"/></svg>"##).unwrap();
    assert!(
        import::load_svg(&path, 20.)
            .unwrap_err()
            .contains("gradient")
    );
}
#[test]
fn quantized_dst_roundtrip_keeps_commands_and_points() {
    let s = engine::generate(&Project::demo()).unwrap();
    let first = dst::decode(&dst::encode(&s, "first").unwrap()).unwrap();
    let second = dst::decode(&dst::encode(&first, "second").unwrap()).unwrap();
    assert_eq!(first.len(), second.len());
    for (a, b) in first.iter().zip(second.iter()) {
        assert_eq!(a.point(), b.point());
        assert_eq!(a.command, b.command);
    }
}

#[test]
fn long_shallow_bezier_is_flattened_with_geometric_tolerance() {
    use embroidery_lab::{CubicBezier, Point};
    let curve = CubicBezier {
        start: Point::new(0., 0.),
        control1: Point::new(0., 2.),
        control2: Point::new(100., 2.),
        end: Point::new(100., 0.),
    };
    let points = curve.flatten(0.03);
    assert!(points.len() > 2);
    let midpoint = Point::new(50., 1.5);
    let distance = points
        .windows(2)
        .map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let t = (((midpoint.x - a.x) * dx + (midpoint.y - a.y) * dy) / (dx * dx + dy * dy))
                .clamp(0., 1.);
            midpoint.distance(a.lerp(b, t))
        })
        .fold(f64::INFINITY, f64::min);
    assert!(distance <= 0.03);
}

#[test]
fn same_winding_nested_svg_is_solid() {
    let path = temp("nonzero.svg");
    std::fs::write(&path,r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><path fill="#257ac2" d="M0 0H80V60H0Z M20 20H60V40H20Z"/></svg>"##).unwrap();
    let project = import::load_svg(&path, 100.).unwrap();
    assert_eq!(project.objects[0].contours.len(), 1);
}

#[test]
fn dst_quantization_respects_requested_maximum_stitch_length() {
    use embroidery_lab::Point;
    for limit in [0.3, 3.5, 12.] {
        for angle in [0., 27., 90., 137.] {
            let mut project = Project::demo();
            let object = &mut project.objects[0];
            object.contours = vec![vec![
                Point::new(1., 1.),
                Point::new(11., 1.),
                Point::new(11., 9.),
                Point::new(1., 9.),
            ]];
            object.settings.length_mm = limit;
            object.settings.angle_deg = angle;
            let decoded =
                dst::decode(&dst::encode(&engine::generate(&project).unwrap(), "length").unwrap())
                    .unwrap();
            assert!(
                dst::stats(&decoded).max_stitch_mm <= limit + 1e-8,
                "limit={limit}, angle={angle}"
            );
        }
    }
}

use embroidery_lab::{Command, Object, Point, Project, engine, import};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

fn load(body: &str) -> Result<Project, String> {
    let path: PathBuf = std::env::temp_dir().join(format!(
        "embroidery-svg-{}-{}.svg",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, body).unwrap();
    let result = import::load_svg(&path, 80.);
    std::fs::remove_file(path).unwrap();
    result
}

fn area(object: &Object) -> f64 {
    object
        .contours
        .iter()
        .map(|r| {
            r.iter()
                .zip(r.iter().cycle().skip(1))
                .take(r.len())
                .map(|(a, b)| a.x * b.y - b.x * a.y)
                .sum::<f64>()
                / 2.
        })
        .sum::<f64>()
        .abs()
}

fn bounds(object: &Object) -> [f64; 4] {
    object.contours.iter().flatten().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, p| [b[0].min(p.x), b[1].min(p.y), b[2].max(p.x), b[3].max(p.y)],
    )
}

fn contains(object: &Object, p: Point) -> bool {
    let mut inside = false;
    for ring in &object.contours {
        for (a, b) in ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
        {
            if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                inside = !inside;
            }
        }
    }
    inside
}

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} differs from {expected} by more than {tolerance}"
    );
}

#[test]
fn solid_open_stroke_keeps_round_caps_width_and_generates_real_stitches() {
    let project=load(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><path id="border" fill="none" stroke="#164e99" stroke-width="4" stroke-linecap="round" d="M10 10H30"/></svg>"##).unwrap();
    assert_eq!(project.objects.len(), 1);
    let object = &project.objects[0];
    assert_eq!(object.color, [22, 78, 153]);
    for (a, e) in bounds(object).into_iter().zip([8., 8., 32., 12.]) {
        near(a, e, 0.03);
    }
    near(area(object), 80. + 4. * std::f64::consts::PI, 0.35);
    assert!(contains(object, Point::new(8.2, 10.)));
    assert!(!contains(object, Point::new(8.2, 8.2)));
    assert!(
        engine::generate_plan(&project)
            .unwrap()
            .stitches
            .iter()
            .filter(|s| s.command == Command::Stitch)
            .count()
            > 20
    );
}

#[test]
fn fill_and_stroke_respect_nonuniform_transform_and_svg_paint_order() {
    for (order, colors) in [
        ("normal", [[255, 0, 0], [0, 0, 0]]),
        ("stroke fill", [[0, 0, 0], [255, 0, 0]]),
    ] {
        let project=load(&format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><g transform="translate(10 5) scale(2 1)"><rect id="box" x="1" y="1" width="10" height="8" fill="#ff0000" stroke="#000000" stroke-width="2" stroke-linejoin="miter" paint-order="{order}"/></g></svg>"##)).unwrap();
        assert_eq!(
            project.objects.iter().map(|o| o.color).collect::<Vec<_>>(),
            colors
        );
        let fill = project
            .objects
            .iter()
            .find(|o| o.color == [255, 0, 0])
            .unwrap();
        let stroke = project
            .objects
            .iter()
            .find(|o| o.color == [0, 0, 0])
            .unwrap();
        assert_eq!(bounds(fill), [12., 6., 32., 14.]);
        assert_eq!(bounds(stroke), [10., 5., 34., 15.]);
        near(area(fill), 160., 1e-6);
        near(area(stroke), 144., 1e-6);
        assert!(
            !contains(stroke, Point::new(22., 10.)),
            "A closed stroke retains its internal hole"
        );
        assert!(engine::generate_plan(&project).is_ok());
    }
}

#[test]
fn dashed_stroke_contains_only_its_painted_spans() {
    let project=load(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><path fill="none" stroke="#123456" stroke-width="2" stroke-dasharray="4 6" d="M5 5H35"/></svg>"##).unwrap();
    assert_eq!(project.objects.len(), 1);
    let object = &project.objects[0];
    near(area(object), 24., 1e-5);
    for x in [7., 17., 27.] {
        assert!(contains(object, Point::new(x, 5.)));
    }
    for x in [12., 22., 32.] {
        assert!(
            !contains(object, Point::new(x, 5.)),
            "dash gap at {x} must remain empty"
        );
    }
    assert!(engine::generate_plan(&project).is_ok());
}

#[test]
fn nonzero_overlapping_subpaths_are_unioned_while_evenodd_preserves_cancellation() {
    for (rule, expected, overlap_filled) in [("nonzero", 650., true), ("evenodd", 500., false)] {
        let project=load(&format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><path fill="#123456" fill-rule="{rule}" d="M0 0H20V20H0Z M10 5H30V25H10Z"/></svg>"##)).unwrap();
        let object = &project.objects[0];
        near(area(object), expected, 1e-5);
        assert_eq!(contains(object, Point::new(15., 10.)), overlap_filled);
        assert!(
            contains(object, Point::new(25., 22.)),
            "the non-nested second contour must not be discarded"
        );
        assert!(engine::generate_plan(&project).is_ok());
    }
}

#[test]
fn nonzero_opposite_winding_and_self_crossings_are_normalized_without_hulls() {
    let opposite=load(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><path fill="#123456" d="M0 0H20V20H0Z M10 5V25H30V5Z"/></svg>"##).unwrap();
    near(area(&opposite.objects[0]), 500., 1e-5);
    assert!(!contains(&opposite.objects[0], Point::new(15., 10.)));
    let crossed=load(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><path fill="#123456" d="M0 0L20 20L0 20L20 0Z"/></svg>"##).unwrap();
    near(area(&crossed.objects[0]), 200., 1e-5);
    assert!(engine::generate_plan(&crossed).is_ok());
}

#[test]
fn local_references_expand_symbols_and_group_transforms_without_losing_color() {
    let project=load(r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="80" height="40"><defs><symbol id="tile" viewBox="0 0 10 10"><rect width="10" height="10" fill="#224488"/></symbol><g id="part"><path fill="#dd2222" d="M0 0H5V5H0Z"/></g></defs><use href="#tile" x="20" y="5" width="20" height="20"/><g transform="translate(50 10)"><use xlink:href="#part"/></g></svg>"##).unwrap();
    assert_eq!(project.objects.len(), 2);
    assert_eq!(project.objects[0].color, [34, 68, 136]);
    assert_eq!(bounds(&project.objects[0]), [20., 5., 40., 25.]);
    assert_eq!(project.objects[1].color, [221, 34, 34]);
    assert_eq!(bounds(&project.objects[1]), [50., 10., 55., 15.]);
    assert!(engine::generate_plan(&project).is_ok());
}

#[test]
fn external_missing_cyclic_and_ambiguous_references_fail_explicitly() {
    for (body, reason) in [
        (r##"<use href="other.svg#x"/>"##, "external"),
        (r##"<use href="#missing"/>"##, "missing"),
        (r##"<g id="a"><use href="#a"/></g>"##, "cyclic"),
        (
            r##"<defs><g id="a"><use href="#b"/></g><g id="b"><use href="#a"/></g></defs><use href="#a"/>"##,
            "cyclic",
        ),
        (
            r##"<defs><rect id="a" width="2" height="2"/><rect id="a" width="3" height="3"/></defs><use href="#a"/>"##,
            "duplicate",
        ),
    ] {
        let error = load(&format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40">{body}</svg>"##
        ))
        .unwrap_err();
        assert!(error.contains(reason), "{error}");
    }
}

#[test]
fn unsupported_paints_and_effects_are_not_silently_rasterized_or_omitted() {
    for (body, reason) in [
        (r##"<text x="1" y="10">Letters</text>"##, "text"),
        (
            r##"<image href="drawing.png" width="20" height="20"/>"##,
            "raster",
        ),
        (
            r##"<defs><linearGradient id="g"><stop stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient></defs><rect width="20" height="20" fill="url(#g)"/>"##,
            "gradient",
        ),
        (
            r##"<rect width="20" height="20" opacity="0.5"/>"##,
            "opacity",
        ),
        (
            r##"<defs><clipPath id="c"><rect width="5" height="5"/></clipPath></defs><rect width="20" height="20" clip-path="url(#c)"/>"##,
            "clipping",
        ),
    ] {
        let error = load(&format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40">{body}</svg>"##
        ))
        .unwrap_err();
        assert!(error.contains(reason), "{error}");
    }
}

#[test]
fn excessively_fine_dash_pattern_is_rejected_before_expansion() {
    let error=load(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><path fill="none" stroke="black" stroke-width="1" stroke-dasharray=".00001 .00001" d="M0 10H70"/></svg>"##).unwrap_err();
    assert!(error.contains("8192 spans"), "{error}");
}

#[test]
fn implicit_viewport_clips_with_stroked_symbols_are_rejected_without_a_coarse_bbox_guess() {
    let error=load(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><defs><symbol id="border" viewBox="0 0 10 10"><path fill="none" stroke="black" stroke-width="1" d="M2 5H8"/></symbol></defs><use href="#border" x="20" y="5" width="20" height="20"/></svg>"##).unwrap_err();
    assert!(error.contains("clipping"), "{error}");
}

#[test]
fn non_default_vector_effect_is_rejected_in_attributes_inline_style_and_stylesheets() {
    for body in [
        r##"<path fill="none" stroke="black" stroke-width="2" vector-effect="non-scaling-stroke" transform="scale(2)" d="M2 5H15"/>"##,
        r##"<path fill="none" stroke="black" stroke-width="2" style="stroke-linecap:round; vector-effect: non-scaling-stroke !important" transform="scale(2)" d="M2 5H15"/>"##,
        r##"<style>path { stroke: black; vector-effect: non-scaling-stroke; }</style><path fill="none" stroke-width="2" transform="scale(2)" d="M2 5H15"/>"##,
    ] {
        let error = load(&format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40">{body}</svg>"##
        ))
        .unwrap_err();
        assert!(error.contains("unsupported vector-effect"), "{error}");
    }
}

#[test]
fn ordinary_none_vector_effect_and_commented_or_quoted_css_text_remain_usable() {
    let project=load(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><style>/* vector-effect: non-scaling-stroke; */ path { vector-effect: none !important; content: 'a;vector-effect:non-scaling-stroke'; }</style><path fill="none" stroke="black" stroke-width="2" vector-effect="none" style="vector-effect: /* default */ none" d="M2 5H15"/></svg>"##).unwrap();
    assert_eq!(project.objects.len(), 1);
    assert!(engine::generate_plan(&project).is_ok());
}

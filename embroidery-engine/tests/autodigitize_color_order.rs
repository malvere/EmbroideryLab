use embroidery_lab::{Command, Object, Point, Project, autodigitize, engine};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

const RED: [u8; 3] = [180, 30, 40];
const BLUE: [u8; 3] = [30, 70, 180];

fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Vec<Point> {
    vec![
        Point::new(x, y),
        Point::new(x + w, y),
        Point::new(x + w, y + h),
        Point::new(x, y + h),
    ]
}
fn object(id: u64, color: [u8; 3], x: f64) -> Object {
    let mut object = Object {
        id,
        color,
        contours: vec![rectangle(x, 0., 2., 2.)],
        ..Object::default()
    };
    object.settings.underlay = false;
    object.settings.tie_in = false;
    object.settings.tie_off = false;
    object
}
fn ids(project: &Project) -> Vec<u64> {
    project.objects.iter().map(|o| o.id).collect()
}
fn colors(project: &Project) -> usize {
    engine::generate_plan(project)
        .unwrap()
        .stitches
        .iter()
        .filter(|s| s.command == Command::ColorChange)
        .count()
}
fn batch(project: &mut Project) {
    let warnings =
        autodigitize::batch_fresh_raster_colors(project, &AtomicBool::new(false)).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn disjoint_eight_color_import_has_seven_color_changes_without_changing_objects() {
    let mut project = Project {
        objects: (0..24)
            .map(|i| object(i + 1, [(i % 8) as u8 * 30, 20, 80], i as f64 * 3.))
            .collect(),
        ..Project::default()
    };
    let before: BTreeMap<_, _> = project
        .objects
        .iter()
        .map(|o| (o.id, serde_json::to_value(o).unwrap()))
        .collect();
    assert_eq!(colors(&project), 23);
    batch(&mut project);
    assert_eq!(colors(&project), 7);
    let after: BTreeMap<_, _> = project
        .objects
        .iter()
        .map(|o| (o.id, serde_json::to_value(o).unwrap()))
        .collect();
    assert_eq!(before, after);
    assert_eq!(&ids(&project)[..3], &[1, 9, 17]);
}

#[test]
fn overlapping_red_blue_red_layers_keep_source_order() {
    let mut project = Project {
        objects: vec![object(1, RED, 0.), object(2, BLUE, 0.5), object(3, RED, 1.)],
        ..Project::default()
    };
    batch(&mut project);
    assert_eq!(ids(&project), [1, 2, 3]);
    assert_eq!(colors(&project), 2);
}

#[test]
fn shared_edge_and_shared_vertex_do_not_create_overlap_dependencies() {
    let mut third = object(3, RED, 4.);
    third.contours = vec![rectangle(2., 2., 2., 2.)];
    let mut project = Project {
        objects: vec![object(1, RED, 0.), object(2, BLUE, 2.), third],
        ..Project::default()
    };
    batch(&mut project);
    assert_eq!(ids(&project), [1, 3, 2]);
    assert_eq!(colors(&project), 1);
}

#[test]
fn differently_colored_island_in_a_hole_is_free_to_batch() {
    let mut frame = object(1, RED, 0.);
    frame.contours = vec![rectangle(0., 0., 10., 10.), rectangle(2., 2., 6., 6.)];
    let mut island = object(2, BLUE, 0.);
    island.contours = vec![rectangle(3., 3., 2., 2.)];
    let mut project = Project {
        objects: vec![frame, island, object(3, RED, 12.)],
        ..Project::default()
    };
    batch(&mut project);
    assert_eq!(ids(&project), [1, 3, 2]);
}

#[test]
fn repeated_batching_is_deterministic_and_idempotent() {
    let original = Project {
        objects: vec![
            object(1, RED, 0.),
            object(2, BLUE, 3.),
            object(3, RED, 6.),
            object(4, BLUE, 9.),
        ],
        ..Project::default()
    };
    let mut a = original.clone();
    let mut b = original;
    batch(&mut a);
    batch(&mut b);
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        serde_json::to_value(&b).unwrap()
    );
    let before = serde_json::to_value(&a).unwrap();
    batch(&mut a);
    assert_eq!(serde_json::to_value(&a).unwrap(), before);
}

#[test]
fn cancellation_retains_the_entire_candidate() {
    let mut project = Project {
        objects: vec![object(1, RED, 0.), object(2, BLUE, 3.), object(3, RED, 6.)],
        ..Project::default()
    };
    let before = serde_json::to_value(&project).unwrap();
    assert!(
        autodigitize::batch_fresh_raster_colors(&mut project, &AtomicBool::new(true))
            .unwrap_err()
            .contains("cancelled")
    );
    assert_eq!(serde_json::to_value(&project).unwrap(), before);
}

#[test]
fn invalid_geometry_retains_original_order_with_actionable_warning() {
    let mut project = Project {
        objects: vec![object(1, RED, 0.), object(2, BLUE, 3.), object(3, RED, 6.)],
        ..Project::default()
    };
    project.objects[1].contours = vec![vec![
        Point::new(0., 0.),
        Point::new(2., 2.),
        Point::new(0., 2.),
        Point::new(2., 0.),
    ]];
    let before = serde_json::to_value(&project).unwrap();
    let warnings =
        autodigitize::batch_fresh_raster_colors(&mut project, &AtomicBool::new(false)).unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0]
            .message
            .contains("original object order retained")
    );
    assert_eq!(serde_json::to_value(&project).unwrap(), before);
}

#[test]
fn ordinary_manual_generation_does_not_batch_colors() {
    let project = Project {
        objects: vec![object(1, RED, 0.), object(2, BLUE, 3.), object(3, RED, 6.)],
        ..Project::default()
    };
    assert_eq!(colors(&project), 2);
    assert_eq!(ids(&project), [1, 2, 3]);
}

fn object_snapshots(project: &Project) -> BTreeMap<u64, Vec<u8>> {
    project
        .objects
        .iter()
        .map(|object| (object.id, serde_json::to_vec(object).unwrap()))
        .collect()
}

fn color_runs(project: &Project) -> usize {
    usize::from(!project.objects.is_empty())
        + project
            .objects
            .windows(2)
            .filter(|pair| pair[0].color != pair[1].color)
            .count()
}

#[test]
fn more_than_two_thousand_sparse_regions_batch_on_either_spatial_axis() {
    for vertical in [false, true] {
        let mut project = Project {
            objects: (0..2048)
                .map(|index| {
                    let mut item = object(index + 1, if index % 2 == 0 { RED } else { BLUE }, 0.);
                    let (x, y) = if vertical {
                        (0., index as f64 * 3.)
                    } else {
                        ((index % 64) as f64 * 3., (index / 64) as f64 * 3.)
                    };
                    item.contours = vec![rectangle(x, y, 2., 2.)];
                    item
                })
                .collect(),
            ..Project::default()
        };
        let before = object_snapshots(&project);
        batch(&mut project);
        assert_eq!(
            color_runs(&project),
            2,
            "No theoretical all-pairs rejection of sparse valid regions"
        );
        assert_eq!(before, object_snapshots(&project));
        assert_eq!(&ids(&project)[..3], &[1, 3, 5]);
    }
}

#[test]
fn prepared_complex_notch_does_not_spend_node_products_on_disjoint_interior() {
    let mut project = Project {
        objects: (0..1024)
            .map(|index| {
                let mut item = object(index + 1, if index % 2 == 0 { RED } else { BLUE }, 0.);
                let center = Point::new(
                    10. + (index % 32) as f64 * 2.5,
                    10. + (index / 32) as f64 * 2.5,
                );
                item.contours = vec![
                    (0..32)
                        .map(|vertex| {
                            let theta = vertex as f64 * std::f64::consts::TAU / 32.;
                            Point::new(center.x + theta.cos() * 0.5, center.y + theta.sin() * 0.5)
                        })
                        .collect(),
                ];
                item
            })
            .collect(),
        ..Project::default()
    };
    // A large concave outline surrounds the small regions in its bounds, but
    // not its filled interior. There is no explicit hole shortcut to use.
    let corners = [
        Point::new(0., 0.),
        Point::new(100., 0.),
        Point::new(100., 5.),
        Point::new(5., 5.),
        Point::new(5., 100.),
        Point::new(0., 100.),
    ];
    let mut outline = Vec::new();
    for side in 0..6 {
        for node in 0..3333 {
            outline.push(corners[side].lerp(corners[(side + 1) % 6], node as f64 / 3333.));
        }
    }
    let mut frame = object(1025, RED, 0.);
    frame.contours = vec![outline];
    project.objects.push(frame);
    let before = object_snapshots(&project);
    batch(&mut project);
    assert_eq!(color_runs(&project), 2);
    assert_eq!(before, object_snapshots(&project));
}

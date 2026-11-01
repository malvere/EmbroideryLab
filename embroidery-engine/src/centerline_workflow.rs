//! Transactional document operation shared by the native editor and CLI.
use crate::{
    Object, Project, Settings, StitchKind,
    centerline::{self, Centerlines},
};

/// Add a visible running copy after the selected area and retain the hidden source.
/// No document changes occur on failure. Isolated pixels remain in the report only.
pub fn running_copy(
    project: &Project,
    selected: usize,
    pixel_size_mm: f64,
) -> Result<(Project, Centerlines), String> {
    project.validate_limits()?;
    let source = project
        .objects
        .get(selected)
        .ok_or("Select an area object first")?;
    if !source.running_paths.is_empty() {
        return Err("Select a filled area; running paths already have centerlines".into());
    }
    let result = centerline::extract(source, pixel_size_mm)?;
    if result.paths.is_empty() {
        return Err(
            "The area produced only isolated points. Use a finer pixel size or a larger area"
                .into(),
        );
    }
    let grid = |point: crate::Point| {
        (
            (point.x * 10.).round() as i64,
            (point.y * 10.).round() as i64,
        )
    };
    if result
        .paths
        .iter()
        .any(|path| path.iter().all(|&p| grid(p) == grid(path[0])))
    {
        return Err("A centerline branch collapses on the 0.1mm export grid. Enlarge the area or use a coarser pixel size".into());
    }
    let id = project
        .objects
        .iter()
        .map(|o| o.id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("Object ID limit reached")?;
    let copy = Object {
        id,
        name: format!("{} · centerlines", source.name),
        running_paths: result.paths.clone(),
        centerline_graph: Some(crate::axis_edit::attach(
            &result.paths,
            source,
            pixel_size_mm,
        )?),
        color: source.color,
        settings: Settings {
            kind: StitchKind::Running,
            length_mm: source.settings.length_mm,
            underlay: false,
            tie_in: source.settings.tie_in,
            tie_off: source.settings.tie_off,
            ..Settings::default()
        },
        ..Object::default()
    };
    let mut candidate = project.clone();
    candidate.version = 4;
    candidate.objects[selected].visible = false;
    candidate.objects.insert(selected + 1, copy);
    candidate.validate_limits()?;
    Ok((candidate, result))
}

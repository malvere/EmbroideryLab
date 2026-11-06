//! Configurable flat-colour raster tracing and solid SVG import.
//! RasterOptions makes resolution, palette, background and contour processing
//! explicit. The old entry point retains 256px/six-colour/near-white defaults.
use crate::model::*;
use std::{collections::BTreeMap, path::Path};

#[path = "raster_options.rs"]
mod raster_options;
#[path = "svg_import.rs"]
mod svg_import;
pub use raster_options::{
    BackgroundMode, MAX_OMITTED_REGION_DETAILS, OmittedRegion, PREPARATION_VERSION,
    PreparationMode, RasterOptions, RasterTraceReport, decode_raster_file, load_image_with_options,
    load_pixels_with_options, load_pixels_with_report, load_pixels_with_report_cancellable,
    load_pixels_with_source_report_cancellable,
};

fn name(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
fn validate_width(width: f64) -> Result<(), String> {
    if !width.is_finite() || !(1.0..=2000.0).contains(&width) {
        Err("Import width must be 1–2000mm".into())
    } else {
        Ok(())
    }
}
fn rgb_distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// Simplify a closed ring with deterministic Ramer–Douglas–Peucker.
pub fn simplify_ring(points: &[Point], tolerance: f64) -> Vec<Point> {
    fn rdp(points: &[Point], tol: f64, out: &mut Vec<Point>) {
        if points.len() < 3 {
            out.extend_from_slice(points);
            return;
        }
        // Iterative subdivision avoids a stack overflow on long raster contours.
        // If the worst-case distance work is exhausted, preserve the boundary.
        let mut keep = vec![false; points.len()];
        keep[0] = true;
        keep[points.len() - 1] = true;
        let mut pending = vec![(0, points.len() - 1)];
        let mut work = 0_usize;
        while let Some((start, end)) = pending.pop() {
            let (a, z) = (points[start], points[end]);
            let (dx, dy) = (z.x - a.x, z.y - a.y);
            let length = dx.hypot(dy);
            let mut farthest = (start, 0.0_f64);
            for (index, p) in points.iter().enumerate().take(end).skip(start + 1) {
                work += 1;
                if work > 5_000_000 {
                    out.extend_from_slice(points);
                    return;
                }
                let distance = if length < 1e-9 {
                    p.distance(a)
                } else {
                    ((p.x - a.x) * dy - (p.y - a.y) * dx).abs() / length
                };
                if distance > farthest.1 {
                    farthest = (index, distance);
                }
            }
            if farthest.1 > tol {
                keep[farthest.0] = true;
                pending.push((farthest.0, end));
                pending.push((start, farthest.0));
            }
        }
        out.extend(
            points
                .iter()
                .zip(keep)
                .filter_map(|(point, kept)| kept.then_some(*point)),
        );
    }
    if points.len() < 4 || !tolerance.is_finite() || tolerance <= 0.0 {
        return points.to_vec();
    }
    let start = points[0];
    let split = (1..points.len())
        .max_by(|&a, &b| {
            points[a]
                .distance(start)
                .total_cmp(&points[b].distance(start))
        })
        .unwrap_or(1);
    let mut first = Vec::new();
    rdp(&points[..=split], tolerance, &mut first);
    let mut second_points = points[split..].to_vec();
    second_points.push(start);
    let mut second = Vec::new();
    rdp(&second_points, tolerance, &mut second);
    first.pop();
    first.extend(second);
    first.pop();
    if first.len() < 3 {
        points.to_vec()
    } else {
        first
    }
}

type Grid = (i32, i32);
fn trace_region(pixels: &[(usize, usize)], member: &[bool], w: usize, h: usize) -> Vec<Vec<Point>> {
    let mut edges: BTreeMap<Grid, Vec<Grid>> = BTreeMap::new();
    let occupied = |x: i32, y: i32| {
        x >= 0
            && y >= 0
            && (x as usize) < w
            && (y as usize) < h
            && member[y as usize * w + x as usize]
    };
    for &(px, py) in pixels {
        let (x, y) = (px as i32, py as i32);
        for (outside, a, b) in [
            (!occupied(x, y - 1), (x, y), (x + 1, y)),
            (!occupied(x + 1, y), (x + 1, y), (x + 1, y + 1)),
            (!occupied(x, y + 1), (x + 1, y + 1), (x, y + 1)),
            (!occupied(x - 1, y), (x, y + 1), (x, y)),
        ] {
            if outside {
                edges.entry(a).or_default().push(b);
            }
        }
    }
    let mut rings = Vec::new();
    while let Some((&start, _)) = edges.first_key_value() {
        let mut current = start;
        let mut direction = (1, 0);
        let mut ring = Vec::new();
        let mut complete = false;
        for _ in 0..(pixels.len() * 4 + 1) {
            ring.push(Point::new(f64::from(current.0), f64::from(current.1)));
            let Some(options) = edges.get_mut(&current) else {
                break;
            };
            let index = options
                .iter()
                .enumerate()
                .max_by_key(|(_, next)| {
                    let d = (next.0 - current.0, next.1 - current.1);
                    let cross = direction.0 * d.1 - direction.1 * d.0;
                    let dot = direction.0 * d.0 + direction.1 * d.1;
                    if cross > 0 {
                        3
                    } else if dot > 0 {
                        2
                    } else if cross < 0 {
                        1
                    } else {
                        0
                    }
                })
                .map(|(i, _)| i)
                .unwrap_or(0);
            let next = options.swap_remove(index);
            let empty = options.is_empty();
            if empty {
                edges.remove(&current);
            }
            direction = (next.0 - current.0, next.1 - current.1);
            current = next;
            if current == start {
                complete = true;
                break;
            }
        }
        if complete && ring.len() >= 3 {
            // Four-connected raster regions can have diagonal void contacts.
            // Their boundary is a closed walk which revisits a grid vertex,
            // not a simple polygon ring. Decompose it into simple cycles while
            // retaining every original directed edge and even-odd occupancy.
            // A hull, epsilon offset or dropped loop would change the artwork.
            if let Some(cycles) = split_grid_walk(&ring) {
                rings.extend(cycles);
            } else {
                // Retain an uncertifiable walk for the explicit geometry
                // diagnostic rather than silently dropping a boundary.
                rings.push(ring);
            }
        }
    }
    rings
}

fn split_grid_walk(ring: &[Point]) -> Option<Vec<Vec<Point>>> {
    let mut path = Vec::<Grid>::new();
    let mut positions = BTreeMap::<Grid, usize>::new();
    let mut cycles = Vec::new();
    for point in ring.iter().chain(ring.first()) {
        let vertex = (point.x as i32, point.y as i32);
        if let Some(&start) = positions.get(&vertex) {
            if path.len() - start < 3 {
                return None;
            }
            cycles.push(
                path[start..]
                    .iter()
                    .map(|&(x, y)| Point::new(f64::from(x), f64::from(y)))
                    .collect(),
            );
            for old in &path[start + 1..] {
                positions.remove(old);
            }
            path.truncate(start + 1);
        } else {
            positions.insert(vertex, path.len());
            path.push(vertex);
        }
    }
    (path.len() == 1 && !cycles.is_empty()).then_some(cycles)
}

pub fn load_image(path: &Path, width_mm: f64) -> Result<Project, String> {
    load_image_with_options(path, width_mm, &RasterOptions::default())
}
pub fn load_svg(path: &Path, width_mm: f64) -> Result<Project, String> {
    svg_import::load(path, width_mm)
}

/// Dispatch import of an editable project, raster artwork or SVG.
pub fn load(path: &Path, width_mm: f64) -> Result<Project, String> {
    match path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase()
        .as_str()
    {
        "svg" => load_svg(path, width_mm),
        "json" | "elab" => Project::load(path),
        "png" | "jpg" | "jpeg" | "webp" => load_image(path, width_mm),
        "pdf" => {
            let page = crate::pdf_import::render_page(path, 0, 2048)?;
            load_pixels_with_options(&page.image, path, width_mm, &RasterOptions::default())
        }
        _ => Err("Supported editable inputs: JPEG, PNG, WebP, PDF, SVG, ELAB/JSON".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ring_preserves_rectangle() {
        let ring = vec![
            Point::new(0., 0.),
            Point::new(1., 0.),
            Point::new(2., 0.),
            Point::new(2., 2.),
            Point::new(0., 2.),
        ];
        assert_eq!(simplify_ring(&ring, 0.01).len(), 4);
    }
    #[test]
    fn raster_boundary_preserves_hole() {
        let (w, h) = (5, 5);
        let mut member = vec![true; w * h];
        member[12] = false;
        let pixels = (0..w * h)
            .filter(|&i| member[i])
            .map(|i| (i % w, i / w))
            .collect::<Vec<_>>();
        let rings = trace_region(&pixels, &member, w, h);
        assert_eq!(rings.len(), 2);
    }
}

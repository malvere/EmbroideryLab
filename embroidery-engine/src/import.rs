//! Configurable flat-colour raster tracing and solid SVG import.
//! RasterOptions makes resolution, palette, background and contour processing
//! explicit. The old entry point retains 256px/six-colour/near-white defaults.
use crate::model::*;
use std::{collections::BTreeMap, path::Path};

#[path = "raster_options.rs"]
mod raster_options;
pub use raster_options::{BackgroundMode, RasterOptions, load_image_with_options};

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
fn nearest(color: [f64; 3], palette: &[[f64; 3]]) -> usize {
    palette
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| rgb_distance(color, **a).total_cmp(&rgb_distance(color, **b)))
        .map_or(0, |(i, _)| i)
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
            rings.push(ring);
        }
    }
    rings
}

pub fn load_image(path: &Path, width_mm: f64) -> Result<Project, String> {
    load_image_with_options(path, width_mm, &RasterOptions::default())
}
pub fn load_svg(path: &Path, width_mm: f64) -> Result<Project, String> {
    validate_width(width_mm)?;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > 20_000_000 {
        return Err("SVG exceeds 20MB import limit".into());
    }
    let text =
        std::str::from_utf8(&bytes).map_err(|_| "SVG import requires uncompressed UTF-8 XML")?;
    let xml = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    if xml
        .descendants()
        .any(|n| n.is_element() && matches!(n.tag_name().name(), "text" | "image" | "use"))
    {
        return Err("SVG text, embedded images and reused symbols must be converted to explicit flat paths first".into());
    }
    let tree =
        usvg::Tree::from_data(&bytes, &usvg::Options::default()).map_err(|e| e.to_string())?;
    let scale = width_mm / f64::from(tree.size().width());
    let mut objects = Vec::new();
    let mut unsupported = Vec::new();
    fn walk(
        group: &usvg::Group,
        scale: f64,
        objects: &mut Vec<Object>,
        unsupported: &mut Vec<String>,
        node_budget: &mut usize,
    ) {
        use usvg::tiny_skia_path::PathSegment;
        for node in group.children() {
            if *node_budget >= 100_000 || objects.len() >= 4096 {
                unsupported.push("SVG geometry complexity limit".into());
                return;
            }
            match node {
                usvg::Node::Group(g) => {
                    if g.opacity().get() < 0.999 || g.blend_mode() != usvg::BlendMode::Normal {
                        unsupported.push("group opacity or blend mode".into());
                    }
                    if g.clip_path().is_some() || g.mask().is_some() || !g.filters().is_empty() {
                        unsupported.push("clipping, masks or filters".into());
                    }
                    walk(g, scale, objects, unsupported, node_budget);
                }
                usvg::Node::Path(path) => {
                    if !path.is_visible() {
                        continue;
                    }
                    let (color, kind) = if let Some(fill) = path.fill() {
                        if fill.opacity().get() < 0.999 {
                            unsupported.push("partial opacity".into());
                        }
                        if path.stroke().is_some() {
                            unsupported.push("combined fill and stroke".into());
                        }
                        match fill.paint() {
                            usvg::Paint::Color(c) => ([c.red, c.green, c.blue], StitchKind::Tatami),
                            _ => {
                                unsupported.push("gradient or pattern fill".into());
                                continue;
                            }
                        }
                    } else if path.stroke().is_some() {
                        unsupported.push("stroke-only paths (convert stroke to outline)".into());
                        continue;
                    } else {
                        continue;
                    };
                    let Some(data) = path.data().clone().transform(path.abs_transform()) else {
                        continue;
                    };
                    let mut rings = Vec::new();
                    let mut ring = Vec::new();
                    let mut current = Point::default();
                    let mut start = Point::default();
                    let convert = |p: usvg::tiny_skia_path::Point| {
                        Point::new(f64::from(p.x) * scale, f64::from(p.y) * scale)
                    };
                    for segment in data.segments() {
                        let starts_contour = matches!(&segment, PathSegment::MoveTo(_));
                        let before = ring.len();
                        match segment {
                            PathSegment::MoveTo(p) => {
                                if ring.len() >= 3 {
                                    rings.push(std::mem::take(&mut ring));
                                } else {
                                    ring.clear();
                                }
                                current = convert(p);
                                start = current;
                                ring.push(current);
                            }
                            PathSegment::LineTo(p) => {
                                current = convert(p);
                                ring.push(current);
                            }
                            PathSegment::QuadTo(c, e) => {
                                let c = convert(c);
                                let e = convert(e);
                                let curve = CubicBezier {
                                    start: current,
                                    control1: current.lerp(c, 2. / 3.),
                                    control2: e.lerp(c, 2. / 3.),
                                    end: e,
                                };
                                ring.extend(curve.flatten(0.03).into_iter().skip(1));
                                current = e;
                            }
                            PathSegment::CubicTo(c1, c2, e) => {
                                let e = convert(e);
                                let curve = CubicBezier {
                                    start: current,
                                    control1: convert(c1),
                                    control2: convert(c2),
                                    end: e,
                                };
                                ring.extend(curve.flatten(0.03).into_iter().skip(1));
                                current = e;
                            }
                            PathSegment::Close => {
                                if ring.last() == Some(&start) {
                                    ring.pop();
                                }
                                if ring.len() >= 3 {
                                    rings.push(std::mem::take(&mut ring));
                                }
                                current = start;
                            }
                        }
                        *node_budget += if starts_contour {
                            1
                        } else {
                            ring.len().saturating_sub(before)
                        };
                        if *node_budget > 100_000 {
                            unsupported.push("SVG exceeds 100,000 flattened nodes".into());
                            return;
                        }
                    }
                    if ring.len() >= 3 {
                        rings.push(ring);
                    }
                    if rings.is_empty() {
                        continue;
                    }
                    if path
                        .fill()
                        .is_some_and(|f| f.rule() == usvg::FillRule::NonZero)
                    {
                        if rings.len() > 256 {
                            unsupported.push("nonzero path exceeds 256 subpaths".into());
                            return;
                        }
                        fn contains(p: Point, r: &[Point]) -> bool {
                            let mut inside = false;
                            for (a, b) in r.iter().zip(r.iter().cycle().skip(1)).take(r.len()) {
                                if (a.y > p.y) != (b.y > p.y)
                                    && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x
                                {
                                    inside = !inside;
                                }
                            }
                            inside
                        }
                        let signs: Vec<i32> = rings
                            .iter()
                            .map(|r| {
                                let area = r
                                    .iter()
                                    .zip(r.iter().cycle().skip(1))
                                    .take(r.len())
                                    .map(|(a, b)| a.x * b.y - b.x * a.y)
                                    .sum::<f64>();
                                if area >= 0. { 1 } else { -1 }
                            })
                            .collect();
                        // For nested nonzero rings, keep only occupancy boundaries;
                        // same-winding nested contours are solid, not holes.
                        let keep: Vec<bool> = rings
                            .iter()
                            .enumerate()
                            .map(|(i, r)| {
                                let before = rings
                                    .iter()
                                    .enumerate()
                                    .filter(|(j, other)| *j != i && contains(r[0], other))
                                    .map(|(j, _)| signs[j])
                                    .sum::<i32>();
                                (before == 0) != (before + signs[i] == 0)
                            })
                            .collect();
                        rings = rings
                            .into_iter()
                            .zip(keep)
                            .filter_map(|(r, keep)| keep.then_some(r))
                            .collect();
                        if rings.is_empty() {
                            continue;
                        }
                    }
                    let id = objects.len() as u64 + 1;
                    let settings = Settings {
                        kind,
                        underlay: kind != StitchKind::Running,
                        ..Settings::default()
                    };
                    objects.push(Object {
                        id,
                        name: if path.id().is_empty() {
                            format!("SVG path {id}")
                        } else {
                            path.id().into()
                        },
                        contours: rings,
                        color,
                        settings,
                        ..Object::default()
                    });
                }
                _ => unsupported.push("text or embedded raster image".into()),
            }
        }
    }
    walk(tree.root(), scale, &mut objects, &mut unsupported, &mut 0);
    if !unsupported.is_empty() {
        unsupported.sort();
        unsupported.dedup();
        return Err(format!(
            "SVG contains unsupported {}; convert text/strokes to flat solid outlines first",
            unsupported.join(", ")
        ));
    }
    if objects.is_empty() {
        return Err("SVG contains no supported solid vector paths".into());
    }
    Ok(Project {
        name: name(path),
        objects,
        width_mm,
        ..Project::default()
    })
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
        "png" | "jpg" | "jpeg" => load_image(path, width_mm),
        _ => Err("Supported editable inputs: JPG, PNG, SVG, ELAB/JSON".into()),
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

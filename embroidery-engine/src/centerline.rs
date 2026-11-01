//! Independent raster-to-centerline preprocessing for embroidery geometry.
//!
//! This module rasterizes an object's even-odd filled area, invokes the binary
//! thinning kernel and traces each skeleton graph edge once. Paths use pixel
//! centers in millimetres; exactly collinear runs omit redundant intermediate
//! pixels without moving the polyline. They are not satin rails or a stitch-order
//! solution.
use crate::{Object, Point, thinning::BinaryMask};
use serde::Serialize;

const MAX_AXIS: usize = 2048;
const MAX_NODES: usize = 100_000;
const MAX_EDGE_ROWS: usize = 20_000_000;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Centerlines {
    /// Open branches; a closed cycle explicitly repeats its first point.
    pub paths: Vec<Vec<Point>>,
    /// Single pixels have no direction and are not converted into stitches.
    pub isolated_points: Vec<Point>,
    pub pixel_size_mm: f64,
    pub foreground_pixels: usize,
    pub skeleton_pixels: usize,
}

/// Extract the digital centerlines of one filled object without changing it.
///
/// Features thinner than one pixel can disappear during rasterization. The
/// caller selects the resolution explicitly; empty rasterizations are errors.
pub fn extract(object: &Object, pixel_size_mm: f64) -> Result<Centerlines, String> {
    if !pixel_size_mm.is_finite() || !(0.005..=10.0).contains(&pixel_size_mm) {
        return Err("Centerline pixel size must be between 0.005 and 10 mm".into());
    }
    let source_nodes = object
        .contours
        .iter()
        .map(Vec::len)
        .sum::<usize>()
        .saturating_add(
            object
                .satin_column
                .as_ref()
                .map_or(0, |c| c.left.len().saturating_add(c.right.len())),
        );
    if source_nodes > MAX_NODES || object.bezier_segments.len() > 512 {
        return Err("Centerline source exceeds the geometry budget".into());
    }
    let valid = |p: &Point| {
        p.x.is_finite() && p.y.is_finite() && p.x.abs() <= 9990.0 && p.y.abs() <= 9990.0
    };
    if object.contours.iter().flatten().any(|p| !valid(p))
        || object
            .satin_column
            .as_ref()
            .is_some_and(|c| c.left.iter().chain(&c.right).any(|p| !valid(p)))
        || object.bezier_segments.iter().any(|c| {
            [c.start, c.control1, c.control2, c.end]
                .iter()
                .any(|p| !valid(p))
        })
    {
        return Err("Centerline source contains invalid coordinates".into());
    }
    let contours = object.flattened_contours();
    if contours.iter().map(Vec::len).sum::<usize>() > MAX_NODES {
        return Err("Flattened centerline source exceeds 100,000 nodes".into());
    }
    let rings: Vec<_> = contours.iter().filter(|ring| ring.len() >= 3).collect();
    if rings.is_empty() {
        return Err("Select a filled contour, Bezier shape or satin column".into());
    }
    let mut min = Point::new(f64::INFINITY, f64::INFINITY);
    let mut max = Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in rings.iter().flat_map(|ring| ring.iter()) {
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        max.x = max.x.max(p.x);
        max.y = max.y.max(p.y);
    }
    let columns = ((max.x - min.x) / pixel_size_mm).ceil();
    let rows = ((max.y - min.y) / pixel_size_mm).ceil();
    if columns < 1.0 || rows < 1.0 {
        return Err("Centerline source has no filled area".into());
    }
    if columns > (MAX_AXIS - 2) as f64 || rows > (MAX_AXIS - 2) as f64 {
        return Err("Centerline raster exceeds 2048 pixels; increase pixel size".into());
    }
    let width = columns as usize + 2;
    let height = rows as usize + 2;
    let origin = Point::new(min.x - pixel_size_mm, min.y - pixel_size_mm);
    let mut intersections = vec![Vec::<f64>::new(); height];
    let mut edge_rows = 0_usize;
    for ring in rings {
        for (a, b) in ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
        {
            if (a.y - b.y).abs() < f64::EPSILON {
                continue;
            }
            // Half-open edge rule: low endpoint included, high excluded.
            let first =
                (((a.y.min(b.y) - origin.y) / pixel_size_mm - 0.5).ceil() as usize).min(height);
            let end =
                (((a.y.max(b.y) - origin.y) / pixel_size_mm - 0.5).ceil() as usize).min(height);
            edge_rows = edge_rows.saturating_add(end.saturating_sub(first));
            if edge_rows > MAX_EDGE_ROWS {
                return Err("Centerline rasterization exceeds the geometry work budget".into());
            }
            for (y, row) in intersections.iter_mut().enumerate().take(end).skip(first) {
                let py = origin.y + (y as f64 + 0.5) * pixel_size_mm;
                row.push(a.x + (py - a.y) * (b.x - a.x) / (b.y - a.y));
            }
        }
    }
    let mut pixels = vec![0_u8; width * height];
    for (y, row) in intersections.iter_mut().enumerate() {
        row.sort_by(f64::total_cmp);
        if row.len() % 2 != 0 {
            return Err("Centerline source has unpaired scanline intersections".into());
        }
        for pair in row.as_chunks::<2>().0 {
            let first = (((pair[0] - origin.x) / pixel_size_mm - 0.5).ceil() as usize).min(width);
            let end = (((pair[1] - origin.x) / pixel_size_mm - 0.5).ceil() as usize).min(width);
            pixels[y * width + first..y * width + end].fill(1);
        }
    }
    let foreground_pixels = pixels.iter().filter(|&&p| p != 0).count();
    if foreground_pixels == 0 {
        return Err("No foreground pixels at this resolution; use a smaller pixel size".into());
    }
    let skeleton = crate::thinning::skeletonize(&BinaryMask {
        width,
        height,
        pixels,
    })?;
    let (paths, isolated_points) = trace(&skeleton, origin, pixel_size_mm)?;
    Ok(Centerlines {
        paths,
        isolated_points,
        pixel_size_mm,
        foreground_pixels,
        skeleton_pixels: skeleton.pixels.iter().filter(|&&p| p != 0).count(),
    })
}

// Clockwise from north. The opposite edge has direction (d + 4) % 8.
const DIRECTIONS: [(isize, isize); 8] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];

fn neighbors(mask: &BinaryMask, index: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
    let x = index % mask.width;
    let y = index / mask.width;
    DIRECTIONS
        .iter()
        .enumerate()
        .filter_map(move |(d, &(dx, dy))| {
            let nx = x.checked_add_signed(dx)?;
            let ny = y.checked_add_signed(dy)?;
            if nx >= mask.width || ny >= mask.height {
                return None;
            }
            let other = ny * mask.width + nx;
            if mask.pixels[other] == 0 {
                return None;
            }
            // An orthogonal two-step route replaces the redundant diagonal edge.
            // This avoids spurious triangles at bends while retaining diagonal lines.
            if dx != 0
                && dy != 0
                && (mask.pixels[y * mask.width + nx] != 0 || mask.pixels[ny * mask.width + x] != 0)
            {
                return None;
            }
            Some((d, other))
        })
}

fn trace(
    mask: &BinaryMask,
    origin: Point,
    pixel_size_mm: f64,
) -> Result<(Vec<Vec<Point>>, Vec<Point>), String> {
    let active: Vec<_> = mask
        .pixels
        .iter()
        .enumerate()
        .filter_map(|(i, &p)| (p != 0).then_some(i))
        .collect();
    if active.len() > MAX_NODES {
        return Err("Centerline skeleton exceeds 100,000 pixels; increase pixel size".into());
    }
    let position = |i: usize| {
        Point::new(
            origin.x + (i % mask.width) as f64 * pixel_size_mm + pixel_size_mm * 0.5,
            origin.y + (i / mask.width) as f64 * pixel_size_mm + pixel_size_mm * 0.5,
        )
    };
    let mut degree = vec![0_u8; mask.pixels.len()];
    for &i in &active {
        degree[i] = neighbors(mask, i).count() as u8;
    }
    let isolated_points: Vec<_> = active
        .iter()
        .filter(|&&i| degree[i] == 0)
        .map(|&i| position(i))
        .collect();
    let mut visited = vec![0_u8; mask.pixels.len()];
    let mut paths = Vec::new();
    let mut total_nodes = isolated_points.len();
    // First trace all junction-to-junction branches, then remaining closed cycles.
    for cycles in [false, true] {
        for &start in &active {
            if (degree[start] == 2) != cycles {
                continue;
            }
            for (direction, next) in neighbors(mask, start) {
                if visited[start] & (1 << direction) != 0 {
                    continue;
                }
                let mut path = vec![position(start)];
                total_nodes += 1;
                let mut previous = start;
                let mut current = next;
                let mut d = direction;
                let mut previous_direction = None;
                loop {
                    visited[previous] |= 1 << d;
                    visited[current] |= 1 << ((d + 4) % 8);
                    // Integer pixel directions make this compression exact;
                    // all bends, endpoints and junction positions remain.
                    if previous_direction == Some(d) {
                        *path.last_mut().expect("path contains its start") = position(current);
                    } else {
                        total_nodes += 1;
                        if total_nodes > MAX_NODES {
                            return Err(
                                "Centerline paths exceed 100,000 nodes; increase pixel size".into(),
                            );
                        }
                        path.push(position(current));
                    }
                    previous_direction = Some(d);
                    if degree[current] != 2 || current == start {
                        break;
                    }
                    let Some((nd, ni)) = neighbors(mask, current)
                        .find(|&(nd, ni)| ni != previous && visited[current] & (1 << nd) == 0)
                    else {
                        break;
                    };
                    previous = current;
                    current = ni;
                    d = nd;
                }
                paths.push(path);
            }
        }
    }
    Ok((paths, isolated_points))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(rows: &[&str]) -> Centerlines {
        let mask = BinaryMask {
            width: rows[0].len(),
            height: rows.len(),
            pixels: rows
                .iter()
                .flat_map(|row| row.bytes().map(|c| u8::from(c == b'#')))
                .collect(),
        };
        let (paths, isolated_points) = trace(&mask, Point::new(0.0, 0.0), 1.0).unwrap();
        Centerlines {
            paths,
            isolated_points,
            pixel_size_mm: 1.0,
            foreground_pixels: 0,
            skeleton_pixels: 0,
        }
    }

    #[test]
    fn bends_do_not_create_diagonal_triangle_or_close_path() {
        let result = graph(&["#..", "#..", "###"]);
        assert_eq!(result.paths.len(), 1);
        assert_eq!(
            result.paths[0],
            vec![
                Point::new(0.5, 0.5),
                Point::new(0.5, 2.5),
                Point::new(2.5, 2.5)
            ]
        );
        assert_ne!(result.paths[0].first(), result.paths[0].last());
    }

    #[test]
    fn traces_branches_once_and_preserves_isolated_points() {
        let result = graph(&["..#..", "..#..", "#####", ".....", "#...."]);
        assert_eq!(result.paths.len(), 3);
        assert_eq!(
            result
                .paths
                .iter()
                .map(|p| p[0].distance(p[1]))
                .sum::<f64>(),
            6.0
        );
        assert_eq!(result.isolated_points, vec![Point::new(0.5, 4.5)]);
        assert!(result.paths.iter().all(|p| p.first() != p.last()));
    }

    #[test]
    fn cycle_repeats_start_and_every_edge_is_covered_once() {
        let result = graph(&["###", "#.#", "###"]);
        assert_eq!(result.paths.len(), 1);
        assert_eq!(result.paths[0].len(), 5);
        assert_eq!(
            result.paths[0]
                .windows(2)
                .map(|p| p[0].distance(p[1]))
                .sum::<f64>(),
            8.0
        );
        assert_eq!(result.paths[0].first(), result.paths[0].last());
    }

    #[test]
    fn diagonal_component_remains_one_open_path() {
        let result = graph(&["#...", ".#..", "..#.", "...#"]);
        assert_eq!(result.paths.len(), 1);
        assert_eq!(result.paths[0].len(), 2);
        assert_eq!(result.paths[0][0], Point::new(0.5, 0.5));
        assert_eq!(result.paths[0][1], Point::new(3.5, 3.5));
    }
}

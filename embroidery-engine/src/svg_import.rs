//! Solid SVG paint becomes editable millimetre regions. Stroke outlines are
//! constructed by the installed SVG/path stack; the sewing engine is unchanged.
use super::{name, validate_width};
use crate::model::{CubicBezier, Object, Point, Project, Settings};
use geo::algorithm::bool_ops::{FillRule, OpType};
use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon, Validation};
use std::{collections::BTreeSet, io::Read, path::Path};
use usvg::tiny_skia_path::{Path as VectorPath, PathSegment};

#[path = "svg_references.rs"]
mod references;

const MAX_NODES: usize = 100_000;
const MAX_OBJECTS: usize = 4096;
const MAX_BOOLEAN_OPERATION: usize = 50_000_000;
const MAX_BOOLEAN_WORK: usize = 250_000_000;
const FLATTEN_MM: f64 = 0.03;

#[derive(Default)]
struct Budget {
    source: usize,
    flattened: usize,
    output: usize,
    boolean_work: usize,
    dash_spans: usize,
    stroke_work: usize,
}

impl Budget {
    fn add(counter: &mut usize, amount: usize, phase: &str) -> Result<(), String> {
        *counter = counter.saturating_add(amount);
        if *counter > MAX_NODES {
            Err(format!(
                "SVG exceeds 100,000 {phase} nodes; simplify or split the artwork"
            ))
        } else {
            Ok(())
        }
    }

    fn normalize(&mut self, count: usize) -> Result<(), String> {
        // Bound crossing-heavy input before allocating a Boolean arrangement.
        let work = count.saturating_mul(count);
        self.boolean_work = self.boolean_work.saturating_add(work);
        if work > MAX_BOOLEAN_OPERATION || self.boolean_work > MAX_BOOLEAN_WORK {
            return Err("SVG fill normalization exceeds the geometry work budget; simplify or split the artwork".into());
        }
        Ok(())
    }
}

fn flatten(
    data: &VectorPath,
    transform: usvg::Transform,
    scale: f64,
    tolerance: f64,
    budget: &mut Budget,
) -> Result<Vec<Vec<Point>>, String> {
    let data = data
        .clone()
        .transform(transform)
        .ok_or("SVG path transform cannot be represented safely")?;
    let convert = |p: usvg::tiny_skia_path::Point| {
        let point = Point::new(f64::from(p.x) * scale, f64::from(p.y) * scale);
        if !point.x.is_finite()
            || !point.y.is_finite()
            || point.x.abs() > 9990.
            || point.y.abs() > 9990.
        {
            Err("SVG geometry exceeds the finite ±9990mm working area".to_string())
        } else {
            Ok(point)
        }
    };
    let mut rings = Vec::new();
    let mut ring = Vec::new();
    let mut current = Point::default();
    let mut start = Point::default();
    let finish = |ring: &mut Vec<Point>, rings: &mut Vec<Vec<Point>>| {
        ring.dedup();
        if ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        if ring.len() >= 3 {
            rings.push(std::mem::take(ring));
        } else {
            ring.clear();
        }
    };
    for segment in data.segments() {
        let points = match segment {
            PathSegment::MoveTo(p) => {
                finish(&mut ring, &mut rings);
                current = convert(p)?;
                start = current;
                vec![current]
            }
            PathSegment::LineTo(p) => {
                current = convert(p)?;
                vec![current]
            }
            PathSegment::QuadTo(c, end) => {
                let c = convert(c)?;
                let end = convert(end)?;
                let points = CubicBezier {
                    start: current,
                    control1: current.lerp(c, 2. / 3.),
                    control2: end.lerp(c, 2. / 3.),
                    end,
                }
                .flatten(tolerance)
                .into_iter()
                .skip(1)
                .collect();
                current = end;
                points
            }
            PathSegment::CubicTo(c1, c2, end) => {
                let end = convert(end)?;
                let points = CubicBezier {
                    start: current,
                    control1: convert(c1)?,
                    control2: convert(c2)?,
                    end,
                }
                .flatten(tolerance)
                .into_iter()
                .skip(1)
                .collect();
                current = end;
                points
            }
            PathSegment::Close => {
                finish(&mut ring, &mut rings);
                current = start;
                Vec::new()
            }
        };
        Budget::add(&mut budget.flattened, points.len(), "flattened")?;
        ring.extend(points);
    }
    // SVG fills implicitly close open subpaths. Stroke outlines are closed.
    finish(&mut ring, &mut rings);
    Ok(rings)
}

fn normalize(
    rings: Vec<Vec<Point>>,
    rule: FillRule,
    budget: &mut Budget,
) -> Result<Vec<Vec<Point>>, String> {
    if rings.is_empty() {
        return Ok(Vec::new());
    }
    budget.normalize(rings.iter().map(Vec::len).sum())?;
    // Each raw SVG contour is supplied as an exterior. The explicit crossing
    // rule, rather than a nested-ring heuristic, determines occupancy, including
    // overlapping and self-crossing subpaths. Union with empty normalizes it.
    let raw = MultiPolygon::new(
        rings
            .into_iter()
            .map(|ring| {
                Polygon::new(
                    LineString::new(ring.into_iter().map(|p| Coord { x: p.x, y: p.y }).collect()),
                    Vec::new(),
                )
            })
            .collect(),
    );
    let area =
        raw.boolean_op_with_fill_rule(&MultiPolygon::<f64>::new(Vec::new()), OpType::Union, rule);
    area.check_validation().map_err(|e| {
        format!("SVG fill could not be normalized into valid contours: {e}; simplify the artwork")
    })?;
    let mut result = Vec::new();
    for polygon in area.0 {
        for line in std::iter::once(polygon.exterior()).chain(polygon.interiors()) {
            let mut ring: Vec<_> = line.0.iter().map(|c| Point::new(c.x, c.y)).collect();
            if ring.first() == ring.last() {
                ring.pop();
            }
            if ring.len() < 3 {
                return Err("SVG normalization produced a degenerate contour".into());
            }
            if ring.iter().any(|p| {
                !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 9990. || p.y.abs() > 9990.
            }) {
                return Err("SVG geometry exceeds the finite ±9990mm working area".into());
            }
            // Stable anchors retain intuitive minimum-corner placement and do
            // not depend on a Boolean kernel's chosen start vertex.
            let anchor = (0..ring.len())
                .min_by(|&a, &b| {
                    ring[a]
                        .x
                        .total_cmp(&ring[b].x)
                        .then(ring[a].y.total_cmp(&ring[b].y))
                })
                .unwrap();
            ring.rotate_left(anchor);
            Budget::add(&mut budget.output, ring.len(), "output")?;
            result.push(ring);
        }
    }
    Ok(result)
}

fn paint_color(paint: &usvg::Paint, opacity: f32) -> Result<[u8; 3], String> {
    if opacity < 0.999 {
        return Err("SVG contains unsupported partial opacity; use opaque solid paints".into());
    }
    match paint {
        usvg::Paint::Color(c) => Ok([c.red, c.green, c.blue]),
        _ => Err(
            "SVG contains unsupported gradient or pattern paint; use opaque solid colors".into(),
        ),
    }
}

fn control_length(data: &VectorPath) -> (f64, usize) {
    let mut current = Point::default();
    let mut start = current;
    let mut length = 0.;
    let mut contours = 0;
    let point = |p: usvg::tiny_skia_path::Point| Point::new(f64::from(p.x), f64::from(p.y));
    for segment in data.segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                current = point(p);
                start = current;
                contours += 1;
            }
            PathSegment::LineTo(p) => {
                let p = point(p);
                length += current.distance(p);
                current = p;
            }
            PathSegment::QuadTo(c, p) => {
                let (c, p) = (point(c), point(p));
                length += current.distance(c) + c.distance(p);
                current = p;
            }
            PathSegment::CubicTo(a, b, p) => {
                let (a, b, p) = (point(a), point(b), point(p));
                length += current.distance(a) + a.distance(b) + b.distance(p);
                current = p;
            }
            PathSegment::Close => {
                length += current.distance(start);
                current = start;
            }
        }
    }
    (length, contours)
}

fn stroke_outline(
    path: &usvg::Path,
    scale: f64,
    budget: &mut Budget,
) -> Result<VectorPath, String> {
    let stroke = path.stroke().unwrap();
    let transform = path.abs_transform();
    // Frobenius norm conservatively bounds magnification under skew/nonuniform
    // scale. Outline and final curve flattening each receive half the mm budget.
    let magnification = f64::from(transform.sx)
        .hypot(f64::from(transform.kx))
        .hypot(f64::from(transform.ky))
        .hypot(f64::from(transform.sy))
        * scale;
    let resolution = magnification / (4. * (FLATTEN_MM / 2.));
    if !resolution.is_finite() || resolution <= 0. || resolution > 1_000_000. {
        return Err(
            "SVG stroke transform exceeds the supported precision budget; simplify its transform"
                .into(),
        );
    }
    let (length, contours) = control_length(path.data());
    let estimate = (length * magnification / (FLATTEN_MM / 2.)).ceil();
    if !estimate.is_finite() || estimate > 2_000_000. {
        return Err(
            "SVG stroke expansion exceeds the geometry work budget; simplify or split the artwork"
                .into(),
        );
    }
    budget.stroke_work = budget
        .stroke_work
        .saturating_add(estimate as usize)
        .saturating_add(path.data().len().saturating_mul(8));
    if budget.stroke_work > 8_000_000 {
        return Err(
            "SVG stroke expansion exceeds the geometry work budget; simplify or split the artwork"
                .into(),
        );
    }
    let style = stroke.to_tiny_skia();
    let dashed;
    let data = if let Some(dash) = &style.dash {
        let array = stroke.dasharray().unwrap();
        let period = array.iter().map(|&n| f64::from(n)).sum::<f64>();
        let spans =
            ((length / period).ceil() + 1.) * array.len().div_ceil(2) as f64 + contours as f64;
        if !spans.is_finite() || spans > 8192. {
            return Err("SVG stroke dash pattern exceeds 8192 spans; increase dash lengths or simplify the artwork".into());
        }
        budget.dash_spans = budget.dash_spans.saturating_add(spans as usize);
        if budget.dash_spans > 8192 {
            return Err("SVG stroke dash pattern exceeds 8192 spans; increase dash lengths or simplify the artwork".into());
        }
        dashed = path
            .data()
            .dash(dash, resolution as f32)
            .ok_or("SVG stroke dash pattern cannot be expanded safely")?;
        &dashed
    } else {
        path.data()
    };
    let outline = data
        .stroke(&style, resolution as f32)
        .ok_or("SVG stroke could not be converted to a valid outline")?;
    if outline.len() > MAX_NODES {
        return Err(
            "SVG stroke outline exceeds 100,000 nodes; simplify or split the artwork".into(),
        );
    }
    Ok(outline)
}

fn add_object(
    path: &usvg::Path,
    rings: Vec<Vec<Point>>,
    color: [u8; 3],
    role: &str,
    combined: bool,
    objects: &mut Vec<Object>,
) -> Result<(), String> {
    if rings.is_empty() {
        return Ok(());
    }
    if objects.len() >= MAX_OBJECTS {
        return Err("SVG exceeds 4096 painted objects; simplify or split the artwork".into());
    }
    let id = objects.len() as u64 + 1;
    let base = if path.id().is_empty() {
        format!("SVG path {id}")
    } else {
        path.id().into()
    };
    objects.push(Object {
        id,
        name: if combined {
            format!("{base} · {role}")
        } else {
            base
        },
        contours: rings,
        color,
        settings: Settings::default(),
        ..Object::default()
    });
    Ok(())
}

// usvg computes stroked bounds using a coarser outline than this importer.
// A redundant-clip proof therefore accepts only stroke-free subtrees.
fn subtree_has_strokes(group: &usvg::Group) -> bool {
    group.children().iter().any(|node| match node {
        usvg::Node::Group(child) => subtree_has_strokes(child),
        usvg::Node::Path(path) => path.is_visible() && path.stroke().is_some(),
        _ => false,
    })
}

fn redundant_viewport_clip(group: &usvg::Group, authored_clips: &BTreeSet<String>) -> bool {
    let Some(clip) = group.clip_path() else {
        return true;
    };
    if subtree_has_strokes(group)
        || authored_clips.contains(clip.id())
        || clip.clip_path().is_some()
        || !clip.transform().is_identity()
        || !clip.root().transform().is_identity()
        || clip.root().children().len() != 1
    {
        return false;
    }
    let usvg::Node::Path(path) = &clip.root().children()[0] else {
        return false;
    };
    if !path.is_visible() || path.stroke().is_some() || !path.abs_transform().is_identity() {
        return false;
    }
    let Some(fill) = path.fill() else {
        return false;
    };
    if fill.opacity().get() != 1. || !matches!(fill.paint(), usvg::Paint::Color(_)) {
        return false;
    }
    let mut points = Vec::new();
    let mut closed = false;
    for segment in path.data().segments() {
        match segment {
            PathSegment::MoveTo(p) if points.is_empty() => points.push(p),
            PathSegment::LineTo(p) if !points.is_empty() && !closed => points.push(p),
            PathSegment::Close if !closed => closed = true,
            _ => return false,
        }
    }
    if points.first() == points.last() {
        points.pop();
    }
    if !closed || points.len() != 4 {
        return false;
    }
    let min_x = points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let max_x = points.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
    let min_y = points.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let max_y = points.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
    if min_x >= max_x
        || min_y >= max_y
        || points
            .iter()
            .any(|p| !(p.x == min_x || p.x == max_x) || !(p.y == min_y || p.y == max_y))
        || points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .take(4)
            .any(|(a, b)| a == b || (a.x != b.x && a.y != b.y))
    {
        return false;
    }
    // usvg stores this conservative, UNCLIPPED painted bound in group-local
    // coordinates. Symbol viewport rectangles use those same coordinates.
    // Inclusive containment allows boundary contact; any real clipping remains
    // unsupported instead of silently throwing away artwork outside a viewport.
    let painted = group.stroke_bounding_box();
    painted.left() >= min_x
        && painted.top() >= min_y
        && painted.right() <= max_x
        && painted.bottom() <= max_y
}

fn walk(
    group: &usvg::Group,
    scale: f64,
    budget: &mut Budget,
    objects: &mut Vec<Object>,
    authored_clips: &BTreeSet<String>,
) -> Result<(), String> {
    for node in group.children() {
        match node {
            usvg::Node::Group(group) => {
                if group.opacity().get() < 0.999 || group.blend_mode() != usvg::BlendMode::Normal {
                    return Err("SVG contains unsupported group opacity or blend mode".into());
                }
                if !redundant_viewport_clip(group, authored_clips)
                    || group.mask().is_some()
                    || !group.filters().is_empty()
                {
                    return Err("SVG contains unsupported clipping, masks or filters".into());
                }
                walk(group, scale, budget, objects, authored_clips)?;
            }
            usvg::Node::Path(path) => {
                if !path.is_visible() {
                    continue;
                }
                Budget::add(&mut budget.source, path.data().len(), "source path")?;
                let order = match path.paint_order() {
                    usvg::PaintOrder::FillAndStroke => [false, true],
                    usvg::PaintOrder::StrokeAndFill => [true, false],
                };
                let combined = path.fill().is_some() && path.stroke().is_some();
                for is_stroke in order {
                    let (color, rings, role) = if is_stroke {
                        let Some(stroke) = path.stroke() else {
                            continue;
                        };
                        let color = paint_color(stroke.paint(), stroke.opacity().get())?;
                        let outline = stroke_outline(path, scale, budget)?;
                        let rings = flatten(
                            &outline,
                            path.abs_transform(),
                            scale,
                            FLATTEN_MM / 2.,
                            budget,
                        )?;
                        (
                            color,
                            normalize(rings, FillRule::NonZero, budget)?,
                            "stroke",
                        )
                    } else {
                        let Some(fill) = path.fill() else {
                            continue;
                        };
                        let color = paint_color(fill.paint(), fill.opacity().get())?;
                        let rings =
                            flatten(path.data(), path.abs_transform(), scale, FLATTEN_MM, budget)?;
                        let rule = match fill.rule() {
                            usvg::FillRule::NonZero => FillRule::NonZero,
                            usvg::FillRule::EvenOdd => FillRule::EvenOdd,
                        };
                        (color, normalize(rings, rule, budget)?, "fill")
                    };
                    add_object(path, rings, color, role, combined, objects)?;
                }
            }
            _ => {
                return Err(
                    "SVG text and embedded images must be converted to explicit vector paths first"
                        .into(),
                );
            }
        }
    }
    Ok(())
}

fn default_vector_effect(value: &str) -> bool {
    let mut parts = value.split('!');
    parts
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("none"))
        && parts
            .next()
            .is_none_or(|value| value.trim().eq_ignore_ascii_case("important"))
        && parts.next().is_none()
}

fn css_has_non_default_vector_effect(css: &str) -> bool {
    let check = |declaration: &str| {
        declaration.split_once(':').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("vector-effect") && !default_vector_effect(value)
        })
    };
    let mut declaration = String::new();
    let mut chars = css.chars().peekable();
    let mut quote = None;
    let mut parentheses = 0usize;
    while let Some(ch) = chars.next() {
        if let Some(delimiter) = quote {
            declaration.push(ch);
            if ch == '\\' {
                if let Some(escaped) = chars.next() {
                    declaration.push(escaped);
                }
            } else if ch == delimiter {
                quote = None;
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(ch) = chars.next() {
                if ch == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            declaration.push(' ');
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                declaration.push(ch);
            }
            '(' => {
                parentheses += 1;
                declaration.push(ch);
            }
            ')' => {
                parentheses = parentheses.saturating_sub(1);
                declaration.push(ch);
            }
            '{' if parentheses == 0 => declaration.clear(),
            ';' | '}' if parentheses == 0 => {
                if check(&declaration) {
                    return true;
                }
                declaration.clear();
            }
            _ => declaration.push(ch),
        }
    }
    check(&declaration)
}

fn validate_vector_effects(document: &roxmltree::Document<'_>) -> Result<(), String> {
    let unsupported = document
        .descendants()
        .filter(|node| node.is_element())
        .any(|node| {
            node.attribute("vector-effect")
                .is_some_and(|value| !default_vector_effect(value))
                || node
                    .attribute("style")
                    .is_some_and(css_has_non_default_vector_effect)
                || (node.tag_name().name() == "style"
                    && css_has_non_default_vector_effect(
                        &node
                            .children()
                            .filter(|child| child.is_text())
                            .filter_map(|child| child.text())
                            .collect::<String>(),
                    ))
        });
    if unsupported {
        Err("SVG contains unsupported vector-effect; convert affected strokes to explicit outlines before import".into())
    } else {
        Ok(())
    }
}

pub(super) fn load(path: &Path, width_mm: f64) -> Result<Project, String> {
    validate_width(width_mm)?;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 20_000_000 {
        return Err("SVG exceeds 20MB import limit".into());
    }
    let mut bytes = Vec::new();
    file.take(20_000_001)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 20_000_000 {
        return Err("SVG exceeds 20MB import limit".into());
    }
    let text =
        std::str::from_utf8(&bytes).map_err(|_| "SVG import requires uncompressed UTF-8 XML")?;
    let document = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    if document
        .descendants()
        .any(|node| node.is_element() && node.tag_name().name() == "text")
    {
        return Err("SVG text must be converted to explicit vector paths before import".into());
    }
    if document
        .descendants()
        .any(|node| node.is_element() && matches!(node.tag_name().name(), "image" | "feImage"))
    {
        return Err("SVG embedded or linked raster images are not vector paths; import the image separately".into());
    }
    validate_vector_effects(&document)?;
    references::validate(&document)?;
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let authored_clips = document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "clipPath")
        .filter_map(|node| node.attribute("id").map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let tree = usvg::Tree::from_data(&bytes, &options).map_err(|e| e.to_string())?;
    let scale = width_mm / f64::from(tree.size().width());
    let mut objects = Vec::new();
    walk(
        tree.root(),
        scale,
        &mut Budget::default(),
        &mut objects,
        &authored_clips,
    )?;
    if objects.is_empty() {
        return Err("SVG contains no supported solid vector paths".into());
    }
    let project = Project {
        name: name(path),
        objects,
        width_mm,
        ..Project::default()
    };
    project.validate_limits()?;
    Ok(project)
}

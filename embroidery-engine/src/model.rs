use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn distance(self, other: Self) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }
    pub fn lerp(self, other: Self, t: f64) -> Self {
        Self::new(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StitchKind {
    Running,
    Satin,
    #[default]
    Tatami,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub kind: StitchKind,
    /// Fill row spacing in mm; paired satin uses same-rail station advance.
    pub spacing_mm: f64,
    pub length_mm: f64,
    pub angle_deg: f64,
    pub auto_angle: bool,
    pub underlay: bool,
    pub edge_underlay: bool,
    pub zigzag_underlay: bool,
    pub underlay_inset_mm: f64,
    pub underlay_spacing_mm: f64,
    pub underlay_length_mm: f64,
    pub pull_comp_mm: f64,
    pub tie_in: bool,
    pub tie_off: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            kind: StitchKind::Tatami,
            spacing_mm: 0.4,
            length_mm: 3.5,
            angle_deg: 0.0,
            auto_angle: false,
            underlay: true,
            edge_underlay: false,
            zigzag_underlay: false,
            underlay_inset_mm: 0.3,
            underlay_spacing_mm: 2.0,
            underlay_length_mm: 3.0,
            pull_comp_mm: 0.0,
            tie_in: true,
            tie_off: true,
        }
    }
}
impl Settings {
    pub fn uses_v4(&self) -> bool {
        self.edge_underlay
            || self.zigzag_underlay
            || self.underlay_inset_mm != 0.3
            || self.underlay_spacing_mm != 2.0
            || self.underlay_length_mm != 3.0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CubicBezier {
    pub start: Point,
    pub control1: Point,
    pub control2: Point,
    pub end: Point,
}
impl CubicBezier {
    pub fn flatten(&self, tolerance_mm: f64) -> Vec<Point> {
        fn recurse(curve: &CubicBezier, tol: f64, depth: u8, out: &mut Vec<Point>) {
            let chord = curve.start.distance(curve.end);
            let control = curve.start.distance(curve.control1)
                + curve.control1.distance(curve.control2)
                + curve.control2.distance(curve.end);
            let perpendicular = |p: Point| {
                if chord < 1e-9 {
                    p.distance(curve.start)
                } else {
                    ((p.x - curve.start.x) * (curve.end.y - curve.start.y)
                        - (p.y - curve.start.y) * (curve.end.x - curve.start.x))
                        .abs()
                        / chord
                }
            };
            let flat = perpendicular(curve.control1).max(perpendicular(curve.control2)) <= tol
                && control - chord <= tol * 2.0;
            if depth >= 12 || flat {
                out.push(curve.end);
                return;
            }
            let a = curve.start.lerp(curve.control1, 0.5);
            let b = curve.control1.lerp(curve.control2, 0.5);
            let c = curve.control2.lerp(curve.end, 0.5);
            let d = a.lerp(b, 0.5);
            let e = b.lerp(c, 0.5);
            let m = d.lerp(e, 0.5);
            recurse(
                &CubicBezier {
                    start: curve.start,
                    control1: a,
                    control2: d,
                    end: m,
                },
                tol,
                depth + 1,
                out,
            );
            recurse(
                &CubicBezier {
                    start: m,
                    control1: e,
                    control2: c,
                    end: curve.end,
                },
                tol,
                depth + 1,
                out,
            );
        }
        let mut out = vec![self.start];
        recurse(self, tolerance_mm.max(0.001), 0, &mut out);
        out
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RailHandles {
    pub incoming: Option<Point>,
    pub outgoing: Option<Point>,
    pub smooth: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SatinRung {
    /// Normalized arc lengths on the unrotated rails, in [0, 1].
    pub left: f64,
    pub right: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SatinColumn {
    /// Open rails, both ordered from the same start end to the same finish end.
    pub left: Vec<Point>,
    pub right: Vec<Point>,
    pub left_handles: Vec<RailHandles>,
    pub right_handles: Vec<RailHandles>,
    pub rungs: Vec<SatinRung>,
    pub closed: bool,
    /// Start position on a closed column, as normalized arc length.
    pub seam: f64,
    pub first_right: bool,
}
impl SatinColumn {
    pub fn flatten_rail(&self, right: bool, tolerance_mm: f64) -> Result<Vec<Point>, String> {
        let (nodes, handles) = if right {
            (&self.right, &self.right_handles)
        } else {
            (&self.left, &self.left_handles)
        };
        if !(2..=1024).contains(&nodes.len()) || (self.closed && nodes.len() < 3) {
            return Err("Each satin rail requires 2–1024 nodes (3 for a closed rail)".into());
        }
        if !handles.is_empty() && handles.len() != nodes.len() {
            return Err("Satin handles must match the rail node count".into());
        }
        let finite = |p: Point| {
            p.x.is_finite() && p.y.is_finite() && p.x.abs() <= 9990. && p.y.abs() <= 9990.
        };
        if nodes.iter().any(|&p| !finite(p))
            || handles.iter().any(|h| {
                h.incoming.is_some_and(|p| !finite(p)) || h.outgoing.is_some_and(|p| !finite(p))
            })
        {
            return Err("Satin rail contains invalid coordinates".into());
        }
        let mut out = vec![nodes[0]];
        let count = nodes.len() - usize::from(!self.closed);
        for i in 0..count {
            let j = (i + 1) % nodes.len();
            if nodes[i].distance(nodes[j]) < 1e-7 {
                return Err("Satin rail has repeated consecutive nodes".into());
            }
            let c1 = handles.get(i).and_then(|h| h.outgoing);
            let c2 = handles.get(j).and_then(|h| h.incoming);
            if c1.is_some() || c2.is_some() {
                let points = CubicBezier {
                    start: nodes[i],
                    control1: c1.unwrap_or(nodes[i]),
                    control2: c2.unwrap_or(nodes[j]),
                    end: nodes[j],
                }
                .flatten(tolerance_mm);
                out.extend(points.into_iter().skip(1));
            } else {
                out.push(nodes[j]);
            }
            if out.len() > 8192 {
                return Err("Flattened satin rail exceeds 8192 nodes".into());
            }
        }
        Ok(out)
    }
    pub fn flattened(&self) -> Result<Self, String> {
        Ok(Self {
            left: self.flatten_rail(false, 0.02)?,
            right: self.flatten_rail(true, 0.02)?,
            left_handles: Vec::new(),
            right_handles: Vec::new(),
            ..self.clone()
        })
    }
    pub fn uses_v4(&self) -> bool {
        !self.left_handles.is_empty()
            || !self.right_handles.is_empty()
            || !self.rungs.is_empty()
            || self.closed
            || self.seam != 0.
            || self.first_right
    }
    pub fn boundary(&self) -> Vec<Point> {
        self.left
            .iter()
            .copied()
            .chain(self.right.iter().rev().copied())
            .collect()
    }
    /// Separate even-odd rings preserve the hole of a closed satin outline.
    pub fn boundary_rings(&self) -> Result<Vec<Vec<Point>>, String> {
        let column = self.flattened()?;
        if column.closed {
            let mut left = column.left;
            let mut right = column.right;
            left.pop();
            right.pop();
            Ok(vec![left, right])
        } else {
            Ok(vec![column.boundary()])
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Object {
    pub id: u64,
    pub name: String,
    /// Closed rings, implicitly closed. All rings use even-odd filling.
    pub contours: Vec<Vec<Point>>,
    /// Open running-stitch polylines. Repeat the first point to close a loop.
    /// Stored with ELAB schema v3; never implicitly closed.
    pub running_paths: Vec<Vec<Point>>,
    /// Optional additional closed cubic contour; editable control handles.
    pub bezier_segments: Vec<CubicBezier>,
    /// Explicit paired-rail geometry, serialized with ELAB schema v2.
    pub satin_column: Option<SatinColumn>,
    pub centerline_graph: Option<crate::axis_edit::AxisGraph>,
    pub color: [u8; 3],
    pub settings: Settings,
    pub visible: bool,
}
impl Default for Object {
    fn default() -> Self {
        Self {
            id: 1,
            name: "Object".into(),
            contours: Vec::new(),
            running_paths: Vec::new(),
            bezier_segments: Vec::new(),
            satin_column: None,
            centerline_graph: None,
            color: [35, 100, 180],
            settings: Settings::default(),
            visible: true,
        }
    }
}
impl Object {
    pub fn flattened_contours(&self) -> Vec<Vec<Point>> {
        let mut out = self.contours.clone();
        if !self.bezier_segments.is_empty() {
            let mut ring = Vec::new();
            for curve in &self.bezier_segments {
                let points = curve.flatten(0.03);
                if ring.is_empty() {
                    ring.extend(points);
                } else {
                    ring.extend(points.into_iter().skip(1));
                }
            }
            if ring.len() >= 3 {
                out.push(ring);
            }
        }
        if let Some(column) = &self.satin_column {
            // Invalid drafts are rejected by generation, not converted to a bogus fill.
            if let Ok(rings) = column.boundary_rings() {
                out.extend(rings);
            }
        }
        out
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub objects: Vec<Object>,
    pub width_mm: f64,
    pub source_image: Option<crate::persistence::SourceImage>,
}
impl Default for Project {
    fn default() -> Self {
        Self {
            version: 1,
            name: "Untitled".into(),
            objects: Vec::new(),
            width_mm: 100.0,
            source_image: None,
        }
    }
}
impl Project {
    /// Bound untrusted project input before flattening or stitch allocation.
    pub fn validate_limits(&self) -> Result<(), String> {
        if let Some(source) = &self.source_image {
            source.validate()?;
        }
        for object in &self.objects {
            if let Some(graph) = &object.centerline_graph {
                graph.validate()?;
                if object.running_paths != graph.paths() {
                    return Err("Running paths must match their editable centerline graph".into());
                }
            }
        }
        if self.objects.len() > 4096 {
            return Err("Project exceeds 4096 objects".into());
        }
        if self.name.len() > 4096 {
            return Err("Project name exceeds 4096 bytes".into());
        }
        let contour_nodes = self
            .objects
            .iter()
            .flat_map(|o| o.contours.iter())
            .map(Vec::len)
            .sum::<usize>();
        let rail_nodes = self
            .objects
            .iter()
            .filter_map(|o| o.satin_column.as_ref())
            .map(|c| c.left.len().saturating_add(c.right.len()))
            .sum::<usize>();
        let running_nodes = self
            .objects
            .iter()
            .flat_map(|o| &o.running_paths)
            .map(Vec::len)
            .sum::<usize>();
        let nodes = contour_nodes
            .saturating_add(rail_nodes)
            .saturating_add(running_nodes);
        let curves = self
            .objects
            .iter()
            .map(|o| o.bezier_segments.len())
            .sum::<usize>();
        if nodes > 100_000 || curves > 512 {
            return Err(
                "Project exceeds 100,000 contour/rail/path nodes or 512 cubic segments".into(),
            );
        }
        let finite = |p: &Point| {
            p.x.is_finite() && p.y.is_finite() && p.x.abs() <= 9990. && p.y.abs() <= 9990.
        };
        if self
            .objects
            .iter()
            .flat_map(|o| o.contours.iter().flatten())
            .any(|p| !finite(p))
            || self
                .objects
                .iter()
                .flat_map(|o| o.running_paths.iter().flatten())
                .any(|p| !finite(p))
            || self
                .objects
                .iter()
                .filter_map(|o| o.satin_column.as_ref())
                .flat_map(|c| c.left.iter().chain(&c.right))
                .any(|p| !finite(p))
            || self
                .objects
                .iter()
                .flat_map(|o| &o.bezier_segments)
                .any(|c| ![c.start, c.control1, c.control2, c.end].iter().all(finite))
        {
            return Err("Project contains invalid geometry coordinates".into());
        }
        for column in self.objects.iter().filter_map(|o| o.satin_column.as_ref()) {
            if column.left_handles.len() > 1024
                || column.right_handles.len() > 1024
                || column.rungs.len() > 1024
                || (!column.left_handles.is_empty()
                    && column.left_handles.len() != column.left.len())
                || (!column.right_handles.is_empty()
                    && column.right_handles.len() != column.right.len())
                || !column.seam.is_finite()
                || !(0.0..1.0).contains(&column.seam)
                || column
                    .left_handles
                    .iter()
                    .chain(&column.right_handles)
                    .any(|h| {
                        h.incoming.is_some_and(|p| !finite(&p))
                            || h.outgoing.is_some_and(|p| !finite(&p))
                    })
                || column.rungs.iter().any(|r| {
                    !r.left.is_finite()
                        || !r.right.is_finite()
                        || !(0.0..=1.0).contains(&r.left)
                        || !(0.0..=1.0).contains(&r.right)
                })
            {
                return Err("Project contains invalid satin handles, rungs or seam".into());
            }
        }
        for object in self.objects.iter().filter(|o| !o.running_paths.is_empty()) {
            if object.settings.kind != StitchKind::Running {
                return Err(format!(
                    "{}: open paths require Running stitch type",
                    object.name
                ));
            }
            if !object.contours.is_empty()
                || !object.bezier_segments.is_empty()
                || object.satin_column.is_some()
            {
                return Err(format!(
                    "{}: open paths must not mix closed contours, cubics or satin rails in one object",
                    object.name
                ));
            }
            if object.running_paths.iter().any(|path| {
                path.len() < 2 || path.windows(2).all(|pair| pair[0].distance(pair[1]) < 1e-7)
            }) {
                return Err(format!(
                    "{}: each open path requires at least two distinct points",
                    object.name
                ));
            }
        }
        Ok(())
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate_limits()?;
        if !matches!(self.version, 1..=4) {
            return Err("Unsupported project schema version".into());
        }
        let mut persisted = self.clone();
        if self.source_image.is_some()
            || self.objects.iter().any(|o| {
                o.centerline_graph.is_some()
                    || o.settings.uses_v4()
                    || o.satin_column.as_ref().is_some_and(SatinColumn::uses_v4)
            })
        {
            persisted.version = 4;
        } else if self.objects.iter().any(|o| !o.running_paths.is_empty()) {
            persisted.version = persisted.version.max(3);
        } else if self.objects.iter().any(|o| o.satin_column.is_some()) {
            persisted.version = persisted.version.max(2);
        }
        crate::persistence::save_project(&persisted, path)
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 32_000_000 {
            return Err("Project file exceeds 32MB".into());
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let project: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if !matches!(project.version, 1..=4) {
            return Err("Unsupported project schema version".into());
        }
        project.validate_limits()?;
        Ok(project)
    }
    pub fn demo() -> Self {
        Self {
            name: "Independent demo".into(),
            objects: vec![Object {
                name: "Holed fill".into(),
                contours: vec![
                    vec![
                        Point::new(10., 10.),
                        Point::new(70., 10.),
                        Point::new(70., 50.),
                        Point::new(10., 50.),
                    ],
                    vec![
                        Point::new(30., 22.),
                        Point::new(50., 22.),
                        Point::new(50., 38.),
                        Point::new(30., 38.),
                    ],
                ],
                ..Object::default()
            }],
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    Stitch,
    Jump,
    ColorChange,
    End,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Stitch {
    pub x: f64,
    pub y: f64,
    pub command: Command,
    pub color: [u8; 3],
    pub object_id: u64,
}
impl Stitch {
    pub fn point(self) -> Point {
        Point::new(self.x, self.y)
    }
}

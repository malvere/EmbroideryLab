//! Physical extents of sewn stitch segments, in project millimetres.

use embroidery_lab::model::{Command, Point, Stitch};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct StitchBounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl StitchBounds {
    pub(super) fn width_mm(self) -> f64 {
        self.max_x - self.min_x
    }

    pub(super) fn height_mm(self) -> f64 {
        self.max_y - self.min_y
    }

    fn include(&mut self, point: Point) {
        self.min_x = self.min_x.min(point.x);
        self.min_y = self.min_y.min(point.y);
        self.max_x = self.max_x.max(point.x);
        self.max_y = self.max_y.max(point.y);
    }
}

/// Measure the complete sewn design, independently of simulation progress.
///
/// Include both ends of every sewn segment. A jump only establishes the start
/// of a later stitch: travel excursions, controls and the unused default origin
/// do not directly enlarge the embroidery size. A displaced ColorChange also
/// establishes the next stitch's start, as allowed by imported DST commands.
/// A first Stitch without positioning sews from origin and includes (0, 0).
pub(super) fn stitched_bounds(stitches: &[Stitch]) -> Option<StitchBounds> {
    let mut current = Some(Point::default());
    let mut bounds: Option<StitchBounds> = None;
    for stitch in stitches {
        match stitch.command {
            Command::Stitch | Command::Jump | Command::ColorChange | Command::Trim => {
                let point = stitch.point();
                if !point.x.is_finite() || !point.y.is_finite() {
                    // Do not invent a segment across malformed positional data.
                    current = None;
                    continue;
                }
                if stitch.command == Command::Stitch {
                    let extent = bounds.get_or_insert(StitchBounds {
                        min_x: point.x,
                        min_y: point.y,
                        max_x: point.x,
                        max_y: point.y,
                    });
                    if let Some(start) = current {
                        extent.include(start);
                    }
                    extent.include(point);
                }
                current = Some(point);
            }
            Command::End => break,
        }
    }
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(command: Command, x: f64, y: f64) -> Stitch {
        Stitch {
            command,
            x,
            y,
            color: [25, 70, 180],
            object_id: 1,
        }
    }

    #[test]
    fn initial_positioning_and_unused_jump_extremes_do_not_enlarge_the_design() {
        let bounds = stitched_bounds(&[
            command(Command::Jump, 500., 600.),
            command(Command::Jump, 30., 40.),
            command(Command::Stitch, 35., 46.),
            command(Command::Jump, -800., -900.),
            command(Command::End, -800., -900.),
        ])
        .unwrap();
        assert_eq!(
            bounds,
            StitchBounds {
                min_x: 30.,
                min_y: 40.,
                max_x: 35.,
                max_y: 46.,
            }
        );
        assert_eq!((bounds.width_mm(), bounds.height_mm()), (5., 6.));
    }

    #[test]
    fn the_start_of_a_sewn_segment_is_part_of_the_embroidery() {
        let bounds = stitched_bounds(&[
            command(Command::Jump, -20., -30.),
            command(Command::Stitch, -8., -10.),
        ])
        .unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (-20., -30.));
        assert_eq!((bounds.max_x, bounds.max_y), (-8., -10.));
        assert_eq!((bounds.width_mm(), bounds.height_mm()), (12., 20.));
    }

    #[test]
    fn a_first_stitch_without_positioning_sews_from_machine_origin() {
        let bounds = stitched_bounds(&[command(Command::Stitch, 10., -20.)]).unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (0., -20.));
        assert_eq!((bounds.max_x, bounds.max_y), (10., 0.));
        assert_eq!((bounds.width_mm(), bounds.height_mm()), (10., 20.));
    }

    #[test]
    fn unused_control_excursions_do_not_change_sewn_extents() {
        let mut second_color = command(Command::Stitch, 16., 28.);
        second_color.color = [190, 20, 10];
        second_color.object_id = 2;
        let bounds = stitched_bounds(&[
            command(Command::Jump, 10., 20.),
            command(Command::Stitch, 14., 24.),
            command(Command::ColorChange, 9000., -9000.),
            command(Command::Jump, 14., 24.),
            second_color,
        ])
        .unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (10., 20.));
        assert_eq!((bounds.max_x, bounds.max_y), (16., 28.));
    }

    #[test]
    fn a_displaced_color_change_positions_the_start_of_the_next_sewn_segment() {
        let bounds = stitched_bounds(&[
            command(Command::ColorChange, 30., 40.),
            command(Command::Stitch, 35., 46.),
            command(Command::ColorChange, 9000., -9000.),
            command(Command::End, 9000., -9000.),
        ])
        .unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (30., 40.));
        assert_eq!((bounds.max_x, bounds.max_y), (35., 46.));
        assert_eq!((bounds.width_mm(), bounds.height_mm()), (5., 6.));
    }

    #[test]
    fn no_sewn_commands_have_no_bounds_and_end_ignores_trailing_data() {
        assert!(stitched_bounds(&[]).is_none());
        assert!(
            stitched_bounds(&[
                command(Command::Jump, 40., 50.),
                command(Command::ColorChange, 40., 50.),
                command(Command::End, 40., 50.),
            ])
            .is_none()
        );
        let bounds = stitched_bounds(&[
            command(Command::Jump, 40., 50.),
            command(Command::Stitch, 44., 56.),
            command(Command::End, 44., 56.),
            command(Command::Stitch, 9000., 9000.),
        ])
        .unwrap();
        assert_eq!((bounds.width_mm(), bounds.height_mm()), (4., 6.));
    }

    #[test]
    fn malformed_positions_do_not_create_nonfinite_bounds_or_a_fictitious_start() {
        let bounds = stitched_bounds(&[
            command(Command::ColorChange, f64::NAN, 0.),
            command(Command::Stitch, 5., 7.),
            command(Command::Stitch, 8., 9.),
            command(Command::Jump, f64::INFINITY, 1000.),
        ])
        .unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (5., 7.));
        assert_eq!((bounds.max_x, bounds.max_y), (8., 9.));
        assert_eq!((bounds.width_mm(), bounds.height_mm()), (3., 2.));
    }
}

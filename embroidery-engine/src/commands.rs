//! One positional convention for planning, preview, diagnostics and export.
//! Controls are stationary; displaced legacy controls become Jump + control.
use crate::model::*;

pub const MACHINE_GRID_MM: f64 = 0.1;
pub const MAX_SEWN_LENGTH_MM: f64 = 12.0;

/// Machine color-block order must follow the exported plan, rather than the
/// editable object list: hidden lower fills may have no needle penetrations.
/// Empty blocks are retained when controls precede/between sewing blocks.
pub fn thread_sidecar(project: &Project, stitches: &[Stitch]) -> serde_json::Value {
    use std::collections::BTreeSet;
    #[derive(Default)]
    struct Block {
        first_command: usize,
        needles: usize,
        colors: BTreeSet<[u8; 3]>,
        objects: BTreeSet<u64>,
    }
    let mut blocks = vec![Block::default()];
    let mut sewn = BTreeSet::new();
    for (index, stitch) in stitches.iter().enumerate() {
        match stitch.command {
            Command::End => break,
            Command::ColorChange => blocks.push(Block {
                first_command: index + 1,
                ..Default::default()
            }),
            Command::Stitch => {
                let block = blocks.last_mut().unwrap();
                block.needles += 1;
                block.colors.insert(stitch.color);
                block.objects.insert(stitch.object_id);
                sewn.insert((stitch.object_id, stitch.color));
            }
            _ => {}
        }
    }
    if sewn.is_empty() {
        blocks.clear();
    }
    let color_blocks: Vec<_> = blocks
        .into_iter()
        .enumerate()
        .map(|(index, block)| {
            let rgb = if block.colors.len() == 1 {
                block.colors.first().copied()
            } else {
                None
            };
            serde_json::json!({"index":index,"first_command_index":block.first_command,
            "rgb":rgb,"observed_rgb":block.colors,"needle_count":block.needles,
            "source_object_ids":block.objects})
        })
        .collect();
    serde_json::json!({
        "format":"EmbroideryLab thread sidecar v2", "units":"mm", "dst_grid_mm":MACHINE_GRID_MM,
        "trim_encoding":project.machine_profile.dst_trim_encoding,
        "color_blocks":color_blocks,
        "note":"Use color_blocks for machine thread order. RGB comes from project assignments; imported machine colors may be illustrative.",
        "source_attribution":"Objects match sewn commands by source ID and RGB; duplicate identities share attribution.",
        "objects":project.objects.iter().filter(|o|o.visible).map(|o|serde_json::json!({
            "id":o.id,"name":o.name,"rgb":o.color,"sewn":sewn.contains(&(o.id,o.color))
        })).collect::<Vec<_>>()
    })
}

pub fn normalize(stitches: &[Stitch]) -> Result<Vec<Stitch>, String> {
    if stitches.len() > 1_000_000 {
        return Err("Design exceeds one million commands".into());
    }
    let metadata = stitches
        .iter()
        .map(|s| CommandMeta {
            role: if s.command == Command::Stitch {
                // Without pass metadata a duplicate may be an intentional tie.
                StitchRole::Lock
            } else {
                StitchRole::Control
            },
            pass_id: 0,
        })
        .collect();
    normalize_plan(MachinePlan {
        stitches: stitches.to_vec(),
        metadata,
        warnings: vec![],
    })
    .map(|plan| plan.stitches)
}

/// Quantize once, preserve lock intent, and bound physical sewn lengths independently
/// of the DST record's per-axis limit. Metadata remains aligned with commands.
pub fn normalize_plan(plan: MachinePlan) -> Result<MachinePlan, String> {
    normalize_plan_with_limits(plan, &MachineProfile::default())
}

pub fn normalize_plan_with_limits(
    plan: MachinePlan,
    limits: &MachineProfile,
) -> Result<MachinePlan, String> {
    if plan.stitches.len() > 1_000_000 {
        return Err("Design exceeds one million commands".into());
    }
    limits.validate()?;
    if plan.stitches.len() != plan.metadata.len() {
        return Err("Machine plan metadata does not match command count".into());
    }
    let mut out = MachinePlan {
        warnings: plan.warnings,
        ..Default::default()
    };
    let mut current = Point::default();
    let mut ended = false;
    for (index, (mut stitch, meta)) in plan.stitches.into_iter().zip(plan.metadata).enumerate() {
        if ended {
            return Err(format!("Command {index} occurs after END"));
        }
        if !stitch.x.is_finite()
            || !stitch.y.is_finite()
            || stitch.x.abs() > 9999.
            || stitch.y.abs() > 9999.
        {
            return Err(format!("Invalid coordinate at command {index}"));
        }
        stitch.x = (stitch.x * 10.).round() / 10.;
        stitch.y = (stitch.y * 10.).round() / 10.;
        let target = stitch.point();
        let moving = matches!(stitch.command, Command::Stitch | Command::Jump);
        if !moving && target.distance(current) > 1e-8 {
            out.stitches.push(Stitch {
                command: Command::Jump,
                ..stitch
            });
            out.metadata.push(CommandMeta {
                role: StitchRole::Control,
                ..meta
            });
            current = target;
        }
        if moving {
            let distance = current.distance(target);
            // Raw machine input has no pass annotations. Stationary Jump
            // controls may be an explicit profile trim sequence and must
            // survive semantic re-export without guessing a universal opcode.
            // Generated zero Travel jumps remain accidental duplicates.
            let preserve_stationary = (stitch.command == Command::Stitch
                && meta.role == StitchRole::Lock)
                || (stitch.command == Command::Jump && meta.role == StitchRole::Control);
            if distance < 1e-8 && !preserve_stationary {
                continue;
            }
            // The target is already quantized. Preserve every physically valid
            // segment intact (especially paired Satin). Only oversized segments
            // need subdivision and a reserve for its new intermediate rounding.
            let parts =
                if stitch.command == Command::Stitch && distance > limits.max_stitch_mm + 1e-8 {
                    (distance / (limits.max_stitch_mm - 0.15)).ceil().max(1.) as usize
                } else {
                    1
                };
            let start = current;
            for i in 1..=parts {
                let mut p = start.lerp(target, i as f64 / parts as f64);
                p.x = (p.x * 10.).round() / 10.;
                p.y = (p.y * 10.).round() / 10.;
                out.stitches.push(Stitch {
                    x: p.x,
                    y: p.y,
                    ..stitch
                });
                out.metadata.push(meta);
            }
            current = target;
        } else {
            out.stitches.push(stitch);
            out.metadata.push(CommandMeta {
                role: StitchRole::Control,
                ..meta
            });
            ended = stitch.command == Command::End;
        }
        if out.stitches.len() > 1_000_000 {
            return Err("Design exceeds one million normalized commands".into());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn command(command: Command, x: f64, y: f64) -> Stitch {
        Stitch {
            x,
            y,
            command,
            color: [0; 3],
            object_id: 7,
        }
    }
    #[test]
    fn displaced_controls_are_explicit_and_normalization_is_idempotent() {
        let normalized = normalize(&[
            command(Command::ColorChange, 20., 30.),
            command(Command::Stitch, 22., 30.),
            command(Command::End, 22., 30.),
        ])
        .unwrap();
        assert_eq!(normalized[0].command, Command::Jump);
        assert_eq!(normalized[1].command, Command::ColorChange);
        assert_eq!(normalized[0].point(), normalized[1].point());
        assert_eq!(
            serde_json::to_value(normalize(&normalized).unwrap()).unwrap(),
            serde_json::to_value(normalized).unwrap()
        );
    }
    #[test]
    fn diagonal_stitches_have_a_physical_limit_after_quantization() {
        let result = normalize(&[command(Command::Stitch, 12.1, 12.1)]).unwrap();
        let mut p = Point::default();
        for s in result {
            assert!(p.distance(s.point()) <= MAX_SEWN_LENGTH_MM);
            p = s.point();
        }
    }

    #[test]
    fn raw_stationary_jump_controls_survive_without_preserving_generated_duplicates() {
        let records = vec![
            command(Command::Jump, 1., 1.),
            command(Command::Jump, 1., 1.),
            command(Command::Jump, 1., 1.),
            command(Command::Jump, 1., 1.),
            command(Command::Stitch, 2., 1.),
            command(Command::End, 2., 1.),
        ];
        let normalized = normalize(&records).unwrap();
        assert_eq!(normalized.len(), records.len());
        for (actual, expected) in normalized.iter().zip(&records) {
            assert_eq!(actual.command, expected.command);
            assert_eq!(actual.point(), expected.point());
        }
        let generated = MachinePlan {
            stitches: records[..4].to_vec(),
            metadata: vec![
                CommandMeta {
                    role: StitchRole::Travel,
                    pass_id: 1
                };
                4
            ],
            warnings: vec![],
        };
        let normalized = normalize_plan(generated).unwrap();
        assert_eq!(normalized.stitches.len(), 1);
        assert_eq!(normalized.stitches[0].point(), Point::new(1., 1.));
    }
}

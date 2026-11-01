//! Experimental Tajima binary DSB codec. Coordinates are millimetres, x right,
//! y down, as in the editor. Protocol interpretation is checked against public
//! pyembroidery DsbReader and three read-only installed DSB examples.
//! This is not a physical-machine or firmware compatibility certification.
use crate::model::{Command, Stitch};
use serde::{Deserialize, Serialize};

const LIMIT: usize = 1_000_000;
const PALETTE: [[u8; 3]; 8] = [
    [35, 100, 190],
    [220, 70, 50],
    [40, 155, 105],
    [190, 120, 35],
    [145, 65, 165],
    [40, 170, 185],
    [65, 65, 75],
    [225, 155, 185],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DsbCommand {
    Stitch,
    Jump,
    Trim,
    Stop,
    NeedleChange(u8),
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DsbRecord {
    pub x: f64,
    pub y: f64,
    pub command: DsbCommand,
}

fn coordinate(x: f64, y: f64) -> Result<(i32, i32), String> {
    if !x.is_finite() || !y.is_finite() || x.abs() > 9999.0 || y.abs() > 9999.0 {
        return Err("DSB coordinates must be finite and within ±9999mm application limits".into());
    }
    Ok(((x * 10.).round() as i32, (y * 10.).round() as i32))
}
fn moving(command: DsbCommand) -> bool {
    matches!(command, DsbCommand::Stitch | DsbCommand::Jump)
}
fn validate(records: &[DsbRecord], require_end: bool) -> Result<(), String> {
    if records.len() > LIMIT {
        return Err("DSB exceeds one million records".into());
    }
    let mut current = (0, 0);
    let mut ended = false;
    for (index, record) in records.iter().enumerate() {
        if ended {
            return Err(format!("DSB record {index} occurs after END"));
        }
        let point = coordinate(record.x, record.y)?;
        if let DsbCommand::NeedleChange(needle) = record.command
            && !(1..=15).contains(&needle)
        {
            return Err("DSB needle number must be 1–15".into());
        }
        if moving(record.command) {
            current = point;
        } else if point != current {
            return Err(format!(
                "Non-moving DSB command at record {index} changes position; add an explicit jump first"
            ));
        }
        if record.command == DsbCommand::End {
            ended = true;
        }
    }
    if require_end && !ended {
        return Err("DSB records lack END".into());
    }
    Ok(())
}

/// Strict decoding: unsupported control bytes are errors, rather than an
/// implicit early END. Non-moving command payload bytes do not move the cursor,
/// matching the public reader. All supported commands remain distinct.
pub fn decode(bytes: &[u8]) -> Result<Vec<DsbRecord>, String> {
    if bytes.len() < 515 || bytes.len() > 64_000_000 {
        return Err("DSB needs a 512-byte header plus records and is limited to 64MB".into());
    }
    let body = &bytes[512..];
    let mut out = Vec::new();
    let mut x = 0_i32;
    let mut y = 0_i32;
    let mut ended = false;
    for (index, record) in body.as_chunks::<3>().0.iter().enumerate() {
        if out.len() >= LIMIT {
            return Err("DSB exceeds one million decoded records".into());
        }
        let control = record[0];
        let command = if control & 0x1f == 0 {
            DsbCommand::Stitch
        } else if control & 0x1f == 1 {
            DsbCommand::Jump
        } else {
            match control {
                0xe7 => DsbCommand::Trim,
                0xe8 => DsbCommand::Stop,
                0xe9..=0xf7 => DsbCommand::NeedleChange(control - 0xe8),
                0xf8 => DsbCommand::End,
                _ => {
                    return Err(format!(
                        "Unsupported DSB control 0x{control:02x} at record {index}"
                    ));
                }
            }
        };
        if moving(command) {
            x += if control & 0x20 != 0 {
                -i32::from(record[2])
            } else {
                i32::from(record[2])
            };
            y += if control & 0x40 != 0 {
                i32::from(record[1])
            } else {
                -i32::from(record[1])
            };
        }
        let item = DsbRecord {
            x: f64::from(x) / 10.,
            y: f64::from(y) / 10.,
            command,
        };
        coordinate(item.x, item.y)?;
        out.push(item);
        if command == DsbCommand::End {
            let trailing = &body[(index + 1) * 3..];
            if trailing
                .iter()
                .any(|&byte| !matches!(byte, 0 | 0x1a | b' '))
            {
                return Err("DSB has unexpected non-padding data after END".into());
            }
            ended = true;
            break;
        }
    }
    if !ended {
        return Err("DSB is truncated or lacks explicit END".into());
    }
    Ok(out)
}

fn append(body: &mut Vec<u8>, dx: i32, dy: i32, command: DsbCommand) -> Result<(), String> {
    if body.len() / 3 >= LIMIT {
        return Err("Encoded DSB exceeds one million records".into());
    }
    let control = match command {
        DsbCommand::Stitch | DsbCommand::Jump => {
            0x80 | if dx < 0 { 0x20 } else { 0 }
                | if dy > 0 { 0x40 } else { 0 }
                | u8::from(command == DsbCommand::Jump)
        }
        DsbCommand::Trim => 0xe7,
        DsbCommand::Stop => 0xe8,
        DsbCommand::NeedleChange(needle) => 0xe8 + needle,
        DsbCommand::End => 0xf8,
    };
    body.extend_from_slice(&[control, dy.unsigned_abs() as u8, dx.unsigned_abs() as u8]);
    Ok(())
}

/// Encode the documented command subset, with canonical movement bit 0x80
/// observed in the installed corpus. Movements split at 255 tenth-mm per axis;
/// this file-level bound is not a technological maximum stitch length.
pub fn encode(records: &[DsbRecord], name: &str) -> Result<Vec<u8>, String> {
    validate(records, false)?;
    let mut body = Vec::with_capacity(records.len().min(LIMIT) * 3);
    let mut current = (0_i32, 0_i32);
    let mut bounds = (0_i32, 0_i32, 0_i32, 0_i32);
    let mut changes = 0;
    let mut ended = false;
    for record in records {
        let target = coordinate(record.x, record.y)?;
        if moving(record.command) {
            let (dx, dy) = (target.0 - current.0, target.1 - current.1);
            let parts = ((dx.abs().max(dy.abs()) + 254) / 255).max(1);
            let start = current;
            for step in 1..=parts {
                let next = (
                    start.0 + (f64::from(dx) * f64::from(step) / f64::from(parts)).round() as i32,
                    start.1 + (f64::from(dy) * f64::from(step) / f64::from(parts)).round() as i32,
                );
                append(
                    &mut body,
                    next.0 - current.0,
                    next.1 - current.1,
                    record.command,
                )?;
                current = next;
                bounds.0 = bounds.0.min(current.0);
                bounds.1 = bounds.1.min(-current.1);
                bounds.2 = bounds.2.max(current.0);
                bounds.3 = bounds.3.max(-current.1);
            }
        } else {
            append(&mut body, 0, 0, record.command)?;
            if matches!(
                record.command,
                DsbCommand::Stop | DsbCommand::NeedleChange(_)
            ) {
                changes += 1;
            }
            if record.command == DsbCommand::End {
                ended = true;
            }
        }
    }
    if !ended {
        append(&mut body, 0, 0, DsbCommand::End)?;
    }
    if changes > 999 {
        return Err("Experimental DSB writer limits header CO to 999 control changes".into());
    }
    let mut label: String = name
        .chars()
        .filter(|character| character.is_ascii() && !character.is_control())
        .take(16)
        .collect();
    if label.trim().is_empty() {
        label = "EmbroideryLab".into();
    }
    // These are ASCII Tajima header fields, not the DST displacement encoding.
    // ST counts every binary record including END, as in all three corpus files.
    let header = format!(
        "LA:{label:<16}\rST:{:7}\rCO:{changes:3}\r+X:{:5}\r-X:{:5}\r+Y:{:5}\r-Y:{:5}\rAX:{:+06}\rAY:{:+06}\rMX:+00000\rMY:+00000\rPD:******\r",
        body.len() / 3,
        bounds.2,
        -bounds.0,
        bounds.3,
        -bounds.1,
        current.0,
        -current.1
    );
    let mut bytes = vec![b' '; 512];
    bytes[..header.len()].copy_from_slice(header.as_bytes());
    bytes[header.len()] = 0x1a;
    bytes.extend(body);
    Ok(bytes)
}

/// Project ColorChange has no physical needle assignment. This experimental
/// conversion emits manual Stop (E8), never guesses an automatic needle change.
pub fn from_stitches(stitches: &[Stitch]) -> Result<Vec<DsbRecord>, String> {
    if stitches.len() > LIMIT {
        return Err("DSB conversion exceeds one million commands".into());
    }
    let mut records: Vec<_> = stitches
        .iter()
        .map(|stitch| DsbRecord {
            x: stitch.x,
            y: stitch.y,
            command: match stitch.command {
                Command::Stitch => DsbCommand::Stitch,
                Command::Jump => DsbCommand::Jump,
                Command::ColorChange => DsbCommand::Stop,
                Command::End => DsbCommand::End,
            },
        })
        .collect();
    if records
        .last()
        .is_none_or(|record| record.command != DsbCommand::End)
    {
        let (x, y) = records.last().map_or((0., 0.), |r| (r.x, r.y));
        records.push(DsbRecord {
            x,
            y,
            command: DsbCommand::End,
        });
    }
    validate(&records, true)?;
    Ok(records)
}

/// View-only projection. Trim becomes a zero-movement Jump, and Stop/Needle
/// become illustrative colour changes. Keep the original DsbRecord sequence for
/// re-export; this projection cannot preserve all DSB machine semantics in DST.
pub fn preview(records: &[DsbRecord]) -> Result<Vec<Stitch>, String> {
    validate(records, true)?;
    let mut palette = 0;
    let mut group = 0;
    let mut out = Vec::with_capacity(records.len());
    for record in records {
        let command = match record.command {
            DsbCommand::Stitch => Command::Stitch,
            DsbCommand::Jump | DsbCommand::Trim => Command::Jump,
            DsbCommand::Stop => {
                palette = (palette + 1) % PALETTE.len();
                group += 1;
                Command::ColorChange
            }
            DsbCommand::NeedleChange(needle) => {
                palette = usize::from(needle - 1) % PALETTE.len();
                group += 1;
                Command::ColorChange
            }
            DsbCommand::End => Command::End,
        };
        out.push(Stitch {
            x: record.x,
            y: record.y,
            command,
            color: PALETTE[palette],
            object_id: group,
        });
    }
    Ok(out)
}

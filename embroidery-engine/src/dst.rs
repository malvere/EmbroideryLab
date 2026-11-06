//! Tajima DST: 512-byte ASCII header and signed balanced-ternary displacements.
//! Independent codec. Format checks cross-checked with pyembroidery's public
//! DstReader/DstWriter, not with proprietary software code.
use crate::model::{Command, Point, Stitch};
use serde::Serialize;

const X_BITS: [(usize, u8, u8); 5] = [(0, 0, 1), (1, 0, 1), (0, 2, 3), (1, 2, 3), (2, 2, 3)];
const Y_BITS: [(usize, u8, u8); 5] = [(0, 7, 6), (1, 7, 6), (0, 5, 4), (1, 5, 4), (2, 5, 4)];
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

fn encode_axis(mut value: i32, bits: &[(usize, u8, u8); 5], out: &mut [u8; 3]) {
    for &(byte, pos, neg) in bits {
        let digit = match value.rem_euclid(3) {
            2 => -1,
            n => n,
        };
        if digit == 1 {
            out[byte] |= 1 << pos;
        } else if digit == -1 {
            out[byte] |= 1 << neg;
        }
        value = (value - digit) / 3;
    }
    debug_assert_eq!(value, 0);
}
fn decode_axis(record: &[u8], bits: &[(usize, u8, u8); 5]) -> Result<i32, String> {
    let mut result = 0;
    let mut weight = 1;
    for &(byte, pos, neg) in bits {
        let p = (record[byte] >> pos) & 1;
        let n = (record[byte] >> neg) & 1;
        if p == 1 && n == 1 {
            return Err("Contradictory signed DST coordinate bits".into());
        }
        result += (i32::from(p) - i32::from(n)) * weight;
        weight *= 3;
    }
    Ok(result)
}

fn append_record(dx: i32, dy: i32, command: Command, body: &mut Vec<u8>) {
    let mut record = [0, 0, 3];
    match command {
        Command::Stitch | Command::Jump => {
            encode_axis(dx, &X_BITS, &mut record);
            encode_axis(-dy, &Y_BITS, &mut record);
            if command == Command::Jump {
                record[2] |= 0x80;
            }
        }
        Command::ColorChange => record[2] = 0xc3,
        Command::Trim => unreachable!("Trim is expanded by the selected DST profile"),
        Command::End => record[2] = 0xf3,
    }
    body.extend_from_slice(&record);
}

pub fn encode(stitches: &[Stitch], name: &str) -> Result<Vec<u8>, String> {
    encode_with_trim(stitches, name, TrimEncoding::ThreeJump)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TrimEncoding {
    /// Conventional three stationary jumps; machine interpretation must be tested.
    #[default]
    ThreeJump,
    /// Retain the uncut thread. Only use when explicitly chosen by the operator.
    KeepThread,
}

pub fn encode_with_trim(
    stitches: &[Stitch],
    name: &str,
    trim: TrimEncoding,
) -> Result<Vec<u8>, String> {
    let normalized = crate::commands::normalize(stitches)?;
    let stitches = &normalized;
    if stitches.len() > 1_000_000 {
        return Err("Design exceeds one million commands".into());
    }
    let mut body = Vec::with_capacity(stitches.len() * 3);
    let mut current = (0_i32, 0_i32);
    let mut bounds = (0_i32, 0_i32, 0_i32, 0_i32);
    let mut colors = 0;
    let mut ended = false;
    for (index, stitch) in stitches.iter().enumerate() {
        if ended {
            return Err(format!("Command {index} occurs after END"));
        }
        if stitch.command == Command::Trim {
            if trim == TrimEncoding::ThreeJump {
                for _ in 0..3 {
                    append_record(0, 0, Command::Jump, &mut body);
                }
            }
            continue;
        }
        if !stitch.x.is_finite()
            || !stitch.y.is_finite()
            || stitch.x.abs() > 9999.0
            || stitch.y.abs() > 9999.0
        {
            return Err(format!("Invalid coordinate at command {index}"));
        }
        let target = (
            (stitch.x * 10.0).round() as i32,
            (stitch.y * 10.0).round() as i32,
        );
        let (dx, dy) = (target.0 - current.0, target.1 - current.1);
        let moving = matches!(stitch.command, Command::Stitch | Command::Jump);
        if moving || target != current {
            let parts = ((dx.abs().max(dy.abs()) + 120) / 121).max(1);
            let start = current;
            for step in 1..=parts {
                let next = (
                    start.0 + (f64::from(dx) * f64::from(step) / f64::from(parts)).round() as i32,
                    start.1 + (f64::from(dy) * f64::from(step) / f64::from(parts)).round() as i32,
                );
                append_record(
                    next.0 - current.0,
                    next.1 - current.1,
                    if moving {
                        stitch.command
                    } else {
                        Command::Jump
                    },
                    &mut body,
                );
                current = next;
                bounds.0 = bounds.0.min(next.0);
                bounds.1 = bounds.1.min(-next.1);
                bounds.2 = bounds.2.max(next.0);
                bounds.3 = bounds.3.max(-next.1);
            }
        }
        if !moving {
            append_record(0, 0, stitch.command, &mut body);
            if stitch.command == Command::ColorChange {
                colors += 1;
            }
            if stitch.command == Command::End {
                ended = true;
            }
        }
        if body.len() / 3 > 999_999 {
            return Err("Encoded design exceeds DST ST field capacity".into());
        }
    }
    if !ended {
        append_record(0, 0, Command::End, &mut body);
    }
    if colors > 999 {
        return Err("DST supports at most 999 header colour changes".into());
    }
    let mut safe_name: String = name
        .chars()
        .filter(char::is_ascii)
        .filter(|c| !c.is_control())
        .take(16)
        .collect();
    if safe_name.trim().is_empty() {
        safe_name = "EmbroideryLab".into();
    }
    let header = format!(
        "LA:{safe_name:<16}\rST:{:7}\rCO:{colors:3}\r+X:{:5}\r-X:{:5}\r+Y:{:5}\r-Y:{:5}\rAX:{:+06}\rAY:{:+06}\rMX:+00000\rMY:+00000\rPD:******\r",
        body.len() / 3,
        bounds.2,
        -bounds.0,
        bounds.3,
        -bounds.1,
        current.0,
        -current.1
    );
    let mut result = vec![b' '; 512];
    result[..header.len()].copy_from_slice(header.as_bytes());
    result[header.len()] = 0x1a;
    result.extend(body);
    Ok(result)
}

pub fn decode(bytes: &[u8]) -> Result<Vec<Stitch>, String> {
    if bytes.len() < 515 {
        return Err("DST requires a 512-byte header and END record".into());
    }
    if bytes.len() > 64_000_000 {
        return Err("DST input exceeds safe memory limit".into());
    }
    if !bytes[..512].windows(3).any(|x| x == b"LA:") {
        return Err("Missing DST LA header".into());
    }
    let mut result = Vec::new();
    let mut current = (0_i32, 0_i32);
    let mut color = 0_usize;
    let mut found_end = false;
    let body = &bytes[512..];
    for (index, record) in body.as_chunks::<3>().0.iter().enumerate() {
        if result.len() >= 1_000_000 {
            return Err("DST exceeds one million decoded commands".into());
        }
        if record[2] & 3 != 3 {
            return Err(format!("Invalid low control bits at DST record {index}"));
        }
        if record[2] & 0xf3 == 0xf3 {
            result.push(Stitch {
                x: f64::from(current.0) / 10.0,
                y: f64::from(current.1) / 10.0,
                command: Command::End,
                color: PALETTE[color % PALETTE.len()],
                object_id: color as u64,
            });
            let trailing = &body[(index + 1) * 3..];
            if trailing.iter().any(|&b| b != 0 && b != 0x1a && b != b' ') {
                return Err("Unexpected non-padding data after DST END".into());
            }
            found_end = true;
            break;
        }
        let command = match record[2] & 0xc3 {
            0xc3 => Command::ColorChange,
            0x83 => Command::Jump,
            0x03 => Command::Stitch,
            _ => return Err(format!("Sequin-mode DST is not supported (record {index})")),
        };
        current.0 += decode_axis(record, &X_BITS)?;
        current.1 -= decode_axis(record, &Y_BITS)?;
        if command == Command::ColorChange {
            let point = Point::new(f64::from(current.0) / 10.0, f64::from(current.1) / 10.0);
            let previous = result.last().map_or(Point::default(), |s| s.point());
            if point.distance(previous) > 1e-9 {
                result.push(Stitch {
                    x: point.x,
                    y: point.y,
                    command: Command::Jump,
                    color: PALETTE[color % PALETTE.len()],
                    object_id: color as u64,
                });
            }
            color += 1;
        }
        result.push(Stitch {
            x: f64::from(current.0) / 10.0,
            y: f64::from(current.1) / 10.0,
            command,
            color: PALETTE[color % PALETTE.len()],
            object_id: color as u64,
        });
    }
    if !found_end {
        return Err("DST is truncated or lacks END".into());
    }
    Ok(result)
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    pub commands: usize,
    pub stitches: usize,
    pub jumps: usize,
    pub color_changes: usize,
    pub trims: usize,
    pub width_mm: f64,
    pub height_mm: f64,
    pub sewn_width_mm: f64,
    pub sewn_height_mm: f64,
    pub stitch_length_mm: f64,
    pub jump_length_mm: f64,
    pub max_stitch_mm: f64,
}
pub fn stats(stitches: &[Stitch]) -> Stats {
    let mut out = Stats::default();
    let mut previous = Point::default();
    let mut bounds: Option<(f64, f64, f64, f64)> = None;
    let mut sewn: Option<(f64, f64, f64, f64)> = None;
    for &s in stitches {
        out.commands += 1;
        let p = s.point();
        let distance = p.distance(previous);
        match s.command {
            Command::Stitch => {
                out.stitches += 1;
                out.stitch_length_mm += distance;
                out.max_stitch_mm = out.max_stitch_mm.max(distance);
                let extrema = (
                    previous.x.min(p.x),
                    previous.y.min(p.y),
                    previous.x.max(p.x),
                    previous.y.max(p.y),
                );
                sewn = Some(match sewn {
                    Some((x0, y0, x1, y1)) => (
                        x0.min(extrema.0),
                        y0.min(extrema.1),
                        x1.max(extrema.2),
                        y1.max(extrema.3),
                    ),
                    None => extrema,
                });
            }
            Command::Jump => {
                out.jumps += 1;
                out.jump_length_mm += distance;
            }
            Command::ColorChange => out.color_changes += 1,
            Command::Trim => out.trims += 1,
            Command::End => break,
        }
        if matches!(s.command, Command::Stitch | Command::Jump) {
            bounds = Some(match bounds {
                Some((x0, y0, x1, y1)) => (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
                None => (p.x, p.y, p.x, p.y),
            });
        }
        previous = p;
    }
    if let Some((x0, y0, x1, y1)) = bounds {
        out.width_mm = x1 - x0;
        out.height_mm = y1 - y0;
    }
    if let Some((x0, y0, x1, y1)) = sewn {
        out.sewn_width_mm = x1 - x0;
        out.sewn_height_mm = y1 - y0;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_representable_axis_values() {
        for value in -121..=121 {
            for bits in [&X_BITS, &Y_BITS] {
                let mut out = [0, 0, 3];
                encode_axis(value, bits, &mut out);
                assert_eq!(decode_axis(&out, bits).unwrap(), value);
            }
        }
    }
    #[test]
    fn known_vectors_and_y_convention() {
        let s = Stitch {
            x: 0.1,
            y: 0.1,
            command: Command::Stitch,
            color: [0; 3],
            object_id: 1,
        };
        let bytes = encode(&[s], "test").unwrap();
        assert_eq!(&bytes[512..515], &[0x41, 0, 3]);
        assert_eq!(decode(&bytes).unwrap()[0].point(), s.point());
    }
    #[test]
    fn long_displacements_split() {
        let s = Stitch {
            x: 100.0,
            y: -50.0,
            command: Command::Jump,
            color: [0; 3],
            object_id: 1,
        };
        let decoded = decode(&encode(&[s], "jump").unwrap()).unwrap();
        assert_eq!(decoded[decoded.len() - 2].point(), s.point());
        assert!(decoded.len() > 2);
    }
    #[test]
    fn rejects_truncation_and_sequin() {
        let s = Stitch {
            x: 0.,
            y: 0.,
            command: Command::Stitch,
            color: [0; 3],
            object_id: 1,
        };
        let mut bytes = encode(&[s], "test").unwrap();
        bytes.truncate(515);
        assert!(decode(&bytes).is_err());
        let mut bytes = encode(&[s], "test").unwrap();
        bytes[514] = 0x43;
        assert!(decode(&bytes).is_err());
    }
    #[test]
    fn header_extents_use_machine_y() {
        let stitches = [
            Stitch {
                x: 2.,
                y: 5.,
                command: Command::Stitch,
                color: [0; 3],
                object_id: 1,
            },
            Stitch {
                x: -3.,
                y: -2.,
                command: Command::Jump,
                color: [0; 3],
                object_id: 1,
            },
        ];
        let bytes = encode(&stitches, "asymmetric").unwrap();
        let header = String::from_utf8_lossy(&bytes[..512]);
        assert!(header.contains("+X:   20\r"));
        assert!(header.contains("-X:   30\r"));
        assert!(header.contains("+Y:   20\r"));
        assert!(header.contains("-Y:   50\r"));
    }
}

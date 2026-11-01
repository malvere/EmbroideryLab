use embroidery_lab::{
    Command, Stitch,
    dsb::{self, DsbCommand as C, DsbRecord as R},
};
fn file(records: &[[u8; 3]]) -> Vec<u8> {
    let mut bytes = vec![b' '; 512];
    for record in records {
        bytes.extend_from_slice(record);
    }
    bytes
}
fn record(x: f64, y: f64, command: C) -> R {
    R { x, y, command }
}

#[test]
fn independent_known_vectors_decode_control_y_x_and_signs() {
    let raw = file(&[[0x80, 10, 20], [0xe0, 30, 40], [0xa1, 5, 7], [0xf8, 0, 0]]);
    assert_eq!(
        dsb::decode(&raw).unwrap(),
        vec![
            record(2., -1., C::Stitch),
            record(-2., 2., C::Stitch),
            record(-2.7, 1.5, C::Jump),
            record(-2.7, 1.5, C::End)
        ]
    );
}

#[test]
fn all_signed_axis_magnitudes_roundtrip_and_use_canonical_high_bit() {
    for value in -255_i32..=255 {
        let x = f64::from(value) / 10.;
        let y = -x;
        let records = [record(x, y, C::Stitch), record(x, y, C::End)];
        let bytes = dsb::encode(&records, "Axis").unwrap();
        assert_eq!(bytes[512] & 0x80, 0x80);
        assert_eq!(bytes[513], value.unsigned_abs() as u8);
        assert_eq!(bytes[514], value.unsigned_abs() as u8);
        assert_eq!(dsb::decode(&bytes).unwrap(), records);
    }
}

#[test]
fn all_supported_commands_remain_distinct_in_raw_roundtrip() {
    let mut records = vec![
        record(12.3, -1.1, C::Jump),
        record(12.5, -0.5, C::Stitch),
        record(12.5, -0.5, C::Trim),
        record(12.5, -0.5, C::Stop),
    ];
    for needle in 1..=15 {
        records.push(record(12.5, -0.5, C::NeedleChange(needle)));
    }
    records.push(record(12.5, -0.5, C::End));
    assert_eq!(
        dsb::decode(&dsb::encode(&records, "Commands").unwrap()).unwrap(),
        records
    );
    let preview = dsb::preview(&records).unwrap();
    assert_eq!(preview.len(), records.len());
    assert_eq!(preview[2].command, Command::Jump);
    assert_eq!(preview[2].point(), preview[1].point());
    assert_eq!(
        preview
            .iter()
            .filter(|s| s.command == Command::ColorChange)
            .count(),
        16
    );
}

#[test]
fn long_displacements_split_without_changing_endpoint() {
    let bytes = dsb::encode(&[record(100., -75., C::Jump)], "Long").unwrap();
    let decoded = dsb::decode(&bytes).unwrap();
    assert!(decoded.len() > 2);
    assert_eq!(*decoded.last().unwrap(), record(100., -75., C::End));
    let mut previous = (0., 0.);
    for r in decoded {
        if matches!(r.command, C::Jump | C::Stitch) {
            assert!((r.x - previous.0).abs() <= 25.500001);
            assert!((r.y - previous.1).abs() <= 25.500001);
        }
        previous = (r.x, r.y);
    }
}

#[test]
fn header_extents_are_machine_y_and_st_counts_end() {
    let bytes = dsb::encode(
        &[
            record(2., -3., C::Stitch),
            record(-5., 4., C::Jump),
            record(-5., 4., C::End),
        ],
        "Header",
    )
    .unwrap();
    let header = String::from_utf8_lossy(&bytes[..512]);
    for field in [
        "ST:      3\r",
        "CO:  0\r",
        "+X:   20\r",
        "-X:   50\r",
        "+Y:   30\r",
        "-Y:   40\r",
        "AX:-00050\r",
        "AY:-00040\r",
    ] {
        assert!(header.contains(field), "{field:?} missing in {header:?}");
    }
}

#[test]
fn unsupported_truncated_and_after_end_data_are_errors() {
    assert!(dsb::decode(&vec![0; 512]).is_err());
    assert!(
        dsb::decode(&file(&[[0x82, 0, 0], [0xf8, 0, 0]]))
            .unwrap_err()
            .contains("Unsupported")
    );
    assert!(dsb::decode(&file(&[[0x80, 0, 1]])).is_err());
    let mut truncated = file(&[[0x80, 0, 1]]);
    truncated.push(0xf8);
    assert!(dsb::decode(&truncated).is_err());
    assert!(dsb::decode(&file(&[[0xf8, 0, 0], [0x80, 0, 1]])).is_err());
    let mut padded = file(&[[0xf8, 0, 0]]);
    padded.extend([0, 0x1a, b' ']);
    assert!(dsb::decode(&padded).is_ok());
}

#[test]
fn decoder_enforces_record_cap_without_silent_truncation() {
    let mut bytes = vec![b' '; 512];
    bytes.extend([0x80, 0, 0].repeat(1_000_000));
    bytes.extend([0xf8, 0, 0]);
    assert!(dsb::decode(&bytes).unwrap_err().contains("one million"));
}

#[test]
fn invalid_typed_records_do_not_silently_drop_semantics() {
    assert!(dsb::encode(&[record(0., 0., C::End), record(0., 0., C::Stitch)], "Bad").is_err());
    for needle in [0, 16] {
        assert!(dsb::encode(&[record(0., 0., C::NeedleChange(needle))], "Needle").is_err());
    }
    assert!(dsb::encode(&[record(f64::NAN, 0., C::Stitch)], "NaN").is_err());
    assert!(dsb::preview(&[record(f64::NAN, 0., C::End)]).is_err());
    assert!(dsb::encode(&[record(1., 0., C::Stop)], "Moving Stop").is_err());
    assert!(dsb::preview(&[record(0., 0., C::Stitch)]).is_err());
    let too_many = vec![record(0., 0., C::Stitch); 1_000_001];
    assert!(dsb::encode(&too_many, "Large").is_err());
}

#[test]
fn project_color_change_is_explicit_manual_stop() {
    let stitch = |x, y, command| Stitch {
        x,
        y,
        command,
        color: [200, 30, 40],
        object_id: 1,
    };
    let source = [
        stitch(1., 2., Command::Jump),
        stitch(1.5, 2.2, Command::Stitch),
        stitch(1.5, 2.2, Command::ColorChange),
        stitch(1.5, 2.2, Command::End),
    ];
    let records = dsb::from_stitches(&source).unwrap();
    assert_eq!(records[2].command, C::Stop);
    let raw = dsb::encode(&records, "Manual stop").unwrap();
    assert_eq!(raw[512 + 2 * 3], 0xe8);
    assert!(
        records
            .iter()
            .all(|r| !matches!(r.command, C::NeedleChange(_)))
    );
}

#[test]
fn nonmoving_payload_is_ignored_as_in_public_reader() {
    let records = dsb::decode(&file(&[
        [0x80, 10, 20],
        [0xe7, 23, 45],
        [0xe8, 2, 3],
        [0xe9, 4, 5],
        [0xf8, 6, 7],
    ]))
    .unwrap();
    assert!(records.iter().all(|r| r.x == 2. && r.y == -1.));
    assert_eq!(records[1].command, C::Trim);
    assert_eq!(records[3].command, C::NeedleChange(1));
}

#[test]
#[ignore = "requires the read-only installed Embird DSB corpus on this Windows VM"]
fn installed_corpus_preserves_all_record_semantics() {
    for name in ["Cat.DSB", "CHR.DSB", "Rose-2.DSB"] {
        let original =
            std::fs::read(std::path::Path::new("C:\\EMBIRD32\\Files").join(name)).unwrap();
        let records = dsb::decode(&original).unwrap();
        assert!(records.len() > 9000);
        let encoded = dsb::encode(&records, name).unwrap();
        assert_eq!(dsb::decode(&encoded).unwrap(), records);
    }
}

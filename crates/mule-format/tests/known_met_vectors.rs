// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_format::ed2k::hash::hash_reader;
use mule_format::met::tags::*;
use mule_format::{KnownMetFile, MetError, MET_HEADER_I64TAGS};
use mule_test_support::met_fixtures::*;
use std::io::Cursor;

/// Construye un registro de `known.met` a partir de una carga útil hasheada con el
/// hasheador eD2K real para que la autocomprobación raíz/hashes de partes pase con éxito.
fn push_record(
    buf: &mut Vec<u8>,
    payload: &[u8],
    name: &str,
    last_modified: u32,
    extra: &mut dyn FnMut(&mut Vec<u8>),
) {
    let hashed = hash_reader(&mut Cursor::new(payload)).unwrap();
    buf.extend_from_slice(&last_modified.to_le_bytes());
    buf.extend_from_slice(hashed.root_hash.as_bytes());
    buf.extend_from_slice(&(hashed.part_hashes.len() as u16).to_le_bytes());
    for ph in &hashed.part_hashes {
        buf.extend_from_slice(ph);
    }

    let mut tags = Vec::new();
    push_string_tag(&mut tags, FT_FILENAME, name);
    push_u64_tag(&mut tags, FT_FILESIZE, hashed.total_bytes);
    extra(&mut tags);

    buf.extend_from_slice(&(count_tags(&tags)).to_le_bytes());
    buf.extend_from_slice(&tags);
}

/// Cuenta el número de etiquetas serializadas en `tags` reanalizándolas.
fn count_tags(tags: &[u8]) -> u32 {
    let mut cr = mule_format::met::reader::CountingReader::new(Cursor::new(tags));
    let mut n = 0u32;
    while cr.read_tag().is_ok() {
        n += 1;
    }
    n
}

#[test]
fn test_known_met_nominal_two_records() {
    let payload_a = vec![0xAAu8; 5_000]; // < 1 part → 1 hash
    let payload_b = vec![0xBBu8; 10_000_000]; // > 1 part → 2 hashes

    let mut buf = vec![MET_HEADER_I64TAGS];
    buf.extend_from_slice(&2u32.to_le_bytes());
    push_record(&mut buf, &payload_a, "small.bin", 1_700_000_000, &mut |t| {
        push_u32_tag(t, FT_LASTSHARED, 1_699_999_000);
        push_u32_tag(t, FT_ATTRANSFERRED, 123_456);
        push_u32_tag(t, FT_ATTRANSFERREDHI, 2);
        push_u32_tag(t, FT_ATREQUESTED, 77);
        push_u32_tag(t, FT_ATACCEPTED, 55);
        push_u32_tag(t, FT_ULPRIORITY, 2); // PR_HIGH
        push_u32_tag(t, FT_KADLASTPUBLISHSRC, 1_690_000_000);
        push_u32_tag(t, FT_FLAGS, 2);
    });
    push_record(&mut buf, &payload_b, "big.bin", 1_700_000_100, &mut |t| {
        push_u32_tag(t, FT_ULPRIORITY, 5); // PR_AUTO → PR_HIGH
    });

    let met = KnownMetFile::read_from(&mut Cursor::new(&buf)).unwrap();
    assert!(met.i64_tags);
    assert_eq!(met.records.len(), 2);
    assert_eq!(met.dropped_records, 0);

    let a = &met.records[0];
    assert_eq!(a.last_modified_utc, 1_700_000_000);
    assert_eq!(a.file_name.as_deref(), Some("small.bin"));
    assert_eq!(a.file_size, Some(5_000));
    assert_eq!(a.part_hashes.len(), 1);
    assert_eq!(a.last_shared_utc, Some(1_699_999_000));
    assert_eq!(a.transferred_bytes, (2u64 << 32) | 123_456);
    assert_eq!(a.requested, 77);
    assert_eq!(a.accepted, 55);
    assert_eq!(a.up_priority, Some(2));
    assert_eq!(a.kad_last_publish_src, Some(1_690_000_000));
    assert_eq!(a.meta_data_ver, 2);

    let b = &met.records[1];
    assert_eq!(b.file_name.as_deref(), Some("big.bin"));
    assert_eq!(b.file_size, Some(10_000_000));
    // 10_000_000 / 9_728_000 → suelo 1 + 1 = 2 partes (parcial final)
    assert_eq!(b.part_hashes.len(), 2);
    assert_eq!(b.up_priority, Some(2)); // PR_AUTO resuelto a PR_HIGH
}

#[test]
fn test_known_met_drops_record_with_wrong_root() {
    let payload = vec![0xCCu8; 5_000];
    let hashed = hash_reader(&mut Cursor::new(&payload)).unwrap();

    let mut buf = vec![MET_HEADER_I64TAGS];
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.extend_from_slice(&1_700_000_000u32.to_le_bytes());
    // Hash raíz corrupto (no coincide con el conjunto de hashes)
    buf.extend_from_slice(&[0xDE; 16]);
    buf.extend_from_slice(&(hashed.part_hashes.len() as u16).to_le_bytes());
    for ph in &hashed.part_hashes {
        buf.extend_from_slice(ph);
    }
    let mut tags = Vec::new();
    push_string_tag(&mut tags, FT_FILENAME, "corrupt.bin");
    push_u64_tag(&mut tags, FT_FILESIZE, hashed.total_bytes);
    buf.extend_from_slice(&(count_tags(&tags)).to_le_bytes());
    buf.extend_from_slice(&tags);

    let met = KnownMetFile::read_from(&mut Cursor::new(&buf)).unwrap();
    assert_eq!(met.records.len(), 0);
    assert_eq!(met.dropped_records, 1);
}

#[test]
fn test_known_met_drops_record_with_wrong_part_count() {
    // file_size declara 2 partes pero el conjunto de hashes almacena 1
    let payload = vec![0xDDu8; 5_000];
    let hashed = hash_reader(&mut Cursor::new(&payload)).unwrap();

    let mut buf = vec![MET_HEADER_I64TAGS];
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.extend_from_slice(&1_700_000_000u32.to_le_bytes());
    buf.extend_from_slice(hashed.root_hash.as_bytes());
    buf.extend_from_slice(&(hashed.part_hashes.len() as u16).to_le_bytes());
    for ph in &hashed.part_hashes {
        buf.extend_from_slice(ph);
    }
    let mut tags = Vec::new();
    push_string_tag(&mut tags, FT_FILENAME, "count.bin");
    // Engaño: simular que el archivo tiene > 1 parte → recuento esperado = 2
    push_u64_tag(&mut tags, FT_FILESIZE, 10_000_000);
    buf.extend_from_slice(&(count_tags(&tags)).to_le_bytes());
    buf.extend_from_slice(&tags);

    let met = KnownMetFile::read_from(&mut Cursor::new(&buf)).unwrap();
    assert_eq!(met.records.len(), 0);
    assert_eq!(met.dropped_records, 1);
}

#[test]
fn test_known_met_bad_header_rejected() {
    let buf = vec![0x11, 0x00, 0x00, 0x00, 0x00];
    let err = KnownMetFile::read_from(&mut Cursor::new(&buf)).unwrap_err();
    assert!(matches!(
        err,
        MetError::UnsupportedHeader {
            offset: 0,
            value: 0x11
        }
    ));
}

#[test]
fn test_known_met_oversized_record_count_rejected() {
    let mut buf = vec![MET_HEADER_I64TAGS];
    buf.extend_from_slice(&600_000u32.to_le_bytes());
    let err = KnownMetFile::read_from(&mut Cursor::new(&buf)).unwrap_err();
    assert!(matches!(err, MetError::InvalidLength { .. }));
}

#[test]
fn test_known_met_truncated_eof() {
    let mut buf = vec![MET_HEADER_I64TAGS];
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.extend_from_slice(&1_700_000_000u32.to_le_bytes());
    // no le siguen datos de hash
    let err = KnownMetFile::read_from(&mut Cursor::new(&buf)).unwrap_err();
    assert!(matches!(err, MetError::UnexpectedEof { .. }));
}

#[test]
fn test_known_met_extra_tags_preserved() {
    let payload = vec![0xEEu8; 4_000];
    let mut buf = vec![MET_HEADER_I64TAGS];
    buf.extend_from_slice(&1u32.to_le_bytes());
    push_record(&mut buf, &payload, "song.mp3", 1_700_000_000, &mut |t| {
        // FT_MEDIA_ARTIST = 0xD0 — va a extra_tags (ruta ConvertED2KTag)
        push_string_tag(t, 0xD0, "Some Artist");
    });
    let met = KnownMetFile::read_from(&mut Cursor::new(&buf)).unwrap();
    let rec = &met.records[0];
    assert_eq!(rec.extra_tags.len(), 1);
    assert_eq!(rec.extra_tags[0].name_id, 0xD0);
    assert_eq!(rec.extra_tags[0].as_str(), Some("Some Artist"));
}

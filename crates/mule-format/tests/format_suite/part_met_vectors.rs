// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_format::{MetError, PartMetFile, PARTFILE_VERSION};
use std::io::Cursor;

fn build_valid_part_met_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    // Cabecera
    bytes.push(PARTFILE_VERSION); // 0xe0 (desplazamiento 0)
    bytes.extend_from_slice(&1_700_000_000u32.to_le_bytes()); // fecha de modificación (desplazamiento 1..5)
    bytes.extend_from_slice(&[0xaa; 16]); // hash de archivo (desplazamiento 5..21)

    // Recuento de hashes de partes: 1
    bytes.extend_from_slice(&1u16.to_le_bytes()); // recuento (desplazamiento 21..23)
    bytes.extend_from_slice(&[0xbb; 16]); // hash de parte 0 (desplazamiento 23..39)

    // Recuento de etiquetas: 2 (desplazamiento 39..43)
    bytes.extend_from_slice(&2u32.to_le_bytes());

    // Etiqueta 1: FT_FILENAME (0x01), tipo cadena (0x02)
    // Usando cabecera de nombre corto: raw_type | 0x80 = 0x82, name_id = 0x01
    bytes.push(0x82); // tipo de etiqueta 0x02 + bandera 0x80
    bytes.push(0x01); // FT_FILENAME
    let filename = b"video.mp4";
    bytes.extend_from_slice(&(filename.len() as u16).to_le_bytes());
    bytes.extend_from_slice(filename);

    // Etiqueta 2: FT_FILESIZE (0x02), tipo uint32 (0x03)
    bytes.push(0x83); // tipo de etiqueta 0x03 + bandera 0x80
    bytes.push(0x02); // FT_FILESIZE
    bytes.extend_from_slice(&10_485_760u32.to_le_bytes()); // 10 MiB

    bytes
}

#[test]
fn test_valid_part_met_parsing() {
    let raw = build_valid_part_met_bytes();
    let mut cursor = Cursor::new(&raw);
    let met = PartMetFile::read_from(&mut cursor).unwrap();

    assert_eq!(met.version, PARTFILE_VERSION);
    assert_eq!(met.file_name.as_deref(), Some("video.mp4"));
    assert_eq!(met.file_size, Some(10_485_760));
    assert_eq!(met.part_hashes.len(), 1);
    assert_eq!(met.part_hashes[0], [0xbb; 16]);
}

#[test]
fn test_unsupported_header_offset() {
    let mut raw = build_valid_part_met_bytes();
    raw[0] = 0x42; // Invalid header version

    let mut cursor = Cursor::new(&raw);
    let err = PartMetFile::read_from(&mut cursor).unwrap_err();

    assert_eq!(
        err,
        MetError::UnsupportedHeader {
            offset: 0,
            value: 0x42
        }
    );
}

#[test]
fn test_unexpected_eof_at_various_offsets() {
    let raw = build_valid_part_met_bytes();

    // 1. Truncar en el desplazamiento 0 (flujo vacío)
    let mut cursor = Cursor::new(&[]);
    let err = PartMetFile::read_from(&mut cursor).unwrap_err();
    assert_eq!(
        err,
        MetError::UnexpectedEof {
            offset: 0,
            context: "version header"
        }
    );

    // 2. Truncar antes de que la fecha de modificación esté completa (longitud 3)
    let mut cursor = Cursor::new(&raw[..3]);
    let err = PartMetFile::read_from(&mut cursor).unwrap_err();
    assert_eq!(
        err,
        MetError::UnexpectedEof {
            offset: 1,
            context: "mod date"
        }
    );

    // 3. Truncar en el hash del archivo (longitud 10)
    let mut cursor = Cursor::new(&raw[..10]);
    let err = PartMetFile::read_from(&mut cursor).unwrap_err();
    assert_eq!(
        err,
        MetError::UnexpectedEof {
            offset: 5,
            context: "file hash"
        }
    );
}

#[test]
fn test_invalid_tag_type_reports_offset() {
    let mut raw = build_valid_part_met_bytes();
    // Desplazamiento donde comienza la etiqueta 1: 43
    // El tipo sin procesar de la etiqueta 1 está en el desplazamiento 43
    raw[43] = 0x80 | 0x7e; // El tipo de etiqueta 0x7e no es válido

    let mut cursor = Cursor::new(&raw);
    let err = PartMetFile::read_from(&mut cursor).unwrap_err();

    assert_eq!(
        err,
        MetError::InvalidTagType {
            offset: 43,
            tag_type: 0x7e
        }
    );
}

/// `.part.met` de prueba estándar (archivo de 1000 bytes) parametrizado por sus huecos (gaps).
fn gaps_met(transferred: u64, gaps: Vec<(u64, u64)>) -> PartMetFile {
    PartMetFile {
        version: PARTFILE_VERSION,
        mod_date: 0,
        file_hash: mule_format::Ed2kHash([0; 16]),
        part_hashes: vec![],
        file_name: Some("test.bin".into()),
        file_size: Some(1000),
        transferred,
        aich_hash: None,
        gaps,
        tags: vec![],
    }
}

#[test]
fn test_compute_completed_ranges_nominal() {
    let met = gaps_met(400, vec![(0, 200), (600, 800)]);

    let completed = met.compute_completed_ranges().unwrap();
    assert_eq!(completed, vec![(200, 600), (800, 1000)]);
}

#[test]
fn test_compute_completed_ranges_overlapping_gaps() {
    let met = gaps_met(500, vec![(100, 300), (250, 500)]);

    let completed = met.compute_completed_ranges().unwrap();
    // El hueco fusionado es 100..500, por lo que lo completado es 0..100 y 500..1000
    assert_eq!(completed, vec![(0, 100), (500, 1000)]);
}

#[test]
fn test_compute_completed_ranges_full_gap_and_empty() {
    let met_empty = gaps_met(0, vec![(0, 1000)]);
    assert_eq!(met_empty.compute_completed_ranges().unwrap(), vec![]);

    let met_full = gaps_met(1000, vec![]);
    assert_eq!(
        met_full.compute_completed_ranges().unwrap(),
        vec![(0, 1000)]
    );
}

#[test]
fn test_compute_completed_ranges_invalid_gap_boundary() {
    let met = gaps_met(100, vec![(200, 1200)]); // Exceeds file size 1000
    assert!(met.compute_completed_ranges().is_err());
}

#[test]
fn test_part_met_write_read_roundtrip() {
    use mule_format::{AichHash, Ed2kHash};

    let met = PartMetFile {
        version: PARTFILE_VERSION,
        mod_date: 1_700_000_123,
        file_hash: Ed2kHash([0xaa; 16]),
        part_hashes: vec![[0x11; 16], [0x22; 16]],
        file_name: Some("movie.mkv".into()),
        file_size: Some(19_000_000),
        transferred: 5_000_000,
        aich_hash: Some(AichHash([0x55; 20])),
        gaps: vec![(5_000_000, 10_000_000), (15_000_000, 19_000_000)],
        tags: vec![],
    };

    let mut buf = Vec::new();
    met.write_to(&mut buf).unwrap();

    let mut cursor = Cursor::new(&buf);
    let parsed = PartMetFile::read_from(&mut cursor).unwrap();

    assert_eq!(parsed.version, PARTFILE_VERSION);
    assert_eq!(parsed.mod_date, 1_700_000_123);
    assert_eq!(parsed.file_hash, Ed2kHash([0xaa; 16]));
    assert_eq!(parsed.part_hashes, vec![[0x11; 16], [0x22; 16]]);
    assert_eq!(parsed.file_name.as_deref(), Some("movie.mkv"));
    assert_eq!(parsed.file_size, Some(19_000_000));
    assert_eq!(parsed.transferred, 5_000_000);
    assert_eq!(parsed.aich_hash, Some(AichHash([0x55; 20])));
    assert_eq!(
        parsed.gaps,
        vec![(5_000_000, 10_000_000), (15_000_000, 19_000_000)]
    );
    // Los huecos invertidos deben reconstruir el rango completado.
    let done = parsed.compute_completed_ranges().unwrap();
    assert_eq!(done, vec![(0, 5_000_000), (10_000_000, 15_000_000)]);
}

#[test]
fn test_part_met_write_large_file_uses_0xe2_and_int64() {
    use mule_format::{Ed2kHash, PARTFILE_VERSION_LARGEFILE};

    let met = PartMetFile {
        version: PARTFILE_VERSION,
        mod_date: 1,
        file_hash: Ed2kHash([0xaa; 16]),
        part_hashes: vec![],
        file_name: Some("huge.iso".into()),
        file_size: Some(5_000_000_000),
        transferred: 0,
        aich_hash: None,
        gaps: vec![],
        tags: vec![],
    };

    let mut buf = Vec::new();
    met.write_to(&mut buf).unwrap();
    assert_eq!(buf[0], PARTFILE_VERSION_LARGEFILE);

    let mut cursor = Cursor::new(&buf);
    let parsed = PartMetFile::read_from(&mut cursor).unwrap();
    assert_eq!(parsed.version, PARTFILE_VERSION_LARGEFILE);
    assert_eq!(parsed.file_size, Some(5_000_000_000));
}

#[test]
fn test_part_met_save_and_open_via_filesystem() {
    use mule_format::Ed2kHash;

    let dir = std::env::temp_dir().join(format!("mule_partmet_save_open_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let met_path = dir.join("001.part.met");

    let met = PartMetFile {
        version: PARTFILE_VERSION,
        mod_date: 42,
        file_hash: Ed2kHash([0xcc; 16]),
        part_hashes: vec![],
        file_name: Some("x.bin".into()),
        file_size: Some(1024),
        transferred: 1024,
        aich_hash: None,
        gaps: vec![],
        tags: vec![],
    };
    met.save(&met_path).unwrap();

    let parsed = PartMetFile::open(&met_path).unwrap();
    assert_eq!(parsed.file_hash, Ed2kHash([0xcc; 16]));
    assert_eq!(parsed.file_name.as_deref(), Some("x.bin"));
    assert_eq!(parsed.transferred, 1024);
    // Archivo temporal limpiado
    assert!(!dir.join("001.part.met.tmp").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

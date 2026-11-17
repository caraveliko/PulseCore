// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use mule_cli::commands::import_amule::run_import_amule;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_import_amule_directory_migration() {
    let amule_tmp = tempdir().unwrap();
    let amule_dir = amule_tmp.path();

    let dest_tmp = tempdir().unwrap();
    let dest_dir = dest_tmp.path().join(".mule");

    // 1. Crear un preferences.dat simulado (versión 0x01 + userhash de 16 bytes)
    let mut pref_bytes = vec![0x01]; // versión
    pref_bytes.extend_from_slice(&[0x42; 16]); // userhash
    fs::write(amule_dir.join("preferences.dat"), &pref_bytes).unwrap();

    // 2. Crear nodes.dat simulado
    fs::write(amule_dir.join("nodes.dat"), b"fake_nodes_data").unwrap();

    // 3. Crear directorio Temp simulado con descarga en curso
    let temp_sub = amule_dir.join("Temp");
    fs::create_dir_all(&temp_sub).unwrap();
    fs::write(temp_sub.join("001.part.met"), b"part_met_header").unwrap();
    fs::write(temp_sub.join("001.part"), b"some_partial_data_chunk_12345").unwrap();

    // 4. Ejecutar migración con link_parts: false
    let summary = run_import_amule(amule_dir, &dest_dir, false, false).unwrap();

    assert!(summary.identity_imported);
    assert_eq!(summary.user_hash, Some(hex::encode([0x42; 16])));
    assert!(summary.nodes_imported);
    assert_eq!(summary.parts_migrated, 1);
    assert_eq!(summary.total_part_bytes, 29); // longitud de "some_partial_data_chunk_12345"

    // Verificar que los archivos de destino existen
    assert!(dest_dir.join("nodes.dat").exists());
    assert!(dest_dir.join("temp/001.part.met").exists());
    assert!(dest_dir.join("temp/001.part").exists());
}

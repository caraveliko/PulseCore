// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_core::{Database, Download, DownloadCategory};
use mule_format::Ed2kHash;
use std::path::PathBuf;
use uuid::Uuid;

#[test]
fn test_categories_crud_and_reorder() {
    let mut db = Database::open_in_memory().expect("failed to open in-memory db");

    let cats = db.get_categories().expect("failed to get categories");
    assert!(cats.is_empty());

    let mut cat1 = DownloadCategory::new(
        "Movies",
        "#E91E63",
        0,
        Some("D:/Media/Movies".into()),
        Some("high".into()),
    );
    cat1.id = "cat-movies".into();

    let mut cat2 = DownloadCategory::new(
        "Music",
        "#2196F3",
        1,
        Some("D:/Media/Music".into()),
        Some("normal".into()),
    );
    cat2.id = "cat-music".into();

    let mut cat3 = DownloadCategory::new("Archives", "#4CAF50", 2, None, None);
    cat3.id = "cat-archives".into();

    db.insert_category(&cat1).expect("insert cat1");
    db.insert_category(&cat2).expect("insert cat2");
    db.insert_category(&cat3).expect("insert cat3");

    let list = db.get_categories().expect("list categories");
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].id, "cat-movies");
    assert_eq!(list[1].id, "cat-music");
    assert_eq!(list[2].id, "cat-archives");

    // Recuperación de categoría individual
    let fetched = db
        .get_category("cat-music")
        .expect("get cat-music")
        .expect("exists");
    assert_eq!(fetched.name, "Music");
    assert_eq!(fetched.color, "#2196F3");
    assert_eq!(
        fetched.default_destination.as_deref(),
        Some("D:/Media/Music")
    );
    assert_eq!(fetched.default_priority.as_deref(), Some("normal"));

    // Actualización
    let mut updated_cat1 = cat1.clone();
    updated_cat1.name = "Cinema & Films".into();
    updated_cat1.color = "#9C27B0".into();
    updated_cat1.default_priority = Some("low".into());
    let ok = db.update_category(&updated_cat1).expect("update");
    assert!(ok);

    let fetched_updated = db.get_category("cat-movies").unwrap().unwrap();
    assert_eq!(fetched_updated.name, "Cinema & Films");
    assert_eq!(fetched_updated.color, "#9C27B0");
    assert_eq!(fetched_updated.default_priority.as_deref(), Some("low"));

    // Reordenación
    db.reorder_categories(&[
        "cat-archives".into(),
        "cat-movies".into(),
        "cat-music".into(),
    ])
    .expect("reorder");

    let reordered = db.get_categories().expect("reordered list");
    assert_eq!(reordered[0].id, "cat-archives");
    assert_eq!(reordered[0].sort_order, 0);
    assert_eq!(reordered[1].id, "cat-movies");
    assert_eq!(reordered[1].sort_order, 1);
    assert_eq!(reordered[2].id, "cat-music");
    assert_eq!(reordered[2].sort_order, 2);
}

#[test]
fn test_category_deletion_reassigns_downloads_without_deleting_them() {
    let mut db = Database::open_in_memory().expect("failed to open in-memory db");

    let mut cat = DownloadCategory::new("Books", "#FF9800", 0, None, None);
    cat.id = "cat-books".into();
    db.insert_category(&cat).expect("insert cat");

    let id1 = Uuid::new_v4();
    let mut dl1 = Download::new(
        "rust_book.pdf",
        10_000,
        Ed2kHash([1u8; 16]),
        None,
        PathBuf::from("downloads/rust_book.pdf"),
        PathBuf::from("temp/dl1.part"),
    );
    dl1.id = id1;
    dl1.category_id = Some("cat-books".into());

    let id2 = Uuid::new_v4();
    let mut dl2 = Download::new(
        "other_file.bin",
        20_000,
        Ed2kHash([2u8; 16]),
        None,
        PathBuf::from("downloads/other_file.bin"),
        PathBuf::from("temp/dl2.part"),
    );
    dl2.id = id2;
    dl2.category_id = None;

    db.insert_download(&dl1).expect("insert dl1");
    db.insert_download(&dl2).expect("insert dl2");

    let loaded = db.get_all_downloads().expect("load downloads");
    assert_eq!(loaded.len(), 2);
    let l1 = loaded.iter().find(|d| d.id == id1).unwrap();
    assert_eq!(l1.category_id.as_deref(), Some("cat-books"));
    let l2 = loaded.iter().find(|d| d.id == id2).unwrap();
    assert_eq!(l2.category_id, None);

    // Eliminar categoría: DEBE reasignar dl1 a NULL, mantener intactos tanto dl1 como dl2
    let reassigned = db.delete_category("cat-books").expect("delete category");
    assert_eq!(reassigned, 1);

    // Verificar que la categoría ha desaparecido
    let cat_fetch = db.get_category("cat-books").expect("fetch");
    assert!(cat_fetch.is_none());

    // Verificar que las descargas SIGUEN PRESENTES y dl1 tiene category_id == None
    let loaded_after = db.get_all_downloads().expect("load downloads after delete");
    assert_eq!(loaded_after.len(), 2);

    let l1_after = loaded_after.iter().find(|d| d.id == id1).unwrap();
    assert_eq!(l1_after.category_id, None);
    assert_eq!(l1_after.display_name, "rust_book.pdf");

    let l2_after = loaded_after.iter().find(|d| d.id == id2).unwrap();
    assert_eq!(l2_after.category_id, None);

    // Asignar manualmente la categoría de nuevo usando set_download_category
    let mut new_cat = DownloadCategory::new("Tech", "#00BCD4", 0, None, None);
    new_cat.id = "cat-tech".into();
    db.insert_category(&new_cat).expect("insert new_cat");

    let set_ok = db
        .set_download_category(&id1, Some("cat-tech"))
        .expect("set_download_category");
    assert!(set_ok);

    let loaded_final = db.get_all_downloads().expect("load downloads final");
    let l1_final = loaded_final.iter().find(|d| d.id == id1).unwrap();
    assert_eq!(l1_final.category_id.as_deref(), Some("cat-tech"));

    // Desasignar categoría con set_download_category(..., None)
    db.set_download_category(&id1, None)
        .expect("clear category");
    let loaded_cleared = db.get_all_downloads().expect("load downloads cleared");
    let l1_cleared = loaded_cleared.iter().find(|d| d.id == id1).unwrap();
    assert_eq!(l1_cleared.category_id, None);
}

#[test]
fn test_phase7_to_phase8_schema_migration() {
    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let db_path = tmp_dir.path().join("state.db");

    // 1. Crear una base de datos heredada de Fase 7 manualmente (sin category_id, sin tablas de historial/categorías/ajustes)
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open legacy db");
        conn.execute_batch(
            "
            CREATE TABLE downloads (
                id TEXT PRIMARY KEY,
                display_name TEXT NOT NULL,
                target_path TEXT NOT NULL,
                temp_path TEXT NOT NULL,
                total_size INTEGER NOT NULL,
                ed2k_hash BLOB NOT NULL,
                aich_root BLOB,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE received_ranges (
                download_id TEXT NOT NULL,
                start_offset INTEGER NOT NULL,
                end_offset INTEGER NOT NULL,
                PRIMARY KEY(download_id, start_offset)
            );

            CREATE TABLE verified_parts (
                download_id TEXT NOT NULL,
                part_index INTEGER NOT NULL,
                verified_at INTEGER NOT NULL,
                PRIMARY KEY(download_id, part_index)
            );
            ",
        )
        .expect("create legacy schema");

        // Insertar una descarga de Fase 7
        let legacy_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO downloads (id, display_name, target_path, temp_path, total_size, ed2k_hash, aich_root, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                legacy_id,
                "legacy_file.iso",
                "downloads/legacy_file.iso",
                "data/temp/legacy.part",
                100_000_000i64,
                [0xABu8; 16].as_slice(),
                Option::<&[u8]>::None,
                "paused",
                1700000000i64,
                1700000000i64,
            ],
        )
        .expect("insert legacy download");
    }

    // 2. Abrir base de datos heredada con Database::open (Fase 8.4)
    let mut db = Database::open(&db_path).expect("migrate legacy db to phase 8.4");

    // 3. Verificar que la descarga heredada se preserva y category_id es None
    let downloads = db.get_all_downloads().expect("fetch migrated downloads");
    assert_eq!(downloads.len(), 1);
    let dl = &downloads[0];
    assert_eq!(dl.display_name, "legacy_file.iso");
    assert_eq!(dl.category_id, None);
    assert_eq!(dl.status.as_str(), "paused");

    // 4. Verificar que las tablas de Fase 8.4 son totalmente funcionales
    let cat = DownloadCategory::new("ISOs", "#FF9800", 0, Some("downloads/ISOs".into()), None);
    db.insert_category(&cat)
        .expect("insert category after migration");
    let cats = db.get_categories().expect("fetch categories");
    assert_eq!(cats.len(), 1);

    // Asignar categoría a la descarga heredada
    db.set_download_category(&dl.id, Some(&cat.id))
        .expect("set category on legacy download");
    let updated_downloads = db.get_all_downloads().expect("fetch updated");
    assert_eq!(
        updated_downloads[0].category_id.as_deref(),
        Some(cat.id.as_str())
    );

    // La tabla de ajustes es funcional
    db.set_setting("retention_days", "60").expect("set setting");
    assert_eq!(
        db.get_setting("retention_days")
            .expect("get setting")
            .as_deref(),
        Some("60")
    );
}

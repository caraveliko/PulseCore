// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use mule_core::model::{EpisodeItem, SharedFile};
use mule_core::persistence::sqlite::Database;
use uuid::Uuid;

#[test]
fn test_insert_episodes_batch_and_indexes() {
    let mut db = Database::open_in_memory().expect("open in-memory db");
    let media_id = Uuid::new_v4().to_string();

    let mut episodes = Vec::new();
    for i in 1..=24 {
        episodes.push(EpisodeItem {
            id: Uuid::new_v4().to_string(),
            media_id: media_id.clone(),
            season_number: 1,
            episode_number: i,
            title: format!("Episode {i}"),
            air_date: Some("2026-01-15".to_string()),
            status: "wanted".to_string(),
            monitored: true,
            file_path: None,
            original_release_name: None,
            download_id: Some(format!("dl-{i}")),
            created_at: 1000,
            updated_at: 1000,
        });
    }

    let inserted = db.insert_episodes_batch(&episodes).expect("batch insert episodes");
    assert_eq!(inserted, 24);

    let loaded = db.get_episodes_for_media(&media_id).expect("get episodes");
    assert_eq!(loaded.len(), 24);
    assert_eq!(loaded[0].episode_number, 1);
    assert_eq!(loaded[23].episode_number, 24);

    // Verify find_episode_by_download_id uses indexed lookup
    let ep_dl = db.find_episode_by_download_id("dl-12").expect("lookup by download_id");
    assert!(ep_dl.is_some());
    assert_eq!(ep_dl.unwrap().episode_number, 12);
}

#[test]
fn test_upsert_shared_files_batch() {
    let mut db = Database::open_in_memory().expect("open in-memory db");

    let mut files = Vec::new();
    for i in 1..=10 {
        files.push(SharedFile {
            file_hash: format!("hash_{:032x}", i),
            file_name: format!("file_{i}.mkv"),
            file_size: 1024 * 1024 * i as u64,
            file_path: format!("/media/file_{i}.mkv"),
            aich_root: None,
            shared_at_unix_ms: 1000,
            enabled: true,
            up_priority: "normal".to_string(),
            is_partial: false,
            completed_parts: Vec::new(),
            uploaded_bytes: 0,
            upload_rate_bps: 0,
            category_id: None,
        });
    }

    let inserted = db.upsert_shared_files_batch(&files).expect("batch upsert shares");
    assert_eq!(inserted, 10);

    let loaded = db.list_shared_files().expect("list shares");
    assert_eq!(loaded.len(), 10);
}

#[test]
fn test_media_release_name_and_subtitle_blacklist() {
    let db = Database::open_in_memory().expect("open in-memory db");
    let media_id = Uuid::new_v4().to_string();

    let item = mule_core::model::MediaItem {
        id: media_id.clone(),
        tmdb_id: Some(603),
        imdb_id: Some("tt0133093".to_string()),
        tvdb_id: None,
        media_type: "movie".to_string(),
        title: "The Matrix".to_string(),
        original_title: None,
        year: Some(1999),
        overview: None,
        poster_path: None,
        backdrop_path: None,
        status: "completed".to_string(),
        monitored: true,
        quality_profile: "1080p-es".to_string(),
        root_folder: "/movies".to_string(),
        file_path: Some("/movies/The Matrix (1999)/The Matrix (1999) [1080p].mkv".to_string()),
        original_release_name: None,
        download_id: None,
        created_at: 1000,
        updated_at: 1000,
    };
    db.insert_media_item(&item).expect("insert media");
    db.set_media_original_release_name(&media_id, "The.Matrix.1999.1080p.BluRay.x264-SPARKS.mkv", 2000)
        .expect("set release name");

    let fetched = db.get_media_item(&media_id).expect("get media").expect("exists");
    assert_eq!(
        fetched.original_release_name.as_deref(),
        Some("The.Matrix.1999.1080p.BluRay.x264-SPARKS.mkv")
    );

    assert!(!db.is_subtitle_blacklisted(&media_id, "opensubtitles", "sub-123").unwrap());
    db.add_subtitle_blacklist(&media_id, "opensubtitles", "sub-123", Some("out_of_sync"), 3000)
        .expect("add blacklist");
    assert!(db.is_subtitle_blacklisted(&media_id, "opensubtitles", "sub-123").unwrap());
    let list = db.list_subtitle_blacklist(&media_id).expect("list blacklist");
    assert_eq!(list, vec![("opensubtitles".to_string(), "sub-123".to_string())]);
}

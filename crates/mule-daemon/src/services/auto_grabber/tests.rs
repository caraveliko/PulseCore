// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use super::*;

    #[test]
    fn test_blacklist_release_matches_hash_and_normalized_title() {
        let mut ag_state = AutoGrabberState::default();
        let target_id = "media-123";
        let now = 1_700_000_000_000;

        ag_state.blacklist_release(
            target_id,
            Some("abc123def"),
            "Pelicula.Ejemplo.(2024).1080p.Latino.mkv",
            WRONG_LANGUAGE_REASON,
            now,
        );

        assert!(ag_state.is_release_blacklisted(target_id, "ABC123DEF", "Otro.Nombre.mkv"));
        assert!(ag_state.is_release_blacklisted(
            target_id,
            "otrohash999",
            "Pelicula Ejemplo (2024) 1080p Latino mkv"
        ));
        assert!(!ag_state.is_release_blacklisted(
            target_id,
            "otrohash999",
            "Pelicula.Ejemplo.(2024).1080p.Castellano.mkv"
        ));

        // Sin hash conocido: genera clave sintética y sigue bloqueando por título normalizado
        ag_state.blacklist_release(
            "ep-456",
            None,
            "Mi.Serie.S01E01.720p.English.mkv",
            WRONG_LANGUAGE_REASON,
            now,
        );
        assert!(ag_state.is_release_blacklisted(
            "ep-456",
            "",
            "Mi Serie S01E01 720p English mkv"
        ));
    }

    #[test]
    fn test_prune_expired_blacklists_keeps_language_and_expires_stalls() {
        let mut ag_state = AutoGrabberState::default();
        let t0 = 1_700_000_000_000i64;

        ag_state.blacklist_hash(
            "ep-1",
            "hash_stall_old",
            "Naruto.01x005.Castellano.mkv",
            "Estancada (0% y 0 fuentes en eD2K durante >45 min)",
            t0,
        );
        ag_state.blacklist_hash(
            "ep-1",
            "hash_wrong_lang_old",
            "Naruto.01x005.Latino.mkv",
            WRONG_LANGUAGE_REASON,
            t0,
        );
        ag_state.blacklist_hash(
            "ep-2",
            "hash_stall_recent",
            "Naruto.01x006.Castellano.mkv",
            "Estancada (0 bytes sin cola activa en eD2K durante >4 h)",
            t0 + STALL_BLACKLIST_TTL_MS - 60_000,
        );

        let now = t0 + STALL_BLACKLIST_TTL_MS + 1_000;
        let pruned = ag_state.prune_expired_blacklists(now, STALL_BLACKLIST_TTL_MS);
        assert_eq!(pruned, 1);
        assert!(!ag_state.is_hash_blacklisted("ep-1", "hash_stall_old"));
        assert!(ag_state.is_hash_blacklisted("ep-1", "hash_wrong_lang_old"));
        assert!(ag_state.is_hash_blacklisted("ep-2", "hash_stall_recent"));
    }

    #[test]
    fn test_skips_sd_pack_when_hd_episodes_exist_in_pool() {
        let item = MediaItem {
            id: "sp-id".to_string(),
            media_type: "tv".to_string(),
            title: "South Park".to_string(),
            original_title: None,
            year: Some(1997),
            overview: None,
            poster_path: None,
            backdrop_path: None,
            status: "downloading".to_string(),
            quality_profile: "1080p-es".to_string(),
            monitored: true,
            root_folder: "/tmp".to_string(),
            tmdb_id: None,
            tvdb_id: None,
            imdb_id: None,
            file_path: None,
            download_id: None,
            original_release_name: None,
            release_date: None,
            digital_release_date: None,
            release_status: None,
            created_at: 0,
            updated_at: 0,
        };
        let sd_pack = ScoredRelease {
            title: "South Park 5 [By Macleod].avi".to_string(),
            protocol: "torrent".to_string(),
            link: "magnet:?xt=urn:btih:2e1e7671d0d22a1959c96a0f1dae2baa5e953837".to_string(),
            hash: "2e1e7671d0d22a1959c96a0f1dae2baa5e953837".to_string(),
            size_bytes: 2_100_000_000,
            sources: 2,
            indexer_name: "BT/apibay".to_string(),
            quality: "SD".to_string(),
            language: "Castellano".to_string(),
            score: 84,
            is_rejected: false,
            rejection_reason: None,
        };
        let ep1 = evaluate_release(
            "South.Park.S05E01.WEB-DL.1080p.Castellano.mkv",
            "ed2k",
            "ed2k://|file|ep1|1000000|aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa|/",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            500_000_000,
            5,
            "eD2k/Kad",
            false,
            "1080p-es",
            "es",
        );
        let ep2 = evaluate_release(
            "South.Park.S05E02.WEB-DL.1080p.Castellano.mkv",
            "ed2k",
            "ed2k://|file|ep2|1000000|bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb|/",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            500_000_000,
            5,
            "eD2k/Kad",
            false,
            "1080p-es",
            "es",
        );

        assert_eq!(
            series::should_skip_sd_pack_for_hd_pool("1080p-es", &sd_pack, &[], &item, &[], 5),
            0
        );
        assert!(
            series::should_skip_sd_pack_for_hd_pool(
                "1080p-es",
                &sd_pack,
                &[ep1.clone(), ep2.clone()],
                &item,
                &[],
                5
            ) >= 2
        );

        let mut hd_pack = sd_pack.clone();
        hd_pack.quality = "1080p".to_string();
        assert_eq!(
            series::should_skip_sd_pack_for_hd_pool("1080p-es", &hd_pack, &[ep1, ep2], &item, &[], 5),
            0
        );
    }

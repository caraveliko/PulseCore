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

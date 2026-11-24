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

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use super::*;

#[test]
fn test_batman_1989_rejects_dark_knight_and_sequels() {
    assert!(!matches_release(
        "Batman.The.Dark.Knight.(Spanish.English.Subs).UHD.4K.HEVC-AC3.by.Geot.mkv",
        "Batman",
        Some("Batman"),
        Some(1989),
        true,
        None,
    ));
    assert!(!matches_release(
        "Batman.The.Dark.Knight.(2008).(Spanish.English.Subs).1080p.mkv",
        "Batman",
        Some("Batman"),
        Some(1989),
        true,
        None,
    ));
    assert!(!matches_release(
        "Batman.Returns.(1992).(Spanish.English.Subs).BDrip.1080p.HDR.HEVC.10b-AC3.by.nara.mkv",
        "Batman",
        Some("Batman"),
        Some(1989),
        true,
        None,
    ));
    assert!(!matches_release(
        "The.Batman.(2022).1080p.Castellano.mkv",
        "Batman",
        Some("Batman"),
        Some(1989),
        true,
        None,
    ));

    assert!(matches_release(
        "Batman.(1989).(Spanish.English.Subs).BDrip.1080p.HEVC-AC3.mkv",
        "Batman",
        Some("Batman"),
        Some(1989),
        true,
        None,
    ));
    assert!(matches_release(
        "(Tim.Burton).(1989).Batman.(Spanish.English.Subs).BDrip.1080p.mkv",
        "Batman",
        Some("Batman"),
        Some(1989),
        true,
        None,
    ));
}

#[test]
fn test_sequel_and_bilingual_releases_match_correctly() {
    assert!(matches_release(
        "Batman.Returns.(1992).(Spanish.English.Subs).BDrip.1080p.HDR.HEVC.10b-AC3.by.nara.mkv",
        "Batman vuelve",
        Some("Batman Returns"),
        Some(1992),
        true,
        None,
    ));
    assert!(!matches_release(
        "Batman.(1989).(Spanish.English.Subs).BDrip.1080p.mkv",
        "Batman Returns",
        Some("Batman Returns"),
        Some(1992),
        true,
        None,
    ));

    assert!(matches_release(
        "(Polanski).(1968).La.semilla.del.diablo.(Rosemary's.Baby).(Spanish.English.Subs).BDrip.1080p.HEVC.mkv",
        "Rosemary's Baby",
        Some("Rosemary's Baby"),
        Some(1968),
        true,
        None,
    ));

    // Dune Part Two / Dune Parte Dos
    assert!(matches_release(
        "Dune.Parte.Dos.2024.2160p.WEB-DL.DDP5.1.Atmos.DV.HDR.H.265-KWK.mkv",
        "Dune: Part Two",
        Some("Dune: Part Two"),
        Some(2024),
        true,
        None,
    ));
    assert!(matches_release(
        "Dune.2.2024.1080p.Castellano.mkv",
        "Dune: Part Two",
        Some("Dune: Part Two"),
        Some(2024),
        true,
        None,
    ));
    assert!(!matches_release(
        "Dune.3.2026.1080p.Castellano.mkv",
        "Dune: Part Two",
        Some("Dune: Part Two"),
        Some(2024),
        true,
        None,
    ));
}

#[test]
fn test_spanish_and_double_episode_tv_matching_and_scorer() {
    assert!(matches_release(
        "VisionQuest.Temporada.1.Cap.102.Castellano.1080p.by.Bryan_122.mkv",
        "VisionQuest",
        None,
        Some(2026),
        false,
        Some((1, 2)),
    ));
    assert!(!matches_release(
        "VisionQuest.1x10.Castellano.1080p.mkv",
        "VisionQuest",
        None,
        Some(2026),
        false,
        Some((1, 1)),
    ));
    assert!(matches_release(
        "VisionQuest.S01E01-E02.Dual.1080p.HEVC.mkv",
        "VisionQuest",
        None,
        Some(2026),
        false,
        Some((1, 2)),
    ));

    // Multiepisodio S01E01-E04
    let span = extract_tv_episode_numbers("VisionQuest.S01E01-E04.1080p.Dual.mkv", 1);
    assert_eq!(span, vec![1, 2, 3, 4]);

    // Paquete de temporada completa
    assert!(matches_season_pack_release(
        "VisionQuest.Temporada.1.Completa.1080p.Castellano.HEVC",
        "VisionQuest",
        None,
        Some(2026),
        1,
    ));
    assert!(matches_season_pack_release(
        "La.casa.del.dragon.(House.of.the.Dragon).S02.1080p.Dual.HEVC",
        "La casa del dragón",
        Some("House of the Dragon"),
        Some(2022),
        2,
    ));
    assert!(!matches_season_pack_release(
        "VisionQuest.S01E02.1080p.Castellano.mkv",
        "VisionQuest",
        None,
        Some(2026),
        1,
    ));

    let rejected_eng = evaluate_release(
        "Batman.1989.1080p.BluRay.x264.English.mkv",
        "ed2k",
        "ed2k://|file|test|3000000000|aaaa|/",
        "aaaa",
        3_000_000_000,
        15,
        "eD2k/Kad",
        true,
        "1080p-es",
        "es",
    );
    assert!(rejected_eng.is_rejected);

    let accepted_dual = evaluate_release(
        "Batman.(1989).(Spanish.English.Subs).BDrip.1080p.HEVC.AC3.by.nara.mkv",
        "ed2k",
        "ed2k://|file|test|3500000000|bbbb|/",
        "bbbb",
        3_500_000_000,
        20,
        "eD2k/Kad",
        true,
        "1080p-es",
        "es",
    );
    assert!(!accepted_dual.is_rejected);
    assert_eq!(accepted_dual.language, "Dual");
    assert!(accepted_dual.score >= 200);
}

#[test]
fn test_english_unknown_and_foreign_releases_strictly_rejected_and_bilingual_matched() {
    // Las versiones de la escena en inglés estadounidense sin etiquetar deben rechazarse cuando preferred_language es español
    let us_scene = evaluate_release(
        "Station.Eleven.2021.S01E07.Goodbye.My.Damaged.Home.1080p.WEBRip.EAC3.HEVC-t3nzin.mkv",
        "ed2k",
        "ed2k://|file|test|1730600138|cccc|/",
        "cccc",
        1_730_600_138,
        5,
        "eD2k/Kad",
        false,
        "1080p-es",
        "es",
    );
    assert!(us_scene.is_rejected);
    assert_eq!(us_scene.language, "Unknown");

    // Las versiones francesas VOSTFR e italianas ITA ENG deben rechazarse como Foreign
    let fr_rel = evaluate_release(
        "Station.Eleven.S01E02.FASTSUB.VOSTFR.WEBRip.x264-WEEDS.mp4",
        "ed2k",
        "ed2k://|file|test|340628426|dddd|/",
        "dddd",
        340_628_426,
        5,
        "eD2k/Kad",
        false,
        "1080p-es",
        "es",
    );
    assert!(fr_rel.is_rejected);
    assert_eq!(fr_rel.language, "Foreign");

    let ita_rel = evaluate_release(
        "Station Eleven S01E01 Wheel of Fire - h264 1080p ITA ENG - e55e.mkv",
        "ed2k",
        "ed2k://|file|test|900000000|eeee|/",
        "eeee",
        900_000_000,
        5,
        "eD2k/Kad",
        false,
        "1080p-es",
        "es",
    );
    assert!(ita_rel.is_rejected);
    assert_eq!(ita_rel.language, "Foreign");

    // La versión en español con (Spa-Eng-Spanishsub) debe aceptarse como Dual
    let spa_dual = evaluate_release(
        "Estacion.once.1x01.Rueda.de.fuego.(Spanish.English.Subs).HMAX.WEB-DL.1080p.x264-AC3.(pxl-sharerip).mkv",
        "ed2k",
        "ed2k://|file|test|1200000000|ffff|/",
        "ffff",
        1_200_000_000,
        5,
        "eD2k/Kad",
        false,
        "1080p-es",
        "es",
    );
    assert!(!spa_dual.is_rejected);
    assert_eq!(spa_dual.language, "Dual");

    let spa_hyphen = evaluate_release(
        "Pantheon.1x02.Ciclos.(Spa-Eng-Spanishsub).HDrip.1080.HEVC.10b-AC3.by.nara.mkv",
        "ed2k",
        "ed2k://|file|test|759010056|1111|/",
        "1111",
        759_010_056,
        5,
        "eD2k/Kad",
        false,
        "1080p-es",
        "es",
    );
    assert!(!spa_hyphen.is_rejected);
    assert_eq!(spa_hyphen.language, "Dual");

    // Coincidencia de título bilingüe: "Station Eleven" <-> "Estacion once"
    assert!(matches_release(
        "Estacion.once.1x01.Rueda.de.fuego.(Spanish.English.Subs).HMAX.WEB-DL.1080p.x264-AC3.(pxl-sharerip).mkv",
        "Station Eleven",
        None,
        Some(2021),
        false,
        Some((1, 1)),
    ));
    assert!(matches_release(
        "Estacion once - Temporada 1 [HDTV][Cap.106][Castellano].avi",
        "Station Eleven",
        None,
        Some(2021),
        false,
        Some((1, 6)),
    ));

    // La versión (Eng-Sub) de un grupo de confianza debe clasificarse como VOSE, no Castellano,
    // y debe puntuar más bajo que una versión Dual real
    let geot_vose = evaluate_release(
        "Pantheon.(2023).2x01.The.Gods.Have.Not.Died.In.Vain.(Eng-Sub).HD.1080p.CHDR.HEVC.10b-AAC.by.Geot.(siemprehevc).mkv",
        "ed2k",
        "ed2k://|file|test|316913519|2222|/",
        "2222",
        316_913_519,
        5,
        "eD2k/Kad",
        false,
        "1080p-es",
        "es",
    );
    assert_eq!(geot_vose.language, "VOSE");

    let bryan_dual = evaluate_release(
        "Pantheon.S02E01.Los.dioses.no.han.muerto.en.vano.(Spanish.English.Subs).WEB-DL.1080p.x264-EAC3.by.Bryan_122.mkv",
        "ed2k",
        "ed2k://|file|test|1168908941|3333|/",
        "3333",
        1_168_908_941,
        5,
        "eD2k/Kad",
        false,
        "1080p-es",
        "es",
    );
    assert_eq!(bryan_dual.language, "Dual");
    assert!(bryan_dual.score > geot_vose.score);

    // _Eng.Subt.Esp debe ser VOSE, no Dual
    let subt_esp = evaluate_release(
        "Estación Once (Station Eleven) S01E04 [Rosencrantz and Guildenstern Aren't Dead]_Eng.Subt.Esp.mkv",
        "ed2k",
        "ed2k://|file|test|1114576439|4444|/",
        "4444",
        1_114_576_439,
        5,
        "eD2k/Kad",
        false,
        "1080p-es",
        "es",
    );
    assert_eq!(subt_esp.language, "VOSE");

    // Monsieur Ibrahim / El señor Ibrahim y las flores del Corán:
    // 1. La versión catalana/francesa `.fr.fr.vo.vos_es` debe ser rechazada como `Foreign`
    let cat_fr_rel = evaluate_release(
        "El.senyor.Ibrahim.i.las.flors.de.l'Alcora.(Monsieur.Ibrahim.et.les.fleurs.du.Coran).Francois.Dupeyron.2003.fr.fr.vo.vos_es.vd_es.mkv",
        "ed2k",
        "ed2k://|file|test|1500000000|5555|/",
        "5555",
        1_500_000_000,
        2,
        "eD2k/Kad",
        true,
        "1080p-es",
        "es",
    );
    assert!(cat_fr_rel.is_rejected);
    assert_eq!(cat_fr_rel.language, "Foreign");

    // 2. La versión Dual 1080p `(spa.fr).1080p.Blurayx264.mkv` debe aceptarse en Dual y coincidir con el título
    let ibrahim_dual = evaluate_release(
        "El señor Ibrahim y las flores del Coran (2003) François Dupeyron (spa.fr).1080p.Blurayx264.mkv",
        "ed2k",
        "ed2k://|file|test|1804599296|6666|/",
        "6666",
        1_804_599_296,
        5,
        "eD2k/Kad",
        true,
        "1080p-es",
        "es",
    );
    assert!(!ibrahim_dual.is_rejected);
    assert_eq!(ibrahim_dual.language, "Dual");
    assert_eq!(ibrahim_dual.quality, "1080p");

    assert!(matches_release(
        "El señor Ibrahim y las flores del Coran (2003) François Dupeyron (spa.fr).1080p.Blurayx264.mkv",
        "Monsieur Ibrahim",
        Some("Monsieur Ibrahim et les Fleurs du Coran"),
        Some(2003),
        true,
        None,
    ));
    assert!(matches_release(
        "El seÃ±or Ibrahim y las flores del Coran (2003) FranÃ§ois Dupeyron (spa.fr).1080p.Blurayx264.mkv",
        "El señor Ibrahim y las flores del Corán",
        Some("Monsieur Ibrahim et les Fleurs du Coran"),
        Some(2003),
        true,
        None,
    ));

    // A Touch of Spice / Un toque de canela (DVD-Rip en castellano)
    let canela_rel = evaluate_release(
        "Un toque de canela.(2003 Tassos Boulmetis). (Spanish) Dvd-Rip Xvid-Ac3.avi",
        "ed2k",
        "ed2k://|file|Un toque de canela.(2003 Tassos Boulmetis). (Spanish) Dvd-Rip Xvid-Ac3.avi|1573390336|a519e07bf0deeb9afc172239d619374d|/",
        "a519e07bf0deeb9afc172239d619374d",
        1_573_390_336,
        7,
        "eD2k/Kad",
        true,
        "1080p-es",
        "es",
    );
    assert!(!canela_rel.is_rejected);
    assert_eq!(canela_rel.language, "Castellano");
    assert_eq!(canela_rel.quality, "SD");
    assert!(matches_release(
        "Un toque de canela.(2003 Tassos Boulmetis). (Spanish) Dvd-Rip Xvid-Ac3.avi",
        "A Touch of Spice",
        Some("Πολίτικη Κουζίνα"),
        Some(2003),
        true,
        None,
    ));
}

#[test]
fn test_ali_g_who_iz_i_matches_spanish_slang_release() {
    let cand = "Ali G Kien es yo (2026) [(Who Iz I) m1080p h264 AC3 5.1 Spa Eng +EAC3 Spa +Sub yamil].mkv";
    assert!(matches_release(
        cand,
        "Ali G: Who Iz I?",
        Some("Ali G: Who Iz I?"),
        Some(2026),
        true,
        None
    ));
    let scored = evaluate_release(
        cand,
        "ed2k",
        "ed2k://|file|sample|4250000000|0123456789abcdef0123456789abcdef|/",
        "0123456789abcdef0123456789abcdef",
        4_250_000_000,
        10,
        "eD2k/Kad",
        true,
        "1080p-es",
        "es",
    );
    assert!(!scored.is_rejected);
    assert_eq!(scored.language, "Dual");
    assert_eq!(scored.quality, "1080p");
    assert!(scored.score > 100);
}

#[test]
fn test_viaje_a_la_luna_1902_silent_matches_and_not_rejected() {
    let cand1 = "Viaje.a.la.Luna(Le.voyage.dans.la.lune(1902).(Silent.Spanishsub).HDrip.DivX-AC3.by.abantos.(seriestv.in).avi";
    assert!(matches_release(
        cand1,
        "El viaje a la Luna",
        Some("Le Voyage dans la Lune"),
        Some(1902),
        true,
        None
    ));
    let scored1 = evaluate_release(
        cand1,
        "ed2k",
        "ed2k://|file|sample|680500000|0123456789abcdef0123456789abcdef|/",
        "0123456789abcdef0123456789abcdef",
        680_500_000, // 649 MB
        10,
        "eD2k/Kad",
        true,
        "1080p-es",
        "es",
    );
    assert!(!scored1.is_rejected);
    assert_eq!(scored1.language, "VOSE");
    assert!(scored1.score > 50);

    let cand2 = "Cine - Georges Méliès - 1902 - Viaje a la Luna (1902) - (Le.voyage.dans.la.lune - MUDA-Spanishsub) - COLOR - Restaurada.mkv";
    assert!(matches_release(
        cand2,
        "El viaje a la Luna",
        Some("Le Voyage dans la Lune"),
        Some(1902),
        true,
        None
    ));
    let scored2 = evaluate_release(
        cand2,
        "ed2k",
        "ed2k://|file|sample|262144000|0123456789abcdef0123456789abcdef|/",
        "0123456789abcdef0123456789abcdef",
        262_144_000, // 250 MB
        8,
        "eD2k/Kad",
        true,
        "1080p-es",
        "es",
    );
    assert!(!scored2.is_rejected);
    assert_eq!(scored2.language, "VOSE");
    assert!(scored2.score > 50);
}

#[test]
fn test_roan_inish_matches_spanish_distribution_title() {
    let cand = "El.secreto.de.la.isla.de.las.focas.(John.Sayles,1994).(Spanish.English.Subs).WEBRip.1080p.x265-AC3.AAC.mkv";
    assert!(matches_release(
        cand,
        "The Secret of Roan Inish",
        Some("The Secret of Roan Inish"),
        Some(1994),
        true,
        None
    ));
    assert!(matches_release_with_alts(
        cand,
        "The Secret of Roan Inish",
        Some("The Secret of Roan Inish"),
        &["El secreto de la isla de las focas"],
        Some(1994),
        true,
        None
    ));
    let scored = evaluate_release(
        cand,
        "ed2k",
        "ed2k://|file|sample|1948000000|0123456789abcdef0123456789abcdef|/",
        "0123456789abcdef0123456789abcdef",
        1_948_000_000,
        12,
        "eD2k/Kad",
        true,
        "1080p-es",
        "es",
    );
    assert!(!scored.is_rejected);
    assert_eq!(scored.language, "Dual");
    assert_eq!(scored.quality, "1080p");
    assert!(scored.score > 100);
}

#[test]
fn test_spanish_title_variants_for_roan_and_luna() {
    let roan_variants =
        spanish_title_search_variants("The Secret of Roan Inish", Some("The Secret of Roan Inish"));
    assert!(roan_variants.iter().any(|v| v.contains("focas")));

    let luna_variants =
        spanish_title_search_variants("A Trip to the Moon", Some("Le Voyage dans la Lune"));
    assert!(luna_variants.iter().any(|v| v.to_lowercase().contains("viaje")));
}

#[test]
fn test_series_with_disambiguation_year_suffix_and_the_body_2026() {
    assert_eq!(strip_trailing_year_suffix("The Body (2026)"), "The Body");
    assert_eq!(strip_trailing_year_suffix("1923"), "1923");
    assert_eq!(strip_trailing_year_suffix("The Office (US)"), "The Office (US)");

    let body_variants = spanish_title_search_variants("The Body (2026)", None);
    assert!(
        body_variants.iter().any(|v| v.eq_ignore_ascii_case("El cuerpo")),
        "Se esperaba la variante 'El cuerpo', obtenido: {body_variants:?}"
    );

    // Lanzamientos reales de 'The Body (2026)' en eD2K/Kad
    assert!(matches_release(
        "El.cuerpo.1x01.Dios.guarrilla.(Spanish.English.Subs).WEBRip.1080p.x265-EAC3.EAC3.Atmos.by.Legan.mkv",
        "The Body (2026)",
        None,
        Some(2026),
        false,
        Some((1, 1)),
    ));
    assert!(matches_release(
        "The Body 2026 1x04 1080p - I Saw Grace Franklin with the Devil (Multisub) - Spanish subs integrados by JuAnItO.mkv",
        "The Body (2026)",
        None,
        Some(2026),
        false,
        Some((1, 4)),
    ));

    let exploradores_rel = "El cuerpo 1x06 (2026) Web Dl 1080x264 E-AC3 5.1 hipolismata ivi25 para exploradoresp2p.mkv";
    assert!(matches_release(
        exploradores_rel,
        "The Body (2026)",
        None,
        Some(2026),
        false,
        Some((1, 6)),
    ));
    let scored_expl = evaluate_release(
        exploradores_rel,
        "ed2k",
        "ed2k://|file|sample|1246000000|0123456789abcdef0123456789abcdef|/",
        "0123456789abcdef0123456789abcdef",
        1_246_000_000,
        5,
        "eD2k/Kad",
        false,
        "1080p-es",
        "es",
    );
    assert!(!scored_expl.is_rejected);
    assert_eq!(scored_expl.language, "Castellano");
    assert_eq!(scored_expl.quality, "1080p");

    // Falsos positivos que deben seguir rechazándose
    assert!(!matches_release(
        "3 Body Problem - S01E01 - Countdown - HDTV-2160p - Spanish 1x01.mkv",
        "The Body (2026)",
        None,
        Some(2026),
        false,
        Some((1, 1)),
    ));
    assert!(!matches_release(
        "El.comisario.10x03.El.diablo.en.el.cuerpo.(Spanish.Subs).WEBRip.1080p.mkv",
        "The Body (2026)",
        None,
        Some(2026),
        false,
        Some((10, 3)),
    ));
    assert!(!matches_release(
        "Cuerpo.de.Élite.1x09.El.virus.(Spanish.Subs).WEBRip.1080p.mkv",
        "The Body (2026)",
        None,
        Some(2026),
        false,
        Some((1, 9)),
    ));
    assert!(!matches_release(
        "The.Body.2004.S01E01.1080p.Castellano.mkv",
        "The Body (2026)",
        None,
        Some(2026),
        false,
        Some((1, 1)),
    ));
}

#[test]
fn test_strict_castilian_confidence_and_filter() {
    let make_rel = |title: &str, indexer: &str| {
        evaluate_release(
            title,
            "torrent",
            &format!("magnet:?xt=urn:btih:{title}"),
            "0123456789abcdef0123456789abcdef",
            2_500_000_000,
            12,
            indexer,
            true,
            "1080p-es",
            "es",
        )
    };

    let rel_verified = make_rel("Dune.Parte.Dos.2024.1080p.Castellano.x265.mkv", "Prowlarr");
    let rel_group = make_rel("Dune.Parte.Dos.2024.1080p.Spanish.by.Bryan_122.mkv", "Prowlarr");
    let rel_indexer = make_rel("Dune.Parte.Dos.2024.1080p.Spanish.mkv", "DonTorrent");
    let rel_dual = make_rel("Dune.Parte.Dos.2024.1080p.Dual.Spanish.English.mkv", "Prowlarr");
    let rel_possible = make_rel("Dune.Parte.Dos.2024.1080p.Spanish.mkv", "1337x");
    let rel_vose = make_rel("Dune.Parte.Dos.2024.1080p.VOSE.mkv", "1337x");
    let rel_latino = make_rel("Dune.Parte.Dos.2024.1080p.Latino.mkv", "1337x");
    let rel_latam_spanish = make_rel("Dune.Parte.Dos.2024.1080p.Spanish.Latam.mkv", "1337x");

    assert_eq!(castilian_confidence_level(&rel_verified), "verified");
    assert_eq!(castilian_confidence_level(&rel_group), "verified");
    assert_eq!(castilian_confidence_level(&rel_indexer), "verified");
    assert_eq!(castilian_confidence_level(&rel_dual), "dual");
    assert_eq!(castilian_confidence_level(&rel_possible), "possible");
    assert_eq!(castilian_confidence_level(&rel_vose), "none");
    assert_eq!(castilian_confidence_level(&rel_latino), "none");
    assert_eq!(castilian_confidence_level(&rel_latam_spanish), "none");

    assert!(is_strict_castilian_release(&rel_verified));
    assert!(is_strict_castilian_release(&rel_possible));
    assert!(!is_strict_castilian_release(&rel_vose));
    assert!(!is_strict_castilian_release(&rel_latino));

    let mut batch = vec![rel_possible.clone(), rel_verified.clone(), rel_vose, rel_latino];
    apply_strict_castilian_filter(&mut batch);

    assert!(!batch[0].is_rejected);
    assert!(!batch[1].is_rejected);
    assert!(batch[1].score > batch[0].score);
    assert!(batch[2].is_rejected);
    assert!(batch[3].is_rejected);
}

#[test]
fn test_sequel_three_never_matches_first_installment() {
    assert!(!matches_release_with_alts(
        "Dune.2021.1080p.Castellano.BluRay.x264.mkv",
        "Dune: Part Three",
        Some("Dune: Part Three"),
        &["Dune", "Dune 3"],
        None,
        true,
        None,
    ));
    assert!(!matches_release_with_alts(
        "Dune.1.1080p.Castellano.mkv",
        "Dune 3",
        None,
        &["Dune"],
        None,
        true,
        None,
    ));
    assert!(matches_release_with_alts(
        "Dune.Parte.Tres.2026.1080p.Castellano.mkv",
        "Dune: Part Three",
        Some("Dune: Part Three"),
        &["Dune 3"],
        Some(2026),
        true,
        None,
    ));
}

#[test]
fn test_anime_absolute_episode_numbering() {
    // Formato NarutoSpain eD2K: "Naruto 1x36 ..." corresponde a S02E01 (abs 36) cuando es anime
    assert!(matches_release_with_options(
        "Naruto 1x36 Clones contra clones (DVD+SAT by Redencion)(Spanish-Japones)(NarutoSpain.com).avi",
        "Naruto",
        None,
        &[],
        Some(2002),
        false,
        Some((2, 1)),
        Some(36),
    ));
    // Formato anime con número absoluto de 3 dígitos
    assert!(matches_release_with_options(
        "Naruto - 036 [1080p][Castellano].mkv",
        "Naruto",
        None,
        &[],
        Some(2002),
        false,
        Some((2, 1)),
        Some(36),
    ));
    // Formato anime con número absoluto de 3 dígitos y título de episodio tras guion
    assert!(matches_release_with_options(
        "Naruto - 036 - Clones contra clones [1080p][Castellano].mkv",
        "Naruto",
        None,
        &[],
        Some(2002),
        false,
        Some((2, 1)),
        Some(36),
    ));
    // Formato tusSeries con temporada alternativa y número absoluto entre paréntesis
    assert!(matches_release_with_options(
        "Naruto.7x20.(176).DVBRip.Spanish.tusSeries.com.avi",
        "Naruto",
        None,
        &[],
        Some(2002),
        false,
        Some((4, 20)),
        Some(176),
    ));
    // Spin-off (Shippuden) debe seguir rechazándose aunque coincida el número absoluto
    assert!(!matches_release_with_options(
        "Naruto Shippuden - 036 - El regreso [1080p][Castellano].mkv",
        "Naruto",
        None,
        &[],
        Some(2002),
        false,
        Some((2, 1)),
        Some(36),
    ));
    // Si NO es anime (absolute_episode = None), "1x36" NO debe coincidir con S02E01
    assert!(!matches_release_with_options(
        "Naruto 1x36 Clones contra clones.avi",
        "Naruto",
        None,
        &[],
        Some(2002),
        false,
        Some((2, 1)),
        None,
    ));
}

#[test]
fn test_ed2k_healthy_sources_outscore_single_source_ghost() {
    let ghost_1080p = evaluate_release(
        "Naruto.01x005.1080p.WEB-DL.Castellano.x265.mkv",
        "ed2k",
        "ed2k://|file|a|400000000|0123456789abcdef0123456789abcdef|/",
        "0123456789abcdef0123456789abcdef",
        400_000_000,
        1,
        "eD2k/Kad",
        false,
        "1080p-es",
        "es",
    );
    let healthy_720p = evaluate_release(
        "Naruto.01x005.720p.Castellano.mkv",
        "ed2k",
        "ed2k://|file|b|400000000|abcdef0123456789abcdef0123456789|/",
        "abcdef0123456789abcdef0123456789",
        400_000_000,
        8,
        "eD2k/Server+Kad",
        false,
        "1080p-es",
        "es",
    );
    let healthy_1080p = evaluate_release(
        "Naruto.01x005.1080p.WEB-DL.Castellano.x265.mkv",
        "ed2k",
        "ed2k://|file|c|400000000|fedcba9876543210fedcba9876543210|/",
        "fedcba9876543210fedcba9876543210",
        400_000_000,
        6,
        "eD2k/Server+Kad",
        false,
        "1080p-es",
        "es",
    );
    assert!(!ghost_1080p.is_rejected);
    assert!(
        healthy_720p.score > ghost_1080p.score,
        "720p con 8 fuentes ({}) debería superar a 1080p fantasma de 1 fuente ({})",
        healthy_720p.score,
        ghost_1080p.score
    );
    assert!(healthy_1080p.score > healthy_720p.score);
}

#[test]
fn test_spaced_episode_synthetic_emule_qt_and_dual_lat_rejected() {
    let title = "El señor de los anillos Los anillos de poder (2022) S01 E05 1080p.mp4";
    assert_eq!(extract_tv_episode_numbers(title, 1), vec![5]);
    assert!(!matches_season_pack_release(
        title,
        "The Lord of the Rings: The Rings of Power",
        Some("The Lord of the Rings: The Rings of Power"),
        Some(2022),
        1,
    ));

    assert!(is_single_video_file_ed2k(title, "ed2k"));
    assert!(is_single_video_file_ed2k("serie.s01.1080p.mkv", "ed2k"));
    assert!(!is_single_video_file_ed2k("serie.s01.1080p.mkv", "torrent"));
    assert!(!is_single_video_file_ed2k("serie.s01.1080p.rar", "ed2k"));

    let syn_torrent = evaluate_release(
        "[torrent emule-qt.org] El señor de los anillos Los anillos de poder (2022) S01 E05 1080p.dual-lat.cinecalidad.run.mp4",
        "ed2k",
        "ed2k://|file|a|1945517076|ed2b0112000099b507b934ed56e09e8d|/",
        "ed2b0112000099b507b934ed56e09e8d",
        1_945_517_076,
        10,
        "eD2k/Server",
        false,
        "1080p-es",
        "es",
    );
    assert!(syn_torrent.is_rejected);

    let syn_kad = evaluate_release(
        "[kad emule-qt.org] naruto.(2002).s02e26.1080p.esp.mkv",
        "ed2k",
        "ed2k://|file|b|1248840079|6155eb4b6f79f57661cf644533ba7eea|/",
        "6155eb4b6f79f57661cf644533ba7eea",
        1_248_840_079,
        5,
        "eD2k/Kad",
        false,
        "1080p-es",
        "es",
    );
    assert!(syn_kad.is_rejected);

    let syn_hash = evaluate_release(
        "El señor de los anillos S01E05 1080p Castellano.mkv",
        "ed2k",
        "ed2k://|file|c|1200000000|ed2b0112000099b507b934ed56e09e8d|/",
        "ed2b0112000099b507b934ed56e09e8d",
        1_200_000_000,
        5,
        "eD2k/Server",
        false,
        "1080p-es",
        "es",
    );
    assert!(syn_hash.is_rejected);

    let dual_lat = evaluate_release(
        "El señor de los anillos Los anillos de poder (2022) S01 E05 1080p.dual-lat.cinecalidad.run.mp4",
        "ed2k",
        "ed2k://|file|d|1945517076|1234567890abcdef1234567890abcdef|/",
        "1234567890abcdef1234567890abcdef",
        1_945_517_076,
        10,
        "eD2k/Server",
        false,
        "1080p-es",
        "es",
    );
    assert!(dual_lat.is_rejected);
    assert_eq!(dual_lat.language, "Latino");
}

#[test]
fn rejects_s10e11_as_s01e11_and_s20e05_as_s02e05() {
    use super::matching::{extract_tv_episode_numbers, matches_release_with_options};

    assert!(extract_tv_episode_numbers("South.Park.S10E11.El.infierno.en.la.tierra.2006.mkv", 1).is_empty());
    assert!(!matches_release_with_options(
        "South.Park.S10E11.El.infierno.en.la.tierra.2006.mkv",
        "South Park",
        None,
        &[],
        Some(1997),
        false,
        Some((1, 11)),
        None,
    ));
    assert!(matches_release_with_options(
        "South.Park.S10E11.El.infierno.en.la.tierra.2006.mkv",
        "South Park",
        None,
        &[],
        Some(1997),
        false,
        Some((10, 11)),
        None,
    ));
    assert!(extract_tv_episode_numbers("Serie.S20E05.1080p.mkv", 2).is_empty());
}

#[test]
fn accepts_spanish_t1_complete_season_pack_for_south_park() {
    use super::matching::matches_season_pack_release;

    assert!(matches_season_pack_release(
        "South Park T1 completa castellano",
        "South Park",
        None,
        Some(1997),
        1,
    ));

    let res = evaluate_release(
        "South Park T1 completa castellano",
        "torrent",
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
        "0123456789abcdef0123456789abcdef01234567",
        2_500_000_000,
        2,
        "BT/apibay",
        false,
        "1080p-es",
        "es",
    );
    assert!(!res.is_rejected, "pack rechazado: {:?}", res.rejection_reason);
    assert!(res.score >= 25, "puntuación insuficiente: {}", res.score);
}

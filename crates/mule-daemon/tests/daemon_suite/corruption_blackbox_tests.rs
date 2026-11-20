// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Pruebas de integración de `CorruptionBlackBox` (eMule `CCorruptionBlackBox`),
//! de la guarda «parte ya completa» de `WriteToBuffer` y del control de ICH —
//! todas sin conexión.

use md4::{Digest, Md4};
use mule_core::AllocationPolicy;
use mule_daemon::services::downloads::DownloadService;
use mule_test_support::peer_fixtures::peer_source;
use tempfile::tempdir;

fn md4_of(data: &[u8]) -> [u8; 16] {
    Md4::digest(data).into()
}

/// `WriteToBuffer` de eMule: cualquier dato que toque una parte ya completa
/// («already hashed chunk») se ignora — una reinyección en una parte
/// cubierta no reescribe ni amplía los rangos recibidos.
#[test]
fn inject_into_completed_part_is_ignored() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    // Dos partes: la inyección de la parte 0 no completa el archivo.
    let link = format!(
        "ed2k://|file|two.mkv|{}|{}|/",
        9_728_000 + 10,
        hex::encode([0xAAu8; 16])
    );
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();

    svc.inject_range(&snap.id, 0, &vec![0x11u8; 9_728_000], Some("9.9.9.9:49662"))
        .unwrap();
    assert_eq!(svc.get_covered_len(&snap.id), Some(9_728_000));

    // Reinyección (datos diferentes) en la parte cubierta → ignorada.
    svc.inject_range(&snap.id, 100, &vec![0xEEu8; 500], Some("8.8.8.8:49662"))
        .unwrap();
    assert_eq!(
        svc.get_covered_len(&snap.id),
        Some(9_728_000),
        "write into completed part must be dropped"
    );

    // La parte 1 sigue siendo inyectable normalmente.
    svc.inject_range(&snap.id, 9_728_000, &[0x22u8; 10], Some("8.8.8.8:49662"))
        .unwrap();
    assert_eq!(svc.get_covered_len(&snap.id), Some(9_728_000 + 10));
}

/// `CorruptionBlackBox::EvaluateData`: el emisor de una parte totalmente
/// dañada supera `CBB_BANTHRESHOLD` — es condenado, desconectado y sus
/// fuentes son ignoradas durante la ingesta (`IsBannedClient`).
#[test]
fn corrupt_sender_is_banned_and_filtered() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let content = vec![0x42u8; 100];
    let file_hash = md4_of(&content);
    let link = format!("ed2k://|file|small.mkv|100|{}|/", hex::encode(file_hash));
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();

    // Conjunto de hashes (hashset) deliberadamente erróneo → la parte falla la verificación MD4.
    svc.store_part_hashes_by_hash(&hex::encode(file_hash), &[[0xEE; 16]])
        .unwrap();

    svc.inject_range(&snap.id, 0, &content, Some("1.2.3.4:49662"))
        .unwrap();

    // El emisor único es condenado (>32 % dañado) y expulsado de la sesión.
    let bans = svc.drain_corruption_bans(&snap.id);
    assert_eq!(bans, vec!["1.2.3.4:49662".to_string()]);
    assert!(svc.is_source_banned("1.2.3.4:49662"));
    assert!(svc.drain_corruption_bans(&snap.id).is_empty());

    // `AddSources` de eMule: las fuentes de un cliente expulsado se ignoran.
    let added = svc
        .upsert_peer_sources(
            &hex::encode(file_hash),
            &[
                peer_source(&hex::encode(file_hash), "1.2.3.4:49662"),
                peer_source(&hex::encode(file_hash), "5.6.7.8:49662"),
            ],
        )
        .unwrap();
    assert_eq!(added, 1, "banned sender must be filtered at ingestion");
}

/// Un par que ha entregado datos verificados además de una parte dañada
/// puede mantenerse por debajo del umbral: no es condenado.
#[test]
fn verified_history_dilutes_corruption_blame() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    // Archivo de 4 partes: 3 partes correctas y luego una parte dañada por el
    // mismo emisor → ratio ~25 % < 32 % → sin expulsión.
    let part = 9_728_000usize;
    let mut whole = vec![0x42u8; part * 4];
    let mkv_hdr = mule_network_policy::content_policy::MINIMAL_VALID_MKV_HEADER;
    whole[..mkv_hdr.len()].copy_from_slice(mkv_hdr);
    let p_hashes: Vec<[u8; 16]> = (0..4)
        .map(|i| md4_of(&whole[i * part..(i + 1) * part]))
        .collect();
    // Corrompe la parte 3.
    whole[3 * part + 100] ^= 0xFF;
    let file_hash = [0xBBu8; 16];
    let link = format!(
        "ed2k://|file|four.mkv|{}|{}|/",
        part * 4,
        hex::encode(file_hash)
    );
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();
    svc.store_part_hashes_by_hash(&hex::encode(file_hash), &p_hashes)
        .unwrap();

    for i in 0..4usize {
        svc.inject_range(
            &snap.id,
            (i * part) as u64,
            &whole[i * part..(i + 1) * part],
            Some("1.2.3.4:49662"),
        )
        .unwrap();
    }

    let bans = svc.drain_corruption_bans(&snap.id);
    assert!(
        bans.is_empty(),
        "sender with 75%% verified history must stay under the 32%% ban threshold, got {bans:?}"
    );
}

/// `ich_enabled = 0` (preferencia ICH desactivada): la parte dañada se
/// descarta por completo en lugar de esperar una recuperación AICH por bloques.
#[test]
fn ich_disabled_evicts_whole_part_without_aich() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();
    svc.db_mut().set_setting("ich_enabled", "0").unwrap();

    // Raíz AICH presente en el enlace — sin la preferencia desactivada la parte
    // iría a la cola AICH; aquí debe descartarse directamente.
    let aich_root = [0x77u8; 20];
    let link = format!(
        "ed2k://|file|aich.mkv|100|{}|h={}|/",
        hex::encode([0xAAu8; 16]),
        data_encoding::BASE32_NOPAD.encode(&aich_root)
    );
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();
    svc.store_part_hashes_by_hash(&hex::encode([0xAAu8; 16]), &[[0xEE; 16]])
        .unwrap();

    svc.inject_range(&snap.id, 0, &[0x42u8; 100], Some("1.2.3.4:49662"))
        .unwrap();

    // Parte descartada por completo → sin ninguna cobertura, sin cola AICH.
    assert_eq!(svc.get_covered_len(&snap.id), Some(0));
    assert!(svc.next_aich_recovery(&snap.id).is_none());
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_core::model::{PeerSource, PeerSourceOrigin, PeerSourceStatus};
use mule_core::persistence::Database;

#[test]
fn test_peer_sources_crud_and_deduplication() {
    let mut db = Database::open_in_memory().expect("open DB in memory");
    let hash = "0123456789abcdef0123456789abcdef";

    // 1. Inicialmente vacío
    let list = db.list_peer_sources(hash).expect("list peer sources");
    assert_eq!(list.len(), 0);

    // 2. Insertar o actualizar dos fuentes de pares
    let s1 = PeerSource {
        file_hash: hash.to_string(),
        address: "192.168.1.50:39735".to_string(),
        ip: 0xC0A80132,
        port: 39735,
        origin: PeerSourceOrigin::Server,
        discovered_at_unix_ms: 1000,
        last_seen_unix_ms: 1000,
        status: PeerSourceStatus::Candidate,
        fail_count: 0,
        last_error: None,
        cooldown_until_unix_ms: None,
        user_hash: Some("0123456789abcdef0123456789abcdef".to_string()),
        crypt_options: 0x07, // admite, solicita, requiere
        buddy_id: None,
        software: None,
        availability: None,
        server_ip: 0,
        server_port: 0,
        udp_port: 49673,
    };

    let s2 = PeerSource {
        file_hash: hash.to_string(),
        address: "10.0.0.88:39735".to_string(),
        ip: 0x0A000058,
        port: 39735,
        origin: PeerSourceOrigin::Search,
        discovered_at_unix_ms: 1000,
        last_seen_unix_ms: 1000,
        status: PeerSourceStatus::Filtered,
        fail_count: 0,
        last_error: Some("IP in ipfilter.dat".to_string()),
        cooldown_until_unix_ms: None,
        user_hash: None,
        crypt_options: 0,
        buddy_id: None,
        software: None,
        availability: None,
        server_ip: 0,
        server_port: 0,
        udp_port: 0,
    };

    let inserted = db
        .upsert_peer_sources(hash, &[s1.clone(), s2.clone()])
        .expect("upsert peer sources");
    assert_eq!(inserted, 2);

    let list = db.list_peer_sources(hash).expect("list peer sources");
    assert_eq!(list.len(), 2);

    // 3. Desduplicación: insertar/actualizar s1 de nuevo con last_seen_unix_ms actualizado
    let s1_updated = PeerSource {
        last_seen_unix_ms: 2000,
        ..s1.clone()
    };
    let inserted2 = db
        .upsert_peer_sources(hash, &[s1_updated])
        .expect("upsert duplicate");
    assert_eq!(inserted2, 1);

    let list = db
        .list_peer_sources(hash)
        .expect("list after deduplication");
    assert_eq!(
        list.len(),
        2,
        "Duplicate (file_hash, address) must not create a new row"
    );
    let found_s1 = list
        .iter()
        .find(|s| s.address == "192.168.1.50:39735")
        .expect("found s1");
    assert_eq!(found_s1.last_seen_unix_ms, 2000);
    assert_eq!(
        found_s1.user_hash.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
    assert!(found_s1.supports_crypt());
    assert!(found_s1.requests_crypt());
    assert!(found_s1.requires_crypt());
    assert!(found_s1.user_hash_bytes().is_some());
    assert_eq!(found_s1.udp_port, 49673);

    // 3b. Una inserción o actualización posterior que desconoce el puerto UDP (0 = desconocido)
    // conserva el valor almacenado — `CASE WHEN excluded.udp_port != 0`.
    let s1_no_udp = PeerSource {
        udp_port: 0,
        last_seen_unix_ms: 3000,
        ..s1.clone()
    };
    db.upsert_peer_sources(hash, &[s1_no_udp])
        .expect("upsert without udp_port");
    let found_s1 = db
        .list_peer_sources(hash)
        .expect("list")
        .into_iter()
        .find(|s| s.address == "192.168.1.50:39735")
        .expect("found s1");
    assert_eq!(found_s1.udp_port, 49673);

    // 4. Actualizar estado y fallo con enfriamiento
    let updated = db
        .update_peer_source_status(
            hash,
            "192.168.1.50:39735",
            PeerSourceStatus::Failed,
            Some("Connection timed out (8s)"),
            Some(1005000),
        )
        .expect("update status");
    assert!(updated);

    let list = db.list_peer_sources(hash).expect("list after fail update");
    let s1_failed = list
        .iter()
        .find(|s| s.address == "192.168.1.50:39735")
        .expect("found s1");
    assert_eq!(s1_failed.status, PeerSourceStatus::Failed);
    assert_eq!(s1_failed.fail_count, 1);
    assert_eq!(
        s1_failed.last_error.as_deref(),
        Some("Connection timed out (8s)")
    );
    assert_eq!(s1_failed.cooldown_until_unix_ms, Some(1005000));

    // 4b. Restablecer enfriamiento (reintento manual)
    let reset = db
        .reset_peer_source_cooldown(hash, "192.168.1.50:39735")
        .expect("reset cooldown");
    assert!(reset);
    let list = db.list_peer_sources(hash).expect("list after reset");
    let s1_reset = list
        .iter()
        .find(|s| s.address == "192.168.1.50:39735")
        .expect("found s1");
    assert_eq!(s1_reset.status, PeerSourceStatus::Allowed);
    assert_eq!(s1_reset.fail_count, 0);
    assert_eq!(s1_reset.cooldown_until_unix_ms, None);

    // 4c. Varios fallos seguidos de una conexión exitosa deben reiniciar `fail_count` a 0
    for _ in 0..2 {
        db.update_peer_source_status(
            hash,
            "192.168.1.50:39735",
            PeerSourceStatus::Failed,
            Some("Temporary error"),
            Some(2_000_000),
        )
        .expect("mark failed");
    }
    db.update_peer_source_status(
        hash,
        "192.168.1.50:39735",
        PeerSourceStatus::Connected,
        None,
        None,
    )
    .expect("mark connected");
    let s1_connected = db
        .list_peer_sources(hash)
        .expect("list after connected")
        .into_iter()
        .find(|s| s.address == "192.168.1.50:39735")
        .expect("found s1");
    assert_eq!(s1_connected.status, PeerSourceStatus::Connected);
    assert_eq!(s1_connected.fail_count, 0);

    // 4d. Al reanunciar una fuente `Failed`, se reactiva solo si su enfriamiento ya venció
    db.update_peer_source_status(
        hash,
        "192.168.1.50:39735",
        PeerSourceStatus::Failed,
        Some("Transient disconnect"),
        Some(5_000_000),
    )
    .expect("mark failed again");
    let re_early = PeerSource {
        status: PeerSourceStatus::Candidate,
        last_seen_unix_ms: 4_000_000,
        cooldown_until_unix_ms: None,
        last_error: None,
        ..s1.clone()
    };
    db.upsert_peer_sources(hash, &[re_early])
        .expect("upsert before cooldown expiry");
    let s1_still_failed = db
        .list_peer_sources(hash)
        .expect("list still failed")
        .into_iter()
        .find(|s| s.address == "192.168.1.50:39735")
        .expect("found s1");
    assert_eq!(s1_still_failed.status, PeerSourceStatus::Failed);

    let re_expired = PeerSource {
        status: PeerSourceStatus::Candidate,
        last_seen_unix_ms: 5_000_000,
        cooldown_until_unix_ms: None,
        last_error: None,
        ..s1.clone()
    };
    db.upsert_peer_sources(hash, &[re_expired])
        .expect("upsert after cooldown expiry");
    let s1_reactivated = db
        .list_peer_sources(hash)
        .expect("list reactivated")
        .into_iter()
        .find(|s| s.address == "192.168.1.50:39735")
        .expect("found s1");
    assert_eq!(s1_reactivated.status, PeerSourceStatus::Candidate);
    assert_eq!(s1_reactivated.cooldown_until_unix_ms, None);
    assert_eq!(s1_reactivated.last_error, None);

    // 5. Eliminar una sola fuente
    let deleted = db
        .delete_peer_source(hash, "10.0.0.88:39735")
        .expect("delete source");
    assert!(deleted);
    let list = db.list_peer_sources(hash).expect("list after delete");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].address, "192.168.1.50:39735");

    // 6. Limpiar fuentes para el hash de archivo
    let cleared = db.clear_peer_sources(hash).expect("clear sources");
    assert_eq!(cleared, 1);
    let list = db.list_peer_sources(hash).expect("list after clear");
    assert_eq!(list.len(), 0);
}

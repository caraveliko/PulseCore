// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use super::*;
use crate::services::peer_session::{PeerConnectionStatus, PeerSessionSnapshot};
use crate::services::swarm_coordinator::types::SwarmCoordinator;
use mule_core::model::PeerSource;
use mule_test_support::peer_fixtures::peer_source;
use std::collections::HashMap;

/// Estado mínimo del daemon en memoria — `mule_test_support` no puede
/// utilizarse aquí: sus auxiliares devuelven el `AppState` del crate externo
/// (versión duplicada del tipo en pruebas unitarias).
fn test_state(dir: &std::path::Path, hash_hex: &str) -> crate::state::AppState {
    let config = crate::config::DaemonConfig::new(dir.join("data"), "test-token")
        .with_workdir_mode(crate::workdir::WorkDirMode::Portable);
    let mut downloads =
        crate::services::downloads::DownloadService::in_memory(dir.join("temp")).unwrap();
    let link = format!("ed2k://|file|f.mkv|10000000|{hash_hex}|/");
    downloads
        .add_ed2k_link(
            &link,
            &dir.join("downloads"),
            mule_core::storage::part_file::AllocationPolicy::None,
            None,
        )
        .unwrap();
    crate::state::AppState::new(config, downloads)
}

/// Contexto «LowID sin Kad» — `can_do_callback` falso para toda
/// fuente (equivalente al anterior `server_is_high_id = false`).
fn test_cb_ctx() -> crate::services::swarm_coordinator::lowid_callback::CallbackCtx {
    crate::services::swarm_coordinator::lowid_callback::CallbackCtx {
        server_connected: true,
        server_high_id: false,
        our_server_ip: 0,
        our_server_port: 0,
        kad_open: false,
    }
}

fn coordinator(state: crate::state::AppState) -> SwarmCoordinator {
    SwarmCoordinator {
        state,
        last_source_query_tcp: HashMap::new(),
        last_source_query_udp: HashMap::new(),
        kad_source_search: HashMap::new(),
        last_kad_file_request: Instant::now(),
        last_heartbeat: Instant::now(),
        server_attempts: HashMap::new(),
        last_server_reconnect: None,
        last_connected_server: None,
        last_source_purge: HashMap::new(),
        last_a4af_full: Instant::now(),
        download_trackers: HashMap::new(),
    }
}

/// `CPartFile::Process` de eMule (`PartFile.cpp:2285-2295`): al ≥80 % de
/// `GetMaxSources`, una fuente `DS_ONQUEUE && IsRemoteQueueFull` es
/// purgada — la sesión activa se termina y la fila de fuente se elimina,
/// mientras que una fuente conectada sin bandera sobrevive.
#[tokio::test]
async fn purge_evicts_remote_queue_full_sources_at_80_percent() {
    let dir = tempfile::tempdir().unwrap();
    let hash = "0123456789abcdef0123456789abcdef".to_string();
    let state = test_state(dir.path(), &hash);
    let file_max = 10usize;
    // 8 fuentes sobre un límite de 10 → 80 %, umbral de eMule alcanzado.
    let sources: Vec<PeerSource> = (0..8)
        .map(|i| peer_source(&hash, &format!("10.0.0.{i}:49662")))
        .collect();
    {
        let mut dl = state.downloads.lock().await;
        dl.upsert_peer_sources(&hash, &sources).unwrap();
    }

    // Sesión activa en cola + remote_queue_full en la fuente 10.0.0.3.
    state
        .peer_session
        .insert_test_session(PeerSessionSnapshot {
            status: PeerConnectionStatus::Queued,
            file_hash: Some(hash.clone()),
            peer_address: Some("10.0.0.3:49662".to_string()),
            remote_queue_full: true,
            ..Default::default()
        })
        .await;

    let mut coord = coordinator(state.clone());
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    coord
        .purge_excess_sources(&hash, &sources, file_max, &test_cb_ctx(), now_ms)
        .await;

    // La fuente RQF es eliminada de la lista persistida.
    let remaining = state
        .downloads
        .lock()
        .await
        .list_peer_sources(&hash)
        .unwrap();
    assert_eq!(remaining.len(), 7);
    assert!(!remaining.iter().any(|s| s.address == "10.0.0.3:49662"));
    // Su sesión activa se termina (`RemoveSource` de eMule destruye el
    // cliente de descarga).
    let snap = state.peer_session.swarm_snapshot(Some(&hash)).await;
    assert!(snap.peers.is_empty());
}

/// Por debajo del umbral del 80 %, no se realiza ninguna purga incluso con una sesión
/// remote-queue-full.
#[tokio::test]
async fn purge_keeps_remote_queue_full_below_80_percent() {
    let dir = tempfile::tempdir().unwrap();
    let hash = "0123456789abcdef0123456789abcdef".to_string();
    let state = test_state(dir.path(), &hash);
    let sources: Vec<PeerSource> = (0..5)
        .map(|i| peer_source(&hash, &format!("10.0.1.{i}:49662")))
        .collect();
    {
        let mut dl = state.downloads.lock().await;
        dl.upsert_peer_sources(&hash, &sources).unwrap();
    }
    state
        .peer_session
        .insert_test_session(PeerSessionSnapshot {
            status: PeerConnectionStatus::Queued,
            file_hash: Some(hash.clone()),
            peer_address: Some("10.0.1.3:49662".to_string()),
            remote_queue_full: true,
            ..Default::default()
        })
        .await;

    let mut coord = coordinator(state.clone());
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    coord
        .purge_excess_sources(&hash, &sources, 10, &test_cb_ctx(), now_ms)
        .await;

    let remaining = state
        .downloads
        .lock()
        .await
        .list_peer_sources(&hash)
        .unwrap();
    assert_eq!(remaining.len(), 5);
    let snap = state.peer_session.swarm_snapshot(Some(&hash)).await;
    assert_eq!(snap.peers.len(), 1);
}

fn udp_target(
    connected_at_unix_ms: Option<i64>,
    next_tcp_reask_unix_ms: Option<i64>,
) -> crate::services::client_udp::UdpReaskTarget {
    crate::services::client_udp::UdpReaskTarget {
        addr: "10.0.0.9:49672".parse().unwrap(),
        file_hash: [7u8; 16],
        connected_at_unix_ms,
        next_tcp_reask_unix_ms,
    }
}

/// `PartFile.cpp:2291-2294` de eMule — límites de la ventana UDP: el ping
/// se envía únicamente cuando la repetición TCP llega en `(1 s, 2 min)`,
/// la conexión tiene ≥ 20 min y no estamos tras cortafuegos.
#[test]
fn udp_reask_window_boundaries() {
    let now_ms = 1_000_000_000i64;
    let old_conn = Some(now_ms - super::UDP_REASK_MIN_CONNECT_AGE_MS);
    // Deadline pile à 2 min → hors fenêtre (eMule exige < MIN2MS(2)).
    assert!(!super::should_send_udp_reask(
        &udp_target(old_conn, Some(now_ms + super::UDP_REASK_WINDOW_MS)),
        now_ms,
        false,
    ));
    // 1 ms por debajo del límite superior → dentro de la ventana.
    assert!(super::should_send_udp_reask(
        &udp_target(old_conn, Some(now_ms + super::UDP_REASK_WINDOW_MS - 1)),
        now_ms,
        false,
    ));
    // Deadline pile à 1 s → hors fenêtre (eMule exige > SEC2MS(1)).
    assert!(!super::should_send_udp_reask(
        &udp_target(old_conn, Some(now_ms + super::UDP_REASK_MIN_LEAD_MS)),
        now_ms,
        false,
    ));
    // 1 ms por encima del límite inferior → dentro de la ventana.
    assert!(super::should_send_udp_reask(
        &udp_target(old_conn, Some(now_ms + super::UDP_REASK_MIN_LEAD_MS + 1)),
        now_ms,
        false,
    ));
    // Plazo límite ya vencido → nunca se envía ping.
    assert!(!super::should_send_udp_reask(
        &udp_target(old_conn, Some(now_ms - 1)),
        now_ms,
        false,
    ));
}

/// eMule exige `curTick >= LastTriedToConnectTime + MIN2MS(20)`: una
/// sesión en cola desde hace menos de 20 min no dispara ningún ping.
#[test]
fn udp_reask_requires_old_connection() {
    let now_ms = 1_000_000_000i64;
    let in_window = Some(now_ms + 60_000);
    // Connexion de 19 min → refusé ; pile 20 min → accepté
    // (eMule utilise >=).
    assert!(!super::should_send_udp_reask(
        &udp_target(
            Some(now_ms - super::UDP_REASK_MIN_CONNECT_AGE_MS + 1),
            in_window,
        ),
        now_ms,
        false,
    ));
    assert!(super::should_send_udp_reask(
        &udp_target(
            Some(now_ms - super::UDP_REASK_MIN_CONNECT_AGE_MS),
            in_window,
        ),
        now_ms,
        false,
    ));
    // `connected_at` desconocido → permisivo (el campo siempre está relleno
    // para una sesión real; ausente = sin prueba de frescura).
    assert!(super::should_send_udp_reask(
        &udp_target(None, in_window),
        now_ms,
        false,
    ));
    // Sin plazo publicado (sesión apenas en cola) → sin ping.
    assert!(!super::should_send_udp_reask(
        &udp_target(Some(now_ms - super::UDP_REASK_MIN_CONNECT_AGE_MS), None,),
        now_ms,
        false,
    ));
}

/// `!theApp.IsFirewalled()` de eMule — un cliente inalcanzable no emite
/// re-asks por UDP (el NAT descartaría la respuesta).
#[test]
fn udp_reask_skipped_when_firewalled() {
    let now_ms = 1_000_000_000i64;
    let target = udp_target(
        Some(now_ms - super::UDP_REASK_MIN_CONNECT_AGE_MS),
        Some(now_ms + 60_000),
    );
    assert!(!super::should_send_udp_reask(&target, now_ms, true));
}

#[test]
fn test_smart_queue_stalled_download_rotates_below_queued_downloads() {
    let stalled_dl = DownloadSnapshot {
        id: uuid::Uuid::new_v4(),
        file_name: "StalledAlmostDone.mkv".to_string(),
        total_bytes: 1_000_000,
        received_bytes: 999_000,
        verified_bytes: 999_000,
        progress: 99.9,
        status: "downloading".to_string(),
        priority: "normal".to_string(),
        source_count: 4,
        download_rate_bps: 0,
        uploaded_bytes: 0,
        created_at_unix_ms: 1000,
        updated_at_unix_ms: 2000,
        category_id: None,
        eta_seconds: None,
        ed2k_hash: "1234567890abcdef1234567890abcdef".to_string(),
        parts: vec![],
        pending_ranges: vec![],
        max_sources: None,
        nas_copy_active: None,
        completion_stage: None,
        copy_progress: None,
        protocol: None,
    };

    let queued_dl = DownloadSnapshot {
        id: uuid::Uuid::new_v4(),
        file_name: "FreshQueuedWithSources.mkv".to_string(),
        total_bytes: 1_000_000,
        received_bytes: 0,
        verified_bytes: 0,
        progress: 0.0,
        status: "queued".to_string(),
        priority: "normal".to_string(),
        source_count: 15,
        download_rate_bps: 0,
        uploaded_bytes: 0,
        created_at_unix_ms: 1500,
        updated_at_unix_ms: 2000,
        category_id: None,
        eta_seconds: None,
        ed2k_hash: "abcdef1234567890abcdef1234567890".to_string(),
        parts: vec![],
        pending_ranges: vec![],
        max_sources: None,
        nas_copy_active: None,
        completion_stage: None,
        copy_progress: None,
        protocol: None,
    };

    let active_score = super::calculate_smart_download_score(&stalled_dl, 1, 4, false, None);
    let queued_score = super::calculate_smart_download_score(&queued_dl, 0, 15, false, None);
    assert!(active_score > queued_score);

    let rotated_score = super::calculate_smart_download_score(&stalled_dl, 0, 4, true, None);
    assert!(rotated_score < queued_score);
    assert_eq!(rotated_score, -50_000.0);
}

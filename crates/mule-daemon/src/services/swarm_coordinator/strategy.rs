// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Tick de evaluación del enjambre, validación de hashes y planificación de conexiones.

use crate::services::peer_session::log_addr_str;
use crate::services::peer_session::log_peer_addr;
use crate::services::peer_session::log_peer_ip;
use crate::services::peer_session::log_server_addr;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mule_core::model::{DownloadStatus, PeerSourceOrigin, PeerSourceStatus};
use mule_protocol::ed2k::GetSourcesRequest;

use crate::services::downloads::types::DownloadSnapshot;
use crate::services::server_session::ServerConnectionStatus;

use super::types::{
    max_source_per_file_soft, max_source_per_file_udp, A4afFileState, DownloadActivityTracker,
    KadSourceSearchState, SwarmCoordinator, SwarmLimits, HEARTBEAT_INTERVAL, KADEMLIA_ASK_INTERVAL,
    KADEMLIA_REASK_INTERVAL, KAD_MAX_FILE_SEARCHES, KAD_MAX_TOTAL_SEARCHES, MAX_CONNECT_PER_TICK,
    MAX_GLOBAL_CONNECT_PER_TICK, SERVER_REASK_INTERVAL, SMART_QUEUE_ROTATION_COOLDOWN,
    SMART_QUEUE_STALL_TIMEOUT, UDP_SERVER_REASK_INTERVAL,
};

/// `PartFile.cpp:2291` de eMule — `UDPReaskForDownload` se dispara mientras
/// la siguiente repetición de petición TCP de la fuente dista menos de 2 minutos
/// (`GetTimeUntilReask() < MIN2MS(2)`).
const UDP_REASK_WINDOW_MS: i64 = 2 * 60 * 1_000;
/// …pero aún a más de 1 segundo de distancia (`GetTimeUntilReask() > SEC2MS(1)`)
/// — la respuesta UDP necesita ~1 min para llegar antes de la repetición TCP, y la
/// ventana evita enviar pings a una fuente cuya repetición TCP ya se disparó.
const UDP_REASK_MIN_LEAD_MS: i64 = 1_000;
/// …y únicamente cuando el último intento de conexión tiene al menos 20 min de antigüedad
/// (`curTick >= GetLastTriedToConnectTime() + MIN2MS(20)`).
const UDP_REASK_MIN_CONNECT_AGE_MS: i64 = 20 * 60 * 1_000;

/// `CPartFile::Process` de eMule (`PartFile.cpp:2291-2294`): una fuente `DS_ONQUEUE`
/// recibe una repetición de petición UDP por ciclo, dentro de la ventana de `(1 s, 2 min)`
/// anterior a su repetición TCP, cuando la conexión tiene ≥ 20 min y nosotros no estamos
/// tras cortafuegos (un NAT descartaría la respuesta — eMule filtra con
/// `!theApp.IsFirewalled()`).
fn should_send_udp_reask(
    target: &crate::services::client_udp::UdpReaskTarget,
    now_ms: i64,
    is_firewalled: bool,
) -> bool {
    if is_firewalled {
        return false;
    }
    let Some(next_reask) = target.next_tcp_reask_unix_ms else {
        return false;
    };
    let until_reask = next_reask.saturating_sub(now_ms);
    if until_reask <= UDP_REASK_MIN_LEAD_MS || until_reask >= UDP_REASK_WINDOW_MS {
        return false;
    }
    if let Some(connected_at) = target.connected_at_unix_ms {
        if now_ms.saturating_sub(connected_at) < UDP_REASK_MIN_CONNECT_AGE_MS {
            return false;
        }
    }
    true
}

/// Puntuación heurística inteligente para seleccionar las descargas concurrentes principales (máx 4).
/// Da prioridad a:
/// 1. Prioridad explícita del usuario (alta > normal > baja)
/// 2. Proximidad al 100% de compleción (terminar primero los archivos casi completos para liberar disco y ranuras)
/// 3. Transferencias activas en curso (bonificación anti-oscilaciones para archivos que reciben bytes actualmente)
/// 4. Disponibilidad de fuentes descubiertas (archivos con fuentes candidatas conocidas)
///
/// Penaliza:
/// - Descargas inactivas/estancadas en enfriamiento de rotación (-50.000 puntos)
/// - Duración de estancamiento prolongado con rendimiento cero en descargas activas (-500 puntos por cada 30 s)
fn calculate_smart_download_score(
    dl: &DownloadSnapshot,
    active_peers: usize,
    source_count: usize,
    is_in_cooldown: bool,
    stall_duration: Option<Duration>,
) -> f64 {
    if is_in_cooldown {
        return -50_000.0;
    }

    let priority_weight = match dl.priority.to_lowercase().as_str() {
        "alta" | "high" => 10_000.0,
        "baja" | "low" => 1_000.0,
        _ => 5_000.0,
    };

    // dl.progress es 0.0 .. 100.0. Mayor progreso = mayor puntuación para completar rápidamente.
    // Si el archivo tiene 0 fuentes conocidas y 0 pares activos, no inflar su puntuación con el progreso
    // para que no bloquee archivos con fuentes viables.
    let progress_weight = if source_count == 0 && active_peers == 0 {
        0.0
    } else {
        dl.progress.clamp(0.0, 100.0) * 40.0
    };

    // Bonificación anti-oscilaciones / actividad: proteger descargas en curso para que no sean intercambiadas
    let activity_bonus = if dl.download_rate_bps > 0 {
        3_000.0
    } else if active_peers > 0 {
        (active_peers as f64 * 400.0).min(2_000.0)
    } else if dl.status.to_lowercase() == "downloading" {
        500.0
    } else {
        0.0
    };

    // Bonificación de viabilidad de fuentes: los archivos con fuentes viables en BD obtienen un impulso moderado
    let source_bonus = if source_count > 0 {
        (source_count as f64 * 20.0).min(1_000.0)
    } else {
        0.0
    };

    // Atenuación por estancamiento: si la descarga está activa pero estancada con 0 B/s, atenuar su puntuación
    let stall_penalty = if let Some(stall) = stall_duration {
        (stall.as_secs_f64() / 30.0) * 500.0
    } else {
        0.0
    };

    priority_weight + progress_weight + activity_bonus + source_bonus - stall_penalty
}

impl SwarmCoordinator {
    /// `CemuleApp::IsFirewalled` de eMule — inalcanzable desde el exterior:
    /// ni un HighID en el servidor conectado ni un socket UDP de Kad abierto.
    /// Los clientes tras cortafuegos omiten por completo los re-asks por UDP.
    async fn is_firewalled(&self) -> bool {
        let s_snap = self.state.server_session.snapshot().await;
        let server_high_id =
            s_snap.status == ServerConnectionStatus::Connected && s_snap.is_high_id;
        let udp_snap = self.state.kad.udp().get_snapshot().await;
        let fw_status = {
            let tracker = self.state.kad.udp().firewall_tracker();
            let fw = tracker.lock().await;
            fw.status
        };
        !(server_high_id
            || (udp_snap.is_active
                && fw_status == crate::services::kad_udp::types::UdpFirewallStatus::Open))
    }

    /// Evalúa todas las descargas activas, ejecuta el descubrimiento de fuentes y se conecta a los pares.
    pub async fn tick(&mut self) {
        // Latido periódico consolidado del sistema (cada 60 s)
        if self.last_heartbeat.elapsed() >= HEARTBEAT_INTERVAL {
            self.last_heartbeat = Instant::now();
            self.emit_heartbeat().await;
            // Cadencia SavePartFile de eMule: persistir acompañantes .part.met para
            // descargas en progreso para que una caída nunca pierda más de ~60 s
            // de estado de huecos. E/S síncronas (fsync + escritura .met) delegadas
            // al grupo de hilos bloqueante de Tokio.
            let downloads = Arc::clone(&self.state.downloads);
            if let Err(e) = tokio::task::spawn_blocking(move || {
                downloads.blocking_lock().save_active_part_mets();
            })
            .await
            {
                tracing::error!("save_active_part_mets spawn_blocking failed: {e}");
            }
        }

        // 0. Autorreparación de UDP de Kad: revivir el oyente si el perfil permite UDP y el estado está habilitado pero el socket está inactivo
        let profile = self.state.network_policy.profile();
        if self.state.udp_enabled() && profile.allows_udp_kad() {
            let snap = self.state.kad.udp().get_snapshot().await;
            if !snap.is_active {
                let bind_ip = if profile.is_loopback_only() {
                    "127.0.0.1"
                } else {
                    "0.0.0.0"
                };
                if let Ok(addr) = format!("{bind_ip}:{}", self.state.kad_port()).parse() {
                    if let Err(e) = self.state.kad.udp().start(addr).await {
                        tracing::warn!("Swarm self-healing: Kad UDP start failed on {addr}: {e}");
                    } else {
                        tracing::info!("Swarm self-healing: successfully revived Kad UDP on {addr}");
                    }
                }
            }
        }

        // 0. Decay stale download rates (reset frozen speed to 0 if inactive > 2s)
        let decayed_snapshots = {
            // Respaldo de persistencia: fsync de los `.part` sucios cuando el
            // flujo de bloques se interrumpió entre dos `inject_range`
            // (la cadencia principal reside en la ruta de recepción —
            // `PART_PERSIST_INTERVAL`). E/S fuera del ejecutor de Tokio.
            let downloads = Arc::clone(&self.state.downloads);
            match tokio::task::spawn_blocking(move || {
                let mut dl_guard = downloads.blocking_lock();
                dl_guard.flush_dirty_downloads();
                dl_guard.decay_stale_rates()
            })
            .await
            {
                Ok(snapshots) => snapshots,
                Err(e) => {
                    tracing::error!("flush_dirty_downloads spawn_blocking failed: {e}");
                    Vec::new()
                }
            }
        };
        for mut snap in decayed_snapshots {
            snap.pending_ranges = self.state.peer_session.try_pending_ranges(&snap.ed2k_hash);
            crate::services::peer_session::emit_download_update(
                &self.state.event_tx,
                &self.state.sequence,
                &snap,
            );
        }

        // 1. Autoverificar y finalizar cualquier descarga completada
        let pending_verifications = {
            let mut dl_guard = self.state.downloads.lock().await;
            dl_guard.get_pending_verifications()
        };

        for (id, temp_path, target_path, expected_hash, total_size) in pending_verifications {
            let state_clone = self.state.clone();
            tokio::spawn(async move {
                // Ruta rápida (`PerformFileComplete` / `CalculateIdentifier` de eMule):
                // la raíz eD2K se deriva del conjunto de hashes de partes ya verificadas
                // en la recepción — ninguna relectura del archivo completo (~total_size de `pread`
                // evitada). Repliegue sobre `hash_reader` cuando el hashset es
                // desconocido/incompleto o contradice el hash esperado.
                let derived_ok = {
                    let dl_guard = state_clone.downloads.lock().await;
                    dl_guard
                        .try_verified_root(&id)
                        .map(|root| root == expected_hash)
                        .unwrap_or(false)
                };
                let hash_res = if derived_ok {
                    tracing::info!(
                        "Download {id}: eD2K root derived from verified part hashset — skipping full file re-read ({total_size} bytes)"
                    );
                    Ok(Ok(mule_format::ed2k::Ed2kHashResult {
                        root_hash: mule_format::Ed2kHash(expected_hash),
                        part_hashes: Vec::new(),
                        total_bytes: total_size,
                    }))
                } else {
                    tracing::info!(
                        "Starting eD2K hash verification for download {id} ({total_size} bytes, temp_path={})...",
                        temp_path.display()
                    );
                    let temp_path_for_hashing = temp_path.clone();
                    tokio::task::spawn_blocking(move || {
                        let mut file = std::fs::File::open(&temp_path_for_hashing)?;
                        mule_format::ed2k::hash_reader(&mut file)
                    })
                    .await
                };

                match hash_res {
                    Ok(Ok(computed)) => {
                        if computed.root_hash.0 == expected_hash {
                            let prep = {
                                let mut dl_guard = state_clone.downloads.lock().await;
                                dl_guard.prepare_finalize(&id)
                            };
                            match prep {
                                Ok((temp_path, target_path, snap)) => {
                                    state_clone.broadcast_download_update(snap.clone());
                                    tracing::info!(
                                        "Download {id} ('{}') verified eD2K hash ({}) - State: Completing (copying asynchronously to NAS/destination: {})",
                                        snap.file_name,
                                        hex::encode(expected_hash),
                                        target_path.display()
                                    );
                                    let state_bg = state_clone.clone();
                                    tokio::task::spawn(async move {
                                        tracing::info!(
                                            "Download {id} ('{}') queued in FIFO for NAS transfer (waiting for single-thread disk lock)...",
                                            snap.file_name
                                        );
                                        let permit = state_bg.nas_transfer_semaphore.acquire().await;
                                        match permit {
                                            Ok(_permit) => {
                                                {
                                                    let mut active_lock = state_bg.active_nas_transfer_id.lock().await;
                                                    *active_lock = Some(id);
                                                }
                                                // Pre-flight space check on destination
                                                let parent_dir = target_path.parent().unwrap_or(&target_path);
                                                let free_target = fs4::available_space(parent_dir).ok();
                                                let file_size = std::fs::metadata(&temp_path).map(|m| m.len()).unwrap_or(0);
                                                if let Some(free) = free_target {
                                                    if file_size > 0 && free < file_size.saturating_add(50 * 1024 * 1024) {
                                                        tracing::error!(
                                                            "Download {id}: Insufficient space on NAS destination (needs {} bytes, available: {} bytes)",
                                                            file_size,
                                                            free
                                                        );
                                                        let mut dl_guard = state_bg.downloads.lock().await;
                                                        if let Ok(snap) = dl_guard.update_status(
                                                            &id,
                                                            mule_core::model::DownloadStatus::Failed,
                                                        ) {
                                                            state_bg.broadcast_download_update(snap);
                                                        }
                                                        let mut active_lock = state_bg.active_nas_transfer_id.lock().await;
                                                        *active_lock = None;
                                                        return;
                                                    }
                                                }

                                                // Transmitir actualización indicando que esta descarga está escribiendo activamente en el NAS
                                                if let Some(mut cur_snap) = {
                                                    let dl_guard = state_bg.downloads.lock().await;
                                                    dl_guard.get_download(&id)
                                                } {
                                                    cur_snap.nas_copy_active = Some(true);
                                                    state_bg.broadcast_download_update(cur_snap);
                                                }
                                                tracing::info!(
                                                    "Download {id} ('{}') acquired NAS lock, transferring to destination now...",
                                                    snap.file_name
                                                );

                                                let final_res = tokio::task::spawn_blocking(move || {
                                                    mule_core::finalize_file(
                                                        &temp_path,
                                                        &target_path,
                                                        mule_core::CollisionStrategy::Rename,
                                                    )
                                                })
                                                .await;

                                                {
                                                    let mut active_lock = state_bg.active_nas_transfer_id.lock().await;
                                                    *active_lock = None;
                                                }

                                                match final_res {
                                                    Ok(Ok(final_path)) => {
                                                        let mut dl_guard = state_bg.downloads.lock().await;
                                                        match dl_guard.commit_finalize(&id, final_path.clone()) {
                                                            Ok(snap) => {
                                                                state_bg.broadcast_download_update(snap.clone());
                                                                tracing::info!(
                                                                    "Download {id} ('{}') successfully finished copying to NAS and finalized",
                                                                    snap.file_name
                                                                );
                                                                drop(dl_guard);

                                                                // Organizar inmediatamente el archivo de forma nativa en la biblioteca (Películas/Series)
                                                                let state_org = state_bg.clone();
                                                                let id_str = id.to_string();
                                                                let fp = final_path.clone();
                                                                tokio::spawn(async move {
                                                                    match crate::services::MediaOrganizer::organize_download(&state_org, &id_str, &fp).await {
                                                                        Ok(Some(dest)) => {
                                                                            tracing::info!("[ORGANIZER] Instantly organized download {id_str} -> '{}'", dest.display());
                                                                        }
                                                                        Ok(None) => {}
                                                                        Err(e) => {
                                                                            tracing::warn!("[ORGANIZER] Instant organize failed for {id_str}: {e}");
                                                                        }
                                                                    }
                                                                });
                                                            }
                                                            Err(e) => {
                                                                tracing::error!("Failed to commit finalize for download {id}: {e}");
                                                            }
                                                        }
                                                    }
                                                    Ok(Err(e)) => {
                                                        tracing::error!("Failed to copy download {id} to destination/NAS: {e}");
                                                        let mut dl_guard = state_bg.downloads.lock().await;
                                                        if let Ok(snap) = dl_guard.update_status(
                                                            &id,
                                                            mule_core::model::DownloadStatus::Failed,
                                                        ) {
                                                            state_bg.broadcast_download_update(snap);
                                                        }
                                                    }
                                                    Err(e) => {
                                                        tracing::error!("Join error during finalize task for {id}: {e}");
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                tracing::error!("Failed to acquire NAS semaphore for download {id}: {e}");
                                            }
                                        }
                                    });
                                }
                                Err(e) => {
                                    tracing::error!("Failed to prepare finalize for download {id}: {e}");
                                }
                            }
                        } else {
                            // Discrepancia global de MD4: si se conoce el conjunto de hashes por parte,
                            // identificar las partes dañadas y desalojar solo esos rangos
                            // para volver a descargarlos (recuperación de partes dañadas de CPartFile en eMule)
                            // en lugar de hacer fallar todo el archivo.
                            let expected_part_hashes = {
                                let dl_guard = state_clone.downloads.lock().await;
                                dl_guard.get_part_hashes_for_download(&id)
                            };

                            let mut corrupted: Vec<u32> = Vec::new();
                            let mut verified_ok: Vec<u32> = Vec::new();
                            if !expected_part_hashes.is_empty() {
                                for (i, expected_ph) in expected_part_hashes.iter().enumerate() {
                                    match computed.part_hashes.get(i) {
                                        Some(actual) if actual == expected_ph => {
                                            verified_ok.push(i as u32)
                                        }
                                        _ => corrupted.push(i as u32),
                                    }
                                }
                            }

                            if corrupted.is_empty() {
                                let (covered_bytes, missing_ranges) = {
                                    let dl_guard = state_clone.downloads.lock().await;
                                    (
                                        dl_guard.get_covered_len(&id).unwrap_or(0),
                                        dl_guard.get_missing_ranges(&id).unwrap_or_default(),
                                    )
                                };
                                let disk_len =
                                    std::fs::metadata(&temp_path).map(|m| m.len()).unwrap_or(0);
                                tracing::error!(
                                    "Download {id} hash mismatch: expected {}, got {}. File size: expected={} bytes, on-disk={} bytes, covered_in_ranges={} bytes. Missing ranges: {:?}",
                                    hex::encode(expected_hash),
                                    hex::encode(computed.root_hash.0),
                                    total_size,
                                    disk_len,
                                    covered_bytes,
                                    missing_ranges
                                );
                                let mut dl_guard = state_clone.downloads.lock().await;
                                if let Ok(snap) = dl_guard
                                    .update_status(&id, mule_core::model::DownloadStatus::Failed)
                                {
                                    state_clone.broadcast_download_update(snap);
                                }
                            } else {
                                tracing::warn!(
                                    "Download {id} root hash mismatch: {} corrupted part(s) {:?} out of {} — evicting for re-download",
                                    corrupted.len(),
                                    corrupted,
                                    expected_part_hashes.len()
                                );
                                let mut dl_guard = state_clone.downloads.lock().await;
                                for part_idx in verified_ok {
                                    let _ = dl_guard.db_mut().mark_part_verified(&id, part_idx);
                                }
                                let evict_ok = match dl_guard.evict_corrupted_parts(&id, &corrupted)
                                {
                                    Ok(snap) => {
                                        state_clone.broadcast_download_update(snap);
                                        true
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to evict corrupted parts for download {id}: {e}");
                                        false
                                    }
                                };
                                // `CorruptionBlackBox::EvaluateData` de eMule pudo
                                // condenar a remitentes: desconexión +
                                // rechazo de sus fuentes.
                                let banned = dl_guard.drain_corruption_bans(&id);
                                drop(dl_guard);
                                for addr in banned {
                                    tracing::warn!(
                                        "[CORRUPTION_BLACKBOX] Banning {addr} for download {id} (>32% corrupted data)"
                                    );
                                    let mut dl_guard = state_clone.downloads.lock().await;
                                    let now_ms = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis() as i64;
                                    let _ = dl_guard.update_peer_source_status(
                                        &hex::encode(expected_hash),
                                        &addr,
                                        mule_core::model::PeerSourceStatus::Allowed,
                                        Some("corruption blackbox: temporary cooldown"),
                                        Some(now_ms + 15 * 60 * 1000),
                                    );
                                    drop(dl_guard);
                                    let _ =
                                        state_clone.peer_session.disconnect_peer(Some(&addr)).await;
                                }
                                if !evict_ok {
                                    let mut dl_guard = state_clone.downloads.lock().await;
                                    if let Ok(snap) = dl_guard.update_status(
                                        &id,
                                        mule_core::model::DownloadStatus::Failed,
                                    ) {
                                        state_clone.broadcast_download_update(snap);
                                    }
                                }
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::error!(
                            "Download {id} hash reader failed for {}: {e}",
                            temp_path.display()
                        );
                        let mut dl_guard = state_clone.downloads.lock().await;
                        let already_finalized = target_path.exists()
                            && std::fs::metadata(&target_path)
                                .map(|m| m.len())
                                .unwrap_or(0)
                                == total_size;
                        if already_finalized {
                            tracing::info!(
                                "Download {id} target file already exists in {}; marking Completed",
                                target_path.display()
                            );
                            if let Ok(snap) = dl_guard
                                .update_status(&id, mule_core::model::DownloadStatus::Completed)
                            {
                                state_clone.broadcast_download_update(snap);
                            }
                        } else if let Ok(snap) =
                            dl_guard.update_status(&id, mule_core::model::DownloadStatus::Failed)
                        {
                            state_clone.broadcast_download_update(snap);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Download {id} spawn_blocking failed: {e}");
                        let mut dl_guard = state_clone.downloads.lock().await;
                        if let Ok(snap) =
                            dl_guard.update_status(&id, mule_core::model::DownloadStatus::Failed)
                        {
                            state_clone.broadcast_download_update(snap);
                        }
                    }
                }
            });
        }

        // 1. Omitir si la red está deshabilitada por política
        if self.state.network_policy.state() == mule_network_policy::NetworkState::Disabled {
            return;
        }

        // 1b. Conmutación por error automática de servidor: si la conexión del servidor eD2K cayó,
        // elegir otro servidor conocido habilitado tras un enfriamiento (semántica de reintento DeadServer
        // de eMule, acotada para que una lista de servidores muertos no gire en bucle).
        self.maybe_reconnect_server().await;

        // 2. Fetch list of eligible downloads (active or queued)
        let eligible_downloads: Vec<DownloadSnapshot> = {
            let dl_guard = self.state.downloads.lock().await;
            dl_guard
                .list_downloads()
                .into_iter()
                .filter(|d| {
                    let s = d.status.to_lowercase();
                    s == "queued"
                        || s == "downloading"
                        || s == "waitingforsources"
                        || s == "waiting_for_sources"
                })
                .collect()
        };

        if eligible_downloads.is_empty() {
            return;
        }

        let now_instant = Instant::now();

        // Smart download concurrency limiter (Default: 30 active downloads)
        let max_concurrent_downloads = {
            let dl_guard = self.state.downloads.lock().await;
            dl_guard
                .get_setting("max_active_downloads")
                .ok()
                .flatten()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(30)
                .clamp(1, 100)
        };

        // 1. Mantener rastreadores de actividad para todas las descargas elegibles
        let eligible_ids: std::collections::HashSet<uuid::Uuid> =
            eligible_downloads.iter().map(|d| d.id).collect();
        self.download_trackers.retain(|id, _| eligible_ids.contains(id));

        for dl in &eligible_downloads {
            let is_currently_active = dl.status.to_lowercase() == "downloading"
                || dl.status.to_lowercase() == "waitingforsources"
                || dl.status.to_lowercase() == "waiting_for_sources";

            let tracker = self
                .download_trackers
                .entry(dl.id)
                .or_insert_with(|| DownloadActivityTracker {
                    last_received_bytes: dl.received_bytes,
                    last_progress_time: now_instant,
                    activated_at: now_instant,
                    cooldown_until: None,
                });

            // Check if rotation cooldown has expired
            if let Some(until) = tracker.cooldown_until {
                if now_instant >= until {
                    tracing::info!(
                        "Smart Queue: Download {} ('{}') rotation cooldown expired. Returning to candidate pool.",
                        dl.id,
                        dl.file_name
                    );
                    tracker.cooldown_until = None;
                    tracker.last_progress_time = now_instant;
                    tracker.activated_at = now_instant;
                    tracker.last_received_bytes = dl.received_bytes;
                }
            }

            let active_peers = self
                .state
                .peer_session
                .active_peer_count_for_hash(&dl.ed2k_hash)
                .await;

            // Comprobación de avance: si llegaron bytes, tasa > 0, o conectado activamente a pares en cola, reiniciar temporizador de estancamiento y limpiar enfriamiento
            if dl.received_bytes > tracker.last_received_bytes || dl.download_rate_bps > 0 || active_peers > 0 {
                tracker.last_received_bytes = dl.received_bytes;
                tracker.last_progress_time = now_instant;
                tracker.cooldown_until = None;
            } else if is_currently_active && tracker.cooldown_until.is_none() {
                // Si ha estado activa con 0 bytes transferidos Y 0 pares conectados por más tiempo que STALL_TIMEOUT:
                let stall_duration = now_instant.saturating_duration_since(tracker.last_progress_time);
                let time_active = now_instant.saturating_duration_since(tracker.activated_at);

                if stall_duration >= SMART_QUEUE_STALL_TIMEOUT
                    && time_active >= SMART_QUEUE_STALL_TIMEOUT
                {
                    tracing::info!(
                        "Smart Queue: Download {} ('{}') stalled (no bytes received and 0 connected peers for {:.0}s). Rotating to queue (cooldown {:.0}s)...",
                        dl.id,
                        dl.file_name,
                        stall_duration.as_secs_f64(),
                        SMART_QUEUE_ROTATION_COOLDOWN.as_secs_f64()
                    );
                    tracker.cooldown_until = Some(now_instant + SMART_QUEUE_ROTATION_COOLDOWN);
                }
            }
        }

        let mut scored_downloads = Vec::with_capacity(eligible_downloads.len());
        for dl in eligible_downloads {
            let active_peers = self
                .state
                .peer_session
                .active_peer_count_for_hash(&dl.ed2k_hash)
                .await;
            let source_count = {
                let dl_guard = self.state.downloads.lock().await;
                dl_guard
                    .list_peer_sources(&dl.ed2k_hash)
                    .map(|v| v.len())
                    .unwrap_or(0)
            };

            let (is_in_cooldown, stall_duration) = self
                .download_trackers
                .get(&dl.id)
                .map(|t| {
                    let in_cd = t.cooldown_until.map(|u| now_instant < u).unwrap_or(false);
                    let stall = if dl.download_rate_bps == 0 && t.last_received_bytes == dl.received_bytes {
                        Some(now_instant.saturating_duration_since(t.last_progress_time))
                    } else {
                        None
                    };
                    (in_cd, stall)
                })
                .unwrap_or((false, None));

            let score = calculate_smart_download_score(
                &dl,
                active_peers,
                source_count,
                is_in_cooldown,
                stall_duration,
            );
            scored_downloads.push((score, dl, active_peers));
        }

        // Sort descending by smart score; FIFO tie-breaker by created_at_unix_ms ascending
        scored_downloads.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.created_at_unix_ms.cmp(&b.1.created_at_unix_ms))
        });

        let (active_candidates, queued_candidates) =
            if scored_downloads.len() > max_concurrent_downloads {
                scored_downloads.split_at(max_concurrent_downloads)
            } else {
                (scored_downloads.as_slice(), [].as_slice())
            };

        // Degradar candidatos en cola: marcar estado como En cola (los pares se mantienen conectados para preservar la posición en cola)
        for (_score, dl, _active_peers) in queued_candidates {
            if dl.status.to_lowercase() != "queued" {
                let mut dl_guard = self.state.downloads.lock().await;
                if let Ok(updated_snap) = dl_guard.update_status(&dl.id, DownloadStatus::Queued) {
                    self.state.broadcast_download_update(updated_snap);
                }
            }
        }

        // Descargas activas que buscarán, marcarán y descargarán activamente en este ciclo:
        let active_downloads: Vec<DownloadSnapshot> = active_candidates
            .iter()
            .map(|(_, dl, _)| dl.clone())
            .collect();

        // Actualizar activated_at para descargas en transición de en cola a activa
        for dl in &active_downloads {
            if dl.status.to_lowercase() == "queued" {
                if let Some(t) = self.download_trackers.get_mut(&dl.id) {
                    t.activated_at = now_instant;
                    t.last_progress_time = now_instant;
                    t.last_received_bytes = dl.received_bytes;
                }
            }
        }

        let now = Instant::now();
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        // Estado A4AF por archivo (`ProcessA4AFClients` de eMule): peso de
        // prioridad y pares activos — ya no se calcula ninguna cuota de asignación,
        // cada archivo conecta sus fuentes hasta su límite.
        // `source_count` alimenta `UpdateAutoDownPriority` (prioridad «auto»).
        let mut a4af_states = Vec::with_capacity(active_downloads.len());
        for dl in &active_downloads {
            if dl.ed2k_hash.is_empty() {
                continue;
            }
            let source_count = {
                let dl_guard = self.state.downloads.lock().await;
                dl_guard
                    .list_peer_sources(&dl.ed2k_hash)
                    .map(|v| v.len())
                    .unwrap_or(0)
            };
            let active_peers = self
                .state
                .peer_session
                .active_peer_count_for_hash(&dl.ed2k_hash)
                .await;
            a4af_states.push(A4afFileState::new(
                dl.ed2k_hash.clone(),
                &dl.priority,
                source_count,
                active_peers,
            ));
        }

        let limits = {
            let dl_guard = self.state.downloads.lock().await;
            let max_g = dl_guard
                .get_setting("max_global_download_peers")
                .ok()
                .flatten()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(self.state.config.max_global_download_peers);
            let max_f = dl_guard
                .get_setting("max_peers_per_download")
                .ok()
                .flatten()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(self.state.config.max_peers_per_download);
            let min_f = dl_guard
                .get_setting("min_peers_per_download")
                .ok()
                .flatten()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(self.state.config.min_peers_per_download);
            SwarmLimits {
                max_global_peers: max_g,
                max_peers_per_file: max_f,
                min_peers_per_file: min_f,
            }
        };

        // `CListenSocket::TooManySockets` de eMule: el límite global de
        // conexiones acota los nuevos intentos de este ciclo, sea cual
        // sea el archivo — primero en llegar, primero en ser atendido.
        let mut connect_budget = limits.max_global_peers.saturating_sub(
            self.state
                .peer_session
                .swarm_snapshot(None)
                .await
                .active_peer_count,
        );

        // Límites de fuentes por archivo (FT_MAXSOURCES en `.part.met` /
        // `CPartFile::GetMaxSources` de eMule): cuando están presentes, el límite por archivo
        // anula el `max_peers_per_file` global para decisiones de conexión, nueva consulta
        // y umbral de purga.
        let file_max_sources: std::collections::HashMap<String, usize> = active_downloads
            .iter()
            .filter_map(|d| {
                if d.ed2k_hash.is_empty() {
                    return None;
                }
                let cap = d
                    .max_sources
                    .map(|v| v as usize)
                    .unwrap_or(limits.max_peers_per_file);
                Some((d.ed2k_hash.clone(), cap))
            })
            .collect();

        // A4AF ("Ask For Another File"): una descarga desatendida puede reclamar una
        // dirección de par actualmente conectada a un archivo de menor prioridad que
        // ya se encuentre por encima de su recuento mínimo de pares. eMule multiplexa archivos por
        // conexión; nuestras sesiones son monoarchivo, por lo que liberamos la sesión
        // y dejamos que el archivo desatendido marque la dirección en el siguiente ciclo.
        // Pasada de eventos A4AF en cada ciclo: fuentes que acaban de reportar NNP son
        // candidatas para intercambio inmediato a otros archivos (eMule dispara
        // `SwapToAnotherFile` ante NNP/FNF, no solo en la pasada periódica).
        self.a4af_event_pass(&active_downloads, now_ms).await;

        // Full A4AF rebalance at eMule's `ProcessA4AFClients` cadence (~8 min).
        if self.last_a4af_full.elapsed() >= super::types::A4AF_FULL_INTERVAL {
            self.last_a4af_full = Instant::now();
            self.a4af_rebalance(&a4af_states, &limits, now_ms).await;
        }

        // Mantenimiento de conexión de clientes UDP — `CPartFile::Process`
        // de eMule (`PartFile.cpp:2291-2294`): `OP_REASKFILEPING` se envía una vez
        // por ciclo, dentro de la ventana de `(1 s, 2 min)` anterior a la repetición
        // TCP de la sesión en cola (`next_tcp_reask_unix_ms`), únicamente cuando la
        // conexión tiene ≥ 20 min y no estamos tras cortafuegos. La deduplicación
        // a nivel de servicio (`UDP_REASK_INTERVAL`, mapa pending-ack)
        // permanece como respaldo contra envíos repetidos dentro de la ventana.
        let is_firewalled = self.is_firewalled().await;
        for target in self.state.peer_session.queued_udp_targets().await {
            if should_send_udp_reask(&target, now_ms, is_firewalled) {
                self.state.client_udp.send_reask_file_ping(&target, 0).await;
            }
        }

        // Ajustes efectivos leídos una vez por ciclo (persistidos en base de datos —
        // Preferencias de eMule → Seguridad): los valores de `config.*` congelados al
        // arrancar no reflejan los cambios aplicados mediante `settings.set`.
        let (obf_enabled, obf_required) = self.state.effective_obfuscation().await;
        let nickname = self.state.current_nickname().await;
        let mut global_connected_this_tick = 0usize;

        for dl in active_downloads {
            let file_info = {
                let dl_guard = self.state.downloads.lock().await;
                dl_guard.get_download_file_info(&dl.id)
            };

            let (_temp_path, _target_path, hash, size) = match file_info {
                Some(info) => info,
                None => continue,
            };

            let hash_hex = hex::encode(hash.0);

            let mut candidate_sources = {
                let dl_guard = self.state.downloads.lock().await;
                dl_guard.list_peer_sources(&hash_hex).unwrap_or_default()
            };

            {
                let score_tracker = self.state.peer_session.score_tracker();
                let tracker = score_tracker.lock().unwrap_or_else(|p| p.into_inner());
                tracker.sort_candidates(&mut candidate_sources);
            }

            // `CPartFile::Process` + `CDownloadQueue::Process` de eMule
            // (Opcodes.h): cada canal de descubrimiento tiene su propia cadencia
            // y su propio umbral — sin ventana adaptativa.
            //   TCP OP_GETSOURCES      : SERVERREASKTIME (15 min), si
            //       fuentes < GetMaxSourcePerFileSoft (90 % del límite, ≤750)
            //   UDP OP_GLOBGETSOURCES2 : UDPSERVERREASKTIME (30 min), si
            //       fuentes < GetMaxSourcePerFileUDP (75 % del límite, ≤50)
            //   Kad search_sources     : KADEMLIAREASKTIME (1 h) *
            //       m_TotalSearchesKad (≤7), mismo umbral que el UDP, más
            //       KADEMLIATOTALFILE búsquedas simultáneas y
            //       KADEMLIAASKTIME entre dos inicios.
            let source_count = candidate_sources.len();
            let file_max = file_max_sources
                .get(&hash_hex)
                .copied()
                .unwrap_or(limits.max_peers_per_file);

            let tcp_due = self
                .last_source_query_tcp
                .get(&hash_hex)
                .is_none_or(|t| now.duration_since(*t) >= SERVER_REASK_INTERVAL);
            if tcp_due
                && source_count < max_source_per_file_soft(file_max)
                && self.state.server_session.snapshot().await.status
                    == ServerConnectionStatus::Connected
            {
                self.last_source_query_tcp.insert(hash_hex.clone(), now);
                let req = GetSourcesRequest::with_obfuscation(hash.0, size, obf_enabled);
                if let Err(e) = self.state.server_session.get_sources(req).await {
                    tracing::debug!("Swarm coordinator get_sources failed for {hash_hex}: {e}");
                } else {
                    tracing::info!(
                        "Swarm coordinator requested sources from eD2K server for file {hash_hex} (obfuscated={})",
                        obf_enabled
                    );
                }
            }

            let udp_due = self
                .last_source_query_udp
                .get(&hash_hex)
                .is_none_or(|t| now.duration_since(*t) >= UDP_SERVER_REASK_INTERVAL);
            if udp_due && source_count < max_source_per_file_udp(file_max) {
                // Consulta de fuentes por UDP (OP_GLOBGETSOURCES2) hacia servidores — su puerto UDP
                // es TCP+4 (convención de eMule). En el eMule oficial (Process() en DownloadQueue.cpp),
                // esta consulta se envía al servidor conectado Y a un lote round-robin de
                // servidores conocidos válidos (hasta 8) para descubrir fuentes registradas en distintos
                // servidores eD2K. Envío sin espera; las respuestas llegan como OP_GLOBFOUNDSOURCES.
                self.last_source_query_udp.insert(hash_hex.clone(), now);

                let connected_udp_addr = self
                    .state
                    .server_session
                    .snapshot()
                    .await
                    .server_address
                    .and_then(|addr_str| addr_str.parse::<SocketAddr>().ok())
                    .map(|mut a| {
                        a.set_port(a.port().saturating_add(4));
                        a
                    });

                if let Some(udp_addr) = connected_udp_addr {
                    self.state
                        .server_udp
                        .send_glob_get_sources(udp_addr, &hash.0, size)
                        .await;
                }

                // Consulta UDP multiservidor a través de servidores conocidos permitidos desde BD
                let other_servers: Vec<SocketAddr> = {
                    let dl_guard = self.state.downloads.lock().await;
                    dl_guard
                        .list_servers()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|s| {
                            s.filter_status == mule_core::model::ServerFilterStatus::Allowed
                                && s.failed_count < 3
                        })
                        .filter_map(|s| s.address.parse::<SocketAddr>().ok())
                        .map(|mut addr| {
                            addr.set_port(addr.port().saturating_add(4));
                            addr
                        })
                        .filter(|addr| connected_udp_addr.as_ref() != Some(addr))
                        .take(8)
                        .collect()
                };

                for srv_udp in other_servers {
                    self.state
                        .server_udp
                        .send_glob_get_sources(srv_udp, &hash.0, size)
                        .await;
                }
            }

            let kad_due = self
                .kad_source_search
                .get(&hash_hex)
                .is_none_or(|s| now >= s.next_reask);
            if kad_due
                && source_count < max_source_per_file_udp(file_max)
                && self.state.network_policy.profile().allows_udp_kad()
                && self.state.kad.in_flight_file_searches().await < KAD_MAX_FILE_SEARCHES
                && now.duration_since(self.last_kad_file_request) >= KADEMLIA_ASK_INTERVAL
            {
                let total_searches = self
                    .kad_source_search
                    .get(&hash_hex)
                    .map(|s| (s.total_searches + 1).min(KAD_MAX_TOTAL_SEARCHES))
                    .unwrap_or(1);
                let kad_reask_interval = if source_count < 40 {
                    // Para archivos con menos fuentes conocidas, buscar en Kad agresivamente
                    // (cada 3 a 10 min máx) para poblar el enjambre rápidamente.
                    Duration::from_secs(3 * 60)
                        .saturating_mul(total_searches)
                        .min(Duration::from_secs(10 * 60))
                } else {
                    KADEMLIA_REASK_INTERVAL * total_searches
                };
                self.kad_source_search.insert(
                    hash_hex.clone(),
                    KadSourceSearchState {
                        next_reask: now + kad_reask_interval,
                        total_searches,
                    },
                );
                self.last_kad_file_request = now;

                let kad = Arc::clone(&self.state.kad);
                let h = hash_hex.clone();
                tracing::info!(
                    "Swarm coordinator triggering Kad search_sources for file {hash_hex} (known sources in DB: {source_count}, search #{total_searches})",
                );
                tokio::spawn(async move {
                    let _ = kad.search_sources(&h, size).await;
                });
            }

            let (client_id, server_ip, server_port, server_is_high_id) = {
                let s_snap = self.state.server_session.snapshot().await;
                if s_snap.status == ServerConnectionStatus::Connected {
                    let cid = s_snap.client_id.unwrap_or(0);
                    if cid == 0 {
                        tracing::debug!(
                            "Skipping peer connections for {hash_hex}: server connected but client_id not yet assigned"
                        );
                        continue;
                    }
                    let (s_ip, s_port) = if let Some(ref addr_str) = s_snap.server_address {
                        if let Ok(sock_addr) = addr_str.parse::<std::net::SocketAddr>() {
                            match sock_addr.ip() {
                                std::net::IpAddr::V4(v4) => {
                                    (u32::from_le_bytes(v4.octets()), sock_addr.port())
                                }
                                std::net::IpAddr::V6(_) => (0u32, sock_addr.port()),
                            }
                        } else {
                            (0u32, 0u16)
                        }
                    } else {
                        (0u32, 0u16)
                    };
                    (cid, s_ip, s_port, s_snap.is_high_id)
                } else {
                    // eMule funciona en modo solo Kad: sin servidor el hello
                    // anuncia client_id = 0 (misma convención que en la ruta
                    // manual `sources.connect`). `server_is_high_id=false`
                    // filtra de oficio las devoluciones de llamada del servidor y las fuentes
                    // LowID, inalcanzables sin servidor.
                    (0u32, 0u32, 0u16, false)
                }
            };

            // `CemuleApp::CanDoCallback` de eMule (Emule.cpp:1067):
            // la alcanzabilidad de fuentes LowID depende de nuestro propio
            // estado — HighID en servidor O Kad conectado sin cortafuegos — y,
            // cuando somos LowID, del servidor en el que reside la fuente
            // (`IsLocalServer`). `kad_open` = `CKademlia::IsConnected() &&
            // !IsFirewalled()`: socket UDP activo + prueba de cortafuegos positiva.
            let cb_ctx = {
                let udp_snap = self.state.kad.udp().get_snapshot().await;
                let fw_status = {
                    let tracker = self.state.kad.udp().firewall_tracker();
                    let fw = tracker.lock().await;
                    fw.status
                };
                super::lowid_callback::CallbackCtx {
                    server_connected: client_id != 0,
                    server_high_id: server_is_high_id,
                    our_server_ip: server_ip,
                    our_server_port: server_port,
                    kad_open: udp_snap.is_active
                        && fw_status == crate::services::kad_udp::types::UdpFirewallStatus::Open,
                }
            };

            // `CanAddSource` de eMule rechaza fuentes `lowid:` mientras
            // `IsFirewalled()` (ni HighID eD2K ni Kad abierto) — la bandera se
            // propaga al embudo de ingesta `upsert_peer_sources` para todos
            // los canales de descubrimiento (servidor TCP/UDP, SX, entrante).
            {
                let mut dl_guard = self.state.downloads.lock().await;
                dl_guard.set_lowid_source_acceptance(cb_ctx.server_high_id || cb_ctx.kad_open);
            }

            // `CPartFile::Process` de eMule: cerca del límite de fuentes por archivo
            // (>= 80%), purgar periódicamente fuentes que no puedan servirnos —
            // LowIDs inalcanzables, marcadas como NNP y fuentes fallidas repetidamente.
            if self
                .last_source_purge
                .get(&hash_hex)
                .is_none_or(|t| t.elapsed() >= super::types::SOURCE_PURGE_INTERVAL)
            {
                self.last_source_purge.insert(hash_hex.clone(), now);
                self.purge_excess_sources(&hash_hex, &candidate_sources, file_max, &cb_ctx, now_ms)
                    .await;
            }

            let swarm_snap = self
                .state
                .peer_session
                .swarm_snapshot(Some(&hash_hex))
                .await;
            let active_count = swarm_snap.active_peer_count;

            // eMule: sin cuota por archivo — cada archivo conecta
            // sus fuentes viables hasta su límite (`GetMaxSources` /
            // `maxsourceperfile`). Solo el límite global de conexiones
            // (`connect_budget`, `maxconnections` de eMule), el límite global
            // por ciclo y el ritmo por archivo acotan los nuevos intentos.
            let global_remaining = MAX_GLOBAL_CONNECT_PER_TICK.saturating_sub(global_connected_this_tick);
            if global_remaining == 0 {
                break;
            }

            let needed = file_max
                .saturating_sub(active_count)
                .min(connect_budget)
                .min(MAX_CONNECT_PER_TICK)
                .min(global_remaining);
            if needed == 0 {
                continue;
            }

            let mut connected_this_tick = 0;
            let mut skipped_cooldown = 0usize;
            let mut skipped_connected = 0usize;
            let total_candidates = candidate_sources.len();
            for src in candidate_sources {
                if connected_this_tick >= needed {
                    break;
                }

                match src.status {
                    PeerSourceStatus::Candidate
                    | PeerSourceStatus::Tried
                    | PeerSourceStatus::Allowed
                    | PeerSourceStatus::Failed
                    | PeerSourceStatus::Connected => {}
                    _ => continue,
                }

                if let Some(cooldown_until) = src.cooldown_until_unix_ms {
                    if cooldown_until > now_ms {
                        skipped_cooldown += 1;
                        continue;
                    }
                }

                let already_connected = swarm_snap
                    .peers
                    .iter()
                    .any(|p| p.peer_address.as_deref() == Some(&src.address));
                if already_connected {
                    skipped_connected += 1;
                    continue;
                }

                // `AddSources` de eMule: los clientes baneados (corrupción,
                // `IsBannedClient`) nunca se vuelven a marcar — la fuente se
                // marca como `Rejected` para no volver como candidata.
                let source_banned = {
                    let dl_guard = self.state.downloads.lock().await;
                    dl_guard.is_source_banned(&src.address)
                };
                if source_banned {
                    // El baneo de IP es temporal (2 h en memoria) — saltamos este ciclo
                    // sin condenar definitivamente la fuente en la base de datos.
                    continue;
                }

                // Fuente LowID: inalcanzable directamente — solicitar al servidor
                // conectado que retransmita una devolución de llamada (`OP_CALLBACKREQUEST` de eMule).
                // El objetivo se conecta entonces entrante; el oyente entrante
                // la asocia por client_id y la adopta como sesión de descarga.
                if let Some(lowid_str) = src.address.strip_prefix("lowid:") {
                    let Ok(lowid) = lowid_str.parse::<u32>() else {
                        continue;
                    };
                    // `CemuleApp::CanDoCallback` de eMule — si ni nuestro ID
                    // eD2K ni Kad permiten la reconexión, la fuente
                    // permanece aparcada (`DS_LOWTOLOWIP` de eMule, reanudación
                    // posible en cuanto nuestro estado cambie).
                    if !super::lowid_callback::can_do_callback(&src, &cb_ctx) {
                        continue;
                    }
                    // `CanAddSource` de eMule: nuestro propio LowID repercutido por
                    // el servidor conectado no debe llamarse (los LowIDs
                    // solo son únicos por servidor — limitado a fuentes
                    // de origen servidor como en eMule).
                    if lowid == client_id && src.origin == PeerSourceOrigin::Server {
                        continue;
                    }
                    // `OP_CALLBACKREQUEST` solo alcanzará a la LowID si
                    // reside en nuestro servidor (`IsLocalServer`) — ninguna otra
                    // opción de eMule (devolución directa, compañero) la portan
                    // las fuentes `lowid:`. Sin opción, eMule elimina la
                    // fuente (`Disconnected("No Callback Option")`).
                    if !super::lowid_callback::has_server_callback_option(&src, &cb_ctx) {
                        let mut dl_guard = self.state.downloads.lock().await;
                        let _ = dl_guard.update_peer_source_status(
                            &hash_hex,
                            &src.address,
                            PeerSourceStatus::Tried,
                            Some("LowID on a foreign server — callback pending server match"),
                            Some(now_ms + 15 * 60 * 1000),
                        );
                        continue;
                    }
                    if self.state.peer_session.has_pending_callback(lowid) {
                        continue;
                    }
                    match self.state.server_session.request_callback(lowid).await {
                        Ok(()) => {
                            tracing::info!(
                                "Requested server callback to LowID source {lowid} for {file_hash}",
                                file_hash = hash_hex
                            );
                            self.state.peer_session.register_pending_callback(
                                lowid,
                                hash_hex.clone(),
                                5 * 60 * 1000,
                            );
                            let mut dl_guard = self.state.downloads.lock().await;
                            let _ = dl_guard.update_peer_source_status(
                                &hash_hex,
                                &src.address,
                                PeerSourceStatus::Tried,
                                None,
                                Some(now_ms + 5 * 60 * 1000),
                            );
                            connected_this_tick += 1;
                        }
                        Err(e) => {
                            tracing::debug!("Callback request for LowID {lowid} failed: {e}");
                        }
                    }
                    continue;
                }

                // Fuente LowID de Kad: alcanzable únicamente a través de su compañero
                // publicado. Enviar KADEMLIA_CALLBACK_REQ al nodo compañero; el
                // cliente LowID se conecta entonces entrante a nuestro puerto TCP y es
                // adoptado por IP remota (pending_kad_callbacks).
                if let Some(ip_str) = src.address.strip_prefix("kadcb:") {
                    let Ok(src_ip) = ip_str.parse::<std::net::IpAddr>() else {
                        continue;
                    };
                    // El par intentará conectarse a nuestro puerto TCP:
                    // misma condición `CanDoCallback` que las fuentes
                    // `lowid:` (HighID en servidor o Kad abierto).
                    if !super::lowid_callback::can_do_callback(&src, &cb_ctx) {
                        continue;
                    }
                    if self.state.peer_session.has_pending_kad_callback(src_ip) {
                        continue;
                    }

                    // `AskForDownload` de eMule (`DownloadClient.cpp:185-206`):
                    // fuente LowID ya solicitada (entrada `queued_peers` =
                    // `GetLastAskedTime() > 0`) y presente en nuestra cola
                    // de subida (`US_ONUPLOADQUEUE`) → `m_bReaskPending`: la
                    // repetición de petición se pospone un ciclo para dejar que el par
                    // marque hacia nosotros en su propia repetición de subida. La
                    // bandera se consume en su conexión entrante.
                    if self
                        .state
                        .inbound_listener
                        .upload_manager()
                        .waiting_rank_by_ip(&src_ip)
                        .await
                        .is_some()
                    {
                        let next_cycle = now_ms
                            + crate::services::peer_session::status_query::FILE_REASK_INTERVAL
                                .as_millis() as i64;
                        if self
                            .state
                            .peer_session
                            .defer_reask_for_queued_peer(src_ip, &hash_hex, next_cycle)
                            .await
                        {
                            let mut dl_guard = self.state.downloads.lock().await;
                            let _ = dl_guard.update_peer_source_status(
                                &hash_hex,
                                &src.address,
                                PeerSourceStatus::Tried,
                                Some("reask deferred — peer waits in our upload queue"),
                                Some(next_cycle),
                            );
                            continue;
                        }
                    }

                    // Paso 4 de `TryToConnect` de eMule (`BaseClient.cpp:1403`) —
                    // la devolución UDP directa precede al relevo por compañero: la
                    // fuente está tras cortafuegos TCP pero abierta en UDP y ha
                    // anunciado el bit en su byte de opciones publicado
                    // (`connect-options & 0x08`, OtherFunctions.cpp:4049).
                    if src.crypt_options
                        & mule_protocol::ed2k::client_udp::CONNECT_OPT_DIRECT_CALLBACK
                        != 0
                        && src.udp_port != 0
                    {
                        let (obf_enabled, obf_required) = self.state.effective_obfuscation().await;
                        // `GetMyConnectOptions(true, false)`: solo banderas de cifrado,
                        // el bit de devolución directa permanece en 0.
                        let conn_opts = (obf_enabled as u8)
                            | ((obf_enabled as u8) << 1)
                            | ((obf_required as u8) << 2);
                        let our_hash = self.state.identity.lock().await.user_hash();
                        let target = SocketAddr::new(src_ip, src.udp_port);
                        if self
                            .state
                            .client_udp
                            .send_direct_callback_req(
                                target,
                                self.state.ed2k_port(),
                                &our_hash,
                                conn_opts,
                            )
                            .await
                        {
                            tracing::info!(
                                "Sent OP_DIRECTCALLBACKREQ to LowID source {peer}:{port} for {hash_hex}",
                                peer = log_peer_ip(src_ip),
                                port = src.udp_port,
                            );
                            self.state.peer_session.register_pending_kad_callback(
                                src_ip,
                                hash_hex.clone(),
                                5 * 60 * 1000,
                            );
                            let mut dl_guard = self.state.downloads.lock().await;
                            let _ = dl_guard.update_peer_source_status(
                                &hash_hex,
                                &src.address,
                                PeerSourceStatus::Tried,
                                None,
                                Some(now_ms + 5 * 60 * 1000),
                            );
                            connected_this_tick += 1;
                            continue;
                        }
                        // Sin socket UDP → la opción no existe,
                        // se recurre al relevo por compañero.
                    }

                    let Some(buddy_bytes) = src.buddy_id_bytes() else {
                        continue;
                    };
                    let buddy_id = mule_core::kad::KadId(buddy_bytes);
                    let Some(buddy) = self.state.kad.find_contact(&buddy_id).await else {
                        tracing::debug!(
                            "Kad buddy {buddy_id:?} for LowID source {peer} not in routing table",
                            peer = log_peer_ip(src_ip)
                        );
                        continue;
                    };
                    let Ok(file_id_bytes) = hex::decode(&hash_hex) else {
                        continue;
                    };
                    let mut file_id = [0u8; 16];
                    file_id.copy_from_slice(&file_id_bytes);
                    let buddy_addr =
                        SocketAddr::new(std::net::IpAddr::V4(buddy.ip), buddy.udp_port);
                    match self
                        .state
                        .kad
                        .udp()
                        .send_callback_req(
                            buddy_addr,
                            &buddy_bytes,
                            &file_id,
                            self.state.ed2k_port(),
                        )
                        .await
                    {
                        Ok(()) => {
                            tracing::info!(
                                "Requested Kad buddy callback to LowID source {peer} via {buddy} for {hash_hex}"
                            , peer = log_peer_ip(src_ip), buddy = log_peer_addr(buddy_addr));
                            self.state.peer_session.register_pending_kad_callback(
                                src_ip,
                                hash_hex.clone(),
                                5 * 60 * 1000,
                            );
                            let mut dl_guard = self.state.downloads.lock().await;
                            let _ = dl_guard.update_peer_source_status(
                                &hash_hex,
                                &src.address,
                                PeerSourceStatus::Tried,
                                None,
                                Some(now_ms + 5 * 60 * 1000),
                            );
                            connected_this_tick += 1;
                        }
                        Err(e) => {
                            tracing::debug!(
                                "Kad buddy callback request for {peer} failed: {e}",
                                peer = log_peer_ip(src_ip)
                            );
                        }
                    }
                    continue;
                }

                let peer_addr: SocketAddr = match src.address.parse() {
                    Ok(a) => a,
                    Err(_) => continue,
                };

                // `CanAddSource` de eMule: nunca marcar nuestro propio
                // extremo — cuando el servidor nos asignó un HighID, nuestro
                // client_id ES nuestra IP pública; una fuente `ip:port`
                // idéntica es nuestro propio anuncio repercutido.
                if server_is_high_id {
                    if let std::net::IpAddr::V4(v4) = peer_addr.ip() {
                        if u32::from_le_bytes(v4.octets()) == client_id
                            && peer_addr.port() == self.state.ed2k_port()
                        {
                            continue;
                        }
                    }
                }

                let permit = match self.state.network_policy.request_tcp_connect(
                    peer_addr,
                    mule_network_policy::ConnectionPurpose::Ed2kPeer,
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        // Rechazo permanente (dirección inválida / ipfilter): la
                        // fuente nunca será marcable — marcarla como Rejected
                        // en lugar de reintentarla en cada ciclo (DeadSourceList
                        // de eMule). Los rechazos transitorios (límite de tasa,
                        // presupuesto) conservan el retroceso normal.
                        if matches!(
                            e,
                            mule_network_policy::PolicyError::AddressViolation(_)
                                | mule_network_policy::PolicyError::IpFiltered(..)
                        ) {
                            tracing::info!(
                                "[SOURCE_SCHED] Source {} for {hash_hex} permanently rejected: {e}",
                                src.address
                            );
                            let mut dl_guard = self.state.downloads.lock().await;
                            let _ = dl_guard.update_peer_source_status(
                                &hash_hex,
                                &src.address,
                                PeerSourceStatus::Rejected,
                                Some(&e.to_string()),
                                None,
                            );
                        } else {
                            tracing::debug!(
                                "Permit denied for peer {peer}: {e}",
                                peer = log_peer_addr(peer_addr)
                            );
                        }
                        continue;
                    }
                };

                let user_hash = self.state.identity.lock().await.user_hash();
                let sx_env = self.state.sx_env.clone();
                let dl_ref = Arc::clone(&self.state.downloads);
                let file_hash_str = hash_hex.clone();
                let ps = self.state.peer_session.clone();
                let client_port = self.state.ed2k_port();
                let kad_port = self.state.kad_port();

                let target_user_hash = src.user_hash_bytes();
                let peer_crypt_options = src.crypt_options;
                let obfu_pad_max = self.state.config.obfuscation_padding_max;
                let nickname = nickname.clone();

                tracing::debug!(
                    "Swarm coordinator initiating peer connection to {peer} for {file_hash_str} (max={file_max}, active={active_count}, obfu_target_hash={:?}, crypt_opts=0x{:02x})",
                    target_user_hash.is_some(),
                    peer_crypt_options
                , peer = log_peer_addr(peer_addr));
                tokio::spawn(async move {
                    if let Err(e) = ps
                        .connect(
                            &file_hash_str,
                            peer_addr,
                            permit,
                            client_port,
                            nickname,
                            user_hash,
                            dl_ref,
                            client_id,
                            server_ip,
                            server_port,
                            kad_port,
                            target_user_hash,
                            peer_crypt_options,
                            obf_enabled,
                            obf_required,
                            obfu_pad_max,
                            sx_env,
                        )
                        .await
                    {
                        tracing::debug!(
                            "Peer connection to {peer} failed: {e}",
                            peer = log_peer_addr(peer_addr)
                        );
                    }
                });

                connected_this_tick += 1;
            }

            if connected_this_tick > 0 {
                tracing::debug!(
                    "[SOURCE_SCHED] File {hash_hex}: dialed {connected_this_tick} new sources this tick \
                     (candidates={total_candidates}, slots_needed={needed}, active={active_count}, \
                     skipped: cooldown={skipped_cooldown}, already_connected={skipped_connected})"
                );
            } else if total_candidates > 0 {
                tracing::debug!(
                    "[SOURCE_SCHED] File {hash_hex}: no source dialed this tick \
                     (candidates={total_candidates}, slots_needed={needed}, active={active_count}, \
                     skipped: cooldown={skipped_cooldown}, already_connected={skipped_connected})"
                );
            }

            // Cada intento de este archivo consume el presupuesto global de
            // conexiones (`maxconnections` de eMule — `TooManySockets`).
            connect_budget = connect_budget.saturating_sub(connected_this_tick);
            global_connected_this_tick += connected_this_tick;

            if dl.status.to_lowercase() == "queued" {
                let new_status = if active_count > 0 || connected_this_tick > 0 {
                    DownloadStatus::Downloading
                } else {
                    DownloadStatus::WaitingForSources
                };
                let mut dl_guard = self.state.downloads.lock().await;
                if let Ok(updated_snap) = dl_guard.update_status(&dl.id, new_status) {
                    self.state.broadcast_download_update(updated_snap);
                }
            } else if (active_count > 0 || connected_this_tick > 0)
                && (dl.status.to_lowercase() == "waitingforsources"
                    || dl.status.to_lowercase() == "waiting_for_sources")
            {
                let mut dl_guard = self.state.downloads.lock().await;
                if let Ok(updated_snap) =
                    dl_guard.update_status(&dl.id, DownloadStatus::Downloading)
                {
                    self.state.broadcast_download_update(updated_snap);
                }
            }
        }
    }

    /// Conmutación por error automática de servidor. Mientras está desconectado, elige
    /// periódicamente un servidor conocido habilitado (excluyendo el que acaba de caer y
    /// cualquier dirección intentada dentro de `SERVER_FAIL_COOLDOWN`) y se reconecta.
    /// Acotado por `SERVER_RECONNECT_INTERVAL` entre intentos.
    async fn maybe_reconnect_server(&mut self) {
        use super::types::{SERVER_FAIL_COOLDOWN, SERVER_RECONNECT_INTERVAL};

        let snap = self.state.server_session.snapshot().await;
        match snap.status {
            ServerConnectionStatus::Connected => {
                self.last_connected_server = snap.server_address;
                return;
            }
            ServerConnectionStatus::Connecting | ServerConnectionStatus::Disconnecting => {
                return;
            }
            ServerConnectionStatus::Disconnected => {}
        }

        if let Some(last) = self.last_server_reconnect {
            if last.elapsed() < SERVER_RECONNECT_INTERVAL {
                return;
            }
        }

        // Elegir un servidor habilitado y no bloqueado no intentado recientemente; preferir
        // un servidor distinto al que acaba de caer.
        let candidates = {
            let dl_guard = self.state.downloads.lock().await;
            dl_guard
                .list_servers()
                .unwrap_or_default()
                .into_iter()
                .filter(|s| {
                    s.filter_status == mule_core::model::ServerFilterStatus::Allowed
                        && s.address.parse::<SocketAddr>().is_ok()
                        && self
                            .server_attempts
                            .get(&s.address)
                            .is_none_or(|t| t.elapsed() >= SERVER_FAIL_COOLDOWN)
                })
                .map(|s| (s.address, s.name, s.preference))
                .collect::<Vec<_>>()
        };
        if candidates.is_empty() {
            return;
        }

        let pick = candidates
            .iter()
            .filter(|(addr, _, _)| Some(addr) != self.last_connected_server.as_ref())
            .max_by_key(|(_, _, pref)| *pref)
            .or_else(|| candidates.iter().max_by_key(|(_, _, pref)| *pref));
        let Some((address, name, _)) = pick else {
            return;
        };

        let addr: SocketAddr = match address.parse() {
            Ok(a) => a,
            Err(_) => return,
        };
        let permit = match self
            .state
            .network_policy
            .request_tcp_connect(addr, mule_network_policy::ConnectionPurpose::Ed2kServer)
        {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!(
                    "Server failover: permit denied for {server}: {e}",
                    server = log_server_addr(addr)
                );
                return;
            }
        };

        let user_hash = self.state.identity.lock().await.user_hash();
        let ed2k_port = self.state.ed2k_port();
        let nickname = self.state.current_nickname().await;
        tracing::info!(
            "Server failover: connecting to {name} ({server})",
            server = log_server_addr(addr)
        );
        self.last_server_reconnect = Some(Instant::now());
        self.server_attempts.insert(address.clone(), Instant::now());
        let _ = self
            .state
            .server_session
            .connect(
                addr,
                Some(name.clone()),
                permit,
                10000,
                15000,
                ed2k_port,
                nickname,
                user_hash,
            )
            .await;
    }

    /// Emite un latido periódico consolidado del sistema que resume el estado completo del daemon.
    pub(crate) async fn emit_heartbeat(&self) {
        // 1. Server info
        let srv_snap = self.state.server_session.snapshot().await;
        let srv_str = match srv_snap.status {
            ServerConnectionStatus::Connected => {
                let name = srv_snap.server_name.as_deref().unwrap_or("Server");
                let addr = srv_snap.server_address.as_deref().unwrap_or("");
                let client_id = srv_snap
                    .client_id
                    .map(|id| format!("{id}"))
                    .unwrap_or_else(|| "N/A".into());
                format!("Connected to {name} ({addr}, ID: {client_id})")
            }
            ServerConnectionStatus::Connecting => "Connecting...".to_string(),
            ServerConnectionStatus::Disconnected => "Disconnected".to_string(),
            _ => format!("{:?}", srv_snap.status),
        };

        // 2. Kad info
        let kad_snap = self.state.kad.get_status().await;
        let kad_str = if kad_snap.is_udp_active {
            format!(
                "Running ({} contacts, {}/{} pkts)",
                kad_snap.total_contacts, kad_snap.packets_sent, kad_snap.packets_received
            )
        } else {
            "Inactive".to_string()
        };

        // 3. Downloads info
        let (active_dl_count, other_dl_count, total_dl_rate, total_sources, dl_session_bytes, banned_count) = {
            let dl_guard = self.state.downloads.lock().await;
            let dls = dl_guard.list_downloads();
            let mut active = 0;
            let mut other = 0;
            let mut rate = 0u64;
            let mut sources = 0;
            for d in dls {
                if d.status.to_lowercase() == "downloading" {
                    active += 1;
                } else {
                    other += 1;
                }
                rate = rate.saturating_add(d.download_rate_bps);
                sources += d.source_count;
            }
            (
                active,
                other,
                rate,
                sources,
                dl_guard.session_downloaded_bytes(),
                dl_guard.banned_source_count(),
            )
        };
        let swarm_snap = self.state.peer_session.swarm_snapshot(None).await;
        let dl_str = format!(
            "{} active, {} other ({:.1} KB/s, {} peers connected, {} known sources, session {:.1} MB)",
            active_dl_count,
            other_dl_count,
            total_dl_rate as f64 / 1024.0,
            swarm_snap.active_peer_count,
            total_sources,
            dl_session_bytes as f64 / (1024.0 * 1024.0),
        );

        // 4. Upload info
        let listener_snap = self.state.inbound_listener.snapshot().await;
        let ul_session_bytes = self
            .state
            .inbound_listener
            .upload_manager()
            .total_uploaded_bytes();
        let ul_str = format!(
            "{} active ({:.1} KB/s, {} queued, session {:.1} MB, {} banned)",
            listener_snap.active_uploads,
            listener_snap.upload_rate_bps as f64 / 1024.0,
            listener_snap.queued_uploads,
            ul_session_bytes as f64 / (1024.0 * 1024.0),
            banned_count,
        );

        tracing::info!(
            "[SYSTEM_HEARTBEAT] Server: {} | Kad: {} | DL: {} | UL: {}",
            srv_str,
            kad_str,
            dl_str,
            ul_str
        );
    }

    /// Pasada de reequilibrio A4AF (semántica de intercambio de fuentes de `CPartFile` en eMule).
    ///
    /// Para cada descarga por debajo de `min_peers_per_file`, examinar sesiones activas de
    /// archivos de *menor prioridad* que se encuentren por encima de su mínimo: si la dirección
    /// del par conectado es una candidata viable para el archivo desatendido, liberar la
    /// sesión para que el archivo desatendido pueda marcarla en el siguiente ciclo.
    /// Como máximo un intercambio por ciclo mantiene la estabilidad del sistema.
    async fn a4af_rebalance(
        &mut self,
        states: &[A4afFileState],
        limits: &SwarmLimits,
        now_ms: i64,
    ) {
        for starved in states {
            if starved.current_active_peers >= limits.min_peers_per_file {
                continue;
            }

            // Direcciones candidatas para el archivo desatendido (únicamente fuentes viables).
            let wanted: std::collections::HashSet<String> = {
                let dl_guard = self.state.downloads.lock().await;
                dl_guard
                    .list_peer_sources(&starved.file_hash)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|s| match s.status {
                        PeerSourceStatus::Candidate
                        | PeerSourceStatus::Tried
                        | PeerSourceStatus::Allowed => true,
                        PeerSourceStatus::Failed => s
                            .cooldown_until_unix_ms
                            .map(|c| c <= now_ms)
                            .unwrap_or(true),
                        _ => false,
                    })
                    .map(|s| s.address.clone())
                    .collect()
            };
            if wanted.is_empty() {
                continue;
            }

            // Live sessions connected to other files.
            let swarm = self.state.peer_session.swarm_snapshot(None).await;
            let donors: std::collections::HashMap<&str, u32> = states
                .iter()
                .map(|d| (d.file_hash.as_str(), d.priority_weight))
                .collect();

            for peer in &swarm.peers {
                let Some(donor_hash) = peer.file_hash.as_deref() else {
                    continue;
                };
                let Some(peer_addr) = peer.peer_address.as_deref() else {
                    continue;
                };
                if donor_hash == starved.file_hash || !wanted.contains(peer_addr) {
                    continue;
                }
                let donor_weight = donors.get(donor_hash).copied().unwrap_or(2);
                // Solo sustraer de un donante de prioridad estrictamente menor que se mantenga
                // por encima de su recuento mínimo de pares.
                if donor_weight >= starved.priority_weight {
                    continue;
                }
                let donor_active = self
                    .state
                    .peer_session
                    .active_peer_count_for_hash(donor_hash)
                    .await;
                if donor_active <= limits.min_peers_per_file {
                    continue;
                }
                tracing::info!(
                    "[A4AF] Swapping source {} from file {} (prio={}) to starved file {} (prio={})",
                    log_addr_str(peer_addr),
                    donor_hash,
                    donor_weight,
                    starved.file_hash,
                    starved.priority_weight
                );
                let _ = self
                    .state
                    .peer_session
                    .disconnect_peer(Some(peer_addr))
                    .await;
                return; // One swap per tick.
            }
        }
    }

    /// Purga condicional de fuentes (temporizadores lentos de `CPartFile::Process` en eMule).
    ///
    /// Solo se ejecuta cuando el recuento de fuentes conocidas del archivo alcanza ~80% de su
    /// `maxSources` (`FT_MAXSOURCES` o el límite global por archivo). Purgadas:
    /// - fuentes `lowid:`/`kadcb:` a las que no podemos devolver la llamada en absoluto
    ///   (`!CanDoCallback` — `DS_LOWTOLOWIP` de eMule, purgadas al 80 %);
    /// - fuentes `kadcb:` sin ID de compañero (inalcanzables);
    /// - fuentes marcadas como "no needed parts" (NNP) — su máscara de partes no cubre
    ///   nada que aún necesitemos;
    /// - fuentes que fallaron repetidamente (fail_count >= 3);
    /// - sesiones marcadas como `remote_queue_full` (`OP_QUEUEFULL` recibido en el
    ///   re-ask UDP) — `DS_ONQUEUE && IsRemoteQueueFull()` de eMule: estas
    ///   fuentes se purgan con prioridad cuando la lista de fuentes alcanza el 80 % de
    ///   `GetMaxSources` (`PartFile.cpp:2285-2295`, cadencia de 1 min).
    ///
    /// La purga elimina como máximo `count - threshold + 1` filas por pasada —
    /// eMule desciende por debajo del umbral (`GetSourceCount() >= 4/5` → retira
    /// incluso en caso de igualdad), el estado final es `threshold - 1`.
    async fn purge_excess_sources(
        &mut self,
        file_hash: &str,
        sources: &[mule_core::model::PeerSource],
        file_max: usize,
        cb_ctx: &super::lowid_callback::CallbackCtx,
        now_ms: i64,
    ) {
        if file_max == 0 || sources.len() < file_max * 4 / 5 {
            return;
        }
        // eMule purga mientras `GetSourceCount() >= GetMaxSources()*4/5` —
        // el estado final es `umbral - 1` (a 8/10 retira 1 fuente, no 0).
        let budget = sources.len() - file_max * 4 / 5 + 1;

        // Sesiones activas marcadas como remote-queue-full — eMule mantiene la bandera
        // en el cliente (`m_bRemoteQueueFull`), nosotros en la instantánea de
        // sesión. Coincidencia por dirección exacta y luego por IP sola (las fuentes
        // `lowid:`/`kadcb:` tienen un peer_address ip:puerto diferente de su
        // dirección de fuente).
        let rqfs = self
            .state
            .peer_session
            .swarm_snapshot(Some(file_hash))
            .await;
        let mut rqf_addrs: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut rqf_ips: std::collections::HashSet<std::net::IpAddr> =
            std::collections::HashSet::new();
        for p in &rqfs.peers {
            if !p.remote_queue_full {
                continue;
            }
            if let Some(addr) = &p.peer_address {
                rqf_addrs.insert(addr.clone());
                if let Ok(sa) = addr.parse::<std::net::SocketAddr>() {
                    rqf_ips.insert(sa.ip());
                }
            }
        }
        let rqf_source = |s: &&mule_core::model::PeerSource| {
            rqf_addrs.contains(&s.address)
                || crate::services::downloads::source_addr_ip(&s.address)
                    .is_some_and(|ip| rqf_ips.contains(&ip))
        };

        let mut removed = 0usize;
        for s in sources {
            if removed >= budget {
                break;
            }
            let nnp = s
                .last_error
                .as_deref()
                .map(|e| e.to_lowercase().contains("no needed parts"))
                .unwrap_or(false);
            let unreachable_lowid = (s.address.starts_with("lowid:")
                || s.address.starts_with("kadcb:"))
                && (!super::lowid_callback::can_do_callback(s, cb_ctx)
                    || (s.address.starts_with("kadcb:") && s.buddy_id.is_none()));
            let repeatedly_failed = s.status == PeerSourceStatus::Failed && s.fail_count >= 3;
            let remote_queue_full = rqf_source(&s);
            if !(nnp || unreachable_lowid || repeatedly_failed || remote_queue_full) {
                continue;
            }
            // eMule purga también los DS_ONQUEUE conectados cuando están en
            // RemoteQueueFull — el estado Connected no los protege.
            if s.status == PeerSourceStatus::Connected && !remote_queue_full {
                continue;
            }
            let why = if remote_queue_full {
                "remote queue full"
            } else if nnp {
                "NNP"
            } else if unreachable_lowid {
                "unreachable LowID"
            } else {
                "repeated failures"
            };
            let mut dl_guard = self.state.downloads.lock().await;
            match dl_guard.delete_peer_source(file_hash, &s.address) {
                Ok(true) => {
                    removed += 1;
                    tracing::info!(
                        "[SOURCE_PURGE] {file_hash}: dropped {why} source {} (swarm over 80% of maxSources={file_max})",
                        s.address
                    );
                    // `RemoveSource` de eMule destruye el cliente de descarga —
                    // terminar la sesión activa asociada si existe
                    // (clave de sesión = peer_address real, no la dirección
                    // de fuente `lowid:`/`kadcb:`).
                    let session_addr = rqfs.peers.iter().find_map(|p| {
                        let pa = p.peer_address.as_ref()?;
                        let src_ip = crate::services::downloads::source_addr_ip(&s.address)?;
                        let pa_ip = crate::services::downloads::source_addr_ip(pa)?;
                        (p.remote_queue_full && (pa == &s.address || pa_ip == src_ip))
                            .then(|| pa.clone())
                    });
                    if let Some(addr) = session_addr {
                        let _ = self.state.peer_session.disconnect_peer(Some(&addr)).await;
                    }
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::debug!("[SOURCE_PURGE] delete {} failed: {e}", s.address)
                }
            }
        }
        if removed > 0 {
            tracing::info!(
                "[SOURCE_PURGE] {file_hash}: removed {removed} sources at {}% of maxSources ({}/{})",
                sources.len() * 100 / file_max.max(1),
                sources.len(),
                file_max
            );
        }
        let _ = now_ms;
    }

    /// A4AF dirigido por eventos (disparadores de intercambio NNP/FNF de `CUpDownClient` en eMule).
    ///
    /// Examina fuentes marcadas recientemente como "no needed parts" (NNP): la fuente es
    /// inútil para el archivo que servía pero puede contener partes para otra descarga
    /// activa. Cuando la misma dirección es una candidata viable de otro
    /// archivo, se limpia su enfriamiento para que el marcador la tome de inmediato
    /// (limitado a una dirección por archivo por ciclo).
    async fn a4af_event_pass(&mut self, downloads: &[DownloadSnapshot], now_ms: i64) {
        let hashes: Vec<String> = downloads
            .iter()
            .filter(|d| !d.ed2k_hash.is_empty())
            .map(|d| d.ed2k_hash.clone())
            .collect();
        if hashes.len() < 2 {
            return;
        }

        // Sources per file, loaded once.
        let mut per_file: std::collections::HashMap<String, Vec<mule_core::model::PeerSource>> =
            std::collections::HashMap::new();
        {
            let dl_guard = self.state.downloads.lock().await;
            for h in &hashes {
                per_file.insert(h.clone(), dl_guard.list_peer_sources(h).unwrap_or_default());
            }
        }

        // Direcciones que acaban de reportar NNP/FNF, indexadas por el archivo o archivos que
        // las marcaron. La bandera es por (archivo, dirección): una fuente que reportó
        // "no needed parts" para el archivo A nunca debe ver limpiado su enfriamiento
        // en el archivo A — solo en otros archivos donde pueda ser útil.
        let mut nnp_files_by_addr: std::collections::HashMap<
            String,
            std::collections::HashSet<String>,
        > = std::collections::HashMap::new();
        for (hash, srcs) in &per_file {
            for s in srcs {
                let flagged = s
                    .last_error
                    .as_deref()
                    .map(|e| {
                        let e = e.to_lowercase();
                        e.contains("no needed parts") || e.contains("file not found")
                    })
                    .unwrap_or(false);
                if flagged {
                    nnp_files_by_addr
                        .entry(s.address.clone())
                        .or_default()
                        .insert(hash.clone());
                }
            }
        }
        if nnp_files_by_addr.is_empty() {
            return;
        }

        // Limpiar el enfriamiento de la misma dirección en *otros* archivos para que el
        // planificador pueda marcarla de inmediato (`SwapToAnotherFile` de eMule).
        for (hash, srcs) in &per_file {
            for s in srcs {
                let flagging_files = match nnp_files_by_addr.get(&s.address) {
                    Some(f) => f,
                    None => continue,
                };
                if flagging_files.contains(hash) {
                    continue;
                }
                let viable = matches!(
                    s.status,
                    PeerSourceStatus::Candidate
                        | PeerSourceStatus::Tried
                        | PeerSourceStatus::Allowed
                ) || (s.status == PeerSourceStatus::Failed
                    && s.cooldown_until_unix_ms.is_some_and(|c| c <= now_ms));
                if !viable {
                    continue;
                }
                let cooling = s.cooldown_until_unix_ms.is_some_and(|c| c > now_ms);
                if !cooling {
                    continue;
                }
                let mut dl_guard = self.state.downloads.lock().await;
                if dl_guard
                    .reset_peer_source_cooldown(hash, &s.address)
                    .unwrap_or(false)
                {
                    tracing::info!(
                        "[A4AF] Event swap: source {} freed by another file, reactivating for {hash}",
                        s.address
                    );
                }
                break; // one event swap per file per tick
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
        let link = format!("ed2k://|file|f.bin|10000000|{hash_hex}|/");
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
    fn test_cb_ctx() -> super::super::lowid_callback::CallbackCtx {
        super::super::lowid_callback::CallbackCtx {
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
            protocol: None,
        };

        let active_score = super::calculate_smart_download_score(&stalled_dl, 1, 4, false, None);
        let queued_score = super::calculate_smart_download_score(&queued_dl, 0, 15, false, None);
        assert!(active_score > queued_score);

        let rotated_score = super::calculate_smart_download_score(&stalled_dl, 0, 4, true, None);
        assert!(rotated_score < queued_score);
        assert_eq!(rotated_score, -50_000.0);
    }
}

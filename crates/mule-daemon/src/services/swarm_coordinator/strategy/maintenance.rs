// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use super::*;

impl SwarmCoordinator {
    /// Conmutación por error automática de servidor. Mientras está desconectado, elige
    /// periódicamente un servidor conocido habilitado (excluyendo el que acaba de caer y
    /// cualquier dirección intentada dentro de `SERVER_FAIL_COOLDOWN`) y se reconecta.
    /// Acotado por `SERVER_RECONNECT_INTERVAL` entre intentos.
    pub(super) async fn maybe_reconnect_server(&mut self) {
        use super::super::types::{SERVER_FAIL_COOLDOWN, SERVER_RECONNECT_INTERVAL};

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
    pub(super) async fn a4af_rebalance(
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
    pub(super) async fn purge_excess_sources(
        &mut self,
        file_hash: &str,
        sources: &[mule_core::model::PeerSource],
        file_max: usize,
        cb_ctx: &super::super::lowid_callback::CallbackCtx,
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
                && (!super::super::lowid_callback::can_do_callback(s, cb_ctx)
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
    pub(super) async fn a4af_event_pass(&mut self, downloads: &[DownloadSnapshot], now_ms: i64) {
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

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use std::collections::VecDeque;
use std::sync::atomic::Ordering;

use crate::services::peer_session::log_addr_str;

use super::*;

impl UploadManager {
    /// Promociona a los clientes en espera mientras queden ranuras disponibles. El invocador debe mantener
    /// el cerrojo de `active_uploads` y pasarlo como argumento.
    pub(super) async fn promote_waiting(
        &self,
        active: &mut std::collections::HashMap<String, UploadSessionSnapshot>,
    ) {
        let mut q = self.queue.lock().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        // Purga de entradas inactivas — eMule `FindBestClientInQueue` retira
        // a los clientes cuya última repetición de petición (re-ask) date de más de
        // `MAX_PURGEQUEUETIME` (1 h) o cuyo archivo solicitado ya no
        // esté compartido (`!sharedfiles->GetFileByID`).
        q.retain(|e| {
            now.saturating_sub(e.last_request_unix_ms) <= MAX_PURGEQUEUE_MS
                && self.hash_still_shared(&e.file_hash)
        });
        // La puntuación evoluciona con la espera: volver a ordenar antes de elegir al
        // mejor candidato (eMule lee `GetScore` en vivo).
        self.sort_queue_by_score(&mut q, now);
        self.promote_locked(&mut q, active, now);
        self.active_count
            .store(Self::uploading_count(active), Ordering::SeqCst);
    }

    /// Cuerpo de `promote_waiting` sobre una cola ya bloqueada — compartido
    /// con `process_tick` que promociona tras cada desalojo de sesión.
    /// Devuelve `true` si al menos un cliente fue promocionado.
    pub(super) fn promote_locked(
        &self,
        q: &mut VecDeque<QueuedUpload>,
        active: &mut std::collections::HashMap<String, UploadSessionSnapshot>,
        now: i64,
    ) -> bool {
        // eMule `FindBestClientInQueue` marca al mejor cliente
        // inalcanzable cuando su puntuación supera a la del mejor conectado —
        // independientemente de la disponibilidad de ranura (`m_bAddNextConnect`
        // → ranura inmediata al reconectar). La cola está ordenada por puntuación
        // decreciente: el primer conectado no bloqueado es el mejor
        // candidato, el primer desconectado no bloqueado/no marcado previamente es
        // el mejor inalcanzable.
        let mut best_conn_score = 0u64;
        let mut best_disc: Option<(usize, u64)> = None;
        for (i, e) in q.iter().enumerate() {
            if e.banned {
                continue;
            }
            let s = e.effective_score(now);
            if e.connected {
                if s > best_conn_score {
                    best_conn_score = s;
                }
            } else if !e.add_next_connect && best_disc.is_none_or(|(_, bs)| s > bs) {
                best_disc = Some((i, s));
            }
        }
        if let Some((di, ds)) = best_disc {
            if ds > best_conn_score {
                q[di].add_next_connect = true;
            }
        }

        let mut promoted = false;
        loop {
            // Misma elegibilidad que `AcceptNewClient`: la tasa objetivo por
            // cliente, la capacidad `MaxSpeed` y el veto de USS se aplican también
            // a la promoción.
            if !self.can_open_slot(active, now) {
                break;
            }
            // eMule `ForceNewClient`: 1 s mínimo entre dos aperturas
            // mientras la tasa medida sea < 100 KB/s — la cola llena no
            // se vacía en ráfaga durante el arranque.
            if self.promotion_paced(active, now) {
                break;
            }
            // Primer cliente conectado, no bloqueado y no activo ya — al
            // estar la cola ordenada por puntuación, es el mejor candidato.
            // Los clientes desconectados no pueden recibir `ACCEPTUPLOADREQ`.
            let Some(idx) = q.iter().position(|e| {
                e.connected && !e.banned && !active.contains_key(&e.client_key)
            }) else {
                break;
            };
            let next = q.remove(idx).expect("queue index in bounds");

            let snap = UploadSessionSnapshot {
                client_key: next.client_key.clone(),
                client_address: next.client_address.clone(),
                file_hash: next.file_hash.clone(),
                file_name: next.file_name.clone(),
                transferred_bytes: 0,
                upload_rate_bps: 0,
                state: UploadSlotState::Uploading,
                queue_rank: None,
                started_at_unix_ms: now,
                rate_window_start_ms: 0,
                rate_window_bytes: 0,
                rate_sampled: false,
                credit_multiplier_bps: next.credit_multiplier_bps,
                file_prio_num: next.file_prio_num,
                // El inicio de espera sobrevive a la promoción — eMule lo conserva
                // en `credits`; volverá a usarse si el cliente es devuelto
                // a la cola por rotación de sesión.
                queued_since_ms: next.enqueued_at_unix_ms,
                auto_prio: next.auto_prio,
            };
            let promoted_hash = snap.file_hash.clone();
            tracing::info!(
                "[UPLOAD_PROMOTED] Client {} promoted from queue to active upload slot for file '{}' (hash: {})",
                log_addr_str(&snap.client_address), snap.file_name, snap.file_hash
            );
            active.insert(next.client_key, snap);
            // `AddUpNextClient` → `m_nLastStartUpload = GetTickCount()`.
            self.last_upload_start_ms.store(now, Ordering::Relaxed);
            // `AddUploadingClient` → `UpdateAutoUpPriority`.
            self.update_auto_up_prio(active, &promoted_hash);
            // eMule `AddUpNextClient` envía OP_ACCEPTUPLOADREQ al
            // socket en espera — el despachador despertado lo emite.
            next.promote_notify.notify_one();
            promoted = true;
        }
        promoted
    }

    /// Actualiza el progreso de subida activo y mide la velocidad real de la ranura.
    ///
    /// La velocidad se mide internamente por ventanas (`RATE_WINDOW_MS`,
    /// primera muestra en `RATE_FIRST_SAMPLE_MS`) a partir de los bytes
    /// realmente enviados — el invocador solo aporta `bytes_sent`, nunca
    /// una tasa estimada. `effective_rate_bps` disminuye hacia 0 cuando el par
    /// deja de pedir bloques.
    ///
    /// Focalización de ranuras (semántica de eMule): una ranura activa cuya tasa medida
    /// permanezca bajo `TRICKLE_RATE_BPS` tras `TRICKLE_GRACE_MS` se degrada a
    /// `Trickle` — mantiene ancho de banda residual pero libera su ranura para que el
    /// mejor cliente en cola sea promocionado. Esto concentra el ancho de banda en los clientes
    /// rápidos.
    pub async fn update_progress(&self, client_key: &str, bytes_sent: u64) {
        let mut demoted = false;
        {
            let mut active = self.active_uploads.lock().await;
            if let Some(cur) = active.get_mut(client_key) {
                cur.transferred_bytes = cur.transferred_bytes.saturating_add(bytes_sent);
                self.total_uploaded_bytes
                    .fetch_add(bytes_sent, Ordering::SeqCst);
                *self
                    .file_upload_totals
                    .write()
                    .unwrap()
                    .entry(cur.file_hash.to_lowercase())
                    .or_default() += bytes_sent;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                if cur.rate_window_start_ms == 0 {
                    cur.rate_window_start_ms = now;
                }
                cur.rate_window_bytes = cur.rate_window_bytes.saturating_add(bytes_sent);
                let elapsed = now.saturating_sub(cur.rate_window_start_ms);
                // Cierra la ventana vencida — o tras 500 ms en la primera
                // muestra — y mide la velocidad realmente servida.
                if elapsed >= RATE_WINDOW_MS
                    || (!cur.rate_sampled && elapsed >= RATE_FIRST_SAMPLE_MS)
                {
                    cur.upload_rate_bps =
                        cur.rate_window_bytes.saturating_mul(1000) / elapsed as u64;
                    cur.rate_window_start_ms = now;
                    cur.rate_window_bytes = 0;
                    cur.rate_sampled = true;
                }
                if cur.state == UploadSlotState::Uploading
                    && cur.rate_sampled
                    && cur.upload_rate_bps < TRICKLE_RATE_BPS
                    && now.saturating_sub(cur.started_at_unix_ms) > TRICKLE_GRACE_MS
                {
                    cur.state = UploadSlotState::Trickle;
                    demoted = true;
                    tracing::info!(
                        "[UPLOAD_TRICKLE] Client {} demoted to trickle slot (rate {} B/s < {} B/s) — slot freed for queue",
                        cur.client_address, cur.upload_rate_bps, TRICKLE_RATE_BPS
                    );
                }
            }
            if demoted {
                self.promote_waiting(&mut active).await;
            }
        }
        if demoted {
            self.emit_uploads_updated().await;
        } else {
            self.emit_uploads_updated_throttled().await;
        }
    }

    /// Libera la ranura de subida activa del cliente y promociona clientes en espera
    /// hasta volver a llenar las ranuras. Acepta tanto `client_key`
    /// (`uh:<userhash>` o `ip:port`) como una dirección simple `ip:port` — esta
    /// última se resuelve mediante el `client_address` almacenado para que las llamadas RPC
    /// que solo conocen la dirección sigan funcionando.
    pub async fn release_slot(&self, client_key: &str) {
        self.release_active(client_key, true).await;
        self.promote_after_release().await;
    }

    /// El socket TCP del cliente acaba de cerrarse — `Disconnected` de eMule **no**
    /// destruye a un cliente en cola (`bDelete = m_eUploadState !=
    /// US_ONUPLOADQUEUE`, `BaseClient.cpp:1169`): la entrada en `waitinglist`
    /// sobrevive hasta `MAX_PURGEQUEUETIME` con su `WaitStartTime` intacto,
    /// y `FindBestClientInQueue` la marca con `m_bAddNextConnect` si su
    /// puntuación supera al mejor conectado. Una ranura activa, en cambio,
    /// siempre se libera (`US_UPLOADING → RemoveFromUploadQueue`).
    pub async fn mark_disconnected(&self, client_key: &str) {
        let resolved_key = self.release_active(client_key, false).await;
        {
            let mut q = self.queue.lock().await;
            for entry in q.iter_mut() {
                if entry.client_key == resolved_key || entry.client_address == client_key {
                    entry.connected = false;
                }
            }
        }
        // Una ranura activa liberada puede promocionar a un cliente en espera conectado.
        self.promote_after_release().await;
    }

    /// Libera la posible ranura activa y resuelve la clave canónica
    /// (`uh:<hash>` si el cliente activo está registrado con `ip:port`).
    /// `drop_queue` retira también la entrada de la cola (finalización explícita);
    /// `!drop_queue` la conserva con `connected = false` según lo aplique
    /// el invocador (`mark_disconnected`). Devuelve la clave resuelta.
    async fn release_active(&self, client_key: &str, drop_queue: bool) -> String {
        let mut active = self.active_uploads.lock().await;
        let resolved_key = if active.contains_key(client_key) {
            client_key.to_string()
        } else {
            active
                .iter()
                .find(|(_, s)| s.client_address == client_key)
                .map(|(k, _)| k.clone())
                .unwrap_or_else(|| client_key.to_string())
        };
        if let Some(cur) = active.remove(&resolved_key) {
            tracing::info!(
                "[UPLOAD_RELEASED] Active upload slot released by client {} for file '{}' (transferred: {} bytes)",
                resolved_key, cur.file_name, cur.transferred_bytes
            );
            // `RemoveUploadingClient` → `UpdateAutoUpPriority`: el archivo
            // acaba de perder un subidor.
            self.update_auto_up_prio(&active, &cur.file_hash);
        }
        drop(active);

        if drop_queue {
            let mut q = self.queue.lock().await;
            q.retain(|entry| {
                entry.client_key != resolved_key && entry.client_address != client_key
            });
        }
        // Y el inicio de espera preservado por la rotación — el cliente ya no
        // volverá a reclamarlo.
        self.rotation_grace.lock().await.remove(&resolved_key);
        self.rotation_notifies.lock().await.remove(&resolved_key);
        resolved_key
    }

    /// Promociona desde la cola mientras queden ranuras libres — vuelve a adquirir el cerrojo
    /// `active_uploads` tras `release_active`.
    async fn promote_after_release(&self) {
        let mut active = self.active_uploads.lock().await;
        self.promote_waiting(&mut active).await;
        drop(active);
        self.emit_uploads_updated().await;
    }

    /// `OP_END_OF_DOWNLOAD` (`0x49`) — eMule `ListenSocket.cpp:537`: el
    /// descargador señala que ha terminado el archivo. Si `file_hash`
    /// coincide con el archivo actualmente subido o solicitado por este cliente,
    /// este es retirado de la ranura activa / de la cola (`RemoveFromUploadQueue`).
    /// Devuelve `true` si el cliente fue retirado; `false` si el hash no
    /// coincide con ningún archivo servido para este cliente (el llamador aplica
    /// entonces `CheckFailedFileIdReqs`).
    pub async fn end_of_download(&self, client_key: &str, file_hash: &str) -> bool {
        let hash_matches = {
            let active = self.active_uploads.lock().await;
            let in_active = active.values().any(|s| {
                (s.client_key == client_key || s.client_address == client_key)
                    && s.file_hash.eq_ignore_ascii_case(file_hash)
            });
            let q = self.queue.lock().await;
            let in_queue = q.iter().any(|e| {
                (e.client_key == client_key || e.client_address == client_key)
                    && e.file_hash.eq_ignore_ascii_case(file_hash)
            });
            in_active || in_queue
        };
        if hash_matches {
            self.release_slot(client_key).await;
        }
        hash_matches
    }

    /// Descriptor de promoción para la entrada en cola de `client_key` — el
    /// despachador lo espera para emitir `OP_ACCEPTUPLOADREQ` en cuanto el
    /// cliente sea promocionado (eMule `AddUpNextClient` escribe en el socket).
    pub async fn promotion_notify(
        &self,
        client_key: &str,
    ) -> Option<std::sync::Arc<tokio::sync::Notify>> {
        self.queue
            .lock()
            .await
            .iter()
            .find(|e| e.client_key == client_key)
            .map(|e| std::sync::Arc::clone(&e.promote_notify))
    }

    /// Descriptor de rotación para una sesión **activa** de `client_key` —
    /// `None` si el cliente no está subiendo datos. El despachador
    /// se suscribe para emitir `OP_OUTOFPARTREQS` cuando `process_tick`
    /// libera la ranura (`SendOutOfPartReqsAndAddToWaitingQueue` de eMule).
    pub async fn rotation_notify(
        &self,
        client_key: &str,
    ) -> Option<std::sync::Arc<tokio::sync::Notify>> {
        if !self
            .active_uploads
            .lock()
            .await
            .contains_key(client_key)
        {
            return None;
        }
        Some(
            self.rotation_notifies
                .lock()
                .await
                .entry(client_key.to_string())
                .or_insert_with(|| std::sync::Arc::new(tokio::sync::Notify::new()))
                .clone(),
        )
    }

    /// Tick periódico — equivalente a `CUploadQueue::Process()` invocado por
    /// `UploadTimer` en eMule:
    /// 1. purga de entradas de cola > `MAX_PURGEQUEUETIME` (1 h),
    /// 2. rotación de sesiones — `CheckForTimeOver` con
    ///    `FullChunkTransfers` (por defecto en eMule): un cliente que ha recibido más de
    ///    `SESSIONMAXTRANS` regresa a la cola si no se puede
    ///    abrir una nueva ranura (`!ForceNewClient()`), computando su tiempo de subida +
    ///    bonificación de 15 min como espera (`GetScore(isdownloading)`),
    /// 3. promoción de los mejores clientes en espera mientras las ranuras lo
    ///    permitan (`ForceNewClient`/`AcceptNewClient`).
    ///
    /// Debe ejecutarse permanentemente, incluso sin interfaz de usuario conectada.
    pub async fn process_tick(&self) {
        let mut active = self.active_uploads.lock().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let mut evicted: Vec<(String, i64)> = Vec::new();
        let mut changed = false;
        {
            let mut q = self.queue.lock().await;
            let before = q.len();
            q.retain(|e| {
                now.saturating_sub(e.last_request_unix_ms) <= MAX_PURGEQUEUE_MS
                    && self.hash_still_shared(&e.file_hash)
            });
            changed |= q.len() != before;
            // Rotación: mientras la cola esté disputada (sin nuevas ranuras
            // disponibles), las sesiones que superen SESSIONMAXTRANS ceden su
            // ranura — eMule `CheckForTimeOver`. La ranura liberada se cubre
            // inmediatamente (eMule promociona tras cada desalojo) antes
            // de evaluar la siguiente sesión.
            loop {
                if q.is_empty() || self.can_open_slot(&active, now) {
                    break;
                }
                let victim = active
                    .values()
                    .find(|s| {
                        s.state == UploadSlotState::Uploading
                            && s.transferred_bytes >= SESSION_MAX_TRANS_BYTES
                            && !self.is_friend(&s.client_key)
                    })
                    .map(|s| s.client_key.clone());
                let Some(victim) = victim else {
                    break;
                };
                if let Some(cur) = active.remove(&victim) {
                    tracing::info!(
                        "[UPLOAD_ROTATED] Client {} upload slot released after {} bytes session ('{}') — re-queues on next request",
                        log_addr_str(&cur.client_address), cur.transferred_bytes, cur.file_name
                    );
                    // eMule `SendOutOfPartReqsAndAddToWaitingQueue` — el
                    // despachador suscrito emite OP_OUTOFPARTREQS (0x57) por
                    // el socket aún abierto para que el descargador
                    // vuelva a estar en cola en lugar de seguir esperando bloques.
                    if let Some(n) =
                        self.rotation_notifies.lock().await.remove(&victim)
                    {
                        n.notify_one();
                    }
                    // eMule: el cliente NO se reencola aquí — vuelve
                    // a la cola en su próximo re-ask.
                    self.update_auto_up_prio(&active, &cur.file_hash);
                    evicted.push((cur.client_key, cur.queued_since_ms));
                }
                self.sort_queue_by_score(&mut q, now);
                changed |= self.promote_locked(&mut q, &mut active, now);
            }
            self.sort_queue_by_score(&mut q, now);
            changed |= self.promote_locked(&mut q, &mut active, now);
            drop(q);
        }
        // Inicio de espera original (conservado en `credits` en eMule) mantenido
        // para que la puntuación acumulada sobreviva a la rotación.
        if !evicted.is_empty() {
            let mut grace = self.rotation_grace.lock().await;
            for (key, since) in evicted {
                grace.insert(key, since);
            }
            drop(grace);
            changed = true;
        }
        self.active_count
            .store(Self::uploading_count(&active), Ordering::SeqCst);
        drop(active);
        if changed {
            self.emit_uploads_updated().await;
        }
    }

    /// Limpia la cola y las ranuras activas al activar el interruptor de corte o reiniciar la red.
    pub async fn abort_all(&self) {
        let mut active = self.active_uploads.lock().await;
        active.clear();
        self.active_count.store(0, Ordering::SeqCst);
        let mut q = self.queue.lock().await;
        q.clear();
        drop(q);
        self.rotation_grace.lock().await.clear();
        drop(active);
        self.emit_uploads_updated().await;
    }

    /// Devuelve la instantánea de la primera subida activa (si existe).
    /// `upload_rate_bps` es la tasa efectiva en el momento de la lectura.
    pub async fn active_snapshot(&self) -> Option<UploadSessionSnapshot> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let active = self.active_uploads.lock().await;
        active.values().next().cloned().map(|mut s| {
            s.upload_rate_bps = s.effective_rate_bps(now);
            s
        })
    }

    /// Devuelve instantáneas de todas las ranuras de subida activas.
    /// `upload_rate_bps` representa la velocidad efectiva en el instante de la lectura
    /// (disminuye hacia 0 si un par deja de pedir bloques).
    pub async fn active_snapshots(&self) -> Vec<UploadSessionSnapshot> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let active = self.active_uploads.lock().await;
        active
            .values()
            .cloned()
            .map(|mut s| {
                s.upload_rate_bps = s.effective_rate_bps(now);
                s
            })
            .collect()
    }

    /// Devuelve la posición basada en 1 de un cliente en espera coincidente por
    /// dirección IP (semántica `CUploadQueue::GetWaitingClientByIP_UDP`:
    /// los pings UDP solo transportan la IP emisora).
    pub async fn waiting_rank_by_ip(&self, ip: &std::net::IpAddr) -> Option<u32> {
        let ip_str = ip.to_string();
        let q = self.queue.lock().await;
        q.iter()
            .position(|entry| {
                entry
                    .client_address
                    .rsplit_once(':')
                    .map(|(host, _)| host == ip_str)
                    .unwrap_or(false)
            })
            .map(|pos| (pos + 1) as u32)
    }

    /// eMule `GetWaitingClientByIP_UDP(ip, udp_port, bIgnorePortOnUniqueIP
    /// = true, &bSenderMultipleIpUnknown)` + comprobación `md4equ(reqfilehash,
    /// sender->GetUploadFileID())` de `ClientUDPSocket.cpp`:
    /// - un cliente en cola con la misma IP **y** el mismo puerto UDP anunciado
    ///   se identifica directamente;
    /// - en caso contrario, si solo existe **un único** cliente en cola con esa IP,
    ///   se acepta (tolerancia de puerto, `bIgnorePortOnUniqueIP`);
    /// - varios clientes en la misma IP sin coincidencia de puerto →
    ///   [`UdpRankLookup::Ambiguous`] (eMule no responde: fuerza una
    ///   conexión TCP para despejar la ambigüedad de NAT);
    /// - el hash solicitado debe coincidir con el archivo del cliente encontrado,
    ///   de lo contrario [`UdpRankLookup::FileMismatch`] (eMule no responde).
    pub async fn waiting_rank_for_udp(
        &self,
        ip: &std::net::IpAddr,
        udp_port: u16,
        file_hash: &str,
    ) -> UdpRankLookup {
        let ip_str = ip.to_string();
        let clean_hash = file_hash.trim().to_lowercase();
        let q = self.queue.lock().await;
        let same_ip = |e: &&QueuedUpload| {
            e.client_address
                .rsplit_once(':')
                .map(|(host, _)| host == ip_str)
                .unwrap_or(false)
        };
        // 1. Coincidencia exacta IP + puerto UDP (desambiguación NAT).
        if let Some(pos) = q.iter().position(|e| {
            same_ip(&e) && e.client_udp_port.is_some_and(|p| p == udp_port)
        }) {
            let entry = &q[pos];
            return if entry.file_hash.eq_ignore_ascii_case(&clean_hash) {
                UdpRankLookup::Rank((pos + 1) as u32)
            } else {
                UdpRankLookup::FileMismatch
            };
        }
        // 2. Tolerancia de «IP única»: solo un cliente en cola en esta IP.
        let matches: Vec<usize> = q
            .iter()
            .enumerate()
            .filter(|(_, e)| same_ip(e))
            .map(|(pos, _)| pos)
            .collect();
        match matches.as_slice() {
            [] => UdpRankLookup::NoSender,
            [pos] => {
                let entry = &q[*pos];
                if entry.file_hash.eq_ignore_ascii_case(&clean_hash) {
                    UdpRankLookup::Rank((*pos + 1) as u32)
                } else {
                    UdpRankLookup::FileMismatch
                }
            }
            _ => UdpRankLookup::Ambiguous,
        }
    }

    /// eMule `SetLastUpRequest` — un `OP_REASKFILEPING` aceptado de un
    /// cliente en cola actualiza su marca temporal de re-ask (la purga
    /// `MAX_PURGEQUEUETIME` de 1 h no debe expulsar a un cliente activo que
    /// carece de socket TCP abierto para re-ask).
    pub async fn note_udp_reask(&self, ip: &std::net::IpAddr) {
        let ip_str = ip.to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let mut q = self.queue.lock().await;
        for entry in q.iter_mut() {
            if entry
                .client_address
                .rsplit_once(':')
                .map(|(host, _)| host == ip_str)
                .unwrap_or(false)
            {
                entry.last_request_unix_ms = now;
            }
        }
    }

    /// Cantidad de clientes esperando actualmente en la cola de subida.
    pub async fn waiting_len(&self) -> usize {
        self.queue.lock().await.len()
    }

    /// Verdadero si la cola de espera está a menos de 50 entradas de su capacidad máxima —
    /// eMule responde `OP_QUEUEFULL` a pings UDP de clientes desconocidos en
    /// dicho estado (`GetWaitingUserCount() + 50 > GetQueueSize()`).
    pub async fn is_nearly_full(&self) -> bool {
        self.waiting_len().await + 50 > self.max_queue.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Devuelve la lista de instantáneas de todas las subidas en cola.
    pub async fn list_queue(&self) -> Vec<UploadSessionSnapshot> {
        let q = self.queue.lock().await;
        q.iter()
            .enumerate()
            .map(|(idx, item)| UploadSessionSnapshot {
                client_key: item.client_key.clone(),
                client_address: item.client_address.clone(),
                file_hash: item.file_hash.clone(),
                file_name: item.file_name.clone(),
                transferred_bytes: 0,
                upload_rate_bps: 0,
                state: UploadSlotState::Waiting,
                queue_rank: Some((idx + 1) as u32),
                started_at_unix_ms: item.enqueued_at_unix_ms,
                rate_window_start_ms: 0,
                rate_window_bytes: 0,
                rate_sampled: false,
                credit_multiplier_bps: item.credit_multiplier_bps,
                file_prio_num: item.file_prio_num,
                queued_since_ms: item.enqueued_at_unix_ms,
                auto_prio: item.auto_prio,
            })
            .collect()
    }
}

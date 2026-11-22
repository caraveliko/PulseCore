// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use super::*;

impl DownloadService {
    /// Actualiza `!theApp.IsFirewalled()` — llamado por el coordinador
    /// de enjambre en cada tick (HighID eD2K o Kad UDP abierto verificado).
    /// Mientras seamos inaccesibles, las fuentes `lowid:` se
    /// rechazan en la ingesta: ninguna devolución de llamada puede hacer
    /// que nos contacten (`CPartFile::CanAddSource`, `PartFile.cpp:2397-2402`).
    pub fn set_lowid_source_acceptance(&mut self, accept: bool) {
        self.accept_lowid_sources = accept;
    }

    pub fn upsert_peer_sources(
        &mut self,
        file_hash: &str,
        sources: &[mule_core::model::PeerSource],
    ) -> Result<usize, DaemonError> {
        // `AddSources` de eMule: los clientes baneados (`IsBannedClient`) se
        // ignoran desde la ingesta, cualquiera que sea el canal de descubrimiento.
        // `IsLowID && IsFirewalled` se rechaza en el mismo punto en eMule
        // (`CanAddSource`) — una LowID no sirve de nada mientras no podamos
        // recibir devoluciones de llamada.
        let accept_lowid = self.accept_lowid_sources;
        let filtered: Vec<mule_core::model::PeerSource> = sources
            .iter()
            .filter(|s| !self.is_source_banned(&s.address))
            .filter(|s| accept_lowid || !s.address.starts_with("lowid:"))
            .cloned()
            .collect();
        if filtered.is_empty() {
            return Ok(0);
        }
        peers_servers::upsert_peer_sources(&mut self.db, &mut self.active, file_hash, &filtered)
    }

    /// Búsqueda mutable hash → descarga activa, compartida por los configuradores de tasa.
    fn active_mut_by_hash(&mut self, hex_hash: &str) -> Option<&mut ActiveDownload> {
        let clean_hex = hex_hash.trim().to_lowercase();
        self.active
            .values_mut()
            .find(|a| a.download.ed2k_hash.to_hex().to_lowercase() == clean_hex)
    }

    fn apply_rate(active: &mut ActiveDownload, rate_bps: u64) {
        active.current_rate_bps = rate_bps;
        active.last_rate_update = if rate_bps > 0 {
            Some(Instant::now())
        } else {
            None
        };
    }

    pub fn set_download_rate_by_hash(
        &mut self,
        hex_hash: &str,
        rate_bps: u64,
    ) -> Option<DownloadSnapshot> {
        let active = self.active_mut_by_hash(hex_hash)?;
        Self::apply_rate(active, rate_bps);
        Some(active.snapshot())
    }

    /// Variante sin instantánea de [`Self::set_download_rate_by_hash`] para la
    /// ruta crítica: la tasa se actualiza (incluso para `decay_stale_rates`)
    /// sin construir un `DownloadSnapshot` — la emisión de eventos es
    /// fusionada por el llamador (`sync_transfer_rates`).
    pub fn update_download_rate_by_hash(&mut self, hex_hash: &str, rate_bps: u64) -> bool {
        let Some(active) = self.active_mut_by_hash(hex_hash) else {
            return false;
        };
        Self::apply_rate(active, rate_bps);
        true
    }

    pub fn decay_stale_rates(&mut self) -> Vec<DownloadSnapshot> {
        let mut decayed = Vec::new();
        for active in self.active.values_mut() {
            if active.current_rate_bps > 0 {
                let is_stale = match active.last_rate_update {
                    Some(inst) => inst.elapsed() > Duration::from_secs(2),
                    None => true,
                };
                if is_stale {
                    active.current_rate_bps = 0;
                    active.last_rate_update = None;
                    decayed.push(active.snapshot());
                }
            }
        }
        decayed
    }

    /// Deriva la raíz eD2K de una descarga a partir de su conjunto de hashes de partes
    /// **sin volver a leer el disco** (eMule `PerformFileComplete` —
    /// `GetFileIdentifier().CalculateIdentifier()` calcula la raíz
    /// a partir de los hashes de partes, nunca volviendo a hashear el archivo).
    /// `Some(root)` solo cuando el hashset es conocido Y cada
    /// parte real está marcada como verificada (`verified_parts`); `None`
    /// en caso contrario — el llamador recurre entonces a `hash_reader`.
    pub fn try_verified_root(&self, id: &Uuid) -> Option<[u8; 16]> {
        let act = self.active.get(id)?;
        let total_size = act.download.total_size;
        if total_size == 0 {
            return None;
        }
        let hash_hex = act.download.ed2k_hash.to_hex().to_lowercase();
        let part_hashes = self.db.get_part_hashes(&hash_hex).ok()?;
        if part_hashes.is_empty() {
            // Archivo < 1 parte: no se persiste ningún hashset, la raíz
            // eD2K es entonces el hash del propio archivo (MD4 único).
            if total_size < ED2K_PART_SIZE as u64 {
                let verified: std::collections::HashSet<u32> =
                    self.db.get_verified_parts(id).ok()?.into_iter().collect();
                if verified.contains(&0) {
                    return Some(act.download.ed2k_hash.0);
                }
            }
            return None;
        }
        let total_parts = total_size.div_ceil(ED2K_PART_SIZE as u64).max(1) as u32;
        let verified: std::collections::HashSet<u32> = self
            .db
            .get_verified_parts(id)
            .ok()?
            .into_iter()
            .collect();
        if !(0..total_parts).all(|p| verified.contains(&p)) {
            return None;
        }
        mule_format::ed2k::ed2k_root_from_part_hashes(total_size, &part_hashes).map(|h| h.0)
    }

    pub fn get_pending_verifications(&mut self) -> Vec<(Uuid, PathBuf, PathBuf, [u8; 16], u64)> {
        let mut pending = Vec::new();
        let now = now_ms();
        for (id, active) in self.active.iter_mut() {
            if active.download.status == DownloadStatus::Completed
                || active.download.status == DownloadStatus::Failed
                || active.download.status == DownloadStatus::Paused
            {
                continue;
            }
            if active.verification_in_progress {
                continue;
            }
            // No entrar en verificación final mientras la recuperación de bloques AICH esté
            // pendiente — las partes corruptas todavía están cubiertas por el RangeSet.
            if !active.aich_pending.is_empty() {
                continue;
            }
            let total_size = active.download.total_size;
            let covered_len = active.ranges.covered_len();
            let is_complete = total_size > 0 && covered_len == total_size;

            if is_complete {
                tracing::info!(
                    "Download {id} ('{}') completed all byte ranges ({covered_len}/{total_size} bytes). Queuing for eD2K hash verification.",
                    active.download.display_name
                );
                active.verification_in_progress = true;
                active.download.status = DownloadStatus::Verifying;
                active.download.updated_at = now;
                let _ = self.db.update_status(id, DownloadStatus::Verifying);
                let _ = active.part_file.take();
                pending.push((
                    *id,
                    active.download.temp_path.clone(),
                    active.download.target_path.clone(),
                    active.download.ed2k_hash.0,
                    total_size,
                ));
            }
        }
        pending
    }

    pub fn list_peer_sources(
        &self,
        file_hash: &str,
    ) -> Result<Vec<mule_core::model::PeerSource>, DaemonError> {
        peers_servers::list_peer_sources(&self.db, file_hash)
    }

    pub fn update_peer_source_status(
        &mut self,
        file_hash: &str,
        address: &str,
        status: mule_core::model::PeerSourceStatus,
        error: Option<&str>,
        cooldown_until_unix_ms: Option<i64>,
    ) -> Result<bool, DaemonError> {
        let res = peers_servers::update_peer_source_status(
            &mut self.db,
            file_hash,
            address,
            status,
            error,
            cooldown_until_unix_ms,
        )?;
        self.refresh_source_count(file_hash);
        Ok(res)
    }

    /// Contador «srclist» al estilo `GetSourceCount()` de eMule: solo las
    /// fuentes activas cuentan — las entradas de lista muerta (fallo, FNF,
    /// filtro IP) permanecen persistidas pero no incrementan el contador.
    /// Actualizado con cada transición de estado o eliminación.
    fn refresh_source_count(&mut self, file_hash: &str) {
        let clean_hex = file_hash.trim().to_lowercase();
        if let Ok(list) = self.db.list_peer_sources(file_hash) {
            let count = list
                .iter()
                .filter(|s| s.status.is_srclist_visible())
                .count() as u32;
            for a in self.active.values_mut() {
                if a.download.ed2k_hash.to_hex().to_lowercase() == clean_hex {
                    a.source_count = count;
                }
            }
        }
    }

    pub fn update_peer_source_details(
        &mut self,
        file_hash: &str,
        address: &str,
        software: Option<&str>,
        availability: Option<&[u8]>,
    ) -> Result<bool, DaemonError> {
        peers_servers::update_peer_source_details(
            &mut self.db,
            file_hash,
            address,
            software,
            availability,
        )
    }

    pub fn update_peer_source_server(
        &mut self,
        file_hash: &str,
        address: &str,
        server_ip: u32,
        server_port: u16,
    ) -> Result<bool, DaemonError> {
        peers_servers::update_peer_source_server(
            &mut self.db,
            file_hash,
            address,
            server_ip,
            server_port,
        )
    }

    /// Persiste los puntos de enlace modernos (`ipv6_endpoint` y `quic_port`) de una fuente de par.
    pub fn update_peer_source_modern_endpoints(
        &mut self,
        file_hash: &str,
        address: &str,
        ipv6_endpoint: Option<&str>,
        quic_port: u16,
    ) -> Result<bool, DaemonError> {
        peers_servers::update_peer_source_modern_endpoints(
            &mut self.db,
            file_hash,
            address,
            ipv6_endpoint,
            quic_port,
        )
    }

    /// Recupera el punto de enlace IPv6 y puerto QUIC de una fuente de par.
    pub fn get_peer_source_modern_endpoints(
        &self,
        file_hash: &str,
        address: &str,
    ) -> Option<(Option<String>, u16)> {
        peers_servers::get_peer_source_modern_endpoints(&self.db, file_hash, address)
            .ok()
            .flatten()
    }

    /// Guarda o refresca la clave pública SUI verificada de un `user_hash`.
    pub fn upsert_verified_peer_pubkey(
        &mut self,
        user_hash: &str,
        pubkey_der: &[u8],
        pubkey_hash: &str,
        verified_at_unix_ms: i64,
        last_ip: Option<&str>,
    ) -> Result<(), DaemonError> {
        peers_servers::upsert_verified_peer_pubkey(
            &mut self.db,
            user_hash,
            pubkey_der,
            pubkey_hash,
            verified_at_unix_ms,
            last_ip,
        )
    }

    /// Recupera la clave pública SUI cacheada para un `user_hash`.
    pub fn get_verified_peer_pubkey(&self, user_hash: &str) -> Option<Vec<u8>> {
        peers_servers::get_verified_peer_pubkey(&self.db, user_hash)
            .ok()
            .flatten()
    }

    /// Purga las claves públicas SUI verificadas anteriores a `cutoff_unix_ms`.
    pub fn prune_stale_pubkeys(&mut self, cutoff_unix_ms: i64) -> Result<usize, DaemonError> {
        peers_servers::prune_stale_pubkeys(&mut self.db, cutoff_unix_ms)
    }

    /// Persiste los hashes de partes AICH recibidos mediante `OP_HASHSETANSWER2`.
    pub fn store_aich_part_hashes(
        &mut self,
        file_hash: &str,
        part_hashes: &[[u8; 20]],
    ) -> Result<(), DaemonError> {
        peers_servers::store_aich_part_hashes(&mut self.db, file_hash, part_hashes)
    }

    /// Carga los hashes de partes AICH persistidos para un hash de archivo.
    pub fn load_aich_part_hashes(&self, file_hash: &str) -> Vec<[u8; 20]> {
        peers_servers::load_aich_part_hashes(&self.db, file_hash).unwrap_or_default()
    }

    pub fn reset_peer_source_cooldown(
        &mut self,
        file_hash: &str,
        address: &str,
    ) -> Result<bool, DaemonError> {
        peers_servers::reset_peer_source_cooldown(&mut self.db, file_hash, address)
    }

    pub fn delete_peer_source(
        &mut self,
        file_hash: &str,
        address: &str,
    ) -> Result<bool, DaemonError> {
        let res = peers_servers::delete_peer_source(&mut self.db, file_hash, address)?;
        self.refresh_source_count(file_hash);
        Ok(res)
    }

    pub fn clear_peer_sources(&mut self, file_hash: &str) -> Result<usize, DaemonError> {
        let res = peers_servers::clear_peer_sources(&mut self.db, file_hash)?;
        self.refresh_source_count(file_hash);
        Ok(res)
    }

    pub fn find_by_ed2k_hash(&self, hex_hash: &str) -> Option<DownloadSnapshot> {
        let clean_hex = hex_hash.trim().to_lowercase();
        for active in self.active.values() {
            if active.download.ed2k_hash.to_hex().to_lowercase() == clean_hex {
                return Some(active.snapshot());
            }
        }
        None
    }

    /// Número de fuentes conocidas que cubren cada parte de una descarga activa —
    /// índice de rareza para la autorreparación del enjambre (ajuste `swarm_healing`).
    /// `None` si el hash no corresponde a una descarga activa.
    ///
    /// Convención eD2K: una fuente completa anuncia `part_count = 0` (máscara
    /// vacía) — cubre por tanto todas las partes. Las fuentes cuya máscara
    /// no haya sido actualizada desde `SOURCE_FRESHNESS_MS` se ignoran para
    /// no contabilizar una disponibilidad obsoleta.
    pub fn part_source_counts(&self, hex_hash: &str) -> Option<Vec<u32>> {
        let part_size = ED2K_PART_SIZE as u64;
        use types::SOURCE_FRESHNESS_MS;
        let clean = hex_hash.trim().to_lowercase();
        let act = self
            .active
            .values()
            .find(|a| a.download.ed2k_hash.to_hex().to_lowercase() == clean)?;
        let total_parts = (act.download.total_size.div_ceil(part_size)).max(1) as usize;
        let max_part = total_parts.min(u16::MAX as usize);
        let now = now_ms();
        let mut counts = vec![0u32; total_parts];
        for src in act.sources.values() {
            if now - src.last_seen_unix_ms > SOURCE_FRESHNESS_MS {
                continue;
            }
            if src.mask.part_count() == 0 {
                // Fuente completa: cubre todas las partes.
                for c in counts.iter_mut() {
                    *c = c.saturating_add(1);
                }
                continue;
            }
            for p in 0..max_part as u16 {
                if src.mask.has_part(p) {
                    counts[p as usize] = counts[p as usize].saturating_add(1);
                }
            }
        }
        Some(counts)
    }

    /// Almacena el conjunto de hashes MD4 por parte para un hash de archivo (aprendido de par, enlace o .part.met).
    pub fn store_part_hashes_by_hash(
        &mut self,
        hex_hash: &str,
        part_hashes: &[[u8; 16]],
    ) -> Result<(), DaemonError> {
        self.db
            .store_part_hashes(&hex_hash.trim().to_lowercase(), part_hashes)
            .map_err(db_err("storing part hashes"))
    }

    /// Carga el conjunto de hashes MD4 por parte para un hash de archivo, si se conoce.
    pub fn get_part_hashes_by_hash(&self, hex_hash: &str) -> Vec<[u8; 16]> {
        self.db
            .get_part_hashes(&hex_hash.trim().to_lowercase())
            .unwrap_or_default()
    }

    /// Carga el conjunto de hashes MD4 por parte para una descarga, si se conoce.
    pub fn get_part_hashes_for_download(&self, id: &Uuid) -> Vec<[u8; 16]> {
        self.active
            .get(id)
            .map(|a| self.get_part_hashes_by_hash(&a.download.ed2k_hash.to_hex()))
            .unwrap_or_default()
    }

    /// Desaloja partes corruptas de los rangos recibidos para que se vuelvan a descargar,
    /// y marca las partes limpiamente verificadas. Devuelve los índices efectivamente desalojados.
    pub fn evict_corrupted_parts(
        &mut self,
        id: &Uuid,
        corrupted: &[u32],
    ) -> Result<DownloadSnapshot, DaemonError> {
        operations::evict_corrupted_parts(&mut self.db, &mut self.active, id, corrupted)
    }

    // ── AICH recovery ───────────────────────────────────────────────────────

    /// Registra un hash maestro AICH recibido de un par (seguimiento de votos,
    /// semántica `CAICHUntrustedHash` de eMule). `masked_ip` = `ip & 0x00F0FFFF`.
    pub fn record_aich_root_vote(
        &mut self,
        id: &Uuid,
        root: [u8; 20],
        masked_ip: u32,
    ) -> operations::AichVoteOutcome {
        operations::record_aich_root_vote(&mut self.db, &mut self.active, id, root, masked_ip)
    }

    /// Devuelve el hash raíz AICH de confianza para una descarga, si existe.
    pub fn trusted_aich_root(&self, id: &Uuid) -> Option<[u8; 20]> {
        operations::trusted_aich_root(&self.active, id)
    }

    /// Siguiente parte corrupta en espera de un `OP_AICHREQUEST` para esta descarga.
    /// Devuelve `(part_index, master_hash)`.
    pub fn next_aich_recovery(&mut self, id: &Uuid) -> Option<(u32, [u8; 20])> {
        operations::next_aich_recovery(&mut self.db, &mut self.active, id)
    }

    /// Lo mismo, resuelto por hash de archivo ed2k.
    pub fn next_aich_recovery_by_hash(&mut self, hex_hash: &str) -> Option<(u32, [u8; 20])> {
        let clean_hex = hex_hash.trim().to_lowercase();
        let id = self
            .active
            .values()
            .find(|a| a.download.ed2k_hash.to_hex().to_lowercase() == clean_hex)
            .map(|a| a.download.id)?;
        self.next_aich_recovery(&id)
    }

    /// Vuelve a encolar o desaloja una parte cuya petición de recuperación AICH falló.
    pub fn fail_aich_recovery(&mut self, id: &Uuid, part: u32) {
        operations::fail_aich_recovery(&mut self.db, &mut self.active, id, part)
    }

    /// Falla la petición de recuperación AICH enviada más recientemente para esta descarga
    /// (utilizado cuando el par responde con un hash de archivo solo — el índice de parte no
    /// se devuelve en paquetes `OP_AICHANSWER` fallidos).
    pub fn fail_last_aich_request(&mut self, id: &Uuid) {
        let part = self.active.get(id).and_then(|a| {
            a.aich_requested
                .iter()
                .max_by_key(|(_, (ts, _))| *ts)
                .map(|(&p, _)| p)
        });
        if let Some(part) = part {
            self.fail_aich_recovery(id, part);
        }
    }

    /// Aplica los datos de recuperación AICH recibidos para una parte corrupta.
    pub fn apply_aich_recovery(
        &mut self,
        id: &Uuid,
        part: u32,
        master_hash: [u8; 20],
        entries: Vec<mule_format::AichRecoveryEntry>,
    ) -> Result<operations::AichRecoveryReport, DaemonError> {
        operations::apply_aich_recovery(
            &mut self.db,
            &mut self.active,
            id,
            part,
            master_hash,
            entries,
        )
    }

}

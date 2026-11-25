// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_core::{AllocationPolicy, Database, DownloadCategory, DownloadStatus, PartFile};
use mule_protocol::ed2k::{PartAvailabilityMask, ED2K_PART_SIZE};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::error::DaemonError;
use crate::services::DaemonEvent;

pub mod cancelled_files;
pub mod clients_met;
pub mod corruption_blackbox;
pub mod emfriends_import;
pub mod history_categories;
pub mod operations;
pub mod part_met;
pub mod peers_servers;
mod shared_files;
mod sources_and_aich;
pub mod staged;
pub mod types;

pub use part_met::{import_part_met, inspect_part_met};
pub use types::{
    make_snapshot, now_ms, ActiveDownload, ActiveSource, DownloadDetail, DownloadSnapshot,
    ImportPartMetResult, PartMetInspection, PartProgressSummary, SourceAvailabilitySummary,
    MAX_IMPORT_PART_COUNT, MAX_IMPORT_TOTAL_SIZE,
};

/// Duración del baneo de una IP de cliente — `CLIENTBANTIME` de eMule
/// (`Opcodes.h:123` — 2 horas).
pub const CLIENT_BAN_DURATION: Duration = Duration::from_secs(2 * 3600);

/// Duración de conservación de un cliente en la lista de seguimiento
/// `(IP,port)→userhash` — `KEEPTRACK_TIME` de eMule (`Opcodes.h:125` — 2 h).
pub const KEEP_TRACK_DURATION: Duration = CLIENT_BAN_DURATION;

/// Construye un cierre `map_err` que envuelve un error de persistencia en
/// `DaemonError::Business` con un contexto legible: `"DB error {context}: {e}"`.
pub(crate) fn db_err<E: std::fmt::Display>(
    context: &'static str,
) -> impl FnOnce(E) -> DaemonError {
    move |e| DaemonError::Business(format!("DB error {context}: {e}"))
}

/// Extrae la `IpAddr` de una dirección de origen `ip:port`; `None` para las
/// direcciones simbólicas `lowid:`/`kadcb:` (sin IP directa que banear).
pub(crate) fn source_addr_ip(addr: &str) -> Option<std::net::IpAddr> {
    addr.parse::<std::net::SocketAddr>()
        .map(|a| a.ip())
        .ok()
        .or_else(|| addr.parse::<std::net::IpAddr>().ok())
}

pub struct DownloadService {
    db: Database,
    temp_dir: PathBuf,
    active: HashMap<Uuid, ActiveDownload>,
    session_start_ms: i64,
    /// Canal de eventos instalado por `AppState` para eventos de ciclo de vida
    /// (`event.downloadAdded/Removed/Completed/Failed`).
    event_tx: Option<tokio::sync::broadcast::Sender<String>>,
    sequence: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
    /// IPs baneadas por el `CorruptionBlackBox`, la inundación de peticiones o un
    /// cambio de hash de usuario (`clientlist->AddBannedClient` de eMule —
    /// `CLIENTBANTIME` = 2 h, `Opcodes.h:123`).
    banned_source_ips: std::collections::HashMap<std::net::IpAddr, std::time::Instant>,
    /// `(IP, puerto TCP anunciado) → (userhash, instante)` — `m_trackedClientsMap`
    /// de eMule (`KEEPTRACK_TIME` = 2 h, `Opcodes.h:125`),
    /// completado cuando el par proporciona su clave pública SUI.
    tracked_clients: std::collections::HashMap<
        (std::net::IpAddr, u16),
        ([u8; 16], std::time::Instant),
    >,
    /// `!theApp.IsFirewalled()` — el daemon acepta fuentes `lowid:`
    /// únicamente mientras sea accesible para una devolución de conexión (HighID
    /// eD2K o Kad UDP abierto verificado); de lo contrario ningún `OP_CALLBACKREQUEST`
    /// puede hacer que nos devuelvan la llamada (`CPartFile::CanAddSource` de eMule,
    /// `PartFile.cpp:2397-2402`, rechazo en la ingesta). Actualizado por el
    /// coordinador de enjambre en cada tick; `false` al inicio — eMule
    /// nace con cortafuegos hasta la asignación de un ID o apertura de Kad.
    accept_lowid_sources: bool,
}

impl DownloadService {
    #[inline]
    pub fn db(&self) -> &Database {
        &self.db
    }

    pub fn new(db_path: &Path, temp_dir: PathBuf) -> Result<Self, DaemonError> {
        let mut db = Database::open(db_path)?;
        std::fs::create_dir_all(&temp_dir)?;

        let session_start_ms = now_ms();
        let existing = db.get_all_downloads().unwrap_or_default();
        let mut active = HashMap::new();
        for mut dl in existing {
            let ranges = db.load_received_ranges(&dl.id).unwrap_or_default();
            let covered = ranges.covered_len();

            // Autorreparación: sondear target_path si la descarga se estaba completando o verificando (evita estancamientos de NFS en archivos completados o en curso)
            let is_finalized_on_disk = matches!(
                dl.status,
                DownloadStatus::Completing | DownloadStatus::Verifying
            ) && dl.target_path.exists()
                && std::fs::metadata(&dl.target_path)
                    .map(|m| m.len())
                    .unwrap_or(0)
                    == dl.total_size;

            if matches!(
                dl.status,
                DownloadStatus::Completing | DownloadStatus::Verifying
            ) && is_finalized_on_disk
            {
                tracing::info!(
                    "Healing download {} ('{}') from {:?} to Completed (target file exists on disk)",
                    dl.id,
                    dl.display_name,
                    dl.status
                );
                dl.status = DownloadStatus::Completed;
                let _ = db.update_status(&dl.id, DownloadStatus::Completed);
            }

            // El archivo .part fue eliminado físicamente mientras el daemon
            // estaba detenido: los rangos persistidos en base de datos apuntan a
            // datos perdidos — la entrada se retira como una
            // eliminación de descarga. Salvaguarda: la carpeta contenedora debe
            // ser accesible; de lo contrario (unidad desconectada) es imposible
            // distinguir una eliminación de una ruta inaccesible.
            if !matches!(
                dl.status,
                DownloadStatus::Completed | DownloadStatus::Removed
            ) && !dl.temp_path.exists()
                && dl.temp_path.parent().is_some_and(|p| p.is_dir())
            {
                tracing::warn!(
                    "Download {} ('{}'): .part file missing on disk — removing download entry",
                    dl.id,
                    dl.display_name
                );
                let _ = db.delete_download(&dl.id);
                part_met::delete_part_met(&dl.temp_path);
                history_categories::record_history_event(
                    &mut db,
                    &dl,
                    covered,
                    0,
                    0,
                    "removed",
                    "unverified",
                    None,
                );
                continue;
            }

            let part_file = if dl.status == DownloadStatus::Completed {
                None
            } else {
                PartFile::open_or_create(&dl.temp_path, dl.total_size, AllocationPolicy::None).ok()
            };

            let source_count = db
                .list_peer_sources(&dl.ed2k_hash.to_hex())
                .map(|s| s.iter().filter(|x| x.status.is_srclist_visible()).count() as u32)
                .unwrap_or(0);
            let is_completed = dl.status == DownloadStatus::Completed;
            active.insert(
                dl.id,
                ActiveDownload {
                    last_status_change_unix_ms: dl.updated_at,
                    session_start_ms,
                    session_bytes_start: covered,
                    connection_attempts: 0,
                    resume_count: 0,
                    last_error: None,
                    download: dl,
                    ranges,
                    part_file,
                    sources: HashMap::new(),
                    current_rate_bps: 0,
                    last_rate_update: None,
                    source_count,
                    verification_in_progress: false,
                    aich_pending: Default::default(),
                    aich_requested: Default::default(),
                    aich_votes: Default::default(),
                    corruption_blackbox: Default::default(),
                    pending_corruption_bans: Vec::new(),
                    persist_dirty: false,
                    last_persist_flush: Instant::now(),
                    staged_parts: HashMap::new(),
                    content_header_verified: is_completed,
                },
            );
        }

        // Aplicar política de retención en el arranque
        let retention_days = db
            .get_setting("history_retention_days")
            .ok()
            .flatten()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(30);
        let _ = db.apply_retention_policy(retention_days);

        Ok(Self {
            db,
            temp_dir,
            active,
            session_start_ms,
            banned_source_ips: Default::default(),
            tracked_clients: Default::default(),
            accept_lowid_sources: false,
            event_tx: None,
            sequence: None,
        })
    }

    pub fn in_memory(temp_dir: PathBuf) -> Result<Self, DaemonError> {
        let db = Database::open_in_memory()?;
        std::fs::create_dir_all(&temp_dir)?;

        Ok(Self {
            db,
            temp_dir,
            active: HashMap::new(),
            session_start_ms: now_ms(),
            banned_source_ips: Default::default(),
            tracked_clients: Default::default(),
            accept_lowid_sources: false,
            event_tx: None,
            sequence: None,
        })
    }

    /// Instala el canal de difusión utilizado para emitir eventos del ciclo
    /// de vida de descargas. Llamado una sola vez por `AppState::new` antes de compartir el servicio.
    pub fn set_event_channel(
        &mut self,
        tx: tokio::sync::broadcast::Sender<String>,
        seq: std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) {
        self.event_tx = Some(tx);
        self.sequence = Some(seq);
    }

    fn emit(&self, make: impl FnOnce(u64) -> DaemonEvent) {
        let (Some(tx), Some(seq)) = (&self.event_tx, &self.sequence) else {
            return;
        };
        make(seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst)).send(tx);
    }

    /// Directorio temporal efectivo: la configuración persistida `temp_dir` tiene prioridad
    /// sobre el directorio de tiempo de construcción ("Directorio temporal" de eMule).
    fn effective_temp_dir(&self) -> PathBuf {
        self.db
            .get_setting("temp_dir")
            .ok()
            .flatten()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.temp_dir.clone())
    }

    pub fn add_ed2k_link(
        &mut self,
        link_str: &str,
        target_dir: &Path,
        policy: AllocationPolicy,
        category_id: Option<String>,
    ) -> Result<DownloadSnapshot, DaemonError> {
        let temp_dir = self.effective_temp_dir();
        std::fs::create_dir_all(&temp_dir)?;
        let snapshot = operations::add_ed2k_link(
            &mut self.db,
            &temp_dir,
            &mut self.active,
            link_str,
            target_dir,
            policy,
            category_id,
        )?;
        self.emit(|sequence| DaemonEvent::DownloadAdded {
            sequence,
            download: snapshot.clone(),
        });
        self.emit(|sequence| DaemonEvent::HistoryEntryAdded { sequence });
        Ok(snapshot)
    }

    pub fn inspect_part_met(
        &self,
        met_path: &Path,
        part_path: Option<&Path>,
    ) -> Result<PartMetInspection, DaemonError> {
        part_met::inspect_part_met(met_path, part_path)
    }

    pub fn import_part_met(
        &mut self,
        met_path: &Path,
        part_path: Option<&Path>,
        target_dir: &Path,
        category_id: Option<String>,
    ) -> Result<ImportPartMetResult, DaemonError> {
        let temp_dir = self.effective_temp_dir();
        std::fs::create_dir_all(&temp_dir)?;
        let (result, active_dl) = part_met::import_part_met(
            &mut self.db,
            &temp_dir,
            &self.active,
            met_path,
            part_path,
            target_dir,
            category_id,
            false,
        )?;

        let id = active_dl.download.id;
        let verif_status = if result.corrupted_parts > 0 {
            "partial_corruption"
        } else if result.verified_parts == result.total_parts && result.total_parts > 0 {
            "verified_ok"
        } else {
            "unverified"
        };
        history_categories::record_history_event(
            &mut self.db,
            &active_dl.download,
            active_dl.ranges.covered_len(),
            0,
            0,
            "imported",
            verif_status,
            None,
        );

        self.active.insert(id, active_dl);
        Ok(result)
    }

    /// Recupera archivos `.part.met` huérfanos encontrados en el directorio temporal
    /// efectivo al iniciar (semántica de carga automática de `CPartFile` de eMule): cualquier met
    /// cuyo hash de archivo no esté ya rastreado se importa mediante la misma ruta en
    /// aislamiento que `downloads.importPartMet` (verificación de hashes de parte,
    /// copia por etapas `{uuid}.part`, estado Paused). Devuelve el número de
    /// descargas recuperadas.
    pub fn recover_orphan_part_mets(&mut self, target_dir: &Path) -> usize {
        let temp_dir = self.effective_temp_dir();
        let entries = match std::fs::read_dir(&temp_dir) {
            Ok(e) => e,
            Err(_) => return 0,
        };

        let mut recovered = 0usize;
        for entry in entries.flatten() {
            let met_path = entry.path();
            let is_met = met_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(".part.met"))
                .unwrap_or(false);
            if !is_met {
                continue;
            }

            // Omitir archivos complementarios que ya pertenecen a una descarga activa
            // (`{uuid}.part.met` escrito por save_part_met).
            let already_tracked = match mule_format::PartMetFile::open(&met_path) {
                Ok(met) => self.active.values().any(|a| {
                    a.download
                        .ed2k_hash
                        .to_hex()
                        .eq_ignore_ascii_case(&met.file_hash.to_hex())
                }),
                Err(_) => false,
            };
            if already_tracked {
                continue;
            }

            match part_met::import_part_met(
                &mut self.db,
                &temp_dir,
                &self.active,
                &met_path,
                None,
                target_dir,
                None,
                true,
            ) {
                Ok((result, active_dl)) => {
                    // El huérfano fue adoptado in situ ({stem}.part renombrado a
                    // {uuid}.part): eliminar el met original para que no vuelva
                    // a procesarse en el siguiente arranque.
                    let _ = std::fs::remove_file(&met_path);
                    let verif_status = if result.corrupted_parts > 0 {
                        "partial_corruption"
                    } else if result.verified_parts == result.total_parts && result.total_parts > 0
                    {
                        "verified_ok"
                    } else {
                        "unverified"
                    };
                    history_categories::record_history_event(
                        &mut self.db,
                        &active_dl.download,
                        active_dl.ranges.covered_len(),
                        0,
                        0,
                        "imported",
                        verif_status,
                        None,
                    );
                    tracing::info!(
                        "Recovered orphaned .part.met '{}' ({}/{} parts verified) as paused download {}",
                        result.download.file_name,
                        result.verified_parts,
                        result.total_parts,
                        active_dl.download.id
                    );
                    self.active.insert(active_dl.download.id, active_dl);
                    recovered += 1;
                }
                Err(e) => {
                    tracing::warn!("Skipping orphaned .part.met {}: {e}", met_path.display());
                }
            }
        }
        recovered
    }

    /// Persiste los acompañantes `.part.met` para cada descarga no terminal —
    /// cadencia `SavePartFile` de eMule, invocada periódicamente por el latido del
    /// coordinador de enjambre y una vez durante el apagado limpio para que una caída nunca
    /// deje el estado de huecos más desfasado que el último vaciado. También vacía los datos
    /// `.part` sucios (fsync diferido por `PART_PERSIST_INTERVAL`): este punto
    /// es el último garantizado antes de la finalización del proceso.
    pub fn save_active_part_mets(&mut self) {
        let ids: Vec<Uuid> = self
            .active
            .values()
            .filter(|act| {
                !matches!(
                    act.download.status,
                    DownloadStatus::Completed | DownloadStatus::Removed
                )
            })
            .map(|act| act.download.id)
            .collect();
        for id in ids {
            if let Some(act) = self.active.get_mut(&id) {
                if let Err(e) = operations::persist_pending_ranges(&mut self.db, act, &id) {
                    tracing::warn!("save_active_part_mets flush {id}: {e}");
                }
                let _ = part_met::save_part_met(&self.db, act);
            }
        }
    }

    pub fn list_downloads(&self) -> Vec<DownloadSnapshot> {
        self.active.values().map(|a| a.snapshot()).collect()
    }

    /// Devuelve `(tasa_descarga_total_bps, descargas_activas)` sin construir instantáneas ni mapas de partes.
    pub fn download_rates_and_active_count(&self) -> (u64, usize) {
        let mut total_bps = 0u64;
        let mut active_count = 0usize;
        for act in self.active.values() {
            if let Some(last) = act.last_rate_update {
                if last.elapsed() <= std::time::Duration::from_secs(2) {
                    total_bps = total_bps.saturating_add(act.current_rate_bps);
                }
            }
            if act.download.status == DownloadStatus::Downloading {
                active_count += 1;
            }
        }
        (total_bps, active_count)
    }

    /// Número total de descargas activas/en cola en memoria.
    pub fn downloads_count(&self) -> usize {
        self.active.len()
    }

    /// Obtiene todos los ajustes persistidos en una única consulta SQLite.
    pub fn get_all_settings(
        &self,
    ) -> Result<std::collections::HashMap<String, String>, DaemonError> {
        self.db.get_all_settings().map_err(DaemonError::from)
    }

    /// Bytes útiles recibidos desde el inicio del daemon: suma, en todas
    /// las descargas activas, de `covered_len() - session_bytes_start`
    /// (el punto de partida se establece en la hidratación / al añadir).
    pub fn session_downloaded_bytes(&self) -> u64 {
        self.active
            .values()
            .map(|a| a.ranges.covered_len().saturating_sub(a.session_bytes_start))
            .sum()
    }

    /// Detalles completos (incluyendo estado por parte y disponibilidad de fuentes) para
    /// que la lista de descargas pueda renderizar un mapa de partes al estilo eMule por fila.
    pub fn list_download_details(&self) -> Vec<DownloadDetail> {
        self.active
            .values()
            .filter_map(|a| operations::get_download_detail(&self.db, &self.active, &a.download.id))
            .collect()
    }

    pub fn get_download(&self, id: &Uuid) -> Option<DownloadSnapshot> {
        self.active.get(id).map(|a| a.snapshot())
    }

    pub fn get_download_detail(&self, id: &Uuid) -> Option<DownloadDetail> {
        operations::get_download_detail(&self.db, &self.active, id)
    }

    pub fn pause_download(&mut self, id: &Uuid) -> Result<DownloadSnapshot, DaemonError> {
        let snap = operations::pause_download(&mut self.db, &mut self.active, id)?;
        self.emit(|sequence| DaemonEvent::HistoryEntryAdded { sequence });
        Ok(snap)
    }

    pub fn resume_download(&mut self, id: &Uuid) -> Result<DownloadSnapshot, DaemonError> {
        let snap = operations::resume_download(&mut self.db, &mut self.active, id)?;
        self.emit(|sequence| DaemonEvent::HistoryEntryAdded { sequence });
        Ok(snap)
    }

    pub fn remove_download(&mut self, id: &Uuid, delete_files: bool) -> Result<(), DaemonError> {
        operations::remove_download(&mut self.db, &mut self.active, id, delete_files)?;
        self.emit(|sequence| DaemonEvent::DownloadRemoved {
            sequence,
            id: id.to_string(),
        });
        self.emit(|sequence| DaemonEvent::HistoryEntryAdded { sequence });
        Ok(())
    }

    pub fn inject_range(
        &mut self,
        id: &Uuid,
        offset: u64,
        data: &[u8],
        sender: Option<&str>,
    ) -> Result<DownloadSnapshot, DaemonError> {
        operations::inject_range(&mut self.db, &mut self.active, id, offset, data, sender)
    }

    /// Variante de [`Self::inject_range`] para la ruta crítica de transferencia:
    /// la instantánea devuelta no incluye `parts` (resumen O(partes × rangos)
    /// demasiado costoso por bloque de 180 KiB). No emitir el retorno hacia los
    /// clientes RPC/WS — solo sirve para leer `status`/`progress`.
    pub fn inject_range_brief(
        &mut self,
        id: &Uuid,
        offset: u64,
        data: &[u8],
        sender: Option<&str>,
    ) -> Result<DownloadSnapshot, DaemonError> {
        operations::inject_range_brief(&mut self.db, &mut self.active, id, offset, data, sender)
    }

    /// Búsqueda ligera hash → id: evita `find_by_ed2k_hash` (instantánea completa)
    /// en la ruta crítica de transferencia, que solo necesita el identificador.
    pub fn id_by_ed2k_hash(&self, hex_hash: &str) -> Option<Uuid> {
        let clean_hex = hex_hash.trim().to_lowercase();
        self.active
            .values()
            .find(|a| a.download.ed2k_hash.to_hex().to_lowercase() == clean_hex)
            .map(|a| a.download.id)
    }

    /// Vaciado periódico de `.part` sucios (fsync diferido) — respaldo cuando
    /// el flujo de bloques se interrumpe entre dos `inject_range` (la cadencia
    /// principal vive en la ruta de recepción). Llamado en el tick del
    /// coordinador de enjambre; no-op cuando no hay nada sucio.
    pub fn flush_dirty_downloads(&mut self) {
        let ids: Vec<Uuid> = self
            .active
            .values()
            .filter(|a| {
                a.persist_dirty && a.last_persist_flush.elapsed() >= types::PART_PERSIST_INTERVAL
            })
            .map(|a| a.download.id)
            .collect();
        for id in ids {
            if let Some(act) = self.active.get_mut(&id) {
                if let Err(e) = operations::persist_pending_ranges(&mut self.db, act, &id) {
                    tracing::warn!("flush_dirty_downloads {id}: {e}");
                }
            }
        }
    }

    /// Vacía los baneos solicitados por el `CorruptionBlackBox` para esta
    /// descarga (`Ban`/`AddBannedClient` de eMule). El llamador debe
    /// desconectar la sesión del par; las IPs devueltas se añaden a la
    /// lista de baneados de sesión consultada por la ingesta y el marcador.
    pub fn drain_corruption_bans(&mut self, id: &Uuid) -> Vec<String> {
        let Some(act) = self.active.get_mut(id) else {
            return Vec::new();
        };
        let bans = std::mem::take(&mut act.pending_corruption_bans);
        for addr in &bans {
            if let Some(ip) = source_addr_ip(addr) {
                self.banned_source_ips
                    .insert(ip, std::time::Instant::now());
            }
        }
        bans
    }

    /// `clientlist->IsBannedClient` de eMule: ¿está la IP baneada desde
    /// hace menos de `CLIENTBANTIME` (2 h)? (`ClientList.cpp:347` — el baneo
    /// caduca, no es permanente).
    pub fn is_source_banned(&self, addr: &str) -> bool {
        let Some(ip) = source_addr_ip(addr) else {
            return false;
        };
        self.banned_source_ips
            .get(&ip)
            .is_some_and(|since| since.elapsed() < CLIENT_BAN_DURATION)
    }

    /// Número de IPs actualmente bajo baneo (no caducadas — para el
    /// latido consolidado).
    pub fn banned_source_count(&self) -> usize {
        self.banned_source_ips
            .values()
            .filter(|since| since.elapsed() < CLIENT_BAN_DURATION)
            .count()
    }

    /// Banea explícitamente una IP por `CLIENTBANTIME` (2 h).
    pub fn ban_source(&mut self, addr: &str) {
        if let Some(ip) = source_addr_ip(addr) {
            self.banned_source_ips
                .insert(ip, std::time::Instant::now());
        }
    }

    /// `AddTrackClient` de eMule (`ClientList.cpp:367`): memoriza el
    /// `(IP, puerto TCP anunciado) → userhash` de un par que presentó su clave
    /// pública SUI, conservado durante `KEEPTRACK_TIME` (2 h).
    pub fn track_client(
        &mut self,
        ip: std::net::IpAddr,
        announced_tcp_port: u16,
        user_hash: [u8; 16],
    ) {
        // Purga perezosa de entradas caducadas (eMule purga en
        // `Process` cada 30 s — aquí en cada inserción, el volumen
        // se mantiene bajo).
        self.tracked_clients
            .retain(|_, (_, seen)| seen.elapsed() < KEEP_TRACK_DURATION);
        self.tracked_clients.insert(
            (ip, announced_tcp_port),
            (user_hash, std::time::Instant::now()),
        );
    }

    /// `ComparePriorUserhash` de eMule (`ClientList.cpp:386`): `false` si este
    /// `(IP, port)` ya fue seguido bajo **otro** userhash — fuerte indicio
    /// de suplantación de identidad (eMule banea la IP en este caso).
    /// `true` si es desconocido o el hash es idéntico.
    pub fn check_prior_userhash(
        &self,
        ip: std::net::IpAddr,
        announced_tcp_port: u16,
        user_hash: &[u8; 16],
    ) -> bool {
        match self.tracked_clients.get(&(ip, announced_tcp_port)) {
            Some((prior, seen)) if seen.elapsed() < KEEP_TRACK_DURATION => prior == user_hash,
            _ => true,
        }
    }

    pub fn update_status(
        &mut self,
        id: &Uuid,
        status: DownloadStatus,
    ) -> Result<DownloadSnapshot, DaemonError> {
        let snapshot = operations::update_status(&mut self.db, &mut self.active, id, status)?;
        match status {
            DownloadStatus::Completed => {
                if let Some(act) = self.active.get(id) {
                    operations::trigger_completion_hook(&act.download.target_path, act.download.category_id.as_deref());
                }
                self.emit(|sequence| DaemonEvent::DownloadCompleted {
                    sequence,
                    download: snapshot.clone(),
                });
            }
            DownloadStatus::Failed => {
                let reason = self.active.get(id).and_then(|a| a.last_error.clone());
                self.emit(|sequence| DaemonEvent::DownloadFailed {
                    sequence,
                    download: snapshot.clone(),
                    reason: reason.clone(),
                });
                self.emit(|sequence| DaemonEvent::HistoryEntryAdded { sequence });
            }
            _ => {}
        }
        Ok(snapshot)
    }

    /// Actualiza la prioridad de archivo al estilo eMule ("low" | "normal" | "high" |
    /// "auto"). "auto" es `PR_AUTO` de eMule (`m_bAutoDownPriority`): la
    /// prioridad efectiva es recalculada por el asignador en cada tick
    /// según el número de fuentes (`UpdateAutoDownPriority`: más de 100
    /// fuentes → low, más de 20 → normal, de lo contrario → high). Elegir un
    /// valor manual desactiva el modo auto — el indicador es el valor almacenado,
    /// como `FT_DLPRIORITY`.
    ///
    /// El asignador del enjambre lo utiliza para sesgar la cuota de ranuras por archivo.
    pub fn set_priority(
        &mut self,
        id: &Uuid,
        priority: &str,
    ) -> Result<DownloadSnapshot, DaemonError> {
        let act = self
            .active
            .get_mut(id)
            .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;
        if !matches!(priority, "low" | "normal" | "high" | "auto") {
            return Err(DaemonError::Business(format!(
                "Invalid priority '{priority}' (expected low|normal|high|auto)"
            )));
        }
        act.download.priority = priority.to_string();
        self.db.update_priority(id, priority)?;
        Ok(act.snapshot())
    }

    /// Actualiza el límite de fuentes por archivo (`FT_MAXSOURCES` de eMule). `None`
    /// restablece el valor predeterminado global `max_peers_per_file`.
    pub fn set_max_sources(
        &mut self,
        id: &Uuid,
        max_sources: Option<u32>,
    ) -> Result<DownloadSnapshot, DaemonError> {
        let act = self
            .active
            .get_mut(id)
            .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;
        if let Some(v) = max_sources {
            if v == 0 || v > 10_000 {
                return Err(DaemonError::Business(format!(
                    "Invalid maxSources '{v}' (expected 1..=10000 or null)"
                )));
            }
        }
        act.download.max_sources = max_sources;
        self.db.update_max_sources(id, max_sources)?;
        Ok(act.snapshot())
    }

    pub fn get_missing_ranges(&self, id: &Uuid) -> Option<Vec<std::ops::Range<u64>>> {
        self.active
            .get(id)
            .map(|a| a.ranges.missing_within(0..a.download.total_size))
    }

    pub fn get_covered_len(&self, id: &Uuid) -> Option<u64> {
        self.active.get(id).map(|a| a.ranges.covered_len())
    }

    pub fn get_download_file_info(
        &self,
        id: &Uuid,
    ) -> Option<(PathBuf, PathBuf, mule_format::Ed2kHash, u64)> {
        self.active.get(id).map(|a| {
            (
                a.download.temp_path.clone(),
                a.download.target_path.clone(),
                a.download.ed2k_hash,
                a.download.total_size,
            )
        })
    }

    pub fn record_source_availability(
        &mut self,
        id: &Uuid,
        source: SocketAddr,
        mask: PartAvailabilityMask,
    ) -> Result<DownloadSnapshot, DaemonError> {
        operations::record_source_availability(&mut self.active, id, source, mask)
    }

    pub fn get_source_summaries(&self, id: &Uuid) -> Option<Vec<SourceAvailabilitySummary>> {
        self.active.get(id).map(|a| {
            a.sources
                .values()
                .map(|s| SourceAvailabilitySummary {
                    source_id: s.source_id.clone(),
                    available_part_count: s.mask.available_parts().count() as u16,
                    total_part_count: s.mask.part_count(),
                    last_seen_unix_ms: s.last_seen_unix_ms,
                })
                .collect()
        })
    }

    pub fn get_sources(&self, id: &Uuid) -> Option<HashMap<SocketAddr, PartAvailabilityMask>> {
        self.active.get(id).map(|a| {
            a.sources
                .iter()
                .map(|(addr, s)| (*addr, s.mask.clone()))
                .collect()
        })
    }

    pub fn prepare_finalize(
        &mut self,
        id: &Uuid,
    ) -> Result<(std::path::PathBuf, std::path::PathBuf, DownloadSnapshot), DaemonError> {
        operations::prepare_finalize(&mut self.db, &mut self.active, id)
    }

    pub fn commit_finalize(
        &mut self,
        id: &Uuid,
        final_path: std::path::PathBuf,
    ) -> Result<DownloadSnapshot, DaemonError> {
        operations::commit_finalize(&mut self.db, &mut self.active, id, final_path)
    }

    pub fn finalize_download(&mut self, id: &Uuid) -> Result<DownloadSnapshot, DaemonError> {
        operations::finalize_download(&mut self.db, &mut self.active, id)
    }

    pub fn set_destination(
        &mut self,
        id: &Uuid,
        target_dir: &Path,
    ) -> Result<DownloadDetail, DaemonError> {
        operations::set_destination(&mut self.db, &mut self.active, id, target_dir)
    }

    /// Actualiza la ruta de destino final de una descarga completada (ej.: tras organización en el NAS)
    pub fn update_target_path(
        &mut self,
        id: &Uuid,
        new_target_path: &Path,
    ) -> Result<(), DaemonError> {
        operations::update_target_path(&mut self.db, &mut self.active, id, new_target_path)
    }

    /// `downloads.rename` — renombra el archivo (display_name + destino;
    /// disco + `shared_files` para una descarga terminada).
    pub fn rename_download(
        &mut self,
        id: &Uuid,
        new_name: &str,
    ) -> Result<DownloadSnapshot, DaemonError> {
        operations::rename_download(&mut self.db, &mut self.active, id, new_name)
    }

    /// `shares.rename` — renombra un archivo compartido completo en disco
    /// y en `shared_files` (delega en `rename_download` cuando el hash
    /// respalda una descarga listada).
    pub fn rename_shared_file(
        &mut self,
        file_hash: &str,
        new_name: &str,
    ) -> Result<(), DaemonError> {
        operations::rename_shared_file(&mut self.db, &mut self.active, file_hash, new_name)
    }

    pub fn retry_connection(&mut self, id: &Uuid) -> Result<DownloadDetail, DaemonError> {
        operations::retry_connection(&mut self.db, &mut self.active, id)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, DaemonError> {
        self.db
            .get_setting(key)
            .map_err(db_err("reading setting"))
    }

    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<(), DaemonError> {
        self.db
            .set_setting(key, value)
            .map_err(db_err("saving setting"))
    }

    pub fn remove_setting(&mut self, key: &str) -> Result<bool, DaemonError> {
        self.db
            .remove_setting(key)
            .map_err(db_err("removing setting"))
    }

    pub fn list_settings(&self) -> Result<Vec<(String, String)>, DaemonError> {
        self.db
            .list_settings()
            .map_err(db_err("listing settings"))
    }

    pub fn query_history(
        &self,
        filter: &mule_core::HistoryFilter,
    ) -> Result<Vec<mule_core::HistoryEntry>, DaemonError> {
        history_categories::query_history(&self.db, filter)
    }

    pub fn get_history_stats(&self) -> Result<mule_core::HistoryStats, DaemonError> {
        history_categories::get_history_stats(&self.db, self.session_start_ms)
    }

    pub fn clear_history(&mut self) -> Result<usize, DaemonError> {
        history_categories::clear_history(&mut self.db)
    }

    pub fn apply_retention_policy(&mut self, retention_days: u32) -> Result<usize, DaemonError> {
        history_categories::apply_retention_policy(&mut self.db, retention_days)
    }

    pub fn export_history(
        &self,
        format: &str,
        filter: &mule_core::HistoryFilter,
    ) -> Result<String, DaemonError> {
        history_categories::export_history(&self.db, format, filter)
    }

    // --- Gestión de categorías ---

    pub fn list_categories(&self) -> Result<Vec<DownloadCategory>, DaemonError> {
        history_categories::list_categories(&self.db)
    }

    pub fn get_category(&self, id: &str) -> Result<Option<DownloadCategory>, DaemonError> {
        history_categories::get_category(&self.db, id)
    }

    pub fn create_category(
        &mut self,
        name: &str,
        color: &str,
        default_destination: Option<String>,
        default_priority: Option<String>,
    ) -> Result<DownloadCategory, DaemonError> {
        history_categories::create_category(
            &mut self.db,
            name,
            color,
            default_destination,
            default_priority,
        )
    }

    pub fn update_category(
        &mut self,
        id: &str,
        name: Option<String>,
        color: Option<String>,
        default_destination: Option<Option<String>>,
        default_priority: Option<Option<String>>,
        sort_order: Option<i32>,
    ) -> Result<DownloadCategory, DaemonError> {
        history_categories::update_category(
            &mut self.db,
            id,
            name,
            color,
            default_destination,
            default_priority,
            sort_order,
        )
    }

    pub fn delete_category(&mut self, id: &str) -> Result<usize, DaemonError> {
        history_categories::delete_category(&mut self.db, &mut self.active, id)
    }

    pub fn reorder_categories(&mut self, category_ids: &[String]) -> Result<(), DaemonError> {
        history_categories::reorder_categories(&mut self.db, category_ids)
    }

    pub fn set_download_category(
        &mut self,
        download_id: &Uuid,
        category_id: Option<String>,
    ) -> Result<DownloadDetail, DaemonError> {
        if !self.active.contains_key(download_id) {
            return Err(DaemonError::Business(format!(
                "Download {download_id} not found"
            )));
        }

        let cat_opt = category_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if let Some(ref cid) = cat_opt {
            let cat_exists = self.get_category(cid)?.is_some();
            if !cat_exists {
                return Err(DaemonError::Business(format!("Category '{cid}' not found")));
            }
        }

        self.db
            .set_download_category(download_id, cat_opt.as_deref())
            .map_err(db_err("setting download category"))?;

        let now = now_ms();
        let active = self
            .active
            .get_mut(download_id)
            .ok_or_else(|| DaemonError::NotFound(format!("Download {download_id} not found")))?;
        active.download.category_id = cat_opt;
        active.download.updated_at = now;

        self.get_download_detail(download_id).ok_or_else(|| {
            DaemonError::Business(format!("Download {download_id} detail not found"))
        })
    }

    pub fn upsert_servers(
        &mut self,
        servers: &[mule_core::model::Ed2kServer],
    ) -> Result<usize, DaemonError> {
        peers_servers::upsert_servers(&mut self.db, servers)
    }

    pub fn list_servers(&self) -> Result<Vec<mule_core::model::Ed2kServer>, DaemonError> {
        peers_servers::list_servers(&self.db)
    }

    pub fn set_server_enabled(
        &mut self,
        address: &str,
        enabled: bool,
    ) -> Result<bool, DaemonError> {
        peers_servers::set_server_enabled(&mut self.db, address, enabled)
    }

    pub fn delete_server(&mut self, address: &str) -> Result<bool, DaemonError> {
        peers_servers::delete_server(&mut self.db, address)
    }

    pub fn clear_servers(&mut self) -> Result<usize, DaemonError> {
        peers_servers::clear_servers(&mut self.db)
    }

    pub fn get_peer_credit(
        &self,
        pubkey_hash: &str,
    ) -> Result<Option<mule_core::model::PeerCredit>, DaemonError> {
        peers_servers::get_peer_credit(&self.db, pubkey_hash)
    }

    pub fn record_credit_upload(
        &mut self,
        pubkey_hash: &str,
        pubkey_hex: &str,
        bytes: u64,
    ) -> Result<(), DaemonError> {
        peers_servers::record_credit_upload(&mut self.db, pubkey_hash, pubkey_hex, bytes)
    }

    pub fn record_credit_download(
        &mut self,
        pubkey_hash: &str,
        pubkey_hex: &str,
        bytes: u64,
    ) -> Result<(), DaemonError> {
        peers_servers::record_credit_download(&mut self.db, pubkey_hash, pubkey_hex, bytes)
    }

    pub fn list_peer_credits(
        &self,
        limit: usize,
    ) -> Result<Vec<mule_core::model::PeerCredit>, DaemonError> {
        peers_servers::list_peer_credits(&self.db, limit)
    }

    pub fn add_friend(&mut self, user_hash: &str, name: &str) -> Result<(), DaemonError> {
        self.db.add_friend(user_hash, name).map_err(Into::into)
    }

    pub fn remove_friend(&mut self, user_hash: &str) -> Result<(), DaemonError> {
        self.db.remove_friend(user_hash).map_err(Into::into)
    }

    pub fn list_friends(&self) -> Result<Vec<mule_core::model::Friend>, DaemonError> {
        self.db.list_friends().map_err(Into::into)
    }

    /// `CClientList::LoadFriendsList` — importa un archivo `emfriends.met`.
    /// Devuelve el informe; el llamador registra cada entrada de `user_hashes`
    /// en el gestor de subidas.
    pub fn import_emfriends_met(
        &mut self,
        path: &std::path::Path,
    ) -> Result<emfriends_import::EmfriendsImportReport, DaemonError> {
        emfriends_import::import_emfriends_met(&mut self.db, path)
    }

    /// `IsCancelledFileByID` — `true` cuando `ed2k_hash` (hex) está en el
    /// conjunto de cancelados (control `remember_cancelled`, al igual que en eMule).
    pub fn is_cancelled_file(&self, ed2k_hash: &str) -> Result<bool, DaemonError> {
        let Some(hash) = cancelled_files::parse_hash16(ed2k_hash) else {
            return Ok(false);
        };
        cancelled_files::is_cancelled_file(&self.db, &hash)
    }

    /// `CKnownFileList::LoadCancelled` — importa un archivo `cancelled.met`.
    pub fn import_cancelled_met(
        &mut self,
        path: &std::path::Path,
    ) -> Result<cancelled_files::CancelledImportReport, DaemonError> {
        cancelled_files::import_cancelled_met(&mut self.db, path)
    }

    /// Número de hashes enmascarados en el conjunto de cancelados (inspección).
    pub fn cancelled_count(&self) -> Result<usize, DaemonError> {
        self.db
            .cancelled_count()
            .map_err(db_err("counting cancelled"))
    }

    /// `CClientCreditsList::LoadList` — importa un `clients.met` de eMule
    /// en `peer_credits` (créditos indexados por la clave pública RSA almacenada).
    pub fn import_clients_met(
        &mut self,
        path: &std::path::Path,
    ) -> Result<clients_met::ClientsMetImportReport, DaemonError> {
        clients_met::import_clients_met(&mut self.db, path)
    }

    pub fn db_ref(&self) -> &Database {
        &self.db
    }

    pub fn db_mut(&mut self) -> &mut Database {
        &mut self.db
    }

    pub fn insert_episodes_batch(&mut self, episodes: &[mule_core::model::EpisodeItem]) -> Result<usize, mule_core::error::CoreError> {
        self.db.insert_episodes_batch(episodes)
    }

    pub fn upsert_shared_files_batch(&mut self, files: &[mule_core::model::SharedFile]) -> Result<usize, mule_core::error::CoreError> {
        self.db.upsert_shared_files_batch(files)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// `CLIENTBANTIME` (2 h): un baneo caduca como `IsBannedClient` de eMule
    /// (`ClientList.cpp:347` — `curTick < dwBantime + CLIENTBANTIME`).
    #[test]
    fn ban_expires_after_client_ban_time() {
        let tmp = tempfile::tempdir().unwrap();
        let mut svc = DownloadService::in_memory(tmp.path().join("t")).unwrap();

        svc.ban_source("1.2.3.4:4662");
        assert!(svc.is_source_banned("1.2.3.4:4662"));

        // Baneo aplicado hace más de 2 h → caducado.
        let ip: std::net::IpAddr = "1.2.3.4".parse().unwrap();
        svc.banned_source_ips.insert(
            ip,
            Instant::now() - CLIENT_BAN_DURATION - Duration::from_secs(1),
        );
        assert!(!svc.is_source_banned("1.2.3.4:4662"));
    }

    /// `ComparePriorUserhash` de eMule (`ClientList.cpp:386`): mismo (IP, port) seguido
    /// bajo otro userhash → `false`; userhash idéntico o desconocido →
    /// `true`. Las entradas caducan tras `KEEPTRACK_TIME` (2 h).
    #[test]
    fn prior_userhash_detection_and_expiry() {
        let tmp = tempfile::tempdir().unwrap();
        let mut svc = DownloadService::in_memory(tmp.path().join("t")).unwrap();
        let ip: std::net::IpAddr = "9.9.9.9".parse().unwrap();

        // Desconocido → OK.
        assert!(svc.check_prior_userhash(ip, 4662, &[0xAA; 16]));

        svc.track_client(ip, 4662, [0xAA; 16]);
        // Mismo hash → OK; hash diferente → usurpación.
        assert!(svc.check_prior_userhash(ip, 4662, &[0xAA; 16]));
        assert!(!svc.check_prior_userhash(ip, 4662, &[0xBB; 16]));
        // Otro puerto → identidad distinta (eMule empareja por (ip, port)).
        assert!(svc.check_prior_userhash(ip, 4663, &[0xBB; 16]));

        // Entrada caducada → OK de nuevo.
        svc.tracked_clients.insert(
            (ip, 4662),
            ([0xAA; 16], Instant::now() - KEEP_TRACK_DURATION - Duration::from_secs(1)),
        );
        assert!(svc.check_prior_userhash(ip, 4662, &[0xBB; 16]));
    }
}

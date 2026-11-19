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
        let active = self.active.get_mut(download_id).unwrap();
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

    /// Almacena en caché la raíz AICH calculada de un archivo compartido.
    pub fn set_shared_aich_root(
        &mut self,
        file_hash: &str,
        aich_root: &[u8; 20],
    ) -> Result<bool, DaemonError> {
        self.db
            .set_shared_aich_root(file_hash, aich_root)
            .map_err(db_err("setting share AICH root"))
    }

    pub fn get_completed_ranges_by_hash(&self, hex_hash: &str) -> Vec<(u64, u64)> {
        let clean_hex = hex_hash.trim().to_lowercase();
        for active in self.active.values() {
            if active.download.ed2k_hash.to_hex().to_lowercase() == clean_hex {
                return active
                    .ranges
                    .ranges()
                    .iter()
                    .map(|r| (r.start, r.end))
                    .collect();
            }
        }
        Vec::new()
    }

    pub fn list_shared_files(&self) -> Result<Vec<mule_core::model::SharedFile>, DaemonError> {
        peers_servers::list_shared_files(&self.db)
    }

    pub fn add_shared_file(
        &mut self,
        file: &mule_core::model::SharedFile,
    ) -> Result<(), DaemonError> {
        peers_servers::add_shared_file(&mut self.db, file)
    }

    /// `UAPPref` de eMule (`GetNewAutoUp`): ¿inician los nuevos archivos compartidos
    /// con prioridad de subida «auto»? (predeterminado `true`).
    pub fn auto_up_priority_enabled(&self) -> bool {
        peers_servers::auto_up_priority_enabled(&self.db)
    }

    /// Añade `delta` bytes al total subido persistido de un archivo compartido.
    pub fn add_share_uploaded_bytes(
        &mut self,
        file_hash: &str,
        delta: u64,
    ) -> Result<(), DaemonError> {
        peers_servers::add_share_uploaded_bytes(&mut self.db, file_hash, delta)
    }

    /// Totales subidos persistidos por hash de archivo (claves en minúsculas).
    pub fn list_share_uploaded_bytes(
        &self,
    ) -> Result<std::collections::HashMap<String, u64>, DaemonError> {
        peers_servers::list_share_uploaded_bytes(&self.db)
    }

    /// Retira un archivo de los recursos compartidos. Con `delete_file`, el archivo en
    /// disco se elimina adicionalmente: para una descarga completada aún
    /// activa se delega en `remove_download` (entrada en BD + archivo final +
    /// recurso compartido); de lo contrario, el archivo señalado por la entrada se elimina
    /// y luego la entrada `shared_files` — y la eventual descarga completada en BD —
    /// se retiran.
    pub fn remove_shared_file(
        &mut self,
        file_hash: &str,
        delete_file: bool,
    ) -> Result<bool, DaemonError> {
        if !delete_file {
            return peers_servers::remove_shared_file(&mut self.db, file_hash);
        }
        let sf = self
            .get_shared_file(file_hash)?
            .ok_or_else(|| DaemonError::NotFound(format!("Shared file {file_hash} not found")))?;
        if sf.is_partial {
            // Recurso compartido transitorio de una descarga en curso: el archivo a
            // eliminar es el `.part`, lo cual es función de
            // `downloads.remove` — nunca eliminarlo aquí.
            return Err(DaemonError::Business(
                "Cannot delete a partial share — remove the download instead".to_string(),
            ));
        }
        if let Some(id) = self
            .active
            .values()
            .find(|a| {
                a.download
                    .ed2k_hash
                    .to_hex()
                    .eq_ignore_ascii_case(file_hash)
            })
            .map(|a| a.download.id)
        {
            self.remove_download(&id, true)?;
            return Ok(true);
        }
        let path = PathBuf::from(&sf.file_path);
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        let removed = peers_servers::remove_shared_file(&mut self.db, file_hash)?;
        // Descarga completada restante en BD (fuera de `active`): su target_path
        // acaba de ser eliminada, la entrada queda muerta — la retiramos.
        let stale_db_id = self
            .db
            .get_all_downloads()
            .unwrap_or_default()
            .into_iter()
            .find(|d| {
                d.status == DownloadStatus::Completed
                    && d.ed2k_hash.to_hex().eq_ignore_ascii_case(file_hash)
            })
            .map(|d| d.id);
        if let Some(id) = stale_db_id {
            let _ = self.db.delete_download(&id);
            return Ok(true);
        }
        Ok(removed)
    }

    /// Establece la prioridad de subida de un archivo compartido ("verylow".."veryhigh").
    pub fn set_shared_up_priority(
        &mut self,
        file_hash: &str,
        up_priority: &str,
    ) -> Result<bool, DaemonError> {
        self.db
            .set_shared_up_priority(file_hash, up_priority)
            .map_err(Into::into)
    }

    pub fn get_shared_file(
        &self,
        file_hash: &str,
    ) -> Result<Option<mule_core::model::SharedFile>, DaemonError> {
        if let Some(sf) = peers_servers::get_shared_file(&self.db, file_hash)? {
            if sf.file_path.is_empty() || Path::new(&sf.file_path).is_file() {
                return Ok(Some(sf));
            }
            // Entrada persistida cuyo archivo ya no existe: no
            // servible — dejamos decidir a las fuentes sintetizadas.
        }
        for act in self.active.values() {
            if !act
                .download
                .ed2k_hash
                .to_hex()
                .eq_ignore_ascii_case(file_hash)
            {
                continue;
            }
            if act.download.status == DownloadStatus::Completed
                && !act.download.target_path.is_file()
            {
                continue;
            }
            let mut sf = Self::shared_entry_for_active(act);
            sf.aich_root = act.download.aich_root.map(|a| a.0);
            return Ok(Some(sf));
        }
        // Las descargas completadas se persisten en la BD incluso cuando no están cargadas
        // en `active` (ej.: tras un reinicio del daemon) — permanecen compartidas
        // al igual que eMule mantiene los archivos finalizados en la lista compartida.
        if let Some(sf) = self.shared_entry_for_completed_db(file_hash) {
            return Ok(Some(sf));
        }
        Ok(None)
    }

    /// Hashes actualmente presentes en la lista de recursos compartidos — `sharedfiles`
    /// de eMule: entradas activas cuyo archivo existe + descargas
    /// completadas persistidas o cargadas cuyo archivo aún existe.
    /// Los `.part` en curso **no** figuran aquí (`FindBestClientInQueue` de eMule
    /// solo consulta `sharedfiles->GetFileByID` — los clientes en espera de un
    /// archivo parcial se purgan igual que en eMule). Utilizado por
    /// `UploadManager` para purgar de la cola las solicitudes huérfanas.
    pub fn list_shared_hashes(&self) -> std::collections::HashSet<String> {
        let mut set = std::collections::HashSet::new();
        if let Ok(shares) = peers_servers::list_shared_files(&self.db) {
            for sf in shares {
                if sf.enabled && (sf.file_path.is_empty() || Path::new(&sf.file_path).is_file()) {
                    set.insert(sf.file_hash.to_lowercase());
                }
            }
        }
        for act in self.active.values() {
            if act.download.status == DownloadStatus::Completed
                && act.download.target_path.is_file()
            {
                set.insert(act.download.ed2k_hash.to_hex().to_lowercase());
            }
        }
        if let Ok(dls) = self.db.get_all_downloads() {
            for d in dls {
                if d.status == DownloadStatus::Completed && d.target_path.is_file() {
                    set.insert(d.ed2k_hash.to_hex().to_lowercase());
                }
            }
        }
        set
    }

    /// Resuelve una descarga completada desde la BD en una entrada compartible —
    /// únicamente cuando el archivo de destino existe realmente en disco.
    fn shared_entry_for_completed_db(
        &self,
        file_hash: &str,
    ) -> Option<mule_core::model::SharedFile> {
        let dl = self.db.get_all_downloads().ok()?.into_iter().find(|d| {
            d.status == mule_core::DownloadStatus::Completed
                && d.ed2k_hash.to_hex().eq_ignore_ascii_case(file_hash)
        })?;
        if !dl.target_path.is_file() {
            return None;
        }
        let mut sf = mule_core::model::SharedFile::new(
            dl.ed2k_hash.to_hex(),
            &dl.display_name,
            dl.total_size,
            dl.target_path.to_string_lossy().as_ref(),
        );
        sf.aich_root = dl.aich_root.map(|a| a.0);
        sf.category_id = dl.category_id.clone();
        Some(sf)
    }

    /// Construye la vista de archivo compartido de una descarga activa (semántica
    /// `CPartFile` de eMule): una descarga completada se resuelve a su ruta de destino; una
    /// descarga en progreso se resuelve a su archivo `.part` como recurso compartido parcial
    /// exponiendo únicamente sus partes eD2K completamente recibidas. Los archivos de partes
    /// siempre son archivos compartidos en eMule (`CDownloadQueue::AddPartFilesToShare`), incluso
    /// antes de que se complete cualquier parte — la máscara vacía `completed_parts` simplemente
    /// no anuncia nada servible todavía.
    fn shared_entry_for_active(act: &ActiveDownload) -> mule_core::model::SharedFile {
        if act.download.status == mule_core::DownloadStatus::Completed {
            let mut sf = mule_core::model::SharedFile::new(
                act.download.ed2k_hash.to_hex(),
                &act.download.display_name,
                act.download.total_size,
                act.download.target_path.to_string_lossy().as_ref(),
            );
            sf.category_id = act.download.category_id.clone();
            return sf;
        }
        let part_size = ED2K_PART_SIZE as u64;
        let part_count = act
            .download
            .total_size
            .div_ceil(part_size)
            .min(u16::MAX as u64) as u16;
        let completed: Vec<u16> = (0..part_count)
            .filter(|&i| {
                let start = i as u64 * part_size;
                let end = ((i as u64 + 1) * part_size).min(act.download.total_size);
                end > start && act.ranges.contains(start..end)
            })
            .collect();
        let mut sf = mule_core::model::SharedFile::new(
            act.download.ed2k_hash.to_hex(),
            &act.download.display_name,
            act.download.total_size,
            act.download.temp_path.to_string_lossy().as_ref(),
        );
        sf.is_partial = true;
        sf.completed_parts = completed;
        sf.category_id = act.download.category_id.clone();
        sf
    }

    /// Cada archivo actualmente servible a los pares: archivos compartidos persistidos más
    /// descargas activas (completadas → archivo completo, en curso → partes
    /// completadas únicamente), al igual que `CSharedFileList` de eMule incluyendo `CPartFile`s.
    pub fn list_all_shared_files(&self) -> Result<Vec<mule_core::model::SharedFile>, DaemonError> {
        let mut list = self.list_shared_files()?;
        // Los recursos compartidos persistidos cuyo archivo desapareció del disco ya
        // no son servibles — filtrados en la lectura para que ni la interfaz,
        // ni la oferta del servidor, ni la publicación Kad anuncien un
        // archivo muerto. Las entradas sin ruta (recurso compartido solo por hash)
        // se conservan.
        list.retain(|f| f.file_path.is_empty() || Path::new(&f.file_path).is_file());
        let persisted: std::collections::HashSet<String> =
            list.iter().map(|f| f.file_hash.to_lowercase()).collect();
        for act in self.active.values() {
            if persisted.contains(&act.download.ed2k_hash.to_hex().to_lowercase()) {
                continue;
            }
            // Misma salvaguarda que `shared_entry_for_completed_db`: destino
            // eliminado físicamente → nada servible a sintetizar.
            if act.download.status == DownloadStatus::Completed
                && !act.download.target_path.is_file()
            {
                continue;
            }
            let mut sf = Self::shared_entry_for_active(act);
            sf.aich_root = act.download.aich_root.map(|a| a.0);
            list.push(sf);
        }
        // Las descargas completadas solo viven en la BD tras un reinicio — fusionarlas
        // también para que la lista de compartidos sobreviva a reinicios (eMule vuelve a hashear
        // los archivos completados en `CSharedFileList` al iniciar).
        for d in self.db.get_all_downloads()? {
            if d.status != mule_core::DownloadStatus::Completed {
                continue;
            }
            if persisted.contains(&d.ed2k_hash.to_hex().to_lowercase()) {
                continue;
            }
            if let Some(sf) = self.shared_entry_for_completed_db(&d.ed2k_hash.to_hex()) {
                list.push(sf);
            }
        }
        Ok(list)
    }

    /// `shares.setCategory` — asocia (o retira) una categoría a un
    /// archivo compartido identificado por su hash eD2K. Actualiza las dos
    /// fuentes posibles: la fila `shared_files` persistida y/o la
    /// descarga (activa o completada en BD) cuyo recurso compartido se sintetiza
    /// — `list_all_shared_files` refleja la categoría desde una u
    /// otra.
    pub fn set_shared_file_category(
        &mut self,
        file_hash: &str,
        category_id: Option<String>,
    ) -> Result<(), DaemonError> {
        let cat = category_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(ref cid) = cat {
            if self.get_category(cid)?.is_none() {
                return Err(DaemonError::Business(format!("Category '{cid}' not found")));
            }
        }
        let mut touched = false;
        // Descarga activa conocida por este hash.
        let active_id = self
            .active
            .values()
            .find(|a| {
                a.download
                    .ed2k_hash
                    .to_hex()
                    .eq_ignore_ascii_case(file_hash)
            })
            .map(|a| a.download.id);
        if let Some(id) = active_id {
            self.db
                .set_download_category(&id, cat.as_deref())
                .map_err(db_err("setting category"))?;
            if let Some(a) = self.active.get_mut(&id) {
                a.download.category_id = cat.clone();
            }
            touched = true;
        } else {
            // Descarga completada presente solo en BD.
            let db_id = self
                .db
                .get_all_downloads()?
                .into_iter()
                .find(|d| d.ed2k_hash.to_hex().eq_ignore_ascii_case(file_hash))
                .map(|d| d.id);
            if let Some(id) = db_id {
                self.db
                    .set_download_category(&id, cat.as_deref())
                    .map_err(db_err("setting category"))?;
                touched = true;
            }
        }
        // Recurso compartido persistido autónomo (archivo añadido fuera de descarga).
        if self
            .db
            .set_shared_file_category(file_hash, cat.as_deref())
            .map_err(db_err("setting share category"))?
        {
            touched = true;
        }
        if !touched {
            return Err(DaemonError::Business(format!(
                "Shared file '{file_hash}' not found"
            )));
        }
        Ok(())
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

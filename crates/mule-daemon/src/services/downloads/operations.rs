// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_core::{AllocationPolicy, Database, Download, DownloadStatus, PartFile, RangeSet};
use mule_format::Ed2kFileLink;
use mule_protocol::ed2k::PartAvailabilityMask;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::cancelled_files;
use super::db_err;
use super::history_categories::record_history_event;
use super::peers_servers;
use super::types::{
    compute_part_summaries, make_snapshot, now_ms, ActiveDownload, ActiveSource, DownloadDetail,
    DownloadSnapshot, PART_PERSIST_INTERVAL,
};
use crate::error::DaemonError;
use std::time::{Duration, Instant};

/// Tamaño de una parte eD2K (`PARTSIZE` de eMule) — granularidad del hash MD4
/// incremental y de la recuperación AICH.
const ED2K_PART_SIZE_U64: u64 = mule_protocol::ed2k::ED2K_PART_SIZE as u64;

pub fn add_ed2k_link(
    db: &mut Database,
    temp_dir: &Path,
    active: &mut HashMap<Uuid, ActiveDownload>,
    link_str: &str,
    target_dir: &Path,
    policy: AllocationPolicy,
    category_id: Option<String>,
) -> Result<DownloadSnapshot, DaemonError> {
    let link =
        Ed2kFileLink::parse(link_str).map_err(|e| DaemonError::InvalidParams(e.to_string()))?;

    let clean_name = mule_format::fix_mojibake(&link.file_name);
    mule_network_policy::content_policy::validate_video_filename(&clean_name)
        .map_err(|e| DaemonError::Business(e.to_string()))?;

    let clean_hex = link.ed2k_hash.to_hex().to_lowercase();
    if let Some(existing) = active
        .values()
        .find(|a| a.download.ed2k_hash.to_hex().to_lowercase() == clean_hex)
    {
        return Err(DaemonError::Business(format!(
            "Ce fichier est déjà dans la liste des téléchargements ('{}', statut: {:?})",
            existing.download.display_name, existing.download.status
        )));
    }

    let id = Uuid::new_v4();
    let temp_filename = format!("{id}.part");
    let temp_path = temp_dir.join(temp_filename);
    let target_path = target_dir.join(&clean_name);

    let download = Download {
        id,
        display_name: clean_name,
        total_size: link.size,
        ed2k_hash: link.ed2k_hash,
        aich_root: link.aich_root.map(mule_format::AichHash),
        target_path,
        temp_path: temp_path.clone(),
        status: DownloadStatus::Queued,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64,
        updated_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64,
        category_id,
        // `DAPPref` de eMule (`GetNewAutoDown`, predeterminado true): una nueva
        // descarga se inicia con prioridad «auto» — el indicador se almacena y
        // el asignador resuelve low/normal/high en cada tick según el
        // número de fuentes (`UpdateAutoDownPriority`). No es una
        // asignación puntual: «auto» permanece activo mientras
        // el usuario no elija una prioridad manual.
        priority: if db
            .get_setting("auto_priority_new_downloads")
            .ok()
            .flatten()
            .map(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "on"))
            .unwrap_or(true)
        {
            "auto".to_string()
        } else {
            "normal".to_string()
        },
        max_sources: None,
    };

    let part_file = PartFile::open_or_create(&temp_path, link.size, policy)
        .map_err(|e| DaemonError::Business(format!("Storage error: {e}")))?;

    db.insert_download(&download)
        .map_err(db_err("persisting state"))?;

    if !link.part_hashes.is_empty() {
        let _ = db.store_part_hashes(&clean_hex, &link.part_hashes);
    }

    let ranges = RangeSet::new();
    let snapshot = make_snapshot(&download, &ranges, 0, 0);
    let now = now_ms();

    active.insert(
        id,
        ActiveDownload {
            download,
            ranges,
            part_file: Some(part_file),
            sources: HashMap::new(),
            connection_attempts: 0,
            resume_count: 0,
            last_error: None,
            last_status_change_unix_ms: now,
            session_start_ms: now,
            session_bytes_start: 0,
            current_rate_bps: 0,
            last_rate_update: None,
            source_count: 0,
            verification_in_progress: false,
            aich_pending: Default::default(),
            aich_requested: Default::default(),
            aich_votes: Default::default(),
            corruption_blackbox: Default::default(),
            pending_corruption_bans: Vec::new(),
            persist_dirty: false,
            last_persist_flush: Instant::now(),
            staged_parts: HashMap::new(),
            content_header_verified: false,
        },
    );

    // eMule crea el archivo complementario .part.met tan pronto como se añade la descarga.
    if let Some(act) = active.get(&id) {
        let _ = super::part_met::save_part_met(db, act);
    }

    Ok(snapshot)
}

pub fn get_download_detail(
    db: &Database,
    active: &HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
) -> Option<DownloadDetail> {
    let act = active.get(id)?;
    let snapshot = act.snapshot();

    let verified_parts_set: std::collections::HashSet<u32> = db
        .get_verified_parts(id)
        .unwrap_or_default()
        .into_iter()
        .collect();

    let parts = compute_part_summaries(
        act.download.total_size,
        &act.ranges,
        act.sources.values(),
        &verified_parts_set,
    );
    let total_verified_bytes: u64 = parts.iter().filter(|p| p.is_verified).map(|p| p.size).sum();

    let aich_part = act
        .download
        .aich_root
        .as_ref()
        .map(|a| format!("&h={}", a.to_base32()))
        .unwrap_or_default();

    let ed2k_link = format!(
        "ed2k://|file|{}|{}|{}|{}/",
        act.download.display_name,
        act.download.total_size,
        hex::encode(act.download.ed2k_hash.0),
        aich_part
    );

    let covered = act.ranges.covered_len();
    let remaining_bytes = act.download.total_size.saturating_sub(covered);

    let effective_rate = snapshot.download_rate_bps;
    let average_rate_bps = if act.session_start_ms > 0 {
        let now_time = now_ms();
        let elapsed_secs = ((now_time - act.session_start_ms) / 1000).max(1) as u64;
        let transferred_session = covered.saturating_sub(act.session_bytes_start);
        transferred_session / elapsed_secs
    } else {
        0
    };

    let eta_seconds = if effective_rate > 0 && remaining_bytes > 0 {
        Some(remaining_bytes / effective_rate)
    } else {
        None
    };

    let mut enriched_snapshot = snapshot;
    enriched_snapshot.verified_bytes = total_verified_bytes;

    Some(DownloadDetail {
        snapshot: enriched_snapshot,
        ed2k_hash: hex::encode(act.download.ed2k_hash.0),
        aich_root: act.download.aich_root.as_ref().map(|a| a.to_base32()),
        ed2k_link,
        temp_path: act.download.temp_path.to_string_lossy().to_string(),
        target_path: act.download.target_path.to_string_lossy().to_string(),
        remaining_bytes,
        average_rate_bps,
        eta_seconds,
        connection_attempts: act.connection_attempts,
        last_error: act.last_error.clone(),
        last_status_change_unix_ms: act.last_status_change_unix_ms,
        parts,
    })
}

pub fn pause_download(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
) -> Result<DownloadSnapshot, DaemonError> {
    let act = active
        .get_mut(id)
        .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;

    if act.download.status == DownloadStatus::Completed {
        return Ok(act.snapshot());
    }

    let now = now_ms();
    act.download.status = DownloadStatus::Paused;
    act.download.updated_at = now;
    act.last_status_change_unix_ms = now;
    db.update_status(id, DownloadStatus::Paused)
        .map_err(db_err("persisting state"))?;

    // Vaciado de los datos recibidos en espera antes de la transición: la
    // contabilidad de los rangos no debe sobrevivir únicamente en memoria
    // durante una pausa (`SavePartFile` de eMule en cambio de estado).
    persist_pending_ranges(db, act, id)?;

    // Persistir .part.met en pausa (eMule guarda el met en transiciones de estado).
    let _ = super::part_met::save_part_met(db, act);

    Ok(act.snapshot())
}

pub fn resume_download(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
) -> Result<DownloadSnapshot, DaemonError> {
    let (dl, ranges, conn_att, res_cnt) = {
        let act = active
            .get_mut(id)
            .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;

        mule_network_policy::content_policy::validate_video_filename(&act.download.display_name)
            .map_err(|e| DaemonError::Business(e.to_string()))?;

        let now = now_ms();
        if act.download.status == DownloadStatus::Failed
            && act.ranges.covered_len() == act.download.total_size
        {
            act.ranges = RangeSet::new();
            act.staged_parts.clear();
            let _ = db.sync_received_ranges(id, &act.ranges);
        }
        if act.part_file.is_none() {
            act.part_file = PartFile::open_or_create(
                &act.download.temp_path,
                act.download.total_size,
                AllocationPolicy::None,
            )
            .ok();
        }
        act.download.status = DownloadStatus::Downloading;
        act.download.updated_at = now;
        act.last_status_change_unix_ms = now;
        act.resume_count = act.resume_count.saturating_add(1);
        act.verification_in_progress = false;
        let _ = db.update_status(id, DownloadStatus::Downloading);

        (
            act.download.clone(),
            act.ranges.clone(),
            act.connection_attempts,
            act.resume_count,
        )
    };

    record_history_event(
        db,
        &dl,
        ranges.covered_len(),
        conn_att,
        res_cnt,
        "resumed",
        "unverified",
        None,
    );

    let act = active
        .get(id)
        .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;
    Ok(act.snapshot())
}

pub fn remove_download(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
    delete_files: bool,
) -> Result<(), DaemonError> {
    let mut temp_path_to_delete = None;
    let mut temp_path_to_keep = None;
    // Descarga terminada: `delete_files` cubre también el archivo final
    // en la carpeta de destino (retirado de la lista de compartidos).
    let mut completed_file_to_delete: Option<(PathBuf, String)> = None;
    let (was_active, history_data) = if let Some(mut act) = active.remove(id) {
        let _ = act.part_file.take();
        if delete_files {
            temp_path_to_delete = Some(act.download.temp_path.clone());
            if act.download.status == DownloadStatus::Completed {
                completed_file_to_delete = Some((
                    act.download.target_path.clone(),
                    act.download.ed2k_hash.to_hex(),
                ));
            }
        } else {
            temp_path_to_keep = Some(act.download.temp_path.clone());
        }
        let verif = if act.download.status == DownloadStatus::Completed {
            "verified_ok"
        } else {
            "unverified"
        };
        (
            true,
            Some((
                act.download,
                act.ranges.covered_len(),
                act.connection_attempts,
                act.resume_count,
                verif,
            )),
        )
    } else {
        (false, None)
    };

    if let Some((dl, transferred, attempts, resumes, verif)) = history_data {
        // `CPartFile::Delete` de eMule → `AddCancelledFileID`: la cancelación
        // de una descarga en curso registra su hash enmascarado en
        // `m_mapCancelledFiles`. Las descargas terminadas salen del
        // ámbito (`Delete` no se aplica a los PartFile finalizados).
        if dl.status != DownloadStatus::Completed {
            cancelled_files::record_cancelled_file(db, &dl.ed2k_hash.0)?;
        }
        record_history_event(
            db,
            &dl,
            transferred,
            attempts,
            resumes,
            "removed",
            verif,
            None,
        );
    }

    let db_deleted = db
        .delete_download(id)
        .map_err(db_err("deleting download"))?;

    if let Some(temp_path) = temp_path_to_delete {
        if temp_path.exists() {
            let _ = std::fs::remove_file(&temp_path);
        }
        super::part_met::delete_part_met(&temp_path);
    }

    // Archivo final compartido: eliminado de la carpeta de destino y retirado
    // de `shared_files` (registrado automáticamente al completarse —
    // `finalize_download`), de lo contrario la lista de compartidos apuntaría a un
    // archivo inexistente.
    if let Some((target_path, hash_hex)) = completed_file_to_delete {
        if target_path.exists() {
            let _ = std::fs::remove_file(&target_path);
        }
        let _ = db.delete_shared_file(&hash_hex);
    }

    // Archivos conservados en disco tras la eliminación: renombrar el met complementario a
    // `.part.met.keep` para que el escaneo de huérfanos al inicio no resucite la
    // descarga que el usuario eliminó deliberadamente. El par sigue siendo importable mediante
    // `downloads.importPartMet` (partPath explícito).
    if let Some(temp_path) = temp_path_to_keep {
        let met_path = super::part_met::part_met_path_for(&temp_path);
        if met_path.exists() {
            let mut keep_name = met_path.as_os_str().to_os_string();
            keep_name.push(".keep");
            let keep_path = PathBuf::from(keep_name);
            let _ = std::fs::remove_file(&keep_path);
            let _ = std::fs::rename(&met_path, &keep_path);
        }
    }

    if was_active || db_deleted {
        Ok(())
    } else {
        Err(DaemonError::NotFound(format!("Download {id} not found")))
    }
}

pub fn inject_range(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
    offset: u64,
    data: &[u8],
    sender: Option<&str>,
) -> Result<DownloadSnapshot, DaemonError> {
    inject_range_impl(db, active, id, offset, data, sender, true)
}

/// Variante de ruta crítica de [`inject_range`]: misma ingestión pero la instantánea
/// devuelta no incluye el resumen por parte (`parts` vacío —
/// `compute_part_summaries` es O(partes × rangos), costoso por bloque de
/// 180 KiB en archivos grandes). No emitir el retorno hacia los
/// clientes RPC/WS — el llamador solo lee `status`/`progress`.
pub(crate) fn inject_range_brief(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
    offset: u64,
    data: &[u8],
    sender: Option<&str>,
) -> Result<DownloadSnapshot, DaemonError> {
    inject_range_impl(db, active, id, offset, data, sender, false)
}

/// Inspecciona la cabecera binaria de la parte 0 recién verificada por MD4,
/// leyendo primero desde el búfer en RAM (`staged_parts`, cero syscalls) y
/// recurriendo a `PartFile` solo si la parte no estaba preparada en memoria.
fn inspect_verified_part0_header(
    act: &mut ActiveDownload,
    part_len: usize,
) -> Result<bool, mule_network_policy::ContentPolicyError> {
    let inspect_len = mule_network_policy::content_policy::HEADER_INSPECT_BYTES.min(part_len);
    if let Some(staged) = act.staged_parts.get(&0) {
        if let Some(slice) = staged.prefix_slice(inspect_len) {
            mule_network_policy::content_policy::validate_video_header(
                slice,
                &act.download.display_name,
            )?;
            return Ok(true);
        }
    }
    if let Some(pf) = act.part_file.as_mut() {
        if let Ok(bytes) = pf.read_range(0, inspect_len) {
            mule_network_policy::content_policy::validate_video_header(
                &bytes,
                &act.download.display_name,
            )?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// Aborta una descarga rechazada por la política de contenido: cierra y borra sus
/// archivos temporales, registra el hash en `cancelled_files` y la marca como fallida.
fn reject_download_by_content_policy(
    db: &mut Database,
    act: &mut ActiveDownload,
    id: &Uuid,
    reason: &str,
) {
    let _ = act.part_file.take();
    if act.download.temp_path.exists() {
        let _ = std::fs::remove_file(&act.download.temp_path);
    }
    super::part_met::delete_part_met(&act.download.temp_path);
    let _ = cancelled_files::record_cancelled_file(db, &act.download.ed2k_hash.0);

    let now = now_ms();
    act.download.status = DownloadStatus::Failed;
    act.download.updated_at = now;
    act.last_status_change_unix_ms = now;
    act.last_error = Some(reason.to_string());
    act.ranges = RangeSet::new();
    act.staged_parts.clear();
    act.persist_dirty = false;
    act.verification_in_progress = false;

    let _ = db.update_status(id, DownloadStatus::Failed);
    let _ = db.sync_received_ranges(id, &act.ranges);
    record_history_event(
        db,
        &act.download,
        0,
        act.connection_attempts,
        act.resume_count,
        "failed",
        "content_policy_rejected",
        Some(reason.to_string()),
    );
}

fn inject_range_impl(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
    offset: u64,
    data: &[u8],
    sender: Option<&str>,
    full_snapshot: bool,
) -> Result<DownloadSnapshot, DaemonError> {
    let snap_of = |act: &ActiveDownload| {
        if full_snapshot {
            act.snapshot()
        } else {
            act.snapshot_brief()
        }
    };
    let act = active
        .get_mut(id)
        .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;

    if act.download.status == DownloadStatus::Completed {
        tracing::debug!(
            "Download {id} is already completed; ignoring incoming range {offset}..{}",
            offset + data.len() as u64
        );
        return Ok(snap_of(act));
    }
    if act.download.status == DownloadStatus::Verifying {
        tracing::debug!(
            "Download {id} is currently verifying; ignoring incoming range {offset}..{}",
            offset + data.len() as u64
        );
        return Ok(snap_of(act));
    }
    if act.download.status == DownloadStatus::Removed {
        tracing::debug!(
            "Download {id} is removed; ignoring incoming range {offset}..{}",
            offset + data.len() as u64
        );
        return Ok(snap_of(act));
    }
    if data.is_empty() {
        return Ok(snap_of(act));
    }

    let end = offset + data.len() as u64;

    // `WriteToBuffer` de eMule: cualquier dato que toque una parte ya completa
    // (sin hueco, por tanto ya procesada por hash o pendiente de verificación) se ignora —
    // « Received data touches already hashed chunk - ignored ». Sin esta
    // guarda, un bloque en vuelo planificado antes de completar la parte
    // sobrescribiría silenciosamente datos ya verificados.
    let first_part = offset / ED2K_PART_SIZE_U64;
    let last_part = (end - 1) / ED2K_PART_SIZE_U64;
    for p in first_part..=last_part {
        let ps = p * ED2K_PART_SIZE_U64;
        let pe = ((p + 1) * ED2K_PART_SIZE_U64).min(act.download.total_size);
        if pe > ps && act.ranges.contains(ps..pe) {
            tracing::warn!(
                "Download {id}: received data {offset}..{end} touches already complete part {p} — ignored (sender: {sender:?})"
            );
            return Ok(snap_of(act));
        }
    }

    // `m_CorruptionBlackBox.ReceivedData` de eMule: procedencia del bloque por parte.
    if let Some(sender) = sender {
        let mut s = offset;
        while s < end {
            let p = s / ED2K_PART_SIZE_U64;
            let pe = ((p + 1) * ED2K_PART_SIZE_U64).min(end);
            act.corruption_blackbox
                .received_data(p as u32, s, pe, sender);
            s = pe;
        }
    }

    // Volver a abrir el archivo .part cuando fue cerrado (fallo de apertura al inicio,
    // expulsión tras verificación): los bytes recibidos deben llegar al disco — el rango
    // solo se registra una vez que la escritura tuvo éxito, nunca como cobertura fantasma.
    if act.part_file.is_none() {
        act.part_file = Some(
            PartFile::open_or_create(
                &act.download.temp_path,
                act.download.total_size,
                AllocationPolicy::None,
            )
            .map_err(|e| DaemonError::Business(format!("Cannot open .part file: {e}")))?,
        );
    }
    if let Some(pf) = &mut act.part_file {
        pf.write_segment(offset, data)
            .map_err(|e| DaemonError::Business(format!("Write error: {e}")))?;
    }
    // Fsync diferido: `sync_data` se procesa en lotes mediante `persist_pending_ranges`
    // (cadencia `PART_PERSIST_INTERVAL` + puntos críticos) — sin fsync
    // por bloque de 180 KiB. La sincronización SQLite de rangos más abajo se mantiene por
    // bloque mediante un delta `INSERT` O(1) (contrato de reanudación tras fallos).
    act.persist_dirty = true;

    // Preparación en RAM (`m_tData` de eMule, libtorrent staging → hash → vaciado):
    // los bytes inyectados alimentan el búfer de su parte — la verificación
    // MD4 al completarse se hará en memoria, sin relectura de disco.
    super::staged::stage_range(
        &mut act.staged_parts,
        act.download.total_size,
        ED2K_PART_SIZE_U64,
        offset,
        data,
    );

    let was_complete =
        act.download.total_size > 0 && act.ranges.covered_len() == act.download.total_size;

    let part_idx = offset / ED2K_PART_SIZE_U64;
    let part_start = part_idx * ED2K_PART_SIZE_U64;
    let part_end = ((part_idx + 1) * ED2K_PART_SIZE_U64).min(act.download.total_size);
    let part_was_complete = part_end > part_start && act.ranges.contains(part_start..part_end);

    act.ranges
        .insert(offset..end)
        .map_err(|e| DaemonError::Business(format!("Range error: {e}")))?;

    let part_is_complete = part_end > part_start && act.ranges.contains(part_start..part_end);
    let mut met_dirty = false;
    if part_is_complete && !part_was_complete {
        met_dirty = true;
        let total_parts = act.download.total_size.div_ceil(ED2K_PART_SIZE_U64).max(1);
        let progress =
            (act.ranges.covered_len() as f64 / act.download.total_size.max(1) as f64) * 100.0;
        tracing::info!(
            "[PART_COMPLETED] File '{}' (hash: {}): Part {}/{} (offset {}..{}) downloaded & written to disk. Progress: {:.1}%",
            act.download.display_name,
            act.download.ed2k_hash.to_hex(),
            part_idx + 1,
            total_parts,
            part_start,
            part_end,
            progress
        );

        // Verificación de integridad incremental incondicional: eMule ejecuta
        // `HashSinglePart` (MD4 + AICH) en cada parte completada,
        // independientemente de la preferencia ICH. `ich_enabled` solo condiciona la
        // elección entre recuperación AICH por bloques y expulsión completa de la
        // parte (`IsCorruptedPart && IsICHEnabled` en FlushBuffer).
        let ich_enabled = db
            .get_setting("ich_enabled")
            .ok()
            .flatten()
            .map(|v| !matches!(v.trim().to_lowercase().as_str(), "0" | "false" | "off"))
            .unwrap_or(true);
        let hash_hex = act.download.ed2k_hash.to_hex().to_lowercase();
        if let Ok(part_hashes) = db.get_part_hashes(&hash_hex) {
            // Para archivos estrictamente menores que una parte, el hash del archivo ES el
            // hash de la parte. Para múltiplos exactos de PART_SIZE, eMule añade una
            // parte final MD4(empty), por lo que el hash de la parte 0 != hash del archivo allí —
            // nunca recurrir al hash del archivo en ese caso.
            let expected = part_hashes.get(part_idx as usize).copied().or({
                if part_hashes.is_empty() && act.download.total_size < ED2K_PART_SIZE_U64 {
                    Some(act.download.ed2k_hash.0)
                } else {
                    None
                }
            });
            if let Some(expected_hash) = expected {
                let part_len = (part_end - part_start) as usize;
                // Hash desde el búfer preparado en RAM (cero `pread`) cuando
                // la parte está completa en él; relectura de disco como respaldo
                // (reanudación a media parte, límite `MAX_STAGED_PARTS` alcanzado).
                let verified = match super::staged::verify_staged(
                    &mut act.staged_parts,
                    part_idx as u32,
                    part_end - part_start,
                    &expected_hash,
                ) {
                    Some(v) => v,
                    None => act
                        .part_file
                        .as_mut()
                        .and_then(|pf| pf.read_range(part_start, part_len).ok())
                        .map(|bytes| mule_core::verify_part_hash(&bytes, &expected_hash))
                        .unwrap_or(false),
                };
                if verified {
                    if part_idx == 0 && !act.content_header_verified {
                        match inspect_verified_part0_header(act, part_len) {
                            Ok(true) => {
                                act.content_header_verified = true;
                            }
                            Ok(false) => {}
                            Err(policy_err) => {
                                let reason = policy_err.to_string();
                                tracing::warn!(
                                    "[CONTENT_POLICY] Descarga eD2K '{}' ({}) rechazada en vuelo en parte 0: {}",
                                    act.download.display_name,
                                    hash_hex,
                                    reason
                                );
                                reject_download_by_content_policy(db, act, id, &reason);
                                return Ok(snap_of(act));
                            }
                        }
                    }
                    let _ = db.mark_part_verified(id, part_idx as u32);
                    act.corruption_blackbox
                        .verified_data(part_idx as u32, part_start, part_end);
                } else {
                    // Ruta AICH de eMule: con un hash maestro de confianza la parte corrupta
                    // se conserva y se pone en cola un OP_AICHREQUEST para que solo los
                    // bloques corruptos de 180 KiB se vuelvan a descargar. Recae en la expulsión
                    // de la parte completa cuando AICH no está disponible/agotado,
                    // o cuando ICH está desactivado (redescarga completa).
                    let attempts = act
                        .aich_requested
                        .get(&(part_idx as u32))
                        .map(|&(_, c)| c)
                        .unwrap_or(0);
                    let already_pending =
                        act.aich_pending.iter().any(|&(p, _)| p == part_idx as u32);
                    if ich_enabled
                        && act.download.aich_root.is_some()
                        && attempts < AICH_MAX_ATTEMPTS
                        && !already_pending
                    {
                        tracing::warn!(
                        "[PART_CORRUPTED] File '{}' (hash: {}): part {} failed MD4 check, queued for AICH block recovery",
                        act.download.display_name,
                        hash_hex,
                        part_idx
                    );
                        act.aich_pending
                            .push_back((part_idx as u32, Instant::now()));
                    } else {
                        // Atribución a los remitentes solo en ausencia de AICH
                        mark_part_corrupted(act, part_idx as u32, part_start, part_end);
                        tracing::warn!(
                        "[PART_CORRUPTED] File '{}' (hash: {}): part {} failed MD4 check, evicting range {}..{} for re-download",
                        act.download.display_name,
                        hash_hex,
                        part_idx,
                        part_start,
                        part_end
                    );
                        let _ = act.ranges.remove(part_start..part_end);
                        let _ = db.unmark_part_verified(id, part_idx as u32);
                    }
                }
            }
        }
        // Búfer preparado liberado al completarse la parte, ya esté
        // verificada, recuperada por AICH o expulsada (`release_pieces_owned_by`
        // de libtorrent — una parte reencolada vuelve a empezar de cero).
        act.staged_parts.remove(&(part_idx as u32));
    }

    // Contabilidad de rangos: delta `INSERT` O(1) por bloque (upsert,
    // las líneas solapadas se refusionan en la carga) — requerido para
    // la reanudación tras fallos. La reescritura completa `sync_received_ranges`
    // sirve de compactación a cadencia `PART_PERSIST_INTERVAL` y en
    // expulsiones (eliminación de rangos).
    db.append_received_range(id, offset, end)
        .map_err(|e| DaemonError::Business(format!("DB sync error: {e}")))?;

    let is_now_complete =
        act.download.total_size > 0 && act.ranges.covered_len() == act.download.total_size;

    // Fsync por lotes de la ruta de recepción (`FlushBuffer` periódico
    // de eMule): `sync_data` del `.part` a cadencia `PART_PERSIST_INTERVAL`,
    // forzado al completar una parte y al final del archivo — las
    // expulsiones de partes corruptas pasan por `met_dirty`, por tanto siempre
    // vaciadas de inmediato. Debe preceder a `part_file.take()`: al
    // completarse el descriptor se cierra y el fsync se volvería imposible.
    if met_dirty || is_now_complete || act.last_persist_flush.elapsed() >= PART_PERSIST_INTERVAL {
        persist_pending_ranges(db, act, id)?;
    }

    if is_now_complete && !was_complete {
        if let Some(pf) = act.part_file.take() {
            // Balance de E/S del conducto de disco — invariante objetivo: 0 `pread`
            // en recepción (todas las verificaciones pasaron por la preparación en
            // RAM), `pwrite_bytes == total_size`.
            let s = pf.io_stats();
            tracing::info!(
                "[IO_STATS] '{}' : {} writes/{} o écrits, {} reads/{} o relus, {} fsync",
                act.download.display_name,
                s.write_ops,
                s.write_bytes,
                s.read_ops,
                s.read_bytes,
                s.sync_ops
            );
        }
        act.staged_parts.clear();
        act.download.status = DownloadStatus::Verifying;
        act.download.updated_at = now_ms();
        let _ = db.update_status(id, DownloadStatus::Verifying);
        met_dirty = true;
    } else if matches!(
        act.download.status,
        DownloadStatus::Queued | DownloadStatus::WaitingForSources
    ) {
        // Solo una descarga pendiente pasa a Downloading al recibir datos.
        // Paused/Failed mantienen su estado: eMule termina las escrituras en vuelo
        // sin reanudar el partfile.
        act.download.status = DownloadStatus::Downloading;
        act.download.updated_at = now_ms();
        let _ = db.update_status(id, act.download.status);
    }

    // Persistir .part.met al completar parte / expulsión por corrupción
    // (SavePartFile de eMule en transiciones de estado).
    if met_dirty {
        let _ = super::part_met::save_part_met(db, act);
    }

    Ok(snap_of(act))
}

/// Punto único de vaciado por lotes de la ruta de recepción: `sync_data` del
/// `.part` cuando `persist_dirty`, seguido de una reescritura completa
/// `sync_received_ranges` — función de **compactación** (delimita la tabla y
/// absorbe los deltas) y de red de seguridad tras expulsión. Invocado
/// por `inject_range_impl` (cadencia [`PART_PERSIST_INTERVAL`], finalización
/// de parte, fin de archivo), por transiciones de estado (`pause`,
/// `update_status`), por el respaldo `flush_dirty_downloads` y por
/// `save_active_part_mets` en el apagado — evita un fsync por bloque de 180 KiB.
pub(crate) fn persist_pending_ranges(
    db: &mut Database,
    act: &mut ActiveDownload,
    id: &Uuid,
) -> Result<(), DaemonError> {
    if !act.persist_dirty {
        return Ok(());
    }
    if let Some(pf) = &mut act.part_file {
        pf.flush()
            .map_err(|e| DaemonError::Business(format!("Flush error: {e}")))?;
    }
    db.sync_received_ranges(id, &act.ranges)
        .map_err(|e| DaemonError::Business(format!("DB sync error: {e}")))?;
    act.persist_dirty = false;
    act.last_persist_flush = Instant::now();
    Ok(())
}

/// Marca los registros de la parte como corruptos y luego evalúa la
/// responsabilidad de los remitentes (`CorruptedData` + `EvaluateData` de eMule):
/// todo remitente que supere `CBB_BANTHRESHOLD` se apila en
/// `pending_corruption_bans` para su desconexión/rechazo por el llamador.
fn mark_part_corrupted(
    act: &mut ActiveDownload,
    part_idx: u32,
    part_start: u64,
    part_end: u64,
) {
    act.corruption_blackbox
        .corrupted_data(part_idx, part_start, part_end);
    for guilty in act.corruption_blackbox.evaluate(part_idx) {
        if !act.pending_corruption_bans.contains(&guilty) {
            tracing::warn!(
                "[CORRUPTION_BLACKBOX] '{}' : {guilty} a dépassé le seuil de corruption (eMule CBB_BANTHRESHOLD) — bannissement demandé",
                act.download.display_name
            );
            act.pending_corruption_bans.push(guilty);
        }
    }
}

/// Expulsa los rangos de partes corruptas del RangeSet recibido para que se vuelvan a descargar,
/// y restablece la descarga a Downloading. Refleja el manejo de partes corruptas de
/// `CPartFile` en eMule tras un fallo de comprobación del hashset.
pub fn evict_corrupted_parts(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
    corrupted: &[u32],
) -> Result<DownloadSnapshot, DaemonError> {
    let act = active
        .get_mut(id)
        .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;

    const ED2K_PART_SIZE: u64 = ED2K_PART_SIZE_U64;
    let total_size = act.download.total_size;

    for &part_idx in corrupted {
        let start = part_idx as u64 * ED2K_PART_SIZE;
        let end = ((part_idx as u64 + 1) * ED2K_PART_SIZE).min(total_size);
        if end > start {
            let _ = act.ranges.remove(start..end);
            mark_part_corrupted(act, part_idx, start, end);
        }
        act.staged_parts.remove(&part_idx);
        let _ = db.unmark_part_verified(id, part_idx);
    }

    db.sync_received_ranges(id, &act.ranges)
        .map_err(|e| DaemonError::Business(format!("DB sync error: {e}")))?;

    // Reabrir el archivo de partes para escrituras posteriores (se cerró al entrar en Verifying).
    if act.part_file.is_none() {
        act.part_file =
            PartFile::open_or_create(&act.download.temp_path, total_size, AllocationPolicy::None)
                .ok();
    }

    let now = now_ms();
    act.download.status = DownloadStatus::Downloading;
    act.download.updated_at = now;
    act.last_status_change_unix_ms = now;
    act.verification_in_progress = false;
    let _ = db.update_status(id, DownloadStatus::Downloading);

    // Limpiar el estado de recuperación AICH para partes expulsadas.
    for &part_idx in corrupted {
        act.aich_pending.retain(|&(p, _)| p != part_idx);
        act.aich_requested.remove(&part_idx);
    }

    tracing::warn!(
        "[CORRUPTION_RECOVERY] Download {id} ('{}'): {} corrupted part(s) evicted, back to Downloading ({:.1}% remaining)",
        act.download.display_name,
        corrupted.len(),
        (1.0 - act.ranges.covered_len() as f64 / total_size.max(1) as f64) * 100.0
    );

    let _ = super::part_met::save_part_met(db, act);
    Ok(act.snapshot())
}

// ── Recuperación AICH (semántica CAICHRecoveryHashSet de eMule) ─────────────────

/// IPs enmascaradas únicas mínimas (`ip & 0x00F0FFFF`) requeridas para confiar en una raíz.
pub const AICH_MIN_UNIQUE_IPS: usize = 10;
/// Proporción mínima de IPs coincidentes para confiar en una raíz (porcentaje).
pub const AICH_MIN_PERCENTAGE: usize = 92;
/// Solicitudes máximas de recuperación AICH enviadas por parte corrupta antes de recurrir
/// a la expulsión de la parte completa.
pub const AICH_MAX_ATTEMPTS: u32 = 2;
/// Una parte pendiente de recuperación AICH durante más tiempo que este se expulsa por completo
/// (protege contra bloqueos cuando ninguna fuente compatible con AICH responde).
pub const AICH_PENDING_TIMEOUT: Duration = Duration::from_secs(120);
/// Retardo de nueva solicitud antes de que una parte ya solicitada pueda pedirse de nuevo.
const AICH_REQUEST_COOLDOWN: Duration = Duration::from_secs(60);

/// Resultado del registro de un voto de hash maestro AICH recibido de un par.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AichVoteOutcome {
    /// La raíz es de confianza (ya conocida, o consenso alcanzado y persistido).
    Trusted,
    /// Voto registrado pero consenso aún no alcanzado.
    Untrusted,
    /// El par notificó una raíz distinta de la de confianza — fuente muerta.
    Mismatch,
}

/// Registra un hash maestro AICH recibido de un par para una descarga.
/// `masked_ip` debe ser `ip_u32 & 0x00F0FFFF` (CAICHUntrustedHash de eMule).
/// Cuando se alcanza el consenso, la raíz se persiste en el registro de la descarga.
pub fn record_aich_root_vote(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
    root: [u8; 20],
    masked_ip: u32,
) -> AichVoteOutcome {
    let act = match active.get_mut(id) {
        Some(a) => a,
        None => return AichVoteOutcome::Mismatch,
    };

    if let Some(trusted) = &act.download.aich_root {
        return if trusted.0 == root {
            AichVoteOutcome::Trusted
        } else {
            AichVoteOutcome::Mismatch
        };
    }

    act.aich_votes.entry(root).or_default().insert(masked_ip);

    // Buscar el candidato más firmado (UntrustedHashReceived de eMule).
    let total_ips: usize = act.aich_votes.values().map(|s| s.len()).sum();
    let (best_root, best_ips) = act
        .aich_votes
        .iter()
        .max_by_key(|(_, ips)| ips.len())
        .map(|(r, ips)| (*r, ips.len()))
        .unwrap_or((root, 0));

    if best_ips >= AICH_MIN_UNIQUE_IPS && (100 * best_ips) / total_ips.max(1) >= AICH_MIN_PERCENTAGE
    {
        act.download.aich_root = Some(mule_format::AichHash(best_root));
        let _ = db.update_aich_root(id, &best_root);
        tracing::info!(
            "[AICH] Master hash for '{}' trusted by consensus ({best_ips}/{total_ips} unique IPs)",
            act.download.display_name
        );
        AichVoteOutcome::Trusted
    } else {
        AichVoteOutcome::Untrusted
    }
}

/// Devuelve la raíz AICH de confianza para una descarga, si existe.
pub fn trusted_aich_root(active: &HashMap<Uuid, ActiveDownload>, id: &Uuid) -> Option<[u8; 20]> {
    active.get(id)?.download.aich_root.map(|a| a.0)
}

/// Extrae la siguiente parte corrupta en espera de datos de recuperación AICH. Las partes cuyo
/// estado pendiente haya caducado se expulsan por completo (respaldo para que las descargas
/// no se queden atascadas si ninguna fuente compatible con AICH responde).
pub fn next_aich_recovery(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
) -> Option<(u32, [u8; 20])> {
    let act = active.get_mut(id)?;
    let master = act.download.aich_root.map(|a| a.0)?;

    let mut deferred = std::collections::VecDeque::new();
    let mut result = None;
    while let Some((part, enqueued)) = act.aich_pending.pop_front() {
        if enqueued.elapsed() > AICH_PENDING_TIMEOUT {
            tracing::warn!(
                "[AICH] Recovery for part {part} of '{}' expired, evicting whole part",
                act.download.display_name
            );
            let start = part as u64 * ED2K_PART_SIZE_U64;
            let end = ((part as u64 + 1) * ED2K_PART_SIZE_U64).min(act.download.total_size);
            if end > start {
                let _ = act.ranges.remove(start..end);
                mark_part_corrupted(act, part, start, end);
            }
            act.staged_parts.remove(&part);
            let _ = db.unmark_part_verified(id, part);
            continue;
        }
        let recent = act
            .aich_requested
            .get(&part)
            .map(|&(ts, _)| ts.elapsed() < AICH_REQUEST_COOLDOWN)
            .unwrap_or(false);
        if recent {
            deferred.push_back((part, enqueued));
            continue;
        }
        let count = act.aich_requested.get(&part).map(|&(_, c)| c).unwrap_or(0);
        act.aich_requested.insert(part, (Instant::now(), count + 1));
        result = Some((part, master));
        break;
    }
    act.aich_pending.extend(deferred);
    let _ = db.sync_received_ranges(id, &act.ranges);
    result
}

/// Marca una solicitud de recuperación AICH pendiente como fallida (el par no pudo servirla):
/// vuelve a poner en cola la parte, o la expulsa por completo una vez agotados los intentos.
pub fn fail_aich_recovery(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
    part: u32,
) {
    let Some(act) = active.get_mut(id) else {
        return;
    };
    let attempts = act.aich_requested.get(&part).map(|&(_, c)| c).unwrap_or(0);
    if attempts >= AICH_MAX_ATTEMPTS {
        tracing::warn!(
            "[AICH] Recovery for part {part} of '{}' failed {attempts} times, evicting whole part",
            act.download.display_name
        );
        evict_part_range(db, act, id, part);
    } else {
        act.aich_pending.push_back((part, Instant::now()));
    }
}

/// Informe producido tras aplicar los datos de recuperación AICH recibidos.
#[derive(Debug)]
pub struct AichRecoveryReport {
    pub part: u32,
    pub corrupted_blocks: u32,
    pub evicted_ranges: Vec<(u64, u64)>,
}

fn evict_part_range(db: &mut Database, act: &mut ActiveDownload, id: &Uuid, part: u32) {
    let start = part as u64 * ED2K_PART_SIZE_U64;
    let end = ((part as u64 + 1) * ED2K_PART_SIZE_U64).min(act.download.total_size);
    if end > start {
        let _ = act.ranges.remove(start..end);
        mark_part_corrupted(act, part, start, end);
    }
    act.staged_parts.remove(&part);
    let _ = db.unmark_part_verified(id, part);
    let _ = db.sync_received_ranges(id, &act.ranges);
    if act.part_file.is_none() {
        act.part_file = PartFile::open_or_create(
            &act.download.temp_path,
            act.download.total_size,
            AllocationPolicy::None,
        )
        .ok();
    }
    act.download.status = DownloadStatus::Downloading;
    act.download.updated_at = now_ms();
    let _ = db.update_status(id, DownloadStatus::Downloading);
}

/// Aplica los datos de recuperación AICH recibidos para una parte corrupta: verifica el árbol
/// de hash contra el hash maestro de confianza, luego procesa el hash de cada bloque de 180 KiB
/// de la parte descargada y expulsa únicamente los rangos de bloques corruptos.
pub fn apply_aich_recovery(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
    part: u32,
    master_hash: [u8; 20],
    entries: Vec<mule_format::AichRecoveryEntry>,
) -> Result<AichRecoveryReport, DaemonError> {
    let act = active
        .get_mut(id)
        .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;

    let trusted = act.download.aich_root.map(|a| a.0);
    if trusted != Some(master_hash) {
        tracing::warn!(
            "[AICH] Recovery answer for part {part} of '{}' carries an untrusted master hash, ignoring",
            act.download.display_name
        );
        return Err(DaemonError::Business(
            "untrusted AICH master hash".to_string(),
        ));
    }

    let block_hashes = match mule_format::read_recovery_data(
        act.download.total_size,
        part,
        mule_format::AichHash(master_hash),
        &entries,
    ) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(
                "[AICH] Invalid recovery data for part {part} of '{}': {e}; evicting whole part",
                act.download.display_name
            );
            act.aich_requested.remove(&part);
            evict_part_range(db, act, id, part);
            return Ok(AichRecoveryReport {
                part,
                corrupted_blocks: u32::MAX,
                evicted_ranges: vec![],
            });
        }
    };

    // El archivo de partes puede haberse cerrado cuando la descarga entró en Verifying;
    // reabrirlo para que se puedan calcular los hashes de los bloques.
    if act.part_file.is_none() {
        act.part_file = PartFile::open_or_create(
            &act.download.temp_path,
            act.download.total_size,
            AllocationPolicy::None,
        )
        .ok();
    }

    let part_start = part as u64 * 9_728_000;
    let part_end = ((part as u64 + 1) * 9_728_000).min(act.download.total_size);
    let mut corrupted_blocks = 0u32;
    let mut evicted_ranges = Vec::new();

    for (i, expected) in block_hashes.iter().enumerate() {
        let block_start = part_start + i as u64 * 184_320;
        let block_end = (block_start + 184_320).min(part_end);
        if block_end <= block_start {
            continue;
        }
        let ok = act
            .part_file
            .as_mut()
            .and_then(|pf| {
                pf.read_range(block_start, (block_end - block_start) as usize)
                    .ok()
            })
            .map(|bytes| mule_format::aich_block_hash(&bytes) == *expected)
            .unwrap_or(false);
        if ok {
            // `m_CorruptionBlackBox.VerifiedData` de eMule sobre el bloque recuperado.
            act.corruption_blackbox
                .verified_data(part, block_start, block_end);
        } else {
            // `CorruptedData` de eMule: atribución del bloque defectuoso al
            // remitente antes de evaluar las expulsiones.
            act.corruption_blackbox
                .corrupted_data(part, block_start, block_end);
            corrupted_blocks += 1;
            evicted_ranges.push((block_start, block_end));
            let _ = act.ranges.remove(block_start..block_end);
        }
    }
    // `m_CorruptionBlackBox.EvaluateData` de eMule tras el barrido de bloques.
    for guilty in act.corruption_blackbox.evaluate(part) {
        if !act.pending_corruption_bans.contains(&guilty) {
            tracing::warn!(
                "[CORRUPTION_BLACKBOX] '{}' : {guilty} a dépassé le seuil de corruption — bannissement demandé",
                act.download.display_name
            );
            act.pending_corruption_bans.push(guilty);
        }
    }

    act.aich_pending.retain(|&(p, _)| p != part);
    act.aich_requested.remove(&part);
    act.staged_parts.remove(&part);

    if corrupted_blocks == 0 {
        let _ = db.mark_part_verified(id, part);
        tracing::info!(
            "[AICH] Part {part} of '{}' fully verified by recovery data",
            act.download.display_name
        );
    } else {
        let _ = db.unmark_part_verified(id, part);
        if act.part_file.is_none() {
            act.part_file = PartFile::open_or_create(
                &act.download.temp_path,
                act.download.total_size,
                AllocationPolicy::None,
            )
            .ok();
        }
        if act.download.status == DownloadStatus::Verifying {
            act.download.status = DownloadStatus::Downloading;
            act.download.updated_at = now_ms();
            let _ = db.update_status(id, DownloadStatus::Downloading);
        }
        tracing::warn!(
            "[AICH] Part {part} of '{}': {corrupted_blocks} corrupted block(s) evicted for re-download",
            act.download.display_name
        );
    }

    db.sync_received_ranges(id, &act.ranges)
        .map_err(|e| DaemonError::Business(format!("DB sync error: {e}")))?;

    let _ = super::part_met::save_part_met(db, act);
    Ok(AichRecoveryReport {
        part,
        corrupted_blocks,
        evicted_ranges,
    })
}

pub fn update_status(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
    status: DownloadStatus,
) -> Result<DownloadSnapshot, DaemonError> {
    let (old_status, dl, transferred, attempts, resumes, last_error) = {
        let act = active
            .get_mut(id)
            .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;

        let old = act.download.status;
        act.download.status = status;
        act.download.updated_at = now_ms();
        if status != DownloadStatus::Verifying {
            act.verification_in_progress = false;
        }
        persist_pending_ranges(db, act, id)?;
        (
            old,
            act.download.clone(),
            act.ranges.covered_len(),
            act.connection_attempts,
            act.resume_count,
            act.last_error.clone(),
        )
    };

    db.update_status(id, status)
        .map_err(db_err("persisting state"))?;

    if status == DownloadStatus::Failed {
        let verif = if old_status == DownloadStatus::Verifying {
            "hash_mismatch"
        } else {
            "unverified"
        };
        record_history_event(
            db,
            &dl,
            transferred,
            attempts,
            resumes,
            "failed",
            verif,
            last_error,
        );
    } else if status == DownloadStatus::Verifying {
        record_history_event(
            db,
            &dl,
            transferred,
            attempts,
            resumes,
            "verified_pending_promotion",
            "unverified",
            None,
        );
    }

    let act = active
        .get(id)
        .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;
    Ok(act.snapshot())
}

pub fn prepare_finalize(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
) -> Result<(PathBuf, PathBuf, DownloadSnapshot), DaemonError> {
    let (temp_path, display_name, already_verified) = {
        let act = active
            .get_mut(id)
            .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;
        let _ = act.part_file.take();
        (
            act.download.temp_path.clone(),
            act.download.display_name.clone(),
            act.content_header_verified,
        )
    };

    if !already_verified {
        if let Err(policy_err) =
            mule_network_policy::content_policy::validate_video_file(&temp_path, &display_name)
        {
            let reason = policy_err.to_string();
            tracing::warn!(
                "[CONTENT_POLICY] Descarga eD2K '{}' ({id}) rechazada antes de finalizar: {}",
                display_name,
                reason
            );
            let act = active
                .get_mut(id)
                .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;
            reject_download_by_content_policy(db, act, id, &reason);
            return Err(DaemonError::Business(reason));
        }
    }

    let act = active
        .get_mut(id)
        .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;
    act.content_header_verified = true;
    act.download.status = DownloadStatus::Completing;
    let now = now_ms();
    act.download.updated_at = now;
    act.last_status_change_unix_ms = now;
    let _ = db.update_status(id, DownloadStatus::Completing);
    let target_path = act.download.target_path.clone();
    Ok((temp_path, target_path, act.snapshot()))
}

pub fn commit_finalize(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
    final_path: PathBuf,
) -> Result<DownloadSnapshot, DaemonError> {
    let (temp_path, dl, transferred, attempts, resumes) = {
        let act = active
            .get_mut(id)
            .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;
        (
            act.download.temp_path.clone(),
            act.download.clone(),
            act.ranges.covered_len(),
            act.connection_attempts,
            act.resume_count,
        )
    };

    // eMule elimina el archivo complementario .part.met una vez que el archivo se ha completado.
    super::part_met::delete_part_met(&temp_path);

    let now = now_ms();
    {
        let act = active
            .get_mut(id)
            .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;
        act.download.status = DownloadStatus::Completed;
        act.download.target_path = final_path.clone();
        // `m_fullname` de eMule sigue el nombre renombrado; el nombre mostrado refleja
        // el archivo realmente generado en disco.
        if let Some(name) = final_path.file_name().and_then(|n| n.to_str()) {
            act.download.display_name = name.to_string();
        }
        act.verification_in_progress = false;
        act.download.updated_at = now;
        act.last_status_change_unix_ms = now;
        act.current_rate_bps = 0;
        let _ = act.part_file.take();
        // La caja negra de corrupción se libera al completarse el archivo.
        act.corruption_blackbox.clear();
    }
    db.update_status(id, DownloadStatus::Completed)
        .map_err(db_err("persisting state"))?;
    db.update_target_path(id, &final_path)
        .map_err(db_err("persisting final path"))?;

    let mut finalized_dl = dl;
    finalized_dl.status = DownloadStatus::Completed;
    finalized_dl.target_path = final_path.clone();
    if let Some(name) = final_path.file_name().and_then(|n| n.to_str()) {
        finalized_dl.display_name = name.to_string();
    }
    record_history_event(
        db,
        &finalized_dl,
        transferred,
        attempts,
        resumes,
        "completed",
        "verified_ok",
        None,
    );

    let mut shared = mule_core::model::SharedFile::new(
        finalized_dl.ed2k_hash.to_hex(),
        &finalized_dl.display_name,
        finalized_dl.total_size,
        final_path.to_string_lossy().as_ref(),
    );
    // `UAPPref` de eMule: el archivo completado se convierte en un compartido «auto».
    if peers_servers::auto_up_priority_enabled(db) {
        shared.up_priority = "auto".to_string();
    }
    let _ = db.upsert_shared_file(&shared);

    // Disparar gancho opcional on_download_complete configurado mediante MULE_ON_DOWNLOAD_COMPLETE
    trigger_completion_hook(&final_path, finalized_dl.category_id.as_deref());

    let act = active
        .get(id)
        .ok_or_else(|| DaemonError::Business(format!("Download {id} not found after finalize")))?;
    Ok(act.snapshot())
}

pub fn finalize_download(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
) -> Result<DownloadSnapshot, DaemonError> {
    let (temp_path, target_path, _) = prepare_finalize(db, active, id)?;

    // `PerformFileComplete` de eMule: el archivo de destino nunca se
    // sobrescribe — colisión resuelta en `name(1).ext`, `name(2).ext`, …
    let final_path = mule_core::finalize_file(
        &temp_path,
        &target_path,
        mule_core::CollisionStrategy::Rename,
    )
    .map_err(|e| DaemonError::Business(format!("Finalize error: {e}")))?;

    commit_finalize(db, active, id, final_path)
}

pub fn trigger_completion_hook(final_path: &Path, category: Option<&str>) {
    let Ok(hook_path) = std::env::var("MULE_ON_DOWNLOAD_COMPLETE") else {
        return;
    };
    if hook_path.trim().is_empty() {
        return;
    }
    let p = Path::new(&hook_path);
    if p.is_file() {
        let mut cmd = std::process::Command::new(p);
        cmd.arg(final_path);
        if let Some(cat) = category {
            cmd.arg(cat);
        }
        tracing::info!(
            "Triggering on_download_complete hook: {} for '{}' (category: {:?})",
            p.display(),
            final_path.display(),
            category
        );
        let _ = cmd.spawn();
    }
}

pub fn set_destination(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
    target_dir: &Path,
) -> Result<DownloadDetail, DaemonError> {
    let act = active
        .get_mut(id)
        .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;

    std::fs::create_dir_all(target_dir)
        .map_err(|e| DaemonError::Business(format!("Cannot create target directory: {e}")))?;

    let probe_file = target_dir.join(format!(".mule_write_probe_{}", Uuid::new_v4()));
    match std::fs::write(&probe_file, b"probe") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe_file);
        }
        Err(e) => {
            return Err(DaemonError::Business(format!(
                "Target directory is not writable: {e}"
            )));
        }
    }

    let new_target_path = target_dir.join(&act.download.display_name);
    act.download.target_path = new_target_path.clone();
    act.download.updated_at = now_ms();

    db.update_target_path(id, &new_target_path)
        .map_err(db_err("updating target path"))?;

    get_download_detail(db, active, id)
        .ok_or_else(|| DaemonError::NotFound(format!("Download {id} detail not found")))
}

/// Actualiza directamente la ruta objetivo de una descarga en la base de datos y en memoria activa.
pub fn update_target_path(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
    new_target_path: &Path,
) -> Result<(), DaemonError> {
    let mut hash_hex_opt = None;
    if let Some(act) = active.get_mut(id) {
        act.download.target_path = new_target_path.to_path_buf();
        if let Some(name) = new_target_path.file_name().and_then(|n| n.to_str()) {
            act.download.display_name = name.to_string();
        }
        act.download.updated_at = now_ms();
        hash_hex_opt = Some(act.download.ed2k_hash.to_hex());
    }
    db.update_target_path(id, new_target_path)
        .map_err(db_err("updating target path"))?;
    if let Some(hash_hex) = hash_hex_opt {
        if let Ok(Some(mut shared)) = db.get_shared_file(&hash_hex) {
            if let Some(name) = new_target_path.file_name().and_then(|n| n.to_str()) {
                shared.file_name = name.to_string();
            }
            shared.file_path = new_target_path.to_string_lossy().to_string();
            let _ = db.upsert_shared_file(&shared);
        }
    }
    Ok(())
}

/// Valida un nombre de archivo proporcionado por el usuario (`downloads.rename`,
/// `shares.rename`): no vacío, sin separadores ni caracteres prohibidos de
/// Windows, ni `.`/`..`, ni punto/espacio final (NTFS).
fn validate_new_file_name(name: &str) -> Result<&str, DaemonError> {
    let trimmed = name.trim();
    let illegal = |c: char| {
        c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
    };
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.ends_with(['.', ' '])
        || trimmed.chars().any(illegal)
    {
        return Err(DaemonError::InvalidParams(format!(
            "Invalid file name: '{name}'"
        )));
    }
    Ok(trimmed)
}

/// `downloads.rename` — renombra una descarga (nombre mostrado + ruta
/// de destino derivada). El `.part`/`.part.met` temporal conserva su
/// nombre UUID; para una descarga completada, el archivo final se renombra en
/// disco y la entrada de `shared_files` hace lo propio. (eMule: renombramiento con F2 en la
/// lista de descargas).
pub fn rename_download(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
    new_name: &str,
) -> Result<DownloadSnapshot, DaemonError> {
    let new_name = validate_new_file_name(new_name)?;
    mule_network_policy::content_policy::validate_video_filename(new_name)
        .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;

    let (old_target, new_target, completed, hash_hex) = {
        let act = active
            .get(id)
            .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;
        if act.download.display_name == new_name {
            return Ok(act.snapshot());
        }
        let parent = act
            .download
            .target_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        (
            act.download.target_path.clone(),
            parent.join(new_name),
            act.download.status == DownloadStatus::Completed,
            act.download.ed2k_hash.to_hex(),
        )
    };
    if active
        .values()
        .any(|o| o.download.id != *id && o.download.target_path == new_target)
    {
        return Err(DaemonError::Business(format!(
            "Another download already targets '{new_name}'"
        )));
    }

    // Descarga terminada: el archivo final existe en disco → renombramiento
    // real + actualización de la entrada de compartido (de lo contrario apuntaría
    // a una ruta inexistente).
    if completed && old_target != new_target && old_target.exists() {
        if new_target.exists() {
            return Err(DaemonError::Business(format!(
                "A file named '{new_name}' already exists in the destination directory"
            )));
        }
        std::fs::rename(&old_target, &new_target)
            .map_err(|e| DaemonError::Business(format!("Cannot rename file on disk: {e}")))?;
        if let Ok(Some(mut shared)) = db.get_shared_file(&hash_hex) {
            shared.file_name = new_name.to_string();
            shared.file_path = new_target.to_string_lossy().to_string();
            let _ = db.upsert_shared_file(&shared);
        }
    }

    let act = active
        .get_mut(id)
        .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;
    act.download.display_name = new_name.to_string();
    act.download.target_path = new_target.clone();
    act.download.updated_at = now_ms();
    db.update_download_name_and_target(id, new_name, &new_target)
        .map_err(db_err("renaming download"))?;
    Ok(act.snapshot())
}

/// `shares.rename` — renombra un archivo compartido completo (disco +
/// `shared_files`). Cuando el hash respalda una descarga aún listada, se
/// delega en `rename_download` para mantener ambas vistas coherentes.
pub fn rename_shared_file(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    file_hash: &str,
    new_name: &str,
) -> Result<(), DaemonError> {
    let new_name = validate_new_file_name(new_name)?;
    mule_network_policy::content_policy::validate_video_filename(new_name)
        .map_err(|e| DaemonError::InvalidParams(e.to_string()))?;

    if let Some(dl_id) = active
        .values()
        .find(|a| a.download.ed2k_hash.to_hex() == file_hash)
        .map(|a| a.download.id)
    {
        rename_download(db, active, &dl_id, new_name)?;
        return Ok(());
    }

    let mut shared = db
        .get_shared_file(file_hash)
        .map_err(db_err("persisting state"))?
        .ok_or_else(|| DaemonError::NotFound(format!("Shared file {file_hash} not found")))?;

    let old_path = PathBuf::from(&shared.file_path);
    let parent = old_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let new_path = parent.join(new_name);

    if old_path != new_path && old_path.exists() {
        if new_path.exists() {
            return Err(DaemonError::Business(format!(
                "A file named '{new_name}' already exists in the directory"
            )));
        }
        std::fs::rename(&old_path, &new_path)
            .map_err(|e| DaemonError::Business(format!("Cannot rename file on disk: {e}")))?;
    }

    shared.file_name = new_name.to_string();
    shared.file_path = new_path.to_string_lossy().to_string();
    db.upsert_shared_file(&shared)
        .map_err(db_err("persisting state"))?;
    Ok(())
}

pub fn retry_connection(
    db: &mut Database,
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
) -> Result<DownloadDetail, DaemonError> {
    let act = active
        .get_mut(id)
        .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;

    let now = now_ms();
    act.connection_attempts = act.connection_attempts.saturating_add(1);
    act.last_error = None;
    act.last_status_change_unix_ms = now;
    act.download.updated_at = now;

    if act.download.status == DownloadStatus::Failed
        || act.download.status == DownloadStatus::Paused
    {
        if act.download.status == DownloadStatus::Failed
            && act.ranges.covered_len() == act.download.total_size
        {
            act.ranges = RangeSet::new();
            act.staged_parts.clear();
            let _ = db.sync_received_ranges(id, &act.ranges);
        }
        if act.part_file.is_none() {
            act.part_file = PartFile::open_or_create(
                &act.download.temp_path,
                act.download.total_size,
                AllocationPolicy::None,
            )
            .ok();
        }
        act.download.status = DownloadStatus::Downloading;
        act.verification_in_progress = false;
        let _ = db.update_status(id, DownloadStatus::Downloading);
    }

    get_download_detail(db, active, id)
        .ok_or_else(|| DaemonError::NotFound(format!("Download {id} detail not found")))
}

pub fn record_source_availability(
    active: &mut HashMap<Uuid, ActiveDownload>,
    id: &Uuid,
    source: SocketAddr,
    mask: PartAvailabilityMask,
) -> Result<DownloadSnapshot, DaemonError> {
    let act = active
        .get_mut(id)
        .ok_or_else(|| DaemonError::NotFound(format!("Download {id} not found")))?;

    let now = now_ms();
    let source_id = format!("src_{:08x}", rand::random::<u32>());
    let entry = act.sources.entry(source).or_insert_with(|| ActiveSource {
        source_id,
        _addr: source,
        mask: mask.clone(),
        last_seen_unix_ms: now,
    });
    entry.mask = mask;
    entry.last_seen_unix_ms = now;
    act.download.updated_at = now;

    Ok(act.snapshot())
}

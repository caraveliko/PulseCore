// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Importación de `known.met` — auxiliar de migración que aplica el índice de
//! archivos compartidos de eMule (`CKnownFileList::LoadKnownFiles`) a nuestra
//! tabla `shared_files`.
//!
//! `known.met` no almacena rutas del sistema de archivos, por lo que los registros
//! se enlazan con los archivos encontrados bajo un directorio mediante
//! `(file_name, file_size)` — la única clave que ambas partes comparten. Los
//! archivos coincidentes se comparten bajo el hash de eMule; los hashes de parte,
//! la raíz AICH, la prioridad de subida y las estadísticas de transferencia se
//! importan tal como las expone `CKnownFile::LoadTagsFromFile`.
//!
//! `verify=false` confía en los registros de hash de eMule (la vía rápida de
//! migración — evitar volver a calcular el hash de todo el directorio de entrada es
//! precisamente para lo que existe esta herramienta); `verify=true` recalcula el
//! hash de cada candidato y solo importa si coincide el hash raíz, que es el propio
//! comportamiento de eMule al compartir (`CAddFileThread::Run` siempre recalcula
//! el hash).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::Mutex;

use crate::error::DaemonError;
use crate::services::downloads::DownloadService;
use crate::services::incoming_share::collect_shareable_files;

/// Resultado de una ejecución de `import_known_met`.
#[derive(Debug, Default, Serialize)]
pub struct KnownMetImportReport {
    /// Registros válidos leídos del archivo (tras verificación).
    pub records: usize,
    /// Registros que eMule descartaría (fallo en la verificación del conjunto de hashes).
    pub dropped_records: usize,
    /// Archivos compartibles encontrados bajo el directorio escaneado.
    pub scanned_files: usize,
    /// Nuevos recursos compartidos registrados a partir de registros coincidentes.
    pub imported: usize,
    /// Hashes ya compartidos enriquecidos (solo aich_root / estadísticas).
    pub enriched: usize,
    /// Candidatos con `verify=true` cuyo hash real difiere del registro.
    pub mismatched: usize,
    /// Registros que no coincidieron con ningún archivo en disco.
    pub unmatched_records: usize,
    /// Directorio que fue escaneado.
    pub dir: String,
}

/// Cadenas de `PR_*` de eMule → `SharedFile::up_priority`.
fn pr_to_up_priority(pr: u8) -> &'static str {
    match pr {
        mule_format::PR_VERYLOW => "verylow",
        mule_format::PR_LOW => "low",
        mule_format::PR_HIGH => "high",
        mule_format::PR_VERYHIGH => "veryhigh",
        _ => "normal",
    }
}

/// Importa `met_path` (`known.met` de eMule) para archivos compartibles bajo `dir`.
pub async fn import_known_met(
    downloads: &Arc<Mutex<DownloadService>>,
    met_path: PathBuf,
    dir: PathBuf,
    verify: bool,
) -> Result<KnownMetImportReport, DaemonError> {
    let met = tokio::task::spawn_blocking(move || mule_format::KnownMetFile::open(&met_path))
        .await
        .map_err(|e| DaemonError::Internal(format!("known.met read task failed: {e}")))?
        .map_err(|e| DaemonError::Business(format!("failed to parse known.met: {e}")))?;

    let walk_dir = dir.clone();
    let files = tokio::task::spawn_blocking(move || collect_shareable_files(&walk_dir))
        .await
        .map_err(|e| DaemonError::Internal(format!("directory walk failed: {e}")))?;

    // (nombre en minúsculas, tamaño) → registro — la única clave de unión disponible.
    let mut by_name_size: HashMap<(String, u64), usize> = HashMap::new();
    for (i, rec) in met.records.iter().enumerate() {
        if let (Some(name), Some(size)) = (&rec.file_name, rec.file_size) {
            by_name_size.entry((name.to_lowercase(), size)).or_insert(i);
        }
    }
    let mut matched_records: HashSet<usize> = HashSet::new();

    let mut report = KnownMetImportReport {
        records: met.records.len(),
        dropped_records: met.dropped_records,
        scanned_files: files.len(),
        dir: dir.to_string_lossy().to_string(),
        ..Default::default()
    };

    for path in files {
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            continue;
        };
        let size = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(_) => continue,
        };
        let Some(&rec_idx) = by_name_size.get(&(file_name.to_lowercase(), size)) else {
            continue;
        };
        let rec = &met.records[rec_idx];
        let hash_hex = rec.file_hash.to_hex().to_lowercase();

        if verify {
            let path_for_hash = path.clone();
            let hashed = tokio::task::spawn_blocking(move || {
                let mut f = std::fs::File::open(&path_for_hash)?;
                mule_format::ed2k::hash_reader(&mut f)
            })
            .await;
            match hashed {
                Ok(Ok(h)) if h.root_hash.as_bytes() == rec.file_hash.as_bytes() => {}
                _ => {
                    report.mismatched += 1;
                    tracing::warn!(
                        "[KNOWNMET] hash mismatch for {} — record skipped",
                        path.display()
                    );
                    continue;
                }
            }
        }
        matched_records.insert(rec_idx);

        let mut dl = downloads.lock().await;
        if let Some(mut sf) = dl.get_shared_file(&hash_hex)? {
            // Recurso compartido existente: enriquecer solamente — nunca restablecer la configuración del usuario.
            if sf.aich_root.is_none() {
                sf.aich_root = rec.aich_root.map(|a| a.0);
                dl.add_shared_file(&sf)?;
            }
            if rec.transferred_bytes > 0 {
                dl.add_share_uploaded_bytes(&hash_hex, rec.transferred_bytes)?;
            }
            report.enriched += 1;
            continue;
        }

        let mut sf = mule_core::model::SharedFile::new(
            &hash_hex,
            &file_name,
            size,
            path.to_string_lossy().as_ref(),
        );
        sf.aich_root = rec.aich_root.map(|a| a.0);
        if let Some(pr) = rec.up_priority {
            sf.up_priority = pr_to_up_priority(pr).to_string();
        } else if dl.auto_up_priority_enabled() {
            sf.up_priority = "auto".to_string();
        }
        dl.add_shared_file(&sf)?;
        if !rec.part_hashes.is_empty() {
            let parts: Vec<[u8; 16]> = rec.part_hashes.iter().map(|p| *p.as_bytes()).collect();
            let _ = dl.store_part_hashes_by_hash(&hash_hex, &parts);
        }
        if rec.transferred_bytes > 0 {
            dl.add_share_uploaded_bytes(&hash_hex, rec.transferred_bytes)?;
        }
        report.imported += 1;
        tracing::info!(
            "[KNOWNMET] imported {} ({}) as {hash_hex}",
            file_name,
            size
        );
    }

    report.unmatched_records = met.records.len().saturating_sub(matched_records.len());
    Ok(report)
}

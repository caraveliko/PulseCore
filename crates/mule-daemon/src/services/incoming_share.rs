// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Compartición de la carpeta de entrada (semántica de `CSharedFileList` de eMule).
//!
//! eMule comparte todos los archivos situados en su directorio "Incoming",
//! incluidos los subdirectorios anidados. Replicamos esto: al inicio, cada
//! archivo regular bajo el directorio de descargas configurado (recursivamente)
//! se calcula su hash con el algoritmo eD2K (hashes de parte MD4 + hash raíz)
//! y se registra como archivo compartido para que los pares remotos puedan
//! solicitarlo.
//!
//! Se desactiva mediante el ajuste `share_download_dir` (`"0"`). Los archivos
//! que ya están compartidos conservan su indicador `enabled` y su prioridad de
//! subida — el escaneo solo *añade* las entradas que faltan, nunca restablece
//! la configuración del usuario.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::services::downloads::DownloadService;

/// Clave de configuración (persistida en la tabla de ajustes): `"0"` desactiva el escaneo.
pub const SETTING_SHARE_DOWNLOAD_DIR: &str = "share_download_dir";

/// Extensiones de archivo omitidas durante el escaneo de la carpeta de entrada (artefactos
/// de descarga, nunca cargas útiles compartibles).
const SKIP_EXTENSIONS: &[&str] = &["part", "met", "part.met", "bak", "tmp", "ts", "emuletd"];

/// Resultado de un escaneo de la carpeta Incoming: entradas recién
/// registradas y entradas convertidas en huérfanas purgadas.
#[derive(Debug, Default, Clone, Copy)]
pub struct ShareScanReport {
    pub added: usize,
    pub removed: usize,
}

/// Escanea `dir` recursivamente y registra cada archivo regular como archivo
/// compartido (eMule también comparte subdirectorios).
///
/// El cálculo de hash se ejecuta en el grupo de hilos bloqueantes (el hash eD2K está acotado por E/S + MD4).
/// Devuelve el número de archivos recién registrados.
pub async fn scan_download_dir_shares(
    downloads: &Arc<Mutex<DownloadService>>,
    dir: PathBuf,
) -> ShareScanReport {
    // Ajuste de exclusión voluntaria; habilitado por defecto como la carpeta Incoming de eMule.
    let enabled = {
        let dl = downloads.lock().await;
        dl.get_setting(SETTING_SHARE_DOWNLOAD_DIR)
            .ok()
            .flatten()
            .map(|v| !matches!(v.trim().to_lowercase().as_str(), "0" | "false" | "off"))
            .unwrap_or(true)
    };
    if !enabled {
        tracing::info!("[INCOMING_SHARE] download-dir sharing disabled by setting");
        return ShareScanReport::default();
    }

    // eMule comparte subdirectorios recursivamente — recopilar todos los archivos
    // compartibles bajo `dir` en el grupo bloqueante (profundidad arbitraria, limitado por E/S).
    let walk_dir = dir.clone();
    let entries: Vec<PathBuf> = match tokio::task::spawn_blocking(move || {
        collect_shareable_files(&walk_dir)
    })
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[INCOMING_SHARE] directory walk failed: {e}");
            return ShareScanReport::default();
        }
    };

    // Reconciliación al estilo `CSharedFileList::Reload`: los recursos compartidos cuyo
    // archivo bajo la carpeta Incoming haya desaparecido del disco
    // (eliminación física fuera del daemon) son retirados. Guarda
    // `dir.is_dir()`: si la carpeta entera es inaccesible (unidad
    // desconectada) cada archivo parecerá faltante — no purgar nada.
    let mut removed = 0usize;
    if dir.is_dir() {
        removed = prune_stale_incoming_shares(downloads, &dir).await;
    } else {
        tracing::warn!(
            "[INCOMING_SHARE] {} unreachable — stale-share prune skipped",
            dir.display()
        );
    }

    let mut added = 0usize;
    for path in entries {
        let Some(file_name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
        else {
            continue;
        };
        let path_for_hash = path.clone();
        let hashed = match tokio::task::spawn_blocking(move || {
            let mut f = std::fs::File::open(&path_for_hash)?;
            mule_format::ed2k::hash_reader(&mut f)
        })
        .await
        {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                tracing::warn!(
                    "[INCOMING_SHARE] hashing failed for {}: {e}",
                    path.display()
                );
                continue;
            }
            Err(e) => {
                tracing::warn!("[INCOMING_SHARE] hash task failed: {e}");
                continue;
            }
        };

        let hash_hex = hashed.root_hash.to_hex().to_lowercase();
        let mut dl = downloads.lock().await;

        // Ya compartido (descarga completada o recurso compartido manual): mantener la
        // entrada existente y sus ajustes de usuario intactos.
        if dl.get_shared_file(&hash_hex).ok().flatten().is_some() {
            continue;
        }

        let mut shared = mule_core::model::SharedFile::new(
            &hash_hex,
            &file_name,
            hashed.total_bytes,
            path.to_string_lossy().as_ref(),
        );
        // `UAPPref` de eMule: un nuevo recurso compartido inicia con prioridad «auto».
        if dl.auto_up_priority_enabled() {
            shared.up_priority = "auto".to_string();
        }
        if let Err(e) = dl.add_shared_file(&shared) {
            tracing::warn!(
                "[INCOMING_SHARE] failed to register {}: {e}",
                path.display()
            );
            continue;
        }
        if !hashed.part_hashes.is_empty() {
            let _ = dl.store_part_hashes_by_hash(&hash_hex, &hashed.part_hashes);
        }
        added += 1;
        tracing::info!(
            "[INCOMING_SHARE] sharing {} ({}) as {hash_hex}",
            file_name,
            hashed.total_bytes
        );
    }

    if added > 0 {
        tracing::info!(
            "[INCOMING_SHARE] {added} file(s) auto-shared from {}",
            dir.display()
        );
    }
    ShareScanReport { added, removed }
}

/// Retira las entradas `shared_files` que apuntan bajo `dir` cuyo archivo
/// ya no existe en el disco. Los recursos compartidos fuera de `dir` se ignoran:
/// una ruta inaccesible allí es indistinguible de una unidad fuera de línea.
/// La comprobación `exists()` por archivo (y no la pertenencia a la lista
/// escaneada) inmuniza frente a subcarpetas ilegibles.
async fn prune_stale_incoming_shares(
    downloads: &Arc<Mutex<DownloadService>>,
    dir: &Path,
) -> usize {
    let dir_norm = normalize_path_str(dir);
    let mut dl = downloads.lock().await;
    let shares = match dl.list_shared_files() {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[INCOMING_SHARE] cannot list shares for prune: {e}");
            return 0;
        }
    };
    let mut removed = 0usize;
    for sf in shares {
        if sf.file_path.is_empty() || !path_is_under_dir(&sf.file_path, &dir_norm) {
            continue;
        }
        if Path::new(&sf.file_path).exists() {
            continue;
        }
        match dl.remove_shared_file(&sf.file_hash, false) {
            Ok(true) => {
                removed += 1;
                tracing::info!(
                    "[INCOMING_SHARE] unshared {} — fichier supprimé du disque",
                    sf.file_name
                );
            }
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(
                    "[INCOMING_SHARE] failed to drop stale share {}: {e}",
                    sf.file_name
                );
            }
        }
    }
    if removed > 0 {
        tracing::info!(
            "[INCOMING_SHARE] {removed} stale share(s) removed under {}",
            dir.display()
        );
    }
    removed
}

/// Normalización léxica para comparación de rutas de Windows:
/// insensible a mayúsculas/minúsculas, separadores `/` frente a `\` unificados.
fn normalize_path_str(p: &Path) -> String {
    p.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

fn path_is_under_dir(file_path: &str, dir_norm: &str) -> bool {
    let p = normalize_path_str(Path::new(file_path));
    p == dir_norm || p.starts_with(&format!("{dir_norm}/"))
}

/// Recorrido en profundidad que recopila cada archivo compartible bajo `dir`.
/// Se entra recursivamente en los subdirectorios (semántica de eMule); las entradas ocultas y
/// los enlaces simbólicos se omiten — esto último previene ciclos en el sistema de archivos.
pub(crate) fn collect_shareable_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let rd = match std::fs::read_dir(&d) {
            Ok(rd) => rd,
            Err(e) => {
                tracing::debug!("[INCOMING_SHARE] cannot scan {}: {e}", d.display());
                continue;
            }
        };
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            if ft.is_dir() {
                let hidden = entry
                    .file_name()
                    .to_str()
                    .map(|n| n.starts_with('.'))
                    .unwrap_or(true);
                if !hidden {
                    stack.push(path);
                }
                continue;
            }
            if is_shareable_file(&path) {
                out.push(path);
            }
        }
    }
    out
}

/// Un archivo es compartible cuando es un archivo regular, no oculto, cuya
/// extensión no es un artefacto de descarga.
fn is_shareable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return false,
    };
    if name.starts_with('.') {
        return false;
    }
    if path.metadata().map(|m| m.len()).unwrap_or(0) == 0 {
        return false;
    }
    let lower = name.to_lowercase();
    if SKIP_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
        return false;
    }
    mule_network_policy::content_policy::is_allowed_video_filename(name)
}

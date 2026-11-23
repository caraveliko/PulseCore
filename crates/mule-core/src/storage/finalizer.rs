// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::CoreError;

/// Intervalo mínimo en bytes entre notificaciones de progreso durante la copia por fragmentos (2 MiB).
pub const COPY_PROGRESS_NOTIFY_INTERVAL_BYTES: u64 = 2 * 1024 * 1024;

/// Estrategia a seguir si el archivo de destino objetivo ya existe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CollisionStrategy {
    #[default]
    FailIfExists,
    Overwrite,
    /// Semántica de `CPartFile::PerformFileComplete` de eMule: nunca sobrescribir un
    /// destino existente — generar `name(1).ext`, `name(2).ext`, …
    /// continuando la numeración a partir de un sufijo `(N)` existente si está presente.
    Rename,
}

/// Finaliza de forma atómica una descarga completada desde su archivo `.part` hacia una
/// ruta de destino. Devuelve la ruta realmente escrita (idéntica a
/// `target_path` a menos que `strategy` sea [`CollisionStrategy::Rename`] y se
/// haya tenido que resolver una colisión).
pub fn finalize_file(
    temp_path: &Path,
    target_path: &Path,
    strategy: CollisionStrategy,
) -> Result<PathBuf, CoreError> {
    finalize_file_with_progress(temp_path, target_path, strategy, |_, _| {})
}

/// Finaliza una descarga completada desde su archivo `.part` hacia la ruta de destino
/// notificando el progreso de copia `(bytes_copiados, bytes_totales)`.
pub fn finalize_file_with_progress<F>(
    temp_path: &Path,
    target_path: &Path,
    strategy: CollisionStrategy,
    mut on_progress: F,
) -> Result<PathBuf, CoreError>
where
    F: FnMut(u64, u64),
{
    let resolved = match strategy {
        CollisionStrategy::FailIfExists if target_path.exists() => {
            return Err(CoreError::TargetFileExists(target_path.to_path_buf()));
        }
        CollisionStrategy::Rename => resolve_collision_path(target_path),
        _ => target_path.to_path_buf(),
    };

    if let Some(parent) = resolved.parent() {
        fs::create_dir_all(parent)?;
    }

    // Intentar primero el renombrado atómico
    if fs::rename(temp_path, &resolved).is_ok() {
        on_progress(1, 1);
    } else {
        // Alternativa mediante copia por fragmentos (alineada a 128 KiB para NFS/sistemas de archivos en red) + eliminación entre límites de sistemas de archivos
        copy_chunked_with_progress(temp_path, &resolved, on_progress)?;
        let _ = fs::remove_file(temp_path);
    }

    Ok(resolved)
}

/// Copia un archivo en fragmentos de 128 KiB (coincidiendo con el wsize estándar de NFS) con una sincronización final,
/// optimizando transferencias entre sistemas de archivos a través de NVMe local y montajes de red.
pub fn copy_chunked(src_path: &Path, dst_path: &Path) -> std::io::Result<u64> {
    copy_chunked_with_progress(src_path, dst_path, |_, _| {})
}

/// Copia un archivo en fragmentos de 128 KiB notificando el progreso acumulado cada
/// [`COPY_PROGRESS_NOTIFY_INTERVAL_BYTES`].
pub fn copy_chunked_with_progress<F>(
    src_path: &Path,
    dst_path: &Path,
    mut on_progress: F,
) -> std::io::Result<u64>
where
    F: FnMut(u64, u64),
{
    const CHUNK_SIZE: usize = 128 * 1024; // 128 KiB alineados con el wsize de NFS
    let mut src = std::fs::File::open(src_path)?;
    let total_bytes = src.metadata().map(|m| m.len()).unwrap_or(0);
    let mut dst = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(dst_path)?;

    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut total_copied = 0u64;
    let mut last_notified = 0u64;
    on_progress(0, total_bytes);

    loop {
        let n = std::io::Read::read(&mut src, &mut buf)?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut dst, &buf[..n])?;
        total_copied += n as u64;
        if total_copied.saturating_sub(last_notified) >= COPY_PROGRESS_NOTIFY_INTERVAL_BYTES {
            last_notified = total_copied;
            on_progress(total_copied, total_bytes);
        }
    }

    dst.sync_data()?;
    on_progress(total_copied, total_bytes);
    Ok(total_copied)
}

/// Nomenclatura de colisiones de `PerformFileComplete` de eMule: si `target` existe, probar
/// `stem(1).ext`, `stem(2).ext`, … Cuando el tronco del archivo ya termina en
/// `(N)` (solo dígitos o vacío, p. ej. `file(3).mkv`), la numeración continúa en
/// N+1 en el tronco desprovisto del sufijo. Los archivos sin extensión producen `stem(N)` (eMule
/// emite `stem(N).` — un punto final que Windows descarta de todos modos).
fn resolve_collision_path(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }

    let file_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let (base, ext) = match file_name.rfind('.') {
        Some(i) => (&file_name[..i], &file_name[i + 1..]),
        None => (file_name, ""),
    };

    // Sufijo `(N)` existente: `file(3).mkv` en colisión reinicia en `file(4).mkv`.
    let mut stem = base;
    let mut namecount: u64 = 0;
    if base.ends_with(')') {
        if let Some(open) = base.rfind('(') {
            let digits = &base[open + 1..base.len() - 1];
            if digits.bytes().all(|b| b.is_ascii_digit()) {
                namecount = digits.parse().unwrap_or(0);
                stem = &base[..open];
            }
        }
    }

    loop {
        namecount += 1;
        let candidate_name = if ext.is_empty() {
            format!("{stem}({namecount})")
        } else {
            format!("{stem}({namecount}).{ext}")
        };
        let candidate = target.with_file_name(candidate_name);
        if !candidate.exists() {
            return candidate;
        }
    }
}

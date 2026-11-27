// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Verificación criptográfica anti-veneno y fusión real de bytes BitTorrent → eD2K.
//!
//! Garantiza que ningún byte procedente de un enjambre BitTorrent candidato contamine una descarga
//! eD2K sin demostrar primero coincidencia criptográfica MD4 (por parte de 9,28 MiB o por raíz eD2K)
//! y conformidad con la política inmutable «Solo Vídeo».

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use mule_network_policy::content_policy::{validate_video_header, HEADER_INSPECT_BYTES};
use mule_protocol::ed2k::ED2K_PART_SIZE;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::services::downloads::{DownloadService, DownloadSnapshot};

/// Tamaño de bloque inyectado por iteración al fusionar bytes de BitTorrent hacia eD2K (256 KiB).
pub const HYBRID_INJECT_BLOCK_SIZE: usize = 256 * 1024;
/// Tope máximo de bytes fusionados por descarga y ciclo de bombeo (8 MiB) para mantener baja latencia.
pub const HYBRID_MAX_BYTES_PER_PUMP: usize = 8 * 1024 * 1024;
/// Número máximo de partes MD4 comprobadas en una misma llamada de validación de candidato.
const MAX_CANDIDATE_PARTS_TO_VERIFY: usize = 4;

/// Resultado de validar criptográficamente un archivo BitTorrent candidato frente a una descarga eD2K.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateVerificationOutcome {
    /// Aún no hay suficiente volumen de piezas SHA-1 continuas para completar una parte MD4 verificable.
    Pending,
    /// Coincidencia criptográfica demostrada byte a byte (MD4 de parte o raíz eD2K).
    Verified { verified_chunks: u32 },
    /// Discrepancia criptográfica o violación de política «Solo Vídeo»: el puente debe rechazarse.
    Rejected { reason: String },
}

/// Resultado de un ciclo de bombeo de rangos verificados desde BitTorrent hacia `DownloadService`.
#[derive(Debug, Clone)]
pub struct PumpOutcome {
    pub bytes_injected: u64,
    pub latest_snapshot: Option<DownloadSnapshot>,
    pub ed2k_completed: bool,
}

/// Calcula la intersección exacta entre rangos verificados por SHA-1 `[inicio, fin)` del torrent
/// y los rangos aún faltantes en la descarga eD2K.
pub fn intersect_verified_with_missing(
    verified: &[(u64, u64)],
    missing: &[Range<u64>],
) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    let (mut vi, mut mi) = (0usize, 0usize);
    while vi < verified.len() && mi < missing.len() {
        let (vs, ve) = verified[vi];
        let (ms, me) = (missing[mi].start, missing[mi].end);
        let start = vs.max(ms);
        let end = ve.min(me);
        if end > start {
            out.push((start, end));
        }
        if ve <= me {
            vi += 1;
        } else {
            mi += 1;
        }
    }
    out
}

/// Comprueba si el conjunto de rangos `verified` cubre íntegramente el intervalo `[start, end)`.
pub fn ranges_cover_interval(verified: &[(u64, u64)], start: u64, end: u64) -> bool {
    if end <= start {
        return true;
    }
    let mut cursor = start;
    for &(rs, re) in verified {
        if re <= cursor {
            continue;
        }
        if rs > cursor {
            return false;
        }
        cursor = re.max(cursor);
        if cursor >= end {
            return true;
        }
    }
    cursor >= end
}

/// Valida criptográficamente un archivo BitTorrent candidato en disco frente al hash eD2K esperado
/// y su conjunto de hashes de partes (si está disponible), aplicando además la política «Solo Vídeo».
pub fn verify_candidate_bt_file_against_ed2k(
    bt_file_path: &Path,
    display_name: &str,
    total_size: u64,
    expected_ed2k_root: &[u8; 16],
    part_hashes: &[[u8; 16]],
    verified_ranges: &[(u64, u64)],
    bt_finished: bool,
) -> CandidateVerificationOutcome {
    if total_size == 0 || !bt_file_path.is_file() {
        return CandidateVerificationOutcome::Pending;
    }

    let part_size = ED2K_PART_SIZE as u64;

    // 1. Inspección temprana de cabecera binaria «Solo Vídeo» en cuanto el inicio esté disponible.
    let header_len = (HEADER_INSPECT_BYTES as u64).min(total_size);
    if header_len > 0 && ranges_cover_interval(verified_ranges, 0, header_len) {
        if let Ok(prefix) = read_file_slice(bt_file_path, 0, header_len as usize) {
            if let Err(err) = validate_video_header(&prefix, display_name) {
                return CandidateVerificationOutcome::Rejected {
                    reason: format!("cabecera binaria rechazada por política Solo Vídeo: {err}"),
                };
            }
        }
    }

    // 2. Caso de archivo pequeño (≤ 9,28 MiB): basta con verificar la raíz eD2K en cuanto esté completo.
    if total_size <= part_size {
        if !bt_finished && !ranges_cover_interval(verified_ranges, 0, total_size) {
            return CandidateVerificationOutcome::Pending;
        }
        return verify_full_ed2k_root(bt_file_path, expected_ed2k_root, 1);
    }

    // 3. Archivo multiparte con hashset conocido: verificar cualquier parte completa de 9,28 MiB.
    if !part_hashes.is_empty() {
        let mut verified_count = 0u32;
        for (part_idx, expected_part_hash) in part_hashes.iter().enumerate() {
            let ps = (part_idx as u64).saturating_mul(part_size);
            let pe = ps.saturating_add(part_size).min(total_size);
            if pe <= ps {
                break;
            }
            if !ranges_cover_interval(verified_ranges, ps, pe) {
                continue;
            }
            let Ok(part_bytes) = read_file_slice(bt_file_path, ps, (pe - ps) as usize) else {
                return CandidateVerificationOutcome::Pending;
            };
            if mule_core::verify_part_hash(&part_bytes, expected_part_hash) {
                verified_count = verified_count.saturating_add(1);
                if verified_count as usize >= MAX_CANDIDATE_PARTS_TO_VERIFY {
                    break;
                }
            } else {
                return CandidateVerificationOutcome::Rejected {
                    reason: format!(
                        "la parte eD2K #{part_idx} del torrent candidato no coincide con el hash MD4 esperado"
                    ),
                };
            }
        }
        if verified_count > 0 {
            return CandidateVerificationOutcome::Verified {
                verified_chunks: verified_count,
            };
        }
    }

    // 4. Sin hashset previo (0 fuentes eD2K): cuando el torrent completa el archivo de vídeo al 100 %,
    //    se verifica la raíz eD2K completa antes de inyectar un solo byte en la descarga eD2K.
    if bt_finished || ranges_cover_interval(verified_ranges, 0, total_size) {
        let total_parts = total_size.div_ceil(part_size) as u32;
        return verify_full_ed2k_root(bt_file_path, expected_ed2k_root, total_parts);
    }

    CandidateVerificationOutcome::Pending
}

fn verify_full_ed2k_root(
    bt_file_path: &Path,
    expected_ed2k_root: &[u8; 16],
    verified_chunks: u32,
) -> CandidateVerificationOutcome {
    let Ok(file) = File::open(bt_file_path) else {
        return CandidateVerificationOutcome::Pending;
    };
    let mut reader = mule_core::EvictingFileReader::new(file);
    match mule_format::ed2k::hash_reader(&mut reader) {
        Ok(computed) if &computed.root_hash.0 == expected_ed2k_root => {
            CandidateVerificationOutcome::Verified { verified_chunks }
        }
        Ok(computed) => CandidateVerificationOutcome::Rejected {
            reason: format!(
                "raíz eD2K discrepante (esperada {}, obtenida {})",
                hex::encode(expected_ed2k_root),
                computed.root_hash.to_hex()
            ),
        },
        Err(err) => CandidateVerificationOutcome::Rejected {
            reason: format!("error leyendo archivo candidato para verificación eD2K: {err}"),
        },
    }
}

fn read_file_slice(path: &Path, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

/// Bombea los rangos verificados por SHA-1 del torrent hacia la descarga eD2K mediante `inject_range`,
/// leyendo del disco en `spawn_blocking` y sin duplicar bytes ya recibidos por eD2K.
pub async fn pump_verified_ranges_into_ed2k(
    downloads: &Arc<Mutex<DownloadService>>,
    download_id: Uuid,
    bt_info_hash: &str,
    bt_file_path: &Path,
    verified_ranges: &[(u64, u64)],
) -> Result<PumpOutcome, String> {
    let missing = {
        let dl_guard = downloads.lock().await;
        let Some(snap) = dl_guard.get_download(&download_id) else {
            return Ok(PumpOutcome {
                bytes_injected: 0,
                latest_snapshot: None,
                ed2k_completed: true,
            });
        };
        if matches!(
            snap.status.as_str(),
            "completed" | "verifying" | "completing" | "removed"
        ) {
            return Ok(PumpOutcome {
                bytes_injected: 0,
                latest_snapshot: Some(snap),
                ed2k_completed: true,
            });
        }
        dl_guard
            .get_missing_ranges(&download_id)
            .unwrap_or_default()
    };

    let to_copy = intersect_verified_with_missing(verified_ranges, &missing);
    if to_copy.is_empty() {
        return Ok(PumpOutcome {
            bytes_injected: 0,
            latest_snapshot: None,
            ed2k_completed: false,
        });
    }

    // Planificar los bloques a leer en disco acotados a `HYBRID_MAX_BYTES_PER_PUMP`.
    let mut read_plan: Vec<(u64, usize)> = Vec::new();
    let mut planned_bytes = 0usize;
    for (start, end) in to_copy {
        let mut cursor = start;
        while cursor < end && planned_bytes < HYBRID_MAX_BYTES_PER_PUMP {
            let rem_range = (end - cursor) as usize;
            let rem_budget = HYBRID_MAX_BYTES_PER_PUMP - planned_bytes;
            let take = rem_range.min(HYBRID_INJECT_BLOCK_SIZE).min(rem_budget);
            if take == 0 {
                break;
            }
            read_plan.push((cursor, take));
            cursor += take as u64;
            planned_bytes += take;
        }
        if planned_bytes >= HYBRID_MAX_BYTES_PER_PUMP {
            break;
        }
    }

    let path_owned = bt_file_path.to_path_buf();
    let blocks = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<(u64, Vec<u8>)>> {
        let mut file = File::open(&path_owned)?;
        let mut loaded = Vec::with_capacity(read_plan.len());
        for (offset, len) in read_plan {
            file.seek(SeekFrom::Start(offset))?;
            let mut buf = vec![0u8; len];
            file.read_exact(&mut buf)?;
            loaded.push((offset, buf));
        }
        Ok(loaded)
    })
    .await
    .map_err(|e| format!("fallo en tarea de lectura híbrida: {e}"))?
    .map_err(|e| format!("error de E/S leyendo archivo BitTorrent para fusión: {e}"))?;

    let sender_tag = format!("bt-hybrid:{}", &bt_info_hash[..bt_info_hash.len().min(8)]);
    let mut bytes_injected = 0u64;
    let mut latest_snapshot: Option<DownloadSnapshot> = None;

    {
        let mut dl_guard = downloads.lock().await;
        for (offset, data) in blocks {
            let chunk_len = data.len() as u64;
            match dl_guard.inject_range(&download_id, offset, &data, Some(&sender_tag)) {
                Ok(snap) => {
                    bytes_injected = bytes_injected.saturating_add(chunk_len);
                    latest_snapshot = Some(snap);
                }
                Err(err) => {
                    return Err(format!("error inyectando bloque híbrido en eD2K: {err}"));
                }
            }
        }
    }

    let ed2k_completed = latest_snapshot.as_ref().is_some_and(|s| {
        s.received_bytes >= s.total_bytes
            || matches!(s.status.as_str(), "completed" | "verifying" | "completing")
    });

    Ok(PumpOutcome {
        bytes_injected,
        latest_snapshot,
        ed2k_completed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mule_core::AllocationPolicy;

    fn sample_mkv_payload(len: usize, fill: u8) -> Vec<u8> {
        let mut data = vec![fill; len];
        // Cabecera EBML/Matroska válida con DocType `matroska` para superar la política «Solo Vídeo».
        let hdr = b"\x1A\x45\xDF\xA3\x9F\x42\x82\x88matroska";
        data[..hdr.len()].copy_from_slice(hdr);
        data
    }

    #[tokio::test]
    async fn rejects_poisoned_bt_file_and_fuses_verified_bt_file_into_ed2k_download() {
        let dir = tempfile::tempdir().unwrap();
        let target_dir = dir.path().join("incoming");
        let temp_dir = dir.path().join("temp");
        std::fs::create_dir_all(&target_dir).unwrap();

        // Crear un vídeo legítimo y calcular su hash eD2K real.
        let good_payload = sample_mkv_payload(64 * 1024, 0x5A);
        let good_path = dir.path().join("bt_good.mkv");
        std::fs::write(&good_path, &good_payload).unwrap();
        let good_hash =
            mule_format::ed2k::hash_reader(&mut std::io::Cursor::new(&good_payload)).unwrap();

        // Crear un vídeo envenenado del MISMO tamaño exacto en bytes pero distinto contenido.
        let bad_payload = sample_mkv_payload(64 * 1024, 0x99);
        let bad_path = dir.path().join("bt_bad.mkv");
        std::fs::write(&bad_path, &bad_payload).unwrap();

        // 1. El archivo envenenado del mismo tamaño exacto DEBE ser rechazado por MD4.
        let bad_check = verify_candidate_bt_file_against_ed2k(
            &bad_path,
            "pelicula.mkv",
            good_payload.len() as u64,
            &good_hash.root_hash.0,
            &good_hash.part_hashes,
            &[(0, good_payload.len() as u64)],
            true,
        );
        assert!(matches!(
            bad_check,
            CandidateVerificationOutcome::Rejected { .. }
        ));

        // 2. El archivo legítimo DEBE verificarse criptográficamente.
        let good_check = verify_candidate_bt_file_against_ed2k(
            &good_path,
            "pelicula.mkv",
            good_payload.len() as u64,
            &good_hash.root_hash.0,
            &good_hash.part_hashes,
            &[(0, good_payload.len() as u64)],
            true,
        );
        assert_eq!(
            good_check,
            CandidateVerificationOutcome::Verified { verified_chunks: 1 }
        );

        // 3. Registrar la descarga eD2K con 0 fuentes eD2K y completarla al 100 % fusionando BT → eD2K.
        let link = format!(
            "ed2k://|file|pelicula.mkv|{}|{}|/",
            good_payload.len(),
            good_hash.root_hash.to_hex()
        );
        let mut dl_svc = DownloadService::in_memory(temp_dir).unwrap();
        let snap = dl_svc
            .add_ed2k_link(&link, &target_dir, AllocationPolicy::None, None)
            .unwrap();
        let downloads = Arc::new(Mutex::new(dl_svc));

        // Simular que la primera mitad llegó por BT, luego la segunda mitad.
        let half = (good_payload.len() / 2) as u64;
        let p1 = pump_verified_ranges_into_ed2k(
            &downloads,
            snap.id,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            &good_path,
            &[(0, half)],
        )
        .await
        .unwrap();
        assert_eq!(p1.bytes_injected, half);
        assert!(!p1.ed2k_completed);

        let p2 = pump_verified_ranges_into_ed2k(
            &downloads,
            snap.id,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            &good_path,
            &[(0, good_payload.len() as u64)],
        )
        .await
        .unwrap();
        assert_eq!(p2.bytes_injected, good_payload.len() as u64 - half);
        assert!(p2.ed2k_completed);

        let final_snap = downloads.lock().await.get_download(&snap.id).unwrap();
        assert_eq!(final_snap.received_bytes, good_payload.len() as u64);
    }
}

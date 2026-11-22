// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

//! Sincronización automática (reescalado de FPS y alineación sub-a-sub) y manual de subtítulos SRT.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;
use tokio::sync::Semaphore;

use super::srt::{parse_srt, SrtCue};

/// Límite de concurrencia para procesos `ffmpeg` de sincronización (protege CPU/RAM en Raspberry Pi).
const MAX_CONCURRENT_SYNC_JOBS: usize = 1;
/// Desplazamiento máximo buscado en la alineación automática (±60 segundos).
const MAX_AUTO_OFFSET_MS: i64 = 60_000;
/// Tamaño del cubo de histograma inicial para estimar el desplazamiento (250 ms).
const COARSE_HISTOGRAM_BUCKET_MS: i64 = 250;
/// Ventana de refinamiento fino alrededor del mejor desplazamiento (±500 ms).
const FINE_SEARCH_RADIUS_MS: i64 = 500;
/// Paso de refinamiento fino (50 ms).
const FINE_SEARCH_STEP_MS: i64 = 50;
/// Tolerancia de emparejamiento entre un inicio de *cue* objetivo y uno de referencia (350 ms).
const MATCH_TOLERANCE_MS: i64 = 350;
/// Mejora mínima de bloques alineados exigida para aplicar la sincronización automática.
const MIN_ALIGNED_GAIN: usize = 3;
/// Número máximo de bloques muestreados para la alineación rápida.
const MAX_ALIGNMENT_SAMPLE_CUES: usize = 400;

static SUBTITLE_SYNC_SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();

fn sync_semaphore() -> &'static Semaphore {
    SUBTITLE_SYNC_SEMAPHORE.get_or_init(|| Semaphore::new(MAX_CONCURRENT_SYNC_JOBS))
}

/// Resultado de la alineación automática frente a una pista de referencia.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncAlignmentResult {
    pub applied: bool,
    pub offset_ms: i64,
    pub matched_before: usize,
    pub matched_after: usize,
}

/// Desplaza todos los bloques del subtítulo en `offset_ms` milisegundos (sin bajar de `0 ms`).
pub fn shift_cues_ms(cues: &mut [SrtCue], offset_ms: i64) {
    if offset_ms == 0 {
        return;
    }
    for cue in cues.iter_mut() {
        let dur = (cue.end_ms - cue.start_ms).max(100);
        let new_start = (cue.start_ms + offset_ms).max(0);
        cue.start_ms = new_start;
        cue.end_ms = new_start + dur;
    }
}

/// Reeescala todas las marcas temporales desde `from_fps_milli` hasta `to_fps_milli`.
pub fn rescale_cues_fps(cues: &mut [SrtCue], from_fps_milli: u32, to_fps_milli: u32) {
    if from_fps_milli == 0 || to_fps_milli == 0 || from_fps_milli == to_fps_milli {
        return;
    }
    let ratio = (from_fps_milli as f64) / (to_fps_milli as f64);
    for cue in cues.iter_mut() {
        let start = ((cue.start_ms as f64) * ratio).round() as i64;
        let end = ((cue.end_ms as f64) * ratio).round() as i64;
        cue.start_ms = start.max(0);
        cue.end_ms = end.max(cue.start_ms + 100);
    }
}

fn evaluate_offset_alignment(target_starts: &[i64], ref_starts: &[i64], offset_ms: i64) -> (usize, i64) {
    let mut hits = 0usize;
    let mut total_err = 0i64;
    let mut r_idx = 0usize;
    for &ts in target_starts {
        let shifted = ts + offset_ms;
        while r_idx < ref_starts.len() && ref_starts[r_idx] < shifted - MATCH_TOLERANCE_MS {
            r_idx += 1;
        }
        if r_idx < ref_starts.len() {
            let mut best_diff = (ref_starts[r_idx] - shifted).abs();
            if r_idx + 1 < ref_starts.len() {
                let next_diff = (ref_starts[r_idx + 1] - shifted).abs();
                if next_diff < best_diff {
                    best_diff = next_diff;
                }
            }
            if best_diff <= MATCH_TOLERANCE_MS {
                hits += 1;
                total_err += best_diff;
            }
        }
    }
    (hits, total_err)
}

/// Alinea `target` contra una pista de referencia `reference` ya sincronizada mediante histograma + refinamiento.
pub fn align_cues_to_reference(
    target: &mut [SrtCue],
    reference: &[SrtCue],
) -> SyncAlignmentResult {
    if target.is_empty() || reference.is_empty() {
        return SyncAlignmentResult {
            applied: false,
            offset_ms: 0,
            matched_before: 0,
            matched_after: 0,
        };
    }

    let mut target_starts: Vec<i64> = target
        .iter()
        .take(MAX_ALIGNMENT_SAMPLE_CUES)
        .map(|c| c.start_ms)
        .collect();
    let mut ref_starts: Vec<i64> = reference
        .iter()
        .take(MAX_ALIGNMENT_SAMPLE_CUES)
        .map(|c| c.start_ms)
        .collect();
    target_starts.sort_unstable();
    ref_starts.sort_unstable();

    let (matched_before, initial_err) = evaluate_offset_alignment(&target_starts, &ref_starts, 0);

    // 1. Histograma grueso de diferencias `ref - target` dentro de `[-MAX_AUTO_OFFSET_MS, +MAX_AUTO_OFFSET_MS]`.
    let mut buckets: HashMap<i64, usize> = HashMap::new();
    for &ts in &target_starts {
        for &rs in &ref_starts {
            let diff = rs - ts;
            if diff.abs() <= MAX_AUTO_OFFSET_MS {
                let bucket = (diff / COARSE_HISTOGRAM_BUCKET_MS) * COARSE_HISTOGRAM_BUCKET_MS;
                *buckets.entry(bucket).or_insert(0) += 1;
            }
        }
    }

    let Some((&coarse_best, _)) = buckets.iter().max_by_key(|(_, &cnt)| cnt) else {
        return SyncAlignmentResult {
            applied: false,
            offset_ms: 0,
            matched_before,
            matched_after: matched_before,
        };
    };

    // 2. Refinamiento fino alrededor del pico grueso minimizando el error absoluto medio.
    let mut best_offset = 0i64;
    let mut best_hits = matched_before;
    let mut best_err = initial_err;
    let mut candidate = coarse_best - FINE_SEARCH_RADIUS_MS;
    while candidate <= coarse_best + FINE_SEARCH_RADIUS_MS {
        let (hits, err) = evaluate_offset_alignment(&target_starts, &ref_starts, candidate);
        if hits > best_hits
            || (hits == best_hits && (err < best_err || (err == best_err && candidate.abs() < best_offset.abs())))
        {
            best_hits = hits;
            best_err = err;
            best_offset = candidate;
        }
        candidate += FINE_SEARCH_STEP_MS;
    }

    if best_offset != 0 && best_hits >= matched_before + MIN_ALIGNED_GAIN {
        shift_cues_ms(target, best_offset);
        return SyncAlignmentResult {
            applied: true,
            offset_ms: best_offset,
            matched_before,
            matched_after: best_hits,
        };
    }

    SyncAlignmentResult {
        applied: false,
        offset_ms: 0,
        matched_before,
        matched_after: matched_before,
    }
}

/// Extrae una pista de subtítulos textual embebida usando `ffmpeg` bajo semáforo de concurrencia.
pub async fn extract_embedded_srt_via_ffmpeg(
    video_path: &Path,
    sub_stream_idx: usize,
) -> Option<Vec<SrtCue>> {
    let _permit = sync_semaphore().acquire().await.ok()?;
    let path_buf = video_path.to_path_buf();
    let map_arg = format!("0:s:{sub_stream_idx}");

    tokio::task::spawn_blocking(move || {
        let output = Command::new("ffmpeg")
            .args(["-v", "quiet", "-i"])
            .arg(&path_buf)
            .args(["-map", &map_arg, "-f", "srt", "pipe:1"])
            .output()
            .ok()?;
        if !output.status.success() || output.stdout.is_empty() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        parse_srt(&text).ok()
    })
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cues(starts: &[i64]) -> Vec<SrtCue> {
        starts
            .iter()
            .enumerate()
            .map(|(i, &s)| SrtCue {
                index: i + 1,
                start_ms: s,
                end_ms: s + 1500,
                text: format!("Línea {}", i + 1),
            })
            .collect()
    }

    #[test]
    fn test_shift_and_rescale_fps() {
        let mut cues = make_cues(&[10_000, 25_000, 100_000]);
        rescale_cues_fps(&mut cues, 25_000, 23_976);
        // 100_000 * (25000 / 23976) ≈ 104_271
        assert!((cues[2].start_ms - 104_271).abs() <= 2);

        shift_cues_ms(&mut cues, -15_000);
        assert_eq!(cues[0].start_ms, 0);
    }

    #[test]
    fn test_align_cues_recovers_constant_offset() {
        let reference = make_cues(&[
            5_000, 12_000, 18_500, 24_000, 31_200, 40_000, 48_700, 55_100, 63_400, 72_000,
        ]);
        // El subtítulo objetivo llega adelantado 4,2 segundos respecto al vídeo.
        let mut target = make_cues(&[
            800, 7_800, 14_300, 19_800, 27_000, 35_800, 44_500, 50_900, 59_200, 67_800,
        ]);

        let res = align_cues_to_reference(&mut target, &reference);
        assert!(res.applied);
        assert!((res.offset_ms - 4_200).abs() <= FINE_SEARCH_STEP_MS);
        assert_eq!(res.matched_after, 10);
    }
}

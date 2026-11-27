// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Puente de rangos verificados por SHA-1 y archivos completados entre `librqbit` y el motor híbrido eD2K.

use std::ops::Range;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Resultado de lanzar un torrent auxiliar orientado a acelerar una descarga eD2K por tamaño exacto.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridTorrentLaunchResult {
    pub torrent_id: usize,
    pub info_hash: String,
    pub file_index: usize,
    pub file_offset: u64,
    pub file_name: String,
    pub file_size: u64,
}

/// Estado instantáneo de rangos verificados por SHA-1 de un archivo de vídeo dentro de un torrent activo.
#[derive(Debug, Clone)]
pub struct VerifiedTorrentFileState {
    pub torrent_id: usize,
    pub info_hash: String,
    pub file_index: usize,
    pub file_path: PathBuf,
    pub file_size: u64,
    pub finished: bool,
    pub peer_count: usize,
    pub download_rate_bps: u64,
    /// Rangos contiguos `[inicio, fin)` (en bytes relativos al inicio del archivo de vídeo)
    /// respaldados al 100 % por piezas BitTorrent ya verificadas por SHA-1.
    pub verified_ranges: Vec<(u64, u64)>,
}

/// Vídeo completado en el motor BitTorrent listo para calcular su hash eD2K y sembrar el puente.
#[derive(Debug, Clone)]
pub struct CompletedTorrentVideo {
    pub torrent_id: usize,
    pub info_hash: String,
    pub torrent_name: String,
    pub file_index: usize,
    pub file_offset: u64,
    pub file_name: String,
    pub file_path: PathBuf,
    pub file_size: u64,
    pub total_torrent_bytes: u64,
}

/// Calcula los rangos de bytes `[inicio, fin)` relativos al archivo objetivo cubiertos por piezas
/// BitTorrent ya verificadas por SHA-1, fusionando rangos adyacentes.
pub fn compute_verified_file_ranges(
    file_offset: u64,
    file_len: u64,
    piece_length: u64,
    piece_range: Range<u32>,
    has_piece: impl Fn(u32) -> bool,
) -> Vec<(u64, u64)> {
    if file_len == 0 || piece_length == 0 {
        return Vec::new();
    }
    let file_end = file_offset.saturating_add(file_len);
    let mut ranges: Vec<(u64, u64)> = Vec::new();

    for piece_idx in piece_range {
        if !has_piece(piece_idx) {
            continue;
        }
        let piece_start = u64::from(piece_idx).saturating_mul(piece_length);
        let piece_end = piece_start.saturating_add(piece_length);

        let inter_start = piece_start.max(file_offset);
        let inter_end = piece_end.min(file_end);
        if inter_end <= inter_start {
            continue;
        }

        let rel_start = inter_start - file_offset;
        let rel_end = inter_end - file_offset;
        if let Some(last) = ranges.last_mut() {
            if rel_start <= last.1 {
                last.1 = last.1.max(rel_end);
                continue;
            }
        }
        ranges.push((rel_start, rel_end));
    }

    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_and_merges_unaligned_multifile_verified_ranges() {
        // Archivo de vídeo empieza en el desplazamiento 500 y mide 2_500 bytes (500..3000).
        // Piezas de 1_000 bytes: pieza 0 [0..1000), pieza 1 [1000..2000), pieza 2 [2000..3000).
        let ranges = compute_verified_file_ranges(500, 2_500, 1_000, 0..3, |p| p == 0 || p == 2);
        assert_eq!(ranges, vec![(0, 500), (1_500, 2_500)]);

        // Si las tres piezas están verificadas, se fusionan en un único rango [0, 2_500).
        let full = compute_verified_file_ranges(500, 2_500, 1_000, 0..3, |_| true);
        assert_eq!(full, vec![(0, 2_500)]);
    }
}

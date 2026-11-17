// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Almacenamiento temporal (staging) en RAM de partes eD2K (modelo `m_tData` de eMule /
//! libtorrent staging → hash → flush): los bytes inyectados se copian en un
//! búfer por parte para permitir la verificación MD4 **sin relectura de disco**
//! al completarse — elimina ~`total_size` bytes de `pread` por descarga.

use mule_core::RangeSet;
use std::collections::HashMap;

/// Límite de memoria: número máximo de partes preparadas simultáneamente por
/// descarga (≈ 4 × 9,3 MiB). Más allá, la verificación recae en la
/// relectura de disco (`PartFile::read_range`).
pub(crate) const MAX_STAGED_PARTS: usize = 4;

/// Búfer de ensamblado de una parte en curso de recepción.
///
/// `pub` requerido por el campo público `ActiveDownload::staged_parts`;
/// todas las operaciones permanecen como `pub(crate)`.
pub struct StagedPart {
    /// Bytes de la parte recibidos en esta sesión, indexados por desplazamiento local.
    /// Dimensionado al tamaño real de la parte (última parte incluida).
    data: Vec<u8>,
    /// Cobertura de los bytes efectivamente preparados (desplazamientos locales).
    covered: RangeSet,
}

impl StagedPart {
    fn new(part_len: usize) -> Self {
        Self {
            data: vec![0u8; part_len],
            covered: RangeSet::new(),
        }
    }

    /// Copia bytes `[local_start, local_start+bytes)` en el búfer.
    fn stage(&mut self, local_start: u64, bytes: &[u8]) {
        let s = local_start as usize;
        self.data[s..s + bytes.len()].copy_from_slice(bytes);
        let _ = self.covered.insert(local_start..local_start + bytes.len() as u64);
    }

    /// Todos los bytes `[0, part_len)` se han preparado en esta sesión → la
    /// parte es procesable por hash enteramente en RAM.
    fn is_fully_staged(&self, part_len: u64) -> bool {
        self.covered.covered_len() == part_len
    }
}

/// Copia los bytes de un rango inyectado en los búferes de las partes
/// afectadas. Las partes ya completas no se preparan (sus datos
/// se ignoran previamente), y más allá de [`MAX_STAGED_PARTS`] las nuevas
/// partes no se preparan — la verificación releerá el disco.
pub(crate) fn stage_range(
    staged: &mut HashMap<u32, StagedPart>,
    total_size: u64,
    part_size: u64,
    offset: u64,
    data: &[u8],
) {
    let end = offset + data.len() as u64;
    let first_part = offset / part_size;
    let last_part = (end - 1) / part_size;
    for p in first_part..=last_part {
        let ps = p * part_size;
        let pe = ((p + 1) * part_size).min(total_size);
        let s = offset.max(ps);
        let e = end.min(pe);
        if s >= e {
            continue;
        }
        let key = p as u32;
        if staged.len() >= MAX_STAGED_PARTS && !staged.contains_key(&key) {
            continue;
        }
        let part_len = (pe - ps) as usize;
        let sp = staged.entry(key).or_insert_with(|| StagedPart::new(part_len));
        let ds = (s - offset) as usize;
        let de = (e - offset) as usize;
        sp.stage(s - ps, &data[ds..de]);
    }
}

/// Hash MD4 de la parte desde su búfer preparado — `None` cuando la parte
/// no está enteramente preparada (descarga reanudada a media parte, límite
/// de memoria alcanzado) y el llamador debe releer el disco.
pub(crate) fn verify_staged(
    staged: &mut HashMap<u32, StagedPart>,
    part_idx: u32,
    part_len: u64,
    expected_hash: &[u8; 16],
) -> Option<bool> {
    let sp = staged.get(&part_idx)?;
    if !sp.is_fully_staged(part_len) {
        return None;
    }
    Some(mule_core::verify_part_hash(&sp.data, expected_hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use md4::{Digest, Md4};

    const PART: u64 = 100;
    const TOTAL: u64 = 250; // 3 parts : 100 + 100 + 50

    fn md4_of(data: &[u8]) -> [u8; 16] {
        Md4::digest(data).into()
    }

    #[test]
    fn stage_and_verify_full_part() {
        let mut staged = HashMap::new();
        let data: Vec<u8> = (0..TOTAL).map(|i| i as u8).collect();
        // Dos bloques para la parte 0, un bloque a caballo entre las partes 1-2.
        stage_range(&mut staged, TOTAL, PART, 0, &data[..60]);
        stage_range(&mut staged, TOTAL, PART, 60, &data[60..]);
        assert_eq!(staged.len(), 3);
        let expected = md4_of(&data[..100]);
        assert_eq!(
            verify_staged(&mut staged, 0, 100, &expected),
            Some(true),
            "part 0 entièrement stagée → hash vérifiable en RAM"
        );
    }

    #[test]
    fn verify_staged_returns_none_when_incomplete() {
        let mut staged = HashMap::new();
        let data = vec![7u8; 80];
        stage_range(&mut staged, TOTAL, PART, 0, &data);
        let expected = md4_of(&[7u8; 100]);
        assert_eq!(
            verify_staged(&mut staged, 0, 100, &expected),
            None,
            "part partiellement stagée → repli disque requis"
        );
    }

    #[test]
    fn verify_staged_detects_corruption() {
        let mut staged = HashMap::new();
        let data = vec![9u8; 100];
        stage_range(&mut staged, TOTAL, PART, 0, &data);
        let wrong = md4_of(&[0u8; 100]);
        assert_eq!(verify_staged(&mut staged, 0, 100, &wrong), Some(false));
    }

    #[test]
    fn staged_parts_bounded_by_max() {
        let mut staged = HashMap::new();
        let block = vec![1u8; 10];
        // Llena MAX_STAGED_PARTS partes distintas.
        for p in 0..MAX_STAGED_PARTS as u64 {
            stage_range(&mut staged, u64::MAX / 2, PART, p * PART, &block);
        }
        assert_eq!(staged.len(), MAX_STAGED_PARTS);
        // Una (MAX+1)-ésima parte no se prepara.
        stage_range(
            &mut staged,
            u64::MAX / 2,
            PART,
            MAX_STAGED_PARTS as u64 * PART,
            &block,
        );
        assert_eq!(staged.len(), MAX_STAGED_PARTS);
        // Pero una parte ya presente continúa absorbiendo bloques.
        stage_range(&mut staged, u64::MAX / 2, PART, 10, &block);
        assert_eq!(staged.len(), MAX_STAGED_PARTS);
    }

    #[test]
    fn range_spanning_part_boundary_splits() {
        let mut staged = HashMap::new();
        let data = vec![3u8; 30];
        // 85..115 cubre el final de la parte 0 y el inicio de la parte 1.
        stage_range(&mut staged, TOTAL, PART, 85, &data);
        let p0 = staged.get(&0).expect("part 0 stagée");
        let p1 = staged.get(&1).expect("part 1 stagée");
        assert_eq!(p0.covered.ranges(), vec![85..100]);
        assert_eq!(p1.covered.ranges(), vec![0..15]);
        assert_eq!(&p0.data[85..100], &[3u8; 15]);
        assert_eq!(&p1.data[..15], &[3u8; 15]);
    }
}

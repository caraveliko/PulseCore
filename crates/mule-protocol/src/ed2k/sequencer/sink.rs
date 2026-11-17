// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Sumidero de bloques en memoria para ensamblar los bloques eD2K descargados.

use super::types::Ed2kSequencerError;
use super::Ed2kDownloadSequencer;
use crate::ed2k::packet::{Ed2kBlockRange, Ed2kSendingPart};

/// Asignación máxima admisible del sumidero en memoria (512 MiB) para evitar la denegación de servicio por falta de memoria (OOM).
pub const MAX_MEMORY_SINK_SIZE: u64 = 512 * 1024 * 1024;

/// Sumidero en memoria para datos de bloques eD2K ensamblados.
///
/// Invariantes:
/// - Búfer preasignado exacto que coincide con el tamaño total del archivo.
/// - Valida cada bloque entrante frente al `Ed2kDownloadSequencer` activo.
/// - Rechaza recepciones duplicadas del mismo rango de bloque (`Ed2kSequencerError::DuplicateBlock`).
/// - Rechaza rangos de bloques que crucen límites de partes (`Ed2kSequencerError::CrossPartBoundary`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kBlockSink {
    file_size: u64,
    buffer: Vec<u8>,
    received_ranges: Vec<(u64, u64)>,
}

impl Ed2kBlockSink {
    /// Crea un nuevo `Ed2kBlockSink` inicializado a cero para un tamaño de archivo dado.
    /// Entra en pánico si `file_size > MAX_MEMORY_SINK_SIZE`.
    pub fn new(file_size: u64) -> Self {
        assert!(
            file_size <= MAX_MEMORY_SINK_SIZE,
            "file_size {} exceeds MAX_MEMORY_SINK_SIZE ({})",
            file_size,
            MAX_MEMORY_SINK_SIZE
        );
        Self {
            file_size,
            buffer: vec![0u8; file_size as usize],
            received_ranges: Vec::new(),
        }
    }

    /// Intenta crear de forma segura un `Ed2kBlockSink`, devolviendo un error si `file_size` supera el límite.
    pub fn try_new(file_size: u64) -> Result<Self, String> {
        if file_size > MAX_MEMORY_SINK_SIZE {
            return Err(format!(
                "Requested file size {file_size} exceeds in-memory sink maximum of {MAX_MEMORY_SINK_SIZE} bytes"
            ));
        }
        Ok(Self {
            file_size,
            buffer: vec![0u8; file_size as usize],
            received_ranges: Vec::new(),
        })
    }

    /// Devuelve el tamaño total del archivo en bytes.
    #[inline]
    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    /// Devuelve el búfer de bytes subyacente.
    #[inline]
    pub fn buffer(&self) -> &[u8] {
        &self.buffer
    }

    /// Devuelve la lista de rangos de bytes escritos con éxito `[start, end)`.
    #[inline]
    pub fn received_ranges(&self) -> &[(u64, u64)] {
        &self.received_ranges
    }

    /// Devuelve verdadero si el rango de bytes semiabierto `[start, end)` está completamente cubierto por fragmentos recibidos.
    pub fn is_range_covered(&self, start: u64, end: u64) -> bool {
        Ed2kDownloadSequencer::is_range_in_list(start, end, &self.received_ranges)
    }

    /// Devuelve verdadero si un rango de bloque `[start, end)` ya se ha recibido o está completamente cubierto.
    pub fn has_range(&self, start: u64, end: u64) -> bool {
        if self
            .received_ranges
            .iter()
            .any(|&(s, e)| s == start && e == end)
        {
            return true;
        }
        self.is_range_covered(start, end)
    }

    /// Devuelve verdadero si algún byte en `[start, end)` se solapa con rangos ya recibidos.
    pub fn has_overlap(&self, start: u64, end: u64) -> bool {
        self.received_ranges
            .iter()
            .any(|&(s, e)| s < end && start < e)
    }

    /// Devuelve el número total de bytes recibidos y escritos hasta ahora.
    pub fn received_bytes_count(&self) -> usize {
        self.received_ranges
            .iter()
            .map(|&(s, e)| (e - s) as usize)
            .sum()
    }

    /// Valida y escribe un `Ed2kSendingPart` entrante en el sumidero.
    pub fn write_block(
        &mut self,
        sequencer: &Ed2kDownloadSequencer,
        part: &Ed2kSendingPart,
    ) -> Result<Ed2kBlockRange, Ed2kSequencerError> {
        let range = sequencer.validate_sending_part(part)?;
        let start = range.start as u64;
        let end = range.end as u64;

        if self.has_overlap(start, end) {
            return Err(Ed2kSequencerError::DuplicateBlock { start, end });
        }

        let start_idx = start as usize;
        let end_idx = end as usize;
        self.buffer[start_idx..end_idx].copy_from_slice(&part.data);
        self.received_ranges.push((start, end));

        Ok(range)
    }
}

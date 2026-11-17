// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Tipos de almacenamiento, enumeraciones de estado y seguimiento del estado de reanudación para eD2K.

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ed2k::packet::{Ed2kBlockRange, Ed2kSendingPart};
use crate::ed2k::sequencer::{Ed2kBlockSink, Ed2kDownloadSequencer, Ed2kSequencerError};

/// Rasgo común para sumideros de bytes capaces de validar y registrar fragmentos eD2K descargados.
pub trait Ed2kBlockWriter {
    /// Valida y escribe un fragmento `Ed2kSendingPart` entrante en el sumidero.
    fn write_block(
        &mut self,
        sequencer: &Ed2kDownloadSequencer,
        part: &Ed2kSendingPart,
    ) -> Result<Ed2kBlockRange, Ed2kSequencerError>;

    /// Devuelve verdadero si el rango de bytes semiabierto `[start, end)` está completamente cubierto.
    fn has_range(&self, start: u64, end: u64) -> bool;
}

impl Ed2kBlockWriter for Ed2kBlockSink {
    fn write_block(
        &mut self,
        sequencer: &Ed2kDownloadSequencer,
        part: &Ed2kSendingPart,
    ) -> Result<Ed2kBlockRange, Ed2kSequencerError> {
        self.write_block(sequencer, part)
    }

    fn has_range(&self, start: u64, end: u64) -> bool {
        self.has_range(start, end)
    }
}

/// Fusiona rangos de bytes semiabiertos `[start, end)` superpuestos o adyacentes.
pub fn merge_intervals(mut ranges: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    if ranges.is_empty() {
        return ranges;
    }
    ranges.sort_unstable_by_key(|r| r.0);
    let mut merged: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
    let mut current = ranges[0];

    for r in ranges.into_iter().skip(1) {
        if r.0 <= current.1 {
            current.1 = std::cmp::max(current.1, r.1);
        } else {
            merged.push(current);
            current = r;
        }
    }
    merged.push(current);
    merged
}

/// Estado del ciclo de vida de una descarga eD2K en el almacenamiento local.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Ed2kPartDownloadStatus {
    /// En curso: descargando bloques en el archivo `.part`.
    #[default]
    Downloading,
    /// Descarga completa: todas las partes verificadas con el MD4 raíz de eD2K, lista para promoción local.
    VerifiedPendingPromotion,
    /// Promocionado con éxito al directorio de descarga de destino; archivo `.part` renombrado y diario eliminado.
    Promoted,
}

/// Estado de diario persistente guardado junto a un archivo de descarga `.part`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ed2kPartResumeState {
    /// Hash de archivo eD2K MD4 de 16 bytes en formato hexadecimal en minúsculas.
    pub file_hash: String,
    /// Tamaño total esperado del archivo en bytes.
    pub file_size: u64,
    /// Lista canónica y fusionada de intervalos de bytes semiabiertos `[start, end)` recibidos y escritos en disco.
    pub completed_ranges: Vec<(u64, u64)>,
    /// Total acumulado de bytes de carga útil recibidos y verificados.
    pub total_bytes_received: u64,
    /// Estado actual de descarga y promoción.
    #[serde(default)]
    pub status: Ed2kPartDownloadStatus,
    /// Marca de tiempo UNIX de la última actualización del diario.
    pub updated_at: u64,
}

impl Ed2kPartResumeState {
    /// Crea un estado de reanudación nuevo para un archivo determinado.
    pub fn new(file_hash: [u8; 16], file_size: u64) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Self {
            file_hash: hex::encode(file_hash),
            file_size,
            completed_ranges: Vec::new(),
            total_bytes_received: 0,
            status: Ed2kPartDownloadStatus::Downloading,
            updated_at: now,
        }
    }

    /// Carga y analiza un estado de reanudación desde el archivo JSON especificado.
    pub fn load_from_file(path: &Path) -> Result<Self, Ed2kSequencerError> {
        let data = fs::read(path).map_err(|e| {
            Ed2kSequencerError::Storage(format!(
                "Failed to read resume file '{}': {e}",
                path.display()
            ))
        })?;
        let state: Self = serde_json::from_slice(&data).map_err(|e| {
            Ed2kSequencerError::Storage(format!(
                "Failed to parse resume JSON '{}': {e}",
                path.display()
            ))
        })?;
        Ok(state)
    }

    /// Persiste atómicamente el estado de reanudación en `path` mediante un archivo temporal.
    pub fn save_to_file(&self, path: &Path) -> Result<(), Ed2kSequencerError> {
        let json_bytes = serde_json::to_vec_pretty(self).map_err(|e| {
            Ed2kSequencerError::Storage(format!("Failed to serialize resume state to JSON: {e}"))
        })?;

        let tmp_path = format!("{}.tmp", path.display());
        let tmp_path = PathBuf::from(tmp_path);

        {
            let mut file = File::create(&tmp_path).map_err(|e| {
                Ed2kSequencerError::Storage(format!(
                    "Failed to create temporary resume file '{}': {e}",
                    tmp_path.display()
                ))
            })?;
            file.write_all(&json_bytes).map_err(|e| {
                Ed2kSequencerError::Storage(format!(
                    "Failed to write to temporary resume file '{}': {e}",
                    tmp_path.display()
                ))
            })?;
            file.sync_all().map_err(|e| {
                Ed2kSequencerError::Storage(format!(
                    "Failed to sync temporary resume file '{}': {e}",
                    tmp_path.display()
                ))
            })?;
        }

        if let Err(e) = fs::rename(&tmp_path, path) {
            if path.exists() {
                let _ = fs::remove_file(path);
                fs::rename(&tmp_path, path).map_err(|e2| {
                    Ed2kSequencerError::Storage(format!(
                        "Failed to atomically rename '{}' to '{}': {e2} (initial error: {e})",
                        tmp_path.display(),
                        path.display()
                    ))
                })?;
            } else {
                return Err(Ed2kSequencerError::Storage(format!(
                    "Failed to rename '{}' to '{}': {e}",
                    tmp_path.display(),
                    path.display()
                )));
            }
        }

        Ok(())
    }

    /// Registra un intervalo de bytes recién escrito `[start, end)` y fusiona rangos contiguos.
    pub fn record_range(&mut self, start: u64, end: u64) {
        if start >= end {
            return;
        }
        self.completed_ranges.push((start, end));
        self.completed_ranges = merge_intervals(std::mem::take(&mut self.completed_ranges));
        self.total_bytes_received = self.completed_ranges.iter().map(|&(s, e)| e - s).sum();
        self.updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
    }
}

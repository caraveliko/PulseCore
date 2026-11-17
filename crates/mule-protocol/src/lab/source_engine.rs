// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Motor seguro de fuentes de ficheros de prueba en memoria para transferencias en bucle local (loopback) de laboratorio.

use std::collections::HashMap;

use bytes::Bytes;

use super::messages::{LabMessage, LabRejectReason};
use crate::limits::MAX_BLOCK_BYTES;

/// Un archivo de prueba alojado por el motor de fuentes de laboratorio.
#[derive(Debug, Clone)]
pub struct HostedFile {
    /// Hash de archivo eD2K de 16 bytes.
    pub file_hash: [u8; 16],
    /// Tamaño total del archivo en bytes.
    pub file_size: u64,
    /// Nombre para mostrar.
    pub name: String,
    /// Carga útil de datos binarios en memoria.
    pub data: Bytes,
}

/// Servidor de fuentes en memoria para pruebas de laboratorio.
///
/// Rechaza entradas de rutas del sistema de archivos procedentes de la red y sirve estrictamente
/// desde un registro interno indexado por hashes de 16 bytes.
#[derive(Debug, Clone, Default)]
pub struct LabSourceEngine {
    files: HashMap<[u8; 16], HostedFile>,
}

impl LabSourceEngine {
    /// Crea un nuevo `LabSourceEngine` vacío.
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    /// Registra un archivo de prueba en el motor de fuentes.
    pub fn host_file(&mut self, file_hash: [u8; 16], name: impl Into<String>, data: Bytes) {
        let file_size = data.len() as u64;
        self.files.insert(
            file_hash,
            HostedFile {
                file_hash,
                file_size,
                name: name.into(),
                data,
            },
        );
    }

    /// Devuelve los hashes de todos los archivos alojados.
    pub fn hosted_hashes(&self) -> Vec<[u8; 16]> {
        self.files.keys().copied().collect()
    }

    /// Genera un mensaje `FileOffer` para el hash especificado si está alojado.
    pub fn offer_file(&self, file_hash: &[u8; 16]) -> Option<LabMessage> {
        self.files.get(file_hash).map(|f| LabMessage::FileOffer {
            file_hash: f.file_hash,
            file_size: f.file_size,
            name: f.name.clone(),
        })
    }

    /// Sirve una petición de bloque de forma segura según los límites del protocolo.
    pub fn serve_block(&self, file_hash: &[u8; 16], start_offset: u64, length: u32) -> LabMessage {
        let file = match self.files.get(file_hash) {
            Some(f) => f,
            None => {
                return LabMessage::BlockRejected {
                    file_hash: *file_hash,
                    start_offset,
                    reason: LabRejectReason::FileNotFound,
                }
            }
        };

        if length == 0 || (length as usize) > MAX_BLOCK_BYTES {
            return LabMessage::BlockRejected {
                file_hash: *file_hash,
                start_offset,
                reason: LabRejectReason::InvalidRange,
            };
        }

        let end_offset = match start_offset.checked_add(length as u64) {
            Some(end) => end,
            None => {
                return LabMessage::BlockRejected {
                    file_hash: *file_hash,
                    start_offset,
                    reason: LabRejectReason::InvalidRange,
                }
            }
        };

        if end_offset > file.file_size {
            return LabMessage::BlockRejected {
                file_hash: *file_hash,
                start_offset,
                reason: LabRejectReason::InvalidRange,
            };
        }

        let start = start_offset as usize;
        let end = end_offset as usize;
        let block_slice = file.data.slice(start..end);

        LabMessage::BlockData {
            file_hash: *file_hash,
            start_offset,
            data: block_slice,
        }
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Rastreadores de peticiones y bloques en curso para sesiones eD2K.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::types::Ed2kSessionError;

/// Metadatos para una petición de estado de archivo en curso.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatusRequest {
    /// Hash MD4 de archivo de 16 bytes solicitado.
    pub file_hash: [u8; 16],
    /// Instante en el que se despachó la petición.
    pub sent_at: Instant,
}

/// Estructura de seguimiento acotada para intercambios `OP_FILESTATUSREQ` pendientes.
#[derive(Debug, Clone)]
pub struct FileStatusTracker {
    pending: HashMap<[u8; 16], FileStatusRequest>,
    max_pending: usize,
}

impl Default for FileStatusTracker {
    fn default() -> Self {
        Self::new(16)
    }
}

impl FileStatusTracker {
    /// Crea un nuevo rastreador con una capacidad máxima en curso.
    pub fn new(max_pending: usize) -> Self {
        Self {
            pending: HashMap::with_capacity(max_pending.min(32)),
            max_pending,
        }
    }

    /// Registra una petición de estado de archivo recién enviada.
    pub fn register_request(
        &mut self,
        file_hash: [u8; 16],
        now: Instant,
    ) -> Result<(), Ed2kSessionError> {
        if self.pending.contains_key(&file_hash) {
            return Err(Ed2kSessionError::DuplicateFileStatusRequest(hex::encode(
                file_hash,
            )));
        }
        if self.pending.len() >= self.max_pending {
            return Err(Ed2kSessionError::TooManyPendingRequests(self.max_pending));
        }
        self.pending.insert(
            file_hash,
            FileStatusRequest {
                file_hash,
                sent_at: now,
            },
        );
        Ok(())
    }

    /// Valida una respuesta entrante y elimina la entrada pendiente.
    pub fn validate_and_remove(
        &mut self,
        file_hash: &[u8; 16],
    ) -> Result<FileStatusRequest, Ed2kSessionError> {
        self.pending
            .remove(file_hash)
            .ok_or_else(|| Ed2kSessionError::UnsolicitedFileStatus(hex::encode(file_hash)))
    }

    /// Comprueba si una petición está actualmente pendiente para el hash especificado.
    pub fn is_pending(&self, file_hash: &[u8; 16]) -> bool {
        self.pending.contains_key(file_hash)
    }

    /// Devuelve el número de peticiones actualmente pendientes.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Elimina y devuelve los hashes de todas las peticiones que superaron el umbral de tiempo de espera.
    pub fn expire_timed_out(&mut self, timeout: Duration, now: Instant) -> Vec<[u8; 16]> {
        let mut expired = Vec::new();
        self.pending.retain(|hash, req| {
            if now.saturating_duration_since(req.sent_at) >= timeout {
                expired.push(*hash);
                false
            } else {
                true
            }
        });
        expired
    }

    /// Borra todas las peticiones pendientes (p. ej., ante una desconexión).
    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

/// Máximo predeterminado de peticiones de bloques simultáneas en curso por par (8).
pub const MAX_IN_FLIGHT_BLOCK_REQUESTS: usize = 8;

/// Clave compuesta que identifica de forma única una petición de bloque eD2K en curso.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct BlockRequestKey {
    /// Hash MD4 de archivo eD2K de 16 bytes.
    pub file_hash: [u8; 16],
    /// Desplazamiento de byte inicial absoluto.
    pub start: u64,
    /// Desplazamiento de byte final absoluto (exclusivo).
    pub end: u64,
}

impl BlockRequestKey {
    /// Crea una nueva `BlockRequestKey`.
    pub fn new(file_hash: [u8; 16], start: u64, end: u64) -> Self {
        Self {
            file_hash,
            start,
            end,
        }
    }

    /// Longitud en bytes del rango de bloque solicitado.
    #[inline]
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// Devuelve verdadero si start >= end.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// Información de seguimiento de una petición de bloque activa en curso.
#[derive(Clone, Debug)]
pub struct PendingBlockRequest {
    /// Clave compuesta única.
    pub key: BlockRequestKey,
    /// Marca de tiempo de cuándo se emitió esta petición.
    pub requested_at: Instant,
    /// Contador de intentos de reintento.
    pub attempt: u16,
    /// Rangos de bytes recibidos hasta ahora para esta petición.
    pub received_ranges: Vec<(u64, u64)>,
}

/// Estructura de seguimiento acotada para intercambios `OP_REQUESTPARTS` pendientes.
#[derive(Debug, Clone)]
pub struct BlockRequestTracker {
    pending: HashMap<BlockRequestKey, PendingBlockRequest>,
    max_pending: usize,
}

impl Default for BlockRequestTracker {
    fn default() -> Self {
        Self::new(MAX_IN_FLIGHT_BLOCK_REQUESTS)
    }
}

impl BlockRequestTracker {
    /// Crea un nuevo rastreador con una capacidad máxima en curso.
    pub fn new(max_pending: usize) -> Self {
        Self {
            pending: HashMap::with_capacity(max_pending.min(32)),
            max_pending,
        }
    }

    /// Devuelve verdadero si el rastreador tiene capacidad para más peticiones.
    #[inline]
    pub fn can_request(&self) -> bool {
        self.pending.len() < self.max_pending
    }

    /// Devuelve el número de peticiones de bloque en curso activas.
    #[inline]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Devuelve verdadero si no hay peticiones de bloque pendientes.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Comprueba si una clave de petición exacta está actualmente pendiente.
    pub fn is_pending(&self, key: &BlockRequestKey) -> bool {
        self.pending.contains_key(key)
    }

    /// Comprueba si alguna petición pendiente para el hash de archivo dado se solapa con [start..end].
    pub fn has_overlap(&self, file_hash: &[u8; 16], start: u64, end: u64) -> bool {
        self.pending
            .keys()
            .any(|k| &k.file_hash == file_hash && k.start < end && start < k.end)
    }

    /// Registra una petición de bloque recién enviada.
    pub fn register_request(
        &mut self,
        key: BlockRequestKey,
        now: Instant,
    ) -> Result<(), Ed2kSessionError> {
        if key.start >= key.end {
            return Err(Ed2kSessionError::InvalidBlockRequest(format!(
                "start offset {} must be strictly less than end offset {}",
                key.start, key.end
            )));
        }

        if self.pending.len() >= self.max_pending {
            return Err(Ed2kSessionError::TooManyPendingBlockRequests(
                self.max_pending,
            ));
        }

        if self.has_overlap(&key.file_hash, key.start, key.end) {
            return Err(Ed2kSessionError::DuplicateBlockRequest(format!(
                "{}:{}-{}",
                hex::encode(key.file_hash),
                key.start,
                key.end
            )));
        }

        self.pending.insert(
            key.clone(),
            PendingBlockRequest {
                key,
                requested_at: now,
                attempt: 1,
                received_ranges: Vec::new(),
            },
        );

        Ok(())
    }

    /// Valida una respuesta de fragmento entrante y elimina la entrada pendiente cuando se ha recibido por completo.
    pub fn validate_and_remove(
        &mut self,
        file_hash: &[u8; 16],
        start: u64,
        end: u64,
    ) -> Result<PendingBlockRequest, Ed2kSessionError> {
        if start >= end {
            return Err(Ed2kSessionError::UnsolicitedBlock(format!(
                "{}:{}-{}",
                hex::encode(file_hash),
                start,
                end
            )));
        }

        let matching_key = self
            .pending
            .keys()
            .find(|k| &k.file_hash == file_hash && start >= k.start && end <= k.end)
            .cloned();

        let key = matching_key.ok_or_else(|| {
            Ed2kSessionError::UnsolicitedBlock(format!(
                "{}:{}-{}",
                hex::encode(file_hash),
                start,
                end
            ))
        })?;

        let is_complete = {
            let req = match self.pending.get_mut(&key) {
                Some(r) => r,
                None => {
                    return Err(Ed2kSessionError::UnsolicitedBlock(format!(
                        "{}:{}-{}",
                        hex::encode(file_hash),
                        start,
                        end
                    )));
                }
            };

            if req
                .received_ranges
                .iter()
                .any(|&(s, e)| s == start && e == end)
            {
                return Err(Ed2kSessionError::DuplicateBlock(format!(
                    "{}:{}-{}",
                    hex::encode(file_hash),
                    start,
                    end
                )));
            }

            req.received_ranges.push((start, end));
            let total_received: u64 = req.received_ranges.iter().map(|&(s, e)| e - s).sum();
            total_received >= key.len()
        };

        if is_complete {
            self.pending.remove(&key).ok_or_else(|| {
                Ed2kSessionError::UnsolicitedBlock(format!(
                    "{}:{}-{}",
                    hex::encode(file_hash),
                    start,
                    end
                ))
            })
        } else {
            self.pending.get(&key).cloned().ok_or_else(|| {
                Ed2kSessionError::UnsolicitedBlock(format!(
                    "{}:{}-{}",
                    hex::encode(file_hash),
                    start,
                    end
                ))
            })
        }
    }

    /// Elimina y devuelve todas las peticiones que superaron el umbral de tiempo de espera.
    pub fn reap_timeouts(&mut self, timeout: Duration, now: Instant) -> Vec<PendingBlockRequest> {
        let mut expired = Vec::new();
        self.pending.retain(|_, req| {
            if now.saturating_duration_since(req.requested_at) >= timeout {
                expired.push(req.clone());
                false
            } else {
                true
            }
        });
        expired
    }

    /// Borra todas las peticiones en curso y las devuelve.
    pub fn cancel_all(&mut self) -> Vec<PendingBlockRequest> {
        self.pending.drain().map(|(_, req)| req).collect()
    }
}

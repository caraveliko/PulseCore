// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Seguimiento y correlación de peticiones de bloques en curso.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use thiserror::Error;

use crate::limits::MAX_IN_FLIGHT_REQUESTS;
use crate::traits::BlockRequest;

/// Errores derivados de violaciones en el seguimiento de peticiones en curso.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum InFlightError {
    /// La tabla de peticiones en curso ha alcanzado la capacidad concurrente máxima (8).
    #[error("Maximum in-flight requests limit ({limit}) reached")]
    CapacityReached {
        /// Recuento de peticiones en curso activas.
        limit: usize,
    },

    /// Ya hay una petición de bloque idéntica pendiente.
    #[error("Duplicate block request for offset {start_offset}")]
    DuplicateRequest {
        /// Desplazamiento inicial en el archivo.
        start_offset: u64,
    },

    /// Se recibieron datos de bloque no solicitados.
    #[error("Unsolicited block data received for offset {start_offset}")]
    UnsolicitedBlock {
        /// Desplazamiento inicial en el archivo.
        start_offset: u64,
    },

    /// Se recibió un rechazo de bloque no solicitado.
    #[error("Unsolicited block rejection received for offset {start_offset}")]
    UnsolicitedRejection {
        /// Desplazamiento inicial en el archivo.
        start_offset: u64,
    },

    /// La longitud del bloque recibido no coincide con el tamaño solicitado.
    #[error("Block length mismatch for offset {start_offset}: expected {expected}, got {actual}")]
    LengthMismatch {
        /// Desplazamiento inicial en el archivo.
        start_offset: u64,
        /// Longitud en bytes esperada.
        expected: u32,
        /// Longitud en bytes real.
        actual: usize,
    },
}

#[derive(Debug, Clone)]
struct InFlightEntry {
    length: u32,
    requested_at: Instant,
}

/// Rastrea y valida las peticiones de bloques de clientes actualmente en curso.
#[derive(Debug, Clone, Default)]
pub struct InFlightTracker {
    pending: HashMap<([u8; 16], u64), InFlightEntry>,
}

impl InFlightTracker {
    /// Crea un nuevo `InFlightTracker` vacío.
    pub fn new() -> Self {
        Self {
            pending: HashMap::with_capacity(MAX_IN_FLIGHT_REQUESTS),
        }
    }

    /// Devuelve verdadero si se puede emitir una nueva petición de bloque.
    pub fn can_request(&self) -> bool {
        self.pending.len() < MAX_IN_FLIGHT_REQUESTS
    }

    /// Devuelve el número de peticiones en curso actualmente activas.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Devuelve verdadero si no hay peticiones pendientes.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Registra una petición de bloque saliente.
    pub fn record_request(&mut self, req: &BlockRequest) -> Result<(), InFlightError> {
        if self.pending.len() >= MAX_IN_FLIGHT_REQUESTS {
            return Err(InFlightError::CapacityReached {
                limit: MAX_IN_FLIGHT_REQUESTS,
            });
        }

        let key = (req.file_hash, req.start_offset);
        if self.pending.contains_key(&key) {
            return Err(InFlightError::DuplicateRequest {
                start_offset: req.start_offset,
            });
        }

        self.pending.insert(
            key,
            InFlightEntry {
                length: req.length,
                requested_at: Instant::now(),
            },
        );
        Ok(())
    }

    /// Valida un bloque entrante frente a las peticiones pendientes y lo consume si es válido.
    pub fn validate_and_remove(
        &mut self,
        file_hash: &[u8; 16],
        start_offset: u64,
        actual_len: usize,
    ) -> Result<(), InFlightError> {
        let key = (*file_hash, start_offset);
        match self.pending.remove(&key) {
            None => Err(InFlightError::UnsolicitedBlock { start_offset }),
            Some(entry) => {
                if entry.length as usize != actual_len {
                    // Reinsertar en caso de discordancia para mantener la coherencia del estado para el informe de fallos
                    self.pending.insert(key, entry.clone());
                    Err(InFlightError::LengthMismatch {
                        start_offset,
                        expected: entry.length,
                        actual: actual_len,
                    })
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Gestiona un rechazo de bloque entrante, liberando capacidad y devolviendo la petición cancelada.
    pub fn handle_rejection(
        &mut self,
        file_hash: &[u8; 16],
        start_offset: u64,
    ) -> Result<BlockRequest, InFlightError> {
        let key = (*file_hash, start_offset);
        match self.pending.remove(&key) {
            None => Err(InFlightError::UnsolicitedRejection { start_offset }),
            Some(entry) => Ok(BlockRequest::new(*file_hash, start_offset, entry.length)),
        }
    }

    /// Identifica y extrae las peticiones de bloque expiradas que superan `timeout`, restaurando la capacidad.
    pub fn reap_timeouts(&mut self, timeout: Duration) -> Vec<BlockRequest> {
        let now = Instant::now();
        let mut expired_keys = Vec::new();

        for (key, entry) in &self.pending {
            if now.duration_since(entry.requested_at) >= timeout {
                expired_keys.push(*key);
            }
        }

        let mut expired_requests = Vec::with_capacity(expired_keys.len());
        for key in expired_keys {
            if let Some(entry) = self.pending.remove(&key) {
                expired_requests.push(BlockRequest::new(key.0, key.1, entry.length));
            }
        }

        expired_requests
    }

    /// Cancela todas las peticiones en curso pendientes (p. ej., ante la desconexión del par), restaurando la capacidad total.
    pub fn cancel_all(&mut self) -> Vec<BlockRequest> {
        let mut requests = Vec::with_capacity(self.pending.len());
        for (key, entry) in self.pending.drain() {
            requests.push(BlockRequest::new(key.0, key.1, entry.length));
        }
        requests
    }

    /// Borra todas las peticiones pendientes.
    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

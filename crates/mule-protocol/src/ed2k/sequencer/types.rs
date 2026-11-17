// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Tipos de error y seguimiento para la secuenciación y planificación de bloques eD2K.

/// Errores que surgen durante la secuenciación de descargas y la planificación de peticiones de bloques.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Ed2kSequencerError {
    /// El par remoto informó de que el archivo no está compartido o es desconocido mediante `OP_FILEREQANSNOFILE` (`0x48`).
    #[error("File is not shared or known by remote peer: {0}")]
    FileNotFound(String),

    /// El tamaño del archivo es cero.
    #[error("File size is 0 bytes")]
    EmptyFile,

    /// El hash del archivo solicitado no coincide con el hash de estado.
    #[error("Hash mismatch: expected {expected}, got {actual}")]
    HashMismatch {
        /// Hexadecimal del hash de archivo esperado.
        expected: String,
        /// Hexadecimal del hash de archivo de estado real.
        actual: String,
    },

    /// El índice de parte está fuera de límites para el archivo.
    #[error("Part index {part_index} is out of bounds (total parts: {total_parts})")]
    PartIndexOutOfBounds {
        /// Índice de parte solicitado.
        part_index: u16,
        /// Número total de partes en el archivo.
        total_parts: u16,
    },

    /// El índice de parte no está disponible en la fuente remota.
    #[error(
        "Part index {part_index} is not available from remote source (total parts: {total_parts})"
    )]
    PartNotAvailable {
        /// Índice de parte solicitado.
        part_index: u16,
        /// Número total de partes en el archivo.
        total_parts: u16,
    },

    /// Parámetros de rango de bloque no válidos.
    #[error("Invalid block range [{start}, {end}): {reason}")]
    InvalidBlockRange {
        /// Desplazamiento de byte inicial.
        start: u64,
        /// Desplazamiento de byte final (exclusivo).
        end: u64,
        /// Explicación del fallo.
        reason: String,
    },

    /// El rango de bloque cruza indebidamente el límite de una parte eD2K.
    #[error("Block range [{start}, {end}) crosses part boundary at {part_boundary}")]
    CrossPartBoundary {
        /// Desplazamiento de byte inicial.
        start: u64,
        /// Desplazamiento de byte final (exclusivo).
        end: u64,
        /// Desplazamiento en bytes del límite de parte cruzado.
        part_boundary: u64,
    },

    /// El número de rangos solicitados supera el máximo del protocolo (3).
    #[error("Requested {count} ranges exceeds maximum allowed {max} ranges per packet")]
    TooManyRanges {
        /// Número de rangos especificados.
        count: usize,
        /// Máximo de rangos permitidos.
        max: usize,
    },

    /// Cero rangos especificados para la petición de bloque.
    #[error("Zero ranges specified for block request")]
    EmptyRanges,

    /// Fallo de codificación en cable.
    #[error("Encoding error: {0}")]
    Encode(String),

    /// La longitud de los datos de carga útil del bloque no coincide con los límites del rango especificado.
    #[error("Block data length mismatch: expected {expected} bytes, got {actual} bytes")]
    BlockDataMismatch {
        /// Recuento de bytes esperado (`end - start`).
        expected: usize,
        /// Recuento de bytes de carga útil real.
        actual: usize,
    },

    /// El rango de bloque ya se había recibido y escrito.
    #[error("Duplicate block received for range [{start}, {end})")]
    DuplicateBlock {
        /// Desplazamiento de byte inicial.
        start: u64,
        /// Desplazamiento de byte final (exclusivo).
        end: u64,
    },

    /// Se recibió una trama comprimida cuando se esperaba una transferencia de bloque sin comprimir.
    #[error("Compressed block transfer is not negotiated or supported in passive profile")]
    CompressedNotSupported,

    /// Error de almacenamiento, sistema de archivos o serialización.
    #[error("Storage error: {0}")]
    Storage(String),
}

/// Representa un rango de bloque activo en curso asignado a un par específico.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InFlightBlock {
    /// Identificador o dirección del par propietario.
    pub peer_id: String,
    /// Índice de parte de destino.
    pub part_index: u16,
    /// Desplazamiento de byte inicial semiabierto.
    pub start: u64,
    /// Desplazamiento de byte final semiabierto (exclusivo).
    pub end: u64,
    /// Marca de tiempo de cuándo se solicitó el bloque (ms unix).
    pub issued_at_unix_ms: i64,
    /// Plazo de expiración tras el cual el bloque puede reasignarse (ms unix).
    pub deadline_unix_ms: i64,
}

impl InFlightBlock {
    /// Crea una nueva entrada de seguimiento de bloque en curso.
    pub fn new(
        peer_id: String,
        part_index: u16,
        start: u64,
        end: u64,
        issued_at_unix_ms: i64,
        timeout_ms: i64,
    ) -> Self {
        Self {
            peer_id,
            part_index,
            start,
            end,
            issued_at_unix_ms,
            deadline_unix_ms: issued_at_unix_ms + timeout_ms,
        }
    }

    /// Devuelve verdadero si el rango de bloque ha expirado en `now_unix_ms`.
    pub fn is_expired(&self, now_unix_ms: i64) -> bool {
        now_unix_ms >= self.deadline_unix_ms
    }

    /// Comprueba si este bloque en curso se solapa con el rango `[s, e)`.
    pub fn overlaps(&self, s: u64, e: u64) -> bool {
        self.start < e && s < self.end
    }
}

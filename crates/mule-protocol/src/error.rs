// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Errores de protocolo y fallos de entramado.

use thiserror::Error;

/// Errores que ocurren durante el entramado de paquetes, segmentación de límites o decodificación de flujo.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum FramingError {
    /// El campo de longitud del paquete excede el máximo permitido.
    #[error("Frame size {size} bytes exceeds maximum limit of {limit} bytes")]
    FrameTooLarge {
        /// Tamaño real analizado en bytes.
        size: usize,
        /// Tamaño máximo permitido en bytes.
        limit: usize,
    },

    /// El campo de longitud del paquete es demasiado pequeño (debe ser de al menos 1 para el código de operación).
    #[error("Frame size {size} bytes is too small (must be at least 1 byte for opcode)")]
    FrameTooSmall {
        /// Tamaño real analizado en bytes.
        size: usize,
    },

    /// El búfer acumulador superó el umbral de seguridad antes de que llegara una trama completa.
    #[error("Connection buffer overflow: {buffered} bytes accumulated without complete frame (limit {limit})")]
    BufferOverflow {
        /// Bytes actualmente en búfer.
        buffered: usize,
        /// Capacidad máxima admisible del búfer.
        limit: usize,
    },

    /// Byte de prefijo de protocolo desconocido o no compatible.
    #[error("Unsupported protocol byte 0x{protocol:02X}")]
    UnsupportedProtocol {
        /// Valor del byte de protocolo rechazado.
        protocol: u8,
    },

    /// Fallo de E/S de bajo nivel durante el entramado.
    #[error("Framing I/O error: {0}")]
    Io(String),
}

/// Errores de dominio y protocolo de cable de alto nivel.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum ProtocolError {
    /// Fallo de análisis o codificación a nivel de entramado.
    #[error(transparent)]
    Framing(#[from] FramingError),

    /// El código de operación recibido no coincide con la transición esperada de la máquina de estados.
    #[error("Unexpected opcode 0x{got:02X} (expected 0x{expected:02X})")]
    UnexpectedOpcode {
        /// Byte del código de operación esperado.
        expected: u8,
        /// Byte del código de operación real recibido.
        got: u8,
    },

    /// Datos de carga útil del paquete mal formados.
    #[error("Malformed packet payload: {0}")]
    MalformedPayload(String),

    /// La longitud de la cadena superó el límite máximo permitido.
    #[error("String length {len} exceeds limit of {limit} bytes")]
    OversizedString {
        /// Longitud en bytes.
        len: usize,
        /// Límite máximo permitido en bytes.
        limit: usize,
    },

    /// El número de etiquetas superó el recuento máximo permitido.
    #[error("Tag count {count} exceeds limit of {limit}")]
    TooManyTags {
        /// Recuento de etiquetas recibido.
        count: usize,
        /// Recuento máximo de etiquetas permitido.
        limit: usize,
    },

    /// La especificación de desplazamiento o rango es inválida o está fuera de límites.
    #[error("Invalid range [{start}..{end}] for file size {file_size}")]
    InvalidRange {
        /// Desplazamiento de inicio del rango.
        start: u64,
        /// Desplazamiento de fin del rango.
        end: u64,
        /// Tamaño total del archivo.
        file_size: u64,
    },

    /// El par cerró la conexión o se agotó el tiempo de espera.
    #[error("Peer disconnected: {0}")]
    PeerDisconnected(String),

    /// Error del sumidero de almacenamiento de descargas.
    #[error("Download sink error: {0}")]
    Sink(String),

    /// Error del planificador de descargas.
    #[error("Download planner error: {0}")]
    Planner(String),

    /// Trama comprimida (0xD4) rechazada a la espera de la etapa de descompresión acotada.
    #[error("Compressed frame 0xD4 (PR_EMULE_COMPRESSED) is not supported prior to bounded decompression stage")]
    CompressedProtocolUnsupported,
}

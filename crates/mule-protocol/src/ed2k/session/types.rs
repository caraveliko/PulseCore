// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Tipos y definiciones de error para sesiones eD2K.

use std::time::Duration;

use super::super::error::{Ed2kEncodeError, Ed2kParseError};
use super::super::peer_info::Ed2kPeerInfo;

/// Categoría de una violación de protocolo o fallo de sesión eD2K.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Ed2kSessionError {
    /// Llegó un mensaje inesperado para el estado actual de la sesión.
    #[error("Unexpected opcode 0x{opcode:02X} in state {state}")]
    UnexpectedMessageInState {
        /// Nombre del estado actual.
        state: &'static str,
        /// Código de operación recibido.
        opcode: u8,
    },

    /// Se recibió Hello más de una vez.
    #[error("Double Hello rejected: session already initialized")]
    DoubleHello,

    /// Se recibió HelloAnswer cuando se esperaba un Hello primero.
    #[error("HelloAnswer received before Hello")]
    HelloAnswerBeforeHello,

    /// Falta una etiqueta obligatoria o no es válida.
    #[error("Missing mandatory tag: {0}")]
    MissingRequiredTag(&'static str),

    /// Versión o capacidades del par incompatibles.
    #[error("Incompatible peer version")]
    IncompatiblePeer,

    /// El par remoto proporcionó un hash de usuario no válido (todo ceros).
    #[error("Invalid user hash: cannot be all zeros")]
    ZeroUserHash,

    /// Intento de enviar una petición antes de que la sesión entrara en el estado Ready.
    #[error("Cannot send request before session is Ready (current state: {0})")]
    RequestBeforeReady(&'static str),

    /// Se recibió una respuesta antes de que la sesión entrara en el estado Ready.
    #[error("Received response before session is Ready (current state: {0})")]
    ResponseBeforeReady(&'static str),

    /// Se recibió un estado de archivo sin una petición en curso o con un hash no coincidente.
    #[error("Unsolicited file status received for hash {0}")]
    UnsolicitedFileStatus(String),

    /// El par remoto informó de que no tiene el archivo solicitado (OP_FILEREQANSNOFILE).
    #[error("Remote peer does not have requested file: {0}")]
    FileNotFound(String),

    /// Se inició una petición de estado de archivo duplicada para un hash de archivo que ya estaba en curso.
    #[error("Duplicate file status request for hash {0}")]
    DuplicateFileStatusRequest(String),

    /// Las peticiones en curso superaron la capacidad máxima permitida.
    #[error("Too many pending file status requests (capacity: {0})")]
    TooManyPendingRequests(usize),

    /// Se recibió un bloque sin una petición en curso o con hash/rango no coincidente.
    #[error("Unsolicited block received: {0}")]
    UnsolicitedBlock(String),

    /// Se inició una petición de bloque duplicada o superpuesta.
    #[error("Duplicate or overlapping block request: {0}")]
    DuplicateBlockRequest(String),

    /// Se recibió un fragmento de bloque duplicado.
    #[error("Duplicate block received: {0}")]
    DuplicateBlock(String),

    /// Las peticiones de bloque en curso superaron la capacidad máxima permitida.
    #[error("Too many pending block requests (capacity: {0})")]
    TooManyPendingBlockRequests(usize),

    /// Expiró el tiempo de espera de la petición de bloque.
    #[error("Block request timed out: {0}")]
    BlockTimeout(String),

    /// Parámetros de petición de bloque no válidos.
    #[error("Invalid block request: {0}")]
    InvalidBlockRequest(String),

    /// Estructura de paquete mal formada.
    #[error("Malformed packet: {0}")]
    Malformed(String),

    /// Fallo de análisis en el búfer de cable entrante.
    #[error("Parse error: {0}")]
    Parse(String),

    /// Byte de protocolo de cable inesperado o no compatible.
    #[error("Unexpected protocol byte 0x{protocol:02X} (expected 0xE3 PR_ED2K or 0xC5 PR_EMULE)")]
    UnexpectedProtocol {
        /// Byte de protocolo rechazado.
        protocol: u8,
    },

    /// Trama comprimida 0xD4 (PR_EMULE_COMPRESSED) no admitida.
    #[error("Compressed protocol 0xD4 (PR_EMULE_COMPRESSED) is not supported")]
    CompressedProtocolUnsupported,

    /// Expiró el tiempo de espera de negociación del protocolo de enlace.
    #[error("Handshake timeout after {0:?}")]
    Timeout(Duration),

    /// Expiró el tiempo de espera de la petición de estado del archivo.
    #[error("File status request timed out for hash {0}")]
    FileStatusTimeout(String),

    /// Reinicio de conexión o fallo de E/S.
    #[error("I/O error: {0}")]
    Io(String),
}

impl From<Ed2kParseError> for Ed2kSessionError {
    fn from(err: Ed2kParseError) -> Self {
        Self::Parse(err.to_string())
    }
}

impl From<Ed2kEncodeError> for Ed2kSessionError {
    fn from(err: Ed2kEncodeError) -> Self {
        Self::Malformed(err.to_string())
    }
}

/// Estados del ciclo de vida de una sesión entre pares eD2K.
#[derive(Debug, Clone, PartialEq)]
pub enum Ed2kSessionState {
    /// Estado inicial desconectado.
    Disconnected,
    /// Estableciendo activamente la conexión de transporte.
    Connecting,
    /// Oyente a la espera del `OP_HELLO` del iniciador.
    AwaitingHello,
    /// Iniciador a la espera del `OP_HELLO_ANSWER` del receptor.
    AwaitingHelloAnswer {
        /// Hash de usuario local que identifica esta instancia de cliente en el protocolo de enlace.
        local_handshake_id: [u8; 16],
    },
    /// Protocolo de enlace completado con éxito, listo para operaciones de pares.
    Ready {
        /// Identidad y metadatos del par remoto negociados.
        peer: Ed2kPeerInfo,
    },
    /// Cerrando la sesión de forma ordenada.
    Closing,
    /// Sesión terminada limpiamente.
    Closed,
    /// Estado de error terminal irreversible.
    Failed {
        /// Motivo del fallo.
        reason: Ed2kSessionError,
    },
}

impl Ed2kSessionState {
    /// Devuelve una cadena de texto legible para mostrar este estado.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Connecting => "Connecting",
            Self::AwaitingHello => "AwaitingHello",
            Self::AwaitingHelloAnswer { .. } => "AwaitingHelloAnswer",
            Self::Ready { .. } => "Ready",
            Self::Closing => "Closing",
            Self::Closed => "Closed",
            Self::Failed { .. } => "Failed",
        }
    }
}

/// Instantánea determinista pura que captura el estado del protocolo y del rastreador de una sesión.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSnapshot {
    /// Nombre del estado del ciclo de vida.
    pub state_name: &'static str,
    /// Si la sesión está completamente establecida y lista para la transferencia de datos.
    pub is_ready: bool,
    /// Recuento de peticiones de estado de archivo en curso activas.
    pub pending_status_count: usize,
    /// Recuento de peticiones de bloque en curso activas.
    pub pending_block_count: usize,
    /// Recuento de paquetes no gestionados o sin procesar recibidos.
    pub unhandled_packets_count: usize,
}

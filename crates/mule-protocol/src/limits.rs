// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Límites del protocolo y constantes para una gestión de red segura y determinista.

/// Tamaño de cabecera de un paquete eD2K (1 byte de protocolo + 4 bytes de longitud little-endian).
pub const HEADER_SIZE: usize = 5;

/// Tamaño de trama máximo permitido (campo de longitud) en bytes (1 MiB).
///
/// Los bloques eD2K estándar son de 180 KiB (184.320 bytes), por lo que 1 MiB proporciona suficiente margen
/// a la vez que previene por completo el agotamiento de memoria ante campos de longitud maliciosos.
pub const MAX_FRAME_BYTES: usize = 1_048_576;

/// Capacidad máxima de búfer permitida por conexión antes de desconectar forzosamente al par (2 MiB).
pub const MAX_BUFFERED_BYTES: usize = 2_097_152;

/// Longitud máxima de cadena permitida en la carga útil de cualquier etiqueta (16 KiB).
pub const MAX_STRING_BYTES: usize = 16_384;

/// Número máximo de etiquetas permitidas en un único mensaje.
pub const MAX_TAGS_PER_MESSAGE: usize = 256;

/// Número máximo de pares devueltos en una única respuesta de fuentes.
pub const MAX_PEERS_PER_RESPONSE: usize = 500;

/// Tamaño estándar de una petición/transferencia de bloque eD2K (180 KiB = 184.320 bytes).
pub const ED2K_BLOCK_SIZE: usize = 184_320;

/// Tamaño estándar de una parte eD2K (9.500 KiB = 9.728.000 bytes).
pub const ED2K_PART_SIZE: usize = 9_728_000;

/// Número máximo de peticiones de bloque en curso activas por sesión de par.
pub const MAX_IN_FLIGHT_REQUESTS: usize = 8;

/// Tamaño máximo de transferencia de bloque (180 KiB = 184.320 bytes).
pub const MAX_BLOCK_BYTES: usize = ED2K_BLOCK_SIZE;

/// Tiempo de espera de negociación del protocolo de enlace (5 segundos).
pub const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Tiempo de espera de inactividad de conexión (30 segundos).
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Máximo de violaciones de protocolo toleradas antes de cerrar la sesión (fallo inmediato).
pub const MAX_PROTOCOL_VIOLATIONS: u8 = 1;

/// Rasgo que define límites de recursos contextuales y según el código de operación para mensajes del protocolo.
pub trait PacketLimits: Send + Sync {
    /// Tamaño máximo de trama en cable permitido para un byte de protocolo y código de operación dados.
    fn max_wire_frame_bytes(&self, protocol: u8, opcode: u8) -> usize;

    /// Tamaño máximo de carga útil decodificada permitido tras el entramado.
    fn max_decoded_payload_bytes(&self, protocol: u8, opcode: u8) -> usize;

    /// Longitud máxima de cadena permitida dentro de las etiquetas para el código de operación especificado.
    fn max_string_bytes(&self, opcode: u8) -> usize;
}

/// Implementación predeterminada de límites globales de paquetes.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultPacketLimits;

impl PacketLimits for DefaultPacketLimits {
    fn max_wire_frame_bytes(&self, _protocol: u8, _opcode: u8) -> usize {
        MAX_FRAME_BYTES
    }

    fn max_decoded_payload_bytes(&self, _protocol: u8, _opcode: u8) -> usize {
        MAX_FRAME_BYTES
    }

    fn max_string_bytes(&self, _opcode: u8) -> usize {
        MAX_STRING_BYTES
    }
}

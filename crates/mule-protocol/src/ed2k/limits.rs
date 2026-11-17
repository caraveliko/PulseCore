// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Umbrales de seguridad y límites de recursos dedicados al análisis de red eD2K.

/// Tamaño de trama máximo predeterminado para paquetes eD2K generales (256 KiB).
pub const ED2K_MAX_FRAME_DEFAULT: usize = 256 * 1024;

/// Tamaño de trama máximo permitido específicamente para `OP_HELLO` y `OP_HELLO_ANSWER` (64 KiB).
pub const ED2K_MAX_HELLO_FRAME: usize = 64 * 1024;

/// Tamaño de trama máximo permitido para paquetes de estado / conjunto de hashes (64 KiB).
pub const ED2K_MAX_STATUS_FRAME: usize = 64 * 1024;

/// Número máximo de etiquetas permitido en un único mensaje (256).
pub const ED2K_MAX_TAGS: usize = 256;

/// Longitud máxima en bytes permitida para cualquier valor o nombre de etiqueta de texto (1 KiB).
pub const ED2K_MAX_STRING_BYTES: usize = 1024;

/// Longitud máxima en bytes permitida para cadenas de servidor como MOTD / banner de identificación (64 KiB).
pub const ED2K_MAX_SERVER_STRING_BYTES: usize = 64 * 1024;

/// Número máximo de partes de fichero en un mapa de bits de estado (65.530 partes).
pub const ED2K_MAX_PARTS: usize = 65_530;

/// Longitud máxima de blob permitida en una etiqueta (64 KiB).
pub const ED2K_MAX_BLOB_BYTES: usize = 64 * 1024;

/// Límites de recursos dedicados para la decodificación de eD2K.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kLimits {
    /// Tamaño de trama máximo admisible (código de operación + carga útil) para paquetes estándar.
    pub max_frame_bytes: usize,
    /// Tamaño de trama máximo admisible para paquetes de saludo / negociación inicial.
    pub max_hello_frame_bytes: usize,
    /// Número máximo de etiquetas permitidas en un único paquete.
    pub max_tags: usize,
    /// Longitud máxima admisible en bytes para cadenas.
    pub max_string_bytes: usize,
    /// Número máximo de partes de fichero permitidas en mensajes de estado.
    pub max_parts: usize,
    /// Longitud máxima admisible en bytes para blobs de etiquetas.
    pub max_blob_bytes: usize,
}

impl Default for Ed2kLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: ED2K_MAX_FRAME_DEFAULT,
            max_hello_frame_bytes: ED2K_MAX_HELLO_FRAME,
            max_tags: ED2K_MAX_TAGS,
            max_string_bytes: ED2K_MAX_STRING_BYTES,
            max_parts: ED2K_MAX_PARTS,
            max_blob_bytes: ED2K_MAX_BLOB_BYTES,
        }
    }
}

impl Ed2kLimits {
    /// Crea una nueva estructura `Ed2kLimits` con valores predeterminados.
    pub fn new() -> Self {
        Self::default()
    }

    /// Crea límites ajustados para la comunicación con servidores eD2K, permitiendo cadenas MOTD / identificación más grandes.
    pub fn for_server() -> Self {
        Self {
            max_string_bytes: ED2K_MAX_SERVER_STRING_BYTES,
            ..Self::default()
        }
    }

    /// Devuelve el tamaño máximo de trama permitido para un código de operación específico.
    pub fn max_frame_for_opcode(&self, opcode: u8) -> usize {
        match opcode {
            // El código de operación 0x01 cubre OP_HELLO (PR_ED2K 0xE3) y OP_EMULE_INFO (PR_EMULE 0xC5)
            super::opcodes::OP_HELLO | super::opcodes::OP_HELLO_ANSWER => {
                self.max_hello_frame_bytes
            }
            super::opcodes::OP_FILESTATUS | super::opcodes::OP_HASHSETANSWER => {
                ED2K_MAX_STATUS_FRAME
            }
            _ => self.max_frame_bytes,
        }
    }
}

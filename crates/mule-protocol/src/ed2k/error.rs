// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Tipos de error estructurados y contextuales para el análisis de eD2K.

use std::fmt;
use thiserror::Error;

/// Categoría específica de un fallo de análisis sintáctico de eD2K.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ed2kErrorKind {
    /// Fin inesperado del búfer de entrada al leer un campo obligatorio.
    UnexpectedEof {
        /// Número de bytes necesarios.
        needed: usize,
        /// Número de bytes realmente disponibles.
        available: usize,
    },
    /// El byte de protocolo no se reconoce como un eD2K o eMule válido.
    UnsupportedProtocol {
        /// Valor del byte de protocolo rechazado.
        byte: u8,
    },
    /// El protocolo comprimido (0xD4) se rechaza explícitamente a la espera de la etapa de descompresión acotada.
    CompressedProtocolUnsupported,
    /// El campo de longitud de trama es menor que el mínimo de 1 byte para el código de operación.
    FrameTooSmall {
        /// Longitud de trama analizada.
        size: usize,
    },
    /// El tamaño de trama supera el límite de seguridad configurado.
    FrameTooLarge {
        /// Longitud de trama analizada.
        size: usize,
        /// Techo de seguridad.
        limit: usize,
    },
    /// Un campo de texto supera la longitud máxima configurada.
    OversizedString {
        /// Longitud en bytes de la cadena.
        size: usize,
        /// Límite de seguridad.
        limit: usize,
    },
    /// El número de etiquetas supera el límite máximo admisible.
    TooManyTags {
        /// Recuento declarado de etiquetas.
        count: usize,
        /// Límite de seguridad.
        limit: usize,
    },
    /// El byte de tipo de etiqueta no está gestionado o no es válido.
    InvalidTagType {
        /// Byte de tipo de etiqueta.
        tag_type: u8,
    },
    /// Campo malformado o violación de restricción semántica.
    MalformedField {
        /// Mensaje de diagnóstico.
        message: String,
    },
    /// Los bits finales en el último byte de un mapa de bits de disponibilidad no son cero.
    InvalidBitfieldTrailingBits {
        /// Resto del recuento de partes módulo 8.
        remainder: usize,
        /// Valor real de la máscara de bits finales encontrado.
        trailing_bits: u8,
    },
    /// El desplazamiento de inicio del bloque es mayor o igual que el desplazamiento de fin.
    InvalidBlockOffsets {
        /// Desplazamiento de byte de inicio.
        start: u64,
        /// Desplazamiento de byte de fin.
        end: u64,
    },
    /// La longitud en bytes de la carga útil del bloque no coincide con el rango declarado (end - start).
    BlockPayloadMismatch {
        /// Longitud en bytes esperada basada en (end - start).
        expected: usize,
        /// Longitud en bytes real en la carga útil de la trama.
        actual: usize,
    },
    /// Un código de operación es ambiguo o no válido para el perfil de compatibilidad activo.
    AmbiguousOpcodeForProfile {
        /// El código de operación que no pudo desambiguarse.
        opcode: u8,
        /// El perfil de compatibilidad activo durante la decodificación.
        profile: super::profile::Ed2kCompatibilityProfile,
        /// Descripción del motivo / contexto.
        reason: &'static str,
    },
}

impl fmt::Display for Ed2kErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBitfieldTrailingBits {
                remainder,
                trailing_bits,
            } => {
                write!(
                    f,
                    "invalid bitfield trailing bits: remainder {remainder}, trailing bits 0x{trailing_bits:02X} must be zero"
                )
            }
            Self::InvalidBlockOffsets { start, end } => {
                write!(
                    f,
                    "invalid block offsets: start offset {start} must be strictly less than end offset {end}"
                )
            }
            Self::BlockPayloadMismatch { expected, actual } => {
                write!(
                    f,
                    "block payload length mismatch: expected {expected} bytes, but got {actual}"
                )
            }
            Self::UnexpectedEof { needed, available } => {
                write!(
                    f,
                    "unexpected EOF: needed {needed} bytes, but only {available} available"
                )
            }
            Self::UnsupportedProtocol { byte } => {
                write!(f, "unsupported protocol byte 0x{byte:02X}")
            }
            Self::CompressedProtocolUnsupported => {
                write!(
                    f,
                    "compressed protocol 0xD4 (PR_EMULE_COMPRESSED) is not supported prior to bounded decompression stage"
                )
            }
            Self::FrameTooSmall { size } => {
                write!(f, "frame size {size} is too small (minimum 1 for opcode)")
            }
            Self::FrameTooLarge { size, limit } => {
                write!(f, "frame size {size} exceeds limit of {limit} bytes")
            }
            Self::OversizedString { size, limit } => {
                write!(f, "string length {size} exceeds limit of {limit} bytes")
            }
            Self::TooManyTags { count, limit } => {
                write!(f, "tag count {count} exceeds limit of {limit}")
            }
            Self::InvalidTagType { tag_type } => {
                write!(f, "invalid tag type byte 0x{tag_type:02X}")
            }
            Self::MalformedField { message } => {
                write!(f, "malformed field: {message}")
            }
            Self::AmbiguousOpcodeForProfile {
                opcode,
                profile,
                reason,
            } => {
                write!(
                    f,
                    "opcode 0x{opcode:02X} is ambiguous or invalid under compatibility profile {profile}: {reason}"
                )
            }
        }
    }
}

/// Error enriquecido y contextual para fallos de decodificación de red eD2K.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub struct Ed2kParseError {
    /// Byte de protocolo en cable (p. ej. 0xE3, 0xC5, 0xD4).
    pub protocol: u8,
    /// Código de operación si se conoce en el momento del error.
    pub opcode: Option<u8>,
    /// Desplazamiento de byte en el búfer de entrada donde ocurrió el fallo.
    pub offset: usize,
    /// Contexto de alto nivel o nombre de campo donde falló el análisis sintáctico.
    pub context: &'static str,
    /// Categoría específica de fallo.
    pub kind: Ed2kErrorKind,
}

impl Ed2kParseError {
    /// Crea un nuevo `Ed2kParseError`.
    pub fn new(
        protocol: u8,
        opcode: Option<u8>,
        offset: usize,
        context: &'static str,
        kind: Ed2kErrorKind,
    ) -> Self {
        Self {
            protocol,
            opcode,
            offset,
            context,
            kind,
        }
    }
}

impl fmt::Display for Ed2kParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "eD2K parse error [protocol=0x{:02X}, opcode={:?}, offset={}, context={}]: {}",
            self.protocol,
            self.opcode.map(|op| format!("0x{op:02X}")),
            self.offset,
            self.context,
            self.kind
        )
    }
}

/// Error enriquecido y contextual para fallos de codificación de red eD2K.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub struct Ed2kEncodeError {
    /// Código de operación que se está codificando.
    pub opcode: u8,
    /// Contexto de alto nivel o nombre de campo donde falló la codificación.
    pub field: &'static str,
    /// Categoría específica de fallo.
    pub kind: Ed2kErrorKind,
}

impl Ed2kEncodeError {
    /// Crea un nuevo `Ed2kEncodeError`.
    pub fn new(opcode: u8, field: &'static str, kind: Ed2kErrorKind) -> Self {
        Self {
            opcode,
            field,
            kind,
        }
    }
}

impl fmt::Display for Ed2kEncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "eD2K encode error [opcode=0x{:02X}, field={}]: {}",
            self.opcode, self.field, self.kind
        )
    }
}

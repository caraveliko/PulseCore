// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Estructuras de consulta y respuesta de fuentes para servidores eD2K.

use crate::ed2k::error::{Ed2kEncodeError, Ed2kErrorKind};
use crate::ed2k::limits::Ed2kLimits;
use crate::ed2k::opcodes::{OP_GETSOURCES, OP_GETSOURCES_OBFU};
use crate::ed2k::protocol::PR_ED2K;
use crate::framing::RawPacket;

/// Petición para encontrar fuentes de un archivo específico (`OP_GETSOURCES` = `0x19` o `OP_GETSOURCES_OBFU` = `0x23`).
#[derive(Debug, Clone, PartialEq)]
pub struct GetSourcesRequest {
    /// Hash MD4 de archivo de 16 bytes.
    pub file_hash: [u8; 16],
    /// Tamaño del archivo en bytes.
    pub file_size: u64,
    /// Si se solicitan fuentes con parámetros de ofuscación (`OP_GETSOURCES_OBFU` = `0x23`).
    pub obfuscation: bool,
}

impl GetSourcesRequest {
    /// Crea un nuevo GetSourcesRequest (OP_GETSOURCES = 0x19 en texto plano heredado).
    pub fn new(file_hash: [u8; 16], file_size: u64) -> Self {
        Self {
            file_hash,
            file_size,
            obfuscation: false,
        }
    }

    /// Crea un nuevo GetSourcesRequest con conmutador de ofuscación (0x23 si es true, 0x19 si es false).
    pub fn with_obfuscation(file_hash: [u8; 16], file_size: u64, obfuscation: bool) -> Self {
        Self {
            file_hash,
            file_size,
            obfuscation,
        }
    }

    /// Codifica la petición en un `RawPacket`.
    pub fn encode(&self, limits: &Ed2kLimits) -> Result<RawPacket, Ed2kEncodeError> {
        let opcode = if self.obfuscation {
            OP_GETSOURCES_OBFU
        } else {
            OP_GETSOURCES
        };
        let mut buf = Vec::with_capacity(24);
        buf.extend_from_slice(&self.file_hash);

        if self.file_size <= u32::MAX as u64 {
            buf.extend_from_slice(&(self.file_size as u32).to_le_bytes());
        } else {
            buf.extend_from_slice(&0u32.to_le_bytes());
            buf.extend_from_slice(&self.file_size.to_le_bytes());
        }

        let payload_len = buf.len() + 1;
        if payload_len > limits.max_frame_bytes {
            return Err(Ed2kEncodeError::new(
                opcode,
                "get_sources",
                Ed2kErrorKind::FrameTooLarge {
                    size: payload_len,
                    limit: limits.max_frame_bytes,
                },
            ));
        }

        Ok(RawPacket::with_vec(PR_ED2K, opcode, buf))
    }
}

/// Entrada de fuente de par devuelta por el servidor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundSourcePeer {
    /// Dirección IPv4 en orden de bytes de red o entero de host.
    pub ip: u32,
    /// Número de puerto.
    pub port: u16,
}

impl FoundSourcePeer {
    /// Convierte la dirección de la fuente al formato de cadena "A.B.C.D:puerto".
    pub fn to_address_string(&self) -> String {
        let bytes = self.ip.to_le_bytes();
        format!(
            "{}.{}.{}.{}:{}",
            bytes[0], bytes[1], bytes[2], bytes[3], self.port
        )
    }
}

/// Respuesta de fuentes encontradas del servidor (`OP_FOUNDSOURCES` = `0x42`).
#[derive(Debug, Clone, PartialEq)]
pub struct FoundSources {
    /// Hash MD4 de archivo de 16 bytes.
    pub file_hash: [u8; 16],
    /// Lista de fuentes.
    pub sources: Vec<FoundSourcePeer>,
}

/// Entrada de fuente de par con indicadores de ofuscación y UserHash opcional (`OP_FOUNDSOURCES_OBFU` = `0x44`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundSourcePeerObfu {
    /// Dirección IPv4 en orden de bytes de red / entero de host.
    pub ip: u32,
    /// Número de puerto.
    pub port: u16,
    /// Byte de configuración de ofuscación.
    pub obf_settings: u8,
    /// Hash de usuario opcional de 16 bytes (presente si obf_settings & 0x80 != 0).
    pub user_hash: Option<[u8; 16]>,
}

impl FoundSourcePeerObfu {
    /// Convierte la dirección de la fuente al formato de cadena "A.B.C.D:puerto".
    pub fn to_address_string(&self) -> String {
        let bytes = self.ip.to_le_bytes();
        format!(
            "{}.{}.{}.{}:{}",
            bytes[0], bytes[1], bytes[2], bytes[3], self.port
        )
    }

    /// Si este par admite ofuscación de protocolo (bit 0).
    #[inline]
    pub fn supports_crypt(&self) -> bool {
        (self.obf_settings & 0x01) != 0
    }

    /// Si este par solicita ofuscación de protocolo (bit 1).
    #[inline]
    pub fn requests_crypt(&self) -> bool {
        (self.obf_settings & 0x02) != 0
    }

    /// Si este par requiere estrictamente ofuscación de protocolo (bit 2).
    #[inline]
    pub fn requires_crypt(&self) -> bool {
        (self.obf_settings & 0x04) != 0
    }
}

/// Respuesta de fuentes encontradas con ofuscación del servidor (`OP_FOUNDSOURCES_OBFU` = `0x44`).
#[derive(Debug, Clone, PartialEq)]
pub struct FoundSourcesObfu {
    /// Hash MD4 de archivo de 16 bytes.
    pub file_hash: [u8; 16],
    /// Lista de fuentes con parámetros de ofuscación.
    pub sources: Vec<FoundSourcePeerObfu>,
}

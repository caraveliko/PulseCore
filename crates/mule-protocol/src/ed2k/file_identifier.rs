// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! `FileIdentifier` — identificación compuesta de un fichero introducida en eMule 0.50
//! y generalizada en 0.72a (`FileIdentifier.h` / `FileIdentifier.cpp`).
//!
//! Formato de red:
//!
//! ```text
//! <options u8>[<md4 hash 16>][<file size u64>][<aich root hash 20>]
//! ```
//!
//! Byte de opciones (`opcodes.h` / `FileIdentifier.cpp:165`):
//! - bits 0..=3 (`0x0F`): presencia de los campos — bit0 = MD4, bit1 = tamaño u64,
//!   bit2 = hash AICH. Estos bits son **obligatorios** en eMule (`bRequiredOptions = 0x0F`):
//!   cualquier opción desconocida en esta mitad provoca un rechazo.
//! - bits 4..=7 (`0xF0`): opciones opcionales reservadas — ignoradas por eMule
//!   (`bOptionalSubOptions = 0xF0`).
//!
//! En la práctica, eMule exige siempre el hash MD4 salvo en un contexto Kad específico
//! (`CFileIdentifierSA::ReadIdentifier(…, bAllowEmptyMD4Hash)`). El tamaño u64
//! también está siempre presente en el tráfico moderno.
//!
//! Referencia: `srchybrid/FileIdentifier.cpp:125-250`, `opcodes.h:286-289`.

use super::error::{Ed2kErrorKind, Ed2kParseError};
use super::tags::TagReader;

/// Bit de opción 0 — hash MD4 presente (`FileIdentifier.cpp` `0x01`).
pub const FILE_IDENTIFIER_OPT_MD4: u8 = 0x01;
/// Bit de opción 1 — tamaño de fichero `u64` presente (`0x02`).
pub const FILE_IDENTIFIER_OPT_SIZE: u8 = 0x02;
/// Bit de opción 2 — hash raíz AICH (20 bytes) presente (`0x04`).
pub const FILE_IDENTIFIER_OPT_AICH: u8 = 0x04;
/// Máscara de opciones obligatorias: eMule rechaza cualquier bit desconocido aquí
/// (`bRequiredOptions = 0x0F`, `FileIdentifier.cpp:185`).
pub const FILE_IDENTIFIER_REQUIRED_MASK: u8 = 0x0F;
/// Máscara de opciones opcionales: bits reservados, ignorados en la recepción
/// (`bOptionalSubOptions = 0xF0`, `FileIdentifier.cpp:187`).
pub const FILE_IDENTIFIER_OPTIONAL_MASK: u8 = 0xF0;

/// Tamaño del hash MD4 eD2K.
pub const MD4_HASH_LEN: usize = 16;
/// Tamaño del hash raíz AICH (SHA-1).
pub const AICH_HASH_LEN: usize = 20;

/// Identificador compuesto de un fichero (`FileIdentifier`, eMule 0.72a).
///
/// Serializado en cabecera de `OP_HASHSETREQUEST2`/`OP_HASHSETANSWER2` y
/// `OP_MULTIPACKET_EXT2`/`OP_MULTIPACKETANSWER_EXT2`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kFileIdentifier {
    /// Hash MD4 del fichero. `None` únicamente en contextos Kad
    /// (`bAllowEmptyMD4Hash`); en TCP eMule exige su presencia.
    pub md4_hash: Option<[u8; MD4_HASH_LEN]>,
    /// Tamaño del fichero en bytes.
    pub file_size: Option<u64>,
    /// Hash raíz AICH (SHA-1 del árbol de hashes).
    pub aich_hash: Option<[u8; AICH_HASH_LEN]>,
    /// Bits opcionales superiores (`0xF0`) leídos en el cable, conservados tal cual
    /// para un viaje de ida y vuelta determinista.
    pub optional_bits: u8,
}

impl Ed2kFileIdentifier {
    /// Identificador estándar TCP: hash MD4 + tamaño (el mínimo exigido por eMule).
    pub fn new(md4_hash: [u8; MD4_HASH_LEN], file_size: u64) -> Self {
        Self {
            md4_hash: Some(md4_hash),
            file_size: Some(file_size),
            aich_hash: None,
            optional_bits: 0,
        }
    }

    /// Byte de opciones codificado.
    pub fn options_byte(&self) -> u8 {
        let mut opts = self.optional_bits & FILE_IDENTIFIER_OPTIONAL_MASK;
        if self.md4_hash.is_some() {
            opts |= FILE_IDENTIFIER_OPT_MD4;
        }
        if self.file_size.is_some() {
            opts |= FILE_IDENTIFIER_OPT_SIZE;
        }
        if self.aich_hash.is_some() {
            opts |= FILE_IDENTIFIER_OPT_AICH;
        }
        opts
    }

    /// Escribe el identificador en el cable (orden: opciones, MD4, tamaño, AICH).
    pub fn write_to(&self, out: &mut Vec<u8>) {
        out.push(self.options_byte());
        if let Some(h) = &self.md4_hash {
            out.extend_from_slice(h);
        }
        if let Some(s) = self.file_size {
            out.extend_from_slice(&s.to_le_bytes());
        }
        if let Some(h) = &self.aich_hash {
            out.extend_from_slice(h);
        }
    }

    /// Lee un `FileIdentifier` desde `reader`.
    ///
    /// `allow_empty_md4` se corresponde con `bAllowEmptyMD4Hash` de
    /// `CFileIdentifierSA::ReadIdentifier` (`FileIdentifier.cpp:198`): `false`
    /// en TCP (eMule exige el MD4), `true` posible en contexto Kad.
    ///
    /// Al igual que eMule (`FileIdentifier.cpp:185-191`), cualquier opción desconocida en la
    /// máscara obligatoria `0x0F` provoca `InvalidField`; los bits opcionales
    /// `0xF0` se toleran y se conservan en `optional_bits`.
    pub fn read_from(
        reader: &mut TagReader<'_>,
        allow_empty_md4: bool,
    ) -> Result<Self, Ed2kParseError> {
        let opts = reader.read_u8("file_identifier_options")?;

        let unknown_required = opts
            & FILE_IDENTIFIER_REQUIRED_MASK
            & !(FILE_IDENTIFIER_OPT_MD4 | FILE_IDENTIFIER_OPT_SIZE | FILE_IDENTIFIER_OPT_AICH);
        if unknown_required != 0 {
            return Err(Ed2kParseError::new(
                reader.protocol(),
                reader.opcode(),
                reader.pos() - 1,
                "file_identifier_options",
                Ed2kErrorKind::MalformedField {
                    message: format!(
                        "file identifier required options unknown: 0x{unknown_required:02X}"
                    ),
                },
            ));
        }

        let md4_hash = if opts & FILE_IDENTIFIER_OPT_MD4 != 0 {
            Some(reader.read_hash("file_identifier_md4")?)
        } else {
            None
        };
        if md4_hash.is_none() && !allow_empty_md4 {
            return Err(Ed2kParseError::new(
                reader.protocol(),
                reader.opcode(),
                reader.pos() - 1,
                "file_identifier_md4_missing",
                Ed2kErrorKind::MalformedField {
                    message: "file identifier without MD4 hash on TCP context".to_string(),
                },
            ));
        }

        let file_size = if opts & FILE_IDENTIFIER_OPT_SIZE != 0 {
            Some(reader.read_u64_le("file_identifier_size")?)
        } else {
            None
        };

        let aich_hash = if opts & FILE_IDENTIFIER_OPT_AICH != 0 {
            let bytes = reader.read_bytes(AICH_HASH_LEN, "file_identifier_aich")?;
            let mut arr = [0u8; AICH_HASH_LEN];
            arr.copy_from_slice(bytes);
            Some(arr)
        } else {
            None
        };

        Ok(Self {
            md4_hash,
            file_size,
            aich_hash,
            optional_bits: opts & FILE_IDENTIFIER_OPTIONAL_MASK,
        })
    }
}

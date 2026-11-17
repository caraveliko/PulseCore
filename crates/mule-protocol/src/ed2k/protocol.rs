// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Bytes mágicos de protocolo e identificadores para redes eD2K históricas.

/// Byte identificador de protocolo estándar de eDonkey2000 (`0xE3`).
pub const PR_ED2K: u8 = 0xE3;

/// Byte identificador de protocolo extendido de eMule (`0xC5`).
pub const PR_EMULE: u8 = 0xC5;

/// Byte identificador de protocolo comprimido de eMule (`0xD4`).
pub const PR_EMULE_COMPRESSED: u8 = 0xD4;

/// Comprueba si el byte dado es un byte mágico reconocido del protocolo eD2K.
#[inline]
pub fn is_valid_ed2k_protocol(protocol: u8) -> bool {
    matches!(protocol, PR_ED2K | PR_EMULE | PR_EMULE_COMPRESSED)
}

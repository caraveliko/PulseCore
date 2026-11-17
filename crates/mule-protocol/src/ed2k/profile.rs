// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Perfiles de compatibilidad para la desambiguación de red eD2K.
//!
//! Las variaciones históricas entre eDonkey2000, eMule y aMule provocaron solapamientos de
//! códigos de operación con cargas útiles y significados semánticos completamente distintos (p. ej., `0x4B`
//! representa `OP_REQUESTPARTS` en eDonkey heredado pero `OP_ASKSHAREDFILESANSWER` en eMule).
//!
//! Para evitar conjeturas heurísticas y corrupción de datos en red, `Ed2kCompatibilityProfile` exige
//! una desambiguación explícita de la disposición y de los códigos de operación.

use std::fmt;

/// Perfil explícito de compatibilidad de cliente para la codificación y decodificación de red eD2K.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Ed2kCompatibilityProfile {
    /// eMule 0.72a canónico (implementación de referencia actual):
    /// - `OP_REQUESTPARTS_EMULE` (`0x47`): disposición de 3 ranuras (carga útil de 40 bytes).
    /// - `OP_SETREQFILEID_EMULE` (`0x4F`): petición de estado de fichero (hash de fichero de 16 bytes).
    /// - Rechaza `0x4B` como petición de bloque (`0x4B` es `OP_ASKSHAREDFILESANSWER` en eMule).
    /// - Rechaza `0x49` como petición de estado de fichero (`0x49` es `OP_END_OF_DOWNLOAD` en eMule).
    #[default]
    Emule072a,

    /// eMule 0.50a histórico: las asignaciones de códigos de operación en cable y disposiciones son
    /// idénticas a `Emule072a` (los códigos de operación ambiguos `0x47`/`0x49`/`0x4B`/`0x4F`
    /// nunca cambiaron entre 0.50a y 0.72a). Conservado para reproducir pruebas históricas;
    /// el nuevo código debería usar `Emule072a`.
    Emule050a,

    /// aMule 2.3.3:
    /// - Sigue las mismas asignaciones de códigos de operación en cable y disposiciones que eMule (`0x47` y `0x4F`).
    Amule233,

    /// eDonkey2000 heredado:
    /// - `OP_REQUESTPARTS` (`0x4B`): admite disposiciones de una ranura de 24 bytes y de 3 ranuras de 40 bytes.
    /// - `OP_FILESTATUSREQ` (`0x49`): petición de estado de fichero (hash de fichero de 16 bytes).
    /// - Rechaza `0x47` como petición de bloque (`0x47` era `OP_SETREQFILEID` en versiones tempranas de eDonkey).
    /// - Rechaza `0x4F` como petición de estado de fichero.
    LegacyEdonkey,

    /// Perfil estricto desconocido:
    /// - Nunca desambigua códigos de operación sobrecargados (`0x47`, `0x4B`, `0x49`, `0x4F`).
    /// - Devuelve estrictamente `Ed2kErrorKind::AmbiguousOpcodeForProfile`.
    UnknownStrict,
}

impl Ed2kCompatibilityProfile {
    /// Devuelve un identificador legible para este perfil.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Emule072a => "Emule072a",
            Self::Emule050a => "Emule050a",
            Self::Amule233 => "Amule233",
            Self::LegacyEdonkey => "LegacyEdonkey",
            Self::UnknownStrict => "UnknownStrict",
        }
    }

    /// Código de operación esperado para solicitar partes bajo este perfil.
    ///
    /// Devuelve `None` si el perfil es `UnknownStrict`.
    pub const fn request_parts_opcode(&self) -> Option<u8> {
        match self {
            Self::Emule072a | Self::Emule050a | Self::Amule233 => {
                Some(super::opcodes::OP_REQUESTPARTS_EMULE)
            } // 0x47
            Self::LegacyEdonkey => Some(super::opcodes::OP_REQUESTPARTS),                    // 0x4B
            Self::UnknownStrict => None,
        }
    }

    /// Código de operación esperado para solicitar el estado del fichero bajo este perfil.
    ///
    /// Devuelve `None` si el perfil es `UnknownStrict`.
    pub const fn file_status_req_opcode(&self) -> Option<u8> {
        match self {
            Self::Emule072a | Self::Emule050a | Self::Amule233 => {
                Some(super::opcodes::OP_SETREQFILEID_EMULE)
            } // 0x4F
            Self::LegacyEdonkey => Some(super::opcodes::OP_FILESTATUSREQ),                   // 0x49
            Self::UnknownStrict => None,
        }
    }
}

impl fmt::Display for Ed2kCompatibilityProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

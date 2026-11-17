// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Analizador de `clients.met` — eMule `CClientCreditsList::LoadList`
//! (`ClientCredits.cpp:156-233`).
//!
//! Disposición del fichero: `[u8 version][u32 count]` seguido de `count`
//! registros `CreditStruct` empaquetados. Existen dos versiones (`Opcodes.h:39-40`):
//!
//! - `0x12` (`CREDITFILE_VERSION`) — `CreditStruct` de 119 bytes
//!   (`ClientCredits.h:41-52`):
//!   `abyKey[16] | nUploadedLo u32 | nDownloadedLo u32 | tLastSeen u32 |
//!    nUploadedHi u32 | nDownloadedHi u32 | nReserved3 u16 |
//!    nKeySize u8 | abySecureIdent[80]`
//! - `0x11` (`CREDITFILE_VERSION_29`) — `CreditStruct_29a` de 38 bytes
//!   (`ClientCredits.h:28-37`), mismos campos salvo `nKeySize`/clave pública.
//!
//! Cualquier otra versión → eMule rechaza el fichero completo.

use std::io::Read;
use std::path::Path;

use super::error::MetError;
use super::reader::{open_met_file, CountingReader};

/// `CREDITFILE_VERSION` (`Opcodes.h:39`).
pub const CREDITFILE_VERSION: u8 = 0x12;
/// `CREDITFILE_VERSION_29` (`Opcodes.h:40`) — disposición 0.29a.
pub const CREDITFILE_VERSION_29: u8 = 0x11;
/// `MAXPUBKEYSIZE` (`ClientCredits.h:21`).
pub const MAXPUBKEYSIZE: usize = 80;
/// Tamaño de `CreditStruct` empaquetado (versión 0x12).
const CREDIT_STRUCT_LEN: usize = 119;
/// Tamaño de `CreditStruct_29a` empaquetado (versión 0x11).
const CREDIT_STRUCT_29A_LEN: usize = 38;
/// Límite de seguridad en el recuento de registros — las listas de créditos de eMule son a escala de usuario
/// (miles, no millones).
pub const MAX_CLIENTS_RECORDS: usize = 5_000_000;

/// Un registro de crédito de `clients.met`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientsMetRecord {
    /// `abyKey` — el hash de usuario eD2K del cliente (clave de créditos en eMule).
    pub key: [u8; 16],
    /// `nUploadedHi:nUploadedLo` — bytes subidos AL par.
    pub uploaded: u64,
    /// `nDownloadedHi:nDownloadedLo` — bytes descargados DESDE el par.
    pub downloaded: u64,
    /// `tLastSeen` — segundos unix; eMule descarta las entradas de más de 150
    /// días al cargar (`ClientCredits.cpp:207-213`).
    pub last_seen: u32,
    /// `abySecureIdent[..nKeySize]` — clave pública RSA cuando el par
    /// completó la identificación segura; `None` para `nKeySize == 0`.
    pub pubkey: Option<Vec<u8>>,
}

/// Contenido analizado de `clients.met`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientsMetFile {
    /// Versión del archivo (`0x12` o `0x11`).
    pub version: u8,
    /// `true` cuando los registros no portaban clave pública (disposición `0x11`).
    pub legacy_format: bool,
    /// Registros en orden de archivo.
    pub records: Vec<ClientsMetRecord>,
}

impl ClientsMetFile {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, MetError> {
        let mut file = open_met_file(path)?;
        Self::read_from(&mut file)
    }

    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self, MetError> {
        let mut cr = CountingReader::new(reader);
        let version = cr.read_u8("clients.met version")?;
        let (record_len, legacy_format) = match version {
            CREDITFILE_VERSION => (CREDIT_STRUCT_LEN, false),
            CREDITFILE_VERSION_29 => (CREDIT_STRUCT_29A_LEN, true),
            other => {
                return Err(MetError::UnsupportedHeader {
                    offset: 0,
                    value: other,
                })
            }
        };
        let count = cr.read_u32_le("clients.met record count")? as usize;
        if count > MAX_CLIENTS_RECORDS {
            return Err(MetError::InvalidLength {
                offset: cr.offset(),
                declared: count as u64,
                maximum: MAX_CLIENTS_RECORDS as u64,
                context: "clients.met record count",
            });
        }
        let mut records = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let raw = cr.read_exact_bytes(record_len, "clients.met credit record")?;
            records.push(parse_credit_struct(&raw, legacy_format));
        }
        Ok(Self {
            version,
            legacy_format,
            records,
        })
    }
}

fn parse_credit_struct(raw: &[u8], legacy: bool) -> ClientsMetRecord {
    // Desplazamientos de campos empaquetados (`ClientCredits.h:41-52`):
    // abyKey@0..16 | nUploadedLo@16 | nDownloadedLo@20 | tLastSeen@24 |
    // nUploadedHi@28 | nDownloadedHi@32 | nReserved3@36 u16 |
    // nKeySize@38 u8 | abySecureIdent@39..119
    let mut key = [0u8; 16];
    key.copy_from_slice(&raw[0..16]);
    let u32_at = |off: usize| u32::from_le_bytes(raw[off..off + 4].try_into().unwrap_or_default());
    let uploaded = ((u32_at(28) as u64) << 32) | u32_at(16) as u64;
    let downloaded = ((u32_at(32) as u64) << 32) | u32_at(20) as u64;
    let last_seen = u32_at(24);
    let pubkey = if legacy {
        None
    } else {
        let key_size = raw[38] as usize;
        (key_size > 0 && key_size <= MAXPUBKEYSIZE)
            .then(|| raw[39..39 + key_size].to_vec())
    };
    ClientsMetRecord {
        key,
        uploaded,
        downloaded,
        last_seen,
        pubkey,
    }
}

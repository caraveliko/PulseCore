// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Lector de `known.met` de eMule — `CKnownFileList::LoadKnownFiles`
//! (`KnownFileList.cpp:73`) y `CKnownFile::LoadFromFile`
//! (`KnownFile.cpp:769`).
//!
//! Disposición:
//! ```text
//! [u8  header]            MET_HEADER (0x0E) o MET_HEADER_I64TAGS (0x0F)
//! [u32 record_count]
//! por registro:
//!   [u32 date_utc]        m_tUTClastModified
//!   [16B file_hash]       raíz MD4 eD2K
//!   [u16 part_count] [part_count × 16B hashes MD4 de parte]
//!   [u32 tag_count]  [tags]
//! ```
//!
//! Un registro se descarta (igual que `delete pRecord` en eMule) cuando el
//! conjunto de hashes de parte falla la verificación final
//! `HasExpectedMD4HashCount` / `CalculateMD4HashByHashSet`
//! (`KnownFile.cpp:775-777`, `FileIdentifier.cpp:174-203`).

use std::io::Read;
use std::path::Path;

use md4::{Digest, Md4};

use crate::aich::AichHash;
use crate::ed2k::hash::{Ed2kHash, ED2K_PART_SIZE};
use crate::met::error::MetError;
use crate::met::reader::CountingReader;
use crate::met::tags::*;

/// `MET_HEADER` (`OpCodes.h:145`) — disposición de etiquetas de 32 bits.
pub const MET_HEADER: u8 = 0x0E;
/// `MET_HEADER_I64TAGS` (`OpCodes.h:146`) — disposición de etiquetas de 64 bits.
pub const MET_HEADER_I64TAGS: u8 = 0x0F;

/// `MAX_EMULE_FILE_SIZE` (`OpCodes.h:103`) — 256 GiB.
pub const MAX_EMULE_FILE_SIZE: u64 = 0x40_0000_0000;

/// Límite superior de la lista de hashes de partes MD4
/// (`FileIdentifier.cpp:185` — `MAX_EMULE_FILE_SIZE / PARTSIZE + 1`).
pub const MAX_PART_HASH_COUNT: usize = (MAX_EMULE_FILE_SIZE / ED2K_PART_SIZE as u64 + 1) as usize;

/// Recuento acotado de registros (eMule no tiene límite explícito; los archivos están acotados
/// por disco — esto protege contra campos de recuento corruptos).
pub const MAX_KNOWN_RECORDS: usize = 500_000;
/// Recuento acotado de etiquetas por registro (el known.met real porta < 50).
pub const MAX_KNOWN_TAG_COUNT: usize = 1_000;

/// Valores de prioridad de subida `PR_*` de eMule (`Opcodes.h` / `KnownFile.cpp`).
pub const PR_VERYLOW: u8 = 4;
/// `PR_LOW` de eMule.
pub const PR_LOW: u8 = 0;
/// `PR_NORMAL` de eMule.
pub const PR_NORMAL: u8 = 1;
/// `PR_HIGH` de eMule.
pub const PR_HIGH: u8 = 2;
/// `PR_VERYHIGH` de eMule (powershare).
pub const PR_VERYHIGH: u8 = 3;
/// `PR_AUTO` de eMule — resuelto a `PR_HIGH` + indicador auto al cargar
/// (`KnownFile.cpp:662`).
pub const PR_AUTO: u8 = 5;

/// Un único registro de `known.met` (`CKnownFile`).
#[derive(Debug, Clone, PartialEq)]
pub struct KnownFileRecord {
    /// `m_tUTClastModified` — mtime UTC del archivo cuando se calculó el hash por última vez.
    pub last_modified_utc: u32,
    /// Hash eD2K del archivo (raíz MD4).
    pub file_hash: Ed2kHash,
    /// Hashes de partes MD4 almacenados (vacío cuando el registro solo conocía la raíz).
    pub part_hashes: Vec<Ed2kHash>,
    /// `FT_FILENAME`.
    pub file_name: Option<String>,
    /// `FT_FILESIZE` (+ `FT_FILESIZE_HI` compuesto cuando está presente).
    pub file_size: Option<u64>,
    /// `FT_AICH_HASH` decodificado de base32 (raíz SHA-1 de 20 bytes).
    pub aich_root: Option<AichHash>,
    /// `FT_LASTSHARED`.
    pub last_shared_utc: Option<u32>,
    /// `FT_ATTRANSFERRED | FT_ATTRANSFERREDHI<<32`.
    pub transferred_bytes: u64,
    /// `FT_ATREQUESTED`.
    pub requested: u32,
    /// `FT_ATACCEPTED`.
    pub accepted: u32,
    /// `FT_ULPRIORITY` tras la desinfección en tiempo de carga de eMule
    /// (`PR_AUTO` → `PR_HIGH` aquí; el indicador auto se pierde — eMule
    /// lo recalcula dinámicamente de todas formas).
    pub up_priority: Option<u8>,
    /// `FT_FLAGS & 0x0F` (`m_uMetaDataVer`).
    pub meta_data_ver: u32,
    /// `FT_KADLASTPUBLISHSRC` — reiniciado a 0 por eMule cuando está absurdamente en el
    /// futuro (`KnownFile.cpp:671-676`); conservamos el valor sin procesar.
    pub kad_last_publish_src: Option<u32>,
    /// `FT_KADLASTPUBLISHNOTES`.
    pub kad_last_publish_notes: Option<u32>,
    /// Etiquetas que eMule encaminaría a `m_taglist` (`ConvertED2KTag` +
    /// rama default) — etiquetas de medios/metadatos y campos desconocidos.
    pub extra_tags: Vec<Tag>,
}

impl KnownFileRecord {
    /// `GetTheoreticalMD4PartHashCount` de eMule
    /// (`FileIdentifier.cpp:223`): `floor(size/PARTSIZE) + 1` para
    /// `size > 0` (el `+1` cubre el resumen de la parte vacía final para
    /// múltiplos exactos).
    pub fn expected_part_hash_count(&self) -> Option<u16> {
        let size = self.file_size?;
        if size == 0 {
            return Some(0);
        }
        Some((size / ED2K_PART_SIZE as u64) as u16 + 1)
    }
}

/// Contenido analizado de `known.met`.
#[derive(Debug, Clone, PartialEq)]
pub struct KnownMetFile {
    /// `true` cuando el archivo utiliza la disposición `MET_HEADER_I64TAGS` (0x0F)
    /// — los enteros de 64 bits ya fueron decodificados por el lector de etiquetas.
    pub i64_tags: bool,
    /// Registros que superaron la verificación final del conjunto de hashes de eMule.
    pub records: Vec<KnownFileRecord>,
    /// Registros que eMule descartaría (`LoadFromFile` devolvió false —
    /// discordancia en el recuento de hashes de partes o incoherencia en el hash raíz).
    pub dropped_records: usize,
}

impl KnownMetFile {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, MetError> {
        let mut file = crate::met::reader::open_met_file(path)?;
        Self::read_from(&mut file)
    }

    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self, MetError> {
        let mut cr = CountingReader::new(reader);

        let header_offset = cr.offset();
        let header = cr.read_u8("known.met header")?;
        let i64_tags = match header {
            MET_HEADER => false,
            MET_HEADER_I64TAGS => true,
            _ => {
                return Err(MetError::UnsupportedHeader {
                    offset: header_offset,
                    value: header,
                })
            }
        };

        let count_offset = cr.offset();
        let count = cr.read_u32_le("record count")? as usize;
        if count > MAX_KNOWN_RECORDS {
            return Err(MetError::InvalidLength {
                offset: count_offset,
                declared: count as u64,
                maximum: MAX_KNOWN_RECORDS as u64,
                context: "record count",
            });
        }

        let mut records = Vec::with_capacity(count.min(1024));
        let mut dropped_records = 0usize;
        for _ in 0..count {
            match read_record(&mut cr)? {
                Some(rec) => records.push(rec),
                None => dropped_records += 1,
            }
        }

        Ok(Self {
            i64_tags,
            records,
            dropped_records,
        })
    }
}

/// `CKnownFile::LoadFromFile` — devuelve `None` donde eMule elimina el
/// registro (verificación final fallida), manteniendo el flujo alineado.
fn read_record<R: Read>(cr: &mut CountingReader<R>) -> Result<Option<KnownFileRecord>, MetError> {
    let last_modified_utc = cr.read_u32_le("record date")?;
    let file_hash = read_hash16(cr, "file hash")?;

    let parts_offset = cr.offset();
    let part_count = cr.read_u16_le("part hash count")? as usize;
    if part_count > MAX_PART_HASH_COUNT {
        return Err(MetError::InvalidLength {
            offset: parts_offset,
            declared: part_count as u64,
            maximum: MAX_PART_HASH_COUNT as u64,
            context: "part hash count",
        });
    }
    let mut part_hashes = Vec::with_capacity(part_count);
    for _ in 0..part_count {
        part_hashes.push(read_hash16(cr, "part hash")?);
    }

    let tag_count_offset = cr.offset();
    let tag_count = cr.read_u32_le("tag count")? as usize;
    if tag_count > MAX_KNOWN_TAG_COUNT {
        return Err(MetError::InvalidLength {
            offset: tag_count_offset,
            declared: tag_count as u64,
            maximum: MAX_KNOWN_TAG_COUNT as u64,
            context: "tag count",
        });
    }

    let mut rec = KnownFileRecord {
        last_modified_utc,
        file_hash,
        part_hashes,
        file_name: None,
        file_size: None,
        aich_root: None,
        last_shared_utc: None,
        transferred_bytes: 0,
        requested: 0,
        accepted: 0,
        up_priority: None,
        meta_data_ver: 0,
        kad_last_publish_src: None,
        kad_last_publish_notes: None,
        extra_tags: Vec::new(),
    };
    let mut transferred_hi: u64 = 0;

    for _ in 0..tag_count {
        let tag = cr.read_tag()?;
        match tag.name_id {
            FT_FILENAME => {
                if let Some(name) = tag.as_str() {
                    if rec.file_name.is_none() {
                        rec.file_name = Some(name.to_string());
                    }
                }
            }
            FT_FILESIZE => {
                if let Some(v) = tag.as_u64() {
                    rec.file_size = Some(v);
                }
            }
            FT_FILESIZE_HI => {
                // eMule 0.72a escribe FILESIZE como una sola etiqueta u64; la
                // compañera _HI aparece en archivos .met foráneos.
                if let Some(hi) = tag.as_u64() {
                    let lo = rec.file_size.unwrap_or(0) & 0xFFFF_FFFF;
                    rec.file_size = Some((hi << 32) | lo);
                }
            }
            FT_ATTRANSFERRED => {
                if let Some(v) = tag.as_u64() {
                    rec.transferred_bytes =
                        (rec.transferred_bytes & 0xFFFF_FFFF_0000_0000) | (v & 0xFFFF_FFFF);
                }
            }
            FT_ATTRANSFERREDHI => {
                if let Some(v) = tag.as_u64() {
                    transferred_hi = v;
                }
            }
            FT_ATREQUESTED => {
                if let Some(v) = tag.as_u64() {
                    rec.requested = v as u32;
                }
            }
            FT_ATACCEPTED => {
                if let Some(v) = tag.as_u64() {
                    rec.accepted = v as u32;
                }
            }
            FT_ULPRIORITY => {
                if let Some(v) = tag.as_u64() {
                    // KnownFile.cpp:662 — PR_AUTO se resuelve en PR_HIGH +
                    // indicador auto; los valores no válidos recurren a PR_NORMAL.
                    rec.up_priority = Some(match v as u8 {
                        PR_AUTO => PR_HIGH,
                        p @ (PR_VERYLOW | PR_LOW | PR_NORMAL | PR_HIGH | PR_VERYHIGH) => p,
                        _ => PR_NORMAL,
                    });
                }
            }
            FT_KADLASTPUBLISHSRC => {
                if let Some(v) = tag.as_u64() {
                    rec.kad_last_publish_src = Some(v as u32);
                }
            }
            FT_KADLASTPUBLISHNOTES => {
                if let Some(v) = tag.as_u64() {
                    rec.kad_last_publish_notes = Some(v as u32);
                }
            }
            FT_FLAGS => {
                if let Some(v) = tag.as_u64() {
                    rec.meta_data_ver = (v as u32) & 0x0F;
                }
            }
            FT_AICH_HASH => {
                if let Some(s) = tag.as_str() {
                    rec.aich_root = AichHash::from_base32(s).ok();
                }
            }
            FT_LASTSHARED => {
                if let Some(v) = tag.as_u64() {
                    rec.last_shared_utc = Some(v as u32);
                }
            }
            FT_AICHHASHSET => {
                // eMule carga e inmediatamente verifica el conjunto de hashes
                // de partes AICH (`KnownFile.cpp:718-727`). Nuestro modelo
                // de archivos compartidos solo necesita la raíz — conservarlo como etiqueta extra normal
                // duplicaría un blob de varios KB; se omite su almacenamiento.
            }
            _ => {
                // Ruta ConvertED2KTag — etiquetas multimedia, FT_PERMISSIONS,
                // FT_KADLASTPUBLISHKEY y campos desconocidos.
                rec.extra_tags.push(tag);
            }
        }
    }

    rec.transferred_bytes |= transferred_hi << 32;

    // Verificación final (KnownFile.cpp:775-777):
    // `ret && HasExpectedMD4HashCount` — el recuento de partes debe coincidir con el
    // recuento teórico derivado del tamaño del archivo (proporcionado por etiqueta), y
    // la raíz almacenada debe ser igual al hash calculado a partir del conjunto de hashes.
    if let Some(expected) = rec.expected_part_hash_count() {
        if rec.part_hashes.len() != expected as usize {
            return Ok(None);
        }
    }
    if !verify_root_from_hashset(&rec.file_hash, &rec.part_hashes) {
        return Ok(None);
    }
    Ok(Some(rec))
}

/// Ruta de verificación de `CFileIdentifier::CalculateMD4HashByHashSet`:
/// conjunto de hashes vacío → OK; un hash → la raíz debe ser igual a él; ≥2 hashes → la raíz
/// debe ser igual a MD4(concatenación).
fn verify_root_from_hashset(file_hash: &Ed2kHash, part_hashes: &[Ed2kHash]) -> bool {
    match part_hashes.len() {
        0 => true,
        1 => part_hashes[0].as_bytes() == file_hash.as_bytes(),
        _ => {
            let mut h = Md4::new();
            for ph in part_hashes {
                h.update(ph.as_bytes());
            }
            let digest: [u8; 16] = h.finalize().into();
            &digest == file_hash.as_bytes()
        }
    }
}

fn read_hash16<R: Read>(cr: &mut CountingReader<R>, context: &'static str) -> Result<Ed2kHash, MetError> {
    let bytes = cr.read_exact_bytes(16, context)?;
    let mut h = [0u8; 16];
    h.copy_from_slice(&bytes);
    Ok(Ed2kHash(h))
}

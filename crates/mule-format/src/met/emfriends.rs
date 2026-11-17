// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Lector de `emfriends.met` de eMule — `CFriendList::LoadList`
//! (`FriendList.cpp:51`) y `CFriend::LoadFromFile` (`Friend.cpp:95`).
//!
//! Disposición:
//! ```text
//! [u8  header]            MET_HEADER (0x0E)
//! [u32 record_count]
//! por registro:
//!   [16B user_hash]       m_abyUserhash (puede ser cero — amigo solo por IP)
//!   [u32 last_used_ip]    m_dwLastUsedIP
//!   [u16 last_used_port]  m_nLastUsedPort
//!   [u32 last_seen]       m_tLastSeen
//!   [u32 last_chatted]    m_tLastChatted
//!   [u32 tag_count]  [tags]
//! ```
//!
//! Etiquetas reconocidas (`Friend.h:42-43`): `FF_NAME` (0x01, cadena → nombre)
//! y `FF_KADID` (0x02, hash16 → KadID). Las demás etiquetas se omiten como en
//! la rama `default` del `switch` de eMule.

use std::io::Read;
use std::path::Path;

use crate::met::error::MetError;
use crate::met::known_met::MET_HEADER;
use crate::met::reader::CountingReader;
use crate::met::tags::{MetTagValue, FT_FILENAME};

/// `FF_KADID` (`Friend.h:43`) — el Kad ID del amigo (etiqueta hash16).
const FF_KADID: u8 = 0x02;
/// `FF_NAME` (`Friend.h:42`) — mismo identificador de etiqueta que `FT_FILENAME` (0x01);
/// el nombre del amigo escrito como etiqueta de cadena.
const FF_NAME: u8 = FT_FILENAME;

/// Recuento acotado de registros — una lista de amigos es pequeña por construcción;
/// el límite solo protege contra campos de recuento corruptos.
const MAX_FRIENDS_RECORDS: usize = 100_000;
/// Recuento acotado de etiquetas por registro — eMule escribe como máximo dos.
const MAX_FRIEND_TAGS: usize = 1_000;

/// Un registro de amigo — campos de `CFriend` tal como los almacena eMule.
#[derive(Debug, Clone, PartialEq)]
pub struct EmfriendsRecord {
    /// `m_abyUserhash` — hash de usuario eD2K; todo ceros para amigos conocidos solo por
    /// IP+puerto (`CFriend::HasUserhash`).
    pub user_hash: [u8; 16],
    /// `m_dwLastUsedIP` — última IP vista para amigos solo por IP.
    pub last_used_ip: u32,
    /// `m_nLastUsedPort`.
    pub last_used_port: u16,
    /// `m_tLastSeen` (segundos unix).
    pub last_seen: u32,
    /// `m_tLastChatted` (segundos unix).
    pub last_chatted: u32,
    /// `m_strName` (etiqueta `FF_NAME`) — la primera cadena gana, como en la guardia
    /// `m_strName.IsEmpty()` de eMule.
    pub name: Option<String>,
    /// `m_abyKadID` (etiqueta `FF_KADID`).
    pub kad_id: Option<[u8; 16]>,
}

impl EmfriendsRecord {
    /// `CFriend::HasUserhash` — hash de usuario distinto de cero.
    pub fn has_userhash(&self) -> bool {
        self.user_hash.iter().any(|&b| b != 0)
    }
}

/// Contenido analizado de `emfriends.met`.
#[derive(Debug, Clone, PartialEq)]
pub struct EmfriendsMetFile {
    /// Registros decodificados con éxito.
    pub records: Vec<EmfriendsRecord>,
}

impl EmfriendsMetFile {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, MetError> {
        let mut file = crate::met::reader::open_met_file(path)?;
        Self::read_from(&mut file)
    }

    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self, MetError> {
        let mut cr = CountingReader::new(reader);
        let header = cr.read_u8("emfriends header")?;
        if header != MET_HEADER {
            return Err(MetError::UnsupportedHeader {
                offset: 0,
                value: header,
            });
        }
        let record_count = cr.read_u32_le("record count")? as usize;
        if record_count > MAX_FRIENDS_RECORDS {
            return Err(MetError::InvalidLength {
                offset: cr.offset(),
                declared: record_count as u64,
                maximum: MAX_FRIENDS_RECORDS as u64,
                context: "friend record count",
            });
        }
        let mut records = Vec::with_capacity(record_count.min(1024));
        for _ in 0..record_count {
            let mut user_hash = [0u8; 16];
            user_hash.copy_from_slice(&cr.read_exact_bytes(16, "user hash")?);
            let last_used_ip = cr.read_u32_le("last used ip")?;
            let last_used_port = cr.read_u16_le("last used port")?;
            let last_seen = cr.read_u32_le("last seen")?;
            let last_chatted = cr.read_u32_le("last chatted")?;
            let tag_count = cr.read_u32_le("tag count")? as usize;
            if tag_count > MAX_FRIEND_TAGS {
                return Err(MetError::InvalidLength {
                    offset: cr.offset(),
                    declared: tag_count as u64,
                    maximum: MAX_FRIEND_TAGS as u64,
                    context: "friend tag count",
                });
            }
            let mut name = None;
            let mut kad_id = None;
            for _ in 0..tag_count {
                let tag = cr.read_tag()?;
                match tag.name_id {
                    // `m_strName.IsEmpty()` — eMule conserva únicamente el primer
                    // valor de FF_NAME.
                    FF_NAME if name.is_none() => {
                        if let MetTagValue::String(s) = tag.value {
                            name = Some(s);
                        }
                    }
                    FF_KADID => {
                        if let MetTagValue::Hash16(h) = tag.value {
                            kad_id = Some(h.0);
                        }
                    }
                    _ => {}
                }
            }
            records.push(EmfriendsRecord {
                user_hash,
                last_used_ip,
                last_used_port,
                last_seen,
                last_chatted,
                name,
                kad_id,
            });
        }
        Ok(Self { records })
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Importación de `emfriends.met` — `CFriend::LoadFromFile` /
//! `CClientList::LoadFriendsList` (`Friend.cpp`, `ClientList.cpp`).
//!
//! eMule carga cada registro en `m_listFriends`; los amigos conocidos solo por
//! IP+puerto (`!HasUserhash`) también entran en la lista. Nuestra tabla `friends`
//! está indexada por hash de usuario (un amigo concede una ranura de subida por
//! hash), por lo que solo se importan los registros con `HasUserhash` — el mismo
//! subconjunto que eMule puede emparejar realmente en `IsFriend`. Los registros
//! importados conservan su `FF_NAME`.

use std::collections::HashSet;
use std::path::Path;

use mule_core::persistence::sqlite::Database;
use mule_format::met::EmfriendsMetFile;
use serde::Serialize;

use crate::error::DaemonError;

use super::db_err;

/// Resultado de `import_emfriends_met`.
#[derive(Debug, Default, Serialize)]
pub struct EmfriendsImportReport {
    /// Registros leídos de `emfriends.met`.
    pub records: usize,
    /// Registros sin hash de usuario — eMule los lista pero nuestra tabla está
    /// indexada por hash, por lo que de todos modos no se pueden emparejar con un par.
    pub skipped_no_userhash: usize,
    /// Registros ya conocidos (mismo hash de usuario) — nombre actualizado.
    pub updated: usize,
    /// Nuevos amigos añadidos.
    pub imported: usize,
    /// Hashes de usuario (hex) añadidos o actualizados — el llamador los registra
    /// en el gestor de subidas (`set_friend`).
    pub user_hashes: Vec<String>,
}

/// `LoadFriendsList` — importa un archivo `emfriends.met` en `friends`.
/// Las entradas existentes conservan su ranura y actualizan su nombre
/// (`add_friend` es un upsert, coincidiendo con la semántica de sobreescritura de `CFriend`).
pub fn import_emfriends_met(
    db: &mut Database,
    path: &Path,
) -> Result<EmfriendsImportReport, DaemonError> {
    let file = EmfriendsMetFile::open(path)
        .map_err(|e| DaemonError::Business(format!("emfriends.met: {e}")))?;
    let known: HashSet<String> = db
        .list_friends()
        .map_err(db_err("listing friends"))?
        .into_iter()
        .map(|f| f.user_hash)
        .collect();
    let mut report = EmfriendsImportReport {
        records: file.records.len(),
        ..Default::default()
    };
    for record in &file.records {
        if !record.has_userhash() {
            report.skipped_no_userhash += 1;
            continue;
        }
        let user_hash = hex::encode(record.user_hash);
        let name = record.name.clone().unwrap_or_default();
        db.add_friend(&user_hash, &name)
            .map_err(db_err("importing friend"))?;
        if known.contains(&user_hash) {
            report.updated += 1;
        } else {
            report.imported += 1;
        }
        report.user_hashes.push(user_hash);
    }
    Ok(report)
}

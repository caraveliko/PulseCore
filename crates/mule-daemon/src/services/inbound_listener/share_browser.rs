// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Exploración de archivos compartidos — eMule `ListenSocket.cpp:651-825`
//! (`OP_ASKSHAREDFILES`/`OP_ASKSHAREDDIRS`/`OP_ASKSHAREDFILESDIR`).
//!
//! Cuando nuestro hello anuncia `NoViewSharedFiles = 0` (bit 2 de MISCOPTIONS1 —
//! el valor predeterminado `vsfaEverybody` de eMule), un par conectado puede enumerar
//! nuestros archivos compartidos. eMule nunca expone rutas reales: los directorios se
//! reducen a seudombres (`GetPseudoDirName` — el último componente de la ruta,
//! deduplicado) más los seudodirectorios `!Incomplete Files` (archivos `.part` en la
//! cola de descargas) y `!Other` (compartidos que no están bajo ningún directorio).

use std::path::Path;

use mule_core::model::SharedFile;
use mule_protocol::ed2k::server::OfferedFile;
use mule_protocol::ed2k::shared_files::{
    offered_file_for_peer, OfferPeerContext, OP_INCOMPLETE_SHARED_FILES,
    OP_OTHER_SHARED_FILES,
};

use crate::services::downloads::DownloadService;

/// `thePrefs.m_iSeeShares` de eMule — `vsfaEverybody`/`vsfaFriends`/`vsfaNobody`.
/// Persistido bajo el ajuste `view_shared_files`; `Everybody` por defecto
/// (eMule viene de serie con archivos compartidos explorables por todos).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShareViewLevel {
    /// Cada par conectado puede explorar (`vsfaEverybody` de eMule).
    Everybody,
    /// Solo los amigos pueden explorar (`vsfaFriends` de eMule, filtrado por `IsFriend`).
    Friends,
    /// Nadie puede explorar (`vsfaNobody` de eMule).
    Nobody,
}

/// Clave de configuración persistida en la tabla de ajustes.
pub(crate) const SETTING_VIEW_SHARED_FILES: &str = "view_shared_files";

fn view_level(downloads: &DownloadService) -> ShareViewLevel {
    match downloads
        .get_setting(SETTING_VIEW_SHARED_FILES)
        .ok()
        .flatten()
        .as_deref()
    {
        Some("friends") => ShareViewLevel::Friends,
        Some("nobody") => ShareViewLevel::Nobody,
        _ => ShareViewLevel::Everybody,
    }
}

/// `thePrefs.CanSeeShares() == vsfaEverybody || (vsfaFriends && client->IsFriend())` de eMule —
/// un amigo se identifica por su user hash (`CFriend` se indexa por `m_abyUserHash`).
pub(crate) fn peer_can_view_shares(
    downloads: &DownloadService,
    peer_user_hash: Option<[u8; 16]>,
) -> bool {
    match view_level(downloads) {
        ShareViewLevel::Everybody => true,
        ShareViewLevel::Nobody => false,
        ShareViewLevel::Friends => peer_user_hash.is_some_and(|h| {
            let hex = hex::encode(h);
            downloads
                .list_friends()
                .unwrap_or_default()
                .iter()
                .any(|f| f.user_hash.eq_ignore_ascii_case(&hex))
        }),
    }
}

/// Nombre de seudodirectorio para un archivo compartido (`GetPseudoDirName` de eMule —
/// nombre base del directorio contenedor, nunca la ruta completa).
fn pseudo_dir_of(f: &SharedFile) -> Option<String> {
    if f.is_partial {
        return Some(OP_INCOMPLETE_SHARED_FILES.to_string());
    }
    if f.file_path.is_empty() {
        return Some(OP_OTHER_SHARED_FILES.to_string());
    }
    let normalized = f.file_path.replace('\\', "/");
    Path::new(&normalized)
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
}

/// Lista de directorios para `OP_ASKSHAREDDIRSANS` (`SendSharedDirectories` en
/// `BaseClient.cpp:2916`): seudonombres de directorios compartidos (deduplicados),
/// luego `!Incomplete Files` cuando existen descargas de partes, y después `!Other`
/// cuando existen compartidos solo con hash.
pub(crate) fn browse_dirs(shares: &[SharedFile]) -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    for f in shares.iter().filter(|f| f.enabled) {
        if let Some(name) = pseudo_dir_of(f) {
            if name != OP_INCOMPLETE_SHARED_FILES
                && name != OP_OTHER_SHARED_FILES
                && !dirs.contains(&name)
            {
                dirs.push(name);
            }
        }
    }
    if shares.iter().any(|f| f.enabled && f.is_partial) {
        dirs.push(OP_INCOMPLETE_SHARED_FILES.to_string());
    }
    if shares
        .iter()
        .any(|f| f.enabled && !f.is_partial && f.file_path.is_empty())
    {
        dirs.push(OP_OTHER_SHARED_FILES.to_string());
    }
    dirs
}

/// Resuelve un seudodirectorio solicitado a sus archivos (`ListenSocket.cpp:706-770`):
/// `!Incomplete Files` → archivos de partes, `!Other` → compartidos sin ruta,
/// de lo contrario el seudonobre se compara con los nombres base de directorios contenedores.
/// Los seudodirectorios desconocidos se resuelven como una lista vacía, igual que en eMule.
pub(crate) fn files_in_dir<'a>(
    shares: &'a [SharedFile],
    requested: &str,
    peer_large_files: bool,
) -> Vec<&'a SharedFile> {
    shares
        .iter()
        .filter(|f| f.enabled)
        .filter(|f| {
            const LARGE_FILE: u64 = 0xFFFF_FFFF;
            f.file_size <= LARGE_FILE || peer_large_files
        })
        .filter(|f| match pseudo_dir_of(f) {
            Some(name) => name == requested,
            None => false,
        })
        .collect()
}

/// Construye las entradas de archivos ofrecidos para un paquete de respuesta
/// (variante de par de `CreateOfferedFilePacket`): ID+puerto reales solo en HighID,
/// los archivos grandes obtienen la etiqueta de 64 bits `FT_FILESIZE` solo cuando el par
/// los admite.
pub(crate) fn offered_entries<'a>(
    shares: impl Iterator<Item = &'a SharedFile>,
    ctx: &OfferPeerContext,
) -> Vec<OfferedFile> {
    shares
        .filter_map(|f| {
            let hash = hex::decode(&f.file_hash).ok()?;
            let file_hash: [u8; 16] = hash.as_slice().try_into().ok()?;
            offered_file_for_peer(
                file_hash,
                &f.file_name,
                f.file_size,
                0, // no user rating in our model (eMule FT_FILERATING stays off)
                ctx,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn share(hash: u8, path: &str, partial: bool) -> SharedFile {
        let mut s = SharedFile::new(hex::encode([hash; 16]), "file.bin", 42, path);
        s.is_partial = partial;
        s
    }

    #[test]
    fn browse_dirs_collects_pseudo_names() {
        let shares = vec![
            share(1, r"C:\Incoming\a.bin", false),
            share(2, r"C:\Incoming\b.bin", false), // same dir -> dedup
            share(3, "", false),                   // hash-only -> !Other
            share(4, r"D:\tmp\c.part", true),      // partial -> !Incomplete
            share(9, "", false).tap_disabled(),    // disabled -> ignored
        ];
        let dirs = browse_dirs(&shares);
        assert_eq!(dirs, vec!["Incoming", "!Incomplete Files", "!Other"]);
    }

    #[test]
    fn files_in_dir_resolves_each_pseudo() {
        let shares = vec![
            share(1, r"C:\Incoming\a.bin", false),
            share(3, "", false),
            share(4, r"D:\tmp\c.part", true),
        ];
        assert_eq!(files_in_dir(&shares, "Incoming", true).len(), 1);
        assert_eq!(files_in_dir(&shares, OP_OTHER_SHARED_FILES, true).len(), 1);
        assert_eq!(
            files_in_dir(&shares, OP_INCOMPLETE_SHARED_FILES, true).len(),
            1
        );
        // Seudodirectorio desconocido -> vacío (eMule envía también una respuesta vacía).
        assert!(files_in_dir(&shares, "no-such-dir", true).is_empty());
        // Filtro de archivos grandes: un archivo >4 GiB se oculta a pares sin soporte.
        let big = {
            let mut s = share(5, r"C:\Incoming\big.bin", false);
            s.file_size = 5_000_000_000;
            s
        };
        let shares = vec![big];
        assert!(files_in_dir(&shares, "Incoming", false).is_empty());
        assert_eq!(files_in_dir(&shares, "Incoming", true).len(), 1);
    }

    trait Tap {
        fn tap_disabled(self) -> Self;
    }
    impl Tap for SharedFile {
        fn tap_disabled(mut self) -> Self {
            self.enabled = false;
            self
        }
    }
}

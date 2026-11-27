// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

//! Registro persistente de títulos eliminados manualmente de la Filmoteca.
//!
//! Evita que `DiskScanner` o el organizador vuelvan a importar y monitorizar
//! automáticamente una serie o película borrada por el usuario, salvo que se
//! busque y añada manualmente de nuevo (`media.add`).

use serde::{Deserialize, Serialize};

use crate::services::downloads::DownloadService;
use crate::services::scorer::strip_trailing_year_suffix;

/// Clave de configuración SQLite donde se persiste la lista de exclusiones.
const DELETED_MEDIA_EXCLUSIONS_KEY: &str = "deleted_media_exclusions_v1";

/// Entrada de exclusión para un título eliminado de la Filmoteca.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeletedMediaExclusion {
    pub media_type: String,
    pub title_norm: String,
    pub tvdb_id: Option<u64>,
    pub tmdb_id: Option<u64>,
    pub deleted_at_ms: i64,
}

/// Normaliza un título eliminando un posible sufijo de año y pasando a minúsculas.
pub(crate) fn normalize_media_title(title: &str) -> String {
    strip_trailing_year_suffix(title).trim().to_lowercase()
}

/// Carga la lista de títulos eliminados manualmente desde la base de datos.
pub fn load_deleted_media_exclusions(dl_svc: &DownloadService) -> Vec<DeletedMediaExclusion> {
    dl_svc
        .get_setting(DELETED_MEDIA_EXCLUSIONS_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Vec<DeletedMediaExclusion>>(&raw).ok())
        .unwrap_or_default()
}

fn matches_exclusion(
    ex: &DeletedMediaExclusion,
    media_type: &str,
    title_norm: &str,
    tvdb_id: Option<u64>,
    tmdb_id: Option<u64>,
) -> bool {
    if !ex.media_type.eq_ignore_ascii_case(media_type) {
        return false;
    }
    if let (Some(a), Some(b)) = (ex.tvdb_id, tvdb_id) {
        if a == b {
            return true;
        }
    }
    if let (Some(a), Some(b)) = (ex.tmdb_id, tmdb_id) {
        if a == b {
            return true;
        }
    }
    !title_norm.is_empty() && !ex.title_norm.is_empty() && ex.title_norm == title_norm
}

/// Comprueba si un título está excluido de re-importación automática.
pub fn is_media_excluded(
    exclusions: &[DeletedMediaExclusion],
    media_type: &str,
    title: &str,
    tvdb_id: Option<u64>,
    tmdb_id: Option<u64>,
) -> bool {
    let title_norm = normalize_media_title(title);
    exclusions
        .iter()
        .any(|ex| matches_exclusion(ex, media_type, &title_norm, tvdb_id, tmdb_id))
}

/// Registra un título borrado manualmente para impedir que vuelva a importarse automáticamente.
pub fn record_deleted_media(
    dl_svc: &mut DownloadService,
    media_type: &str,
    title: &str,
    tvdb_id: Option<u64>,
    tmdb_id: Option<u64>,
    now_ms: i64,
) {
    let title_norm = normalize_media_title(title);
    if title_norm.is_empty() && tvdb_id.is_none() && tmdb_id.is_none() {
        return;
    }

    let mut exclusions = load_deleted_media_exclusions(dl_svc);
    if let Some(existing) = exclusions
        .iter_mut()
        .find(|ex| matches_exclusion(ex, media_type, &title_norm, tvdb_id, tmdb_id))
    {
        if existing.tvdb_id.is_none() {
            existing.tvdb_id = tvdb_id;
        }
        if existing.tmdb_id.is_none() {
            existing.tmdb_id = tmdb_id;
        }
        existing.deleted_at_ms = now_ms;
    } else {
        exclusions.push(DeletedMediaExclusion {
            media_type: media_type.to_string(),
            title_norm,
            tvdb_id,
            tmdb_id,
            deleted_at_ms: now_ms,
        });
    }

    if let Ok(json) = serde_json::to_string(&exclusions) {
        let _ = dl_svc.set_setting(DELETED_MEDIA_EXCLUSIONS_KEY, &json);
    }
}

/// Elimina la exclusión cuando el usuario añade manualmente de nuevo un título.
pub fn remove_deleted_media_exclusion(
    dl_svc: &mut DownloadService,
    media_type: &str,
    title: &str,
    tvdb_id: Option<u64>,
    tmdb_id: Option<u64>,
) {
    let title_norm = normalize_media_title(title);
    let mut exclusions = load_deleted_media_exclusions(dl_svc);
    let before = exclusions.len();
    exclusions.retain(|ex| !matches_exclusion(ex, media_type, &title_norm, tvdb_id, tmdb_id));
    if exclusions.len() != before {
        if let Ok(json) = serde_json::to_string(&exclusions) {
            let _ = dl_svc.set_setting(DELETED_MEDIA_EXCLUSIONS_KEY, &json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excluye_por_tvdb_tmdb_y_titulo_normalizado() {
        let exclusions = vec![DeletedMediaExclusion {
            media_type: "tv".to_string(),
            title_norm: "naruto".to_string(),
            tvdb_id: Some(78857),
            tmdb_id: Some(46260),
            deleted_at_ms: 1000,
        }];

        assert!(is_media_excluded(&exclusions, "tv", "Naruto (2002)", None, None));
        assert!(is_media_excluded(&exclusions, "tv", "Otro Nombre", Some(78857), None));
        assert!(is_media_excluded(&exclusions, "tv", "Otro Nombre", None, Some(46260)));
        assert!(!is_media_excluded(&exclusions, "movie", "Naruto", Some(78857), Some(46260)));
        assert!(!is_media_excluded(&exclusions, "tv", "Naruto Shippuden", None, None));
    }
}

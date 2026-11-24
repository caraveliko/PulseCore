// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::path::{Path, PathBuf};
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use crate::services::downloads::types::now_ms;
use crate::state::AppState;
use mule_core::model::{EpisodeItem, MediaItem};

/// Tamaño mínimo de un archivo de vídeo independiente en una carpeta entrante adicional (50 MiB).
const MIN_STANDALONE_VIDEO_BYTES: u64 = 50 * 1024 * 1024;
/// Tamaño máximo considerado como extracto «sample / trailer» (85 MiB).
const MAX_SAMPLE_VIDEO_BYTES: u64 = 85 * 1024 * 1024;

/// Corrige el mojibake Latin-1 / UTF-8 en nombres de archivo (ej. "La cabaÃ±a" -> "La cabaña")
pub fn fix_mojibake(s: &str) -> String {
    mule_format::fix_mojibake(s)
}

/// Extrae el año de una cadena, ej. "La cabaña (2017)" o "Oddity.2024.1080p"
pub fn extract_year(name: &str) -> Option<u32> {
    if let Some(open) = name.find('(') {
        let rest = &name[open + 1..];
        if let Some(close) = rest.find(')') {
            let inside = rest[..close].trim();
            if inside.len() == 4 {
                if let Ok(y) = inside.parse::<u32>() {
                    if (1900..=2050).contains(&y) {
                        return Some(y);
                    }
                }
            }
        }
    }
    let chars: Vec<char> = name.chars().collect();
    let n = chars.len();
    for i in 0..n {
        if i + 4 <= n && chars[i..i + 4].iter().all(|c| c.is_ascii_digit()) {
            let prev_boundary = i == 0 || !chars[i - 1].is_alphanumeric();
            let next_boundary = (i + 4 == n) || !chars[i + 4].is_alphanumeric();
            if prev_boundary && next_boundary {
                let s: String = chars[i..i + 4].iter().collect();
                if let Ok(y) = s.parse::<u32>() {
                    if (1900..=2050).contains(&y) {
                        return Some(y);
                    }
                }
            }
        }
    }
    None
}

/// Limpia el título de la película a partir del nombre de archivo, eliminando el año y etiquetas comunes
pub fn clean_movie_title(name: &str) -> String {
    let mut title = name.split('(').next().unwrap_or(name);
    if let Some(pos) = title.find('[') {
        title = &title[..pos];
    }
    let lower = title.to_lowercase();
    let mut end = title.len();
    for tag in &[
        "1080p", "720p", "2160p", "4k", "bluray", "bdrip", "webrip", "hdrip", "x264", "x265",
        "h264", "h265", "dvdrip", "ac3", "dts", "spanish", "castellano", "english", "ingles",
        "vose", "dual", "latino", "subs", "subtitulado", "aac", "eac3", "hevc",
    ] {
        if let Some(pos) = lower.find(tag) {
            if pos < end && pos > 0 {
                end = pos;
            }
        }
    }
    let sub = &title[..end];
    let mut cleaned = sub
        .replace(['.', '_', '-'], " ")
        .trim()
        .to_string();

    // Si el título termina con un año de 4 dígitos ej. "Oddity 2024", eliminar el año
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    if let Some(last) = words.last() {
        if last.len() == 4 {
            if let Ok(y) = last.parse::<u32>() {
                if (1900..=2050).contains(&y) {
                    cleaned = words[..words.len() - 1].join(" ");
                }
            }
        }
    }

    cleaned.trim().to_string()
}

/// Sanea caracteres de nombre de archivo prohibidos en sistemas de archivos Linux / Unix o reproductores estándar
pub fn sanitize_name(name: &str) -> String {
    name.replace(':', " -")
        .replace(['/', '\\'], "-")
        .replace(['*', '?', '"', '<', '>'], "")
        .replace('|', "-")
        .trim()
        .to_string()
}

/// Perfil de compatibilidad de nombres de carpetas y metadatos para servidores multimedia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaServerProfile {
    Jellyfin,
    Plex,
    Kodi,
}

impl MediaServerProfile {
    pub fn from_str_lossy(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "plex" => Self::Plex,
            "kodi" | "infuse" | "universal" => Self::Kodi,
            _ => Self::Jellyfin,
        }
    }
}

/// Construye el nombre de carpeta para una película según el perfil del servidor multimedia.
pub fn format_movie_folder_name(
    title: &str,
    year: Option<u32>,
    tmdb_id: Option<u64>,
    profile: MediaServerProfile,
) -> String {
    let title_clean = sanitize_name(title);
    let year_part = year.map(|y| format!(" ({y})")).unwrap_or_default();
    match (profile, tmdb_id) {
        (MediaServerProfile::Jellyfin, Some(tid)) => format!("{title_clean}{year_part} [tmdbid-{tid}]"),
        (MediaServerProfile::Plex, Some(tid)) => format!("{title_clean}{year_part} {{tmdb-{tid}}}"),
        _ => format!("{title_clean}{year_part}"),
    }
}

/// Construye el nombre de carpeta para una serie según el perfil del servidor multimedia.
pub fn format_series_folder_name(
    title: &str,
    year: Option<u32>,
    tvdb_id: Option<u64>,
    profile: MediaServerProfile,
) -> String {
    let sanitized_title = sanitize_name(title);
    let year_str = year.map(|y| format!(" ({y})")).unwrap_or_default();
    match (profile, tvdb_id) {
        (MediaServerProfile::Jellyfin, Some(tid)) => format!("{sanitized_title}{year_str} [tvdbid-{tid}]"),
        (MediaServerProfile::Plex, Some(tid)) => format!("{sanitized_title}{year_str} {{tvdb-{tid}}}"),
        _ => format!("{sanitized_title}{year_str}"),
    }
}

/// Resuelve la carpeta de serie existente en disco si ya existe una; de lo contrario construye el nombre canónico.
pub fn resolve_series_folder_name(
    root_folder: &Path,
    series_title: &str,
    year: Option<u32>,
    tvdb_id: Option<u64>,
    profile: MediaServerProfile,
) -> String {
    let clean_target = clean_movie_title(series_title).to_lowercase();
    let norm_target = crate::services::scorer::normalize_accents_and_symbols(&clean_target);

    if let Ok(entries) = std::fs::read_dir(root_folder) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(tid) = tvdb_id {
                if fname.contains(&format!("tvdbid-{tid}")) || fname.contains(&format!("tvdb-{tid}")) {
                    return fname;
                }
            }
            let clean_fname = clean_movie_title(&fname).to_lowercase();
            let norm_fname = crate::services::scorer::normalize_accents_and_symbols(&clean_fname);
            if clean_fname == clean_target || norm_fname == norm_target {
                if let Some(y) = year {
                    if fname.contains(&y.to_string()) {
                        return fname;
                    }
                } else {
                    return fname;
                }
            }
        }
    }

    format_series_folder_name(series_title, year, tvdb_id, profile)
}

/// Detecta la etiqueta de calidad a partir del nombre de archivo
pub fn detect_quality_tag(filename: &str) -> &'static str {
    let lower = filename.to_lowercase();
    if lower.contains("2160p") || lower.contains("4k") || lower.contains("uhd") {
        "2160p"
    } else if lower.contains("1080p") || lower.contains("fhd") || lower.contains("bluray") || lower.contains("bdrip") {
        "1080p"
    } else if lower.contains("720p") || lower.contains("hdrip") || lower.contains("hdtv") {
        "720p"
    } else {
        "SD"
    }
}

/// Comprueba si el error devuelto por el sistema operativo indica un enlace entre dispositivos o sistemas de archivos diferentes.
/// - En Unix / Linux: `EXDEV` (código de error 18).
/// - En Windows: `ERROR_NOT_SAME_DEVICE` (código 17) o `ERROR_INVALID_FUNCTION` (código 1) en ciertos sistemas de archivos o unidades de red.
fn is_cross_device_error(err: &std::io::Error) -> bool {
    match err.raw_os_error() {
        Some(18) => true,
        #[cfg(target_os = "windows")]
        Some(17) | Some(1) => true,
        _ => false,
    }
}

/// Realiza un enlace duro desde el origen al destino (o copia en caso de límite entre dispositivos) y elimina el origen de incoming
/// para que no queden archivos duplicados ni residuos en incoming.
pub async fn hardlink_or_copy(source: &Path, target: &Path) -> std::io::Result<()> {
    let src = source.to_path_buf();
    let tgt = target.to_path_buf();
    tokio::task::spawn_blocking(move || {
        if src == tgt {
            return Ok(());
        }
        crate::services::nfs_mount::verify_destination_mount_ready(&tgt, false)?;
        if tgt.exists() {
            if let (Ok(src_meta), Ok(tgt_meta)) = (std::fs::metadata(&src), std::fs::metadata(&tgt)) {
                if src_meta.len() == tgt_meta.len() {
                    // El destino ya contiene el archivo organizado; eliminar origen de incoming
                    if src.exists() {
                        let _ = std::fs::remove_file(&src);
                        info!(
                            "[ORGANIZER] Removed duplicate source '{}' (target '{}' already exists)",
                            src.display(),
                            tgt.display()
                        );
                    }
                    return Ok(());
                }
                if src_meta.len() > 0 {
                    let _ = std::fs::remove_file(&tgt);
                }
            }
        }

        if let Some(parent) = tgt.parent() {
            std::fs::create_dir_all(parent)?;
        }

        match std::fs::hard_link(&src, &tgt) {
            Ok(_) => {
                if src.exists() {
                    let _ = std::fs::remove_file(&src);
                }
                info!(
                    "[ORGANIZER] Moved (hardlink + unlink source) successfully: '{}' -> '{}'",
                    src.display(),
                    tgt.display()
                );
                Ok(())
            }
            Err(e) if is_cross_device_error(&e) => {
                // Enlace entre dispositivos cruzados, incoming y la mediateca están en montajes/sistemas de archivos separados
                info!(
                    "[ORGANIZER] Cross-device boundary detected; falling back to optimized NAS/local transfer and removing source to leave zero residues on host: '{}' -> '{}'",
                    src.display(),
                    tgt.display()
                );
                crate::services::nas_transfer::transfer_to_destination(
                    &src,
                    &tgt,
                    crate::services::nas_transfer::DEFAULT_NAS_TRANSFER_RATE_MBPS,
                    None::<fn(&crate::services::nas_transfer::NasTransferProgress)>,
                )?;
                if src.exists() {
                    let _ = std::fs::remove_file(&src);
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    })
    .await
    .unwrap_or_else(|e| Err(std::io::Error::other(e)))
}

/// Elimina archivos de subtítulos complementarios (.srt, .sub, .vtt) asociados con un archivo de vídeo reemplazado
pub fn remove_companion_subtitles_sync(video_path: &Path) {
    let Some(stem) = video_path.file_stem().and_then(|s| s.to_str()) else {
        return;
    };
    let Some(parent) = video_path.parent() else {
        return;
    };
    for sub_ext in &["srt", "sub", "vtt"] {
        for pattern in &[
            format!("{stem}.{sub_ext}"),
            format!("{stem}.es.{sub_ext}"),
            format!("{stem}.spa.{sub_ext}"),
            format!("{stem}.en.{sub_ext}"),
        ] {
            let cand = parent.join(pattern);
            if cand.is_file() {
                let _ = std::fs::remove_file(&cand);
            }
        }
    }
}

/// Elimina cualquier archivo de vídeo reemplazado en `season_dir` que coincida con el mismo `(season_num, ep_num)`
/// que `dest_file` (o coincida con `old_file_path`), junto con sus subtítulos complementarios.
fn purge_superseded_episode_files(
    season_dir: &Path,
    season_num: u32,
    ep_num: u32,
    dest_file: &Path,
    old_file_path: Option<&str>,
) {
    if let Some(old_fp) = old_file_path {
        let old_p = Path::new(old_fp);
        if !old_fp.is_empty() && old_p != dest_file && old_p.is_file() {
            info!(
                "[ORGANIZER] Removing superseded episode file '{}' (replaced by '{}')",
                old_p.display(),
                dest_file.display()
            );
            remove_companion_subtitles_sync(old_p);
            let _ = std::fs::remove_file(old_p);
        }
    }

    if let Ok(entries) = std::fs::read_dir(season_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_file() || p == dest_file {
                continue;
            }
            let Some(fname) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !mule_network_policy::content_policy::is_allowed_video_filename(fname) {
                continue;
            }
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some((s, e)) = crate::services::disk_scanner::extract_season_episode(Some(season_num), &fname) {
                if s == season_num && e == ep_num {
                    info!(
                        "[ORGANIZER] Removing duplicate episode file '{}' in '{}' for S{:02}E{:02}",
                        p.display(),
                        season_dir.display(),
                        season_num,
                        ep_num
                    );
                    remove_companion_subtitles_sync(&p);
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }
}

/// Elimina cualquier archivo de vídeo reemplazado en `movie_dir` distinto de `dest_file` (y `old_file_path`),
/// junto con sus subtítulos complementarios.
fn purge_superseded_movie_files(
    movie_dir: &Path,
    dest_file: &Path,
    old_file_path: Option<&str>,
) {
    if let Some(old_fp) = old_file_path {
        let old_p = Path::new(old_fp);
        if !old_fp.is_empty() && old_p != dest_file && old_p.is_file() {
            info!(
                "[ORGANIZER] Removing superseded movie file '{}' (replaced by '{}')",
                old_p.display(),
                dest_file.display()
            );
            remove_companion_subtitles_sync(old_p);
            let _ = std::fs::remove_file(old_p);
        }
    }

    if let Ok(entries) = std::fs::read_dir(movie_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_file() || p == dest_file {
                continue;
            }
            let Some(fname) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !mule_network_policy::content_policy::is_allowed_video_filename(fname) {
                continue;
            }
            info!(
                "[ORGANIZER] Removing duplicate movie file '{}' in '{}'",
                p.display(),
                movie_dir.display()
            );
            remove_companion_subtitles_sync(&p);
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// Devuelve true si `path` ya está organizado dentro de un subdirectorio de `movies_dir` o `series_dir`.
pub async fn is_already_in_organized_library(state: &AppState, path: &Path) -> bool {
    let movies_dir = state.effective_movies_dir().await;
    let series_dir = state.effective_series_dir().await;
    for lib_root in [&movies_dir, &series_dir, &state.config.movies_dir, &state.config.series_dir] {
        if path.starts_with(lib_root) && path.parent() != Some(lib_root.as_path()) {
            return true;
        }
    }
    false
}

/// Copia o crea enlaces duros para archivos de subtítulos complementarios (.srt, .sub, .vtt) si están presentes junto al vídeo
async fn copy_companion_subtitles(source_video: &Path, dest_video: &Path) {
    let src_stem = match source_video.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s,
        None => return,
    };
    let parent = match source_video.parent() {
        Some(p) => p,
        None => return,
    };

    for sub_ext in &["srt", "sub", "vtt"] {
        for pattern in &[
            format!("{src_stem}.{sub_ext}"),
            format!("{src_stem}.es.{sub_ext}"),
            format!("{src_stem}.spa.{sub_ext}"),
        ] {
            let cand_sub = parent.join(pattern);
            if cand_sub.is_file() {
                let dest_sub = dest_video.with_extension(format!("es.{sub_ext}"));
                let _ = hardlink_or_copy(&cand_sub, &dest_sub).await;
                break;
            }
        }
    }
}

/// Verifica de forma asíncrona que el destino no sea un punto de montaje NFS/red caído
/// ni viole la protección estricta de red antes de escribir o copiar subtítulos.
async fn ensure_destination_mount_ready(
    dest_path: &Path,
    require_network_mount: bool,
) -> Result<(), String> {
    let dest = dest_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        crate::services::nfs_mount::verify_destination_mount_ready(&dest, require_network_mount)
    })
    .await
    .map_err(|e| format!("Error de tarea al verificar montaje NFS: {e}"))?
    .map_err(|e| {
        warn!(
            "[ORGANIZER] Guardián NFS bloqueó la transferencia a '{}': {}",
            dest_path.display(),
            e
        );
        e.to_string()
    })
}

/// Resuelve la carpeta raíz activa de destino sustituyendo rutas obsoletas o locales por defecto
/// por la ruta efectiva actual (p. ej. tras configurar `nas_root_dir`).
fn resolve_active_root_folder(
    state: &AppState,
    stored_root: &str,
    effective_root: &Path,
    default_dl_dir: &Path,
    default_subdir: &str,
) -> PathBuf {
    let trimmed = stored_root.trim();
    if trimmed.is_empty() {
        return effective_root.to_path_buf();
    }
    let stored_path = Path::new(trimmed);
    if stored_path == effective_root {
        return effective_root.to_path_buf();
    }
    if state.is_default_local_media_subdir(stored_path, default_dl_dir, default_subdir)
        || stored_path.starts_with(default_dl_dir.join("media"))
        || stored_path.starts_with(state.config.default_download_dir.join("media"))
    {
        return effective_root.to_path_buf();
    }
    let status = crate::services::nfs_mount::inspect_directory_status(stored_path, false);
    if !status.unmounted_fstab_trap && !status.is_network_mount && !stored_path.exists() {
        return effective_root.to_path_buf();
    }
    stored_path.to_path_buf()
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Genera un archivo `movie.nfo` XML compatible con Kodi, Jellyfin, Emby e Infuse.
pub fn build_movie_nfo_xml(item: &MediaItem) -> String {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<movie>\n");
    xml.push_str(&format!("  <title>{}</title>\n", xml_escape(&item.title)));
    if let Some(ref orig) = item.original_title {
        xml.push_str(&format!("  <originaltitle>{}</originaltitle>\n", xml_escape(orig)));
    }
    if let Some(y) = item.year {
        xml.push_str(&format!("  <year>{y}</year>\n"));
    }
    if let Some(ref ov) = item.overview {
        xml.push_str(&format!("  <plot>{}</plot>\n", xml_escape(ov)));
    }
    if let Some(tid) = item.tmdb_id {
        xml.push_str(&format!("  <uniqueid type=\"tmdb\" default=\"true\">{tid}</uniqueid>\n"));
    }
    if let Some(ref imdb) = item.imdb_id {
        xml.push_str(&format!("  <uniqueid type=\"imdb\">{}</uniqueid>\n", xml_escape(imdb)));
    }
    xml.push_str("</movie>\n");
    xml
}

/// Genera un archivo `tvshow.nfo` XML compatible con Kodi, Jellyfin, Emby e Infuse.
pub fn build_tvshow_nfo_xml(item: &MediaItem) -> String {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<tvshow>\n");
    xml.push_str(&format!("  <title>{}</title>\n", xml_escape(&item.title)));
    if let Some(ref orig) = item.original_title {
        xml.push_str(&format!("  <originaltitle>{}</originaltitle>\n", xml_escape(orig)));
    }
    if let Some(y) = item.year {
        xml.push_str(&format!("  <year>{y}</year>\n"));
    }
    if let Some(ref ov) = item.overview {
        xml.push_str(&format!("  <plot>{}</plot>\n", xml_escape(ov)));
    }
    if let Some(tvdb) = item.tvdb_id {
        xml.push_str(&format!("  <uniqueid type=\"tvdb\" default=\"true\">{tvdb}</uniqueid>\n"));
    }
    if let Some(tid) = item.tmdb_id {
        xml.push_str(&format!("  <uniqueid type=\"tmdb\">{tid}</uniqueid>\n"));
    }
    if let Some(ref imdb) = item.imdb_id {
        xml.push_str(&format!("  <uniqueid type=\"imdb\">{}</uniqueid>\n", xml_escape(imdb)));
    }
    xml.push_str("</tvshow>\n");
    xml
}

fn resolve_artwork_url(raw: &str, size_segment: &str) -> String {
    if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else if raw.starts_with('/') {
        format!("https://image.tmdb.org/t/p/{size_segment}{raw}")
    } else {
        format!("https://image.tmdb.org/t/p/{size_segment}/{raw}")
    }
}

/// Lee un ajuste booleano en base de datos aplicando un valor por defecto.
async fn get_bool_setting(state: &AppState, key: &str, default: bool) -> bool {
    let dl_svc = state.downloads.lock().await;
    dl_svc
        .get_setting(key)
        .ok()
        .flatten()
        .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
        .unwrap_or(default)
}

/// Comprueba si la organización automática en modo Media Center está activa (por defecto: `true`).
pub async fn is_organize_enabled(state: &AppState) -> bool {
    get_bool_setting(state, "organize_enabled", true).await
}

async fn active_media_server_profile(state: &AppState) -> MediaServerProfile {
    let dl_svc = state.downloads.lock().await;
    let raw = dl_svc
        .get_setting("media_server_profile")
        .ok()
        .flatten()
        .unwrap_or_else(|| "jellyfin".to_string());
    MediaServerProfile::from_str_lossy(&raw)
}

async fn is_nfo_artwork_enabled(state: &AppState) -> bool {
    get_bool_setting(state, "generate_nfo_and_artwork", true).await
}

async fn write_nfo_and_artwork_if_enabled(
    state: &AppState,
    target_dir: &Path,
    item: &MediaItem,
    is_movie: bool,
) {
    if !is_nfo_artwork_enabled(state).await {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(target_dir) {
        warn!("[ORGANIZER] Failed creating dir for NFO/artwork '{}': {e}", target_dir.display());
        return;
    }
    let nfo_name = if is_movie { "movie.nfo" } else { "tvshow.nfo" };
    let nfo_path = target_dir.join(nfo_name);
    if !nfo_path.exists() {
        let xml = if is_movie {
            build_movie_nfo_xml(item)
        } else {
            build_tvshow_nfo_xml(item)
        };
        let _ = std::fs::write(&nfo_path, xml);
    }

    if state.network_policy.state() == mule_network_policy::NetworkState::Disabled {
        return;
    }
    let poster_url = item.poster_path.as_deref().filter(|s| !s.is_empty()).map(|s| resolve_artwork_url(s, "w500"));
    let fanart_url = item.backdrop_path.as_deref().filter(|s| !s.is_empty()).map(|s| resolve_artwork_url(s, "w1280"));
    let poster_dest = target_dir.join("poster.jpg");
    let fanart_dest = target_dir.join("fanart.jpg");

    if poster_url.is_none() && fanart_url.is_none() {
        return;
    }
    tokio::spawn(async move {
        let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
        else {
            return;
        };
        if let Some(url) = poster_url {
            if !poster_dest.exists() {
                if let Ok(resp) = client.get(&url).send().await {
                    if resp.status().is_success() {
                        if let Ok(bytes) = resp.bytes().await {
                            let _ = std::fs::write(&poster_dest, &bytes);
                        }
                    }
                }
            }
        }
        if let Some(url) = fanart_url {
            if !fanart_dest.exists() {
                if let Ok(resp) = client.get(&url).send().await {
                    if resp.status().is_success() {
                        if let Ok(bytes) = resp.bytes().await {
                            let _ = std::fs::write(&fanart_dest, &bytes);
                        }
                    }
                }
            }
        }
    });
}

pub struct MediaOrganizer;

impl MediaOrganizer {
    /// Notifica a los servidores de medios locales (Jellyfin/Emby, Plex, Kodi) para refrescar sus bibliotecas
    pub fn notify_media_servers(state: &AppState) {
        if state.network_policy.state() == mule_network_policy::NetworkState::Disabled {
            return;
        }
        let downloads = state.downloads.clone();
        let cfg_jf_url = state.config.jellyfin_url.clone();
        let cfg_jf_key = state.config.jellyfin_api_key.clone();
        let cfg_plex_url = state.config.plex_url.clone();

        tokio::spawn(async move {
            let (jf_url, jf_key, plex_url, plex_token, kodi_url) = {
                let dls = downloads.lock().await;
                let read_s = |k: &str| dls.get_setting(k).ok().flatten().unwrap_or_default().trim().to_string();
                let j_u = {
                    let s = read_s("jellyfin_url");
                    if s.is_empty() { cfg_jf_url } else { s }
                };
                let j_k = {
                    let s = read_s("jellyfin_api_key");
                    if s.is_empty() { cfg_jf_key.unwrap_or_default() } else { s }
                };
                let p_u = {
                    let s = read_s("plex_url");
                    if s.is_empty() { cfg_plex_url.unwrap_or_default() } else { s }
                };
                let p_t = read_s("plex_token");
                let k_u = read_s("kodi_url");
                (j_u, j_k, p_u, p_t, k_u)
            };

            let Ok(client) = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
            else {
                return;
            };

            // 1. Jellyfin / Emby
            if !jf_url.is_empty() {
                let url = format!("{}/Library/Refresh", jf_url.trim_end_matches('/'));
                let mut req = client.post(&url);
                if !jf_key.is_empty() {
                    req = req.header("Authorization", format!("MediaBrowser Token=\"{jf_key}\""));
                }
                let _ = req.send().await;
            }

            // 2. Plex Media Server
            if !plex_url.is_empty() {
                let base = plex_url.trim_end_matches('/');
                let refresh_url = if base.contains("/library/") {
                    base.to_string()
                } else {
                    format!("{base}/library/sections/all/refresh")
                };
                let final_url = if !plex_token.is_empty() && !refresh_url.contains("X-Plex-Token=") {
                    let sep = if refresh_url.contains('?') { '&' } else { '?' };
                    format!("{refresh_url}{sep}X-Plex-Token={plex_token}")
                } else {
                    refresh_url
                };
                let _ = client.get(&final_url).send().await;
            }

            // 3. Kodi JSON-RPC
            if !kodi_url.is_empty() {
                let url = format!("{}/jsonrpc", kodi_url.trim_end_matches('/'));
                let payload = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "VideoLibrary.Scan",
                    "id": 1
                })
                .to_string();
                let _ = client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .body(payload)
                    .send()
                    .await;
            }

            info!("[ORGANIZER] Dispatched library refresh signal to configured media servers");
        });
    }

    /// Organiza un único archivo completado mediante su ID de descarga
    pub async fn organize_download(
        state: &AppState,
        download_id: &str,
        completed_file_path: &Path,
    ) -> Result<Option<PathBuf>, String> {
        if !is_organize_enabled(state).await {
            return Ok(None);
        }
        if !completed_file_path.is_file() {
            return Ok(None);
        }

        let server_profile = active_media_server_profile(state).await;

        // Omitir archivos que ya están organizados dentro de una subcarpeta de película o serie
        if is_already_in_organized_library(state, completed_file_path).await {
            return Ok(None);
        }

        let Some(raw_file_name) = completed_file_path.file_name().and_then(|n| n.to_str()) else {
            return Ok(None);
        };
        if !mule_network_policy::content_policy::is_allowed_video_filename(raw_file_name) {
            return Ok(None);
        }
        let ext = mule_network_policy::content_policy::normalized_extension(raw_file_name)
            .unwrap_or_else(|| "mkv".to_string());

        // Rechazar clips de muestra / tráiler para evitar organizarlos como episodios o películas completos
        if let Some(fname) = completed_file_path.file_name().and_then(|n| n.to_str()) {
            let lower = fname.to_lowercase();
            let size = completed_file_path.metadata().map(|m| m.len()).unwrap_or(0);
            if size > 0
                && size < MAX_SAMPLE_VIDEO_BYTES
                && (lower.contains("sample") || lower.contains("trailer") || lower.contains("rarbg.com"))
            {
                let _ = std::fs::remove_file(completed_file_path);
                info!(
                    "[ORGANIZER] Removed sample/trailer video clip '{}' ({} bytes)",
                    completed_file_path.display(),
                    size
                );
                return Ok(None);
            }
        }

        let now = now_ms();
        let quality = detect_quality_tag(&completed_file_path.to_string_lossy());
        let effective_dl_dir = state.effective_download_dir().await;
        let require_net = state.nas_require_network_mount().await;

        // 1. Comprobar si la descarga corresponde a un episodio de TV
        let ep_match = {
            let dl_svc = state.downloads.lock().await;
            dl_svc.db().find_episode_by_download_id(download_id).ok().flatten()
        };

        if let Some(ep) = ep_match {
            let series_item = {
                let dl_svc = state.downloads.lock().await;
                dl_svc.db().get_media_item(&ep.media_id).ok().flatten()
            };

            if let Some(series) = series_item {
                let effective_series_root = state.effective_series_dir().await;
                let active_series_root = resolve_active_root_folder(
                    state,
                    &series.root_folder,
                    &effective_series_root,
                    &effective_dl_dir,
                    "Series",
                );
                ensure_destination_mount_ready(&active_series_root, require_net).await?;

                let series_title = sanitize_name(&series.title);
                let series_folder = resolve_series_folder_name(
                    &active_series_root,
                    &series.title,
                    series.year,
                    series.tvdb_id,
                    server_profile,
                );

                let season_folder = format!("Season {:02}", ep.season_number);

                let ep_title_clean = sanitize_name(&ep.title);
                let ep_file_name = if !ep_title_clean.is_empty() && !ep_title_clean.to_lowercase().starts_with("episode") {
                    format!(
                        "{} - S{:02}E{:02} - {} [{}].{}",
                        series_title, ep.season_number, ep.episode_number, ep_title_clean, quality, ext
                    )
                } else {
                    format!(
                        "{} - S{:02}E{:02} [{}].{}",
                        series_title, ep.season_number, ep.episode_number, quality, ext
                    )
                };

                let dest_dir = active_series_root
                    .join(&series_folder)
                    .join(&season_folder);
                let dest_file = dest_dir.join(&ep_file_name);

                ensure_destination_mount_ready(&dest_file, require_net).await?;
                copy_companion_subtitles(completed_file_path, &dest_file).await;

                hardlink_or_copy(completed_file_path, &dest_file)
                    .await
                    .map_err(|e| format!("Hardlink/copy failed: {e}"))?;

                purge_superseded_episode_files(
                    &dest_dir,
                    ep.season_number,
                    ep.episode_number,
                    &dest_file,
                    ep.file_path.as_deref(),
                );

                let dest_str = dest_file.to_string_lossy().to_string();
                {
                    let mut dl_svc = state.downloads.lock().await;
                    if let Ok(dl_uuid) = Uuid::parse_str(download_id) {
                        let _ = dl_svc.update_target_path(&dl_uuid, &dest_file);
                    }
                    let orig_release = completed_file_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string());
                    let _ = dl_svc.db().update_episode_status(
                        &ep.id,
                        "completed",
                        Some(&dest_str),
                        None,
                        now,
                    );
                    if let Some(ref rel) = orig_release {
                        let _ = dl_svc.db().set_episode_original_release_name(&ep.id, rel, now);
                    }

                    // Si todos los episodios monitorizados están ahora completados, marcar la serie como completada
                    let all_eps = dl_svc.db().get_episodes_for_media(&series.id).unwrap_or_default();
                    let remaining = all_eps.iter().filter(|e| e.monitored && e.status != "completed").count();
                    if remaining == 0 {
                        let _ = dl_svc.db().update_media_status(&series.id, "completed", None, now);
                        info!("[ORGANIZER] All episodes completed for series '{}'; marked completed!", series.title);
                    }
                }

                info!(
                    "[ORGANIZER] Organized episode '{} S{:02}E{:02}' -> '{}'",
                    series.title, ep.season_number, ep.episode_number, dest_str
                );

                // Obtener subtítulos en español en segundo plano automáticamente
                let sub_target = dest_file.clone();
                let sub_title = series.title.clone();
                let sub_year = series.year;
                let sub_imdb = series.imdb_id.clone();
                let s_num = ep.season_number;
                let e_num = ep.episode_number;
                tokio::spawn(async move {
                    let _ = crate::services::SubtitleService::auto_fetch_for_video(
                        &sub_target,
                        &sub_title,
                        sub_year,
                        sub_imdb.as_deref(),
                        false,
                        Some((s_num, e_num)),
                        "spa",
                    ).await;
                });

                Self::notify_media_servers(state);
                let series_root_dir = active_series_root.join(&series_folder);
                write_nfo_and_artwork_if_enabled(state, &series_root_dir, &series, false).await;
                return Ok(Some(dest_file));
            }
        }

        // 2. Comprobar si la descarga corresponde a una película
        let movie_match = {
            let dl_svc = state.downloads.lock().await;
            dl_svc.db().find_media_by_download_id(download_id).ok().flatten().filter(|m| m.media_type == "movie")
        };

        if let Some(item) = movie_match {
            let year_str = item.year.map(|y| y.to_string()).unwrap_or_else(|| "".to_string());
            let title_clean = sanitize_name(&item.title);

            let folder_name = format_movie_folder_name(&item.title, item.year, item.tmdb_id, server_profile);

            let file_name = if !year_str.is_empty() {
                format!("{} ({}) [{}].{}", title_clean, year_str, quality, ext)
            } else {
                format!("{} [{}].{}", title_clean, quality, ext)
            };

            let effective_movies_root = state.effective_movies_dir().await;
            let active_movies_root = resolve_active_root_folder(
                state,
                &item.root_folder,
                &effective_movies_root,
                &effective_dl_dir,
                "Movies",
            );
            let dest_dir = active_movies_root.join(&folder_name);
            let dest_file = dest_dir.join(&file_name);

            ensure_destination_mount_ready(&dest_file, require_net).await?;
            copy_companion_subtitles(completed_file_path, &dest_file).await;

            hardlink_or_copy(completed_file_path, &dest_file)
                .await
                .map_err(|e| format!("Hardlink/copy failed: {e}"))?;

            purge_superseded_movie_files(&dest_dir, &dest_file, item.file_path.as_deref());

            let orig_release = completed_file_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string());
            let dest_str = dest_file.to_string_lossy().to_string();
            {
                let mut dl_svc = state.downloads.lock().await;
                if let Ok(dl_uuid) = Uuid::parse_str(download_id) {
                    let _ = dl_svc.update_target_path(&dl_uuid, &dest_file);
                }
                let _ = dl_svc.db().update_media_completed(&item.id, &dest_str, now);
                if let Some(ref rel) = orig_release {
                    let _ = dl_svc.db().set_media_original_release_name(&item.id, rel, now);
                }
            }

            info!(
                "[ORGANIZER] Organized movie '{}' -> '{}'",
                item.title, dest_str
            );
            write_nfo_and_artwork_if_enabled(state, &dest_dir, &item, true).await;

            // Obtener subtítulos en español en segundo plano automáticamente
            let sub_target = dest_file.clone();
            let sub_title = item.title.clone();
            let sub_year = item.year;
            let sub_imdb = item.imdb_id.clone();
            tokio::spawn(async move {
                let _ = crate::services::SubtitleService::auto_fetch_for_video(
                    &sub_target,
                    &sub_title,
                    sub_year,
                    sub_imdb.as_deref(),
                    true,
                    None,
                    "spa",
                ).await;
            });

            Self::notify_media_servers(state);
            return Ok(Some(dest_file));
        }

        // 3. Respaldo: Identificación automática para descargas manuales de eMule/Kad/Torrent
        let file_stem = completed_file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let raw_filename = completed_file_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        let clean_filename = fix_mojibake(raw_filename);
        let clean_stem = fix_mojibake(file_stem);

        // A. ¿Es un episodio de una serie conocida?
        if let Some((s_num, ep_num)) = crate::services::disk_scanner::extract_season_episode(None, &clean_filename) {
            let dl_svc = state.downloads.lock().await;
            let db = dl_svc.db();
            if let Ok(shows) = db.list_media_items(Some("tv"), None) {
                let matched_show = shows.into_iter().find(|s| {
                    let st = s.title.to_lowercase();
                    let cf = clean_filename.to_lowercase();
                    cf.contains(&st) || (s.original_title.as_ref().map(|ot| cf.contains(&ot.to_lowercase())).unwrap_or(false))
                });

                if let Some(series) = matched_show {
                    let existing_ep = db.get_episodes_for_media(&series.id).ok().and_then(|eps| {
                        eps.into_iter().find(|e| e.season_number == s_num && e.episode_number == ep_num)
                    });
                    drop(dl_svc);

                    let effective_series_root = state.effective_series_dir().await;
                    let active_series_root = resolve_active_root_folder(
                        state,
                        &series.root_folder,
                        &effective_series_root,
                        &effective_dl_dir,
                        "Series",
                    );
                    ensure_destination_mount_ready(&active_series_root, require_net).await?;

                    let series_title = sanitize_name(&series.title);
                    let series_folder = resolve_series_folder_name(
                        &active_series_root,
                        &series.title,
                        series.year,
                        series.tvdb_id,
                        server_profile,
                    );
                    let season_folder = format!("Season {:02}", s_num);
                    let dest_dir = active_series_root
                        .join(&series_folder)
                        .join(&season_folder);

                    let ep_title_clean = existing_ep.as_ref().map(|e| sanitize_name(&e.title)).unwrap_or_default();
                    let ep_file_name = if !ep_title_clean.is_empty() && !ep_title_clean.to_lowercase().starts_with("episode") && !ep_title_clean.ends_with(".mkv") {
                        format!("{} - S{:02}E{:02} - {} [{}].{}", series_title, s_num, ep_num, ep_title_clean, quality, ext)
                    } else {
                        format!("{} - S{:02}E{:02} [{}].{}", series_title, s_num, ep_num, quality, ext)
                    };

                    let dest_file = dest_dir.join(&ep_file_name);
                    let old_ep_fp = existing_ep.as_ref().and_then(|e| e.file_path.clone());

                    ensure_destination_mount_ready(&dest_file, require_net).await?;
                    copy_companion_subtitles(completed_file_path, &dest_file).await;

                    hardlink_or_copy(completed_file_path, &dest_file)
                        .await
                        .map_err(|e| format!("Hardlink/copy failed: {e}"))?;

                    purge_superseded_episode_files(
                        &dest_dir,
                        s_num,
                        ep_num,
                        &dest_file,
                        old_ep_fp.as_deref(),
                    );

                    let dest_str = dest_file.to_string_lossy().to_string();
                    {
                        let mut dl_svc = state.downloads.lock().await;
                        if let Ok(dl_uuid) = Uuid::parse_str(download_id) {
                            let _ = dl_svc.update_target_path(&dl_uuid, &dest_file);
                        }
                        let orig_release = completed_file_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(|s| s.to_string());
                        if let Some(mut ep) = existing_ep {
                            ep.file_path = Some(dest_str.clone());
                            if orig_release.is_some() {
                                ep.original_release_name = orig_release;
                            }
                            ep.status = "completed".to_string();
                            ep.download_id = Some(download_id.to_string());
                            ep.updated_at = now;
                            let _ = dl_svc.db().insert_episode(&ep);
                        } else {
                            let ep_id = Uuid::new_v4().to_string();
                            let ep_item = EpisodeItem {
                                id: ep_id,
                                media_id: series.id.clone(),
                                season_number: s_num,
                                episode_number: ep_num,
                                title: ep_file_name.clone(),
                                air_date: None,
                                status: "completed".to_string(),
                                monitored: true,
                                file_path: Some(dest_str.clone()),
                                original_release_name: orig_release,
                                download_id: Some(download_id.to_string()),
                                created_at: now,
                                updated_at: now,
                            };
                            let _ = dl_svc.db().insert_episode(&ep_item);
                        }
                    }

                    info!("[ORGANIZER] Auto-organized manual download episode '{} S{:02}E{:02}' -> '{}'", series.title, s_num, ep_num, dest_str);

                    let sub_target = dest_file.clone();
                    let sub_title = series.title.clone();
                    let sub_year = series.year;
                    let sub_imdb = series.imdb_id.clone();
                    tokio::spawn(async move {
                        let _ = crate::services::SubtitleService::auto_fetch_for_video(
                            &sub_target, &sub_title, sub_year, sub_imdb.as_deref(), false, Some((s_num, ep_num)), "spa"
                        ).await;
                    });

                    Self::notify_media_servers(state);
                    return Ok(Some(dest_file));
                }
            }
        }

        // B. ¿Es una película?
        let parsed_year = extract_year(&clean_stem);
        let parsed_title = clean_movie_title(&clean_stem);

        if !parsed_title.is_empty() {
            let dl_svc = state.downloads.lock().await;
            let db = dl_svc.db();

            let mut matched_movie = None;
            if let Ok(movies) = db.list_media_items(Some("movie"), None) {
                let norm_pt = crate::services::scorer::normalize_accents_and_symbols(&parsed_title);
                matched_movie = movies.into_iter().find(|m| {
                    let mt = m.title.to_lowercase();
                    let pt = parsed_title.to_lowercase();
                    let norm_mt = crate::services::scorer::normalize_accents_and_symbols(&m.title);
                    let norm_orig = m.original_title.as_ref().map(|o| crate::services::scorer::normalize_accents_and_symbols(o)).unwrap_or_default();
                    (mt == pt || mt.contains(&pt) || pt.contains(&mt)
                        || norm_mt == norm_pt || norm_mt.contains(&norm_pt) || norm_pt.contains(&norm_mt)
                        || (!norm_orig.is_empty() && (norm_orig == norm_pt || norm_orig.contains(&norm_pt) || norm_pt.contains(&norm_orig))))
                        && (parsed_year.is_none() || m.year == parsed_year || m.year.is_none())
                });
            }
            drop(dl_svc);

            if matched_movie.is_none() {
                if let Ok(results) = state.media_metadata.search_movies(&parsed_title).await {
                    let best = if let Some(y) = parsed_year {
                        results.into_iter().find(|r| r.year == Some(y))
                    } else {
                        results.into_iter().next()
                    };

                    let effective_movies_root = state.effective_movies_dir().await;
                    if let Some(meta) = best {
                        let id = Uuid::new_v4().to_string();
                        let item = MediaItem {
                            id: id.clone(),
                            tmdb_id: meta.tmdb_id,
                            imdb_id: meta.imdb_id,
                            tvdb_id: None,
                            media_type: "movie".to_string(),
                            title: meta.title,
                            original_title: meta.original_title,
                            year: meta.year.or(parsed_year),
                            overview: meta.overview,
                            poster_path: meta.poster_url,
                            backdrop_path: meta.backdrop_url,
                            status: "completed".to_string(),
                            monitored: true,
                            quality_profile: "1080p-es".to_string(),
                            root_folder: effective_movies_root.to_string_lossy().to_string(),
                            file_path: None,
                            original_release_name: None,
                            download_id: Some(download_id.to_string()),
                            created_at: now,
                            updated_at: now,
                        };
                        let dl_svc = state.downloads.lock().await;
                        let _ = dl_svc.db().insert_media_item(&item);
                        matched_movie = Some(item);
                    }
                }
            }

            let effective_movies_root = state.effective_movies_dir().await;
            let final_movie = matched_movie.unwrap_or_else(|| {
                MediaItem {
                    id: Uuid::new_v4().to_string(),
                    tmdb_id: None,
                    imdb_id: None,
                    tvdb_id: None,
                    media_type: "movie".to_string(),
                    title: parsed_title.clone(),
                    original_title: None,
                    year: parsed_year,
                    overview: None,
                    poster_path: None,
                    backdrop_path: None,
                    status: "completed".to_string(),
                    monitored: true,
                    quality_profile: "1080p-es".to_string(),
                    root_folder: effective_movies_root.to_string_lossy().to_string(),
                    file_path: None,
                    original_release_name: None,
                    download_id: Some(download_id.to_string()),
                    created_at: now,
                    updated_at: now,
                }
            });

            let year_str = final_movie.year.map(|y| y.to_string()).unwrap_or_default();
            let title_clean = sanitize_name(&final_movie.title);

            let folder_name = format_movie_folder_name(
                &final_movie.title,
                final_movie.year,
                final_movie.tmdb_id,
                server_profile,
            );

            let file_name = if !year_str.is_empty() {
                format!("{} ({}) [{}].{}", title_clean, year_str, quality, ext)
            } else {
                format!("{} [{}].{}", title_clean, quality, ext)
            };

            let active_movies_root = resolve_active_root_folder(
                state,
                &final_movie.root_folder,
                &effective_movies_root,
                &effective_dl_dir,
                "Movies",
            );
            let dest_dir = active_movies_root.join(&folder_name);
            let dest_file = dest_dir.join(&file_name);

            ensure_destination_mount_ready(&dest_file, require_net).await?;
            copy_companion_subtitles(completed_file_path, &dest_file).await;

            hardlink_or_copy(completed_file_path, &dest_file)
                .await
                .map_err(|e| format!("Hardlink/copy failed: {e}"))?;

            purge_superseded_movie_files(&dest_dir, &dest_file, final_movie.file_path.as_deref());

            let orig_release = completed_file_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string());
            let dest_str = dest_file.to_string_lossy().to_string();
            {
                let mut dl_svc = state.downloads.lock().await;
                if let Ok(dl_uuid) = Uuid::parse_str(download_id) {
                    let _ = dl_svc.update_target_path(&dl_uuid, &dest_file);
                }
                let _ = dl_svc.db().update_media_completed(&final_movie.id, &dest_str, now);
                if let Some(ref rel) = orig_release {
                    let _ = dl_svc.db().set_media_original_release_name(&final_movie.id, rel, now);
                }
            }

            info!("[ORGANIZER] Auto-organized manual download movie '{}' -> '{}'", final_movie.title, dest_str);
            write_nfo_and_artwork_if_enabled(state, &dest_dir, &final_movie, true).await;

            let sub_target = dest_file.clone();
            let sub_title = final_movie.title.clone();
            let sub_year = final_movie.year;
            let sub_imdb = final_movie.imdb_id.clone();
            tokio::spawn(async move {
                let _ = crate::services::SubtitleService::auto_fetch_for_video(
                    &sub_target, &sub_title, sub_year, sub_imdb.as_deref(), true, None, "spa"
                ).await;
            });

            Self::notify_media_servers(state);
            return Ok(Some(dest_file));
        }

        Ok(None)
    }

    /// Barrido completo sobre descargas activas de eMule y torrents de BitTorrent para organizar archivos terminados
    pub async fn sweep_and_organize(state: &AppState) -> Value {
        if !is_organize_enabled(state).await {
            return serde_json::json!({ "organized": [], "skipped": "simple_mode" });
        }
        let mut organized_files = Vec::new();

        // 1. Comprobar descargas de eMule
        let completed_ed2k: Vec<(String, PathBuf)> = {
            let dl_svc = state.downloads.lock().await;
            let list = dl_svc.list_download_details();
            list.into_iter()
                .filter(|d| d.snapshot.status == "completed")
                .map(|d| (d.snapshot.id.to_string(), PathBuf::from(d.target_path)))
                .collect()
        };

        let effective_dl_dir = state.effective_download_dir().await;
        for (dl_id, path) in completed_ed2k {
            if path.is_file() && !is_already_in_organized_library(state, &path).await {
                if let Ok(Some(dest)) = Self::organize_download(state, &dl_id, &path).await {
                    organized_files.push(json!({
                        "protocol": "ed2k",
                        "downloadId": dl_id,
                        "source": path.to_string_lossy(),
                        "target": dest.to_string_lossy()
                    }));
                }
            }
        }

        // 2. Comprobar torrents de BitTorrent
        if let Ok(torrent_engine) = state.torrent_engine().await {
            let torrents = torrent_engine.list();
            for tor in torrents {
                if tor.status == "completed" {
                    let out_dir = PathBuf::from(&tor.output_dir);
                    // Archivo individual o directorio
                    let candidate = out_dir.join(&tor.name);
                    let mut torrent_organized_any = false;
                    if candidate.is_file() {
                        if let Ok(Some(dest)) = Self::organize_download(state, &tor.info_hash, &candidate).await {
                            torrent_organized_any = true;
                            organized_files.push(json!({
                                "protocol": "torrent",
                                "downloadId": tor.info_hash,
                                "source": candidate.to_string_lossy(),
                                "target": dest.to_string_lossy()
                            }));
                        }
                    } else if candidate.is_dir() {
                        // Escanear directorio en busca de archivos de vídeo
                        if let Ok(entries) = std::fs::read_dir(&candidate) {
                            for entry in entries.flatten() {
                                let p = entry.path();
                                if p.is_file() {
                                    let is_video = p
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .is_some_and(mule_network_policy::content_policy::is_allowed_video_filename);
                                    if is_video {
                                        if let Ok(Some(dest)) = Self::organize_download(state, &tor.info_hash, &p).await {
                                            torrent_organized_any = true;
                                            organized_files.push(json!({
                                                "protocol": "torrent",
                                                "downloadId": tor.info_hash,
                                                "source": p.to_string_lossy(),
                                                "target": dest.to_string_lossy()
                                            }));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Una vez que todos los archivos de vídeo en el torrent completado han sido organizados (o el candidato ya no existe),
                    // eliminar el torrent del motor y borrar cualquier archivo/carpeta restante en incoming.
                    if torrent_organized_any || !candidate.exists() {
                        let _ = torrent_engine.delete(tor.id, true).await;
                        if candidate.is_dir() {
                            let _ = std::fs::remove_dir_all(&candidate);
                        } else if candidate.is_file() {
                            let _ = std::fs::remove_file(&candidate);
                        }
                        info!(
                            "[ORGANIZER] Cleaned up completed torrent '{}' from incoming after organization",
                            tor.name
                        );
                    }
                }
            }
        }

        // 3. Escanear directorios de descarga y los directorios adicionales configurados en busca de archivos de vídeo independientes
        let mut incoming_dirs = vec![effective_dl_dir, state.config.default_download_dir.clone()];
        for d in &state.config.extra_incoming_dirs {
            if !incoming_dirs.contains(d) {
                incoming_dirs.push(d.clone());
            }
        }

        for inc_dir in &incoming_dirs {
            if inc_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(inc_dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_file() && !is_already_in_organized_library(state, &p).await {
                            let size = p.metadata().map(|m| m.len()).unwrap_or(0);
                            if size < MIN_STANDALONE_VIDEO_BYTES {
                                continue;
                            }
                            let is_video = p
                                .file_name()
                                .and_then(|n| n.to_str())
                                .is_some_and(mule_network_policy::content_policy::is_allowed_video_filename);
                            if is_video {
                                let pseudo_id = format!("manual-{}", p.file_name().unwrap_or_default().to_string_lossy());
                                if let Ok(Some(dest)) = Self::organize_download(state, &pseudo_id, &p).await {
                                    organized_files.push(json!({
                                        "protocol": "manual",
                                        "downloadId": pseudo_id,
                                        "source": p.to_string_lossy(),
                                        "target": dest.to_string_lossy()
                                    }));
                                }
                            }
                        }
                    }
                }
            }
        }

        json!({
            "organized": organized_files
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fix_mojibake() {
        assert_eq!(fix_mojibake("La cabaÃ±a (2017)"), "La cabaña (2017)");
        assert_eq!(fix_mojibake("Normal Title (2020)"), "Normal Title (2020)");
    }

    #[test]
    fn test_extract_year_organizer() {
        assert_eq!(extract_year("La cabaña (2017)"), Some(2017));
        assert_eq!(extract_year("The.Matrix.1999.1080p"), Some(1999));
        assert_eq!(extract_year("Title Without Year"), None);
    }

    #[test]
    fn test_clean_movie_title() {
        assert_eq!(clean_movie_title("La cabaña (2017)"), "La cabaña");
        assert_eq!(clean_movie_title("The Matrix (1999) [1080p]"), "The Matrix");
        assert_eq!(clean_movie_title("Oddity.2024.1080p.x264"), "Oddity");
    }

    #[tokio::test]
    async fn test_hardlink_or_copy_removes_source_from_incoming() {
        use tempfile::tempdir;
        let incoming_dir = tempdir().unwrap();
        let library_dir = tempdir().unwrap();

        let src_file = incoming_dir.path().join("Episode.S01E01.mkv");
        std::fs::write(&src_file, b"test video data payload").unwrap();

        let tgt_file = library_dir.path().join("Series").join("Season 01").join("Series - S01E01 [1080p].mkv");

        hardlink_or_copy(&src_file, &tgt_file).await.unwrap();

        assert!(tgt_file.exists(), "Target file must exist in organized library");
        assert!(!src_file.exists(), "Source file must be unlinked from incoming directory");
        assert_eq!(std::fs::read(&tgt_file).unwrap(), b"test video data payload");
    }

    #[tokio::test]
    async fn test_nas_root_dir_priority_and_localized_autodiscovery() {
        use tempfile::tempdir;
        let data_dir = tempdir().unwrap();
        let nas_dir = tempdir().unwrap();
        std::fs::create_dir(nas_dir.path().join("Películas")).unwrap();
        std::fs::create_dir(nas_dir.path().join("TV Shows")).unwrap();

        let cfg = crate::config::DaemonConfig::new(data_dir.path().to_path_buf(), "tok");
        let stale_local_movies = cfg.default_download_dir.join("media").join("Movies");
        let dl_svc =
            crate::services::downloads::DownloadService::in_memory(data_dir.path().join("temp"))
                .unwrap();
        let state = AppState::new(cfg, dl_svc);

        // Simular una petición antigua de UI que envía nasRootDir junto al fallback local en moviesDir
        crate::rpc::handlers::settings::handle_settings_set(
            &state,
            Some(json!({
                "nasRootDir": nas_dir.path().to_string_lossy(),
                "moviesDir": stale_local_movies.to_string_lossy(),
            })),
        )
        .await
        .unwrap();

        assert_eq!(
            state.effective_movies_dir().await,
            nas_dir.path().join("Películas")
        );
        assert_eq!(
            state.effective_series_dir().await,
            nas_dir.path().join("TV Shows")
        );

        let settings_resp = crate::rpc::handlers::settings::handle_settings_get(&state)
            .await
            .unwrap();
        assert_eq!(settings_resp["moviesDir"], "");
        assert_eq!(
            settings_resp["effectiveMoviesDir"],
            nas_dir.path().join("Películas").to_string_lossy().as_ref()
        );
        assert_eq!(
            settings_resp["effectiveSeriesDir"],
            nas_dir.path().join("TV Shows").to_string_lossy().as_ref()
        );
    }

    #[tokio::test]
    async fn test_organizer_require_network_mount_blocks_local_destination() {
        use tempfile::tempdir;
        let data_dir = tempdir().unwrap();
        let nas_dir = tempdir().unwrap();
        let incoming_dir = tempdir().unwrap();

        let cfg = crate::config::DaemonConfig::new(data_dir.path().to_path_buf(), "tok");
        let dl_svc =
            crate::services::downloads::DownloadService::in_memory(data_dir.path().join("temp"))
                .unwrap();
        let state = AppState::new(cfg, dl_svc);

        crate::rpc::handlers::settings::handle_settings_set(
            &state,
            Some(json!({
                "nasRootDir": nas_dir.path().to_string_lossy(),
                "nasRequireNfs": true,
            })),
        )
        .await
        .unwrap();

        let item = MediaItem {
            id: "movie-guard-1".to_string(),
            tmdb_id: Some(603),
            imdb_id: None,
            tvdb_id: None,
            media_type: "movie".to_string(),
            title: "The Matrix".to_string(),
            original_title: None,
            year: Some(1999),
            overview: None,
            poster_path: None,
            backdrop_path: None,
            status: "downloading".to_string(),
            monitored: true,
            quality_profile: "1080p-es".to_string(),
            root_folder: nas_dir.path().join("Movies").to_string_lossy().to_string(),
            file_path: None,
            original_release_name: None,
            download_id: Some("dl-guard-1".to_string()),
            created_at: 1,
            updated_at: 1,
        };
        {
            let dl_guard = state.downloads.lock().await;
            dl_guard.db().insert_media_item(&item).unwrap();
        }

        let src_file = incoming_dir.path().join("The.Matrix.1999.1080p.mkv");
        std::fs::write(&src_file, b"video payload").unwrap();

        let res = MediaOrganizer::organize_download(&state, "dl-guard-1", &src_file).await;
        assert!(res.is_err(), "Debe bloquear la transferencia a disco local cuando nasRequireNfs está activo");
        assert!(src_file.exists(), "El archivo fuente en incoming debe conservarse intacto");
        assert!(!nas_dir.path().join("Movies").exists(), "No debe crear subcarpetas en destino bloqueado");
    }

    #[test]
    fn test_format_folder_names_by_profile() {
        assert_eq!(
            format_movie_folder_name("Matrix", Some(1999), Some(603), MediaServerProfile::Jellyfin),
            "Matrix (1999) [tmdbid-603]"
        );
        assert_eq!(
            format_movie_folder_name("Matrix", Some(1999), Some(603), MediaServerProfile::Plex),
            "Matrix (1999) {tmdb-603}"
        );
        assert_eq!(
            format_movie_folder_name("Matrix", Some(1999), Some(603), MediaServerProfile::Kodi),
            "Matrix (1999)"
        );
        assert_eq!(
            format_series_folder_name("Band of Brothers", Some(2001), Some(74205), MediaServerProfile::Plex),
            "Band of Brothers (2001) {tvdb-74205}"
        );
    }

    #[test]
    fn test_build_movie_nfo_xml_escapes_special_chars() {
        let item = MediaItem {
            id: "m1".to_string(),
            tmdb_id: Some(603),
            imdb_id: Some("tt0133093".to_string()),
            tvdb_id: None,
            media_type: "movie".to_string(),
            title: "Tom & Jerry <Test>".to_string(),
            original_title: None,
            year: Some(1999),
            overview: Some("Sinopsis \"especial\"".to_string()),
            poster_path: None,
            backdrop_path: None,
            status: "completed".to_string(),
            monitored: true,
            quality_profile: "1080p-es".to_string(),
            root_folder: "/movies".to_string(),
            file_path: None,
            original_release_name: None,
            download_id: None,
            created_at: 1,
            updated_at: 1,
        };
        let xml = build_movie_nfo_xml(&item);
        assert!(xml.contains("<title>Tom &amp; Jerry &lt;Test&gt;</title>"));
        assert!(xml.contains("<plot>Sinopsis &quot;especial&quot;</plot>"));
        assert!(xml.contains("<uniqueid type=\"tmdb\" default=\"true\">603</uniqueid>"));
    }
}

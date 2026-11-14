// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Creación y restauración de copias de seguridad portables (`.zip`) de Pulse Core.
//! Empaqueta la identidad criptográfica (`cryptkey.dat`, `userhash.dat`), nodos Kad
//! (`nodes.dat`), lista de servidores (`server.met`) y el manifiesto de ajustes,
//! créditos de pares, amigos, categorías, indexadores y biblioteca multimedia.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

use data_encoding::BASE64;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::JsonRpcError;
use crate::state::{AppState, SETTING_WEBUI_PASSWORD_HASH};

/// Nombre del manifiesto JSON incluido en el archivo `.zip` de copia de seguridad.
const BACKUP_MANIFEST_FILENAME: &str = "backup_manifest.json";
/// Archivo de contactos Kad de eMule.
const NODES_DAT_FILENAME: &str = "nodes.dat";
/// Archivo binario de servidores eD2K.
const SERVER_MET_FILENAME: &str = "server.met";
/// Archivo binario con el hash de usuario de 16 bytes.
const USERHASH_DAT_FILENAME: &str = "userhash.dat";
/// Tamaño máximo permitido para el `.zip` recibido o generado en memoria (64 MiB).
const MAX_BACKUP_ZIP_BYTES: usize = 64 * 1024 * 1024;
/// Límite de registros de créditos de pares incluidos en una copia.
const MAX_BACKUP_CREDITS: usize = 100_000;
/// Límite de contactos Kad incluidos en una copia.
const MAX_BACKUP_KAD_CONTACTS: usize = 50_000;
/// Tamaño esperado del hash de usuario binario de eMule (`userhash.dat`).
const USERHASH_LEN: usize = 16;

/// Firmas y constantes del formato estándar PKZIP.
const ZIP_LOCAL_FILE_HEADER_SIG: u32 = 0x0403_4b50;
const ZIP_CENTRAL_DIR_HEADER_SIG: u32 = 0x0201_4b50;
const ZIP_END_OF_CENTRAL_DIR_SIG: u32 = 0x0605_4b50;
const ZIP_VERSION_20: u16 = 20;
const ZIP_FLAG_UTF8: u16 = 0x0800;
const ZIP_METHOD_STORE: u16 = 0;
const ZIP_METHOD_DEFLATE: u16 = 8;
const ZIP_EOCD_MIN_LEN: usize = 22;
const ZIP_MAX_COMMENT_SEARCH: usize = 65_557;
const ZIP_LOCAL_HEADER_FIXED_LEN: usize = 30;
const ZIP_CENTRAL_HEADER_FIXED_LEN: usize = 46;
const ZIP_DOS_DEFAULT_DATE: u16 = 0x5421;
const CRC32_POLY_REFLECTED: u32 = 0xEDB8_8320;

/// Claves de ajuste que representan rutas del sistema local y deben omitirse
/// si la copia se restaura en un sistema operativo incompatible.
const LOCAL_PATH_SETTING_KEYS: &[&str] = &[
    "default_download_directory",
    "temp_dir",
    "movies_dir",
    "series_dir",
    "nas_root_dir",
    "ipfilter_path",
    "server_autoupdate_path",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest {
    version: String,
    created_at_unix_ms: i64,
    os: String,
    settings: Vec<(String, String)>,
    peer_credits: Vec<mule_core::model::PeerCredit>,
    friends: Vec<mule_core::model::Friend>,
    categories: Vec<mule_core::model::DownloadCategory>,
    servers: Vec<mule_core::model::Ed2kServer>,
    kad_contacts: Vec<mule_core::KadContact>,
    external_indexers: Vec<mule_core::model::ExternalIndexer>,
    media_items: Vec<mule_core::model::MediaItem>,
    episodes: Vec<mule_core::model::EpisodeItem>,
}

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (CRC32_POLY_REFLECTED & mask);
        }
    }
    !crc
}

fn read_u16_le(buf: &[u8], offset: usize) -> Result<u16, String> {
    let bytes: [u8; 2] = buf
        .get(offset..offset + 2)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| "Cabecera ZIP truncada (u16)".to_string())?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32_le(buf: &[u8], offset: usize) -> Result<u32, String> {
    let bytes: [u8; 4] = buf
        .get(offset..offset + 4)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| "Cabecera ZIP truncada (u32)".to_string())?;
    Ok(u32::from_le_bytes(bytes))
}

fn build_zip_archive(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
    struct CentralMeta<'a> {
        name: &'a str,
        crc: u32,
        method: u16,
        comp_size: u32,
        uncomp_size: u32,
        local_offset: u32,
    }

    let mut out = Vec::new();
    let mut central_entries = Vec::with_capacity(entries.len());

    for &(name, raw) in entries {
        if raw.len() > u32::MAX as usize {
            return Err(format!("Entrada demasiado grande para ZIP: {name}"));
        }
        let name_bytes = name.as_bytes();
        let name_len = u16::try_from(name_bytes.len())
            .map_err(|_| format!("Nombre de archivo ZIP demasiado largo: {name}"))?;

        let crc = crc32_ieee(raw);
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(raw)
            .map_err(|e| format!("Error comprimiendo {name}: {e}"))?;
        let deflated = encoder
            .finish()
            .map_err(|e| format!("Error finalizando compresión de {name}: {e}"))?;

        let (method, payload): (u16, Vec<u8>) = if deflated.len() < raw.len() {
            (ZIP_METHOD_DEFLATE, deflated)
        } else {
            (ZIP_METHOD_STORE, raw.to_vec())
        };

        let local_offset = u32::try_from(out.len())
            .map_err(|_| "El archivo ZIP supera el límite de 4 GiB".to_string())?;
        let comp_size = payload.len() as u32;
        let uncomp_size = raw.len() as u32;

        // Cabecera local de archivo PKZIP
        out.extend_from_slice(&ZIP_LOCAL_FILE_HEADER_SIG.to_le_bytes());
        out.extend_from_slice(&ZIP_VERSION_20.to_le_bytes());
        out.extend_from_slice(&ZIP_FLAG_UTF8.to_le_bytes());
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&ZIP_DOS_DEFAULT_DATE.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&comp_size.to_le_bytes());
        out.extend_from_slice(&uncomp_size.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(&payload);

        central_entries.push(CentralMeta {
            name,
            crc,
            method,
            comp_size,
            uncomp_size,
            local_offset,
        });
    }

    let cd_offset = u32::try_from(out.len())
        .map_err(|_| "El archivo ZIP supera el límite de 4 GiB".to_string())?;

    for meta in &central_entries {
        let name_bytes = meta.name.as_bytes();
        let name_len = name_bytes.len() as u16;
        out.extend_from_slice(&ZIP_CENTRAL_DIR_HEADER_SIG.to_le_bytes());
        out.extend_from_slice(&ZIP_VERSION_20.to_le_bytes());
        out.extend_from_slice(&ZIP_VERSION_20.to_le_bytes());
        out.extend_from_slice(&ZIP_FLAG_UTF8.to_le_bytes());
        out.extend_from_slice(&meta.method.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&ZIP_DOS_DEFAULT_DATE.to_le_bytes());
        out.extend_from_slice(&meta.crc.to_le_bytes());
        out.extend_from_slice(&meta.comp_size.to_le_bytes());
        out.extend_from_slice(&meta.uncomp_size.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&meta.local_offset.to_le_bytes());
        out.extend_from_slice(name_bytes);
    }

    let cd_size = u32::try_from(out.len() - cd_offset as usize)
        .map_err(|_| "El directorio central ZIP supera el límite de 4 GiB".to_string())?;
    let entry_count = u16::try_from(central_entries.len())
        .map_err(|_| "Demasiadas entradas en el archivo ZIP".to_string())?;

    // Fin del directorio central (EOCD)
    out.extend_from_slice(&ZIP_END_OF_CENTRAL_DIR_SIG.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&entry_count.to_le_bytes());
    out.extend_from_slice(&entry_count.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());

    if out.len() > MAX_BACKUP_ZIP_BYTES {
        return Err("La copia de seguridad supera el tamaño máximo permitido (64 MiB)".to_string());
    }
    Ok(out)
}

fn sanitize_zip_entry_name(raw_name: &str) -> Option<String> {
    let trimmed = raw_name.trim();
    if trimmed.is_empty()
        || trimmed.ends_with('/')
        || trimmed.ends_with('\\')
        || trimmed.contains("..")
    {
        return None;
    }
    let normalized = trimmed.replace('\\', "/");
    let file_name = normalized.rsplit('/').next()?.trim();
    if file_name.is_empty() || file_name.contains(':') {
        return None;
    }
    Some(file_name.to_string())
}

fn parse_zip_archive(data: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    if data.len() < ZIP_EOCD_MIN_LEN || data.len() > MAX_BACKUP_ZIP_BYTES {
        return Err("Archivo ZIP vacío o fuera del límite de tamaño (64 MiB)".to_string());
    }

    let min_eocd_pos = data.len().saturating_sub(ZIP_MAX_COMMENT_SEARCH);
    let max_eocd_pos = data.len() - ZIP_EOCD_MIN_LEN;
    let eocd_pos = (min_eocd_pos..=max_eocd_pos)
        .rev()
        .find(|&pos| read_u32_le(data, pos) == Ok(ZIP_END_OF_CENTRAL_DIR_SIG))
        .ok_or_else(|| "El archivo no es un ZIP válido (falta EOCD)".to_string())?;

    let total_entries = read_u16_le(data, eocd_pos + 10)? as usize;
    let cd_size = read_u32_le(data, eocd_pos + 12)? as usize;
    let cd_offset = read_u32_le(data, eocd_pos + 16)? as usize;
    let cd_end = cd_offset
        .checked_add(cd_size)
        .ok_or_else(|| "Desbordamiento de desplazamiento en directorio central ZIP".to_string())?;
    if cd_end > eocd_pos {
        return Err("Directorio central ZIP fuera de límites".to_string());
    }

    let mut cursor = cd_offset;
    let mut total_uncompressed = 0usize;
    let mut files = HashMap::new();

    for _ in 0..total_entries {
        if cursor + ZIP_CENTRAL_HEADER_FIXED_LEN > cd_end {
            return Err("Entrada de directorio central ZIP truncada".to_string());
        }
        if read_u32_le(data, cursor)? != ZIP_CENTRAL_DIR_HEADER_SIG {
            return Err("Firma de directorio central ZIP inválida".to_string());
        }

        let method = read_u16_le(data, cursor + 10)?;
        let expected_crc = read_u32_le(data, cursor + 16)?;
        let comp_size = read_u32_le(data, cursor + 20)? as usize;
        let uncomp_size = read_u32_le(data, cursor + 24)? as usize;
        let name_len = read_u16_le(data, cursor + 28)? as usize;
        let extra_len = read_u16_le(data, cursor + 30)? as usize;
        let comment_len = read_u16_le(data, cursor + 32)? as usize;
        let local_offset = read_u32_le(data, cursor + 42)? as usize;

        let name_start = cursor + ZIP_CENTRAL_HEADER_FIXED_LEN;
        let name_end = name_start + name_len;
        let next_cursor = name_end + extra_len + comment_len;
        if next_cursor > cd_end {
            return Err("Metadatos de entrada ZIP truncados".to_string());
        }
        let raw_name = std::str::from_utf8(&data[name_start..name_end])
            .map_err(|_| "Nombre de archivo ZIP no UTF-8".to_string())?;
        cursor = next_cursor;

        let Some(clean_name) = sanitize_zip_entry_name(raw_name) else {
            continue;
        };

        total_uncompressed = total_uncompressed
            .checked_add(uncomp_size)
            .ok_or_else(|| "Tamaño descomprimido excedido".to_string())?;
        if total_uncompressed > MAX_BACKUP_ZIP_BYTES {
            return Err(
                "El contenido descomprimido supera el máximo permitido (64 MiB)".to_string(),
            );
        }

        if local_offset + ZIP_LOCAL_HEADER_FIXED_LEN > cd_offset {
            return Err(format!("Cabecera local ZIP inválida para {clean_name}"));
        }
        if read_u32_le(data, local_offset)? != ZIP_LOCAL_FILE_HEADER_SIG {
            return Err(format!("Firma local ZIP inválida para {clean_name}"));
        }
        let local_name_len = read_u16_le(data, local_offset + 26)? as usize;
        let local_extra_len = read_u16_le(data, local_offset + 28)? as usize;
        let payload_start = local_offset + ZIP_LOCAL_HEADER_FIXED_LEN + local_name_len + local_extra_len;
        let payload_end = payload_start
            .checked_add(comp_size)
            .ok_or_else(|| "Desbordamiento de carga útil ZIP".to_string())?;
        if payload_end > cd_offset {
            return Err(format!("Datos comprimidos truncados para {clean_name}"));
        }

        let compressed = &data[payload_start..payload_end];
        let content = match method {
            ZIP_METHOD_STORE => compressed.to_vec(),
            ZIP_METHOD_DEFLATE => {
                let decoder = DeflateDecoder::new(compressed);
                let mut limited = decoder.take((MAX_BACKUP_ZIP_BYTES as u64) + 1);
                let mut out = Vec::with_capacity(uncomp_size.min(MAX_BACKUP_ZIP_BYTES));
                limited
                    .read_to_end(&mut out)
                    .map_err(|e| format!("Error descomprimiendo {clean_name}: {e}"))?;
                if out.len() > MAX_BACKUP_ZIP_BYTES {
                    return Err(format!("Entrada {clean_name} supera el límite de 64 MiB"));
                }
                out
            }
            other => {
                return Err(format!(
                    "Método de compresión ZIP no soportado ({other}) en {clean_name}"
                ));
            }
        };

        if crc32_ieee(&content) != expected_crc {
            return Err(format!("Error de verificación CRC-32 en {clean_name}"));
        }

        files.insert(clean_name, content);
    }

    Ok(files)
}

fn is_path_compatible_with_host(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }
    #[cfg(windows)]
    {
        !(trimmed.starts_with('/') && !trimmed.starts_with("//"))
    }
    #[cfg(not(windows))]
    {
        let is_drive_letter =
            trimmed.len() >= 2 && trimmed.as_bytes()[0].is_ascii_alphabetic() && trimmed.as_bytes()[1] == b':';
        let is_unc = trimmed.starts_with("\\\\");
        !(is_drive_letter || is_unc)
    }
}

fn atomic_write_file(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp.bak");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn core_err(err: mule_core::CoreError) -> JsonRpcError {
    crate::error::DaemonError::Core(err).into()
}

/// `settings.exportBackup` — genera una copia de seguridad portátil en formato `.zip`
/// con identidad (`cryptkey.dat`, `userhash.dat`), nodos Kad, servidores, créditos,
/// amigos, categorías, indexadores, biblioteca multimedia y configuración.
pub async fn handle_settings_export_backup(state: &AppState) -> Result<Value, JsonRpcError> {
    let manifest = {
        let dls = state.downloads.lock().await;
        let db = dls.db_ref();

        let settings = dls.list_settings()?;
        let peer_credits = db.list_peer_credits(MAX_BACKUP_CREDITS).map_err(core_err)?;
        let friends = db.list_friends().map_err(core_err)?;
        let categories = db.get_categories().map_err(core_err)?;
        let servers = db.list_servers().map_err(core_err)?;
        let kad_contacts = db
            .list_kad_contacts(MAX_BACKUP_KAD_CONTACTS)
            .map_err(core_err)?;
        let external_indexers = db.list_external_indexers().map_err(core_err)?;
        let media_items = db.list_media_items(None, None).map_err(core_err)?;

        let mut episodes = Vec::new();
        for item in &media_items {
            if item.media_type == "tv" {
                episodes.extend(db.get_episodes_for_media(&item.id).map_err(core_err)?);
            }
        }

        let created_at_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        BackupManifest {
            version: env!("CARGO_PKG_VERSION").to_string(),
            created_at_unix_ms,
            os: std::env::consts::OS.to_string(),
            settings,
            peer_credits,
            friends,
            categories,
            servers,
            kad_contacts,
            external_indexers,
            media_items,
            episodes,
        }
    };

    let data_dir = state.config.data_dir.clone();
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| JsonRpcError::internal(format!("Error serializando manifiesto: {e}")))?;

    let cryptkey_name = mule_crypto::cryptkey::CRYPTKEY_FILENAME;
    let cryptkey_bytes = tokio::fs::read(data_dir.join(cryptkey_name)).await.ok();
    let userhash_bytes = tokio::fs::read(data_dir.join(USERHASH_DAT_FILENAME))
        .await
        .ok();
    let nodes_bytes = tokio::fs::read(data_dir.join(NODES_DAT_FILENAME))
        .await
        .ok();
    let server_met_bytes = tokio::fs::read(data_dir.join(SERVER_MET_FILENAME))
        .await
        .ok();

    let has_cryptkey = cryptkey_bytes.is_some();
    let has_userhash = userhash_bytes.is_some();
    let has_nodes_dat = nodes_bytes.is_some();

    let mut entries: Vec<(&str, &[u8])> = vec![(BACKUP_MANIFEST_FILENAME, &manifest_bytes)];
    if let Some(ref b) = cryptkey_bytes {
        entries.push((cryptkey_name, b));
    }
    if let Some(ref b) = userhash_bytes {
        entries.push((USERHASH_DAT_FILENAME, b));
    }
    if let Some(ref b) = nodes_bytes {
        entries.push((NODES_DAT_FILENAME, b));
    }
    if let Some(ref b) = server_met_bytes {
        entries.push((SERVER_MET_FILENAME, b));
    }

    let zip_bytes = build_zip_archive(&entries).map_err(|e| JsonRpcError::internal(&e))?;
    let size_bytes = zip_bytes.len();
    let zip_base64 = BASE64.encode(&zip_bytes);

    let now = time::OffsetDateTime::now_utc();
    let file_name = format!(
        "pulse-core-backup-{:04}-{:02}-{:02}.zip",
        now.year(),
        now.month() as u8,
        now.day()
    );

    Ok(json!({
        "fileName": file_name,
        "zipBase64": zip_base64,
        "sizeBytes": size_bytes,
        "summary": {
            "hasCryptkey": has_cryptkey,
            "hasUserhash": has_userhash,
            "hasNodesDat": has_nodes_dat,
            "settingsCount": manifest.settings.len(),
            "creditsCount": manifest.peer_credits.len(),
            "serversCount": manifest.servers.len(),
            "kadContactsCount": manifest.kad_contacts.len(),
            "categoriesCount": manifest.categories.len(),
            "friendsCount": manifest.friends.len(),
            "indexersCount": manifest.external_indexers.len(),
            "mediaCount": manifest.media_items.len(),
        }
    }))
}

/// `settings.restoreBackup` — restaura una copia de seguridad `.zip` recibida en Base64,
/// validando la identidad RSA (`cryptkey.dat`), recargando el servicio de identidad y
/// fusionando créditos, ajustes, servidores, amigos, categorías, Kad e indexadores.
pub async fn handle_settings_restore_backup(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Faltan parámetros"))?;
    let zip_b64 = params
        .get("zipBase64")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| JsonRpcError::invalid_params("Falta el parámetro 'zipBase64'"))?;

    let zip_bytes = BASE64
        .decode(zip_b64.as_bytes())
        .map_err(|e| JsonRpcError::invalid_params(&format!("Base64 inválido: {e}")))?;

    let files = parse_zip_archive(&zip_bytes).map_err(|e| JsonRpcError::business(&e))?;

    let cryptkey_name = mule_crypto::cryptkey::CRYPTKEY_FILENAME;
    if !files.contains_key(BACKUP_MANIFEST_FILENAME) && !files.contains_key(cryptkey_name) {
        return Err(JsonRpcError::business(
            "El archivo .zip no contiene una copia de seguridad válida de Pulse Core",
        ));
    }

    let data_dir = &state.config.data_dir;
    let mut restored_identity = false;

    if let Some(raw_key) = files.get(cryptkey_name) {
        let key = mule_crypto::cryptkey::parse_cryptkey_bytes(raw_key)
            .map_err(|e| JsonRpcError::business(format!("{cryptkey_name} inválido: {e}")))?;
        mule_crypto::cryptkey::save_cryptkey(data_dir, &key)
            .map_err(|e| JsonRpcError::business(format!("Error guardando {cryptkey_name}: {e}")))?;
        restored_identity = true;
    }

    if let Some(raw_uh) = files.get(USERHASH_DAT_FILENAME) {
        if raw_uh.len() != USERHASH_LEN {
            return Err(JsonRpcError::business(format!(
                "{USERHASH_DAT_FILENAME} inválido (se esperaban {USERHASH_LEN} bytes)"
            )));
        }
        atomic_write_file(&data_dir.join(USERHASH_DAT_FILENAME), raw_uh)
            .map_err(|e| JsonRpcError::business(format!("Error escribiendo {USERHASH_DAT_FILENAME}: {e}")))?;
        restored_identity = true;
    }

    if restored_identity {
        let new_id = crate::services::IdentityService::load_or_generate(data_dir);
        state.propagate_identity(new_id, "BackupRestore").await;
    }

    let mut nodes_dat_restored = false;
    if let Some(raw_nodes) = files.get(NODES_DAT_FILENAME) {
        let nodes_path = data_dir.join(NODES_DAT_FILENAME);
        atomic_write_file(&nodes_path, raw_nodes)
            .map_err(|e| JsonRpcError::business(format!("Error guardando {NODES_DAT_FILENAME}: {e}")))?;
        state.kad.load_nodes_dat_candidates(&nodes_path).await;
        nodes_dat_restored = true;
    }

    if let Some(raw_server_met) = files.get(SERVER_MET_FILENAME) {
        atomic_write_file(&data_dir.join(SERVER_MET_FILENAME), raw_server_met)
            .map_err(|e| JsonRpcError::business(format!("Error guardando {SERVER_MET_FILENAME}: {e}")))?;
    }

    let mut settings_restored = 0usize;
    let mut settings_skipped_paths = 0usize;
    let mut credits_restored = 0usize;
    let mut friends_restored = 0usize;
    let mut categories_restored = 0usize;
    let mut servers_restored = 0usize;
    let mut kad_contacts_restored = 0usize;
    let mut indexers_restored = 0usize;
    let mut media_restored = 0usize;
    let mut changed_keys: Vec<String> = Vec::new();
    let mut restored_friends_hashes: Vec<String> = Vec::new();

    if let Some(raw_manifest) = files.get(BACKUP_MANIFEST_FILENAME) {
        let manifest: BackupManifest = serde_json::from_slice(raw_manifest)
            .map_err(|e| JsonRpcError::business(format!("Manifiesto de copia inválido: {e}")))?;

        let mut dls = state.downloads.lock().await;

        for (k, v) in &manifest.settings {
            if LOCAL_PATH_SETTING_KEYS.contains(&k.as_str()) && !is_path_compatible_with_host(v) {
                settings_skipped_paths += 1;
                continue;
            }
            dls.set_setting(k, v)?;
            if k == SETTING_WEBUI_PASSWORD_HASH {
                state.set_webui_password_hash(Some(v.clone()));
            } else if k == "lifetime_downloaded_bytes" {
                if let Ok(bytes) = v.parse::<u64>() {
                    state
                        .lifetime_downloaded_bytes
                        .store(bytes, std::sync::atomic::Ordering::SeqCst);
                }
            } else if k == "lifetime_uploaded_bytes" {
                if let Ok(bytes) = v.parse::<u64>() {
                    state
                        .lifetime_uploaded_bytes
                        .store(bytes, std::sync::atomic::Ordering::SeqCst);
                }
            }
            changed_keys.push(k.clone());
            settings_restored += 1;
        }

        for credit in &manifest.peer_credits {
            dls.db_mut().upsert_peer_credit(credit).map_err(core_err)?;
            credits_restored += 1;
        }

        for friend in &manifest.friends {
            dls.add_friend(&friend.user_hash, &friend.name)?;
            restored_friends_hashes.push(format!("uh:{}", friend.user_hash));
            friends_restored += 1;
        }

        for cat in &manifest.categories {
            if dls.get_category(&cat.id)?.is_some() {
                dls.db_mut().update_category(cat).map_err(core_err)?;
            } else {
                dls.db_mut().insert_category(cat).map_err(core_err)?;
            }
            categories_restored += 1;
        }

        if !manifest.servers.is_empty() {
            servers_restored = dls
                .db_mut()
                .upsert_servers(&manifest.servers)
                .map_err(core_err)?;
        }

        if !manifest.kad_contacts.is_empty() {
            kad_contacts_restored = dls
                .db_mut()
                .batch_upsert_kad_contacts(&manifest.kad_contacts)
                .map_err(core_err)?;
        }

        for idx in &manifest.external_indexers {
            dls.db_ref()
                .insert_external_indexer(idx)
                .map_err(core_err)?;
            indexers_restored += 1;
        }

        for item in &manifest.media_items {
            dls.db_ref().insert_media_item(item).map_err(core_err)?;
            media_restored += 1;
        }

        if !manifest.episodes.is_empty() {
            dls.insert_episodes_batch(&manifest.episodes)
                .map_err(core_err)?;
        }
    }

    if !restored_friends_hashes.is_empty() {
        let upload_mgr = state.inbound_listener.upload_manager();
        for uh in restored_friends_hashes {
            upload_mgr.set_friend(&uh, true);
        }
    }

    if kad_contacts_restored > 0 {
        state.kad.init_from_db().await;
    }

    state.emit_event(|sequence| crate::services::DaemonEvent::SettingsChanged {
        sequence,
        keys: if changed_keys.is_empty() {
            vec!["backupRestored".to_string()]
        } else {
            changed_keys
        },
    });

    tracing::info!(
        "[BACKUP] Copia restaurada (identidad={restored_identity}, ajustes={settings_restored}, créditos={credits_restored}, servidores={servers_restored}, kad={kad_contacts_restored})"
    );

    Ok(json!({
        "restored": true,
        "identityRestored": restored_identity,
        "nodesDatRestored": nodes_dat_restored,
        "settingsRestored": settings_restored,
        "settingsSkippedIncompatiblePaths": settings_skipped_paths,
        "creditsRestored": credits_restored,
        "friendsRestored": friends_restored,
        "categoriesRestored": categories_restored,
        "serversRestored": servers_restored,
        "kadContactsRestored": kad_contacts_restored,
        "indexersRestored": indexers_restored,
        "mediaRestored": media_restored,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zip_roundtrip_and_manifest() {
        let manifest_json = br#"{"version":"0.3.0-beta.1","settings":[]}"#;
        let fake_key = b"RSA-KEY-DATA-EXAMPLE-0123456789ABCDEF";
        let zip = build_zip_archive(&[
            (BACKUP_MANIFEST_FILENAME, manifest_json),
            ("cryptkey.dat", fake_key),
        ])
        .unwrap();

        let parsed = parse_zip_archive(&zip).unwrap();
        assert_eq!(
            parsed.get(BACKUP_MANIFEST_FILENAME).map(Vec::as_slice),
            Some(manifest_json.as_slice())
        );
        assert_eq!(
            parsed.get("cryptkey.dat").map(Vec::as_slice),
            Some(fake_key.as_slice())
        );
    }

    #[test]
    fn test_zip_rejects_path_traversal() {
        assert_eq!(sanitize_zip_entry_name("../../etc/passwd"), None);
        assert_eq!(sanitize_zip_entry_name("dir/"), None);
        assert_eq!(
            sanitize_zip_entry_name("folder/nodes.dat"),
            Some("nodes.dat".to_string())
        );
    }
}

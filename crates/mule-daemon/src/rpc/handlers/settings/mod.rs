// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::JsonRpcError;
use crate::state::AppState;

mod fs_browser;
mod logs;
mod system_metrics;

pub use fs_browser::{handle_settings_list_dirs, handle_settings_pick_folder};
pub use logs::handle_logs;
pub use system_metrics::{
    get_cached_nas_disk_space, handle_disk_space, handle_nas_info,
    handle_stats_reset_lifetime_totals, handle_status, handle_stop,
};
use system_metrics::NAS_DISK_CACHE;

/// Longitud máxima permitida para la contraseña opcional de la WebUI.
const MAX_WEBUI_PASSWORD_LEN: usize = 128;

/// Claves camelCase reconocidas por `settings.set` — cualquier otra clave es
/// ignorada (se emite una advertencia para facilitar el diagnóstico de erratas).
const KNOWN_SETTINGS_KEYS: &[&str] = &[
    "defaultDownloadDir",
    "historyRetentionDays",
    "maxGlobalDownloadPeers",
    "maxPeersPerDownload",
    "minPeersPerDownload",
    "shareDownloadDir",
    "nickname",
    "obfuscationEnabled",
    "obfuscationRequired",
    "maxConnections",
    "maxConnPerMin",
    "maxUploadQueue",
    "ussEnabled",
    "ed2kPort",
    "kadPort",
    "udpEnabled",
    "upnpEnabled",
    "capacityDownloadKbs",
    "capacityUploadKbs",
    "minUploadKbs",
    "idleUploadCapKbs",
    "streamingUploadCapKbs",
    "smartSlotTargetKbs",
    "quotaEcoUploadKbs",
    "ichEnabled",
    "rememberCancelled",
    "rememberDownloaded",
    "serverReconnect",
    "serverAutoconnect",
    "serverAutoremoveDead",
    "serverAutoupdateList",
    "serverAutoupdatePath",
    "safeConnect",
    "autoPriorityNewDownloads",
    "autoPriorityUploads",
    "swarmHealing",
    "tempDir",
    "identityRotationHours",
    "dailyUploadQuotaGb",
    "paranoidMode",
    "viewSharedFiles",
    "maxActiveDownloads",
    "nasRootDir",
    "nasRequireNfs",
    "moviesDir",
    "seriesDir",
    "sonarrUrl",
    "radarrUrl",
    "jellyfinUrl",
    "jellyfinApiKey",
    "plexUrl",
    "plexToken",
    "kodiUrl",
    "autoGrabberEnabled",
    "organizeEnabled",
    "mediaServerProfile",
    "generateNfoAndArtwork",
    "qualityUpgradeEnabled",
    "qualityMaxResolution",
    "scorerAntiFake",
    "subtitlesAutoDownload",
    "subtitlesLanguage",
    "subtitlesVariant",
    "subtitlesAllowHi",
    "subtitlesMinScore",
    "subtitlesAutoSync",
    "opensubtitlesApiKey",
    "cleanerEnabled",
    "mcpEnabled",
    "mcpToolProfile",
    "ramProfile",
    "logLevel",
    "updateChannel",
    "webuiPassword",
    "legalDisclaimerAccepted",
];

pub async fn handle_settings_get(state: &AppState) -> Result<Value, JsonRpcError> {
    let (
        custom_dir,
        retention_days,
        max_global,
        max_per_file,
        min_per_file,
        max_active_downloads,
        share_dir,
        nickname,
        obf_enabled,
        obf_required,
        max_conn,
        conn_per_min,
        max_queue,
    ) = {
        let dls = state.downloads.lock().await;
        let dir = dls.get_setting("default_download_directory")?;
        let ret = dls
            .get_setting("history_retention_days")?
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(30);
        let max_g = dls
            .get_setting("max_global_download_peers")?
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(state.config.max_global_download_peers);
        let max_f = dls
            .get_setting("max_peers_per_download")?
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(state.config.max_peers_per_download);
        let min_f = dls
            .get_setting("min_peers_per_download")?
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(state.config.min_peers_per_download);
        let max_active_dls = dls
            .get_setting("max_active_downloads")?
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(30);
        let share = dls
            .get_setting("share_download_dir")?
            .map(|v| !matches!(v.as_str(), "0" | "false" | "off"))
            .unwrap_or(true);
        let nickname = dls
            .get_setting("nickname")?
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "eMuleRust".to_string());
        let bool_setting = |key: &str, default: bool| {
            dls.get_setting(key)
                .ok()
                .flatten()
                .map(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "on"))
                .unwrap_or(default)
        };
        let obf_enabled = bool_setting("obfuscation_enabled", state.config.obfuscation_enabled);
        let obf_required = bool_setting("obfuscation_required", state.config.obfuscation_required);
        let max_conn = dls
            .get_setting("max_connections")?
            .and_then(|s| s.parse::<usize>().ok())
            .map(|v| v.clamp(1, mule_network_policy::MAX_PEER_CONNECTIONS_CAP))
            .unwrap_or(state.network_policy.budgets().max_active_peer_connections);
        let conn_per_min = dls
            .get_setting("max_conn_per_min")?
            .and_then(|s| s.parse::<u32>().ok())
            .map(|v| v.clamp(1, 1200))
            .unwrap_or(state.network_policy.budgets().rate_limit_per_min);
        let max_queue = dls
            .get_setting("max_upload_queue")?
            .and_then(|s| s.parse::<usize>().ok())
            .map(|v| v.clamp(10, 5000))
            .unwrap_or(crate::services::upload_queue::MAX_UPLOAD_QUEUE);
        (
            dir,
            ret,
            max_g,
            max_f,
            min_f,
            max_active_dls,
            share,
            nickname,
            obf_enabled,
            obf_required,
            max_conn,
            conn_per_min,
            max_queue,
        )
    };
    let uss_enabled = state.uss.config().enabled;
    let upnp_enabled = state.upnp.snapshot().await.enabled;
    let (cap_dl, cap_ul) = {
        let dls = state.downloads.lock().await;
        (
            dls.get_setting("capacity_download_kbs")?
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0),
            dls.get_setting("capacity_upload_kbs")?
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0),
        )
    };
    let current_dir = custom_dir
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| state.config.default_download_dir.clone());

    let bools = {
        let dls = state.downloads.lock().await;
        let read = |key: &str, default: bool| {
            dls.get_setting(key)
                .ok()
                .flatten()
                .map(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "on"))
                .unwrap_or(default)
        };
        let u64_setting = |key: &str| {
            dls.get_setting(key)
                .ok()
                .flatten()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(0)
        };
        let min_up = dls
            .get_setting("min_upload_kbs")?
            .and_then(|s| s.parse::<u64>().ok())
            // eMule 0.72a `MinUpload` default = 1 Ko/s (clamped >= 1).
            .unwrap_or(1);

        let default_local_media = state.config.default_download_dir.join("media");
        let nas_root_persisted = dls
            .get_setting("nas_root_dir")
            .ok()
            .flatten()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let env_nas = (state.config.nas_root_dir != default_local_media
            && !state.config.nas_root_dir.as_os_str().is_empty())
        .then(|| state.config.nas_root_dir.to_string_lossy().to_string());
        let nas_root_dir_out = if !nas_root_persisted.is_empty() {
            nas_root_persisted
        } else {
            env_nas.unwrap_or_default()
        };
        let nas_root_active = !nas_root_dir_out.is_empty();

        let custom_media_dir = |key: &str, subdir: &str, cfg_dir: &std::path::Path| -> String {
            let persisted = dls
                .get_setting(key)
                .ok()
                .flatten()
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            if !persisted.is_empty() {
                if nas_root_active
                    && state.is_default_local_media_subdir(
                        std::path::Path::new(&persisted),
                        &current_dir,
                        subdir,
                    )
                {
                    return String::new();
                }
                return persisted;
            }
            if !nas_root_active
                && !cfg_dir.as_os_str().is_empty()
                && !state.is_default_local_media_subdir(cfg_dir, &current_dir, subdir)
            {
                return cfg_dir.to_string_lossy().to_string();
            }
            String::new()
        };

    let mut base_settings = json!({
            "minUploadKbs": min_up,
            "smartSlotTargetKbs": u64_setting("smart_slot_target_kbs"),
            "ichEnabled": read("ich_enabled", true),
            "rememberCancelled": read("remember_cancelled", true),
            "rememberDownloaded": read("remember_downloaded", true),
            "serverReconnect": read("server_reconnect", true),
            "serverAutoconnect": read("server_autoconnect", false),
            "serverAutoremoveDead": read("server_autoremove_dead", true),
            "serverAutoupdateList": read("server_autoupdate_list", false),
            "safeConnect": read("safe_connect", false),
            // eMule `DAPPref`/`UAPPref` — por defecto `true` en ambos casos.
            "autoPriorityNewDownloads": read("auto_priority_new_downloads", true),
            "autoPriorityUploads": read("auto_priority_uploads", true),
            // Opciones propias de eMuleRust (sin equivalente en eMule).
            "identityRotationHours": u64_setting("identity_rotation_hours"),
            "dailyUploadQuotaGb": u64_setting("daily_upload_quota_gb"),
            "quotaEcoUploadKbs": u64_setting("quota_eco_upload_kbs"),
            "uploadQuotaUsedBytes": u64_setting("upload_quota_used_bytes"),
            "paranoidMode": read("paranoid_mode", false),
            "swarmHealing": read("swarm_healing", true),
            "serverAutoupdatePath": dls
                .get_setting("server_autoupdate_path")
                .ok()
                .flatten()
                .unwrap_or_default(),
            "tempDir": dls
                .get_setting("temp_dir")
                .ok()
                .flatten()
                .unwrap_or_default(),
            "moviesDir": custom_media_dir("movies_dir", "Movies", &state.config.movies_dir),
            "seriesDir": custom_media_dir("series_dir", "Series", &state.config.series_dir),
            "nasRootDir": nas_root_dir_out,
            "nasRequireNfs": read("nas_require_nfs", false),
            "autoGrabberEnabled": read("auto_grabber_enabled", true),
            "organizeEnabled": read("organize_enabled", true),
            "mediaServerProfile": dls
                .get_setting("media_server_profile")
                .ok()
                .flatten()
                .unwrap_or_else(|| "jellyfin".to_string()),
            "generateNfoAndArtwork": read("generate_nfo_and_artwork", true),
            "qualityUpgradeEnabled": read("quality_upgrade_enabled", true),
            "qualityMaxResolution": dls
                .get_setting("quality_max_resolution")
                .ok()
                .flatten()
                .unwrap_or_else(|| "1080p".to_string()),
            "scorerAntiFake": read("scorer_anti_fake", true),
            "subtitlesAutoDownload": read("subtitles_auto_download", true),
            "subtitlesLanguage": dls
                .get_setting("subtitles_language")
                .ok()
                .flatten()
                .unwrap_or_else(|| "spa".to_string()),
            "cleanerEnabled": read("cleaner_enabled", true),
            "legalDisclaimerAccepted": read("legal_disclaimer_accepted", false),
            "mcpEnabled": read("mcp_enabled", true),
            "mcpToolProfile": dls
                .get_setting("mcp_tool_profile")
                .ok()
                .flatten()
                .unwrap_or_else(|| "safe".to_string()),
            "ramProfile": dls
                .get_setting("ram_profile")
                .ok()
                .flatten()
                .unwrap_or_else(|| "balanced".to_string()),
            "sonarrUrl": dls.get_setting("sonarr_url").ok().flatten().unwrap_or_default(),
            "radarrUrl": dls.get_setting("radarr_url").ok().flatten().unwrap_or_default(),
            "jellyfinUrl": dls.get_setting("jellyfin_url").ok().flatten().unwrap_or_default(),
        });
        if let Some(obj) = base_settings.as_object_mut() {
            obj.insert("jellyfinApiKey".to_string(), json!(dls.get_setting("jellyfin_api_key").ok().flatten().unwrap_or_default()));
            obj.insert("plexUrl".to_string(), json!(dls.get_setting("plex_url").ok().flatten().unwrap_or_default()));
            obj.insert("plexToken".to_string(), json!(dls.get_setting("plex_token").ok().flatten().unwrap_or_default()));
            obj.insert("kodiUrl".to_string(), json!(dls.get_setting("kodi_url").ok().flatten().unwrap_or_default()));
            obj.insert(
                "subtitlesVariant".to_string(),
                json!(dls.get_setting("subtitles_variant").ok().flatten().unwrap_or_else(|| "es-ES".to_string())),
            );
            obj.insert("subtitlesAllowHi".to_string(), json!(read("subtitles_allow_hi", false)));
            obj.insert(
                "subtitlesMinScore".to_string(),
                json!(dls.get_setting("subtitles_min_score").ok().flatten().and_then(|s| s.parse::<u32>().ok()).unwrap_or(40)),
            );
            obj.insert(
                "subtitlesAutoSync".to_string(),
                json!(dls.get_setting("subtitles_auto_sync").ok().flatten().unwrap_or_else(|| "embedded_sub".to_string())),
            );
            obj.insert(
                "opensubtitlesApiKey".to_string(),
                json!(dls.get_setting("opensubtitles_api_key").ok().flatten().unwrap_or_default()),
            );
        }
        base_settings
    };

    let effective_movies_dir = state.effective_movies_dir().await;
    let effective_series_dir = state.effective_series_dir().await;
    let effective_nas_root = state.effective_nas_root_dir().await;
    let fallback_nas_root = effective_nas_root.clone();
    let (nfs_status, discovered_mounts) = tokio::task::spawn_blocking(move || {
        let status =
            crate::services::nfs_mount::inspect_directory_status(&effective_nas_root, true);
        let mounts = crate::services::nfs_mount::discover_system_network_mounts();
        (status, mounts)
    })
    .await
    .unwrap_or_else(|_| {
        (
            crate::services::nfs_mount::inspect_directory_status(&fallback_nas_root, false),
            Vec::new(),
        )
    });

    let portable_default =
        crate::workdir::WorkDirectoryResolver::resolve(None, None).default_download_dir;
    let hardlink_capable = are_paths_on_same_device(&current_dir, &fallback_nas_root);
    let active_update_channel = state.updater.get_channel().await;
    let mut result = json!({
        "updateChannel": active_update_channel,
        "defaultDownloadDir": current_dir.to_string_lossy(),
        "portableDownloadDir": portable_default.to_string_lossy(),
        "workdirPath": state.config.data_dir.to_string_lossy(),
        "runningInDocker": crate::services::updater::is_running_in_docker(),
        "hardlinkCapable": hardlink_capable,
        "isCustom": custom_dir.is_some(),
        "effectiveMoviesDir": effective_movies_dir.to_string_lossy(),
        "effectiveSeriesDir": effective_series_dir.to_string_lossy(),
        "nfsStatus": nfs_status,
        "discoveredMounts": discovered_mounts,
        "authToken": state.config.auth_token.clone(),
        "webuiPasswordEnabled": state.has_webui_password(),
        "historyRetentionDays": retention_days,
        "maxGlobalDownloadPeers": max_global,
        "maxPeersPerDownload": max_per_file,
        "minPeersPerDownload": min_per_file,
        "maxActiveDownloads": max_active_downloads,
        "shareDownloadDir": share_dir,
        "viewSharedFiles": state
            .downloads
            .lock()
            .await
            .get_setting(crate::services::inbound_listener::share_browser::SETTING_VIEW_SHARED_FILES)
            .ok()
            .flatten()
            .unwrap_or_else(|| "everybody".to_string()),
        "nickname": nickname,
        "obfuscationEnabled": obf_enabled,
        "obfuscationRequired": obf_required,
        "maxConnections": max_conn,
        "maxConnPerMin": conn_per_min,
        "maxUploadQueue": max_queue,
        "ussEnabled": uss_enabled,
        "idleUploadCapKbs": state.uss.config().idle_upload_cap_bps / 1024,
        "streamingUploadCapKbs": state.uss.config().streaming_upload_cap_bps / 1024,
        "ed2kPort": state.ed2k_port(),
        "kadPort": state.kad_port(),
        "udpEnabled": state.udp_enabled(),
        "upnpEnabled": upnp_enabled,
        "capacityDownloadKbs": cap_dl,
        "capacityUploadKbs": cap_ul,
        // Solo en tiempo de ejecución — refleja la especificación EnvFilter efectiva, no
        // un ajuste persistido.
        "logLevel": state.current_log_level(),
    });
    if let (Some(out), Some(extra)) = (result.as_object_mut(), bools.as_object()) {
        out.extend(extra.clone());
    }
    Ok(result)
}

pub async fn handle_settings_set(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let mut updated_dir = None;
    let mut updated_retention = None;
    let mut updated_max_global = None;
    let mut updated_max_per_file = None;
    let mut updated_min_per_file = None;
    let mut updated_max_active = None;
    let mut updated_share_dir = None;
    let mut keys: Vec<&str> = Vec::new();

    if let Some(dir_str) = params["defaultDownloadDir"].as_str() {
        let dir = std::path::PathBuf::from(dir_str);
        // Validar que el directorio sea creable y editable
        std::fs::create_dir_all(&dir)
            .map_err(|e| JsonRpcError::business(format!("Cannot create directory: {e}")))?;

        let probe = dir.join(format!(".mule_write_probe_{}", Uuid::new_v4()));
        match std::fs::write(&probe, b"probe") {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
            }
            Err(e) => {
                return Err(JsonRpcError::business(format!(
                    "Directory is not writable: {e}"
                )));
            }
        }

        state
            .downloads
            .lock()
            .await
            .set_setting("default_download_directory", dir_str)?;
        updated_dir = Some(dir_str.to_string());
    }

    if let Some(ret_days_val) = params.get("historyRetentionDays") {
        let ret_days = ret_days_val.as_u64().ok_or_else(|| {
            JsonRpcError::invalid_params("Invalid 'historyRetentionDays' parameter")
        })? as u32;
        let mut dls = state.downloads.lock().await;
        dls.set_setting("history_retention_days", &ret_days.to_string())?;
        dls.apply_retention_policy(ret_days)?;
        updated_retention = Some(ret_days);
    }

    if let Some(val) = params.get("maxGlobalDownloadPeers") {
        let max_g = val.as_u64().ok_or_else(|| {
            JsonRpcError::invalid_params("Invalid 'maxGlobalDownloadPeers' parameter")
        })? as usize;
        let mut dls = state.downloads.lock().await;
        dls.set_setting("max_global_download_peers", &max_g.to_string())?;
        updated_max_global = Some(max_g);
    }

    if let Some(val) = params.get("maxPeersPerDownload") {
        let max_f = val.as_u64().ok_or_else(|| {
            JsonRpcError::invalid_params("Invalid 'maxPeersPerDownload' parameter")
        })? as usize;
        let mut dls = state.downloads.lock().await;
        dls.set_setting("max_peers_per_download", &max_f.to_string())?;
        updated_max_per_file = Some(max_f);
    }

    if let Some(val) = params.get("minPeersPerDownload") {
        let min_f = val.as_u64().ok_or_else(|| {
            JsonRpcError::invalid_params("Invalid 'minPeersPerDownload' parameter")
        })? as usize;
        let mut dls = state.downloads.lock().await;
        dls.set_setting("min_peers_per_download", &min_f.to_string())?;
        updated_min_per_file = Some(min_f);
    }

    if let Some(val) = params.get("maxActiveDownloads") {
        let max_a = val.as_u64().ok_or_else(|| {
            JsonRpcError::invalid_params("Invalid 'maxActiveDownloads' parameter")
        })? as usize;
        let clamped = max_a.clamp(1, 100);
        let mut dls = state.downloads.lock().await;
        dls.set_setting("max_active_downloads", &clamped.to_string())?;
        updated_max_active = Some(clamped);
    }

    if let Some(val) = params.get("shareDownloadDir") {
        let share = val
            .as_bool()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'shareDownloadDir' parameter"))?;
        state
            .downloads
            .lock()
            .await
            .set_setting("share_download_dir", if share { "1" } else { "0" })?;
        updated_share_dir = Some(share);
    }

    if let Some(val) = params.get("nickname") {
        let nick = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'nickname' parameter"))?
            .trim();
        if nick.is_empty() || nick.len() > 64 {
            return Err(JsonRpcError::invalid_params(
                "'nickname' must be 1..64 characters",
            ));
        }
        state.downloads.lock().await.set_setting("nickname", nick)?;
        keys.push("nickname");
    }

    let mut obf_changed = false;
    if let Some(val) = params.get("obfuscationEnabled") {
        let v = val
            .as_bool()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'obfuscationEnabled'"))?;
        state
            .downloads
            .lock()
            .await
            .set_setting("obfuscation_enabled", if v { "1" } else { "0" })?;
        obf_changed = true;
        keys.push("obfuscationEnabled");
    }
    if let Some(val) = params.get("obfuscationRequired") {
        let v = val
            .as_bool()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'obfuscationRequired'"))?;
        state
            .downloads
            .lock()
            .await
            .set_setting("obfuscation_required", if v { "1" } else { "0" })?;
        obf_changed = true;
        keys.push("obfuscationRequired");
    }
    if obf_changed {
        let (enabled, required) = state.effective_obfuscation().await;
        state.inbound_listener.set_obfuscation(enabled, required);
        // Propagar la preferencia a la sesión del servidor (eMule
        // IsServerCryptLayerConnection) — de lo contrario el ajuste solo actúa sobre
        // los pares entrantes.
        state
            .server_session
            .set_obfuscation_preference(enabled || required, state.config.obfuscation_padding_max);
        // Actualizar el byte de opciones de conexión buddy-relay anunciado en Kad.
        let connect_opts: u8 = if enabled { 0x03 } else { 0 } | if required { 0x04 } else { 0 };
        state
            .kad
            .udp()
            .set_buddy_tcp_info(state.ed2k_port(), connect_opts);
    }

    let mut budgets = state.network_policy.budgets();
    let mut budgets_changed = false;
    if let Some(val) = params.get("maxConnections") {
        // Mismo límite que en el arranque (`state.rs`): el presupuesto de red está
        // limitado a [1, 65000] — eMule no limita MaxConnections bajo
        // NT+, mantenemos una salvaguarda contra errores tipográficos. Se restringe antes
        // de persistir para que el valor almacenado permanezca idéntico al
        // realmente aplicado — de lo contrario `set_budgets` rechazaba tras
        // la escritura en la base de datos.
        let v = val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'maxConnections'"))?
            .clamp(1, mule_network_policy::MAX_PEER_CONNECTIONS_CAP as u64)
            as usize;
        state
            .downloads
            .lock()
            .await
            .set_setting("max_connections", &v.to_string())?;
        budgets.max_active_peer_connections = v;
        budgets_changed = true;
        keys.push("maxConnections");
    }
    if let Some(val) = params.get("maxConnPerMin") {
        let v = val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'maxConnPerMin'"))?
            .clamp(1, 1200) as u32;
        state
            .downloads
            .lock()
            .await
            .set_setting("max_conn_per_min", &v.to_string())?;
        budgets.rate_limit_per_min = v;
        budgets_changed = true;
        keys.push("maxConnPerMin");
    }
    if budgets_changed {
        state
            .network_policy
            .set_budgets(budgets)
            .map_err(|e| JsonRpcError::business(format!("{e}")))?;
    }

    if let Some(val) = params.get("maxUploadQueue") {
        // Clamp identique à `UploadManager::set_max_queue` ([10, 5000],
        // eMule QueueSize) avant persistance : stockée = appliquée.
        let v = val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'maxUploadQueue'"))?
            .clamp(10, 5000) as usize;
        state
            .downloads
            .lock()
            .await
            .set_setting("max_upload_queue", &v.to_string())?;
        state.inbound_listener.upload_manager().set_max_queue(v);
        keys.push("maxUploadQueue");
    }

    if let Some(val) = params.get("ussEnabled") {
        let v = val
            .as_bool()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'ussEnabled'"))?;
        state.uss.update_config(|c| c.enabled = v).await;
        keys.push("ussEnabled");
    }

    // --- Ports & UDP (eMule Preferences → Connexion) ---
    let mut ports_changed = false;
    if let Some(val) = params.get("ed2kPort") {
        let v = val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'ed2kPort'"))?;
        if v == 0 || v > 65535 {
            return Err(JsonRpcError::invalid_params("'ed2kPort' must be 1..65535"));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("ed2k_port", &v.to_string())?;
        ports_changed = true;
        keys.push("ed2kPort");
    }
    if let Some(val) = params.get("kadPort") {
        let v = val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'kadPort'"))?;
        if v == 0 || v > 65535 {
            return Err(JsonRpcError::invalid_params("'kadPort' must be 1..65535"));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("kad_port", &v.to_string())?;
        ports_changed = true;
        keys.push("kadPort");
    }
    if let Some(val) = params.get("udpEnabled") {
        let v = val
            .as_bool()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'udpEnabled'"))?;
        state
            .downloads
            .lock()
            .await
            .set_setting("udp_enabled", if v { "1" } else { "0" })?;
        state.set_udp_enabled(v);
        apply_udp_state(state, v).await;
        keys.push("udpEnabled");
    }
    if let Some(val) = params.get("upnpEnabled") {
        let v = val
            .as_bool()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'upnpEnabled'"))?;
        state
            .downloads
            .lock()
            .await
            .set_setting("upnp_enabled", if v { "1" } else { "0" })?;
        state.upnp.set_enabled(v).await;
        let upnp = state.upnp.clone();
        let (tcp, udp) = (state.ed2k_port(), state.kad_port());
        tokio::spawn(async move {
            if v {
                let _ = upnp.map_ports(tcp, udp).await;
            } else {
                let _ = upnp.unmap_ports().await;
            }
        });
        keys.push("upnpEnabled");
    }
    for key in ["capacityDownloadKbs", "capacityUploadKbs"] {
        if let Some(val) = params.get(key) {
            let v = val
                .as_u64()
                .ok_or_else(|| JsonRpcError::invalid_params("Invalid capacity value"))?;
            let snake = if key == "capacityDownloadKbs" {
                "capacity_download_kbs"
            } else {
                "capacity_upload_kbs"
            };
            state
                .downloads
                .lock()
                .await
                .set_setting(snake, &v.to_string())?;
            keys.push(key);
        }
    }
    if let Some(v) = params.get("capacityUploadKbs").and_then(|v| v.as_u64()) {
        // Capacidad de subida = velocidad de línea de referencia utilizada por USS como su
        // límite manual de base (eMule la utiliza para el cálculo de ranuras).
        state.uss.note_manual_limit(v.saturating_mul(1024));
    }
    if params.get("capacityDownloadKbs").is_some() || params.get("capacityUploadKbs").is_some() {
        // eMule limita los límites efectivos a la capacidad declarada:
        // una reducción de capacidad reduce el límite en consecuencia.
        crate::rpc::handlers::network::apply_capacity_clamp(state).await?;
    }
    if let Some(val) = params.get("minUploadKbs") {
        let v = val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'minUploadKbs'"))?;
        if v > 100_000 {
            return Err(JsonRpcError::invalid_params(
                "'minUploadKbs' must be 0..100000",
            ));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("min_upload_kbs", &v.to_string())?;
        state
            .inbound_listener
            .upload_manager()
            .set_min_upload_rate(v.saturating_mul(1024));
        keys.push("minUploadKbs");
    }
    if let Some(val) = params.get("idleUploadCapKbs") {
        let v = val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'idleUploadCapKbs'"))?;
        if v > 1_000_000 {
            return Err(JsonRpcError::invalid_params(
                "'idleUploadCapKbs' must be 0..1000000",
            ));
        }
        state
            .uss
            .update_config(|c| c.idle_upload_cap_bps = v.saturating_mul(1024))
            .await;
        keys.push("idleUploadCapKbs");
    }
    if let Some(val) = params.get("streamingUploadCapKbs") {
        let v = val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'streamingUploadCapKbs'"))?;
        if v > 1_000_000 {
            return Err(JsonRpcError::invalid_params(
                "'streamingUploadCapKbs' must be 0..1000000",
            ));
        }
        state
            .uss
            .update_config(|c| c.streaming_upload_cap_bps = v.saturating_mul(1024))
            .await;
        keys.push("streamingUploadCapKbs");
    }
    if let Some(val) = params.get("quotaEcoUploadKbs") {
        let v = val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'quotaEcoUploadKbs'"))?;
        if v > 1_000_000 {
            return Err(JsonRpcError::invalid_params(
                "'quotaEcoUploadKbs' must be 0..1000000",
            ));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("quota_eco_upload_kbs", &v.to_string())?;
        keys.push("quotaEcoUploadKbs");
    }
    if let Some(val) = params.get("smartSlotTargetKbs") {
        let v = val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'smartSlotTargetKbs'"))?;
        if v > 100_000 {
            return Err(JsonRpcError::invalid_params(
                "'smartSlotTargetKbs' must be 0..100000",
            ));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("smart_slot_target_kbs", &v.to_string())?;
        state
            .inbound_listener
            .upload_manager()
            .set_target_slot_speed(v.saturating_mul(1024));
        keys.push("smartSlotTargetKbs");
    }
    for (camel, snake) in [
        ("ichEnabled", "ich_enabled"),
        ("rememberCancelled", "remember_cancelled"),
        ("rememberDownloaded", "remember_downloaded"),
        ("serverReconnect", "server_reconnect"),
        ("serverAutoconnect", "server_autoconnect"),
        ("serverAutoremoveDead", "server_autoremove_dead"),
        ("serverAutoupdateList", "server_autoupdate_list"),
        ("safeConnect", "safe_connect"),
        ("autoPriorityNewDownloads", "auto_priority_new_downloads"),
        ("autoPriorityUploads", "auto_priority_uploads"),
        ("swarmHealing", "swarm_healing"),
    ] {
        if let Some(val) = params.get(camel) {
            let v = val
                .as_bool()
                .ok_or_else(|| JsonRpcError::invalid_params(&format!("Invalid '{camel}'")))?;
            state
                .downloads
                .lock()
                .await
                .set_setting(snake, if v { "1" } else { "0" })?;
            keys.push(camel);
        }
    }
    if let Some(val) = params.get("viewSharedFiles") {
        // eMule `thePrefs.m_iSeeShares` — `vsfaEverybody`/`vsfaFriends`/`vsfaNobody`.
        let v = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'viewSharedFiles' parameter"))?;
        if !matches!(v, "everybody" | "friends" | "nobody") {
            return Err(JsonRpcError::invalid_params(
                "'viewSharedFiles' must be \"everybody\", \"friends\" or \"nobody\"",
            ));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting(crate::services::inbound_listener::share_browser::SETTING_VIEW_SHARED_FILES, v)?;
        keys.push("viewSharedFiles");
    }
    if let Some(val) = params.get("serverAutoupdatePath") {
        let p = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'serverAutoupdatePath' parameter"))?;
        if p.len() > 512 {
            return Err(JsonRpcError::invalid_params(
                "'serverAutoupdatePath' is too long",
            ));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("server_autoupdate_path", p)?;
        keys.push("serverAutoupdatePath");
    }
    if let Some(val) = params.get("tempDir") {
        let dir_str = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'tempDir'"))?
            .trim();
        if !dir_str.is_empty() {
            let dir = std::path::Path::new(dir_str);
            std::fs::create_dir_all(dir)
                .map_err(|e| JsonRpcError::business(format!("Cannot create temp dir: {e}")))?;
            let probe = dir.join(format!(".mule_write_probe_{}", Uuid::new_v4()));
            match std::fs::write(&probe, b"probe") {
                Ok(_) => {
                    let _ = std::fs::remove_file(&probe);
                }
                Err(e) => {
                    return Err(JsonRpcError::business(format!(
                        "Temp dir is not writable: {e}"
                    )));
                }
            }
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("temp_dir", dir_str)?;
        keys.push("tempDir");
    }
    // --- Options propres à eMuleRust ---
    if let Some(val) = params.get("identityRotationHours") {
        let v = val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'identityRotationHours'"))?;
        if v > 8760 {
            return Err(JsonRpcError::invalid_params(
                "'identityRotationHours' must be 0..8760",
            ));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("identity_rotation_hours", &v.to_string())?;
        keys.push("identityRotationHours");
    }
    if let Some(val) = params.get("dailyUploadQuotaGb") {
        let v = val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'dailyUploadQuotaGb'"))?;
        if v > 1_048_576 {
            return Err(JsonRpcError::invalid_params(
                "'dailyUploadQuotaGb' must be 0..1048576",
            ));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("daily_upload_quota_gb", &v.to_string())?;
        keys.push("dailyUploadQuotaGb");
    }
    if let Some(val) = params.get("paranoidMode") {
        let v = val
            .as_bool()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'paranoidMode'"))?;
        state
            .downloads
            .lock()
            .await
            .set_setting("paranoid_mode", if v { "1" } else { "0" })?;
        state.inbound_listener.set_paranoid_mode(v);
        keys.push("paranoidMode");
    }
    if let Some(val) = params.get("nasRequireNfs") {
        let v = val
            .as_bool()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'nasRequireNfs'"))?;
        state
            .downloads
            .lock()
            .await
            .set_setting("nas_require_nfs", if v { "1" } else { "0" })?;
        keys.push("nasRequireNfs");
    }

    let require_net = state.nas_require_network_mount().await;
    let effective_dl_dir = state.effective_download_dir().await;
    let old_effective_movies = state.effective_movies_dir().await;
    let old_effective_series = state.effective_series_dir().await;
    let mut media_roots_changed = false;

    if let Some(val) = params.get("nasRootDir") {
        let dir_str = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'nasRootDir'"))?
            .trim();
        if !dir_str.is_empty() {
            let nas_path = std::path::PathBuf::from(dir_str);
            let check_path = nas_path.clone();
            let check_res = tokio::task::spawn_blocking(move || {
                crate::services::nfs_mount::verify_destination_mount_ready(&check_path, require_net)
            })
            .await
            .map_err(|e| JsonRpcError::internal(format!("Mount check failed: {e}")))?;
            if let Err(e) = check_res {
                tracing::warn!(
                    "[SETTINGS] Ruta NAS '{}' no verificada como lista ({e}); se guarda la ruta pero no se crean subcarpetas en disco local",
                    dir_str
                );
            }
        }
        {
            let mut dls = state.downloads.lock().await;
            dls.set_setting("nas_root_dir", dir_str)?;
            if !dir_str.is_empty() {
                for (key, subdir) in [("movies_dir", "Movies"), ("series_dir", "Series")] {
                    if let Ok(Some(cur)) = dls.get_setting(key) {
                        if state.is_default_local_media_subdir(
                            std::path::Path::new(cur.trim()),
                            &effective_dl_dir,
                            subdir,
                        ) {
                            let _ = dls.set_setting(key, "");
                        }
                    }
                }
            }
        }
        if let Ok(mut cache) = NAS_DISK_CACHE.write() {
            *cache = None;
        }
        keys.push("nasRootDir");
        media_roots_changed = true;
    }

    let nas_root_active = {
        let dls = state.downloads.lock().await;
        let db_nas = dls
            .get_setting("nas_root_dir")
            .ok()
            .flatten()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let default_local_media = state.config.default_download_dir.join("media");
        db_nas
            || (state.config.nas_root_dir != default_local_media
                && !state.config.nas_root_dir.as_os_str().is_empty())
    };

    for (camel, snake, subdir) in [
        ("moviesDir", "movies_dir", "Movies"),
        ("seriesDir", "series_dir", "Series"),
    ] {
        if let Some(val) = params.get(camel) {
            let mut dir_str = val
                .as_str()
                .ok_or_else(|| JsonRpcError::invalid_params(&format!("Invalid '{camel}'")))?
                .trim()
                .to_string();
            if nas_root_active
                && !dir_str.is_empty()
                && state.is_default_local_media_subdir(
                    std::path::Path::new(&dir_str),
                    &effective_dl_dir,
                    subdir,
                )
            {
                dir_str.clear();
            }
            if !dir_str.is_empty() {
                let target_path = std::path::PathBuf::from(&dir_str);
                let res = tokio::task::spawn_blocking(move || {
                    crate::services::nfs_mount::verify_destination_mount_ready(
                        &target_path,
                        require_net,
                    )?;
                    std::fs::create_dir_all(&target_path)
                })
                .await;
                if let Ok(Err(e)) = res {
                    tracing::warn!(
                        "[SETTINGS] Omitiendo create_dir_all en '{}' por protección NFS/red: {}",
                        dir_str,
                        e
                    );
                }
            }
            state
                .downloads
                .lock()
                .await
                .set_setting(snake, &dir_str)?;
            keys.push(camel);
            media_roots_changed = true;
        }
    }

    if media_roots_changed {
        let new_movies_str = state
            .effective_movies_dir()
            .await
            .to_string_lossy()
            .into_owned();
        let new_series_str = state
            .effective_series_dir()
            .await
            .to_string_lossy()
            .into_owned();
        let now_ts = crate::services::downloads::now_ms();

        let dls = state.downloads.lock().await;
        if let Ok(items) = dls.db().list_media_items(None, None) {
            for mut item in items {
                let (target_root, old_eff, subdir) = if item.media_type == "tv" {
                    (&new_series_str, &old_effective_series, "Series")
                } else {
                    (&new_movies_str, &old_effective_movies, "Movies")
                };
                let stored_path = std::path::Path::new(item.root_folder.trim());
                let is_incomplete = item.file_path.as_deref().unwrap_or("").trim().is_empty();
                let is_default_root = item.root_folder.trim().is_empty()
                    || state.is_default_local_media_subdir(stored_path, &effective_dl_dir, subdir)
                    || stored_path.starts_with(effective_dl_dir.join("media"))
                    || stored_path.starts_with(state.config.default_download_dir.join("media"))
                    || stored_path == old_eff.as_path();
                if (is_incomplete || is_default_root) && item.root_folder != *target_root {
                    item.root_folder = target_root.clone();
                    item.updated_at = now_ts;
                    let _ = dls.db().insert_media_item(&item);
                }
            }
        }
    }
    for (camel, snake) in [
        ("autoGrabberEnabled", "auto_grabber_enabled"),
        ("organizeEnabled", "organize_enabled"),
        ("generateNfoAndArtwork", "generate_nfo_and_artwork"),
        ("qualityUpgradeEnabled", "quality_upgrade_enabled"),
        ("scorerAntiFake", "scorer_anti_fake"),
        ("subtitlesAutoDownload", "subtitles_auto_download"),
        ("subtitlesAllowHi", "subtitles_allow_hi"),
        ("cleanerEnabled", "cleaner_enabled"),
        ("legalDisclaimerAccepted", "legal_disclaimer_accepted"),
    ] {
        if let Some(val) = params.get(camel) {
            let v = val
                .as_bool()
                .ok_or_else(|| JsonRpcError::invalid_params(&format!("Invalid '{camel}'")))?;
            state
                .downloads
                .lock()
                .await
                .set_setting(snake, if v { "1" } else { "0" })?;
            keys.push(camel);
        }
    }
    if let Some(val) = params.get("mediaServerProfile") {
        let profile = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'mediaServerProfile'"))?
            .trim()
            .to_lowercase();
        if !["jellyfin", "plex", "kodi"].contains(&profile.as_str()) {
            return Err(JsonRpcError::invalid_params(
                "'mediaServerProfile' must be 'jellyfin', 'plex' or 'kodi'",
            ));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("media_server_profile", &profile)?;
        keys.push("mediaServerProfile");
    }
    if let Some(val) = params.get("qualityMaxResolution") {
        let res = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'qualityMaxResolution'"))?
            .trim()
            .to_lowercase();
        if !["720p", "1080p", "2160p"].contains(&res.as_str()) {
            return Err(JsonRpcError::invalid_params(
                "'qualityMaxResolution' must be '720p', '1080p' or '2160p'",
            ));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("quality_max_resolution", &res)?;
        keys.push("qualityMaxResolution");
    }
    if let Some(val) = params.get("subtitlesLanguage") {
        let lang = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'subtitlesLanguage'"))?
            .trim();
        state
            .downloads
            .lock()
            .await
            .set_setting("subtitles_language", lang)?;
        keys.push("subtitlesLanguage");
    }
    if let Some(val) = params.get("subtitlesVariant") {
        let variant = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'subtitlesVariant'"))?
            .trim();
        if !["es-ES", "es-LA", "any"].contains(&variant) {
            return Err(JsonRpcError::invalid_params(
                "'subtitlesVariant' must be 'es-ES', 'es-LA' or 'any'",
            ));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("subtitles_variant", variant)?;
        keys.push("subtitlesVariant");
    }
    if let Some(val) = params.get("subtitlesMinScore") {
        let score = val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'subtitlesMinScore'"))?;
        if score > 200 {
            return Err(JsonRpcError::invalid_params(
                "'subtitlesMinScore' must be between 0 and 200",
            ));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("subtitles_min_score", &score.to_string())?;
        keys.push("subtitlesMinScore");
    }
    if let Some(val) = params.get("subtitlesAutoSync") {
        let mode = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'subtitlesAutoSync'"))?
            .trim();
        if !["off", "fps_only", "embedded_sub"].contains(&mode) {
            return Err(JsonRpcError::invalid_params(
                "'subtitlesAutoSync' must be 'off', 'fps_only' or 'embedded_sub'",
            ));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("subtitles_auto_sync", mode)?;
        keys.push("subtitlesAutoSync");
    }
    if let Some(val) = params.get("opensubtitlesApiKey") {
        let key = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'opensubtitlesApiKey'"))?
            .trim();
        state
            .downloads
            .lock()
            .await
            .set_setting("opensubtitles_api_key", key)?;
        keys.push("opensubtitlesApiKey");
    }
    if let Some(val) = params.get("updateChannel") {
        let raw_channel = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'updateChannel'"))?;
        let normalized = crate::services::updater::service::normalize_update_channel(raw_channel)
            .ok_or_else(|| {
                JsonRpcError::invalid_params("'updateChannel' must be 'stable' or 'beta'")
            })?;
        state
            .downloads
            .lock()
            .await
            .set_setting("update_channel", normalized)?;
        state
            .updater
            .set_channel(normalized)
            .await
            .map_err(|e| JsonRpcError::business(e.to_string()))?;
        keys.push("updateChannel");
    }
    if let Some(val) = params.get("mcpEnabled") {
        let en = val
            .as_bool()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'mcpEnabled'"))?;
        state
            .downloads
            .lock()
            .await
            .set_setting("mcp_enabled", if en { "1" } else { "0" })?;
        keys.push("mcpEnabled");
    }
    if let Some(val) = params.get("mcpToolProfile") {
        let profile = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'mcpToolProfile'"))?
            .trim()
            .to_lowercase();
        if !["read", "safe", "full"].contains(&profile.as_str()) {
            return Err(JsonRpcError::invalid_params(
                "'mcpToolProfile' must be 'read', 'safe' or 'full'",
            ));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("mcp_tool_profile", &profile)?;
        keys.push("mcpToolProfile");
    }
    if let Some(val) = params.get("ramProfile") {
        let profile = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'ramProfile'"))?
            .trim()
            .to_lowercase();
        if !["low", "balanced", "high"].contains(&profile.as_str()) {
            return Err(JsonRpcError::invalid_params(
                "'ramProfile' must be 'low', 'balanced' or 'high'",
            ));
        }

        // Aplicar presupuestos de red e I/O en caliente según el perfil seleccionado
        let mut b = state.network_policy.budgets();
        match profile.as_str() {
            "low" => {
                b.max_total_buffer_bytes = 8 * 1024 * 1024; // 8 MiB buffer global
                b.max_active_peer_connections = 120;
            }
            "high" => {
                b.max_total_buffer_bytes = 64 * 1024 * 1024; // 64 MiB buffer global
                b.max_active_peer_connections = 1000;
            }
            _ => {
                b.max_total_buffer_bytes = 32 * 1024 * 1024; // 32 MiB equilibrado
                b.max_active_peer_connections = 500;
            }
        }
        let _ = state.network_policy.set_budgets(b);

        state
            .downloads
            .lock()
            .await
            .set_setting("ram_profile", &profile)?;
        tracing::info!("[CACHE] Perfil de memoria RAM configurado: {profile}");
        keys.push("ramProfile");
    }
    for (camel, snake) in [
        ("sonarrUrl", "sonarr_url"),
        ("radarrUrl", "radarr_url"),
        ("jellyfinUrl", "jellyfin_url"),
        ("jellyfinApiKey", "jellyfin_api_key"),
        ("plexUrl", "plex_url"),
        ("plexToken", "plex_token"),
        ("kodiUrl", "kodi_url"),
    ] {
        if let Some(val) = params.get(camel) {
            let s = val
                .as_str()
                .ok_or_else(|| JsonRpcError::invalid_params(&format!("Invalid '{camel}'")))?
                .trim();
            state
                .downloads
                .lock()
                .await
                .set_setting(snake, s)?;
            keys.push(camel);
        }
    }
    // Nivel de registro en caliente — únicamente en tiempo de ejecución, nunca persistido (un
    // `trace` persistente tras reiniciar llenaría los archivos). Acepta un
    // nivel simple («debug») o una especificación EnvFilter completa.
    if let Some(val) = params.get("logLevel") {
        let spec = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'logLevel'"))?;
        let control = state
            .log_control
            .as_ref()
            .ok_or_else(|| JsonRpcError::internal("Log control unavailable"))?;
        control
            .set(spec)
            .map_err(|e| JsonRpcError::invalid_params(&e))?;
        tracing::info!("[LOG] Niveau de log changé à chaud: {spec}");
        keys.push("logLevel");
    }
    if let Some(val) = params.get("webuiPassword") {
        let pass = val
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'webuiPassword'"))?
            .trim();
        if pass.len() > MAX_WEBUI_PASSWORD_LEN {
            return Err(JsonRpcError::invalid_params("'webuiPassword' is too long"));
        }
        if pass.is_empty() {
            state
                .downloads
                .lock()
                .await
                .remove_setting(crate::state::SETTING_WEBUI_PASSWORD_HASH)?;
            state.set_webui_password_hash(None);
        } else {
            let hash = AppState::hash_webui_password(pass);
            state
                .downloads
                .lock()
                .await
                .set_setting(crate::state::SETTING_WEBUI_PASSWORD_HASH, &hash)?;
            state.set_webui_password_hash(Some(hash));
        }
        keys.push("webuiPassword");
    }
    if ports_changed {
        apply_port_change(state).await?;
    }

    if updated_max_global.is_some() || updated_max_per_file.is_some() {
        let (current_max_g, current_max_f) = {
            let dls = state.downloads.lock().await;
            let g = dls
                .get_setting("max_global_download_peers")
                .ok()
                .flatten()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(state.config.max_global_download_peers);
            let f = dls
                .get_setting("max_peers_per_download")
                .ok()
                .flatten()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(state.config.max_peers_per_download);
            (g, f)
        };
        state
            .peer_session
            .set_swarm_limits(current_max_f, current_max_g)
            .await;

        let mut b = state.network_policy.budgets();
        if b.max_active_peer_connections < current_max_g {
            b.max_active_peer_connections =
                current_max_g.min(mule_network_policy::MAX_PEER_CONNECTIONS_CAP);
            let _ = state.network_policy.set_budgets(b);
        }
    }

    if updated_dir.is_some() {
        keys.push("defaultDownloadDir");
    }
    if updated_retention.is_some() {
        keys.push("historyRetentionDays");
    }
    if updated_max_global.is_some() {
        keys.push("maxGlobalDownloadPeers");
    }
    if updated_max_per_file.is_some() {
        keys.push("maxPeersPerDownload");
    }
    if updated_min_per_file.is_some() {
        keys.push("minPeersPerDownload");
    }
    if updated_max_active.is_some() {
        keys.push("maxActiveDownloads");
    }
    if updated_share_dir.is_some() {
        keys.push("shareDownloadDir");
    }
    if let Some(obj) = params.as_object() {
        for k in obj.keys() {
            if !KNOWN_SETTINGS_KEYS.contains(&k.as_str()) {
                tracing::warn!("settings.set: clé inconnue ignorée '{k}'");
            }
        }
    }

    if !keys.is_empty() {
        state.emit_event(|sequence| crate::services::DaemonEvent::SettingsChanged {
            sequence,
            keys: keys.iter().map(|k| k.to_string()).collect(),
        });
    }

    Ok(json!({
        "success": true,
        "defaultDownloadDir": updated_dir,
        "historyRetentionDays": updated_retention,
        "maxGlobalDownloadPeers": updated_max_global,
        "maxPeersPerDownload": updated_max_per_file,
        "minPeersPerDownload": updated_min_per_file,
        "shareDownloadDir": updated_share_dir,
        "nickname": params.get("nickname").and_then(|v| v.as_str()),
        "obfuscationEnabled": params.get("obfuscationEnabled").and_then(|v| v.as_bool()),
        "obfuscationRequired": params.get("obfuscationRequired").and_then(|v| v.as_bool()),
        "maxConnections": params.get("maxConnections").and_then(|v| v.as_u64()),
        "maxConnPerMin": params.get("maxConnPerMin").and_then(|v| v.as_u64()),
        "maxUploadQueue": params.get("maxUploadQueue").and_then(|v| v.as_u64()),
        "ussEnabled": params.get("ussEnabled").and_then(|v| v.as_bool()),
    }))
}

pub async fn handle_settings_set_retention_days(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let retention_days = params["retentionDays"]
        .as_u64()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'retentionDays' parameter"))?
        as u32;

    let mut dls = state.downloads.lock().await;
    dls.set_setting("history_retention_days", &retention_days.to_string())?;
    let pruned_count = dls.apply_retention_policy(retention_days)?;
    drop(dls);
    state.emit_event(|sequence| crate::services::DaemonEvent::SettingsChanged {
        sequence,
        keys: vec!["historyRetentionDays".to_string()],
    });
    Ok(json!({
        "success": true,
        "retentionDays": retention_days,
        "prunedCount": pruned_count
    }))
}

pub async fn handle_settings_reset_download_dir(state: &AppState) -> Result<Value, JsonRpcError> {
    state
        .downloads
        .lock()
        .await
        .remove_setting("default_download_directory")?;

    let portable_default =
        crate::workdir::WorkDirectoryResolver::resolve(None, None).default_download_dir;
    state.emit_event(|sequence| crate::services::DaemonEvent::SettingsChanged {
        sequence,
        keys: vec!["defaultDownloadDir".to_string()],
    });
    Ok(json!({
        "success": true,
        "defaultDownloadDir": portable_default.to_string_lossy(),
        "isCustom": false
    }))
}

/// Aplica un cambio de puerto TCP/UDP en tiempo de ejecución (eMule «Puerto cliente»):
/// los valores persistidos se recargan en los atómicos de puerto efectivo, el
/// listener TCP entrante se revincula si estaba escuchando, el socket Kad UDP
/// se revincula en el nuevo puerto (cuando UDP está activo), y se actualizan los puertos
/// anunciados para buddy/UPnP.
async fn apply_port_change(state: &AppState) -> Result<(), JsonRpcError> {
    let (tcp, udp) = {
        let dl = state.downloads.lock().await;
        let tcp = dl
            .get_setting("ed2k_port")?
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or_else(|| state.ed2k_port());
        let udp = dl
            .get_setting("kad_port")?
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or_else(|| state.kad_port());
        (tcp, udp)
    };
    state.set_effective_ports(tcp, udp);
    state.kad.set_ed2k_tcp_port(tcp);
    let (obf_e, obf_r) = state.effective_obfuscation().await;
    let opts: u8 = if obf_e { 0x03 } else { 0 } | if obf_r { 0x04 } else { 0 };
    state.kad.udp().set_buddy_tcp_info(tcp, opts);
    state.upnp.set_ports(tcp, udp).await;

    let profile = state.network_policy.profile();
    let bind_ip = if profile.is_loopback_only() {
        "127.0.0.1"
    } else {
        "0.0.0.0"
    };

    // Revincular el listener TCP entrante si ya estaba escuchando.
    if state.inbound_listener.snapshot().await.is_listening {
        state.inbound_listener.stop().await;
        let addr: std::net::SocketAddr = format!("{bind_ip}:{tcp}")
            .parse()
            .map_err(|e| JsonRpcError::invalid_params(&format!("Invalid bind addr: {e}")))?;
        state
            .inbound_listener
            .start(addr)
            .await
            .map_err(|e| JsonRpcError::business(format!("TCP rebind failed: {e}")))?;
    }

    // Revincular Kad UDP en el nuevo puerto cuando UDP esté habilitado y permitido.
    if state.udp_enabled() && profile.allows_udp_kad() {
        if let Ok(addr) = format!("{bind_ip}:{udp}").parse::<std::net::SocketAddr>() {
            let current_addr = state.kad.udp().bind_addr().await;
            if current_addr != Some(addr) {
                let _ = state.kad.udp().stop().await;
                if let Err(e) = state.kad.udp().start(addr).await {
                    tracing::warn!("Kad UDP rebind failed on {addr}: {e}");
                }
            }
        }
    }
    Ok(())
}

/// Inicia/detiene los servicios UDP según el selector `udp_enabled` (eMule
/// «UDP desactivado»). Kad UDP + servidor UDP comparten el selector; la capa UDP
/// de clientes va montada sobre el socket Kad, por lo que lo sigue automáticamente.
pub(crate) async fn apply_udp_state(state: &AppState, enabled: bool) {
    let profile = state.network_policy.profile();
    if enabled && profile.allows_udp_kad() {
        let ip = if profile.is_loopback_only() {
            "127.0.0.1"
        } else {
            "0.0.0.0"
        };
        if let Ok(addr) = format!("{ip}:{}", state.kad_port()).parse::<std::net::SocketAddr>() {
            let current_addr = state.kad.udp().bind_addr().await;
            if current_addr != Some(addr) {
                if let Err(e) = state.kad.udp().start(addr).await {
                    tracing::warn!("Kad UDP start failed on {addr}: {e}");
                }
            }
        }
        if let Ok(ip_addr) = ip.parse() {
            let _ = state.server_udp.start(ip_addr).await;
        }
        let _ = state.start_quic_transport().await;
    } else {
        let _ = state.kad.udp().stop().await;
        state.server_udp.stop().await;
        state.stop_quic_transport().await;
    }
}

/// Resuelve el primer ancestro existente de una ruta para consultar sus metadatos en disco.
#[cfg(unix)]
fn existing_ancestor_metadata(path: &std::path::Path) -> Option<std::fs::Metadata> {
    let mut current = Some(path);
    while let Some(p) = current {
        if let Ok(meta) = std::fs::metadata(p) {
            return Some(meta);
        }
        current = p.parent();
    }
    None
}

/// Comprueba si dos rutas residen en el mismo dispositivo (`st_dev`) para permitir enlaces duros (`hard_link`).
pub(crate) fn are_paths_on_same_device(
    path_a: &std::path::Path,
    path_b: &std::path::Path,
) -> Option<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta_a = existing_ancestor_metadata(path_a)?;
        let meta_b = existing_ancestor_metadata(path_b)?;
        Some(meta_a.dev() == meta_b.dev())
    }
    #[cfg(not(unix))]
    {
        let _ = (path_a, path_b);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::are_paths_on_same_device;

    #[test]
    fn test_are_paths_on_same_device_mismo_tempdir() {
        let tmp = tempfile::tempdir().expect("tempdir válido");
        let dir_a = tmp.path().join("downloads");
        let dir_b = tmp.path().join("media/Movies");
        std::fs::create_dir_all(&dir_a).expect("crear downloads");
        #[cfg(unix)]
        assert_eq!(are_paths_on_same_device(&dir_a, &dir_b), Some(true));
        #[cfg(not(unix))]
        assert_eq!(are_paths_on_same_device(&dir_a, &dir_b), None);
    }
}


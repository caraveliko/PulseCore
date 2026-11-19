// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::JsonRpcError;
use crate::rpc::methods::DAEMON_VERSION;
use crate::state::AppState;

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
    "sonarrApiKey",
    "radarrUrl",
    "radarrApiKey",
    "jellyfinUrl",
    "jellyfinApiKey",
    "plexUrl",
    "autoGrabberEnabled",
    "scorerAntiFake",
    "subtitlesAutoDownload",
    "subtitlesLanguage",
    "cleanerEnabled",
    "mcpEnabled",
    "mcpToolProfile",
    "ramProfile",
    "logLevel",
    "webuiPassword",
    "legalDisclaimerAccepted",
];

fn query_disk_space(path: &std::path::Path) -> (Option<u64>, Option<u64>) {
    let check_path = if path.exists() {
        path
    } else if let Some(parent) = path.parent() {
        if parent.exists() {
            parent
        } else {
            return (None, None);
        }
    } else {
        return (None, None);
    };

    let free = fs4::available_space(check_path).ok();
    let total = fs4::total_space(check_path).ok();
    (free, total)
}

type NasDiskCacheEntry = (std::time::Instant, std::path::PathBuf, Option<u64>, Option<u64>);

static NAS_DISK_CACHE: std::sync::RwLock<Option<NasDiskCacheEntry>> =
    std::sync::RwLock::new(None);

pub fn get_cached_nas_disk_space(nas_path: std::path::PathBuf) -> (Option<u64>, Option<u64>) {
    if let Ok(guard) = NAS_DISK_CACHE.read() {
        if let Some((ts, ref cached_path, free, total)) = *guard {
            if cached_path == &nas_path && ts.elapsed() < std::time::Duration::from_secs(60) {
                return (free, total);
            }
        }
    }

    let nas_for_task = nas_path.clone();
    tokio::task::spawn(async move {
        let res = tokio::task::spawn_blocking(move || {
            let ready = crate::services::nfs_mount::verify_destination_mount_ready(
                &nas_for_task,
                false,
            )
            .is_ok();
            if ready && nas_for_task.exists() {
                (nas_for_task.clone(), query_disk_space(&nas_for_task))
            } else {
                (nas_for_task, (None, None))
            }
        })
        .await;

        if let Ok((checked_path, (free, total))) = res {
            if let Ok(mut guard) = NAS_DISK_CACHE.write() {
                *guard = Some((std::time::Instant::now(), checked_path, free, total));
            }
        }
    });

    if let Ok(guard) = NAS_DISK_CACHE.read() {
        if let Some((_, ref cached_path, free, total)) = *guard {
            if cached_path == &nas_path {
                return (free, total);
            }
        }
    }

    (None, None)
}

pub async fn handle_status(state: &AppState) -> Result<Value, JsonRpcError> {
    let dl_count = state.downloads.lock().await.list_downloads().len();
    let capabilities = crate::rpc::capabilities::daemon_capabilities();

    let default_dl_dir = state
        .effective_download_dir()
        .await
        .to_string_lossy()
        .to_string();
    let nas_root_dir = state
        .effective_nas_root_dir()
        .await
        .to_string_lossy()
        .to_string();

    let (dl_free, dl_total) = query_disk_space(std::path::Path::new(&default_dl_dir));
    let (data_free, data_total) = query_disk_space(&state.config.data_dir);
    let (nas_free, nas_total) =
        get_cached_nas_disk_space(std::path::PathBuf::from(&nas_root_dir));

    Ok(json!({
        "version": DAEMON_VERSION,
        "status": "healthy",
        "downloadsCount": dl_count,
        "instanceId": state.instance_id.to_string(),
        "capabilities": capabilities,
        "workdirMode": format!("{:?}", state.config.workdir_mode),
        "workdirPath": state.config.data_dir.to_string_lossy(),
        "defaultDownloadDir": default_dl_dir,
        "networkState": state.network_policy.state().as_str(),
        "networkProfile": state.network_policy.profile().to_string(),
        "logLevel": state.current_log_level(),
        "storage": {
            "incoming": {
                "path": default_dl_dir,
                "freeBytes": dl_free,
                "totalBytes": dl_total,
            },
            "dataDir": {
                "path": state.config.data_dir.to_string_lossy(),
                "freeBytes": data_free,
                "totalBytes": data_total,
            },
            "nas": {
                "path": nas_root_dir,
                "freeBytes": nas_free,
                "totalBytes": nas_total,
            }
        }
    }))
}

#[cfg(windows)]
fn query_windows_memory() -> (u64, u64) {
    #[repr(C)]
    struct MemoryStatusEx {
        dw_length: u32,
        dw_memory_load: u32,
        ull_total_phys: u64,
        ull_avail_phys: u64,
        ull_total_page_file: u64,
        ull_avail_page_file: u64,
        ull_total_virtual: u64,
        ull_avail_virtual: u64,
        ull_avail_extended_virtual: u64,
    }
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(lp_buffer: *mut MemoryStatusEx) -> i32;
    }
    let mut status = MemoryStatusEx {
        dw_length: std::mem::size_of::<MemoryStatusEx>() as u32,
        dw_memory_load: 0,
        ull_total_phys: 0,
        ull_avail_phys: 0,
        ull_total_page_file: 0,
        ull_avail_page_file: 0,
        ull_total_virtual: 0,
        ull_avail_virtual: 0,
        ull_avail_extended_virtual: 0,
    };
    // SAFETY: `status.dw_length` está inicializado al tamaño exacto de la estructura.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok != 0 {
        (status.ull_total_phys, status.ull_avail_phys)
    } else {
        (0, 0)
    }
}

fn get_host_metrics() -> Value {
    let mut cpu_pct: f64 = 0.0;
    if let Ok(loadavg) = std::fs::read_to_string("/proc/loadavg") {
        if let Some(first) = loadavg.split_whitespace().next() {
            if let Ok(load1) = first.parse::<f64>() {
                let cores = std::thread::available_parallelism()
                    .map(|n| n.get() as f64)
                    .unwrap_or(4.0);
                cpu_pct = ((load1 / cores) * 100.0).clamp(0.0, 100.0);
            }
        }
    }

    #[cfg(windows)]
    let (mem_total, mem_avail) = query_windows_memory();
    #[cfg(not(windows))]
    let (mem_total, mem_avail) = {
        let mut total: u64 = 0;
        let mut avail: u64 = 0;
        if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
            for line in meminfo.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    let kb: u64 = rest.split_whitespace().next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    total = kb * 1024;
                } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
                    let kb: u64 = rest.split_whitespace().next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    avail = kb * 1024;
                }
            }
        }
        (total, avail)
    };
    let mem_used = mem_total.saturating_sub(mem_avail);
    let hostname = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Servidor".to_string());

    json!({
        "hostname": hostname,
        "os": std::env::consts::OS,
        "cpuPct": cpu_pct.round() as u64,
        "memTotalBytes": mem_total,
        "memUsedBytes": mem_used,
        "memAvailableBytes": mem_avail,
    })
}

pub async fn handle_disk_space(state: &AppState) -> Result<Value, JsonRpcError> {
    let default_dl_dir = state
        .effective_download_dir()
        .await
        .to_string_lossy()
        .to_string();
    let nas_root_dir = state
        .effective_nas_root_dir()
        .await
        .to_string_lossy()
        .to_string();

    let nas_for_inspect = std::path::PathBuf::from(&nas_root_dir);
    let nfs_status = tokio::task::spawn_blocking(move || {
        crate::services::nfs_mount::inspect_directory_status(&nas_for_inspect, false)
    })
    .await
    .ok();

    let (dl_free, dl_total) = query_disk_space(std::path::Path::new(&default_dl_dir));
    let (data_free, data_total) = query_disk_space(&state.config.data_dir);
    let (nas_free, nas_total) =
        get_cached_nas_disk_space(std::path::PathBuf::from(&nas_root_dir));

    Ok(json!({
        "incoming": {
            "path": default_dl_dir,
            "freeBytes": dl_free,
            "totalBytes": dl_total,
        },
        "dataDir": {
            "path": state.config.data_dir.to_string_lossy(),
            "freeBytes": data_free,
            "totalBytes": data_total,
        },
        "nas": {
            "path": nas_root_dir,
            "freeBytes": nas_free,
            "totalBytes": nas_total,
            "nfs": nfs_status,
        },
        "host": get_host_metrics(),
    }))
}

static NAS_TELEMETRY_CACHE: std::sync::RwLock<Option<(std::time::Instant, Value)>> =
    std::sync::RwLock::new(None);

fn parse_nas_telemetry(raw: &str) -> Value {
    let parts: Vec<&str> = raw.split("###").collect();
    let mut load1: f64 = 0.0;
    let mut load5: f64 = 0.0;
    let mut load15: f64 = 0.0;
    if let Some(part0) = parts.first() {
        let fields: Vec<&str> = part0.split_whitespace().collect();
        if fields.len() >= 3 {
            load1 = fields[0].parse::<f64>().unwrap_or(0.0);
            load5 = fields[1].parse::<f64>().unwrap_or(0.0);
            load15 = fields[2].parse::<f64>().unwrap_or(0.0);
        }
    }

    let mut mem_total: u64 = 0;
    let mut mem_used: u64 = 0;
    let mut mem_free: u64 = 0;
    let mut mem_avail: u64 = 0;
    let mut swap_total: u64 = 0;
    let mut swap_used: u64 = 0;
    if let Some(part1) = parts.get(1) {
        for line in part1.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if line.starts_with("Mem:") && fields.len() >= 4 {
                mem_total = fields[1].parse().unwrap_or(0);
                mem_used = fields[2].parse().unwrap_or(0);
                mem_free = fields[3].parse().unwrap_or(0);
                if fields.len() >= 7 {
                    mem_avail = fields[6].parse().unwrap_or(0);
                }
            } else if line.starts_with("Swap:") && fields.len() >= 3 {
                swap_total = fields[1].parse().unwrap_or(0);
                swap_used = fields[2].parse().unwrap_or(0);
            }
        }
    }

    let mut vol_total: u64 = 0;
    let mut vol_used: u64 = 0;
    let mut vol_free: u64 = 0;
    let mut vol_pct: u32 = 0;
    if let Some(part2) = parts.get(2) {
        for line in part2.lines() {
            if line.contains("/volume1") {
                let fields: Vec<&str> = line.split_whitespace().collect();
                if fields.len() >= 5 {
                    vol_total = fields[1].parse().unwrap_or(0);
                    vol_used = fields[2].parse().unwrap_or(0);
                    vol_free = fields[3].parse().unwrap_or(0);
                    let pct_str = fields[4].trim_end_matches('%');
                    vol_pct = pct_str.parse().unwrap_or(0);
                }
            }
        }
    }

    let raid_healthy = parts.get(3).is_some_and(|s| s.contains("[UU]"));

    let parse_temp = |s: &str| -> Option<u32> {
        for token in s.split_whitespace().rev() {
            if let Ok(n) = token.parse::<u32>() {
                if (10..100).contains(&n) {
                    return Some(n);
                }
            }
        }
        None
    };

    let temp_sda = parts.get(4).and_then(|s| parse_temp(s));
    let temp_sdb = parts.get(5).and_then(|s| parse_temp(s));

    let mem_used_pct = if mem_total > 0 {
        ((mem_used as f64 / mem_total as f64) * 1000.0).round() / 10.0
    } else {
        0.0
    };

    let swap_used_pct = if swap_total > 0 {
        ((swap_used as f64 / swap_total as f64) * 1000.0).round() / 10.0
    } else {
        0.0
    };

    json!({
        "reachable": true,
        "model": "Synology DS212j",
        "cpu": {
            "load1": load1,
            "load5": load5,
            "load15": load15,
            "pctEstimated": ((load1 * 100.0).clamp(0.0, 100.0) as u32),
        },
        "memory": {
            "totalBytes": mem_total,
            "usedBytes": mem_used,
            "freeBytes": mem_free,
            "availableBytes": mem_avail,
            "usedPct": mem_used_pct,
        },
        "swap": {
            "totalBytes": swap_total,
            "usedBytes": swap_used,
            "usedPct": swap_used_pct,
        },
        "volume": {
            "path": "/volume1",
            "totalBytes": vol_total,
            "usedBytes": vol_used,
            "freeBytes": vol_free,
            "usedPct": vol_pct,
        },
        "raid": {
            "name": "md2",
            "healthy": raid_healthy,
            "status": if raid_healthy { "active raid1 [2/2] [UU]" } else { "degraded" },
        },
        "disks": [
            {
                "id": "sda",
                "model": "WD Red WD30EFRX",
                "tempC": temp_sda,
                "health": "Normal",
            },
            {
                "id": "sdb",
                "model": "WD Red WD30EFRX",
                "tempC": temp_sdb,
                "health": "Normal",
            }
        ]
    })
}

pub async fn handle_nas_info(state: &AppState) -> Result<Value, JsonRpcError> {
    let effective_nas_root = state.effective_nas_root_dir().await;
    let nas_for_inspect = effective_nas_root.clone();
    let nfs_status = tokio::task::spawn_blocking(move || {
        crate::services::nfs_mount::inspect_directory_status(&nas_for_inspect, false)
    })
    .await
    .ok();

    let attach_nfs = |mut v: Value| -> Value {
        if let (Some(obj), Some(ref status)) = (v.as_object_mut(), &nfs_status) {
            obj.insert("nfs".to_string(), json!(status));
        }
        v
    };

    // 1. Comprobar la caché (TTL de 60s para evitar reactivaciones de CPU en Raspberry Pi)
    if let Ok(cache) = NAS_TELEMETRY_CACHE.read() {
        if let Some((ts, ref val)) = *cache {
            if ts.elapsed() < std::time::Duration::from_secs(60) {
                return Ok(attach_nfs(val.clone()));
            }
        }
    }

    // 2. Consultar mediante SSH multiplexado sin contraseña al NAS si está configurado
    if let Some(ref ssh_target) = state.config.nas_ssh_target {
        let port_str = state.config.nas_ssh_port.to_string();
        let ssh_res = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::process::Command::new("ssh")
                .args([
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "ConnectTimeout=2",
                    "-o",
                    "StrictHostKeyChecking=no",
                    "-o",
                    "ControlMaster=auto",
                    "-o",
                    "ControlPath=/tmp/ssh-nas-%r@%h:%p",
                    "-o",
                    "ControlPersist=10m",
                    "-p",
                    &port_str,
                    ssh_target.as_str(),
                    "cat /proc/loadavg; echo '###'; free -b; echo '###'; df -B1 /volume1; echo '###'; cat /proc/mdstat; echo '###'; smartctl -d ata -A /dev/sda 2>/dev/null | grep '194 Temp'; echo '###'; smartctl -d ata -A /dev/sdb 2>/dev/null | grep '194 Temp'",
                ])
                .output(),
        )
        .await;

        if let Ok(Ok(output)) = ssh_res {
            if output.status.success() {
                let raw = String::from_utf8_lossy(&output.stdout);
                let val = parse_nas_telemetry(&raw);
                if let Ok(mut cache) = NAS_TELEMETRY_CACHE.write() {
                    *cache = Some((std::time::Instant::now(), val.clone()));
                }
                return Ok(attach_nfs(val));
            }
        }
    }

    // Respaldo: si existe un valor en caché, devolverlo con la marca de desactualizado (stale)
    if let Ok(cache) = NAS_TELEMETRY_CACHE.read() {
        if let Some((_, ref val)) = *cache {
            let mut stale_val = val.clone();
            if let Some(obj) = stale_val.as_object_mut() {
                obj.insert("stale".to_string(), json!(true));
            }
            return Ok(attach_nfs(stale_val));
        }
    }

    // Respaldo 2: espacio en disco del montaje NFS (en caché y sin bloqueo)
    let (free_b, total_b) = get_cached_nas_disk_space(effective_nas_root.clone());
    Ok(attach_nfs(json!({
        "reachable": false,
        "model": "NAS",
        "volume": {
            "path": effective_nas_root.to_string_lossy(),
            "freeBytes": free_b,
            "totalBytes": total_b,
        }
    })))
}

pub async fn handle_stop(state: &AppState) -> Result<Value, JsonRpcError> {
    let notify = state.shutdown_notify.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        notify.notify_waiters();
    });
    Ok(json!({ "stopping": true }))
}

/// `stats.resetLifetimeTotals` — restablece a cero los contadores acumulados
/// «desde la instalación» (descarga + subida): los contadores en
/// memoria que alimentan `statsTick` Y los valores persistidos. Los totales
/// de sesión no se tocan; los bytes transmitidos tras el restablecimiento
/// vuelven a acumularse a partir de cero.
pub async fn handle_stats_reset_lifetime_totals(state: &AppState) -> Result<Value, JsonRpcError> {
    state
        .lifetime_downloaded_bytes
        .store(0, std::sync::atomic::Ordering::SeqCst);
    state
        .lifetime_uploaded_bytes
        .store(0, std::sync::atomic::Ordering::SeqCst);
    let mut dl = state.downloads.lock().await;
    dl.set_setting("lifetime_downloaded_bytes", "0")?;
    dl.set_setting("lifetime_uploaded_bytes", "0")?;
    Ok(json!({ "reset": true }))
}

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

        json!({
            "minUploadKbs": min_up,
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
        })
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
    let mut result = json!({
        "defaultDownloadDir": current_dir.to_string_lossy(),
        "portableDownloadDir": portable_default.to_string_lossy(),
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
        ("scorerAntiFake", "scorer_anti_fake"),
        ("subtitlesAutoDownload", "subtitles_auto_download"),
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

/// Devuelve entradas de registro del daemon procesadas a partir de archivos JSON rotativos
pub async fn handle_logs(state: &AppState, params: Option<Value>) -> Result<Value, JsonRpcError> {
    let limit = params
        .as_ref()
        .and_then(|p| p.get("limit"))
        .and_then(|v| v.as_u64())
        .unwrap_or(200)
        .min(1000) as usize;

    let filter_level = params
        .as_ref()
        .and_then(|p| p.get("level"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_uppercase());

    let search_query = params
        .as_ref()
        .and_then(|p| p.get("search"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase());

    let logs_dir = state.config.data_dir.join("logs");

    tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();
        if !logs_dir.is_dir() {
            return Ok(json!({ "logs": entries, "total": 0 }));
        }

        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&logs_dir)
            .ok()
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with("daemon") && n.ends_with(".log"))
                            .unwrap_or(false)
                    })
                    .collect()
            })
            .unwrap_or_default();

        files.sort();
        files.reverse(); // los más recientes primero

        for file_path in files {
            if entries.len() >= limit {
                break;
            }
            if let Ok(file) = std::fs::File::open(&file_path) {
                use std::io::{BufRead, BufReader};
                let reader = BufReader::new(file);
                let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();

                for line in lines.into_iter().rev() {
                    if entries.len() >= limit {
                        break;
                    }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                        let level = v.get("level").and_then(|l| l.as_str()).unwrap_or("INFO");
                        if let Some(ref fl) = filter_level {
                            if fl != "ALL" && level != fl {
                                continue;
                            }
                        }

                        let target = v.get("target").and_then(|t| t.as_str()).unwrap_or("");
                        let msg = v
                            .get("fields")
                            .and_then(|f| f.get("message"))
                            .and_then(|m| m.as_str())
                            .unwrap_or("");

                        if let Some(ref sq) = search_query {
                            if !msg.to_lowercase().contains(sq) && !target.to_lowercase().contains(sq) {
                                continue;
                            }
                        }

                        let ts = v.get("timestamp").and_then(|t| t.as_str()).unwrap_or("");
                        entries.push(json!({
                            "timestamp": ts,
                            "level": level,
                            "target": target,
                            "message": msg
                        }));
                    }
                }
            }
        }

        entries.reverse();

        Ok(json!({
            "logs": entries,
            "total": entries.len()
        }))
    })
    .await
    .map_err(|e| JsonRpcError::internal(format!("Logs task error: {e}")))?
}

/// Lista subdirectorios existentes en el servidor para el explorador de carpetas de la WebUI.
pub async fn handle_settings_list_dirs(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let requested = params
        .as_ref()
        .and_then(|p| p.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from);

    let fallback = state.config.default_download_dir.clone();

    tokio::task::spawn_blocking(move || {
        let current = resolve_existing_directory(requested, &fallback);
        let parent_path = current
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().into_owned());

        let mut directories = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&current) {
            for entry in entries.flatten() {
                let Ok(ft) = entry.file_type() else {
                    continue;
                };
                if !ft.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = entry.path().to_string_lossy().into_owned();
                directories.push((name, path));
            }
        }
        directories.sort_by_key(|a| a.0.to_lowercase());

        let dirs_json: Vec<Value> = directories
            .into_iter()
            .map(|(name, path)| json!({ "name": name, "path": path }))
            .collect();

        Ok(json!({
            "currentPath": current.to_string_lossy(),
            "parentPath": parent_path,
            "directories": dirs_json,
            "roots": list_available_roots(),
        }))
    })
    .await
    .map_err(|e| JsonRpcError::internal(format!("List dirs task error: {e}")))?
}

fn resolve_existing_directory(
    requested: Option<std::path::PathBuf>,
    fallback: &std::path::Path,
) -> std::path::PathBuf {
    let mut candidate = requested.unwrap_or_else(|| fallback.to_path_buf());
    loop {
        if candidate.is_dir() {
            return candidate;
        }
        match candidate.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => {
                candidate = parent.to_path_buf();
            }
            _ => break,
        }
    }
    if fallback.is_dir() {
        return fallback.to_path_buf();
    }
    #[cfg(windows)]
    {
        std::path::PathBuf::from("C:\\")
    }
    #[cfg(not(windows))]
    {
        std::path::PathBuf::from("/")
    }
}

fn list_available_roots() -> Vec<String> {
    #[cfg(windows)]
    {
        (b'A'..=b'Z')
            .map(|drive| format!("{}:\\", drive as char))
            .filter(|root| std::path::Path::new(root).is_dir())
            .collect()
    }
    #[cfg(not(windows))]
    {
        ["/", "/home", "/mnt", "/media"]
            .into_iter()
            .filter(|root| std::path::Path::new(root).is_dir())
            .map(str::to_string)
            .collect()
    }
}

/// Abre el selector nativo de carpetas en Windows; en otros sistemas indica `supported: false`.
pub async fn handle_settings_pick_folder(params: Option<Value>) -> Result<Value, JsonRpcError> {
    #[cfg(windows)]
    {
        let initial_path = params
            .as_ref()
            .and_then(|p| p.get("initialPath"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        tokio::task::spawn_blocking(move || pick_folder_windows(initial_path))
            .await
            .map_err(|e| JsonRpcError::internal(format!("Pick folder task error: {e}")))?
    }
    #[cfg(not(windows))]
    {
        let _ = params;
        Ok(json!({ "supported": false }))
    }
}

#[cfg(windows)]
fn pick_folder_windows(initial_path: Option<String>) -> Result<Value, JsonRpcError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let script = concat!(
        "Add-Type -AssemblyName System.Windows.Forms;",
        "$dlg = New-Object System.Windows.Forms.FolderBrowserDialog;",
        "$dlg.Description = 'Seleccionar carpeta';",
        "$dlg.ShowNewFolderButton = $true;",
        "if ($env:MULE_PICK_INITIAL_PATH -and (Test-Path -LiteralPath $env:MULE_PICK_INITIAL_PATH)) { $dlg.SelectedPath = $env:MULE_PICK_INITIAL_PATH };",
        "$top = New-Object System.Windows.Forms.Form;",
        "$top.TopMost = $true;",
        "if ($dlg.ShowDialog($top) -eq [System.Windows.Forms.DialogResult]::OK) {",
        " [Console]::OutputEncoding = [System.Text.Encoding]::UTF8;",
        " Write-Output $dlg.SelectedPath",
        "}"
    );

    let mut cmd = std::process::Command::new("powershell");
    cmd.creation_flags(CREATE_NO_WINDOW)
        .args(["-NoProfile", "-STA", "-Command", script]);
    if let Some(ref initial) = initial_path {
        cmd.env("MULE_PICK_INITIAL_PATH", initial);
    }

    let output = cmd
        .output()
        .map_err(|e| JsonRpcError::internal(format!("No se pudo abrir el selector de carpetas: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(JsonRpcError::internal(format!(
            "El selector de carpetas falló: {}",
            stderr.trim()
        )));
    }

    let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if selected.is_empty() {
        Ok(json!({
            "supported": true,
            "cancelled": true,
            "path": Value::Null,
        }))
    } else {
        Ok(json!({
            "supported": true,
            "cancelled": false,
            "path": selected,
        }))
    }
}

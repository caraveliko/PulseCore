// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde_json::{json, Value};

use crate::error::JsonRpcError;
use crate::rpc::methods::DAEMON_VERSION;
use crate::state::AppState;

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

pub(super) type NasDiskCacheEntry = (std::time::Instant, std::path::PathBuf, Option<u64>, Option<u64>);

pub(super) static NAS_DISK_CACHE: std::sync::RwLock<Option<NasDiskCacheEntry>> =
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
    let dl_count = state.downloads.lock().await.downloads_count();
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


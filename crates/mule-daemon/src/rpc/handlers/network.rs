// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde_json::{json, Value};

use crate::error::JsonRpcError;
use crate::state::AppState;

fn emit_policy_changed(state: &AppState) {
    state.emit_event(|sequence| crate::services::DaemonEvent::NetworkPolicyChanged { sequence });
}

pub async fn handle_network_get_policy(state: &AppState) -> Result<Value, JsonRpcError> {
    let profile = state.network_policy.profile();
    let net_state = state.network_policy.state();
    let budgets = state.network_policy.budgets();
    let peer_conns = state.network_policy.active_peer_connections_count();
    let server_conns = state.network_policy.active_server_connections_count();
    let listeners = state.network_policy.active_listeners_count();

    Ok(json!({
        "profile": profile.to_string(),
        "state": net_state.as_str(),
        "budgets": budgets,
        "activeConnections": {
            "peers": peer_conns,
            "servers": server_conns,
            "listeners": listeners
        }
    }))
}

pub async fn handle_network_set_profile(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params"))?;
    let profile_str = params["profile"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'profile' string"))?;
    let profile: mule_network_policy::NetworkProfile = profile_str
        .parse()
        .map_err(|e: String| JsonRpcError::invalid_params(&e))?;

    state.network_policy.set_profile(profile);
    emit_policy_changed(state);
    handle_network_get_policy(state).await
}

pub async fn handle_network_disconnect_all(state: &AppState) -> Result<Value, JsonRpcError> {
    state.network_policy.disconnect_all();
    // El token cancelado destruye limpiamente todas las sesiones TCP (pares
    // y servidor) — su epílogo libera el mapa de sesiones, los bloques en vuelo y los
    // estados en SQLite. `last_target` se conserva para que `network.enable`
    // pueda restaurar la sesión mediante el observador de reconexión.
    let _ = state.server_session.disconnect_for_kill_switch().await;
    state.inbound_listener.stop().await;
    // Los sockets UDP no están vinculados al token de cancelación — sin detención
    // explícita seguirían emitiendo/recibiendo durante el corte
    // de red (kill switch).
    let _ = state.kad.udp().stop().await;
    state.server_udp.stop().await;
    state.stop_quic_transport().await;
    // Los supervisores (observador de reconexión, coordinador de enjambre, rotador
    // de identidad, cuota de subida) permanecen vivos: se auto-inhiben
    // mientras el estado sea `Disabled` y se reanudan en `network.enable`.
    // Solo los pares de laboratorio, no cubiertos por el token, se
    // cancelan aquí.
    #[cfg(feature = "lab-tools")]
    {
        let mut peer = state.active_lab_peer.lock().await;
        if let Some((_, h)) = peer.take() {
            h.abort();
        }
    }
    emit_policy_changed(state);
    Ok(json!({
        "success": true,
        "state": state.network_policy.state().as_str()
    }))
}

pub async fn handle_network_enable(state: &AppState) -> Result<Value, JsonRpcError> {
    let was_disabled = state.network_policy.state() == mule_network_policy::NetworkState::Disabled;
    state.network_policy.enable();
    if was_disabled {
        // `enable()` acaba de crear un nuevo token de cancelación: reiniciar
        // los servicios que estaban vinculados al anterior (oyente entrante,
        // sockets UDP, mantenimiento de Kad, USS, refresco UPnP). Los
        // supervisores nunca fueron terminados — retoman solos su
        // pulso en cuanto el estado deja de ser `Disabled`, y el observador
        // reconecta al `last_target` conservado por el interruptor de corte.
        let profile = state.network_policy.profile();
        let bind_ip = if profile.is_loopback_only() {
            "127.0.0.1"
        } else {
            "0.0.0.0"
        };

        if profile.is_preview() || profile.is_loopback_only() {
            if let Ok(bind_addr) =
                format!("{bind_ip}:{}", state.ed2k_port()).parse::<std::net::SocketAddr>()
            {
                match state.inbound_listener.start(bind_addr).await {
                    Ok(bound) => tracing::info!("Inbound eD2K listener restarted on {bound}"),
                    Err(e) => {
                        tracing::warn!("Failed to restart inbound listener on {bind_addr}: {e}")
                    }
                }
            }
        }

        if state.udp_enabled() {
            crate::rpc::handlers::settings::apply_udp_state(state, true).await;
            if profile.allows_udp_kad() {
                let handle = state
                    .kad
                    .start_background_maintenance(state.network_policy.cancellation_token());
                state.register_task(handle.abort_handle()).await;
            }
        }

        state.uss.start(
            state.network_policy.rate_limiters(),
            state.network_policy.cancellation_token(),
        );

        if state.upnp.snapshot().await.enabled && profile.is_preview() {
            let upnp_svc = state.upnp.clone();
            let (tcp, udp) = (state.ed2k_port(), state.kad_port());
            tokio::spawn(async move {
                let _ = upnp_svc.map_ports(tcp, udp).await;
            });
            let refresh = state.upnp.clone().start_periodic_refresh(
                std::time::Duration::from_secs(45 * 60),
                state.network_policy.cancellation_token(),
            );
            state.register_task(refresh.abort_handle()).await;
        }
    }
    emit_policy_changed(state);
    Ok(json!({
        "success": true,
        "state": state.network_policy.state().as_str()
    }))
}

pub async fn handle_network_get_audit_log(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let limit = params
        .as_ref()
        .and_then(|p| p["limit"].as_u64())
        .unwrap_or(50) as usize;
    let limit = limit.clamp(1, 500);

    let entries = state.network_policy.audit_entries(limit);
    Ok(json!({
        "entries": entries
    }))
}

pub async fn handle_network_set_budgets(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params"))?;
    let budgets_val = if params.get("budgets").is_some() {
        &params["budgets"]
    } else {
        &params
    };
    let budgets: mule_network_policy::ResourceBudgets = serde_json::from_value(budgets_val.clone())
        .map_err(|e| JsonRpcError::invalid_params(&format!("Invalid budgets format: {e}")))?;

    state
        .network_policy
        .set_budgets(budgets)
        .map_err(|e| JsonRpcError::invalid_params(&format!("Failed to set budgets: {e}")))?;

    emit_policy_changed(state);
    handle_network_get_policy(state).await
}

pub async fn handle_filter_get_status(state: &AppState) -> Result<Value, JsonRpcError> {
    let stats = state.network_policy.ip_filter().stats();
    Ok(crate::rpc::handlers::json_value(&stats))
}

pub async fn handle_filter_import_file(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params"))?;
    let path_str = params["filePath"]
        .as_str()
        .or_else(|| params["path"].as_str())
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'filePath' or 'path'"))?;

    let path = std::path::Path::new(path_str);
    if !path.exists() {
        return Err(JsonRpcError::invalid_params(&format!(
            "Specified file does not exist: {}",
            path_str
        )));
    }

    state
        .network_policy
        .ip_filter()
        .load_from_file(path)
        .map_err(|e| {
            JsonRpcError::invalid_params(&format!("Failed to parse ipfilter file: {e}"))
        })?;

    // Persistir la ruta de origen para recargar el filtro en el arranque (eMule
    // mantiene ipfilter.dat cargado entre reinicios).
    state
        .downloads
        .lock()
        .await
        .set_setting("ipfilter_path", path_str)?;

    // Mismo formato que `filter.getStatus`/`setEnabled`/`clear`: el cliente
    // procesa un `FilterStatusModel` plano — el informe de importación ya
    // está expuesto mediante `stats.last_import_report` (almacenado por
    // `load_from_file`).
    handle_filter_get_status(state).await
}

pub async fn handle_filter_set_enabled(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params"))?;
    let enabled = params["enabled"]
        .as_bool()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'enabled' boolean"))?;

    state.network_policy.ip_filter().set_enabled(enabled);
    state
        .downloads
        .lock()
        .await
        .set_setting("ipfilter_enabled", if enabled { "1" } else { "0" })?;
    handle_filter_get_status(state).await
}

pub async fn handle_filter_clear(state: &AppState) -> Result<Value, JsonRpcError> {
    state.network_policy.ip_filter().clear();
    state
        .downloads
        .lock()
        .await
        .set_setting("ipfilter_path", "")?;
    handle_filter_get_status(state).await
}

pub async fn handle_limits_get(state: &AppState) -> Result<Value, JsonRpcError> {
    let limiters = state.network_policy.rate_limiters();
    let download = limiters.download_bucket.rate_per_sec();
    let upload = limiters.upload_bucket.rate_per_sec();
    let conn_per_sec = limiters.conn_bucket.rate_per_sec();
    let server_req_per_sec = limiters.server_bucket.rate_per_sec();
    let kad_pkt_per_sec = limiters.kad_bucket.rate_per_sec();

    let uss = state.uss.snapshot();
    let uss_cfg = state.uss.config();
    let (persisted_dl, persisted_ul) = read_persisted_limits(state).await;
    let smart_slot_target_kbs = state
        .inbound_listener
        .upload_manager()
        .target_slot_speed_bps()
        / 1024;
    let quota_eco_upload_kbs = {
        let dls = state.downloads.lock().await;
        dls.get_setting("quota_eco_upload_kbs")
            .ok()
            .flatten()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0)
    };

    Ok(json!({
        "downloadBytesPerSec": download,
        "uploadBytesPerSec": upload,
        // Valores *configurados* persistidos — el depósito anterior lleva
        // el valor efectivo (proporción eD2K de eMule ya aplicada si subida < 20 KB/s).
        "downloadConfiguredBytesPerSec": persisted_dl.unwrap_or(download),
        "uploadConfiguredBytesPerSec": persisted_ul.unwrap_or(upload),
        "downloadRatioApplied": download < persisted_dl.unwrap_or(download),
        "connectionsPerMinute": conn_per_sec * 60,
        "serverRequestsPerSec": server_req_per_sec,
        "kadPacketsPerSec": kad_pkt_per_sec,
        "smartSlotTargetKbs": smart_slot_target_kbs,
        "quotaEcoUploadKbs": quota_eco_upload_kbs,
        "uss": {
            "enabled": uss.enabled,
            "state": uss.state,
            "currentLimitBytesPerSec": uss.current_limit_bps,
            "latencyMs": uss.latency_ms,
            "lowestPingMs": uss.lowest_ping_ms,
            "queuingDelayMs": uss.queuing_delay_ms,
            "reason": uss.reason,
            "idleCapActive": uss.idle_cap_active,
            "streamingCapActive": uss.streaming_cap_active,
            "activeStreams": uss.active_streams,
            "quotaEcoActive": uss.quota_eco_active,
            "ed2kLimitBytesPerSec": uss.ed2k_upload_limit_bps,
            "torrentLimitBytesPerSec": uss.torrent_upload_limit_bps,
            "acceptNewClient": uss.accept_new_client,
            "pingTarget": uss.ping_target,
            "config": uss_cfg,
        }
    }))
}

pub async fn handle_limits_set(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params"))?;
    // Semilla de la configuración activa: una actualización parcial no debe restablecer
    // silenciosamente los otros limitadores a los valores predeterminados de compilación.
    let live = state.network_policy.rate_limiters();
    let mut cfg = mule_filter::RateLimitsConfig {
        download_bytes_per_sec: live.download_bucket.rate_per_sec(),
        upload_bytes_per_sec: live.upload_bucket.rate_per_sec(),
        connections_per_minute: (live.conn_bucket.rate_per_sec() * 60) as u32,
        server_requests_per_sec: live.server_bucket.rate_per_sec() as u32,
        kad_packets_per_sec: live.kad_bucket.rate_per_sec() as u32,
    };

    let (cap_dl_kbs, cap_ul_kbs) = read_capacities(state).await;
    let (persisted_dl, persisted_ul) = read_persisted_limits(state).await;

    // Límite *configurado* (prioridad: parámetro > ajuste persistido > depósito
    // activo). El depósito solo contiene el valor efectivo (proporción ya aplicada) —
    // sembrar sobre el depósito haría decrecer el límite en cada llamada.
    let dl_cfg = params["downloadBytesPerSec"]
        .as_u64()
        .or_else(|| params["downloadLimit"].as_u64().map(|kb| kb * 1024))
        .map(|dl| clamp_to_capacity(dl, cap_dl_kbs, 9, 10))
        .or(persisted_dl)
        .unwrap_or_else(|| live.download_bucket.rate_per_sec());
    let dl_bytes = params["downloadBytesPerSec"]
        .as_u64()
        .or_else(|| params["downloadLimit"].as_u64().map(|kb| kb * 1024));

    let ul_cfg = params["uploadBytesPerSec"]
        .as_u64()
        .or_else(|| params["uploadLimit"].as_u64().map(|kb| kb * 1024))
        .map(|ul| clamp_to_capacity(ul, cap_ul_kbs, 4, 5))
        .or(persisted_ul)
        .unwrap_or_else(|| live.upload_bucket.rate_per_sec());
    let ul_bytes = params["uploadBytesPerSec"]
        .as_u64()
        .or_else(|| params["uploadLimit"].as_u64().map(|kb| kb * 1024));
    if ul_bytes.is_some() {
        state.uss.note_manual_limit(ul_cfg);
        // eMule: cuando DynUp (USS) está activo, el límite de subida configurado
        // es su techo (`LastCommonRouteFinder` acota el límite
        // dinámico a MaxUpload). 0 = techo ilimitado.
        if state.uss.snapshot().enabled {
            state.uss.update_config(|c| c.max_upload_bps = ul_cfg).await;
        }
    }
    state.uss.note_manual_download_limit(dl_cfg);

    // `GetMaxDownloadInBytesPerSec` de eMule: la descarga *efectiva* está
    // limitada a coef×subida cuando la subida < 20 KB/s. El persistido mantiene
    // el valor configurado; solo el depósito recibe el valor efectivo.
    cfg.upload_bytes_per_sec = ul_cfg;
    cfg.download_bytes_per_sec =
        mule_network_policy::emule_ud_ratio(dl_cfg, effective_upload_for_ratio(state, ul_cfg));
    if let Some(cpm) = params["connectionsPerMinute"].as_u64() {
        cfg.connections_per_minute = cpm as u32;
    }
    if let Some(s) = params["serverRequestsPerSec"].as_u64() {
        cfg.server_requests_per_sec = s as u32;
    }
    if let Some(k) = params["kadPacketsPerSec"].as_u64() {
        cfg.kad_packets_per_sec = k as u32;
    }

    state.network_policy.rate_limiters().apply_config(&cfg);
    if !state.uss.snapshot().enabled {
        state
            .uss
            .apply_split_upload_limit(ul_cfg, &state.network_policy.rate_limiters())
            .await;
    }
    if let Some(engine) = state.torrent.get() {
        engine.set_download_limit_bps(cfg.download_bytes_per_sec);
    }

    // Persistir los campos proporcionados (eMule guarda MaxUpload/MaxDownload en
    // preferences.ini) para que los limitadores sobrevivan a los reinicios del daemon. Se persisten
    // los valores *configurados* — la proporción upload/download se recalcula
    // en cada aplicación, nunca se reescribe en los ajustes.
    {
        let mut dls = state.downloads.lock().await;
        if dl_bytes.is_some() {
            dls.set_setting("rate_limit_download_bps", &dl_cfg.to_string())?;
        }
        if ul_bytes.is_some() {
            dls.set_setting("rate_limit_upload_bps", &ul_cfg.to_string())?;
        }
        for (param, key) in [
            ("connectionsPerMinute", "rate_limit_conn_per_min"),
            ("serverRequestsPerSec", "rate_limit_server_req_per_sec"),
            ("kadPacketsPerSec", "rate_limit_kad_pkt_per_sec"),
        ] {
            if let Some(v) = params[param].as_u64() {
                dls.set_setting(key, &v.to_string())?;
            }
        }
    }

    // Actualización opcional de la configuración de USS (Upload Speed Sense).
    if let Some(uss_params) = params.get("uss") {
        let enabled = uss_params["enabled"].as_bool();
        let ping_host = uss_params["pingHost"].as_str().map(|s| s.to_string());
        let min_up = uss_params["minUploadBytesPerSec"].as_u64();
        let max_up = uss_params["maxUploadBytesPerSec"].as_u64();
        let idle_cap = uss_params["idleUploadCapBytesPerSec"].as_u64();
        let streaming_cap = uss_params["streamingUploadCapBytesPerSec"].as_u64();
        let use_ms = uss_params["useMillisecondTolerance"].as_bool();
        let tol_pct = uss_params["pingTolerancePercent"].as_u64();
        let tol_ms = uss_params["pingToleranceMs"].as_u64();
        let up_div = uss_params["goingUpDivider"].as_u64();
        let down_div = uss_params["goingDownDivider"].as_u64();
        let n_pings = uss_params["numberOfPings"].as_u64();
        state
            .uss
            .update_config(|cfg| {
                if let Some(v) = enabled {
                    cfg.enabled = v;
                }
                if uss_params.get("pingHost").is_some() {
                    cfg.ping_host = ping_host;
                }
                if let Some(v) = min_up {
                    cfg.min_upload_bps = v;
                }
                if let Some(v) = max_up {
                    cfg.max_upload_bps = v;
                }
                if let Some(v) = idle_cap {
                    cfg.idle_upload_cap_bps = v;
                }
                if let Some(v) = streaming_cap {
                    cfg.streaming_upload_cap_bps = v;
                }
                if let Some(v) = use_ms {
                    cfg.use_ms_tolerance = v;
                }
                if let Some(v) = tol_pct {
                    cfg.ping_tolerance_percent = v as u32;
                }
                if let Some(v) = tol_ms {
                    cfg.ping_tolerance_ms = v as u32;
                }
                if let Some(v) = up_div {
                    cfg.going_up_divider = v as u32;
                }
                if let Some(v) = down_div {
                    cfg.going_down_divider = v as u32;
                }
                if let Some(v) = n_pings {
                    cfg.number_of_pings = (v as usize).max(1);
                }
            })
            .await;
    }

    if let Some(slot_val) = params.get("smartSlotTargetKbs") {
        let slot_kbs = slot_val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'smartSlotTargetKbs'"))?;
        if slot_kbs > 100_000 {
            return Err(JsonRpcError::invalid_params(
                "'smartSlotTargetKbs' must be 0..100000",
            ));
        }
        state
            .inbound_listener
            .upload_manager()
            .set_target_slot_speed(slot_kbs.saturating_mul(1024));
        state
            .downloads
            .lock()
            .await
            .set_setting("smart_slot_target_kbs", &slot_kbs.to_string())?;
    }

    if let Some(eco_val) = params.get("quotaEcoUploadKbs") {
        let eco_kbs = eco_val
            .as_u64()
            .ok_or_else(|| JsonRpcError::invalid_params("Invalid 'quotaEcoUploadKbs'"))?;
        if eco_kbs > 1_000_000 {
            return Err(JsonRpcError::invalid_params(
                "'quotaEcoUploadKbs' must be 0..1000000",
            ));
        }
        state
            .downloads
            .lock()
            .await
            .set_setting("quota_eco_upload_kbs", &eco_kbs.to_string())?;
    }

    handle_limits_get(state).await
}

/// Capacidades de línea declaradas (`capacity_download_kbs` / `capacity_upload_kbs`,
/// KB/s — `DownloadCapacity`/`UploadCapacityNew` de eMule). 0 = no indicada.
async fn read_capacities(state: &AppState) -> (u64, u64) {
    let dls = state.downloads.lock().await;
    let read = |key: &str| {
        dls.get_setting(key)
            .ok()
            .flatten()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0)
    };
    (read("capacity_download_kbs"), read("capacity_upload_kbs"))
}

/// Límites *configurados* persistidos (`rate_limit_*_bps`). eMule almacena
/// `MaxDownload`/`MaxUpload` brutos en preferences.ini — la proporción
/// subida/descarga es un techo recalculado al leer, nunca reescrito.
/// El depósito solo contiene el valor *efectivo*: por tanto, es necesario releer
/// el ajuste para evitar que la proporción se aplique en cascada.
async fn read_persisted_limits(state: &AppState) -> (Option<u64>, Option<u64>) {
    let dls = state.downloads.lock().await;
    let read = |key: &str| {
        dls.get_setting(key)
            .ok()
            .flatten()
            .and_then(|s| s.trim().parse::<u64>().ok())
    };
    (
        read("rate_limit_download_bps"),
        read("rate_limit_upload_bps"),
    )
}

/// `LoadPreferences` de eMule (Preferences.cpp): el límite efectivo no puede
/// superar la capacidad de línea declarada — download ≤ capacidad × 9/10,
/// upload ≤ capacidad × 4/5. Un límite a 0 (ilimitado) nunca se restringe.
fn clamp_to_capacity(limit_bps: u64, capacity_kbs: u64, num: u64, den: u64) -> u64 {
    if limit_bps == 0 || capacity_kbs == 0 {
        return limit_bps;
    }
    let cap = capacity_kbs.saturating_mul(1024).saturating_mul(num) / den;
    limit_bps.min(cap)
}

/// Límite de subida de referencia para la proporción de eMule: cuando USS (DynUp)
/// gobierna el depósito, su límite dinámico actual hace fe (eMule utiliza
/// `uploadqueue->GetDatarate()` cuando DynUp está activo); en caso contrario, el límite
/// de subida configurado.
fn effective_upload_for_ratio(state: &AppState, configured_ul_bps: u64) -> u64 {
    let snap = state.uss.snapshot();
    if snap.enabled && snap.current_limit_bps > 0 {
        snap.current_limit_bps
    } else {
        configured_ul_bps
    }
}

/// Reaplica la restricción de capacidad y luego la proporción upload/download a los
/// límites actuales cuando `capacity*Kbs` cambia mediante `settings.set`
/// (misma regla que en `limits.set`). Los valores *configurados* persistidos
/// hacen fe — los depósitos contienen el valor efectivo.
pub async fn apply_capacity_clamp(state: &AppState) -> Result<(), JsonRpcError> {
    let live = state.network_policy.rate_limiters();
    let (persisted_dl, persisted_ul) = read_persisted_limits(state).await;
    let dl_cfg = persisted_dl.unwrap_or_else(|| live.download_bucket.rate_per_sec());
    let ul_cfg = persisted_ul.unwrap_or_else(|| live.upload_bucket.rate_per_sec());
    let (cap_dl, cap_ul) = read_capacities(state).await;
    let clamped_dl = clamp_to_capacity(dl_cfg, cap_dl, 9, 10);
    let clamped_ul = clamp_to_capacity(ul_cfg, cap_ul, 4, 5);
    let eff_dl = mule_network_policy::emule_ud_ratio(
        clamped_dl,
        effective_upload_for_ratio(state, clamped_ul),
    );
    let eff_ul = clamped_ul;
    if eff_dl == live.download_bucket.rate_per_sec()
        && eff_ul == live.upload_bucket.rate_per_sec()
        && clamped_dl == dl_cfg
        && clamped_ul == ul_cfg
    {
        return Ok(());
    }
    {
        let mut dls = state.downloads.lock().await;
        if clamped_dl != dl_cfg {
            dls.set_setting("rate_limit_download_bps", &clamped_dl.to_string())?;
        }
        if clamped_ul != ul_cfg {
            dls.set_setting("rate_limit_upload_bps", &clamped_ul.to_string())?;
        }
    }
    state.uss.note_manual_limit(clamped_ul);
    // El techo de USS sigue el límite de subida configurado (misma regla que en
    // `limits.set`: MaxUpload acota el límite dinámico cuando USS está
    // activo). 0 = techo ilimitado.
    if state.uss.snapshot().enabled {
        state
            .uss
            .update_config(|c| c.max_upload_bps = clamped_ul)
            .await;
    }
    state.uss.note_manual_download_limit(clamped_dl);
    let cfg = mule_filter::RateLimitsConfig {
        download_bytes_per_sec: eff_dl,
        upload_bytes_per_sec: eff_ul,
        connections_per_minute: (live.conn_bucket.rate_per_sec() * 60) as u32,
        server_requests_per_sec: live.server_bucket.rate_per_sec() as u32,
        kad_packets_per_sec: live.kad_bucket.rate_per_sec() as u32,
    };
    state.network_policy.rate_limiters().apply_config(&cfg);
    if !state.uss.snapshot().enabled {
        state
            .uss
            .apply_split_upload_limit(ul_cfg, &state.network_policy.rate_limiters())
            .await;
    }
    if let Some(engine) = state.torrent.get() {
        engine.set_download_limit_bps(cfg.download_bytes_per_sec);
    }
    Ok(())
}

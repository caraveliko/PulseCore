// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Vigilante ligero de reproducciones activas en Jellyfin y Plex para modular la subida.

use std::sync::Arc;

use crate::state::AppState;

/// Intervalo de sondeo a Jellyfin / Plex cuando el recorte por streaming está activado (s).
pub const STREAMING_POLL_INTERVAL_SECS: u64 = 15;
/// Tiempo máximo de espera por petición HTTP a los servidores multimedia locales (s).
pub const STREAMING_HTTP_TIMEOUT_SECS: u64 = 4;

/// Cuenta las sesiones activas (no pausadas) en la respuesta JSON de `/Sessions` de Jellyfin.
pub fn parse_jellyfin_sessions_active(body: &str) -> usize {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(body)
    else {
        return 0;
    };
    items
        .iter()
        .filter(|item| {
            let has_media = item.get("NowPlayingItem").is_some_and(|v| !v.is_null());
            let paused = item
                .get("PlayState")
                .and_then(|ps| ps.get("IsPaused"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            has_media && !paused
        })
        .count()
}

/// Cuenta las sesiones activas en la respuesta JSON o XML de `/status/sessions` de Plex.
pub fn parse_plex_sessions_active(body: &str) -> usize {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return 0;
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(mc) = json.get("MediaContainer") {
            if let Some(meta) = mc.get("Metadata").and_then(|m| m.as_array()) {
                return meta
                    .iter()
                    .filter(|entry| {
                        entry
                            .get("Player")
                            .and_then(|p| p.get("state"))
                            .and_then(|s| s.as_str())
                            != Some("paused")
                    })
                    .count();
            }
            return mc.get("size").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        }
        return 0;
    }
    // Respaldo XML (`<MediaContainer size="N">`) cuando Plex no responde JSON.
    if trimmed.starts_with('<') {
        if let Some(pos) = trimmed.find("size=\"") {
            let rest = &trimmed[pos + 6..];
            if let Some(end) = rest.find('"') {
                return rest[..end].parse::<usize>().unwrap_or(0);
            }
        }
    }
    0
}

/// Consulta Jellyfin y/o Plex y devuelve el total de reproducciones activas detectadas.
pub async fn probe_active_streams(
    client: &reqwest::Client,
    jf_url: &str,
    jf_key: &str,
    plex_url: &str,
    plex_token: &str,
) -> usize {
    let mut total = 0usize;

    if !jf_url.is_empty() && !jf_key.is_empty() {
        let url = format!("{}/Sessions", jf_url.trim_end_matches('/'));
        match client
            .get(&url)
            .header("Authorization", format!("MediaBrowser Token=\"{jf_key}\""))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(text) = resp.text().await {
                    total = total.saturating_add(parse_jellyfin_sessions_active(&text));
                }
            }
            Ok(resp) => {
                tracing::debug!(status = %resp.status(), "Jellyfin /Sessions devolvió estado no exitoso");
            }
            Err(err) => {
                tracing::debug!(error = %err, "No se pudo consultar Jellyfin /Sessions");
            }
        }
    }

    if !plex_url.is_empty() {
        let base = plex_url.trim_end_matches('/');
        let mut url = format!("{base}/status/sessions");
        if !plex_token.is_empty() {
            url.push_str(&format!("?X-Plex-Token={plex_token}"));
        }
        match client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(text) = resp.text().await {
                    total = total.saturating_add(parse_plex_sessions_active(&text));
                }
            }
            Ok(resp) => {
                tracing::debug!(status = %resp.status(), "Plex /status/sessions devolvió estado no exitoso");
            }
            Err(err) => {
                tracing::debug!(error = %err, "No se pudo consultar Plex /status/sessions");
            }
        }
    }

    total
}

/// Inicia la tarea periódica que detecta reproducciones en Jellyfin/Plex cuando está activado
/// `streamingUploadCapKbs`.
pub fn spawn_streaming_watcher(state: Arc<AppState>) {
    let task_state = Arc::clone(&state);
    let handle = tokio::spawn(async move {
        let state = task_state;
        let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(STREAMING_HTTP_TIMEOUT_SECS))
            .build()
        else {
            return;
        };
        let mut ticker =
            tokio::time::interval(std::time::Duration::from_secs(STREAMING_POLL_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            ticker.tick().await;
            if state.uss.config().streaming_upload_cap_bps == 0 {
                state.uss.set_streaming_active(false, 0);
                continue;
            }

            let (jf_url, jf_key, plex_url, plex_token) = {
                let dls = state.downloads.lock().await;
                let read_s = |k: &str| {
                    dls.get_setting(k)
                        .ok()
                        .flatten()
                        .unwrap_or_default()
                        .trim()
                        .to_string()
                };
                let j_u = {
                    let s = read_s("jellyfin_url");
                    if s.is_empty() {
                        state.config.jellyfin_url.clone()
                    } else {
                        s
                    }
                };
                let j_k = {
                    let s = read_s("jellyfin_api_key");
                    if s.is_empty() {
                        state.config.jellyfin_api_key.clone().unwrap_or_default()
                    } else {
                        s
                    }
                };
                let p_u = {
                    let s = read_s("plex_url");
                    if s.is_empty() {
                        state.config.plex_url.clone().unwrap_or_default()
                    } else {
                        s
                    }
                };
                let p_t = read_s("plex_token");
                (j_u, j_k, p_u, p_t)
            };

            let count = probe_active_streams(&client, &jf_url, &jf_key, &plex_url, &plex_token).await;
            state.uss.set_streaming_active(count > 0, count);
        }
    });

    let abort = handle.abort_handle();
    state
        .abort_handles
        .try_lock()
        .map(|mut h| h.push(abort))
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jellyfin_counts_only_unpaused_playing_sessions() {
        let json = r#"[
            {"Id": "1", "NowPlayingItem": {"Name": "Movie"}, "PlayState": {"IsPaused": false}},
            {"Id": "2", "NowPlayingItem": {"Name": "Paused"}, "PlayState": {"IsPaused": true}},
            {"Id": "3"}
        ]"#;
        assert_eq!(parse_jellyfin_sessions_active(json), 1);
        assert_eq!(parse_jellyfin_sessions_active("invalid"), 0);
    }

    #[test]
    fn plex_parses_json_and_xml_sessions() {
        let json = r#"{"MediaContainer":{"size":2,"Metadata":[{"Player":{"state":"playing"}},{"Player":{"state":"paused"}}]}}"#;
        assert_eq!(parse_plex_sessions_active(json), 1);
        let xml = r#"<?xml version="1.0"?><MediaContainer size="2"></MediaContainer>"#;
        assert_eq!(parse_plex_sessions_active(xml), 2);
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Vigilante ligero de reproducciones activas en Jellyfin y Kodi para modular la subida.

use std::sync::Arc;

use crate::state::AppState;

/// Intervalo de sondeo a Jellyfin / Kodi cuando el recorte por streaming está activado (s).
pub const STREAMING_POLL_INTERVAL_SECS: u64 = 15;
/// Tiempo máximo de espera por petición HTTP a los reproductores multimedia locales (s).
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

/// Cuenta los reproductores activos en la respuesta JSON-RPC de `Player.GetActivePlayers` de Kodi.
pub fn parse_kodi_active_players(body: &str) -> usize {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(body) else {
        return 0;
    };
    val.get("result")
        .and_then(|r| r.as_array())
        .map(Vec::len)
        .unwrap_or(0)
}

/// Consulta Jellyfin y/o Kodi y devuelve el total de reproducciones activas detectadas.
pub async fn probe_active_streams(
    client: &reqwest::Client,
    jf_url: &str,
    jf_key: &str,
    kodi_url: &str,
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

    if !kodi_url.is_empty() {
        let url = format!("{}/jsonrpc", kodi_url.trim_end_matches('/'));
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "Player.GetActivePlayers",
            "id": 1
        })
        .to_string();
        match client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(payload)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(text) = resp.text().await {
                    total = total.saturating_add(parse_kodi_active_players(&text));
                }
            }
            Ok(resp) => {
                tracing::debug!(status = %resp.status(), "Kodi Player.GetActivePlayers devolvió estado no exitoso");
            }
            Err(err) => {
                tracing::debug!(error = %err, "No se pudo consultar Kodi Player.GetActivePlayers");
            }
        }
    }

    total
}

/// Inicia la tarea periódica que detecta reproducciones en Jellyfin y Kodi cuando está activado
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

            let (profile, jf_url, jf_key, kodi_url) = {
                let dls = state.downloads.lock().await;
                let read_s = |k: &str| {
                    dls.get_setting(k)
                        .ok()
                        .flatten()
                        .unwrap_or_default()
                        .trim()
                        .to_string()
                };
                let prof = read_s("media_server_profile").to_lowercase();
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
                let k_u = read_s("kodi_url");
                (prof, j_u, j_k, k_u)
            };

            if profile == "local" {
                state.uss.set_streaming_active(false, 0);
                continue;
            }

            let count = probe_active_streams(&client, &jf_url, &jf_key, &kodi_url).await;
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
    fn kodi_counts_active_players_from_jsonrpc() {
        let playing = r#"{"id":1,"jsonrpc":"2.0","result":[{"playerid":1,"playertype":"internal","type":"video"}]}"#;
        assert_eq!(parse_kodi_active_players(playing), 1);
        let idle = r#"{"id":1,"jsonrpc":"2.0","result":[]}"#;
        assert_eq!(parse_kodi_active_players(idle), 0);
        assert_eq!(parse_kodi_active_players("invalid"), 0);
    }
}

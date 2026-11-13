// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, Request, State,
    },
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tower_http::services::{ServeDir, ServeFile};

use crate::rpc::methods::{handle_request, DAEMON_VERSION};
use crate::rpc::types::{JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::state::AppState;

#[derive(rust_embed::RustEmbed)]
#[folder = "../../web"]
pub struct EmbeddedWebAssets;

pub fn create_router(state: AppState) -> Router {
    let mut router = Router::new()
        .route("/health", get(health_check))
        .route("/rpc", get(ws_handler))
        .route("/api/rpc", post(handle_http_rpc))
        .route("/api", post(handle_http_rpc))
        .route("/session-config.js", get(session_config_js))
        .route(
            "/transmission/rpc",
            axum::routing::post(crate::rpc::transmission::handle_transmission_rpc)
                .get(crate::rpc::transmission::handle_transmission_rpc),
        )
        .route(
            "/transmission/web",
            get(|| async { axum::response::Redirect::temporary("/") }),
        )
        .route(
            "/transmission/web/",
            get(|| async { axum::response::Redirect::temporary("/") }),
        )
        .route(
            "/torznab/api",
            axum::routing::get(crate::rpc::torznab::handle_torznab),
        )
        .route(
            "/indexer/amule/api",
            axum::routing::get(crate::rpc::torznab::handle_torznab),
        );

    // If an explicit web_dir is passed and contains index.html, serve from disk (for hot reloading/dev).
    // Otherwise fallback seamlessly to embedded web assets compiled into the binary.
    let disk_web_dir = state
        .config
        .web_dir
        .as_ref()
        .filter(|dir| dir.join("index.html").is_file());

    if let Some(web_dir) = disk_web_dir {
        router = router.fallback_service(
            ServeDir::new(web_dir).fallback(ServeFile::new(web_dir.join("index.html"))),
        );
    } else {
        router = router
            .route("/", get(embedded_index_handler))
            .route("/{*path}", get(embedded_asset_handler));
    }

    router
        .layer(middleware::from_fn(control_plane_security_guard))
        .with_state(state)
}

/// Vérifie qu'une adresse IP appartient au loopback ou au réseau privé local (RFC1918 / Link-Local / ULA).
pub fn is_trusted_lan_or_loopback_ip(ip: IpAddr) -> bool {
    let canonical = match ip {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
    };
    match canonical {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Vérifie que l'en-tête `Host` correspond à une adresse IP locale/LAN ou un nom d'hôte local
/// (protection contre les attaques par DNS Rebinding depuis un navigateur).
pub fn is_allowed_host_header(host_header: &str) -> bool {
    let host = host_header.trim();
    if host.is_empty() {
        return true;
    }
    // Gestion IPv6 entre crochets `[::1]:4660`
    let host_only = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    let lower = host_only.trim().to_ascii_lowercase();
    if lower.is_empty() || lower == "localhost" || lower.ends_with(".local") || lower.ends_with(".lan") || lower.ends_with(".home") {
        return true;
    }
    // Autoriser les noms d'hôtes locaux courts sans domaine public (ex: "raspberry", "raspberrypi")
    if !lower.contains('.') {
        return true;
    }
    if let Ok(ip) = lower.parse::<IpAddr>() {
        return is_trusted_lan_or_loopback_ip(ip);
    }
    false
}

/// Vérifie que l'en-tête `Origin` (envoyé par les navigateurs lors des requêtes POST / WebSocket)
/// provient bien de l'interface locale/LAN et non d'un site web externe malveillant (CSRF / CSWSH).
pub fn is_allowed_origin_header(origin_header: &str) -> bool {
    let origin = origin_header.trim();
    if origin.is_empty() || origin == "null" || origin.starts_with("file://") {
        return true;
    }
    let without_scheme = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .or_else(|| origin.strip_prefix("ws://"))
        .or_else(|| origin.strip_prefix("wss://"))
        .unwrap_or(origin);
    let host_part = without_scheme.split('/').next().unwrap_or("");
    is_allowed_host_header(host_part)
}

async fn control_plane_security_guard(req: Request, next: Next) -> impl IntoResponse {
    // 1. Isolation IP : refuser toute connexion provenant d'Internet hors LAN/Loopback
    let client_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));

    if !is_trusted_lan_or_loopback_ip(client_ip) {
        tracing::warn!(
            "[SECURITY] Blocked external non-LAN access to control port from {}",
            client_ip
        );
        return (
            StatusCode::FORBIDDEN,
            "403 Forbidden: Access restricted to local network (LAN/Loopback)",
        )
            .into_response();
    }

    // 2. Protection anti-DNS Rebinding sur l'en-tête Host
    if let Some(host_val) = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
    {
        if !is_allowed_host_header(host_val) {
            tracing::warn!(
                "[SECURITY] Blocked potential DNS Rebinding request with Host '{}' from {}",
                host_val,
                client_ip
            );
            return (
                StatusCode::FORBIDDEN,
                "403 Forbidden: Untrusted Host header (DNS Rebinding protection)",
            )
                .into_response();
        }
    }

    // 3. Protection anti-CSRF / Cross-Site WebSocket Hijacking sur l'en-tête Origin
    if let Some(origin_val) = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        if !is_allowed_origin_header(origin_val) {
            tracing::warn!(
                "[SECURITY] Blocked cross-origin request from Origin '{}' (client {})",
                origin_val,
                client_ip
            );
            return (
                StatusCode::FORBIDDEN,
                "403 Forbidden: Cross-Origin request blocked",
            )
                .into_response();
        }
    }

    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::X_FRAME_OPTIONS,
        header::HeaderValue::from_static("SAMEORIGIN"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        header::HeaderValue::from_static("no-referrer"),
    );
    resp
}

async fn embedded_index_handler() -> impl IntoResponse {
    match EmbeddedWebAssets::get("index.html") {
        Some(content) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
                (header::PRAGMA, "no-cache"),
                (header::EXPIRES, "0"),
            ],
            content.data,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn embedded_asset_handler(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> impl IntoResponse {
    let clean = path.trim_start_matches('/');
    let target = if clean.is_empty() { "index.html" } else { clean };

    match EmbeddedWebAssets::get(target) {
        Some(content) => {
            let mime = mime_guess::from_path(target).first_or_octet_stream();
            (
                [
                    (header::CONTENT_TYPE, mime.as_ref()),
                    (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
                ],
                content.data,
            )
                .into_response()
        }
        None => match EmbeddedWebAssets::get("index.html") {
            Some(index) => (
                [
                    (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                    (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
                    (header::PRAGMA, "no-cache"),
                    (header::EXPIRES, "0"),
                ],
                index.data,
            )
                .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
    }
}

async fn health_check() -> impl IntoResponse {
    Json(json!({
        "status": "healthy",
        "daemonVersion": DAEMON_VERSION
    }))
}

async fn handle_http_rpc(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    // Vérification du jeton Bearer si fourni, ou accès LAN/Loopback déjà validé par `control_plane_security_guard`.
    let expected = state.config.auth_token.as_str();
    let mut is_authenticated = if expected.is_empty() {
        true
    } else if let Some(auth_hdr) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        auth_hdr
            .strip_prefix("Bearer ")
            .map(|t| t.trim() == expected)
            .unwrap_or(false)
    } else {
        // Autorisé sans mot de passe uniquement depuis le réseau local (filtré en amont par control_plane_security_guard)
        true
    };
    let resp = handle_request(&state, req, &mut is_authenticated).await;
    Json(resp)
}

/// Emits `window.MULE_SESSION = {endpoint, token}` consumed by the bundled
/// web UI to auto-connect. The token is only exposed when the socket is
/// loopback-bound — on a LAN-facing bind a remote page must provide `auth_token`.
async fn session_config_js(State(state): State<AppState>) -> impl IntoResponse {
    let token = state.config.auth_token.clone();
    let endpoint = if state.config.bind_addr.ip().is_unspecified() {
        "/rpc".to_string()
    } else {
        format!("ws://{}/rpc", state.config.bind_addr)
    };
    let session = json!({
        "endpoint": endpoint,
        "token": token,
        "instanceId": state.instance_id.to_string(),
    });
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        format!("window.MULE_SESSION={session};"),
    )
        .into_response()
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

/// Maps a notification method name to its subscription topic, so clients
/// can opt into a subset of the broadcast stream via `events.subscribe`.
/// `None` = topic-less event that is always forwarded (e.g. snapshotRequired).
/// Delegates to `DaemonEvent::topic` — the single source of truth.
fn event_topic(method: &str) -> Option<&'static str> {
    crate::services::DaemonEvent::Custom {
        method: method.to_string(),
        payload: serde_json::Value::Null,
    }
    .topic()
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx_events = state.event_tx.subscribe();
    let is_authenticated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
        state.config.auth_token.is_empty(),
    ));

    // Per-session topic filter: `None` = forward everything (default, for
    // backward compatibility with clients that never call events.subscribe
    // with topics).
    let topics = std::sync::Arc::new(tokio::sync::RwLock::<
        Option<std::collections::HashSet<String>>,
    >::new(None));

    // Outbound message queue so spawned request handlers can send responses concurrently
    let (tx_out, mut rx_out) = tokio::sync::mpsc::channel::<Message>(128);

    loop {
        tokio::select! {
            // Outbound message from spawned tasks or broadcast
            out_msg = rx_out.recv() => {
                match out_msg {
                    Some(msg) => {
                        if sender.send(msg).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            // Incoming client message
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let parsed: Result<JsonRpcRequest, _> = serde_json::from_str(&text);
                        match parsed {
                            Ok(req) => {
                                // Session-scoped subscription handled here, not
                                // in handle_request, because it mutates this
                                // connection's own event filter. Only honored
                                // once authenticated — otherwise falls through
                                // to handle_request which answers auth_failed.
                                if req.method == "events.subscribe"
                                    && is_authenticated
                                        .load(std::sync::atomic::Ordering::SeqCst)
                                {
                                    let topics_clone = topics.clone();
                                    let tx_out_clone = tx_out.clone();
                                    tokio::spawn(async move {
                                        let set: Option<std::collections::HashSet<String>> =
                                            req.params
                                                .as_ref()
                                                .and_then(|p| p.get("topics"))
                                                .and_then(|t| t.as_array())
                                                .map(|arr| {
                                                    arr.iter()
                                                        .filter_map(|v| v.as_str().map(String::from))
                                                        .collect()
                                                });
                                        *topics_clone.write().await = set;
                                        if let Some(id) = req.id {
                                            let resp = JsonRpcResponse::success(
                                                id,
                                                serde_json::json!({ "subscribed": true }),
                                            );
                                            if let Ok(json_str) = serde_json::to_string(&resp) {
                                                let _ = tx_out_clone.send(Message::Text(json_str.into())).await;
                                            }
                                        }
                                    });
                                    continue;
                                }

                                let state_clone = state.clone();
                                let auth_clone = is_authenticated.clone();
                                let tx_out_clone = tx_out.clone();

                                tokio::spawn(async move {
                                    tracing::debug!("RPC incoming request: method={}, id={:?}", req.method, req.id);
                                    let mut auth_val = auth_clone.load(std::sync::atomic::Ordering::SeqCst);
                                    if let Some(resp) = handle_request(&state_clone, req, &mut auth_val).await {
                                        auth_clone.store(auth_val, std::sync::atomic::Ordering::SeqCst);
                                        if let Some(ref err) = resp.error {
                                            tracing::warn!("RPC response error: id={:?}, code={}, msg={}", resp.id, err.code, err.message);
                                        } else {
                                            tracing::debug!("RPC response success: id={:?}", resp.id);
                                        }
                                        if let Ok(json_str) = serde_json::to_string(&resp) {
                                            let _ = tx_out_clone.send(Message::Text(json_str.into())).await;
                                        }
                                    }
                                });
                            }
                            Err(_) => {
                                let err_resp = JsonRpcResponse::error(
                                    serde_json::Value::Null,
                                    JsonRpcError::new(-32700, "Parse error"),
                                );
                                if let Ok(json_str) = serde_json::to_string(&err_resp) {
                                    let _ = tx_out.send(Message::Text(json_str.into())).await;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            // Outgoing broadcast events (forwarded only to authenticated sessions)
            event = rx_events.recv() => {
                match event {
                    Ok(event_json) => {
                        if is_authenticated.load(std::sync::atomic::Ordering::SeqCst) {
                            let forward = match &*topics.read().await {
                                None => true,
                                Some(set) => {
                                    let method = serde_json::from_str::<serde_json::Value>(&event_json)
                                        .ok()
                                        .and_then(|v| v.get("method").and_then(|m| m.as_str()).map(String::from));
                                    match method.as_deref().and_then(event_topic) {
                                        Some(topic) => set.contains(topic),
                                        None => true, // control events always pass
                                    }
                                }
                            };
                            if forward {
                                let _ = tx_out.send(Message::Text(event_json.into())).await;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        // This client fell behind: events were dropped. Ask it to
                        // resynchronize via a full snapshot instead of pretending
                        // the stream is continuous.
                        if is_authenticated.load(std::sync::atomic::Ordering::SeqCst) {
                            let notif = JsonRpcNotification::new(
                                "event.snapshotRequired",
                                serde_json::json!({ "skipped": skipped }),
                            );
                            if let Ok(json_str) = serde_json::to_string(&notif) {
                                let _ = tx_out.send(Message::Text(json_str.into())).await;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lan_and_loopback_ip_guard() {
        assert!(is_trusted_lan_or_loopback_ip("127.0.0.1".parse().unwrap()));
        assert!(is_trusted_lan_or_loopback_ip("::1".parse().unwrap()));
        assert!(is_trusted_lan_or_loopback_ip(
            "192.168.1.166".parse().unwrap()
        ));
        assert!(is_trusted_lan_or_loopback_ip("10.0.0.5".parse().unwrap()));
        assert!(is_trusted_lan_or_loopback_ip(
            "172.20.0.10".parse().unwrap()
        ));
        // Les adresses IP publiques Internet doivent être refusées
        assert!(!is_trusted_lan_or_loopback_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_trusted_lan_or_loopback_ip(
            "203.0.113.99".parse().unwrap()
        ));
    }

    #[test]
    fn test_dns_rebinding_host_guard() {
        assert!(is_allowed_host_header("localhost:4660"));
        assert!(is_allowed_host_header("127.0.0.1:4660"));
        assert!(is_allowed_host_header("192.168.1.166:4660"));
        assert!(is_allowed_host_header("raspberry:4660"));
        assert!(is_allowed_host_header("raspberry.local:4660"));
        // Les domaines externes de DNS Rebinding doivent être bloqués
        assert!(!is_allowed_host_header("evil-rebind.attacker.com:4660"));
        assert!(!is_allowed_host_header("example.org"));
    }

    #[test]
    fn test_csrf_origin_guard() {
        assert!(is_allowed_origin_header("http://192.168.1.166:4660"));
        assert!(is_allowed_origin_header("http://localhost:4660"));
        assert!(is_allowed_origin_header("null"));
        assert!(!is_allowed_origin_header("https://malicious-site.com"));
        assert!(!is_allowed_origin_header("http://8.8.8.8:4660"));
    }
}


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

/// Dirección IP del cliente propagada por el guardia de seguridad de plano de control.
#[derive(Clone, Copy)]
pub(crate) struct ClientIp(pub(crate) IpAddr);

pub fn create_router(state: AppState) -> Router {
    let mut router = Router::new()
        .route("/health", get(health_check))
        .route("/rpc", get(ws_handler))
        .route("/api/rpc", post(handle_http_rpc))
        .route("/api", post(handle_http_rpc))
        .route("/api/auth/login", post(handle_webui_login))
        .route(
            "/api/system/upload-update",
            post(crate::rpc::handlers::updater_upload::handle_upload_update).layer(
                axum::extract::DefaultBodyLimit::max(
                    crate::services::updater::constants::MAX_UPDATE_PAYLOAD_BYTES,
                ),
            ),
        )
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

    // Si se pasa un web_dir explícito y contiene index.html, servir desde disco (para recarga en caliente/desarrollo).
    // De lo contrario, recurrir de forma transparente a los activos web embebidos compilados en el binario.
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

/// Comprueba si una dirección IP corresponde a la interfaz local (loopback),
/// normalizando direcciones IPv6 mapeadas a IPv4 (`::ffff:127.0.0.1`).
pub fn is_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.is_loopback(),
            None => v6.is_loopback(),
        },
    }
}

/// Comprueba que una dirección IP pertenezca a loopback o a la red privada local (RFC1918 / Link-Local / ULA).
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

/// Comprueba que la cabecera `Host` corresponda a una dirección IP local/LAN o a un nombre de host local
/// (protección contra ataques de reconexión DNS / DNS Rebinding desde un navegador).
pub fn is_allowed_host_header(host_header: &str) -> bool {
    let host = host_header.trim();
    if host.is_empty() {
        return true;
    }
    // Gestión de IPv6 entre corchetes `[::1]:4660`
    let host_only = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    let lower = host_only.trim().to_ascii_lowercase();
    if lower.is_empty() || lower == "localhost" || lower.ends_with(".local") || lower.ends_with(".lan") || lower.ends_with(".home") {
        return true;
    }
    // Permitir nombres de host locales cortos sin dominio público (ej.: "raspberry", "raspberrypi")
    if !lower.contains('.') {
        return true;
    }
    if let Ok(ip) = lower.parse::<IpAddr>() {
        return is_trusted_lan_or_loopback_ip(ip);
    }
    false
}

/// Comprueba que la cabecera `Origin` (enviada por los navegadores en peticiones POST / WebSocket)
/// provenga de la interfaz local/LAN y no de un sitio web externo malicioso (CSRF / CSWSH).
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

async fn control_plane_security_guard(mut req: Request, next: Next) -> impl IntoResponse {
    // 1. Aislamiento IP: rechazar cualquier conexión procedente de Internet fuera de LAN/Loopback
    let client_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));

    req.extensions_mut().insert(ClientIp(client_ip));

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

    // 2. Protección anti-DNS Rebinding en la cabecera Host
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

    // 3. Protección anti-CSRF / Cross-Site WebSocket Hijacking en la cabecera Origin
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

async fn handle_webui_login(
    State(state): State<AppState>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.has_webui_password() {
        return (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "token": state.config.auth_token,
            })),
        );
    }

    let candidate = payload.get("password").and_then(|v| v.as_str());
    match candidate {
        Some(pass) if state.verify_webui_password(pass) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "token": state.config.auth_token,
            })),
        ),
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "ok": false,
                "error": "Contraseña incorrecta",
            })),
        ),
    }
}

async fn handle_http_rpc(
    State(state): State<AppState>,
    headers: HeaderMap,
    ext_ip: Option<axum::Extension<ClientIp>>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    let client_ip = ext_ip
        .map(|e| e.0 .0)
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    // Verificación del token Bearer si se proporciona, o acceso LAN/Loopback ya validado por `control_plane_security_guard`.
    let expected = state.config.auth_token.as_str();
    let mut is_authenticated = if expected.is_empty() {
        !(state.has_webui_password() && !is_loopback_ip(client_ip))
    } else if let Some(auth_hdr) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        auth_hdr
            .strip_prefix("Bearer ")
            .map(|t| t.trim() == expected)
            .unwrap_or(false)
    } else {
        // Sin contraseña de WebUI o desde loopback local se permite directamente;
        // si hay contraseña y la petición viene de la red local se exige token Bearer.
        !(state.has_webui_password() && !is_loopback_ip(client_ip))
    };
    let resp = handle_request(&state, req, &mut is_authenticated).await;
    Json(resp)
}

/// Emite `window.MULE_SESSION = {endpoint, token, instanceId, requiresPassword}` para la WebUI.
/// Si hay contraseña configurada para la WebUI y la petición no procede de loopback, oculta el
/// token y activa `requiresPassword: true` hasta autenticarse en `/api/auth/login`.
async fn session_config_js(State(state): State<AppState>, req: Request) -> impl IntoResponse {
    let client_ip = req
        .extensions()
        .get::<ClientIp>()
        .map(|c| c.0)
        .or_else(|| {
            req.extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ci| ci.0.ip())
        })
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let requires_password = state.has_webui_password() && !is_loopback_ip(client_ip);
    let token = if requires_password {
        String::new()
    } else {
        state.config.auth_token.clone()
    };
    let endpoint = if state.config.bind_addr.ip().is_unspecified() {
        "/rpc".to_string()
    } else {
        format!("ws://{}/rpc", state.config.bind_addr)
    };
    let session = json!({
        "endpoint": endpoint,
        "token": token,
        "instanceId": state.instance_id.to_string(),
        "requiresPassword": requires_password,
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

/// Asocia el nombre de un método de notificación con su tema (topic) de suscripción,
/// de modo que los clientes puedan suscribirse a un subconjunto del flujo de difusión mediante `events.subscribe`.
/// `None` = evento sin tema que siempre se retransmite (ej.: snapshotRequired).
/// Delega en `DaemonEvent::topic` — la única fuente de verdad.
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

    // Filtro de temas por sesión: `None` = retransmitir todo (predeterminado, para
    // compatibilidad con versiones anteriores de clientes que nunca llaman a events.subscribe
    // con temas).
    let topics = std::sync::Arc::new(tokio::sync::RwLock::<
        Option<std::collections::HashSet<String>>,
    >::new(None));

    // Cola de mensajes salientes para que los manejadores de peticiones lanzados puedan enviar respuestas simultáneamente
    let (tx_out, mut rx_out) = tokio::sync::mpsc::channel::<Message>(128);

    loop {
        tokio::select! {
            // Mensaje saliente de tareas secundarias o difusión
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
            // Mensaje entrante del cliente
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let parsed: Result<JsonRpcRequest, _> = serde_json::from_str(&text);
                        match parsed {
                            Ok(req) => {
                                // Suscripción de ámbito de sesión gestionada aquí, no
                                // en handle_request, porque muta el filtro de eventos de esta
                                // propia conexión. Solo se atiende una vez autenticado
                                // — de lo contrario continúa hacia handle_request que responde auth_failed.
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
            // Eventos de difusión salientes (retransmitidos solo a sesiones autenticadas)
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
                        // Este cliente se retrasó: se descartaron eventos. Pedirle que
                        // se resincronice mediante una instantánea completa en lugar de fingir
                        // que el flujo es continuo.
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
        // Las direcciones IP públicas de Internet deben ser rechazadas
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
        // Los dominios externos de DNS Rebinding deben ser bloqueados
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

    #[test]
    fn test_loopback_ip() {
        assert!(is_loopback_ip("127.0.0.1".parse().unwrap()));
        assert!(is_loopback_ip("::1".parse().unwrap()));
        assert!(is_loopback_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!is_loopback_ip("192.168.1.50".parse().unwrap()));
        assert!(!is_loopback_ip("::ffff:192.168.1.50".parse().unwrap()));
    }
}


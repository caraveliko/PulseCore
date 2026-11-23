// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Pruebas de integración sin conexión del sistema integral de notificaciones (loopback únicamente).

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use axum::{
    body::Bytes,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Router,
};
use mule_core::{Download, DownloadStatus, NotificationFilter, RangeSet};
use mule_daemon::rpc::methods::handle_request;
use mule_daemon::services::notifications::start_notification_monitor;
use mule_daemon::services::DaemonEvent;
use mule_format::Ed2kHash;
use mule_network_policy::NetworkProfile;
use mule_test_support::{make_req, rpc_state, rpc_state_with_profile};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[derive(Clone, Default)]
struct MockCapture {
    requests: Arc<Mutex<Vec<CapturedHttpRequest>>>,
    fail_next_webhook_count: Arc<AtomicUsize>,
}

#[derive(Clone, Debug)]
struct CapturedHttpRequest {
    path: String,
    auth: Option<String>,
    pulse_token: Option<String>,
    title: Option<String>,
    priority: Option<String>,
    tags: Option<String>,
    body: String,
}

async fn handle_mock_capture(
    State(state): State<MockCapture>,
    AxumPath(rest): AxumPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, &'static str) {
    let path = format!("/{rest}");
    let header_str = |name: &str| -> Option<String> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(ToString::to_string)
    };

    let req_entry = CapturedHttpRequest {
        path: path.clone(),
        auth: header_str("authorization"),
        pulse_token: header_str("x-pulsecore-token"),
        title: header_str("title"),
        priority: header_str("priority"),
        tags: header_str("tags"),
        body: String::from_utf8_lossy(&body).to_string(),
    };
    state.requests.lock().await.push(req_entry);

    if path.contains("retry-webhook") {
        let prev = state.fail_next_webhook_count.load(Ordering::SeqCst);
        if prev > 0
            && state
                .fail_next_webhook_count
                .compare_exchange(prev, prev - 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            return (StatusCode::INTERNAL_SERVER_ERROR, "error transitorio");
        }
    }

    (StatusCode::OK, "ok")
}

async fn spawn_mock_http_server() -> (std::net::SocketAddr, MockCapture) {
    let capture = MockCapture::default();
    let app = Router::new()
        .route("/{*rest}", post(handle_mock_capture))
        .with_state(capture.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback mock");
    let addr = listener.local_addr().expect("addr mock");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, capture)
}

#[tokio::test]
async fn test_notifications_rpc_crud_event_and_secret_masking() {
    let (state, _dir) = rpc_state("tok-notif-crud");
    let mut auth = true;
    let mut rx = state.event_tx.subscribe();

    // 1. Prueba WebUI crea una notificación y la difunde por event_tx bajo tema "notifications".
    let test_res = handle_request(
        &state,
        make_req(1, "notifications.test", Some(json!({ "channel": "webui" }))),
        &mut auth,
    )
    .await
    .expect("respuesta test webui")
    .result
    .expect("resultado exitoso test webui");
    assert_eq!(test_res["ok"], true);
    assert_eq!(test_res["channel"], "webui");

    let wire_json = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("tiempo de espera evento WS")
        .expect("trama evento WS");
    let wire_val: Value = serde_json::from_str(&wire_json).expect("JSON evento");
    assert_eq!(wire_val["method"], "event.notificationCreated");
    assert_eq!(wire_val["params"]["notification"]["kind"], "test");

    let topic = DaemonEvent::Custom {
        method: "event.notificationCreated".to_string(),
        payload: Value::Null,
    }
    .topic();
    assert_eq!(topic, Some("notifications"));

    // 2. Insertar una segunda notificación de categoría multimedia y comprobar list + filtros + markRead + clear.
    {
        let mut dl = state.downloads.lock().await;
        dl.db_mut()
            .insert_notification(&mule_core::NotificationRecord {
                id: 0,
                kind: "mediaReady".to_string(),
                severity: "success".to_string(),
                category: "media".to_string(),
                title: "Listo para ver".to_string(),
                body: "Película Ejemplo (2025)".to_string(),
                metadata_json: None,
                is_read: false,
                created_at_unix_ms: 9_999_999_999_999,
            })
            .expect("inserción segunda notificación");
    }

    let list_all = handle_request(&state, make_req(2, "notifications.list", None), &mut auth)
        .await
        .expect("list_all")
        .result
        .expect("res list_all");
    assert_eq!(list_all["unreadCount"], 2);
    assert_eq!(list_all["notifications"].as_array().unwrap().len(), 2);

    let list_media = handle_request(
        &state,
        make_req(3, "notifications.list", Some(json!({ "category": "media" }))),
        &mut auth,
    )
    .await
    .expect("list_media")
    .result
    .expect("res list_media");
    let media_arr = list_media["notifications"].as_array().unwrap();
    assert_eq!(media_arr.len(), 1);
    let media_id = media_arr[0]["id"].as_i64().unwrap();

    let mark_one = handle_request(
        &state,
        make_req(4, "notifications.markRead", Some(json!({ "id": media_id }))),
        &mut auth,
    )
    .await
    .expect("mark_one")
    .result
    .expect("res mark_one");
    assert_eq!(mark_one["updated"], 1);
    assert_eq!(mark_one["unreadCount"], 1);

    let mark_all = handle_request(
        &state,
        make_req(5, "notifications.markRead", Some(json!({ "all": true }))),
        &mut auth,
    )
    .await
    .expect("mark_all")
    .result
    .expect("res mark_all");
    assert_eq!(mark_all["updated"], 1);
    assert_eq!(mark_all["unreadCount"], 0);

    let clear_res = handle_request(&state, make_req(6, "notifications.clear", None), &mut auth)
        .await
        .expect("clear")
        .result
        .expect("res clear");
    assert_eq!(clear_res["cleared"], 2);
    assert_eq!(clear_res["unreadCount"], 0);

    // 3. setConfig + getConfig: preservación y enmascarado estricto de secretos.
    let set_res = handle_request(
        &state,
        make_req(
            7,
            "notifications.setConfig",
            Some(json!({
                "enabled": true,
                "externalEnabled": true,
                "diskLowThresholdGb": 12,
                "telegram": {
                    "enabled": true,
                    "botToken": "super-secret-tg-token",
                    "chatId": "555001"
                },
                "ntfy": {
                    "enabled": true,
                    "url": "https://ntfy.example/topic",
                    "token": "super-secret-ntfy-token"
                },
                "discord": {
                    "enabled": true,
                    "webhookUrl": "https://discord.com/api/webhooks/999/secret-hook"
                },
                "webhook": {
                    "enabled": true,
                    "url": "https://hooks.example/pulse",
                    "secret": "super-secret-webhook"
                }
            })),
        ),
        &mut auth,
    )
    .await
    .expect("setConfig")
    .result
    .expect("res setConfig");

    let raw_set_json = serde_json::to_string(&set_res).unwrap();
    for (campo, valor) in [
        ("telegram.botToken", "super-secret-tg-token"),
        ("ntfy.token", "super-secret-ntfy-token"),
        ("discord.webhookUrl", "secret-hook"),
        ("webhook.secret", "super-secret-webhook"),
    ] {
        assert!(
            !raw_set_json.contains(valor),
            "La vista de configuración no debe filtrar la credencial de {campo}"
        );
    }
    assert_eq!(set_res["telegram"]["hasBotToken"], true);
    assert_eq!(set_res["ntfy"]["hasToken"], true);
    assert_eq!(set_res["discord"]["hasWebhookUrl"], true);
    assert_eq!(set_res["webhook"]["hasSecret"], true);

    // Parche parcial sin secretos -> conserva credenciales; clearSecret -> borra.
    let get_after = handle_request(
        &state,
        make_req(
            8,
            "notifications.setConfig",
            Some(json!({
                "telegram": { "enabled": false },
                "webhook": { "clearSecret": true }
            })),
        ),
        &mut auth,
    )
    .await
    .expect("setConfig parcial")
    .result
    .expect("res parcial");
    assert_eq!(get_after["telegram"]["enabled"], false);
    assert_eq!(get_after["telegram"]["hasBotToken"], true);
    assert_eq!(get_after["webhook"]["hasSecret"], false);
}

#[tokio::test]
async fn test_external_channels_loopback_delivery_and_retries() {
    let (mock_addr, capture) = spawn_mock_http_server().await;
    let (state, _dir) = rpc_state_with_profile("tok-ext", NetworkProfile::LocalLab);
    let mut auth = true;

    state
        .notifications
        .set_telegram_api_base(format!("http://{mock_addr}"))
        .await;

    handle_request(
        &state,
        make_req(
            1,
            "notifications.setConfig",
            Some(json!({
                "enabled": true,
                "externalEnabled": true,
                "telegram": {
                    "enabled": true,
                    "botToken": "123:ABC",
                    "chatId": "424242"
                },
                "ntfy": {
                    "enabled": true,
                    "url": format!("http://{mock_addr}/ntfy/pulse-topic"),
                    "token": "ntfy-bearer-1"
                },
                "discord": {
                    "enabled": true,
                    "webhookUrl": format!("http://{mock_addr}/discord/hook")
                },
                "webhook": {
                    "enabled": true,
                    "url": format!("http://{mock_addr}/retry-webhook"),
                    "secret": "hdr-secret-99"
                }
            })),
        ),
        &mut auth,
    )
    .await
    .expect("setConfig")
    .result
    .expect("res setConfig");

    // 1. Telegram
    let tg_res = handle_request(
        &state,
        make_req(2, "notifications.test", Some(json!({ "channel": "telegram" }))),
        &mut auth,
    )
    .await
    .expect("tg")
    .result
    .expect("res tg");
    assert_eq!(tg_res["ok"], true, "Fallo Telegram: {tg_res}");

    // 2. ntfy
    let ntfy_res = handle_request(
        &state,
        make_req(3, "notifications.test", Some(json!({ "channel": "ntfy" }))),
        &mut auth,
    )
    .await
    .expect("ntfy")
    .result
    .expect("res ntfy");
    assert_eq!(ntfy_res["ok"], true, "Fallo ntfy: {ntfy_res}");

    // 3. Discord
    let disc_res = handle_request(
        &state,
        make_req(4, "notifications.test", Some(json!({ "channel": "discord" }))),
        &mut auth,
    )
    .await
    .expect("discord")
    .result
    .expect("res discord");
    assert_eq!(disc_res["ok"], true, "Fallo Discord: {disc_res}");

    // 4. Webhook genérico con reintento automático (1er fallo 500 -> 2do éxito 200)
    capture.fail_next_webhook_count.store(1, Ordering::SeqCst);
    let wh_res = handle_request(
        &state,
        make_req(5, "notifications.test", Some(json!({ "channel": "webhook" }))),
        &mut auth,
    )
    .await
    .expect("webhook")
    .result
    .expect("res webhook");
    assert_eq!(wh_res["ok"], true, "Fallo Webhook con reintento: {wh_res}");

    let reqs = capture.requests.lock().await;
    let tg_req = reqs
        .iter()
        .find(|r| r.path == "/bot123:ABC/sendMessage")
        .expect("petición Telegram");
    let tg_json: Value = serde_json::from_str(&tg_req.body).expect("JSON Telegram");
    assert_eq!(tg_json["chat_id"], "424242");
    assert_eq!(tg_json["parse_mode"], "HTML");

    let ntfy_req = reqs
        .iter()
        .find(|r| r.path == "/ntfy/pulse-topic")
        .expect("petición ntfy");
    assert_eq!(ntfy_req.auth.as_deref(), Some("Bearer ntfy-bearer-1"));
    assert_eq!(ntfy_req.title.as_deref(), Some("Prueba de Pulse Core"));
    assert!(ntfy_req.priority.is_some());
    assert!(ntfy_req.tags.is_some());

    let disc_req = reqs
        .iter()
        .find(|r| r.path == "/discord/hook")
        .expect("petición Discord");
    let disc_json: Value = serde_json::from_str(&disc_req.body).expect("JSON Discord");
    assert_eq!(disc_json["username"], "Pulse Core");
    assert!(disc_json["embeds"].as_array().unwrap().len() == 1);

    let wh_reqs: Vec<&CapturedHttpRequest> = reqs
        .iter()
        .filter(|r| r.path == "/retry-webhook")
        .collect();
    assert_eq!(wh_reqs.len(), 2, "Debe haber 1 reintento tras el 500");
    assert_eq!(wh_reqs[1].pulse_token.as_deref(), Some("hdr-secret-99"));
    let wh_json: Value = serde_json::from_str(&wh_reqs[1].body).expect("JSON Webhook");
    assert_eq!(wh_json["source"], "pulse-core");
    assert_eq!(wh_json["notification"]["kind"], "test");

    // Verificar estado guardado por canal en getConfig.
    let cfg_view = handle_request(
        &state,
        make_req(6, "notifications.getConfig", None),
        &mut auth,
    )
    .await
    .expect("getConfig")
    .result
    .expect("res getConfig");
    assert!(cfg_view["channelStatuses"]["webhook"]["lastSuccessUnixMs"].is_i64());
    assert!(cfg_view["channelStatuses"]["webhook"]["lastError"].is_null());
}

#[tokio::test]
async fn test_security_ssrf_and_kill_switch_block_external_delivery() {
    let (mock_addr, capture) = spawn_mock_http_server().await;
    let (state, _dir) =
        rpc_state_with_profile("tok-sec", NetworkProfile::PublicEd2kKadPreview);
    let mut auth = true;

    // 1. Perfil público debe rechazar destinos loopback o metadata cloud sin emitir tráfico.
    for target in [
        format!("http://{mock_addr}/ssrf-loopback"),
        "http://169.254.169.254/latest/meta-data".to_string(),
    ] {
        handle_request(
            &state,
            make_req(
                1,
                "notifications.setConfig",
                Some(json!({
                    "externalEnabled": true,
                    "webhook": { "enabled": true, "url": target }
                })),
            ),
            &mut auth,
        )
        .await
        .expect("setConfig ssrf");

        let res = handle_request(
            &state,
            make_req(2, "notifications.test", Some(json!({ "channel": "webhook" }))),
            &mut auth,
        )
        .await
        .expect("test ssrf")
        .result
        .expect("res test ssrf");
        assert_eq!(res["ok"], false);
        assert!(res["error"].as_str().unwrap_or("").contains("prohibited") || res["error"].as_str().unwrap_or("").contains("política"));
    }

    assert!(
        capture.requests.lock().await.is_empty(),
        "Ninguna petición debe haber alcanzado el servidor loopback"
    );

    // 2. En LocalLab pero con interruptor de corte (kill switch) activo -> bloqueado.
    state.network_policy.set_profile(NetworkProfile::LocalLab);
    handle_request(&state, make_req(3, "network.disconnectAll", None), &mut auth)
        .await
        .expect("disconnectAll");

    handle_request(
        &state,
        make_req(
            4,
            "notifications.setConfig",
            Some(json!({
                "externalEnabled": true,
                "ntfy": { "enabled": true, "url": format!("http://{mock_addr}/blocked-by-kill") }
            })),
        ),
        &mut auth,
    )
    .await
    .expect("setConfig kill");

    let kill_res = handle_request(
        &state,
        make_req(5, "notifications.test", Some(json!({ "channel": "ntfy" }))),
        &mut auth,
    )
    .await
    .expect("test kill")
    .result
    .expect("res kill");
    assert_eq!(kill_res["ok"], false);
    assert!(kill_res["error"]
        .as_str()
        .unwrap_or("")
        .contains("interruptor de corte"));
    assert!(capture.requests.lock().await.is_empty());
}

#[tokio::test]
async fn test_monitor_persists_notification_on_download_completed_event() {
    let (state, dir) = rpc_state("tok-mon");
    let monitor_handle = start_notification_monitor(state.clone());

    // Esperar un breve instante para que la tarea del monitor quede suscrita al canal broadcast.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut dl = Download::new(
        "Documental.Prueba.2026.1080p.mkv",
        1_500_000_000,
        Ed2kHash::from_bytes([0x11; 16]),
        None,
        dir.path().join("Documental.Prueba.2026.1080p.mkv"),
        dir.path().join("doc.part"),
    );
    dl.status = DownloadStatus::Completed;
    let snap = mule_daemon::services::downloads::make_snapshot(&dl, &RangeSet::new(), 0, 0);
    state.emit_event(|sequence| DaemonEvent::DownloadCompleted {
        sequence,
        download: snap,
    });

    let mut found = Vec::new();
    for _ in 0..40 {
        {
            let guard = state.downloads.lock().await;
            found = guard
                .db()
                .list_notifications(&NotificationFilter::default())
                .expect("consulta notificaciones");
        }
        if !found.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    monitor_handle.abort();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, "downloadCompleted");
    assert_eq!(found[0].category, "downloads");
    assert!(found[0].body.contains("Documental.Prueba.2026.1080p.mkv"));
    assert!(found[0].body.contains("eD2K"));
}

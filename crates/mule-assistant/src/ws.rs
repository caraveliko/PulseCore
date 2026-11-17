// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Servidor WebSocket loopback que expone la sesión a la interfaz Flutter.
//!
//! Protocolo JSON-RPC mínimo (mensajes de texto, un objeto JSON por trama):
//! - `assistant.hello {token}` — primera llamada obligatoria; mismo token
//!   que la sesión del daemon, el asistente no inventa una autenticación
//!   paralela;
//! - `assistant.chat {text}` → `result: {accepted}` seguido de los eventos
//!   en notificaciones `assistant.event` y el cierre mediante
//!   `assistant.turnEnd`;
//! - `assistant.confirm {allow}` → responde a la notificación
//!   `assistant.confirmRequired` en espera;
//! - `assistant.cancel` → cancela el turno en curso y cierra los
//!   `tool_calls` pendientes en el historial.
//!
//! Una sesión = un historial: las conexiones comparten la misma
//! conversación (decisión deliberada — el asistente es monousuario).

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::error::AssistantError;
use crate::session::Session;

/// Versión del servidor expuesta en `assistant.hello`.
const SERVER_NAME: &str = "mule-assistant";

/// Estado compartido del servidor: sesión única + token de autenticación.
struct WsState {
    session: Arc<Mutex<Session>>,
    expected_token: String,
}

/// Estado de una conexión: autenticación, turno en curso y
/// confirmación en espera de respuesta.
#[derive(Default)]
struct ConnState {
    authenticated: bool,
    turn: Option<JoinHandle<()>>,
    pending_confirm: Option<oneshot::Sender<bool>>,
}

/// Sirve el punto de conexión WS `ws://<addr>/assistant` en el listener proporcionado:
/// la dirección debe ser local (postura de aislamiento loopback). Tomar el
/// listener por parámetro permite el enlace efímero `127.0.0.1:0` en pruebas.
pub async fn serve_ws(
    listener: tokio::net::TcpListener,
    session: Session,
    expected_token: String,
) -> Result<(), AssistantError> {
    let addr = listener
        .local_addr()
        .map_err(|e| AssistantError::Transport(format!("local_addr : {e}")))?;
    if !addr.ip().is_loopback() {
        return Err(AssistantError::Config(format!(
            "bind WS '{addr}' non loopback — refusé par la posture d'isolation"
        )));
    }
    let state = Arc::new(WsState {
        session: Arc::new(Mutex::new(session)),
        expected_token,
    });
    let app = Router::new()
        .route("/assistant", get(ws_upgrade))
        .with_state(state);
    info!("assistant WS en écoute sur ws://{addr}/assistant");
    axum::serve(listener, app)
        .await
        .map_err(|e| AssistantError::Transport(format!("serve WS : {e}")))
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<WsState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<WsState>) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Todo lo que se envía al cliente transita por este canal: el bucle
    // de lectura Y la tarea de turno escriben en él.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        while let Some(v) = out_rx.recv().await {
            if ws_tx
                .send(Message::Text(v.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let conn = Arc::new(Mutex::new(ConnState::default()));

    while let Some(msg) = ws_rx.next().await {
        let Ok(msg) = msg else { break };
        let Message::Text(text) = msg else { continue };
        let req: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                send_error(&out_tx, Value::Null, -32700, &format!("JSON illisible : {e}"));
                continue;
            }
        };
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

        match method {
            "assistant.hello" => handle_hello(&conn, &state, &out_tx, id, &params).await,
            "assistant.chat" => {
                handle_chat(&conn, &state, &out_tx, id, &params).await;
            }
            "assistant.confirm" => handle_confirm(&conn, &out_tx, id, &params).await,
            "assistant.cancel" => handle_cancel(&conn, &state, &out_tx, id).await,
            "" => send_error(&out_tx, id, -32600, "champ 'method' absent"),
            other => send_error(&out_tx, id, -32601, &format!("méthode inconnue : {other}")),
        }
    }

    // Conexión cerrada: cancela el turno en curso y el escritor (writer).
    if let Some(t) = conn.lock().await.turn.take() {
        t.abort();
    }
    writer.abort();
}

/// `assistant.hello`: verifica el token de sesión del daemon.
async fn handle_hello(
    conn: &Arc<Mutex<ConnState>>,
    state: &Arc<WsState>,
    out: &mpsc::UnboundedSender<Value>,
    id: Value,
    params: &Value,
) {
    let token = params.get("token").and_then(|t| t.as_str()).unwrap_or("");
    if token != state.expected_token {
        send_error(out, id, -32001, "jeton invalide");
        return;
    }
    conn.lock().await.authenticated = true;
    let tools = state.session.lock().await.visible_tool_count();
    send_result(
        out,
        id,
        json!({
            "server": SERVER_NAME,
            "version": env!("CARGO_PKG_VERSION"),
            "tools": tools,
        }),
    );
}

/// `assistant.chat`: inicia un turno (solo uno a la vez por conexión).
async fn handle_chat(
    conn: &Arc<Mutex<ConnState>>,
    state: &Arc<WsState>,
    out: &mpsc::UnboundedSender<Value>,
    id: Value,
    params: &Value,
) {
    {
        let c = conn.lock().await;
        if !c.authenticated {
            send_error(out, id, -32002, "authentification requise (assistant.hello)");
            return;
        }
        if c.turn.is_some() {
            send_error(out, id, -32003, "un tour est déjà en cours — annuler d'abord");
            return;
        }
    }
    let text = params
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    if text.trim().is_empty() {
        send_error(out, id, -32602, "paramètre 'text' vide");
        return;
    }

    let session = state.session.clone();
    let out_ev = out.clone();
    let conn_ev = conn.clone();

    let handle = tokio::spawn(async move {
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let (confirm_tx, mut confirm_rx) = mpsc::channel(8);

        let result = {
            let mut sess = session.lock().await;
            let turn = sess.user_turn(&text, &events_tx, &confirm_tx);
            tokio::pin!(turn);
            loop {
                tokio::select! {
                    res = &mut turn => {
                        // El Final sale justo antes de completarse: drenar
                        // los eventos aún en cola antes de terminar.
                        while let Ok(ev) = events_rx.try_recv() {
                            let _ = out_ev.send(json!({
                                "method": "assistant.event",
                                "params": ev,
                            }));
                        }
                        break res;
                    }
                    ev = events_rx.recv() => {
                        let Some(ev) = ev else { continue };
                        let _ = out_ev.send(json!({
                            "method": "assistant.event",
                            "params": ev,
                        }));
                    }
                    req = confirm_rx.recv() => {
                        let Some(r) = req else { continue };
                        let _ = out_ev.send(json!({
                            "method": "assistant.confirmRequired",
                            "params": {
                                "name": r.call.name,
                                "args": r.call.args,
                            },
                        }));
                        conn_ev.lock().await.pending_confirm = Some(r.respond);
                    }
                }
            }
        };

        let _ = out_ev.send(json!({
            "method": "assistant.turnEnd",
            "params": { "error": result.err().map(|e| e.to_string()) },
        }));
    });

    conn.lock().await.turn = Some(handle);
    send_result(out, id, json!({ "accepted": true }));
}

/// `assistant.confirm`: resuelve la confirmación en espera.
async fn handle_confirm(
    conn: &Arc<Mutex<ConnState>>,
    out: &mpsc::UnboundedSender<Value>,
    id: Value,
    params: &Value,
) {
    if !conn.lock().await.authenticated {
        send_error(out, id, -32002, "authentification requise (assistant.hello)");
        return;
    }
    let allow = params.get("allow").and_then(|a| a.as_bool()).unwrap_or(false);
    let responder = conn.lock().await.pending_confirm.take();
    match responder {
        Some(tx) => {
            let _ = tx.send(allow);
            send_result(out, id, json!({ "confirmed": allow }));
        }
        None => send_error(out, id, -32004, "aucune confirmation en attente"),
    }
}

/// `assistant.cancel`: aborta el turno y sella los `tool_calls`
/// pendientes: las API exigen un resultado por llamada solicitada.
async fn handle_cancel(
    conn: &Arc<Mutex<ConnState>>,
    state: &Arc<WsState>,
    out: &mpsc::UnboundedSender<Value>,
    id: Value,
) {
    if !conn.lock().await.authenticated {
        send_error(out, id, -32002, "authentification requise (assistant.hello)");
        return;
    }
    let handle = conn.lock().await.turn.take();
    match handle {
        Some(h) => {
            h.abort();
            state.session.lock().await.seal_cancelled_turn();
            send_result(out, id, json!({ "cancelled": true }));
        }
        None => send_error(out, id, -32005, "aucun tour en cours"),
    }
}

/// Respuesta `result` JSON-RPC.
fn send_result(out: &mpsc::UnboundedSender<Value>, id: Value, result: Value) {
    let _ = out.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
}

/// Respuesta `error` JSON-RPC (códigos alineados con las convenciones -32xxx).
fn send_error(out: &mpsc::UnboundedSender<Value>, id: Value, code: i64, message: &str) {
    warn!("erreur WS {code} : {message}");
    let _ = out.send(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    }));
}

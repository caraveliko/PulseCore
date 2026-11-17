// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Pruebas de extremo a extremo del bucle agéntico: `MockBackend` guionizado +
//! daemon de pruebas loopback (`mule-test-support`). Sin tráfico saliente.

use std::net::SocketAddr;
use std::sync::Arc;

use mule_assistant::config::AssistantConfig;
use mule_assistant::llm::mock::{text_turn, tool_call_turn, MockBackend};
use mule_assistant::llm::{AssistantTurn, Role};
use mule_assistant::session::{AssistantEvent, ConfirmRequest, Session};
use mule_mcp::tools::ToolProfile;
use mule_test_support::spawn_test_daemon;
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::mpsc;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

struct Harness {
    session: Session,
    backend: Arc<MockBackend>,
    addr: SocketAddr,
    events_tx: mpsc::UnboundedSender<AssistantEvent>,
    events_rx: mpsc::UnboundedReceiver<AssistantEvent>,
    confirm_tx: mpsc::Sender<ConfirmRequest>,
    confirm_rx: mpsc::Receiver<ConfirmRequest>,
    _dir: TempDir,
}

async fn build_session(turns: Vec<AssistantTurn>) -> Harness {
    let (addr, token, dir) = spawn_test_daemon(TOKEN).await;
    let backend = Arc::new(MockBackend::new(turns));
    let session = connect(&addr, &token, backend.clone(), AssistantConfig::default()).await;
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let (confirm_tx, confirm_rx) = mpsc::channel(8);
    Harness {
        session,
        backend,
        addr,
        events_tx,
        events_rx,
        confirm_tx,
        confirm_rx,
        _dir: dir,
    }
}

async fn connect(
    addr: &SocketAddr,
    token: &str,
    backend: Arc<MockBackend>,
    config: AssistantConfig,
) -> Session {
    Session::connect(
        Box::new(backend),
        config,
        Some(format!("ws://{addr}/rpc")),
        Some(token.to_string()),
        None,
        None,
    )
    .await
    .expect("connexion session de test")
}

/// Drena todos los eventos emitidos durante el turno.
fn drain_events(rx: &mut mpsc::UnboundedReceiver<AssistantEvent>) -> Vec<AssistantEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

/// Turno de texto simple: el modelo responde sin herramientas → `Final`.
#[tokio::test]
async fn test_simple_text_turn() {
    let mut h = build_session(vec![text_turn("Réponse du modèle")]).await;
    assert!(h.session.visible_tool_count() > 0);

    h.session
        .user_turn("bonjour", &h.events_tx, &h.confirm_tx)
        .await
        .unwrap();

    let events = drain_events(&mut h.events_rx);
    assert!(events.iter().any(|e| matches!(
        e,
        AssistantEvent::Final { content } if content == "Réponse du modèle"
    )));
    assert!(events
        .iter()
        .any(|e| matches!(e, AssistantEvent::Token { content } if content == "Réponse du modèle")));
}

/// Llamada a herramienta: el modelo solicita `emule_status`, el daemon responde,
/// y luego el modelo concluye: el historial devuelto al backend contiene
/// `assistant(tool_calls)` y luego `tool(tool_name)` en orden.
#[tokio::test]
async fn test_tool_call_executed_then_final() {
    let mut h = build_session(vec![
        tool_call_turn("emule_status", json!({})),
        text_turn("Voici l'état du daemon"),
    ])
    .await;

    h.session
        .user_turn("état ?", &h.events_tx, &h.confirm_tx)
        .await
        .unwrap();

    let events = drain_events(&mut h.events_rx);
    assert!(events
        .iter()
        .any(|e| matches!(e, AssistantEvent::ToolStart { name, .. } if name == "emule_status")));
    assert!(events.iter().any(|e| matches!(
        e,
        AssistantEvent::ToolResult { name, is_error, .. } if name == "emule_status" && !is_error
    )));
    assert!(events
        .iter()
        .any(|e| matches!(e, AssistantEvent::Final { content } if content.contains("Voici"))));

    // Historial visto por el backend en la segunda llamada: …, assistant(tool_calls),
    // tool(emule_status): en este orden.
    let history = h.backend.last_history();
    let assistant_idx = history
        .iter()
        .position(|m| m.role == Role::Assistant && !m.tool_calls.is_empty())
        .expect("message assistant avec tool_calls présent");
    let tool_idx = history
        .iter()
        .position(|m| m.role == Role::Tool && m.tool_name.as_deref() == Some("emule_status"))
        .expect("message tool emule_status présent");
    assert!(assistant_idx < tool_idx, "ordre assistant→tool respecté");
}

/// Herramienta fuera de la lista de permitidos: rechazada sin contactar al daemon,
/// el rechazo regresa al modelo como resultado `role: "tool"` (el bucle no se rompe)
/// y la confirmación nunca se solicita.
#[tokio::test]
async fn test_out_of_allowlist_tool_refused() {
    let mut h = build_session(vec![
        tool_call_turn("emule_shutdown", json!({})),
        text_turn("refus expliqué"),
    ])
    .await;

    h.session
        .user_turn("arrête tout", &h.events_tx, &h.confirm_tx)
        .await
        .unwrap();

    let events = drain_events(&mut h.events_rx);
    assert!(events.iter().any(|e| matches!(
        e,
        AssistantEvent::ToolResult { name, is_error, content }
            if name == "emule_shutdown" && *is_error && content.contains("allowlist")
    )));
    // No se emitió ningún ToolStart: la herramienta no se ejecutó.
    assert!(!events
        .iter()
        .any(|e| matches!(e, AssistantEvent::ToolStart { name, .. } if name == "emule_shutdown")));
    // Sin confirmación solicitada: el canal no recibió nada.
    assert!(h.confirm_rx.try_recv().is_err());
    // El bucle continuó hasta el Final.
    assert!(events
        .iter()
        .any(|e| matches!(e, AssistantEvent::Final { .. })));
}

/// Confirmación aceptada: `emule_add_link` se ejecuta tras `o`.
#[tokio::test]
async fn test_confirmed_tool_executes() {
    let mut h = build_session(vec![
        tool_call_turn(
            "emule_add_link",
            json!({ "link": "ed2k://|file|test.bin|1048576|31D6CFE0D16AE931B73C59D7E0C089C0|/" }),
        ),
        text_turn("lien ajouté"),
    ])
    .await;

    let mut confirm_rx = std::mem::replace(&mut h.confirm_rx, mpsc::channel(1).1);
    let responder = tokio::spawn(async move {
        while let Some(req) = confirm_rx.recv().await {
            assert_eq!(req.call.name, "emule_add_link");
            let _ = req.respond.send(true);
        }
    });

    h.session
        .user_turn("ajoute ce lien", &h.events_tx, &h.confirm_tx)
        .await
        .unwrap();
    responder.abort();

    let events = drain_events(&mut h.events_rx);
    assert!(events
        .iter()
        .any(|e| matches!(e, AssistantEvent::ToolStart { name, .. } if name == "emule_add_link")));
    assert!(events.iter().any(|e| matches!(
        e,
        AssistantEvent::ToolResult { name, is_error, .. } if name == "emule_add_link" && !is_error
    )));
}

/// Confirmación rechazada: la herramienta no se ejecuta, el modelo recibe
/// «annulé par l'utilisateur» como resultado de herramienta.
#[tokio::test]
async fn test_denied_tool_not_executed() {
    let mut h = build_session(vec![
        tool_call_turn(
            "emule_add_link",
            json!({ "link": "ed2k://|file|x.bin|1|31D6CFE0D16AE931B73C59D7E0C089C0|/" }),
        ),
        text_turn("compris, rien ajouté"),
    ])
    .await;

    let mut confirm_rx = std::mem::replace(&mut h.confirm_rx, mpsc::channel(1).1);
    let responder = tokio::spawn(async move {
        while let Some(req) = confirm_rx.recv().await {
            let _ = req.respond.send(false);
        }
    });

    h.session
        .user_turn("ajoute", &h.events_tx, &h.confirm_tx)
        .await
        .unwrap();
    responder.abort();

    let events = drain_events(&mut h.events_rx);
    assert!(!events
        .iter()
        .any(|e| matches!(e, AssistantEvent::ToolStart { .. })));
    assert!(events.iter().any(|e| matches!(
        e,
        AssistantEvent::ToolResult { name, is_error, content }
            if name == "emule_add_link" && *is_error && content.contains("annulé")
    )));
}

/// Sin respondedor de confirmación (canal cerrado), una llamada por confirmar
/// se rechaza: comportamiento seguro por defecto en modo no interactivo.
#[tokio::test]
async fn test_confirm_without_responder_refused() {
    let mut h = build_session(vec![
        tool_call_turn("emule_connect_server", json!({})),
        text_turn("impossible"),
    ])
    .await;
    drop(h.confirm_rx); // cierra el canal: no hay respondedor

    h.session
        .user_turn("connecte un serveur", &h.events_tx, &h.confirm_tx)
        .await
        .unwrap();

    let events = drain_events(&mut h.events_rx);
    assert!(events.iter().any(|e| matches!(
        e,
        AssistantEvent::ToolResult { name, is_error, content }
            if name == "emule_connect_server" && *is_error && content.contains("répondeur")
    )));
}

/// El bucle está acotado por `max_tool_rounds`: un modelo que siempre solicita
/// una herramienta se interrumpe con un `Notice`.
#[tokio::test]
async fn test_tool_round_limit() {
    let turns = (0..6)
        .map(|_| tool_call_turn("emule_status", json!({})))
        .collect();
    let mut h = build_session(turns).await;

    // Reconexión con un límite estricto: mismo daemon, mismo backend.
    h.session = connect(
        &h.addr,
        TOKEN,
        h.backend.clone(),
        AssistantConfig {
            max_tool_rounds: 2,
            tool_profile: ToolProfile::Full,
            ..Default::default()
        },
    )
    .await;

    h.session
        .user_turn("encore", &h.events_tx, &h.confirm_tx)
        .await
        .unwrap();

    let events = drain_events(&mut h.events_rx);
    assert!(events
        .iter()
        .any(|e| matches!(e, AssistantEvent::Notice { .. })));
    // 2 rondas × 1 llamada = 2 ToolResult, no más.
    let tool_results = events
        .iter()
        .filter(|e| matches!(e, AssistantEvent::ToolResult { .. }))
        .count();
    assert_eq!(tool_results, 2);
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Pruebas del backend Ollama: validación de endpoint y análisis del flujo
//! NDJSON frente a un servidor loopback simulado. Sin tráfico saliente.

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use mule_assistant::config::AssistantConfig;
use mule_assistant::llm::ollama::OllamaBackend;
use mule_assistant::llm::{ChatMessage, LlmBackend, ToolSchema};
use serde_json::{json, Value};
use tokio::net::TcpListener;

/// Validación del endpoint: se exige http + IP literal en bucle local (loopback).
#[test]
fn test_endpoint_validation() {
    // Correcto: bucle local literal
    let ok = AssistantConfig {
        endpoint: "http://127.0.0.1:11434".to_string(),
        ..Default::default()
    };
    assert!(ok.validate().is_ok());

    // Correcto: IPv6 en bucle local
    let ok6 = AssistantConfig {
        endpoint: "http://[::1]:11434".to_string(),
        ..Default::default()
    };
    assert!(ok6.validate().is_ok());

    // Rechazo: nombre de host a resolver
    let host = AssistantConfig {
        endpoint: "http://localhost:11434".to_string(),
        ..Default::default()
    };
    assert!(host.validate().is_err());

    // Rechazo: no local sin anulación
    let remote = AssistantConfig {
        endpoint: "http://8.8.8.8:11434".to_string(),
        ..Default::default()
    };
    assert!(remote.validate().is_err());

    // Aceptado con anulación explícita
    let remote_allowed = AssistantConfig {
        endpoint: "http://8.8.8.8:11434".to_string(),
        allow_remote_endpoint: true,
        ..Default::default()
    };
    assert!(remote_allowed.validate().is_ok());

    // Rechazo: esquema no http
    let https = AssistantConfig {
        endpoint: "https://127.0.0.1:11434".to_string(),
        ..Default::default()
    };
    assert!(https.validate().is_err());
}

/// Cuerpo NDJSON preparado del falso Ollama: dos fragmentos de contenido, un
/// tool_call y luego el fragmento final `done` con métricas.
fn scripted_ndjson() -> String {
    [
        json!({"model":"m","message":{"role":"assistant","content":"Bon"},"done":false}),
        json!({"model":"m","message":{"role":"assistant","content":"jour","tool_calls":[{"function":{"name":"emule_status","arguments":{}}}]},"done":false}),
        json!({"model":"m","done":true,"done_reason":"stop","load_duration":10_000_000_u64,"prompt_eval_duration":20_000_000_u64,"eval_duration":30_000_000_u64,"eval_count":7}),
    ]
    .iter()
    .map(|v| v.to_string())
    .collect::<Vec<_>>()
    .join("\n")
        + "\n"
}

/// Captura del cuerpo de la petición recibida por el servidor simulado.
type CapturedBody = Arc<Mutex<Option<Value>>>;

async fn fake_chat(State(captured): State<CapturedBody>, Json(body): Json<Value>) -> impl IntoResponse {
    *captured.lock().unwrap() = Some(body);
    axum::response::Response::builder()
        .header("content-type", "application/x-ndjson")
        .body(scripted_ndjson())
        .unwrap()
}

/// Ensambla el turno completo desde el flujo: contenido transmitido al sumidero (sink),
/// tool_calls acumulados y métricas del fragmento `done`.
#[tokio::test]
async fn test_ollama_stream_parsing() {
    let captured: CapturedBody = Arc::new(Mutex::new(None));
    let app = Router::new()
        .route("/api/chat", post(fake_chat))
        .with_state(captured.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let config = AssistantConfig {
        endpoint: format!("http://{addr}"),
        model: "test-model".to_string(),
        ..Default::default()
    };
    let backend = OllamaBackend::new(&config).unwrap();

    // Historial que incluye un mensaje tool: verifica la serialización.
    let history = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("salut"),
        ChatMessage::tool("emule_status", "{\"ok\":true}"),
    ];
    let tools = vec![ToolSchema {
        name: "emule_status".to_string(),
        description: "status".to_string(),
        parameters: json!({"type":"object","properties":{}}),
    }];

    let tokens = Arc::new(Mutex::new(Vec::<String>::new()));
    let tokens2 = tokens.clone();
    let sink = move |t: &str| {
        tokens2.lock().unwrap().push(t.to_string());
        true
    };

    let turn = backend.chat(&history, &tools, &sink).await.unwrap();

    // Contenido concatenado + transmitido en dos fragmentos.
    assert_eq!(turn.content, "Bonjour");
    assert_eq!(tokens.lock().unwrap().clone(), vec!["Bon", "jour"]);

    // Llamada a herramienta acumulada y traducida al formato neutro.
    assert_eq!(turn.tool_calls.len(), 1);
    assert_eq!(turn.tool_calls[0].name, "emule_status");
    assert!(turn.tool_calls[0].id.is_none());

    // Métricas convertidas ns → ms.
    let m = turn.metrics.unwrap();
    assert_eq!(m.load_duration_ms, Some(10));
    assert_eq!(m.prompt_eval_duration_ms, Some(20));
    assert_eq!(m.eval_duration_ms, Some(30));
    assert_eq!(m.eval_count, Some(7));

    // Cuerpo de la petición: tools en formato function, tool_name serializado.
    let sent = captured.lock().unwrap().clone().unwrap();
    assert_eq!(sent["model"], "test-model");
    assert_eq!(sent["stream"], true);
    assert_eq!(sent["tools"][0]["function"]["name"], "emule_status");
    assert_eq!(sent["messages"][0]["role"], "system");
    assert_eq!(sent["messages"][2]["role"], "tool");
    assert_eq!(sent["messages"][2]["tool_name"], "emule_status");
    assert_eq!(sent["messages"][2]["name"], "emule_status");
}

/// Respuesta de error HTTP → `AssistantError::Backend` con el estado.
#[tokio::test]
async fn test_ollama_http_error() {
    async fn err_handler() -> impl IntoResponse {
        (axum::http::StatusCode::NOT_FOUND, "model not found")
    }
    let app = Router::new().route("/api/chat", post(err_handler));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let config = AssistantConfig {
        endpoint: format!("http://{addr}"),
        ..Default::default()
    };
    let backend = OllamaBackend::new(&config).unwrap();
    let res = backend.chat(&[], &[], &|_| true).await;
    let err = res.unwrap_err().to_string();
    assert!(err.contains("404"), "statut HTTP attendu dans l'erreur : {err}");
}

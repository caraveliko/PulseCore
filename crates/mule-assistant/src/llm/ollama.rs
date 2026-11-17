// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Backend nativo de Ollama: `POST {endpoint}/api/chat` en streaming NDJSON.
//!
//! Bloqueo de red (postura de aislamiento loopback del proyecto):
//! - redirecciones HTTP desactivadas (`reqwest` las sigue por defecto);
//! - proxies de entorno desactivados (`HTTP_PROXY`/`HTTPS_PROXY` podrían
//!   desviar el tráfico fuera de loopback);
//! - endpoint validado como IP literal loopback por [`AssistantConfig`];
//! - tiempos de espera acotados en la configuración.
//!
//! Formato de cable: cada fragmento NDJSON contiene `message.content`,
//! `message.thinking` (acumulado, nunca transmitido en flujo) y
//! `message.tool_calls` (fragmentos acumulados — ejecutados únicamente tras
//! su ensamblado completo).

use std::future::Future;
use std::pin::Pin;

use futures_util::StreamExt;
use serde_json::{json, Value};
use tracing::debug;

use crate::config::{AssistantConfig, LLM_CONNECT_TIMEOUT};
use crate::error::AssistantError;
use crate::llm::{
    AssistantTurn, ChatMessage, LlmBackend, LlmMetrics, Role, ToolCall, ToolSchema,
};

/// Las duraciones de Ollama se reportan en nanosegundos.
const NS_PER_MS: u64 = 1_000_000;

/// Backend LLM para la API nativa de Ollama (`/api/chat`).
pub struct OllamaBackend {
    client: reqwest::Client,
    endpoint: String,
    model: String,
}

impl OllamaBackend {
    /// Construye el backend y su cliente HTTP bloqueado.
    pub fn new(config: &AssistantConfig) -> Result<Self, AssistantError> {
        config.validate()?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(LLM_CONNECT_TIMEOUT)
            .timeout(config.llm_timeout)
            .build()
            .map_err(|e| AssistantError::Backend(format!("construction du client HTTP : {e}")))?;
        Ok(Self {
            client,
            endpoint: config.endpoint.trim_end_matches('/').to_string(),
            model: config.model.clone(),
        })
    }
}

impl LlmBackend for OllamaBackend {
    fn chat<'a>(
        &'a self,
        history: &'a [ChatMessage],
        tools: &'a [ToolSchema],
        sink: &'a crate::llm::TokenSink<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AssistantTurn, AssistantError>> + Send + 'a>> {
        Box::pin(async move { self.chat_inner(history, tools, sink).await })
    }
}

impl OllamaBackend {
    async fn chat_inner(
        &self,
        history: &[ChatMessage],
        tools: &[ToolSchema],
        sink: &crate::llm::TokenSink<'_>,
    ) -> Result<AssistantTurn, AssistantError> {
        let body = json!({
            "model": self.model,
            "messages": history.iter().map(to_ollama_message).collect::<Vec<_>>(),
            "tools": tools.iter().map(to_ollama_tool).collect::<Vec<_>>(),
            "stream": true,
        });

        let resp = self
            .client
            .post(format!("{}/api/chat", self.endpoint))
            .json(&body)
            .send()
            .await
            .map_err(|e| AssistantError::Backend(format!("requête Ollama : {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AssistantError::Backend(format!(
                "Ollama HTTP {status} : {}",
                text.chars().take(200).collect::<String>()
            )));
        }

        // Flujo NDJSON: una línea JSON por fragmento (chunk).
        let mut turn = AssistantTurn::default();
        let mut metrics = LlmMetrics::default();
        let mut buffer = String::new();
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk
                .map_err(|e| AssistantError::Backend(format!("stream Ollama : {e}")))?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            // Consume las líneas completas; el resto espera al siguiente fragmento.
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer.drain(..=pos);
                if line.is_empty() {
                    continue;
                }
                let done = consume_chunk(&line, &mut turn, &mut metrics, sink)?;
                if done {
                    turn.metrics = Some(metrics);
                    return Ok(turn);
                }
            }
        }

        // Flujo terminado sin fragmento `done`: se acepta lo que se haya acumulado.
        turn.metrics = Some(metrics);
        Ok(turn)
    }
}

/// Procesa una línea NDJSON: acumula content/thinking/tool_calls y las
/// métricas del fragmento final. Devuelve `true` cuando se alcanza `done`.
fn consume_chunk(
    line: &str,
    turn: &mut AssistantTurn,
    metrics: &mut LlmMetrics,
    sink: &crate::llm::TokenSink<'_>,
) -> Result<bool, AssistantError> {
    let chunk: Value = serde_json::from_str(line)
        .map_err(|e| AssistantError::Backend(format!("chunk Ollama illisible : {e}")))?;

    if let Some(msg) = chunk.get("message") {
        if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
            if !content.is_empty() {
                turn.content.push_str(content);
                if !sink(content) {
                    debug!("sink LLM fermé — streaming interrompu");
                }
            }
        }
        if let Some(thinking) = msg.get("thinking").and_then(|t| t.as_str()) {
            turn.thinking.push_str(thinking);
        }
        if let Some(calls) = msg.get("tool_calls").and_then(|c| c.as_array()) {
            for c in calls {
                if let Some(call) = parse_tool_call(c) {
                    turn.tool_calls.push(call);
                }
            }
        }
    }

    let done = chunk.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
    if done {
        if let Some(reason) = chunk.get("done_reason").and_then(|r| r.as_str()) {
            if reason != "stop" {
                debug!("done_reason Ollama : {reason}");
            }
        }
        metrics.load_duration_ms = chunk
            .get("load_duration")
            .and_then(|v| v.as_u64())
            .map(|ns| ns / NS_PER_MS);
        metrics.prompt_eval_duration_ms = chunk
            .get("prompt_eval_duration")
            .and_then(|v| v.as_u64())
            .map(|ns| ns / NS_PER_MS);
        metrics.eval_duration_ms = chunk
            .get("eval_duration")
            .and_then(|v| v.as_u64())
            .map(|ns| ns / NS_PER_MS);
        metrics.eval_count = chunk.get("eval_count").and_then(|v| v.as_u64());
    }
    Ok(done)
}

/// `{"function":{"name","arguments"}}` → [`ToolCall`] (`id` ausente en
/// Ollama nativo; conservado si alguna vez estuviera presente).
fn parse_tool_call(raw: &Value) -> Option<ToolCall> {
    let func = raw.get("function")?;
    let name = func.get("name")?.as_str()?.to_string();
    let args = func.get("arguments").cloned().unwrap_or_else(|| json!({}));
    Some(ToolCall {
        id: raw.get("id").and_then(|i| i.as_str()).map(String::from),
        name,
        args,
    })
}

/// Serialización en cable de un mensaje: `role`, `content`, `tool_calls`
/// en formato de función de Ollama, y `tool_name` + `name` en los resultados
/// de herramientas (ambas claves se envían para cubrir las versiones
/// de Ollama que esperan una u otra).
fn to_ollama_message(msg: &ChatMessage) -> Value {
    let mut out = json!({
        "role": match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        },
        "content": msg.content,
    });
    if !msg.tool_calls.is_empty() {
        out["tool_calls"] = msg
            .tool_calls
            .iter()
            .map(|c| {
                json!({
                    "function": {
                        "name": c.name,
                        "arguments": c.args,
                    }
                })
            })
            .collect();
    }
    if let Some(name) = &msg.tool_name {
        out["tool_name"] = json!(name);
        out["name"] = json!(name);
    }
    out
}

/// Serialización en cable de una herramienta: `{type:"function", function:{…}}`.
fn to_ollama_tool(tool: &ToolSchema) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        }
    })
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Backend compatible con la API de OpenAI / LM Studio / LocalAI:
//! `POST {endpoint}/v1/chat/completions` en streaming Server-Sent Events (SSE).
//!
//! Bloqueo de red (postura de aislamiento loopback del proyecto):
//! - redirecciones HTTP desactivadas;
//! - proxies de entorno desactivados;
//! - endpoint validado como IP literal loopback por [`AssistantConfig`] a menos
//!   que se habilite explícitamente `allow_remote_endpoint`.
//! - streaming SSE con recopilación de llamadas a herramientas (`tool_calls`).

use std::future::Future;
use std::pin::Pin;

use futures_util::StreamExt;
use serde_json::{json, Value};
use tracing::debug;

use crate::config::{AssistantConfig, LLM_CONNECT_TIMEOUT};
use crate::error::AssistantError;
use crate::llm::{
    AssistantTurn, ChatMessage, LlmBackend, Role, ToolCall, ToolSchema,
};

/// Backend LLM para APIs compatibles con OpenAI (`/v1/chat/completions`).
pub struct OpenAiCompatibleBackend {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    api_key: Option<String>,
}

impl OpenAiCompatibleBackend {
    /// Construye el backend y su cliente HTTP bloqueado.
    pub fn new(config: &AssistantConfig, api_key: Option<String>) -> Result<Self, AssistantError> {
        config.validate()?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(LLM_CONNECT_TIMEOUT)
            .timeout(config.llm_timeout)
            .build()
            .map_err(|e| AssistantError::Backend(format!("construcción del cliente HTTP: {e}")))?;
        Ok(Self {
            client,
            endpoint: config.endpoint.trim_end_matches('/').to_string(),
            model: config.model.clone(),
            api_key,
        })
    }
}

impl LlmBackend for OpenAiCompatibleBackend {
    fn chat<'a>(
        &'a self,
        history: &'a [ChatMessage],
        tools: &'a [ToolSchema],
        sink: &'a crate::llm::TokenSink<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AssistantTurn, AssistantError>> + Send + 'a>> {
        Box::pin(async move { self.chat_inner(history, tools, sink).await })
    }
}

impl OpenAiCompatibleBackend {
    async fn chat_inner(
        &self,
        history: &[ChatMessage],
        tools: &[ToolSchema],
        sink: &crate::llm::TokenSink<'_>,
    ) -> Result<AssistantTurn, AssistantError> {
        let tools_payload: Vec<Value> = tools.iter().map(to_openai_tool).collect();

        let mut body = json!({
            "model": self.model,
            "messages": history.iter().map(to_openai_message).collect::<Vec<_>>(),
            "stream": true,
        });

        if !tools_payload.is_empty() {
            body["tools"] = json!(tools_payload);
        }

        let url = if self.endpoint.ends_with("/v1") {
            format!("{}/chat/completions", self.endpoint)
        } else if self.endpoint.ends_with("/chat/completions") {
            self.endpoint.clone()
        } else {
            format!("{}/v1/chat/completions", self.endpoint)
        };

        let mut req = self.client.post(&url).json(&body);
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| AssistantError::Backend(format!("petición OpenAI-compatible: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AssistantError::Backend(format!(
                "OpenAI HTTP {status}: {}",
                text.chars().take(200).collect::<String>()
            )));
        }

        let mut turn = AssistantTurn::default();
        let mut buffer = String::new();
        let mut stream = resp.bytes_stream();

        // Estructura temporal para ir acumulando fragmentos de llamadas a herramientas por índice
        let mut in_flight_tools: Vec<(Option<String>, String, String)> = Vec::new();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk
                .map_err(|e| AssistantError::Backend(format!("stream SSE OpenAI: {e}")))?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer.drain(..=pos);
                if line.is_empty() {
                    continue;
                }

                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if data == "[DONE]" {
                        assemble_tool_calls(&mut turn, in_flight_tools);
                        return Ok(turn);
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        consume_openai_chunk(&v, &mut turn, &mut in_flight_tools, sink);
                    }
                }
            }
        }

        assemble_tool_calls(&mut turn, in_flight_tools);
        Ok(turn)
    }
}

fn assemble_tool_calls(turn: &mut AssistantTurn, in_flight: Vec<(Option<String>, String, String)>) {
    for (id, name, args_raw) in in_flight {
        let args = serde_json::from_str::<Value>(&args_raw).unwrap_or(json!({}));
        turn.tool_calls.push(ToolCall { id, name, args });
    }
}

fn consume_openai_chunk(
    chunk: &Value,
    turn: &mut AssistantTurn,
    in_flight: &mut Vec<(Option<String>, String, String)>,
    sink: &crate::llm::TokenSink<'_>,
) {
    let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) else {
        return;
    };
    for choice in choices {
        let Some(delta) = choice.get("delta") else { continue };

        if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
            if !content.is_empty() {
                turn.content.push_str(content);
                if !sink(content) {
                    debug!("sink LLM cerrado — streaming interrumpido");
                }
            }
        }

        if let Some(calls) = delta.get("tool_calls").and_then(|tc| tc.as_array()) {
            for c in calls {
                let idx = c.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                while in_flight.len() <= idx {
                    in_flight.push((None, String::new(), String::new()));
                }

                if let Some(id) = c.get("id").and_then(|i| i.as_str()) {
                    in_flight[idx].0 = Some(id.to_string());
                }

                if let Some(func) = c.get("function") {
                    if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                        in_flight[idx].1.push_str(name);
                    }
                    if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                        in_flight[idx].2.push_str(args);
                    }
                }
            }
        }
    }
}

fn to_openai_message(m: &ChatMessage) -> Value {
    match m.role {
        Role::System => json!({ "role": "system", "content": m.content }),
        Role::User => json!({ "role": "user", "content": m.content }),
        Role::Assistant => {
            if m.tool_calls.is_empty() {
                json!({ "role": "assistant", "content": m.content })
            } else {
                let tool_calls: Vec<Value> = m
                    .tool_calls
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        json!({
                            "id": c.id.clone().unwrap_or_else(|| format!("call_{i}")),
                            "type": "function",
                            "function": {
                                "name": c.name,
                                "arguments": serde_json::to_string(&c.args).unwrap_or_default(),
                            }
                        })
                    })
                    .collect();
                json!({
                    "role": "assistant",
                    "content": if m.content.is_empty() { Value::Null } else { json!(m.content) },
                    "tool_calls": tool_calls,
                })
            }
        }
        Role::Tool => {
            json!({
                "role": "tool",
                "tool_call_id": m.tool_name.clone().unwrap_or_else(|| "call_0".to_string()),
                "content": m.content,
            })
        }
    }
}

fn to_openai_tool(schema: &ToolSchema) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": schema.name,
            "description": schema.description,
            "parameters": schema.parameters,
        }
    })
}

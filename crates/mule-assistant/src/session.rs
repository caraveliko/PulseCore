// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Sesión del asistente: bucle acotado «modelo → herramientas → modelo».
//!
//! Invariantes:
//! - el mensaje `Assistant` completo (con sus `tool_calls`) se inserta en
//!   el historial ANTES que los resultados — requisito de las API nativas;
//! - los `tool_calls` recibidos en flujo se acumulan y luego se validan con
//!   [`crate::policy::check_call`] ANTES de cualquier ejecución;
//! - los errores de herramientas no rompen el bucle: se devuelven al modelo
//!   como mensajes `role: "tool"`;
//! - la interfaz de usuario recibe el resultado completo mediante
//!   [`AssistantEvent::ToolResult`] y el modelo la versión compactada
//!   ([`crate::tools_bridge`]).

use std::path::PathBuf;

use mule_mcp::tools::execute_tool;
use mule_mcp::DaemonClient;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

use crate::config::{AssistantConfig, SYSTEM_PROMPT};
use crate::error::AssistantError;
use crate::llm::{ChatMessage, LlmBackend, Role, ToolCall, ToolSchema};
use crate::policy;
use crate::tools_bridge;

/// Eventos emitidos por la sesión hacia el transporte (REPL, WebSocket):
/// serializados tal cual en notificaciones `assistant.event` en el WS.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AssistantEvent {
    /// Fragmento de texto generado por el modelo (streaming).
    Token {
        /// Fragmento recibido.
        content: String,
    },
    /// Inicio de ejecución de una herramienta: emitido inmediatamente tras
    /// la validación, sin esperar al fin de la llamada RPC.
    ToolStart {
        /// Nombre de la herramienta.
        name: String,
        /// Argumentos exactos transmitidos al daemon.
        args: Value,
    },
    /// Resultado completo de una herramienta (el modelo recibe la versión truncada).
    ToolResult {
        /// Nombre de la herramienta.
        name: String,
        /// Verdadero si la llamada falló o fue rechazada/cancelada.
        is_error: bool,
        /// Contenido íntegro del resultado.
        content: String,
    },
    /// Respuesta textual final del turno.
    Final {
        /// Respuesta completa (tras ensamblar los tokens).
        content: String,
    },
    /// Información de fin de turno sin respuesta (límite de rondas…).
    Notice {
        /// Mensaje explicativo.
        message: String,
    },
    /// Error fatal del turno (backend, daemon).
    Error {
        /// Mensaje de error.
        message: String,
    },
}

/// Solicitud de confirmación remitida al transporte: el usuario decide
/// sobre los argumentos exactos de la llamada.
pub struct ConfirmRequest {
    /// Llamada propuesta por el modelo (nombre + argumentos mostrados tal cual).
    pub call: ToolCall,
    /// Canal de respuesta: `true` = ejecutar, `false`/`closed` = rechazar.
    pub respond: oneshot::Sender<bool>,
}

/// Sesión de conversación: historial, herramientas visibles y cliente
/// del daemon. Una sesión = un historial; el transporte (REPL/WS) solo
/// encamina entradas, eventos y confirmaciones.
pub struct Session {
    daemon: DaemonClient,
    backend: Box<dyn LlmBackend>,
    config: AssistantConfig,
    capabilities: Vec<String>,
    tools: Vec<ToolSchema>,
    history: Vec<ChatMessage>,
}

impl Session {
    /// Conexión al daemon y posterior construcción de la superficie de herramientas:
    /// las capacidades se leen mediante `system.status` (el `system.hello`
    /// interno de `DaemonClient` no conserva su respuesta).
    pub async fn connect(
        backend: Box<dyn LlmBackend>,
        config: AssistantConfig,
        url: Option<String>,
        token: Option<String>,
        workdir: Option<PathBuf>,
        app: Option<PathBuf>,
    ) -> Result<Self, AssistantError> {
        config.validate()?;
        let daemon = DaemonClient::connect(url, token, workdir, app)
            .await
            .map_err(AssistantError::Daemon)?;

        let capabilities = fetch_capabilities(&daemon).await;
        let tools = policy::visible_tools(config.tool_profile, &capabilities)
            .iter()
            .map(tools_bridge::from_tool_definition)
            .collect();

        Ok(Self {
            daemon,
            backend,
            config,
            capabilities,
            tools,
            history: vec![ChatMessage::system(SYSTEM_PROMPT)],
        })
    }

    /// Cantidad de herramientas presentadas actualmente al modelo (diagnóstico).
    pub fn visible_tool_count(&self) -> usize {
        self.tools.len()
    }

    /// Sella los `tool_calls` que quedaron sin resultado tras la cancelación
    /// de un turno: las API exigen un mensaje `tool` por cada llamada solicitada;
    /// de lo contrario, el historial conservaría llamadas pendientes que el modelo
    /// podría reproducir o rechazar.
    pub fn seal_cancelled_turn(&mut self) {
        let Some(idx) = self
            .history
            .iter()
            .rposition(|m| m.role == Role::Assistant && !m.tool_calls.is_empty())
        else {
            return;
        };
        // Ejecución secuencial: los mensajes Tool posteriores contabilizan
        // las llamadas ya respondidas; el resto queda pendiente.
        let answered = self.history[idx + 1..]
            .iter()
            .filter(|m| m.role == Role::Tool)
            .count();
        let pending: Vec<String> = self.history[idx]
            .tool_calls
            .iter()
            .skip(answered)
            .map(|c| c.name.clone())
            .collect();
        for name in pending {
            self.history
                .push(ChatMessage::tool(name, "annulé par l'utilisateur"));
        }
    }

    /// Actualiza las capacidades y herramientas visibles; llamar tras una
    /// reconexión o cambio de variante (flavor) del daemon.
    pub async fn refresh_capabilities(&mut self) {
        self.capabilities = fetch_capabilities(&self.daemon).await;
        self.tools = policy::visible_tools(self.config.tool_profile, &self.capabilities)
            .iter()
            .map(tools_bridge::from_tool_definition)
            .collect();
    }

    /// Un turno de usuario completo, acotado por `config.max_tool_rounds`.
    ///
    /// `events` recibe tokens, inicios/fines de herramientas y respuesta final;
    /// `confirms` recibe las solicitudes de confirmación: sin respondedor
    /// (canal cerrado), cualquier herramienta por confirmar se rechaza (por defecto seguro).
    pub async fn user_turn(
        &mut self,
        input: &str,
        events: &mpsc::UnboundedSender<AssistantEvent>,
        confirms: &mpsc::Sender<ConfirmRequest>,
    ) -> Result<(), AssistantError> {
        self.history.push(ChatMessage::user(input));

        let events_sink = events.clone();
        let sink = move |token: &str| -> bool {
            events_sink
                .send(AssistantEvent::Token {
                    content: token.to_string(),
                })
                .is_ok()
        };

        for _round in 0..self.config.max_tool_rounds {
            let turn = self
                .backend
                .chat(&self.history, &self.tools, &sink)
                .await?;

            if let Some(m) = &turn.metrics {
                debug!(
                    load_ms = m.load_duration_ms,
                    prompt_eval_ms = m.prompt_eval_duration_ms,
                    eval_ms = m.eval_duration_ms,
                    eval_count = m.eval_count,
                    "métriques backend LLM"
                );
            }

            // Historial: el mensaje del asistente completo ANTES de los
            // resultados de herramientas (requisito de las API nativas).
            self.history.push(ChatMessage::assistant(&turn));

            if turn.tool_calls.is_empty() {
                let _ = events.send(AssistantEvent::Final {
                    content: turn.content.clone(),
                });
                return Ok(());
            }

            for call in turn.tool_calls {
                self.execute_one_call(call, events, confirms).await;
            }
        }

        let _ = events.send(AssistantEvent::Notice {
            message: format!(
                "limite de {} tours outils atteinte — réponse interrompue",
                self.config.max_tool_rounds
            ),
        });
        Ok(())
    }

    /// Ejecuta una llamada a herramienta: validación de política, confirmación
    /// eventual, RPC al daemon y apilado del resultado en el historial.
    async fn execute_one_call(
        &mut self,
        call: ToolCall,
        events: &mpsc::UnboundedSender<AssistantEvent>,
        confirms: &mpsc::Sender<ConfirmRequest>,
    ) {
        // Validación recomprobada justo antes de la ejecución.
        if let Err(e) = policy::check_call(self.config.tool_profile, &self.capabilities, &call.name)
        {
            self.push_tool_outcome(&call.name, true, &e.to_string(), events);
            return;
        }

        // Confirmación del usuario sobre los argumentos exactos.
        if policy::requires_confirmation(&call.name) {
            let (tx, rx) = oneshot::channel();
            if confirms
                .send(ConfirmRequest {
                    call: call.clone(),
                    respond: tx,
                })
                .await
                .is_err()
            {
                self.push_tool_outcome(
                    &call.name,
                    true,
                    "refusé : aucun répondeur de confirmation actif",
                    events,
                );
                return;
            }
            match rx.await {
                Ok(true) => {}
                _ => {
                    self.push_tool_outcome(
                        &call.name,
                        true,
                        "annulé par l'utilisateur",
                        events,
                    );
                    return;
                }
            }
        }

        let _ = events.send(AssistantEvent::ToolStart {
            name: call.name.clone(),
            args: call.args.clone(),
        });

        let result = execute_tool(&self.daemon, &call.name, call.args.clone()).await;
        let full_text = result
            .content
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let _ = events.send(AssistantEvent::ToolResult {
            name: call.name.clone(),
            is_error: result.is_error,
            content: full_text,
        });

        // El modelo recibe la versión compactada/truncada.
        let compacted = tools_bridge::compact_tool_result(&result, self.config.tool_result_max_chars);
        self.history.push(ChatMessage::tool(&call.name, compacted));
    }

    /// Apila un resultado de herramienta rechazado/cancelado y emite el evento
    /// correspondiente: el modelo ve el rechazo como un resultado, el bucle
    /// no se rompe.
    fn push_tool_outcome(
        &mut self,
        tool_name: &str,
        is_error: bool,
        message: &str,
        events: &mpsc::UnboundedSender<AssistantEvent>,
    ) {
        let _ = events.send(AssistantEvent::ToolResult {
            name: tool_name.to_string(),
            is_error,
            content: message.to_string(),
        });
        self.history.push(ChatMessage::tool(tool_name, message));
    }
}

/// Lee las capacidades del daemon mediante `system.status`: en caso de fallo,
/// conjunto vacío (las herramientas con capacidad requerida se ocultarán, por
/// defecto seguro) y aviso registrado.
async fn fetch_capabilities(daemon: &DaemonClient) -> Vec<String> {
    match daemon.call("system.status", json!({})).await {
        Ok(res) => res
            .get("capabilities")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        Err(e) => {
            warn!("lecture des capacités daemon impossible : {e}");
            Vec::new()
        }
    }
}

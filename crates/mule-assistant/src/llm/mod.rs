// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Abstracción del backend LLM y tipos de la conversación.
//!
//! El trait [`LlmBackend`] es intencionadamente minimalista: recibe el
//! historial y los esquemas de herramientas, envía los tokens a través de
//! `sink` sobre la marcha y devuelve el turno completo — `tool_calls`
//! acumulados, validables ANTES de cualquier ejecución. Cada backend traduce
//! el formato de cable de su API a estos tipos neutros.

pub mod mock;
pub mod ollama;
pub mod openai;

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AssistantError;

/// Rol de un mensaje de conversación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Directrices iniciales del sistema.
    System,
    /// Entrada del usuario.
    User,
    /// Respuesta del modelo (texto y/o llamadas a herramientas).
    Assistant,
    /// Resultado de una herramienta devuelto al modelo.
    Tool,
}

/// Llamada a herramienta emitida por el modelo.
///
/// `id` es **opcional**: la API nativa de Ollama no exige
/// identificador de llamada a diferencia del formato OpenAI; cada
/// backend conserva su identificador eventual sin presuponerlo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Identificador eventual de llamada (OpenAI); ausente en Ollama nativo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Nombre de la herramienta (`emule_*`).
    pub name: String,
    /// Argumentos JSON propuestos por el modelo.
    #[serde(default)]
    pub args: Value,
}

/// Mensaje de historial intercambiado con el backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Rol del emisor.
    pub role: Role,
    /// Contenido textual (vacío en un turno de llamadas puras).
    #[serde(default)]
    pub content: String,
    /// Llamadas a herramientas solicitadas: presentes únicamente en `Assistant`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Nombre de la herramienta cuyo resultado porta este mensaje: únicamente en `Tool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl ChatMessage {
    /// Mensaje del sistema (directrices).
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_name: None,
        }
    }

    /// Entrada del usuario.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_name: None,
        }
    }

    /// Respuesta del modelo: agregada al historial ANTES de los resultados
    /// de herramientas, completa con sus `tool_calls` (requisito de Ollama/OpenAI).
    pub fn assistant(turn: &AssistantTurn) -> Self {
        Self {
            role: Role::Assistant,
            content: turn.content.clone(),
            tool_calls: turn.tool_calls.clone(),
            tool_name: None,
        }
    }

    /// Resultado de herramienta: `tool_name` identifica la herramienta en Ollama.
    pub fn tool(tool_name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_name: Some(tool_name.into()),
        }
    }
}

/// Turno de respuesta completo del modelo.
#[derive(Debug, Clone, Default)]
pub struct AssistantTurn {
    /// Texto destinado al usuario.
    pub content: String,
    /// Llamadas a herramientas solicitadas: vacío = respuesta final.
    pub tool_calls: Vec<ToolCall>,
    /// Contenido de pensamiento («thinking») de los modelos que lo emiten: recopilado
    /// para diagnóstico pero nunca mostrado al usuario por defecto.
    pub thinking: String,
    /// Métricas reportadas por el backend (tiempos de Ollama…).
    pub metrics: Option<LlmMetrics>,
}

/// Métricas de inferencia reportadas por el backend (ms / tokens).
#[derive(Debug, Clone, Default)]
pub struct LlmMetrics {
    /// Duración de la carga del modelo.
    pub load_duration_ms: Option<u64>,
    /// Duración de la evaluación del prompt.
    pub prompt_eval_duration_ms: Option<u64>,
    /// Duración de la generación.
    pub eval_duration_ms: Option<u64>,
    /// Cantidad de tokens generados.
    pub eval_count: Option<u64>,
}

/// Esquema de herramienta presentado al modelo: formato neutro que cada backend
/// serializa a su manera (Ollama `{type:"function", function:{…}}`).
#[derive(Debug, Clone)]
pub struct ToolSchema {
    /// Nombre de la herramienta.
    pub name: String,
    /// Descripción funcional.
    pub description: String,
    /// JSON Schema de los parámetros.
    pub parameters: Value,
}

/// Sumidero (sink) de tokens: devuelve `false` cuando el consumidor se ha desconectado;
/// el backend debe entonces interrumpir el streaming cuanto antes.
pub type TokenSink<'a> = dyn Fn(&str) -> bool + Send + Sync + 'a;

/// Backend de inferencia intercambiable (Ollama, mock de pruebas…).
///
/// Seguridad de objetos (object-safety): el future se empaqueta en Box para permitir
/// `Box<dyn LlmBackend>`; el backend se selecciona en tiempo de ejecución según la
/// configuración.
pub trait LlmBackend: Send + Sync {
    /// Un turno de conversación: transmitir `content` en flujo hacia `sink`, acumular
    /// `tool_calls` y devolver el turno completo.
    fn chat<'a>(
        &'a self,
        history: &'a [ChatMessage],
        tools: &'a [ToolSchema],
        sink: &'a TokenSink<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AssistantTurn, AssistantError>> + Send + 'a>>;
}

/// Delegación para `Arc<T>`: permite a las pruebas conservar un manejador
/// del backend tras inyectarlo en la sesión (`Box<dyn>`).
impl<T: LlmBackend + ?Sized> LlmBackend for std::sync::Arc<T> {
    fn chat<'a>(
        &'a self,
        history: &'a [ChatMessage],
        tools: &'a [ToolSchema],
        sink: &'a TokenSink<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AssistantTurn, AssistantError>> + Send + 'a>> {
        (**self).chat(history, tools, sink)
    }
}

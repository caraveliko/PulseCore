// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Backend de pruebas guionizado: sin E/S, conforme a la regla
//! «solo pruebas sin conexión» de AGENTS.md.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use crate::error::AssistantError;
use crate::llm::{
    AssistantTurn, ChatMessage, LlmBackend, LlmMetrics, TokenSink, ToolSchema,
};

/// Backend LLM de extracción de cola: cada llamada a [`LlmBackend::chat`]
/// consume el siguiente [`AssistantTurn`] preparado y envía su contenido
/// a través del sumidero (sink). Cola vacía = turno vacío (respuesta final sin texto).
pub struct MockBackend {
    turns: Mutex<VecDeque<AssistantTurn>>,
    /// Último historial recibido: para aserciones de prueba.
    last_history: Mutex<Vec<ChatMessage>>,
}

impl MockBackend {
    /// Construye el backend con la secuencia de turnos a reproducir.
    pub fn new(turns: Vec<AssistantTurn>) -> Self {
        Self {
            turns: Mutex::new(turns.into()),
            last_history: Mutex::new(Vec::new()),
        }
    }

    /// Copia del último historial recibido (aserciones de prueba).
    pub fn last_history(&self) -> Vec<ChatMessage> {
        self.last_history.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl LlmBackend for MockBackend {
    fn chat<'a>(
        &'a self,
        history: &'a [ChatMessage],
        _tools: &'a [ToolSchema],
        sink: &'a TokenSink<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AssistantTurn, AssistantError>> + Send + 'a>> {
        Box::pin(async move {
            *self.last_history.lock().unwrap_or_else(|e| e.into_inner()) = history.to_vec();
            let turn = self
                .turns
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop_front()
                .unwrap_or_default();
            if !turn.content.is_empty() {
                let _ = sink(&turn.content);
            }
            Ok(turn)
        })
    }
}

/// Fabrica un turno de «texto simple» para las pruebas.
pub fn text_turn(content: &str) -> AssistantTurn {
    AssistantTurn {
        content: content.to_string(),
        ..Default::default()
    }
}

/// Fabrica un turno de «llamada a herramienta» para las pruebas.
pub fn tool_call_turn(name: &str, args: serde_json::Value) -> AssistantTurn {
    AssistantTurn {
        tool_calls: vec![crate::llm::ToolCall {
            id: None,
            name: name.to_string(),
            args,
        }],
        ..Default::default()
    }
}

/// Métricas simuladas para las pruebas.
pub fn fake_metrics() -> Option<LlmMetrics> {
    Some(LlmMetrics {
        load_duration_ms: Some(10),
        prompt_eval_duration_ms: Some(20),
        eval_duration_ms: Some(30),
        eval_count: Some(5),
    })
}

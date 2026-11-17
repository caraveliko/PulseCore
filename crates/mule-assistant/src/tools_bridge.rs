// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Puente entre las `ToolDefinition` MCP y los esquemas neutros del backend.
//!
//! Sin redefinición de esquema: los `inputSchema` JSON Schema ya
//! construidos por `mule-mcp` se reutilizan tal cual — tienen el formato
//! esperado por el tool calling de Ollama/OpenAI.

use mule_mcp::protocol::{ToolCallResult, ToolDefinition};
use serde_json::Value;

use crate::llm::ToolSchema;

/// Conversión `ToolDefinition` MCP → esquema neutro para el modelo.
pub fn from_tool_definition(def: &ToolDefinition) -> ToolSchema {
    ToolSchema {
        name: def.name.clone(),
        description: def.description.clone(),
        parameters: def.input_schema.clone(),
    }
}

/// Representación compacta de un resultado de herramienta para el contexto del modelo:
/// JSON minificado (el `ToolCallResult::json` MCP está formateado con sangría, demasiado
/// costoso en tokens) y luego truncado a `max_chars` con el total real
/// indicado. La interfaz de usuario recibe la versión completa mediante [`crate::AssistantEvent`].
pub fn compact_tool_result(result: &ToolCallResult, max_chars: usize) -> String {
    let raw = result
        .content
        .iter()
        .map(|c| c.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    // Minifica únicamente si el contenido es JSON válido; de lo contrario, el
    // texto sin formato se devuelve tal cual (mensajes de error MCP).
    let compact = serde_json::from_str::<Value>(&raw)
        .map(|v| v.to_string())
        .unwrap_or(raw);

    let char_count = compact.chars().count();
    if char_count <= max_chars {
        compact
    } else {
        let truncated: String = compact.chars().take(max_chars).collect();
        format!("{truncated}… [tronqué pour le modèle — {char_count} caractères au total]")
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

pub mod downloads;
pub mod kad;
pub mod notifications;
pub mod policy;
pub mod servers;
pub mod shares;
pub mod system;

use crate::client::DaemonClient;
use crate::protocol::{ToolCallResult, ToolDefinition};
use serde_json::Value;
use tracing::info;

pub use downloads::resolve_download_id;
pub use policy::{classify, ToolProfile, ToolRisk};

/// Propiedad de esquema simple `{ "type": ty, "description": desc }`.
///
/// Fuente única del boilerplate repetido en las ~87 definiciones de herramientas;
/// las propiedades complejas (enum, items, default…) permanecen como `json!` puro.
pub(crate) fn prop(ty: &str, desc: &str) -> Value {
    serde_json::json!({ "type": ty, "description": desc })
}

/// Esquema de entrada de una herramienta: `{"type":"object","properties":…,"required":…}`.
/// `required` se omite cuando la lista está vacía.
pub(crate) fn obj_schema(properties: Value, required: &[&str]) -> Value {
    let mut schema = serde_json::json!({ "type": "object", "properties": properties });
    if !required.is_empty() {
        schema["required"] = serde_json::json!(required);
    }
    schema
}

pub fn list_tools() -> Vec<ToolDefinition> {
    let mut tools = Vec::with_capacity(96);
    tools.extend(system::list_tools());
    tools.extend(downloads::list_tools());
    tools.extend(servers::list_tools());
    tools.extend(shares::list_tools());
    tools.extend(kad::list_tools());
    tools.extend(notifications::list_tools());
    tools
}

/// Variante de [`list_tools`] filtrada por perfil: las herramientas fuera del perfil
/// — o no clasificadas — se ocultan. El perfil `Full` reproduce `list_tools`.
pub fn list_tools_for(profile: ToolProfile) -> Vec<ToolDefinition> {
    list_tools()
        .into_iter()
        .filter(|t| {
            classify(&t.name)
                .map(|risk| profile.allows(risk))
                .unwrap_or(false)
        })
        .collect()
}

/// Variante de [`execute_tool`] que aplica el perfil antes de cualquier llamada RPC:
/// una herramienta fuera del perfil es rechazada sin contactar con el daemon. Filtrar
/// `tools/list` solo es cosmético — un cliente puede invocar por su nombre
/// una herramienta que no figure en la lista.
pub async fn execute_tool_for(
    profile: ToolProfile,
    client: &DaemonClient,
    tool_name: &str,
    args: Value,
) -> ToolCallResult {
    match classify(tool_name) {
        Some(risk) if profile.allows(risk) => execute_tool(client, tool_name, args).await,
        Some(_) => ToolCallResult::error(format!(
            "Tool '{tool_name}' blocked by profile '{}'",
            profile.as_str()
        )),
        None => ToolCallResult::error(format!("Unknown tool: {tool_name}")),
    }
}

pub async fn execute_tool(client: &DaemonClient, tool_name: &str, args: Value) -> ToolCallResult {
    info!("Executing MCP tool: {tool_name} with args: {args}");

    if let Some(res) = system::handle_tool(client, tool_name, &args).await {
        return res;
    }
    if let Some(res) = downloads::handle_tool(client, tool_name, &args).await {
        return res;
    }
    if let Some(res) = servers::handle_tool(client, tool_name, &args).await {
        return res;
    }
    if let Some(res) = shares::handle_tool(client, tool_name, &args).await {
        return res;
    }
    if let Some(res) = kad::handle_tool(client, tool_name, &args).await {
        return res;
    }
    if let Some(res) = notifications::handle_tool(client, tool_name, &args).await {
        return res;
    }

    ToolCallResult::error(format!("Unknown tool: {tool_name}"))
}

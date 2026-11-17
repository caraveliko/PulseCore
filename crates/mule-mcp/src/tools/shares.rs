// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Registros y despachadores de herramientas MCP para compartidos, fuentes, subidas y escuchas.

pub mod handlers;
pub mod schemas;

use crate::client::DaemonClient;
use crate::protocol::{ToolCallResult, ToolDefinition};
use serde_json::Value;

/// Devuelve las definiciones de herramientas para compartidos, fuentes, subidas y escuchas.
pub fn list_tools() -> Vec<ToolDefinition> {
    schemas::definitions()
}

/// Despacha las llamadas a herramientas de compartidos/fuentes.
pub async fn handle_tool(
    client: &DaemonClient,
    tool_name: &str,
    args: &Value,
) -> Option<ToolCallResult> {
    handlers::execute(client, tool_name, args).await
}

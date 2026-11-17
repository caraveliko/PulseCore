// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Registros y despachadores de herramientas MCP para descargas y categorías.

pub mod handlers;
pub mod schemas;

use crate::client::DaemonClient;
use crate::protocol::{ToolCallResult, ToolDefinition};
pub use handlers::resolve_download_id;
use serde_json::Value;

/// Devuelve las definiciones de herramientas para la gestión de descargas y categorías.
pub fn list_tools() -> Vec<ToolDefinition> {
    schemas::definitions()
}

/// Despacha las llamadas a herramientas de descargas/categorías.
pub async fn handle_tool(
    client: &DaemonClient,
    tool_name: &str,
    args: &Value,
) -> Option<ToolCallResult> {
    handlers::execute(client, tool_name, args).await
}

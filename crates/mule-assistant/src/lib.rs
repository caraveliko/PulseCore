// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Asistente de IA local para eMuleRust.
//!
//! Puente fino entre un LLM local (Ollama en loopback) y el JSON-RPC del
//! daemon: el modelo propone llamadas a herramientas, [`session::Session`]
//! las valida ([`policy`]) y luego las ejecuta mediante `mule_mcp::execute_tool`;
//! el daemon sigue siendo la autoridad de validación y seguridad. No se crea
//! ningún nuevo registro de acciones: los esquemas y el mapeo RPC se
//! reutilizan desde `mule-mcp`.

pub mod config;
pub mod error;
pub mod llm;
pub mod policy;
pub mod repl;
pub mod session;
pub mod tools_bridge;
pub mod ws;

pub use config::AssistantConfig;
pub use error::AssistantError;
pub use llm::{AssistantTurn, ChatMessage, LlmBackend, LlmMetrics, Role, ToolCall, ToolSchema};
pub use session::{AssistantEvent, ConfirmRequest, Session};

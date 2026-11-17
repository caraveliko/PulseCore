// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Errores tipados del bucle del asistente.

/// Error fatal del asistente (configuración, backend, daemon o
/// transporte). Los rechazos de política no son errores: se devuelven al
/// modelo como resultados de herramienta.
#[derive(Debug, thiserror::Error)]
pub enum AssistantError {
    /// Configuración no válida (punto de conexión no local sin anulación…).
    #[error("configuration invalide : {0}")]
    Config(String),

    /// Fallo en el backend LLM (HTTP, análisis, flujo).
    #[error("backend LLM : {0}")]
    Backend(String),

    /// Fallo de conexión o de llamada al daemon.
    #[error("daemon : {0}")]
    Daemon(String),

    /// Error de transporte (REPL, WebSocket).
    #[error("transport : {0}")]
    Transport(String),
}

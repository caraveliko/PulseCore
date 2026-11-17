// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Configuración del asistente: endpoint LLM, modelo y presupuestos.
//!
//! Todos los ajustes viven aquí; no hay ningún valor literal en la lógica,
//! conforme a la regla «constantes y ajustes» de AGENTS.md.

use std::time::Duration;

use mule_mcp::tools::ToolProfile;

use crate::error::AssistantError;

/// Punto de conexión LLM por defecto: API nativa de Ollama en bucle local (loopback).
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:11434";

/// Modelo por defecto: debe implementar la llamada a herramientas (tool calling) de Ollama.
pub const DEFAULT_MODEL: &str = "qwen3:8b";

/// Número máximo de turnos «modelo → herramientas → modelo» por mensaje
/// de usuario: acota el bucle agéntico.
pub const DEFAULT_MAX_TOOL_ROUNDS: u32 = 4;

/// Tamaño máximo (caracteres) del resultado de herramienta devuelto al modelo:
/// la interfaz de usuario recibe siempre la versión completa mediante [`crate::AssistantEvent`].
pub const DEFAULT_TOOL_RESULT_MAX_CHARS: usize = 4000;

/// Tiempo de espera global de una petición hacia el backend LLM: una generación
/// prolongada en una CPU modesta no debe truncarse prematuramente.
pub const DEFAULT_LLM_TIMEOUT: Duration = Duration::from_secs(300);

/// Tiempo de espera de establecimiento de la conexión HTTP con el backend.
pub const LLM_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Puerto por defecto del servidor WebSocket del asistente.
pub const DEFAULT_WS_PORT: u16 = 4680;

/// Prompt del sistema: contexto y directrices para el modelo.
/// Redactado en español de España.
pub const SYSTEM_PROMPT: &str = "Eres el asistente integrado de eMuleRust, un cliente P2P eD2K/Kad. \
Controlas la aplicación a través de las herramientas proporcionadas: utilízalas para obtener el estado real \
(descargas, Kad, servidores) en lugar de adivinar. Encadena las llamadas si es necesario. \
Cuando una información baste, responde directamente sin realizar una nueva llamada. \
Responde en español de España, de forma concisa. Si una herramienta falla, explica el error al usuario.";

/// Configuración validada de una sesión del asistente.
#[derive(Debug, Clone)]
pub struct AssistantConfig {
    /// URL base del backend LLM (ej. `http://127.0.0.1:11434`).
    pub endpoint: String,
    /// Nombre del modelo en el backend.
    pub model: String,
    /// Permite explícitamente un punto de conexión no local; desactivado por
    /// defecto para ajustarse a la postura de aislamiento del proyecto.
    pub allow_remote_endpoint: bool,
    /// Límite del bucle agéntico (véase [`DEFAULT_MAX_TOOL_ROUNDS`]).
    pub max_tool_rounds: u32,
    /// Truncamiento del resultado de la herramienta para el modelo
    /// (véase [`DEFAULT_TOOL_RESULT_MAX_CHARS`]).
    pub tool_result_max_chars: usize,
    /// Tiempo de espera HTTP global hacia el backend.
    pub llm_timeout: Duration,
    /// Techo de riesgo de las herramientas expuestas al modelo.
    pub tool_profile: ToolProfile,
}

impl Default for AssistantConfig {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_ENDPOINT.to_string(),
            model: DEFAULT_MODEL.to_string(),
            allow_remote_endpoint: false,
            max_tool_rounds: DEFAULT_MAX_TOOL_ROUNDS,
            tool_result_max_chars: DEFAULT_TOOL_RESULT_MAX_CHARS,
            llm_timeout: DEFAULT_LLM_TIMEOUT,
            tool_profile: ToolProfile::Full,
        }
    }
}

impl AssistantConfig {
    /// Valida el punto de conexión: esquema `http` y host **IP literal** en bucle local
    /// (`127.0.0.1`, `::1`): los nombres de host se rechazan por ser resolubles
    /// hacia el exterior. `--allow-remote-endpoint` es la única anulación permitida.
    pub fn validate(&self) -> Result<(), AssistantError> {
        let parsed = url::Url::parse(&self.endpoint).map_err(|e| {
            AssistantError::Config(format!("endpoint illisible '{}': {e}", self.endpoint))
        })?;
        if parsed.scheme() != "http" {
            return Err(AssistantError::Config(format!(
                "endpoint '{}' : seul le scheme http est accepté",
                self.endpoint
            )));
        }
        let is_loopback = match parsed.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            Some(url::Host::Domain(_)) => {
                return Err(AssistantError::Config(format!(
                    "endpoint '{}' : l'hôte doit être une IP littérale (127.0.0.1 ou ::1), pas un nom à résoudre",
                    self.endpoint
                )));
            }
            None => {
                return Err(AssistantError::Config(format!(
                    "endpoint '{}' sans hôte",
                    self.endpoint
                )));
            }
        };
        if !is_loopback && !self.allow_remote_endpoint {
            return Err(AssistantError::Config(format!(
                "endpoint '{}' non loopback : passer --allow-remote-endpoint pour l'autoriser explicitement",
                self.endpoint
            )));
        }
        Ok(())
    }
}

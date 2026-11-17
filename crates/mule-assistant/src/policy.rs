// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Salvaguardas específicas de la IA: qué herramientas puede ver e invocar
//! el modelo, y cuáles exigen confirmación humana.
//!
//! Tres filtros acumulativos, verificados de nuevo justo antes de cada
//! ejecución — las capacidades o la conexión pueden cambiar durante una
//! sesión:
//! 1. [`TOOL_ALLOWLIST`]: subconjunto seleccionado presentado al modelo
//!    (límite estático — la fiabilidad del tool calling de un modelo local
//!    se mide, no se da por supuesta como universal);
//! 2. [`ToolProfile`]: límite de riesgo (`mule-mcp::tools::policy`);
//! 3. Capacidades reales del daemon (`system.status` → `capabilities`) —
//!    la intersección oculta las herramientas cuya capacidad esté ausente.

use mule_mcp::protocol::ToolDefinition;
use mule_mcp::tools::{classify, list_tools_for, ToolProfile, ToolRisk};

use crate::error::AssistantError;

/// Subconjunto seleccionado de herramientas presentadas al modelo: operaciones habituales
/// de un cliente eD2K/Kad. Ajustar tras medir la tasa de selección correcta
/// en los modelos de destino.
const TOOL_ALLOWLIST: &[&str] = &[
    // Estado general
    "emule_status",
    "emule_get_settings",
    // Búsqueda y conexión de red
    "emule_search",
    "emule_list_servers",
    "emule_connect_server",
    "emule_disconnect_server",
    "emule_kad_status",
    // Descargas
    "emule_list_downloads",
    "emule_get_download_details",
    "emule_add_link",
    "emule_pause_download",
    "emule_resume_download",
    // Archivos compartidos y fuentes
    "emule_list_shares",
    "emule_swarm_status",
    "emule_list_sources",
];

/// Herramientas de la lista de permitidos que exigen confirmación del usuario mostrando
/// los argumentos exactos: efectos externos o difícilmente reversibles.
/// `emule_pause_download`/`emule_resume_download` permanecen libres:
/// reversibles y puramente locales. Cualquier herramienta clasificada como
/// [`ToolRisk::Destructive`] requiere también confirmación (defensa en
/// profundidad si el perfil `full` dejara pasar una destructiva).
const CONFIRM_REQUIRED: &[&str] = &[
    "emule_add_link",
    "emule_connect_server",
    "emule_disconnect_server",
];

/// Capacidad del daemon requerida por herramienta: prefijos evaluados en orden,
/// la primera coincidencia gana; sin coincidencia = siempre permitida. La nomenclatura
/// de las capacidades (`daemon_capabilities()` en mule-daemon) es
/// por dominio, no por herramienta; esta asignación es la única traducción.
const TOOL_CAPABILITY_PREFIXES: &[(&str, &str)] = &[
    ("emule_kad_", "kad-contacts"),
    ("emule_list_categor", "categories"),
    ("emule_create_categor", "categories"),
    ("emule_update_categor", "categories"),
    ("emule_delete_categor", "categories"),
    ("emule_reorder_categor", "categories"),
    ("emule_download_set_category", "categories"),
    ("emule_list_history", "history"),
    ("emule_history_stats", "history"),
    ("emule_export_history", "history"),
    ("emule_clear_history", "history"),
    ("emule_get_network_policy", "network-policy"),
    ("emule_set_network_profile", "network-policy"),
    ("emule_network_", "network-policy"),
    ("emule_set_network_budgets", "network-policy"),
    ("emule_get_filter_status", "ip-filter"),
    ("emule_import_ip_filter", "ip-filter"),
    ("emule_set_filter_enabled", "ip-filter"),
    ("emule_clear_ip_filter", "ip-filter"),
    ("emule_set_limits", "rate-limits"),
    ("emule_list_credits", "peer-credits"),
    ("emule_credits_stats", "peer-credits"),
    ("emule_get_identity", "secure-ident"),
    ("emule_rotate_identity", "secure-ident"),
    ("emule_list_shares", "shared-files"),
    ("emule_add_share", "shared-files"),
    ("emule_remove_share", "shared-files"),
    ("emule_rename_share", "shared-files"),
    ("emule_set_share_enabled", "shared-files"),
    ("emule_list_servers", "servers"),
    ("emule_update_servers_from_url", "servers"),
    ("emule_inspect_server_met", "servers"),
    ("emule_import_server_met", "servers"),
    ("emule_set_server_enabled", "servers"),
    ("emule_delete_server", "servers"),
    ("emule_clear_servers", "servers"),
    ("emule_connect_server", "server-connection"),
    ("emule_disconnect_server", "server-connection"),
    ("emule_search", "server-search"),
    ("emule_query_server_sources", "server-sources"),
    ("emule_list_sources", "peer-sources"),
    ("emule_add_manual_source", "peer-sources"),
    ("emule_clear_sources", "peer-sources"),
    ("emule_connect_source", "peer-connection"),
    ("emule_disconnect_source", "peer-connection"),
    ("emule_swarm_status", "swarm-dual-source"),
    ("emule_list_uploads", "upload-single-slot"),
    ("emule_cancel_upload", "upload-single-slot"),
    ("emule_list_downloads", "downloads"),
    ("emule_get_download_details", "downloads"),
    ("emule_add_link", "downloads"),
    ("emule_pause_download", "downloads"),
    ("emule_resume_download", "downloads"),
    ("emule_remove_download", "downloads"),
    ("emule_rename_download", "downloads"),
    ("emule_set_destination", "downloads"),
    ("emule_retry_download", "downloads"),
    ("emule_inspect_part_met", "downloads"),
    ("emule_import_part_met", "downloads"),
];

/// Capacidad requerida para `tool_name`: `None` = sin capacidad dedicada.
/// Pública para la prueba que verifica la tabla frente a
/// `daemon_capabilities()`.
pub fn required_capability(tool_name: &str) -> Option<&'static str> {
    TOOL_CAPABILITY_PREFIXES
        .iter()
        .find(|(prefix, _)| tool_name.starts_with(prefix))
        .map(|(_, cap)| *cap)
}

/// Herramientas visibles para el modelo: `list_tools_for(profile)` filtrada
/// por la lista de permitidos y luego por las capacidades reales del daemon.
pub fn visible_tools(profile: ToolProfile, capabilities: &[String]) -> Vec<ToolDefinition> {
    list_tools_for(profile)
        .into_iter()
        .filter(|t| TOOL_ALLOWLIST.contains(&t.name.as_str()))
        .filter(|t| {
            required_capability(&t.name)
                .map(|cap| capabilities.iter().any(|c| c == cap))
                .unwrap_or(true)
        })
        .collect()
}

/// Verificación de ejecución: llamada justo antes de `execute_tool`, no
/// solo al confeccionar la lista: el modelo puede solicitar una
/// herramienta fuera de la lista de permitidos y las capacidades pueden haber cambiado.
pub fn check_call(
    profile: ToolProfile,
    capabilities: &[String],
    tool_name: &str,
) -> Result<(), AssistantError> {
    if !TOOL_ALLOWLIST.contains(&tool_name) {
        return Err(AssistantError::Config(format!(
            "outil '{tool_name}' hors de l'allowlist assistant"
        )));
    }
    let risk = classify(tool_name).ok_or_else(|| {
        AssistantError::Config(format!("outil '{tool_name}' non classé dans la policy MCP"))
    })?;
    if !profile.allows(risk) {
        return Err(AssistantError::Config(format!(
            "outil '{tool_name}' bloqué par le profil '{}'",
            profile.as_str()
        )));
    }
    if let Some(cap) = required_capability(tool_name) {
        if !capabilities.iter().any(|c| c == cap) {
            return Err(AssistantError::Config(format!(
                "capacité '{cap}' absente du daemon pour '{tool_name}'"
            )));
        }
    }
    Ok(())
}

/// Verdadero si la herramienta exige confirmación del usuario antes de la ejecución:
/// miembros explícitos de [`CONFIRM_REQUIRED`] más cualquier destructiva
/// (último bastión en el asistente; el daemon sigue siendo la autoridad).
pub fn requires_confirmation(tool_name: &str) -> bool {
    CONFIRM_REQUIRED.contains(&tool_name) || classify(tool_name) == Some(ToolRisk::Destructive)
}

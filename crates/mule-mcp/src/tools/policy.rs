// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Clasificación de riesgo y perfiles de filtrado de las herramientas MCP.
//!
//! Fuente única de verdad para «qué herramienta puede hacer qué»: las tres
//! tablas siguientes cubren la totalidad de [`crate::tools::list_tools`]
//! — `tests/tool_profile_tests.rs` falla si una herramienta añadida no está
//! clasificada, lo que obliga a revisar el nivel de riesgo con cada nueva
//! herramienta.
//!
//! El filtrado se aplica en DOS lugares: `tools/list` (visibilidad) y
//! `tools/call` (ejecución) — filtrar únicamente la lista es cosmético, un
//! cliente puede invocar por su nombre una herramienta no listada.

use clap::ValueEnum;

/// Nivel de riesgo de una herramienta MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRisk {
    /// Solo lectura: ninguna mutación de estado en el daemon.
    Read,
    /// Mutación de estado reversible o con efecto acotado (conexión, pausa,
    /// importación, publicación…).
    Mutate,
    /// Acción destructiva o crítica: pérdida de datos, parada del daemon,
    /// corte de red, pérdida de identidad/créditos.
    Destructive,
}

/// Perfil de filtrado expuesto mediante `--tool-profile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum ToolProfile {
    /// Herramientas de solo lectura — exposición sin mutación.
    Read,
    /// Lectura + mutaciones, sin acciones destructivas.
    Safe,
    /// Todas las herramientas — comportamiento histórico, perfil por defecto.
    #[default]
    Full,
}

impl ToolProfile {
    /// Verdadero si el perfil autoriza la ejecución/presentación de una herramienta
    /// de nivel `risk`.
    pub fn allows(self, risk: ToolRisk) -> bool {
        match self {
            ToolProfile::Read => risk == ToolRisk::Read,
            ToolProfile::Safe => risk != ToolRisk::Destructive,
            ToolProfile::Full => true,
        }
    }

    /// Etiqueta estable utilizada en los mensajes de error y registros.
    pub fn as_str(self) -> &'static str {
        match self {
            ToolProfile::Read => "read",
            ToolProfile::Safe => "safe",
            ToolProfile::Full => "full",
        }
    }
}

/// Herramientas de solo lectura — sin efectos secundarios en el daemon.
const READ_TOOLS: &[&str] = &[
    // system
    "emule_status",
    "emule_get_settings",
    "emule_get_network_policy",
    "emule_get_audit_log",
    "emule_get_filter_status",
    "emule_disk_space",
    "emule_nas_info",
    "emule_system_logs",
    "emule_media_list",
    "emule_search_all",
    // servers
    "emule_list_servers",
    "emule_inspect_server_met",
    "emule_query_server_sources",
    "emule_search",
    // kad
    "emule_kad_status",
    "emule_kad_ping",
    "emule_kad_find_node",
    "emule_kad_list_contacts",
    "emule_kad_search_sources",
    "emule_kad_search_notes",
    "emule_kad_inspect_nodes_dat",
    // compartidos / subidas / identidad / historial
    "emule_list_sources",
    "emule_swarm_status",
    "emule_list_shares",
    "emule_list_uploads",
    "emule_listener_status",
    "emule_upnp_status",
    "emule_get_identity",
    "emule_list_credits",
    "emule_credits_stats",
    "emule_list_history",
    "emule_history_stats",
    "emule_export_history",
    // downloads
    "emule_list_downloads",
    "emule_get_download_details",
    "emule_inspect_part_met",
    "emule_list_categories",
];

/// Herramientas destructivas o críticas: pérdida irreversible, parada, corte
/// de red global, pérdida de identidad (los créditos eD2K están ligados al hash).
const DESTRUCTIVE_TOOLS: &[&str] = &[
    "emule_shutdown",
    "emule_network_disconnect_all",
    "emule_clear_ip_filter",
    "emule_delete_server",
    "emule_clear_servers",
    "emule_kad_clear_contacts",
    "emule_clear_sources",
    "emule_remove_share",
    "emule_cancel_upload",
    "emule_rotate_identity",
    "emule_clear_history",
    "emule_remove_download",
    "emule_delete_category",
];

/// Herramientas de mutación de estado reversible o acotada.
const MUTATE_TOOLS: &[&str] = &[
    // system
    "emule_update_settings",
    "emule_reset_download_dir",
    "emule_set_limits",
    "emule_set_network_profile",
    "emule_network_enable",
    "emule_set_network_budgets",
    "emule_import_ip_filter",
    "emule_set_filter_enabled",
    // servers
    "emule_connect_server",
    "emule_disconnect_server",
    "emule_update_servers_from_url",
    "emule_import_server_met",
    "emule_set_server_enabled",
    // kad
    "emule_kad_bootstrap",
    "emule_kad_start_udp",
    "emule_kad_stop_udp",
    "emule_kad_publish_source",
    "emule_kad_publish_keywords",
    "emule_kad_publish_note",
    "emule_kad_import_nodes_dat",
    // shares / uploads / listener / upnp
    "emule_add_manual_source",
    "emule_connect_source",
    "emule_disconnect_source",
    "emule_add_share",
    "emule_rename_share",
    "emule_set_share_enabled",
    "emule_start_listener",
    "emule_stop_listener",
    "emule_upnp_map_ports",
    "emule_upnp_unmap_ports",
    "emule_upnp_set_enabled",
    // downloads
    "emule_add_link",
    "emule_pause_download",
    "emule_resume_download",
    "emule_rename_download",
    "emule_set_destination",
    "emule_download_set_category",
    "emule_retry_download",
    "emule_import_part_met",
    "emule_create_category",
    "emule_update_category",
    "emule_reorder_categories",
];

/// Clasificación de una herramienta por nombre. `None` = herramienta no catalogada:
/// tratada como «rechazada» fuera del perfil `Full` (defecto seguro), y detectada
/// por la prueba de exhaustividad.
pub fn classify(tool_name: &str) -> Option<ToolRisk> {
    if READ_TOOLS.contains(&tool_name) {
        return Some(ToolRisk::Read);
    }
    if DESTRUCTIVE_TOOLS.contains(&tool_name) {
        return Some(ToolRisk::Destructive);
    }
    if MUTATE_TOOLS.contains(&tool_name) {
        return Some(ToolRisk::Mutate);
    }
    None
}

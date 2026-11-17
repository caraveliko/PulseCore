// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Pruebas de la política del asistente: lista de permitidos, capacidades y confirmaciones.

use mule_assistant::policy::{
    check_call, required_capability, requires_confirmation, visible_tools,
};
use mule_daemon::rpc::capabilities::daemon_capabilities;
use mule_mcp::tools::{list_tools, ToolProfile};

/// La lista de permitidos solo debe hacer referencia a herramientas existentes; de lo contrario, un
/// cambio de nombre de herramienta MCP pasaría desapercibido.
#[test]
fn test_allowlist_references_known_tools() {
    let known: std::collections::HashSet<String> =
        list_tools().iter().map(|t| t.name.clone()).collect();
    let caps: Vec<String> = daemon_capabilities()
        .iter()
        .map(|s| s.to_string())
        .collect();
    for tool in visible_tools(ToolProfile::Full, &caps) {
        assert!(known.contains(&tool.name), "outil inconnu : {}", tool.name);
    }
}

/// Cada capacidad referenciada por la correspondencia existe en
/// `daemon_capabilities()`; de lo contrario, una herramienta quedaría oculta por error.
/// Con todas las capacidades reales presentes, toda la lista de permitidos es
/// visible (perfil full).
#[test]
fn test_mapped_capabilities_exist_in_daemon() {
    let caps = daemon_capabilities();
    for tool in list_tools() {
        if let Some(cap) = required_capability(&tool.name) {
            assert!(
                caps.contains(&cap),
                "capacité '{cap}' (pour {}) absente de daemon_capabilities()",
                tool.name
            );
        }
    }
    let visible = visible_tools(ToolProfile::Full, &caps_strs(&caps));
    let names: Vec<&str> = visible.iter().map(|t| t.name.as_str()).collect();
    for expected in [
        "emule_status",
        "emule_search",
        "emule_add_link",
        "emule_kad_status",
    ] {
        assert!(names.contains(&expected), "{expected} masqué à tort");
    }
}

/// Las herramientas cuya capacidad esté ausente se ocultan.
#[test]
fn test_missing_capability_hides_tool() {
    let visible = visible_tools(ToolProfile::Full, &[]);
    // "downloads" ausente → las herramientas de descargas se ocultan.
    assert!(!visible.iter().any(|t| t.name == "emule_add_link"));
    // emule_status no requiere capacidad → permanece visible.
    assert!(visible.iter().any(|t| t.name == "emule_status"));
}

/// check_call: lista de permitidos, perfil y capacidades son todos recomprobados.
#[test]
fn test_check_call_enforces_all_layers() {
    let caps = caps_strs(&daemon_capabilities());

    // Fuera de la lista de permitidos → rechazo incluso si la herramienta existe y es Read.
    assert!(check_call(ToolProfile::Full, &caps, "emule_clear_history").is_err());

    // Lista de permitidos + perfil read → rechazo de mutaciones.
    assert!(check_call(ToolProfile::Read, &caps, "emule_add_link").is_err());

    // Lista de permitidos + capacidad ausente → rechazo.
    assert!(check_call(ToolProfile::Full, &[], "emule_add_link").is_err());

    // Todo alineado → aceptado.
    assert!(check_call(ToolProfile::Full, &caps, "emule_status").is_ok());
    assert!(check_call(ToolProfile::Full, &caps, "emule_add_link").is_ok());
}

/// Confirmaciones: las herramientas con efectos externos y las destructivas.
#[test]
fn test_requires_confirmation() {
    assert!(requires_confirmation("emule_add_link"));
    assert!(requires_confirmation("emule_connect_server"));
    // Destructiva: confirmación incluso si está presente en un perfil full.
    assert!(requires_confirmation("emule_shutdown"));
    // Reversible local: sin confirmación.
    assert!(!requires_confirmation("emule_pause_download"));
    assert!(!requires_confirmation("emule_status"));
}

fn caps_strs(caps: &[&'static str]) -> Vec<String> {
    caps.iter().map(|s| s.to_string()).collect()
}

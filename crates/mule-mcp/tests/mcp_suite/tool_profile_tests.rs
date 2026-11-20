// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Pruebas del perfil de filtrado `--tool-profile` (`mule-mcp::tools::policy`).

use mule_mcp::tools::policy::{classify, ToolProfile, ToolRisk};
use mule_mcp::tools::{execute_tool_for, list_tools, list_tools_for};
use mule_mcp::DaemonClient;
use mule_test_support::spawn_test_daemon;
use serde_json::json;

/// Exhaustividad de la clasificación: toda herramienta expuesta debe tener un
/// nivel de riesgo — esta prueba es la que obliga a revisar el riesgo al
/// añadir una nueva herramienta.
#[test]
fn test_every_tool_is_classified() {
    for tool in list_tools() {
        assert!(
            classify(&tool.name).is_some(),
            "herramienta no clasificada en policy.rs: {}",
            tool.name
        );
    }
}

/// El perfil `read` solo expone herramientas de lectura.
#[test]
fn test_read_profile_exposes_only_read_tools() {
    let tools = list_tools_for(ToolProfile::Read);
    assert!(!tools.is_empty());
    for tool in &tools {
        assert_eq!(
            classify(&tool.name),
            Some(ToolRisk::Read),
            "herramienta no-Read expuesta en perfil read: {}",
            tool.name
        );
    }
    // Referencias conocidas: una lectura presente, una mutación y una
    // destructiva ausentes.
    assert!(tools.iter().any(|t| t.name == "emule_status"));
    assert!(!tools.iter().any(|t| t.name == "emule_add_link"));
    assert!(!tools.iter().any(|t| t.name == "emule_shutdown"));
}

/// El perfil `safe` excluye las destructivas pero mantiene las mutaciones.
#[test]
fn test_safe_profile_excludes_destructive_tools() {
    let tools = list_tools_for(ToolProfile::Safe);
    for tool in &tools {
        assert_ne!(
            classify(&tool.name),
            Some(ToolRisk::Destructive),
            "herramienta destructiva expuesta en perfil safe: {}",
            tool.name
        );
    }
    assert!(tools.iter().any(|t| t.name == "emule_status"));
    assert!(tools.iter().any(|t| t.name == "emule_add_link"));
    assert!(!tools.iter().any(|t| t.name == "emule_shutdown"));
    assert!(!tools.iter().any(|t| t.name == "emule_remove_download"));
}

/// El perfil `full` reproduce la lista completa.
#[test]
fn test_full_profile_exposes_everything() {
    assert_eq!(list_tools_for(ToolProfile::Full).len(), list_tools().len());
}

/// El rechazo de ejecución tiene lugar ANTES de cualquier contacto con el daemon:
/// una herramienta fuera de perfil se rechaza incluso con un cliente válido.
#[tokio::test]
async fn test_execute_tool_blocked_by_profile() {
    let (addr, token, _dir) =
        spawn_test_daemon("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef").await;
    let client = DaemonClient::connect(Some(format!("ws://{addr}/rpc")), Some(token), None, None)
        .await
        .expect("DaemonClient connection should succeed");

    // Mutación rechazada en perfil read
    let res = execute_tool_for(
        ToolProfile::Read,
        &client,
        "emule_add_link",
        json!({ "link": "ed2k://|file|x.mkv|1|31D6CFE0D16AE931B73C59D7E0C089C0|/" }),
    )
    .await;
    assert!(res.is_error);
    assert!(res.content[0].text.contains("blocked by profile 'read'"));

    // Destructiva rechazada en perfil safe
    let res = execute_tool_for(ToolProfile::Safe, &client, "emule_shutdown", json!({})).await;
    assert!(res.is_error);
    assert!(res.content[0].text.contains("blocked by profile 'safe'"));

    // Herramienta desconocida: rechazo independientemente del perfil
    let res = execute_tool_for(ToolProfile::Full, &client, "emule_nope", json!({})).await;
    assert!(res.is_error);
    assert!(res.content[0].text.contains("Unknown tool"));

    // Lectura autorizada en perfil read: la llamada llega al daemon
    let res = execute_tool_for(ToolProfile::Read, &client, "emule_status", json!({})).await;
    assert!(!res.is_error, "status blocked unexpectedly: {:?}", res);
}

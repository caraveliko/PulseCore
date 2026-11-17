// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Manejadores JSON-RPC del espacio de nombres `system.*` para el sistema de actualizaciones.

use serde_json::{json, Value};

use crate::error::JsonRpcError;
use crate::state::AppState;

/// Maneja la petición `system.checkUpdate`: fuerza la consulta de versiones disponibles.
pub async fn handle_check_update(
    state: &AppState,
    _params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let status_snapshot = state
        .updater
        .check_for_updates()
        .await
        .map_err(|e| JsonRpcError::internal(format!("Error al verificar actualizaciones: {e}")))?;

    Ok(serde_json::to_value(status_snapshot).unwrap_or(json!({"status": "error"})))
}

/// Maneja la petición `system.getUpdateStatus`: devuelve la instantánea del estado actual.
pub async fn handle_get_update_status(state: &AppState) -> Result<Value, JsonRpcError> {
    let status_snapshot = state.updater.get_status().await;
    Ok(serde_json::to_value(status_snapshot).unwrap_or(json!({"status": "idle"})))
}

/// Maneja la petición `system.applyUpdate`: descarga, verifica y aplica la sustitución atómica.
pub async fn handle_apply_update(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let auto_restart = params
        .as_ref()
        .and_then(|p| p.get("autoRestart"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // 1. Descarga el paquete a staging y verifica SHA-256 + firma Ed25519
    let staging_path = state
        .updater
        .download_and_stage()
        .await
        .map_err(|e| JsonRpcError::internal(format!("Fallo durante la preparación de la actualización: {e}")))?;

    tracing::info!(
        staging = %staging_path.display(),
        auto_restart,
        "Actualización verificada y preparada en staging"
    );

    if auto_restart {
        // 2. Aplica la sustitución atómica del ejecutable en disco
        state
            .updater
            .apply_update()
            .await
            .map_err(|e| JsonRpcError::internal(format!("Fallo al sustituir el ejecutable: {e}")))?;

        // Marca que se debe relanzar el proceso al culminar el apagado
        state.updater.mark_restart_pending();

        // 3. Notifica apagado ordenado para reiniciar el proceso
        state.shutdown_notify.notify_waiters();

        Ok(json!({
            "accepted": true,
            "stage": "restarting",
            "message": "Actualización aplicada correctamente. Demonio reiniciando de forma ordenada."
        }))
    } else {
        Ok(json!({
            "accepted": true,
            "stage": "ready_to_install",
            "stagingPath": staging_path.to_string_lossy(),
            "message": "Actualización preparada. Se aplicará al próximo reinicio o solicitud."
        }))
    }
}

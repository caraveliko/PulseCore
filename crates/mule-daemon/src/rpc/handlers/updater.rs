// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Manejadores JSON-RPC del espacio de nombres `system.*` para el sistema de actualizaciones.

use std::time::Duration;

use serde_json::{json, Value};

use crate::error::JsonRpcError;
use crate::services::notifications::{
    constants::DISK_LOW_COOLDOWN_SECS, NotificationCategory, NotificationEventInput,
    NotificationKind, NotificationSeverity,
};
use crate::services::updater::{
    is_managed_by_systemd, is_running_in_docker, UpdateStatusSnapshot, UpdaterError,
    AUTO_UPDATE_CHECK_INTERVAL, AUTO_UPDATE_INITIAL_DELAY, RESTART_RESPONSE_GRACE_MS,
};
use crate::state::AppState;

/// Serializa una instantánea de estado de actualización junto con la información real del entorno.
async fn attach_environment(state: &AppState, snapshot: UpdateStatusSnapshot) -> Value {
    let env = state.updater.environment_info().await;
    let mut obj = match serde_json::to_value(snapshot) {
        Ok(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    if let Ok(env_val) = serde_json::to_value(env) {
        obj.insert("environment".to_string(), env_val);
    }
    Value::Object(obj)
}

/// Programa el apagado ordenado tras un breve margen para que la respuesta HTTP/RPC llegue al cliente.
pub(crate) fn schedule_graceful_restart(state: &AppState) {
    state.updater.mark_restart_pending();
    let notify = std::sync::Arc::clone(&state.shutdown_notify);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(RESTART_RESPONSE_GRACE_MS)).await;
        notify.notify_waiters();
    });
}

/// Maneja la petición `system.checkUpdate`: fuerza la consulta de versiones disponibles.
pub async fn handle_check_update(
    state: &AppState,
    _params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    match state.updater.check_for_updates(&state.network_policy).await {
        Ok(snapshot) => {
            if let UpdateStatusSnapshot::UpdateAvailable {
                ref version,
                ref current_version,
                ..
            } = snapshot
            {
                let _ = state
                    .notifications
                    .notify(
                        state,
                        NotificationEventInput {
                            kind: NotificationKind::UpdateAvailable,
                            severity: NotificationSeverity::Info,
                            category: NotificationCategory::System,
                            title: "Nueva actualización disponible".to_string(),
                            body: format!(
                                "Pulse Core v{version} disponible (actual: v{current_version})."
                            ),
                            metadata: Some(json!({
                                "version": version,
                                "currentVersion": current_version,
                            })),
                            dedupe_key: Some(format!("update:{version}")),
                            cooldown_secs: DISK_LOW_COOLDOWN_SECS,
                        },
                    )
                    .await;
            }
            Ok(attach_environment(state, snapshot).await)
        }
        Err(
            UpdaterError::HttpStatus { .. }
            | UpdaterError::NetworkBlocked(_)
            | UpdaterError::Network(_)
            | UpdaterError::TooManyRedirects { .. }
            | UpdaterError::InvalidManifest(_)
            | UpdaterError::InvalidChannel(_)
            | UpdaterError::UnsupportedPlatform(_)
            | UpdaterError::PayloadTooLarge { .. }
            | UpdaterError::InvalidExecutableFormat(_),
        ) => {
            let snapshot = state.updater.get_status().await;
            Ok(attach_environment(state, snapshot).await)
        }
        Err(e) => Err(JsonRpcError::internal(format!(
            "Error al verificar actualizaciones: {e}"
        ))),
    }
}

/// Maneja la petición `system.getUpdateStatus`: devuelve la instantánea actual y el entorno.
pub async fn handle_get_update_status(state: &AppState) -> Result<Value, JsonRpcError> {
    let status_snapshot = state.updater.get_status().await;
    Ok(attach_environment(state, status_snapshot).await)
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

    // 1. Descarga en streaming, verifica SHA-256 + Ed25519 + ELF/PE y prepara en staging
    let staging_path = state
        .updater
        .download_and_stage(&state.network_policy)
        .await
        .map_err(|e| {
            JsonRpcError::internal(format!("Fallo durante la preparación de la actualización: {e}"))
        })?;

    let managed_by_systemd = is_managed_by_systemd();

    tracing::info!(
        staging = %staging_path.display(),
        auto_restart,
        managed_by_systemd,
        "Actualización verificada y preparada en staging"
    );

    if auto_restart {
        // 2. Aplica la sustitución atómica del ejecutable en disco
        state
            .updater
            .apply_update()
            .await
            .map_err(|e| JsonRpcError::internal(format!("Fallo al sustituir el ejecutable: {e}")))?;

        // 3. Programa apagado ordenado (bajo systemd la unidad relanza el proceso actualizado)
        schedule_graceful_restart(state);

        let message = if managed_by_systemd {
            "Actualización aplicada correctamente. systemd está reiniciando Pulse Core."
        } else {
            "Actualización aplicada correctamente. Demonio reiniciando de forma ordenada."
        };

        Ok(json!({
            "accepted": true,
            "stage": "restarting",
            "managedBySystemd": managed_by_systemd,
            "message": message
        }))
    } else {
        Ok(json!({
            "accepted": true,
            "stage": "ready_to_install",
            "managedBySystemd": managed_by_systemd,
            "stagingPath": staging_path.to_string_lossy(),
            "message": "Actualización preparada. Se aplicará al próximo reinicio o solicitud."
        }))
    }
}

/// Maneja la petición `system.rollbackUpdate`: restaura la copia `.old` previa y reinicia si se solicita.
pub async fn handle_rollback_update(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let auto_restart = params
        .as_ref()
        .and_then(|p| p.get("autoRestart"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    match state.updater.rollback_update().await {
        Ok(()) => {
            if auto_restart {
                schedule_graceful_restart(state);
            }
            let managed_by_systemd = is_managed_by_systemd();
            let message = if auto_restart {
                if managed_by_systemd {
                    "Copia anterior (.old) restaurada. systemd está reiniciando Pulse Core."
                } else {
                    "Copia anterior (.old) restaurada. Pulse Core se está reiniciando."
                }
            } else {
                "Copia anterior (.old) restaurada en disco. Se aplicará en el próximo reinicio."
            };
            Ok(json!({
                "accepted": true,
                "restarting": auto_restart,
                "managedBySystemd": managed_by_systemd,
                "message": message,
            }))
        }
        Err(UpdaterError::NoRollbackAvailable) => Err(JsonRpcError::business(
            UpdaterError::NoRollbackAvailable.to_string(),
        )),
        Err(e) => Err(JsonRpcError::internal(format!(
            "Fallo al revertir a la versión anterior: {e}"
        ))),
    }
}

/// Supervisor en segundo plano que comprueba e instala actualizaciones automáticamente
/// cuando `auto_update_enabled` está activo (modo servidor / Raspberry Pi).
pub fn start_auto_update_monitor(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(AUTO_UPDATE_INITIAL_DELAY).await;
        let mut interval = tokio::time::interval(AUTO_UPDATE_CHECK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            interval.tick().await;
            if !state.updater.is_auto_update_enabled() {
                continue;
            }
            if is_running_in_docker()
                || state.network_policy.state() == mule_network_policy::NetworkState::Disabled
                || state.network_policy.profile().is_loopback_only()
            {
                continue;
            }
            let env = state.updater.environment_info().await;
            if !env.can_self_update {
                continue;
            }
            if let Ok(UpdateStatusSnapshot::UpdateAvailable { version, .. }) =
                state.updater.check_for_updates(&state.network_policy).await
            {
                tracing::info!(
                    version = %version,
                    "Autoactualización activa: descargando e instalando nueva versión"
                );
                if let Err(err) = state.updater.download_and_stage(&state.network_policy).await {
                    tracing::warn!("Error descargando autoactualización v{version}: {err}");
                    continue;
                }
                if let Err(err) = state.updater.apply_update().await {
                    tracing::warn!("Error aplicando autoactualización v{version}: {err}");
                    continue;
                }
                let _ = state
                    .notifications
                    .notify(
                        &state,
                        NotificationEventInput {
                            kind: NotificationKind::UpdateAvailable,
                            severity: NotificationSeverity::Info,
                            category: NotificationCategory::System,
                            title: format!("Pulse Core actualizado a v{version}"),
                            body: "Se ha instalado automáticamente la nueva versión oficial. Reiniciando servicio…".to_string(),
                            metadata: Some(json!({ "version": version, "autoApplied": true })),
                            dedupe_key: Some(format!("auto-update:{version}")),
                            cooldown_secs: DISK_LOW_COOLDOWN_SECS,
                        },
                    )
                    .await;
                schedule_graceful_restart(&state);
                break;
            }
        }
    })
}

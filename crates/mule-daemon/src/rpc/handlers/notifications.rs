// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Manejadores JSON-RPC del espacio de nombres `notifications.*`.

use mule_core::NotificationFilter;
use serde_json::{json, Value};

use crate::error::JsonRpcError;
use crate::services::notifications::constants::{
    DEFAULT_NOTIFICATION_LIST_LIMIT, MAX_NOTIFICATION_LIST_LIMIT,
};
use crate::services::notifications::error::NotificationError;
use crate::services::notifications::NotificationConfigPatch;
use crate::services::DaemonEvent;
use crate::state::AppState;

fn map_notification_err(err: NotificationError) -> JsonRpcError {
    match err {
        NotificationError::InvalidChannel(msg) => JsonRpcError::invalid_params(&msg),
        NotificationError::NotConfigured(msg) | NotificationError::NetworkBlocked(msg) => {
            JsonRpcError::business(msg)
        }
        NotificationError::HttpStatus { status, detail } => {
            JsonRpcError::business(format!("Error HTTP {status}: {detail}"))
        }
        NotificationError::Transport(e) => JsonRpcError::business(e.to_string()),
        NotificationError::Storage(msg) | NotificationError::Serialization(msg) => {
            JsonRpcError::internal(msg)
        }
    }
}

/// Lista el historial de notificaciones persistidas junto al contador de no leídas.
pub async fn handle_notifications_list(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let limit = params
        .as_ref()
        .and_then(|p| p.get("limit"))
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).clamp(1, MAX_NOTIFICATION_LIST_LIMIT))
        .unwrap_or(DEFAULT_NOTIFICATION_LIST_LIMIT);
    let unread_only = params
        .as_ref()
        .and_then(|p| p.get("unreadOnly"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let category = params
        .as_ref()
        .and_then(|p| p.get("category"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all")
        .map(ToString::to_string);

    let filter = NotificationFilter {
        category,
        unread_only,
        limit: Some(limit),
    };

    let (notifications, unread_count) = {
        let dl = state.downloads.lock().await;
        let db = dl.db_ref();
        let list = db
            .list_notifications(&filter)
            .map_err(|e| JsonRpcError::business(e.to_string()))?;
        let unread = db
            .count_unread_notifications()
            .map_err(|e| JsonRpcError::business(e.to_string()))?;
        (list, unread)
    };

    Ok(json!({
        "notifications": notifications,
        "unreadCount": unread_count,
    }))
}

/// Marca una notificación o todas las pendientes como leídas.
pub async fn handle_notifications_mark_read(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| {
        JsonRpcError::invalid_params("Se requiere el objeto de parámetros con 'id' o 'all'")
    })?;
    let mark_all = params
        .get("all")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let (updated, unread_count) = {
        let mut dl = state.downloads.lock().await;
        let db = dl.db_mut();
        let updated = if mark_all {
            db.mark_all_notifications_read()
                .map_err(|e| JsonRpcError::business(e.to_string()))?
        } else if let Some(id) = params.get("id").and_then(|v| v.as_i64()) {
            let found = db
                .mark_notification_read(id)
                .map_err(|e| JsonRpcError::business(e.to_string()))?;
            usize::from(found)
        } else {
            return Err(JsonRpcError::invalid_params(
                "Debe indicar 'id' numérico o 'all: true'",
            ));
        };
        let unread = db
            .count_unread_notifications()
            .map_err(|e| JsonRpcError::business(e.to_string()))?;
        (updated, unread)
    };

    Ok(json!({
        "updated": updated,
        "unreadCount": unread_count,
    }))
}

/// Vacía el historial de notificaciones almacenado en la base de datos.
pub async fn handle_notifications_clear(state: &AppState) -> Result<Value, JsonRpcError> {
    let cleared = state
        .downloads
        .lock()
        .await
        .db_mut()
        .clear_notifications()
        .map_err(|e| JsonRpcError::business(e.to_string()))?;

    Ok(json!({
        "cleared": cleared,
        "unreadCount": 0,
    }))
}

/// Devuelve la configuración actual de notificaciones con los secretos enmascarados.
pub async fn handle_notifications_get_config(state: &AppState) -> Result<Value, JsonRpcError> {
    let view = state.notifications.get_config_view().await;
    Ok(crate::rpc::handlers::json_value(&view))
}

/// Actualiza parcialmente la configuración de notificaciones y la persiste en SQLite.
pub async fn handle_notifications_set_config(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params
        .ok_or_else(|| JsonRpcError::invalid_params("Se requiere el objeto de configuración"))?;
    let patch: NotificationConfigPatch = serde_json::from_value(params)
        .map_err(|e| JsonRpcError::invalid_params(&format!("Parámetros inválidos: {e}")))?;

    let view = state
        .notifications
        .update_config(state, patch)
        .await
        .map_err(map_notification_err)?;

    state.emit_event(|sequence| DaemonEvent::SettingsChanged {
        sequence,
        keys: vec!["notificationsConfig".to_string()],
    });

    Ok(crate::rpc::handlers::json_value(&view))
}

/// Envía una notificación de prueba al canal solicitado (`webui`, `telegram`, `ntfy`, `discord`, `webhook`).
pub async fn handle_notifications_test(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let channel = params
        .as_ref()
        .and_then(|p| p.get("channel"))
        .and_then(|v| v.as_str())
        .unwrap_or("webui");

    let result = state
        .notifications
        .send_test(state, channel)
        .await
        .map_err(map_notification_err)?;

    Ok(crate::rpc::handlers::json_value(&result))
}

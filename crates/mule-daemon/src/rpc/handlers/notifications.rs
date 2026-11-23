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
use crate::services::notifications::types::{
    DiscordChannelPatch, NotificationConfigPatch, NotificationEventsConfig, NtfyChannelPatch,
    QuietHoursConfig, TelegramChannelPatch, WebhookChannelPatch,
};
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

/// Extrae un booleano opcional de un objeto JSON, rechazando tipos distintos de `bool`/`null`.
fn opt_bool(obj: &serde_json::Map<String, Value>, field: &str) -> Result<Option<bool>, JsonRpcError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(JsonRpcError::invalid_params(&format!(
            "Parámetro '{field}' debe ser booleano"
        ))),
    }
}

/// Extrae un entero `u64` opcional de un objeto JSON.
fn opt_u64(obj: &serde_json::Map<String, Value>, field: &str) -> Result<Option<u64>, JsonRpcError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| {
            JsonRpcError::invalid_params(&format!("Parámetro '{field}' debe ser entero positivo"))
        }),
    }
}

/// Extrae un entero `u8` opcional de un objeto JSON validando el rango.
fn opt_u8(obj: &serde_json::Map<String, Value>, field: &str) -> Result<Option<u8>, JsonRpcError> {
    match opt_u64(obj, field)? {
        None => Ok(None),
        Some(n) => u8::try_from(n).map(Some).map_err(|_| {
            JsonRpcError::invalid_params(&format!("Parámetro '{field}' fuera de rango (0..=255)"))
        }),
    }
}

/// Extrae una cadena opcional de un objeto JSON.
fn opt_string(
    obj: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<String>, JsonRpcError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(JsonRpcError::invalid_params(&format!(
            "Parámetro '{field}' debe ser una cadena"
        ))),
    }
}

/// Extrae un subobjeto JSON opcional (ignorando ausencia o `null`).
fn opt_object<'a>(
    obj: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<&'a serde_json::Map<String, Value>>, JsonRpcError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(sub)) => Ok(Some(sub)),
        Some(_) => Err(JsonRpcError::invalid_params(&format!(
            "Parámetro '{field}' debe ser un objeto JSON"
        ))),
    }
}

/// Construye `NotificationEventsConfig` desde un subobjeto JSON aplicando valores por defecto.
fn parse_events_config(
    obj: &serde_json::Map<String, Value>,
) -> Result<NotificationEventsConfig, JsonRpcError> {
    let mut cfg = NotificationEventsConfig::default();
    if let Some(v) = opt_bool(obj, "downloadCompleted")? {
        cfg.download_completed = v;
    }
    if let Some(v) = opt_bool(obj, "downloadFailed")? {
        cfg.download_failed = v;
    }
    if let Some(v) = opt_bool(obj, "mediaReady")? {
        cfg.media_ready = v;
    }
    if let Some(v) = opt_bool(obj, "mediaGrabbed")? {
        cfg.media_grabbed = v;
    }
    if let Some(v) = opt_bool(obj, "serverDisconnected")? {
        cfg.server_disconnected = v;
    }
    if let Some(v) = opt_bool(obj, "updateAvailable")? {
        cfg.update_available = v;
    }
    if let Some(v) = opt_bool(obj, "diskLow")? {
        cfg.disk_low = v;
    }
    if let Some(v) = opt_bool(obj, "systemWarning")? {
        cfg.system_warning = v;
    }
    Ok(cfg)
}

/// Construye `QuietHoursConfig` desde un subobjeto JSON aplicando valores por defecto.
fn parse_quiet_hours_config(
    obj: &serde_json::Map<String, Value>,
) -> Result<QuietHoursConfig, JsonRpcError> {
    let mut cfg = QuietHoursConfig::default();
    if let Some(v) = opt_bool(obj, "enabled")? {
        cfg.enabled = v;
    }
    if let Some(v) = opt_u8(obj, "startHour")? {
        cfg.start_hour = v;
    }
    if let Some(v) = opt_u8(obj, "startMinute")? {
        cfg.start_minute = v;
    }
    if let Some(v) = opt_u8(obj, "endHour")? {
        cfg.end_hour = v;
    }
    if let Some(v) = opt_u8(obj, "endMinute")? {
        cfg.end_minute = v;
    }
    if let Some(v) = opt_bool(obj, "allowCritical")? {
        cfg.allow_critical = v;
    }
    Ok(cfg)
}

/// Construye `NotificationConfigPatch` extrayendo explícitamente cada campo desde JSON.
fn parse_notification_config_patch(params: &Value) -> Result<NotificationConfigPatch, JsonRpcError> {
    let obj = params
        .as_object()
        .ok_or_else(|| JsonRpcError::invalid_params("Se requiere un objeto de configuración"))?;

    let events = opt_object(obj, "events")?.map(parse_events_config).transpose()?;
    let quiet_hours = opt_object(obj, "quietHours")?
        .map(parse_quiet_hours_config)
        .transpose()?;
    let telegram = opt_object(obj, "telegram")?
        .map(|sub| {
            Ok::<_, JsonRpcError>(TelegramChannelPatch {
                enabled: opt_bool(sub, "enabled")?,
                bot_token: opt_string(sub, "botToken")?,
                chat_id: opt_string(sub, "chatId")?,
                clear_bot_token: opt_bool(sub, "clearBotToken")?,
            })
        })
        .transpose()?;
    let ntfy = opt_object(obj, "ntfy")?
        .map(|sub| {
            Ok::<_, JsonRpcError>(NtfyChannelPatch {
                enabled: opt_bool(sub, "enabled")?,
                url: opt_string(sub, "url")?,
                token: opt_string(sub, "token")?,
                clear_token: opt_bool(sub, "clearToken")?,
            })
        })
        .transpose()?;
    let discord = opt_object(obj, "discord")?
        .map(|sub| {
            Ok::<_, JsonRpcError>(DiscordChannelPatch {
                enabled: opt_bool(sub, "enabled")?,
                webhook_url: opt_string(sub, "webhookUrl")?,
                clear_webhook_url: opt_bool(sub, "clearWebhookUrl")?,
            })
        })
        .transpose()?;
    let webhook = opt_object(obj, "webhook")?
        .map(|sub| {
            Ok::<_, JsonRpcError>(WebhookChannelPatch {
                enabled: opt_bool(sub, "enabled")?,
                url: opt_string(sub, "url")?,
                secret: opt_string(sub, "secret")?,
                clear_secret: opt_bool(sub, "clearSecret")?,
            })
        })
        .transpose()?;

    Ok(NotificationConfigPatch {
        enabled: opt_bool(obj, "enabled")?,
        external_enabled: opt_bool(obj, "externalEnabled")?,
        disk_low_threshold_gb: opt_u64(obj, "diskLowThresholdGb")?,
        events,
        quiet_hours,
        telegram,
        ntfy,
        discord,
        webhook,
    })
}

/// Actualiza parcialmente la configuración de notificaciones y la persiste en SQLite.
pub async fn handle_notifications_set_config(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params
        .ok_or_else(|| JsonRpcError::invalid_params("Se requiere el objeto de configuración"))?;
    let patch = parse_notification_config_patch(&params)?;

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

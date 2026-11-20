// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Herramientas MCP para el centro de notificaciones y canales push.

use serde_json::{json, Value};

use crate::client::DaemonClient;
use crate::protocol::{ToolCallResult, ToolDefinition};

use super::{obj_schema, prop};

/// Devuelve las definiciones MCP del espacio de notificaciones.
pub fn list_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "emule_list_notifications".to_string(),
            description: "Lista el historial de notificaciones de Pulse Core junto con el contador de alertas sin leer.".to_string(),
            input_schema: obj_schema(
                json!({
                    "limit": prop("integer", "Número máximo de notificaciones a devolver (por defecto: 40)"),
                    "unread_only": prop("boolean", "Si es true, devuelve únicamente notificaciones no leídas"),
                    "category": {
                        "type": "string",
                        "enum": ["all", "downloads", "media", "network", "system"],
                        "description": "Filtro opcional por categoría temática"
                    }
                }),
                &[],
            ),
        },
        ToolDefinition {
            name: "emule_mark_notifications_read".to_string(),
            description: "Marca como leída una notificación concreta (por id) o todas las pendientes (all: true).".to_string(),
            input_schema: obj_schema(
                json!({
                    "id": prop("integer", "Identificador numérico de la notificación a marcar"),
                    "all": prop("boolean", "Si es true, marca todas las notificaciones como leídas")
                }),
                &[],
            ),
        },
        ToolDefinition {
            name: "emule_clear_notifications".to_string(),
            description: "Vacía por completo el historial persistido de notificaciones en SQLite.".to_string(),
            input_schema: obj_schema(json!({}), &[]),
        },
        ToolDefinition {
            name: "emule_get_notifications_config".to_string(),
            description: "Obtiene la configuración de notificaciones y canales push (Telegram, ntfy, Discord, Webhook) con secretos enmascarados.".to_string(),
            input_schema: obj_schema(json!({}), &[]),
        },
        ToolDefinition {
            name: "emule_update_notifications_config".to_string(),
            description: "Actualiza parcialmente la configuración de notificaciones, eventos activos, horario silencioso y credenciales de canales push.".to_string(),
            input_schema: obj_schema(
                json!({
                    "config": prop("object", "Objeto parche JSON compatible con NotificationConfigPatch (enabled, externalEnabled, diskLowThresholdGb, events, quietHours, telegram, ntfy, discord, webhook)")
                }),
                &["config"],
            ),
        },
        ToolDefinition {
            name: "emule_test_notification_channel".to_string(),
            description: "Envía una notificación de prueba al canal especificado para verificar conectividad y credenciales.".to_string(),
            input_schema: obj_schema(
                json!({
                    "channel": {
                        "type": "string",
                        "enum": ["webui", "telegram", "ntfy", "discord", "webhook"],
                        "description": "Canal destino a comprobar (por defecto: webui)"
                    }
                }),
                &[],
            ),
        },
    ]
}

async fn call_notifications_rpc(
    client: &DaemonClient,
    method: &str,
    params: Value,
    err_ctx: &str,
) -> ToolCallResult {
    match client.call(method, params).await {
        Ok(res) => ToolCallResult::json(&res),
        Err(e) => ToolCallResult::error(format!("{err_ctx}: {e}")),
    }
}

/// Despacha las invocaciones MCP relacionadas con notificaciones.
pub async fn handle_tool(
    client: &DaemonClient,
    tool_name: &str,
    args: &Value,
) -> Option<ToolCallResult> {
    match tool_name {
        "emule_list_notifications" => {
            let mut params = serde_json::Map::new();
            if let Some(limit) = args.get("limit").and_then(|v| v.as_u64()) {
                params.insert("limit".to_string(), json!(limit));
            }
            if let Some(unread) = args.get("unread_only").and_then(|v| v.as_bool()) {
                params.insert("unreadOnly".to_string(), json!(unread));
            }
            if let Some(cat) = args.get("category").and_then(|v| v.as_str()) {
                params.insert("category".to_string(), json!(cat));
            }
            Some(
                call_notifications_rpc(
                    client,
                    "notifications.list",
                    Value::Object(params),
                    "No se pudo listar el historial de notificaciones",
                )
                .await,
            )
        }
        "emule_mark_notifications_read" => {
            let params = if args.get("all").and_then(|v| v.as_bool()) == Some(true) {
                json!({ "all": true })
            } else if let Some(id) = args.get("id").and_then(|v| v.as_i64()) {
                json!({ "id": id })
            } else {
                return Some(ToolCallResult::error(
                    "Debe indicar 'id' numérico o 'all: true'".to_string(),
                ));
            };
            Some(
                call_notifications_rpc(
                    client,
                    "notifications.markRead",
                    params,
                    "No se pudieron marcar las notificaciones como leídas",
                )
                .await,
            )
        }
        "emule_clear_notifications" => Some(
            call_notifications_rpc(
                client,
                "notifications.clear",
                json!({}),
                "No se pudo limpiar el historial de notificaciones",
            )
            .await,
        ),
        "emule_get_notifications_config" => Some(
            call_notifications_rpc(
                client,
                "notifications.getConfig",
                json!({}),
                "No se pudo obtener la configuración de notificaciones",
            )
            .await,
        ),
        "emule_update_notifications_config" => {
            let Some(config) = args.get("config").filter(|v| v.is_object()).cloned() else {
                return Some(ToolCallResult::error(
                    "Parámetro obligatorio ausente o no válido: 'config' (objeto JSON)".to_string(),
                ));
            };
            Some(
                call_notifications_rpc(
                    client,
                    "notifications.setConfig",
                    config,
                    "No se pudo actualizar la configuración de notificaciones",
                )
                .await,
            )
        }
        "emule_test_notification_channel" => {
            let channel = args
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("webui");
            Some(
                call_notifications_rpc(
                    client,
                    "notifications.test",
                    json!({ "channel": channel }),
                    "No se pudo enviar la notificación de prueba",
                )
                .await,
            )
        }
        _ => None,
    }
}

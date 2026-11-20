// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Adaptadores de formato y entrega para Telegram, ntfy, Discord y Webhook genérico.

use mule_core::model::NotificationRecord;
use mule_network_policy::NetworkPolicyManager;
use serde_json::json;

use super::constants::{
    DISCORD_EMBED_COLOR_ERROR, DISCORD_EMBED_COLOR_INFO, DISCORD_EMBED_COLOR_SUCCESS,
    DISCORD_EMBED_COLOR_WARNING,
};
use super::error::NotificationError;
use super::http_client::post_with_retries;
use super::types::{
    DiscordChannelConfig, NotificationSeverity, NtfyChannelConfig, TelegramChannelConfig,
    WebhookChannelConfig,
};

/// Escapa caracteres especiales HTML requeridos por el modo `HTML` de Telegram.
pub fn escape_telegram_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Elimina saltos de línea y caracteres de control de un valor de cabecera HTTP.
pub fn sanitize_http_header_value(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c == '\r' || c == '\n' || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn parse_severity(raw: &str) -> NotificationSeverity {
    match raw {
        "success" => NotificationSeverity::Success,
        "warning" => NotificationSeverity::Warning,
        "error" => NotificationSeverity::Error,
        _ => NotificationSeverity::Info,
    }
}

fn discord_color_for_severity(severity: NotificationSeverity) -> u32 {
    match severity {
        NotificationSeverity::Success => DISCORD_EMBED_COLOR_SUCCESS,
        NotificationSeverity::Info => DISCORD_EMBED_COLOR_INFO,
        NotificationSeverity::Warning => DISCORD_EMBED_COLOR_WARNING,
        NotificationSeverity::Error => DISCORD_EMBED_COLOR_ERROR,
    }
}

fn ntfy_priority_and_tags(severity: NotificationSeverity) -> (&'static str, &'static str) {
    match severity {
        NotificationSeverity::Success => ("default", "white_check_mark"),
        NotificationSeverity::Info => ("default", "bell"),
        NotificationSeverity::Warning => ("high", "warning"),
        NotificationSeverity::Error => ("urgent", "rotating_light"),
    }
}

/// Construye el cuerpo JSON para Telegram Bot API (`sendMessage`).
pub fn build_telegram_payload(chat_id: &str, record: &NotificationRecord) -> serde_json::Value {
    let icon = parse_severity(&record.severity).icon();
    let title = escape_telegram_html(&record.title);
    let body = escape_telegram_html(&record.body);
    let text = if body.is_empty() {
        format!("<b>{icon} {title}</b>")
    } else {
        format!("<b>{icon} {title}</b>\n{body}")
    };

    json!({
        "chat_id": chat_id.trim(),
        "text": text,
        "parse_mode": "HTML",
        "disable_web_page_preview": true,
    })
}

/// Envía una notificación mediante Telegram Bot API.
pub async fn send_telegram(
    api_base: &str,
    cfg: &TelegramChannelConfig,
    record: &NotificationRecord,
    network_policy: &NetworkPolicyManager,
) -> Result<(), NotificationError> {
    let token = cfg.bot_token.trim();
    let chat_id = cfg.chat_id.trim();
    if token.is_empty() || chat_id.is_empty() {
        return Err(NotificationError::NotConfigured(
            "Telegram requiere configurar el token del bot y el Chat ID".to_string(),
        ));
    }

    let url = format!("{}/bot{}/sendMessage", api_base.trim_end_matches('/'), token);
    let payload = build_telegram_payload(chat_id, record);
    let body = serde_json::to_vec(&payload)
        .map_err(|e| NotificationError::Serialization(e.to_string()))?;

    post_with_retries(&url, &[], &body, "application/json", network_policy).await
}

/// Envía una notificación hacia un tema ntfy (ntfy.sh o instancia autoalojada).
pub async fn send_ntfy(
    cfg: &NtfyChannelConfig,
    record: &NotificationRecord,
    network_policy: &NetworkPolicyManager,
) -> Result<(), NotificationError> {
    let url = cfg.url.trim();
    if url.is_empty() {
        return Err(NotificationError::NotConfigured(
            "ntfy requiere configurar la URL del tema".to_string(),
        ));
    }

    let sev = parse_severity(&record.severity);
    let (priority, tags) = ntfy_priority_and_tags(sev);
    let title = sanitize_http_header_value(&record.title);

    let mut headers = vec![
        ("Title".to_string(), title),
        ("Priority".to_string(), priority.to_string()),
        ("Tags".to_string(), tags.to_string()),
    ];
    let token = cfg.token.trim();
    if !token.is_empty() {
        headers.push(("Authorization".to_string(), format!("Bearer {token}")));
    }

    post_with_retries(
        url,
        &headers,
        record.body.as_bytes(),
        "text/plain; charset=utf-8",
        network_policy,
    )
    .await
}

/// Construye la carga JSON con formato *embed* para Discord Webhooks.
pub fn build_discord_payload(record: &NotificationRecord) -> serde_json::Value {
    let sev = parse_severity(&record.severity);
    let title = format!("{} {}", sev.icon(), record.title);
    json!({
        "username": "Pulse Core",
        "embeds": [{
            "title": title,
            "description": record.body,
            "color": discord_color_for_severity(sev),
            "footer": {
                "text": format!("Pulse Core · {}", record.category),
            }
        }]
    })
}

/// Envía una notificación mediante un Webhook de Discord.
pub async fn send_discord(
    cfg: &DiscordChannelConfig,
    record: &NotificationRecord,
    network_policy: &NetworkPolicyManager,
) -> Result<(), NotificationError> {
    let url = cfg.webhook_url.trim();
    if url.is_empty() {
        return Err(NotificationError::NotConfigured(
            "Discord requiere configurar la URL del Webhook".to_string(),
        ));
    }

    let payload = build_discord_payload(record);
    let body = serde_json::to_vec(&payload)
        .map_err(|e| NotificationError::Serialization(e.to_string()))?;

    post_with_retries(url, &[], &body, "application/json", network_policy).await
}

/// Construye la carga JSON estándar para el canal Webhook genérico.
pub fn build_webhook_payload(record: &NotificationRecord) -> serde_json::Value {
    json!({
        "source": "pulse-core",
        "version": crate::rpc::methods::DAEMON_VERSION,
        "notification": record,
    })
}

/// Envía una notificación mediante un Webhook HTTP POST JSON genérico.
pub async fn send_webhook(
    cfg: &WebhookChannelConfig,
    record: &NotificationRecord,
    network_policy: &NetworkPolicyManager,
) -> Result<(), NotificationError> {
    let url = cfg.url.trim();
    if url.is_empty() {
        return Err(NotificationError::NotConfigured(
            "Webhook genérico requiere configurar la URL de destino".to_string(),
        ));
    }

    let payload = build_webhook_payload(record);
    let body = serde_json::to_vec(&payload)
        .map_err(|e| NotificationError::Serialization(e.to_string()))?;

    let mut headers = Vec::new();
    let secret = sanitize_http_header_value(&cfg.secret);
    if !secret.is_empty() {
        headers.push(("X-PulseCore-Token".to_string(), secret));
    }

    post_with_retries(url, &headers, &body, "application/json", network_policy).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(severity: &str) -> NotificationRecord {
        NotificationRecord {
            id: 42,
            kind: "downloadCompleted".to_string(),
            severity: severity.to_string(),
            category: "downloads".to_string(),
            title: "Película <1080p> & Extras".to_string(),
            body: "Completada\nlista para ver".to_string(),
            metadata_json: None,
            is_read: false,
            created_at_unix_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn test_telegram_html_escaping_and_payload() {
        let rec = sample_record("success");
        let payload = build_telegram_payload("987654", &rec);
        assert_eq!(payload["chat_id"], "987654");
        assert_eq!(payload["parse_mode"], "HTML");
        let text = payload["text"].as_str().unwrap();
        assert!(text.contains("&lt;1080p&gt; &amp; Extras"));
        assert!(!text.contains("<1080p>"));
    }

    #[test]
    fn test_ntfy_header_sanitization_and_discord_colors() {
        let cleaned = sanitize_http_header_value("Título\r\ninjectado\nX-Evil: 1");
        assert!(!cleaned.contains('\r'));
        assert!(!cleaned.contains('\n'));

        let ok_embed = build_discord_payload(&sample_record("success"));
        assert_eq!(
            ok_embed["embeds"][0]["color"],
            json!(DISCORD_EMBED_COLOR_SUCCESS)
        );
        let err_embed = build_discord_payload(&sample_record("error"));
        assert_eq!(
            err_embed["embeds"][0]["color"],
            json!(DISCORD_EMBED_COLOR_ERROR)
        );
    }

    #[test]
    fn test_generic_webhook_payload_contains_notification() {
        let rec = sample_record("info");
        let payload = build_webhook_payload(&rec);
        assert_eq!(payload["source"], "pulse-core");
        assert_eq!(payload["notification"]["id"], 42);
        assert_eq!(payload["notification"]["kind"], "downloadCompleted");
    }
}

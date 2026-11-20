// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Tipos de dominio, configuración persistible, parches y vistas seguras de notificaciones.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::constants::{
    BYTES_PER_GIB, BYTES_PER_KIB, BYTES_PER_MIB, DEFAULT_DISK_LOW_THRESHOLD_GB,
    DEFAULT_QUIET_END_HOUR, DEFAULT_QUIET_END_MINUTE, DEFAULT_QUIET_START_HOUR,
    DEFAULT_QUIET_START_MINUTE, MAX_DISK_LOW_THRESHOLD_GB, MAX_HOUR_24H, MAX_MINUTE_HOUR,
    MIN_DISK_LOW_THRESHOLD_GB, MINUTES_PER_HOUR,
};

/// Tipo específico de suceso notificable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationKind {
    DownloadCompleted,
    DownloadFailed,
    MediaReady,
    MediaGrabbed,
    ServerDisconnected,
    UpdateAvailable,
    DiskLow,
    SystemWarning,
    Test,
}

impl NotificationKind {
    /// Representación canónica en cable y SQLite.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DownloadCompleted => "downloadCompleted",
            Self::DownloadFailed => "downloadFailed",
            Self::MediaReady => "mediaReady",
            Self::MediaGrabbed => "mediaGrabbed",
            Self::ServerDisconnected => "serverDisconnected",
            Self::UpdateAvailable => "updateAvailable",
            Self::DiskLow => "diskLow",
            Self::SystemWarning => "systemWarning",
            Self::Test => "test",
        }
    }

    /// Categoría lógica asociada de forma predeterminada.
    pub fn category(&self) -> NotificationCategory {
        match self {
            Self::DownloadCompleted | Self::DownloadFailed => NotificationCategory::Downloads,
            Self::MediaReady | Self::MediaGrabbed => NotificationCategory::Media,
            Self::ServerDisconnected => NotificationCategory::Network,
            Self::UpdateAvailable | Self::DiskLow | Self::SystemWarning | Self::Test => {
                NotificationCategory::System
            }
        }
    }
}

/// Nivel de severidad de una notificación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationSeverity {
    Info,
    Success,
    Warning,
    Error,
}

impl NotificationSeverity {
    /// Representación textual persistida en SQLite y WebUI.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    /// Indica si la notificación se considera crítica y puede saltar el horario silencioso.
    pub fn is_critical(&self) -> bool {
        matches!(self, Self::Error)
    }

    /// Icono visual legible reutilizado en canales de texto enriquecido.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Success => "✅",
            Self::Warning => "⚠️",
            Self::Error => "🚨",
            Self::Info => "ℹ️",
        }
    }
}

/// Categoría temática para el filtrado en el centro de notificaciones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationCategory {
    Downloads,
    Media,
    Network,
    System,
}

impl NotificationCategory {
    /// Representación textual persistida en SQLite.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Downloads => "downloads",
            Self::Media => "media",
            Self::Network => "network",
            Self::System => "system",
        }
    }
}

/// Entrada de evento interno para construir, persistir y despachar una notificación.
#[derive(Debug, Clone)]
pub struct NotificationEventInput {
    pub kind: NotificationKind,
    pub severity: NotificationSeverity,
    pub category: NotificationCategory,
    pub title: String,
    pub body: String,
    pub metadata: Option<serde_json::Value>,
    pub dedupe_key: Option<String>,
    pub cooldown_secs: u64,
}

/// Banderas por tipo de evento que controlan si debe generar notificación.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationEventsConfig {
    pub download_completed: bool,
    pub download_failed: bool,
    pub media_ready: bool,
    pub media_grabbed: bool,
    pub server_disconnected: bool,
    pub update_available: bool,
    pub disk_low: bool,
    pub system_warning: bool,
}

impl Default for NotificationEventsConfig {
    fn default() -> Self {
        Self {
            download_completed: true,
            download_failed: true,
            media_ready: true,
            media_grabbed: true,
            server_disconnected: true,
            update_available: true,
            disk_low: true,
            system_warning: true,
        }
    }
}

/// Configuración de horario silencioso (*No molestar*) para canales externos.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct QuietHoursConfig {
    pub enabled: bool,
    pub start_hour: u8,
    pub start_minute: u8,
    pub end_hour: u8,
    pub end_minute: u8,
    pub allow_critical: bool,
}

impl Default for QuietHoursConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            start_hour: DEFAULT_QUIET_START_HOUR,
            start_minute: DEFAULT_QUIET_START_MINUTE,
            end_hour: DEFAULT_QUIET_END_HOUR,
            end_minute: DEFAULT_QUIET_END_MINUTE,
            allow_critical: true,
        }
    }
}

/// Configuración interna del canal Telegram Bot API.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct TelegramChannelConfig {
    pub enabled: bool,
    pub bot_token: String,
    pub chat_id: String,
}

/// Configuración interna del canal ntfy (público o autoalojado).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct NtfyChannelConfig {
    pub enabled: bool,
    pub url: String,
    pub token: String,
}

/// Configuración interna del canal Discord Webhook.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct DiscordChannelConfig {
    pub enabled: bool,
    pub webhook_url: String,
}

/// Configuración interna del canal Webhook JSON genérico.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct WebhookChannelConfig {
    pub enabled: bool,
    pub url: String,
    pub secret: String,
}

/// Configuración completa del sistema de notificaciones almacenada en SQLite.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationConfig {
    pub enabled: bool,
    pub external_enabled: bool,
    pub disk_low_threshold_gb: u64,
    pub events: NotificationEventsConfig,
    pub quiet_hours: QuietHoursConfig,
    pub telegram: TelegramChannelConfig,
    pub ntfy: NtfyChannelConfig,
    pub discord: DiscordChannelConfig,
    pub webhook: WebhookChannelConfig,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            external_enabled: false,
            disk_low_threshold_gb: DEFAULT_DISK_LOW_THRESHOLD_GB,
            events: NotificationEventsConfig::default(),
            quiet_hours: QuietHoursConfig::default(),
            telegram: TelegramChannelConfig::default(),
            ntfy: NtfyChannelConfig::default(),
            discord: DiscordChannelConfig::default(),
            webhook: WebhookChannelConfig::default(),
        }
    }
}

impl NotificationConfig {
    /// Comprueba si un tipo de evento concreto está habilitado en las preferencias.
    pub fn is_kind_enabled(&self, kind: NotificationKind) -> bool {
        match kind {
            NotificationKind::DownloadCompleted => self.events.download_completed,
            NotificationKind::DownloadFailed => self.events.download_failed,
            NotificationKind::MediaReady => self.events.media_ready,
            NotificationKind::MediaGrabbed => self.events.media_grabbed,
            NotificationKind::ServerDisconnected => self.events.server_disconnected,
            NotificationKind::UpdateAvailable => self.events.update_available,
            NotificationKind::DiskLow => self.events.disk_low,
            NotificationKind::SystemWarning => self.events.system_warning,
            NotificationKind::Test => true,
        }
    }

    /// Aplica una actualización parcial conservando los secretos existentes si no se sobrescriben.
    pub fn apply_patch(&mut self, patch: NotificationConfigPatch) {
        if let Some(v) = patch.enabled {
            self.enabled = v;
        }
        if let Some(v) = patch.external_enabled {
            self.external_enabled = v;
        }
        if let Some(gb) = patch.disk_low_threshold_gb {
            self.disk_low_threshold_gb =
                gb.clamp(MIN_DISK_LOW_THRESHOLD_GB, MAX_DISK_LOW_THRESHOLD_GB);
        }
        if let Some(events) = patch.events {
            self.events = events;
        }
        if let Some(mut qh) = patch.quiet_hours {
            qh.start_hour = qh.start_hour.min(MAX_HOUR_24H);
            qh.start_minute = qh.start_minute.min(MAX_MINUTE_HOUR);
            qh.end_hour = qh.end_hour.min(MAX_HOUR_24H);
            qh.end_minute = qh.end_minute.min(MAX_MINUTE_HOUR);
            self.quiet_hours = qh;
        }
        if let Some(tg) = patch.telegram {
            if let Some(enabled) = tg.enabled {
                self.telegram.enabled = enabled;
            }
            if let Some(chat_id) = tg.chat_id {
                self.telegram.chat_id = chat_id.trim().to_string();
            }
            if tg.clear_bot_token.unwrap_or(false) {
                self.telegram.bot_token.clear();
            } else if let Some(token) = tg.bot_token {
                let trimmed = token.trim();
                if !trimmed.is_empty() {
                    self.telegram.bot_token = trimmed.to_string();
                }
            }
        }
        if let Some(ntfy) = patch.ntfy {
            if let Some(enabled) = ntfy.enabled {
                self.ntfy.enabled = enabled;
            }
            if let Some(url) = ntfy.url {
                self.ntfy.url = url.trim().to_string();
            }
            if ntfy.clear_token.unwrap_or(false) {
                self.ntfy.token.clear();
            } else if let Some(token) = ntfy.token {
                let trimmed = token.trim();
                if !trimmed.is_empty() {
                    self.ntfy.token = trimmed.to_string();
                }
            }
        }
        if let Some(discord) = patch.discord {
            if let Some(enabled) = discord.enabled {
                self.discord.enabled = enabled;
            }
            if discord.clear_webhook_url.unwrap_or(false) {
                self.discord.webhook_url.clear();
            } else if let Some(url) = discord.webhook_url {
                let trimmed = url.trim();
                if !trimmed.is_empty() {
                    self.discord.webhook_url = trimmed.to_string();
                }
            }
        }
        if let Some(webhook) = patch.webhook {
            if let Some(enabled) = webhook.enabled {
                self.webhook.enabled = enabled;
            }
            if let Some(url) = webhook.url {
                self.webhook.url = url.trim().to_string();
            }
            if webhook.clear_secret.unwrap_or(false) {
                self.webhook.secret.clear();
            } else if let Some(secret) = webhook.secret {
                let trimmed = secret.trim();
                if !trimmed.is_empty() {
                    self.webhook.secret = trimmed.to_string();
                }
            }
        }
    }

    /// Construye la proyección segura sin exponer secretos en claro al cliente RPC.
    pub fn to_view(&self, channel_statuses: HashMap<String, ChannelStatus>) -> NotificationConfigView {
        NotificationConfigView {
            enabled: self.enabled,
            external_enabled: self.external_enabled,
            disk_low_threshold_gb: self.disk_low_threshold_gb,
            events: self.events.clone(),
            quiet_hours: self.quiet_hours.clone(),
            telegram: TelegramChannelView {
                enabled: self.telegram.enabled,
                chat_id: self.telegram.chat_id.clone(),
                has_bot_token: !self.telegram.bot_token.trim().is_empty(),
            },
            ntfy: NtfyChannelView {
                enabled: self.ntfy.enabled,
                url: self.ntfy.url.clone(),
                has_token: !self.ntfy.token.trim().is_empty(),
            },
            discord: DiscordChannelView {
                enabled: self.discord.enabled,
                has_webhook_url: !self.discord.webhook_url.trim().is_empty(),
            },
            webhook: WebhookChannelView {
                enabled: self.webhook.enabled,
                url: self.webhook.url.clone(),
                has_secret: !self.webhook.secret.trim().is_empty(),
            },
            channel_statuses,
        }
    }
}

/// Parche parcial recibido desde `notifications.setConfig`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationConfigPatch {
    pub enabled: Option<bool>,
    pub external_enabled: Option<bool>,
    pub disk_low_threshold_gb: Option<u64>,
    pub events: Option<NotificationEventsConfig>,
    pub quiet_hours: Option<QuietHoursConfig>,
    pub telegram: Option<TelegramChannelPatch>,
    pub ntfy: Option<NtfyChannelPatch>,
    pub discord: Option<DiscordChannelPatch>,
    pub webhook: Option<WebhookChannelPatch>,
}

/// Parche parcial para el canal Telegram.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TelegramChannelPatch {
    pub enabled: Option<bool>,
    pub bot_token: Option<String>,
    pub chat_id: Option<String>,
    pub clear_bot_token: Option<bool>,
}

/// Parche parcial para el canal ntfy.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NtfyChannelPatch {
    pub enabled: Option<bool>,
    pub url: Option<String>,
    pub token: Option<String>,
    pub clear_token: Option<bool>,
}

/// Parche parcial para el canal Discord.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiscordChannelPatch {
    pub enabled: Option<bool>,
    pub webhook_url: Option<String>,
    pub clear_webhook_url: Option<bool>,
}

/// Parche parcial para el canal Webhook genérico.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebhookChannelPatch {
    pub enabled: Option<bool>,
    pub url: Option<String>,
    pub secret: Option<String>,
    pub clear_secret: Option<bool>,
}

/// Vista enmascarada de Telegram devuelta al cliente RPC.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelegramChannelView {
    pub enabled: bool,
    pub chat_id: String,
    pub has_bot_token: bool,
}

/// Vista enmascarada de ntfy devuelta al cliente RPC.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NtfyChannelView {
    pub enabled: bool,
    pub url: String,
    pub has_token: bool,
}

/// Vista enmascarada de Discord devuelta al cliente RPC.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscordChannelView {
    pub enabled: bool,
    pub has_webhook_url: bool,
}

/// Vista enmascarada de Webhook genérico devuelta al cliente RPC.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebhookChannelView {
    pub enabled: bool,
    pub url: String,
    pub has_secret: bool,
}

/// Vista completa de configuración devuelta por `notifications.getConfig` y `notifications.setConfig`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationConfigView {
    pub enabled: bool,
    pub external_enabled: bool,
    pub disk_low_threshold_gb: u64,
    pub events: NotificationEventsConfig,
    pub quiet_hours: QuietHoursConfig,
    pub telegram: TelegramChannelView,
    pub ntfy: NtfyChannelView,
    pub discord: DiscordChannelView,
    pub webhook: WebhookChannelView,
    pub channel_statuses: HashMap<String, ChannelStatus>,
}

/// Último estado observable de entrega de un canal externo.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStatus {
    pub last_attempt_unix_ms: Option<i64>,
    pub last_success_unix_ms: Option<i64>,
    pub last_error: Option<String>,
    pub last_latency_ms: Option<u64>,
}

/// Resultado de una entrega de prueba ejecutada mediante `notifications.test`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDeliveryResult {
    pub channel: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// Hora y minuto actuales en zona local (con respaldo seguro a UTC si el entorno no expone huso).
pub fn current_local_hour_minute() -> (u8, u8) {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    (now.hour(), now.minute())
}

/// Determina si la hora indicada cae dentro de la ventana de horario silencioso.
pub fn is_within_quiet_hours(cfg: &QuietHoursConfig, hour: u8, minute: u8) -> bool {
    if !cfg.enabled {
        return false;
    }
    let cur = u16::from(hour.min(MAX_HOUR_24H)) * MINUTES_PER_HOUR
        + u16::from(minute.min(MAX_MINUTE_HOUR));
    let start = u16::from(cfg.start_hour.min(MAX_HOUR_24H)) * MINUTES_PER_HOUR
        + u16::from(cfg.start_minute.min(MAX_MINUTE_HOUR));
    let end = u16::from(cfg.end_hour.min(MAX_HOUR_24H)) * MINUTES_PER_HOUR
        + u16::from(cfg.end_minute.min(MAX_MINUTE_HOUR));

    if start == end {
        false
    } else if start < end {
        cur >= start && cur < end
    } else {
        cur >= start || cur < end
    }
}

/// Formatea un tamaño en bytes de forma compacta y legible en español.
pub fn format_bytes_human(bytes: u64) -> String {
    if bytes >= BYTES_PER_GIB {
        format!("{:.2} GiB", bytes as f64 / BYTES_PER_GIB as f64)
    } else if bytes >= BYTES_PER_MIB {
        format!("{:.1} MiB", bytes as f64 / BYTES_PER_MIB as f64)
    } else if bytes >= BYTES_PER_KIB {
        format!("{:.0} KiB", bytes as f64 / BYTES_PER_KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Recorta una cadena respetando límites UTF-8 por número máximo de caracteres.
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let mut out: String = trimmed.chars().take(keep).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quiet_hours_overnight_and_daytime_windows() {
        let overnight = QuietHoursConfig {
            enabled: true,
            start_hour: 23,
            start_minute: 0,
            end_hour: 8,
            end_minute: 0,
            allow_critical: true,
        };
        assert!(is_within_quiet_hours(&overnight, 23, 0));
        assert!(is_within_quiet_hours(&overnight, 2, 30));
        assert!(is_within_quiet_hours(&overnight, 7, 59));
        assert!(!is_within_quiet_hours(&overnight, 8, 0));
        assert!(!is_within_quiet_hours(&overnight, 15, 0));

        let daytime = QuietHoursConfig {
            enabled: true,
            start_hour: 14,
            start_minute: 30,
            end_hour: 16,
            end_minute: 15,
            allow_critical: false,
        };
        assert!(!is_within_quiet_hours(&daytime, 14, 29));
        assert!(is_within_quiet_hours(&daytime, 14, 30));
        assert!(is_within_quiet_hours(&daytime, 15, 45));
        assert!(!is_within_quiet_hours(&daytime, 16, 15));

        let disabled = QuietHoursConfig {
            enabled: false,
            ..overnight
        };
        assert!(!is_within_quiet_hours(&disabled, 2, 0));
    }

    #[test]
    fn test_patch_preserves_and_clears_secrets_and_masks_view() {
        let mut cfg = NotificationConfig::default();
        cfg.apply_patch(NotificationConfigPatch {
            enabled: Some(true),
            external_enabled: Some(true),
            disk_low_threshold_gb: Some(5000),
            telegram: Some(TelegramChannelPatch {
                enabled: Some(true),
                bot_token: Some("tg-secret-token".to_string()),
                chat_id: Some("12345".to_string()),
                clear_bot_token: None,
            }),
            discord: Some(DiscordChannelPatch {
                enabled: Some(true),
                webhook_url: Some("https://discord.com/api/webhooks/secret".to_string()),
                clear_webhook_url: None,
            }),
            ..Default::default()
        });

        assert_eq!(cfg.disk_low_threshold_gb, MAX_DISK_LOW_THRESHOLD_GB);
        assert_eq!(cfg.telegram.bot_token, "tg-secret-token");

        // Un parche vacío u omitido conserva los secretos existentes.
        cfg.apply_patch(NotificationConfigPatch {
            telegram: Some(TelegramChannelPatch {
                enabled: Some(false),
                bot_token: Some("   ".to_string()),
                chat_id: None,
                clear_bot_token: None,
            }),
            ..Default::default()
        });
        assert!(!cfg.telegram.enabled);
        assert_eq!(cfg.telegram.bot_token, "tg-secret-token");

        let view_json = serde_json::to_string(&cfg.to_view(HashMap::new())).unwrap();
        assert!(!view_json.contains("tg-secret-token"));
        assert!(!view_json.contains("webhooks/secret"));
        assert!(view_json.contains("\"hasBotToken\":true"));
        assert!(view_json.contains("\"hasWebhookUrl\":true"));

        // Borrado explícito mediante clear_*.
        cfg.apply_patch(NotificationConfigPatch {
            telegram: Some(TelegramChannelPatch {
                clear_bot_token: Some(true),
                ..Default::default()
            }),
            discord: Some(DiscordChannelPatch {
                clear_webhook_url: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        });
        assert!(cfg.telegram.bot_token.is_empty());
        assert!(cfg.discord.webhook_url.is_empty());
        let view = cfg.to_view(HashMap::new());
        assert!(!view.telegram.has_bot_token);
        assert!(!view.discord.has_webhook_url);
    }
}

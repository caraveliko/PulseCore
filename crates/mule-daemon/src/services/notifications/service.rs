// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Servicio central de notificaciones: persistencia, difusión WebSocket y entrega multicanal.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use mule_core::model::NotificationRecord;
use mule_network_policy::NetworkPolicyManager;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, warn};

use crate::services::downloads::types::now_ms;
use crate::services::DaemonEvent;
use crate::state::AppState;

use super::channels::{send_discord, send_ntfy, send_telegram, send_webhook};
use super::constants::{
    DEDUPE_CACHE_MAX_ENTRIES, MAX_NOTIFICATION_BODY_LEN, MAX_NOTIFICATION_TITLE_LEN,
    MAX_STORED_NOTIFICATIONS, SETTING_NOTIFICATIONS_CONFIG, TELEGRAM_API_BASE_URL,
};
use super::error::NotificationError;
use super::types::{
    current_local_hour_minute, is_within_quiet_hours, truncate_chars, ChannelDeliveryResult,
    ChannelStatus, NotificationCategory, NotificationConfig, NotificationConfigPatch,
    NotificationConfigView, NotificationEventInput, NotificationKind, NotificationSeverity,
};

/// Servicio encargado de gestionar reglas, historial y envíos multicanal de notificaciones.
pub struct NotificationService {
    config: RwLock<NotificationConfig>,
    channel_statuses: RwLock<HashMap<String, ChannelStatus>>,
    dedupe_cache: Mutex<HashMap<String, Instant>>,
    telegram_api_base: RwLock<String>,
}

impl NotificationService {
    /// Crea el servicio cargando la configuración JSON persistida en SQLite si existe.
    pub fn new_from_setting(raw_json: Option<&str>) -> Self {
        let cfg = match raw_json {
            Some(raw) if !raw.trim().is_empty() => match serde_json::from_str::<NotificationConfig>(raw) {
                Ok(parsed) => parsed,
                Err(err) => {
                    warn!("Configuración de notificaciones inválida en SQLite, usando valores predeterminados: {err}");
                    NotificationConfig::default()
                }
            },
            _ => NotificationConfig::default(),
        };

        Self {
            config: RwLock::new(cfg),
            channel_statuses: RwLock::new(HashMap::new()),
            dedupe_cache: Mutex::new(HashMap::new()),
            telegram_api_base: RwLock::new(TELEGRAM_API_BASE_URL.to_string()),
        }
    }

    /// Recarga en memoria la configuración tras restaurar una copia de seguridad.
    pub async fn reload_from_json(&self, raw_json: &str) {
        if raw_json.trim().is_empty() {
            return;
        }
        match serde_json::from_str::<NotificationConfig>(raw_json) {
            Ok(parsed) => {
                *self.config.write().await = parsed;
            }
            Err(err) => {
                warn!("No se pudo recargar la configuración de notificaciones tras restaurar copia: {err}");
            }
        }
    }

    /// Sobrescribe la URL base de Telegram (útil exclusivamente para pruebas loopback sin conexión).
    pub async fn set_telegram_api_base(&self, base_url: String) {
        *self.telegram_api_base.write().await = base_url;
    }

    /// Devuelve el umbral configurado de espacio libre en disco (GiB).
    pub async fn current_disk_low_threshold_gb(&self) -> u64 {
        self.config.read().await.disk_low_threshold_gb
    }

    /// Devuelve una vista segura de la configuración actual sin exponer secretos.
    pub async fn get_config_view(&self) -> NotificationConfigView {
        let cfg = self.config.read().await.clone();
        let statuses = self.channel_statuses.read().await.clone();
        cfg.to_view(statuses)
    }

    /// Aplica un parche parcial sobre la configuración y la persiste en SQLite.
    pub async fn update_config(
        &self,
        state: &AppState,
        patch: NotificationConfigPatch,
    ) -> Result<NotificationConfigView, NotificationError> {
        let updated = {
            let mut cfg = self.config.write().await;
            cfg.apply_patch(patch);
            cfg.clone()
        };

        let serialized = serde_json::to_string(&updated)
            .map_err(|e| NotificationError::Serialization(e.to_string()))?;

        {
            let mut dl = state.downloads.lock().await;
            dl.set_setting(SETTING_NOTIFICATIONS_CONFIG, &serialized)
                .map_err(|e| NotificationError::Storage(e.to_string()))?;
        }

        let statuses = self.channel_statuses.read().await.clone();
        Ok(updated.to_view(statuses))
    }

    /// Persiste una notificación, la difunde por WebSocket y la despacha a canales externos activos.
    pub async fn notify(
        &self,
        state: &AppState,
        input: NotificationEventInput,
    ) -> Result<Option<NotificationRecord>, NotificationError> {
        let cfg = self.config.read().await.clone();
        if !cfg.enabled {
            return Ok(None);
        }
        if input.kind != NotificationKind::Test && !cfg.is_kind_enabled(input.kind) {
            return Ok(None);
        }

        if let Some(ref key) = input.dedupe_key {
            if input.cooldown_secs > 0 && !self.try_acquire_dedupe(key, input.cooldown_secs).await {
                debug!(dedupe_key = %key, "Notificación omitida por ventana de enfriamiento");
                return Ok(None);
            }
        }

        let metadata_json = input
            .metadata
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let record = NotificationRecord {
            id: 0,
            kind: input.kind.as_str().to_string(),
            severity: input.severity.as_str().to_string(),
            category: input.category.as_str().to_string(),
            title: truncate_chars(&input.title, MAX_NOTIFICATION_TITLE_LEN),
            body: truncate_chars(&input.body, MAX_NOTIFICATION_BODY_LEN),
            metadata_json,
            is_read: false,
            created_at_unix_ms: now_ms(),
        };

        let saved = {
            let mut dl = state.downloads.lock().await;
            let db = dl.db_mut();
            let inserted = db
                .insert_notification(&record)
                .map_err(|e| NotificationError::Storage(e.to_string()))?;
            if let Err(err) = db.prune_old_notifications(MAX_STORED_NOTIFICATIONS) {
                warn!("No se pudo podar el historial de notificaciones: {err}");
            }
            inserted
        };

        state.emit_event(|sequence| DaemonEvent::NotificationCreated {
            sequence,
            notification: saved.clone(),
        });

        let (hour, minute) = current_local_hour_minute();
        let in_quiet = is_within_quiet_hours(&cfg.quiet_hours, hour, minute);
        let allow_external =
            cfg.external_enabled && (!in_quiet || (cfg.quiet_hours.allow_critical && input.severity.is_critical()));

        if allow_external {
            self.spawn_external_dispatch(state, cfg, saved.clone()).await;
        }

        Ok(Some(saved))
    }

    /// Ejecuta una prueba directa de envío sobre el canal indicado (`webui`, `telegram`, `ntfy`, `discord`, `webhook`).
    pub async fn send_test(
        &self,
        state: &AppState,
        channel: &str,
    ) -> Result<ChannelDeliveryResult, NotificationError> {
        let normalized = channel.trim().to_ascii_lowercase();
        if normalized == "webui" {
            let started = Instant::now();
            let record = NotificationRecord {
                id: 0,
                kind: NotificationKind::Test.as_str().to_string(),
                severity: NotificationSeverity::Info.as_str().to_string(),
                category: NotificationCategory::System.as_str().to_string(),
                title: "Prueba de notificaciones".to_string(),
                body: "El centro de notificaciones de Pulse Core funciona correctamente.".to_string(),
                metadata_json: None,
                is_read: false,
                created_at_unix_ms: now_ms(),
            };
            let saved = {
                let mut dl = state.downloads.lock().await;
                let db = dl.db_mut();
                let inserted = db
                    .insert_notification(&record)
                    .map_err(|e| NotificationError::Storage(e.to_string()))?;
                let _ = db.prune_old_notifications(MAX_STORED_NOTIFICATIONS);
                inserted
            };
            state.emit_event(|sequence| DaemonEvent::NotificationCreated {
                sequence,
                notification: saved,
            });
            return Ok(ChannelDeliveryResult {
                channel: "webui".to_string(),
                ok: true,
                latency_ms: started.elapsed().as_millis() as u64,
                error: None,
            });
        }

        let cfg = self.config.read().await.clone();
        let tg_base = self.telegram_api_base.read().await.clone();
        let test_record = NotificationRecord {
            id: 0,
            kind: NotificationKind::Test.as_str().to_string(),
            severity: NotificationSeverity::Success.as_str().to_string(),
            category: NotificationCategory::System.as_str().to_string(),
            title: "Prueba de Pulse Core".to_string(),
            body: "Este es un aviso de verificación enviado desde la configuración de notificaciones."
                .to_string(),
            metadata_json: None,
            is_read: false,
            created_at_unix_ms: now_ms(),
        };

        let result = dispatch_single_channel(
            &normalized,
            &cfg,
            &tg_base,
            &test_record,
            &state.network_policy,
        )
        .await;

        if matches!(result, Err(NotificationError::InvalidChannel(_))) {
            return result;
        }

        let delivery = match result {
            Ok(res) => res,
            Err(err) => return Err(err),
        };

        self.record_channel_status(&delivery).await;
        Ok(delivery)
    }

    async fn try_acquire_dedupe(&self, key: &str, cooldown_secs: u64) -> bool {
        let mut cache = self.dedupe_cache.lock().await;
        let cooldown = Duration::from_secs(cooldown_secs);
        let now = Instant::now();
        if let Some(prev) = cache.get(key) {
            if now.saturating_duration_since(*prev) < cooldown {
                return false;
            }
        }
        if cache.len() >= DEDUPE_CACHE_MAX_ENTRIES {
            cache.retain(|_, ts| now.saturating_duration_since(*ts) < cooldown);
            if cache.len() >= DEDUPE_CACHE_MAX_ENTRIES {
                cache.clear();
            }
        }
        cache.insert(key.to_string(), now);
        true
    }

    async fn record_channel_status(&self, delivery: &ChannelDeliveryResult) {
        let now = now_ms();
        let mut statuses = self.channel_statuses.write().await;
        let entry = statuses.entry(delivery.channel.clone()).or_default();
        entry.last_attempt_unix_ms = Some(now);
        entry.last_latency_ms = Some(delivery.latency_ms);
        if delivery.ok {
            entry.last_success_unix_ms = Some(now);
            entry.last_error = None;
        } else {
            entry.last_error = delivery.error.clone();
        }
    }

    async fn spawn_external_dispatch(
        &self,
        state: &AppState,
        cfg: NotificationConfig,
        record: NotificationRecord,
    ) {
        let mut targets: Vec<&'static str> = Vec::new();
        if cfg.telegram.enabled {
            targets.push("telegram");
        }
        if cfg.ntfy.enabled {
            targets.push("ntfy");
        }
        if cfg.discord.enabled {
            targets.push("discord");
        }
        if cfg.webhook.enabled {
            targets.push("webhook");
        }
        if targets.is_empty() {
            return;
        }

        let tg_base = self.telegram_api_base.read().await.clone();
        let policy = state.network_policy.clone();
        let svc = state.notifications.clone();

        tokio::spawn(async move {
            for channel in targets {
                match dispatch_single_channel(channel, &cfg, &tg_base, &record, &policy).await {
                    Ok(res) => {
                        if !res.ok {
                            warn!(
                                channel = %res.channel,
                                error = ?res.error,
                                "Error al entregar notificación externa"
                            );
                        }
                        svc.record_channel_status(&res).await;
                    }
                    Err(err) => {
                        warn!(channel = %channel, "Fallo inesperado despachando canal: {err}");
                    }
                }
            }
        });
    }
}

async fn dispatch_single_channel(
    channel: &str,
    cfg: &NotificationConfig,
    tg_base: &str,
    record: &NotificationRecord,
    network_policy: &NetworkPolicyManager,
) -> Result<ChannelDeliveryResult, NotificationError> {
    let started = Instant::now();
    let res = match channel {
        "telegram" => send_telegram(tg_base, &cfg.telegram, record, network_policy).await,
        "ntfy" => send_ntfy(&cfg.ntfy, record, network_policy).await,
        "discord" => send_discord(&cfg.discord, record, network_policy).await,
        "webhook" => send_webhook(&cfg.webhook, record, network_policy).await,
        other => return Err(NotificationError::InvalidChannel(other.to_string())),
    };
    let latency_ms = started.elapsed().as_millis() as u64;
    Ok(match res {
        Ok(()) => ChannelDeliveryResult {
            channel: channel.to_string(),
            ok: true,
            latency_ms,
            error: None,
        },
        Err(err) => ChannelDeliveryResult {
            channel: channel.to_string(),
            ok: false,
            latency_ms,
            error: Some(err.to_string()),
        },
    })
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Subsistema modular de notificaciones de Pulse Core (WebUI, historial SQLite y canales push).

pub mod channels;
pub mod constants;
pub mod error;
pub mod http_client;
pub mod monitor;
pub mod service;
pub mod types;

pub use monitor::start_notification_monitor;
pub use service::NotificationService;
pub use types::{
    ChannelDeliveryResult, ChannelStatus, NotificationCategory, NotificationConfig,
    NotificationConfigPatch, NotificationConfigView, NotificationEventInput, NotificationKind,
    NotificationSeverity,
};

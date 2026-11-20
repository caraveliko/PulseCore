// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Constantes y límites operativos del sistema de notificaciones de Pulse Core.

/// Clave de ajuste persistida en SQLite para la configuración de notificaciones.
pub const SETTING_NOTIFICATIONS_CONFIG: &str = "notifications_config_v1";

/// Número máximo de notificaciones conservadas en la base de datos SQLite.
pub const MAX_STORED_NOTIFICATIONS: usize = 200;

/// Límite predeterminado de notificaciones devueltas por `notifications.list`.
pub const DEFAULT_NOTIFICATION_LIST_LIMIT: usize = 50;

/// Límite máximo de notificaciones que puede solicitar `notifications.list`.
pub const MAX_NOTIFICATION_LIST_LIMIT: usize = 200;

/// Longitud máxima en caracteres permitida para el título de una notificación.
pub const MAX_NOTIFICATION_TITLE_LEN: usize = 160;

/// Longitud máxima en caracteres permitida para el cuerpo de una notificación.
pub const MAX_NOTIFICATION_BODY_LEN: usize = 1_000;

/// Tamaño máximo leído del cuerpo de error HTTP devuelto por un canal externo.
pub const MAX_ERROR_RESPONSE_BYTES: usize = 4_096;

/// Tiempo máximo de espera por petición HTTP hacia un canal externo (segundos).
pub const DISPATCH_HTTP_TIMEOUT_SECS: u64 = 10;

/// Número máximo de reintentos ante fallos transitorios de red o servidor (429 / 5xx).
pub const MAX_EXTERNAL_RETRIES: u32 = 2;

/// Base de retroceso exponencial entre reintentos de envío externo (milisegundos).
pub const RETRY_BACKOFF_BASE_MS: u64 = 400;

/// Umbral predeterminado de espacio libre mínimo en disco antes de alertar (GiB).
pub const DEFAULT_DISK_LOW_THRESHOLD_GB: u64 = 5;

/// Mínimo configurable para el umbral de espacio libre en disco (GiB).
pub const MIN_DISK_LOW_THRESHOLD_GB: u64 = 1;

/// Máximo configurable para el umbral de espacio libre en disco (GiB).
pub const MAX_DISK_LOW_THRESHOLD_GB: u64 = 1_000;

/// Retardo inicial tras el arranque antes de la primera comprobación de espacio en disco (segundos).
pub const DISK_INITIAL_DELAY_SECS: u64 = 60;

/// Intervalo entre comprobaciones periódicas de espacio libre en disco y NAS (segundos).
pub const DISK_CHECK_INTERVAL_SECS: u64 = 15 * 60;

/// Enfriamiento mínimo entre alertas repetidas de espacio bajo para un mismo volumen (segundos).
pub const DISK_LOW_COOLDOWN_SECS: u64 = 6 * 3_600;

/// Enfriamiento mínimo entre alertas repetidas de desconexión del servidor eD2K (segundos).
pub const SERVER_DISCONNECT_COOLDOWN_SECS: u64 = 10 * 60;

/// Cadencia de sondeo del estado de torrents completados en el monitor de notificaciones (segundos).
pub const TORRENT_POLL_INTERVAL_SECS: u64 = 10;

/// Capacidad máxima de la caché en memoria para deduplicación con enfriamiento.
pub const DEDUPE_CACHE_MAX_ENTRIES: usize = 256;

/// Hora de inicio predeterminada del horario silencioso (23:00).
pub const DEFAULT_QUIET_START_HOUR: u8 = 23;

/// Minuto de inicio predeterminado del horario silencioso.
pub const DEFAULT_QUIET_START_MINUTE: u8 = 0;

/// Hora de fin predeterminada del horario silencioso (08:00).
pub const DEFAULT_QUIET_END_HOUR: u8 = 8;

/// Minuto de fin predeterminado del horario silencioso.
pub const DEFAULT_QUIET_END_MINUTE: u8 = 0;

/// URL base oficial de la API de bots de Telegram.
pub const TELEGRAM_API_BASE_URL: &str = "https://api.telegram.org";

/// Color hexadecimal del *embed* de Discord para notificaciones exitosas.
pub const DISCORD_EMBED_COLOR_SUCCESS: u32 = 0x22C55E;

/// Color hexadecimal del *embed* de Discord para notificaciones informativas.
pub const DISCORD_EMBED_COLOR_INFO: u32 = 0x38BDF8;

/// Color hexadecimal del *embed* de Discord para advertencias.
pub const DISCORD_EMBED_COLOR_WARNING: u32 = 0xF59E0B;

/// Color hexadecimal del *embed* de Discord para errores críticos.
pub const DISCORD_EMBED_COLOR_ERROR: u32 = 0xEF4444;

/// Número de bytes en un gibibyte (GiB).
pub const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

/// Número de bytes en un mebibyte (MiB).
pub const BYTES_PER_MIB: u64 = 1024 * 1024;

/// Número de bytes en un kibibyte (KiB).
pub const BYTES_PER_KIB: u64 = 1024;

/// Código de estado HTTP correspondiente a exceso de peticiones (*Too Many Requests*).
pub const HTTP_TOO_MANY_REQUESTS: u16 = 429;

/// Código de estado HTTP mínimo correspondiente a errores del servidor (`5xx`).
pub const HTTP_SERVER_ERROR_MIN: u16 = 500;

/// Hora máxima válida en formato 24 horas.
pub const MAX_HOUR_24H: u8 = 23;

/// Minuto máximo válido en una hora.
pub const MAX_MINUTE_HOUR: u8 = 59;

/// Minutos contenidos en una hora.
pub const MINUTES_PER_HOUR: u16 = 60;

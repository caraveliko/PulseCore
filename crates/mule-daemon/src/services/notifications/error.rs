// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Tipos de error tipados para el subsistema de notificaciones.

use thiserror::Error;

/// Errores producidos al validar, persistir o entregar notificaciones.
#[derive(Debug, Error)]
pub enum NotificationError {
    /// Bloqueado por la política de red anti-SSRF o el interruptor de corte (*kill switch*).
    #[error("Envío bloqueado por la política de red: {0}")]
    NetworkBlocked(String),

    /// El nombre del canal de notificación indicado no es reconocido.
    #[error("Canal de notificación no válido: {0}")]
    InvalidChannel(String),

    /// El canal seleccionado carece de la configuración mínima obligatoria.
    #[error("Canal sin configurar: {0}")]
    NotConfigured(String),

    /// El punto de enlace remoto devolvió un código de estado HTTP no satisfactorio.
    #[error("Error HTTP {status} desde el canal remoto: {detail}")]
    HttpStatus {
        /// Código de estado HTTP recibido.
        status: u16,
        /// Extracto acotado de la respuesta de error.
        detail: String,
    },

    /// Error de transporte o conexión HTTP.
    #[error("Error de transporte HTTP: {0}")]
    Transport(#[from] reqwest::Error),

    /// Error al leer o persistir en la base de datos local.
    #[error("Error de almacenamiento local: {0}")]
    Storage(String),

    /// Error al serializar la carga útil o configuración.
    #[error("Error de serialización: {0}")]
    Serialization(String),
}

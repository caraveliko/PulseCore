// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde::{Deserialize, Serialize};

/// Estado actual del ciclo de vida de una conexión a un servidor eD2K.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerConnectionStatus {
    /// Ninguna conexión activa.
    Disconnected,
    /// Apretón de manos TCP y solicitud de inicio de sesión en curso.
    Connecting,
    /// Sesión iniciada con éxito y listo para operar.
    Connected,
    /// Desconexión en curso.
    Disconnecting,
}

impl ServerConnectionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Disconnecting => "disconnecting",
        }
    }
}

/// Instantánea de la sesión actual con el servidor eD2K.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServerSessionSnapshot {
    /// Estado actual de la conexión.
    pub status: ServerConnectionStatus,
    /// Dirección del servidor ("ip:puerto") si está conectado o conectando.
    pub server_address: Option<String>,
    /// Nombre del servidor si se recibió en la identificación o saludo.
    pub server_name: Option<String>,
    /// Descripción del servidor.
    pub server_desc: Option<String>,
    /// ID de cliente asignada por el servidor.
    pub client_id: Option<u32>,
    /// Verdadero si el ID de cliente es >= 16_777_216 (HighID).
    pub is_high_id: bool,
    /// Mensaje de bienvenida / MOTD del servidor.
    pub welcome_message: Option<String>,
    /// Usuarios activos notificados por el servidor.
    pub current_users: Option<u32>,
    /// Archivos activos notificados por el servidor.
    pub current_files: Option<u32>,
    /// Marca temporal de finalización de la conexión (milisegundos unix).
    pub connected_at_unix_ms: Option<i64>,
    /// Estimación del tiempo de ida y vuelta (ping / RTT) en milisegundos.
    pub ping_ms: Option<u32>,
}

impl Default for ServerSessionSnapshot {
    fn default() -> Self {
        Self {
            status: ServerConnectionStatus::Disconnected,
            server_address: None,
            server_name: None,
            server_desc: None,
            client_id: None,
            is_high_id: false,
            welcome_message: None,
            current_users: None,
            current_files: None,
            connected_at_unix_ms: None,
            ping_ms: None,
        }
    }
}

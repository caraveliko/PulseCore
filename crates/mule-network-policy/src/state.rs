// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde::{Deserialize, Serialize};

/// Estado de alto nivel del ciclo de vida del subsistema de red.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkState {
    /// La red está completamente deshabilitada (interruptor de corte activado, o no inicializada).
    /// No hay sockets activos y no se permiten nuevos intentos de conexión.
    Disabled,
    /// La política está activa y lista en modo de vista previa; actualmente no hay conexiones activas abiertas.
    #[default]
    PreviewActive,
    /// Una o más conexiones están activamente abiertas y operando dentro de la política.
    Connected,
    /// El interruptor de corte o el apagado ordenado están activamente en curso.
    ShuttingDown,
    /// El subsistema de red encontró un error fatal o un fallo de presupuesto irrecuperable.
    Error,
}

impl NetworkState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::PreviewActive => "preview-active",
            Self::Connected => "connected",
            Self::ShuttingDown => "shutting-down",
            Self::Error => "error",
        }
    }

    /// Indica si este estado permite solicitar nuevos permisos de socket.
    pub fn is_operational(&self) -> bool {
        matches!(self, Self::PreviewActive | Self::Connected)
    }
}

impl std::fmt::Display for NetworkState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

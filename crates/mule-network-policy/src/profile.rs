// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde::{Deserialize, Serialize};

/// Perfil de conectividad que define restricciones de límites de direcciones y capacidades protocolarias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkProfile {
    /// Confinamiento estricto de laboratorio: solo se permiten direcciones de bucle local (loopback) (127.0.0.0/8, ::1).
    LocalLab,
    /// Vista previa controlada: permite la conexión a servidores y pares eD2K públicos.
    /// Rechaza estrictamente LAN privadas, loopback, enlace local, multidifusión, etc. (anti-SSRF).
    PublicEd2kPreview,
    /// Vista previa controlada con soporte UDP para Kademlia.
    /// Mismas reglas de filtrado de direcciones que PublicEd2kPreview.
    #[default]
    PublicEd2kKadPreview,
}

impl std::fmt::Display for NetworkProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LocalLab => write!(f, "local-lab"),
            Self::PublicEd2kPreview => write!(f, "public-ed2k-preview"),
            Self::PublicEd2kKadPreview => write!(f, "public-ed2k-kad-preview"),
        }
    }
}

impl std::str::FromStr for NetworkProfile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "local-lab" | "locallab" | "lab" => Ok(Self::LocalLab),
            "public-ed2k-preview" | "ed2k-preview" => Ok(Self::PublicEd2kPreview),
            "public-ed2k-kad-preview" | "kad-preview" => Ok(Self::PublicEd2kKadPreview),
            other => Err(format!("Unknown network profile: '{other}'")),
        }
    }
}

impl NetworkProfile {
    /// Indica si este perfil restringe todo el tráfico exclusivamente a direcciones de bucle local (loopback).
    pub fn is_loopback_only(&self) -> bool {
        matches!(self, Self::LocalLab)
    }

    /// Indica si este perfil permite operaciones UDP para el enrutamiento de Kademlia.
    pub fn allows_udp_kad(&self) -> bool {
        matches!(self, Self::LocalLab | Self::PublicEd2kKadPreview)
    }

    /// Indica si este perfil es un perfil de vista previa externa.
    pub fn is_preview(&self) -> bool {
        matches!(self, Self::PublicEd2kPreview | Self::PublicEd2kKadPreview)
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

//! Configuración tipada, errores e instantánea honesta de capacidades BitTorrent.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Límite predeterminado de pares simultáneos por descarga torrent.
pub const DEFAULT_TORRENT_PEER_LIMIT: usize = 32;
/// Cadena identificadora de cliente anunciada en cabeceras HTTP y extensiones BitTorrent.
pub const DEFAULT_TORRENT_CLIENT_NAME: &str = "PulseCore/2026";

/// Errores tipados del motor BitTorrent integrado.
#[derive(Debug, Error)]
pub enum TorrentError {
    /// La política de red impide iniciar o consultar BitTorrent (interruptor de corte o perfil).
    #[error("motor BitTorrent denegado por política de red: {0}")]
    PolicyDenied(String),
    /// URL HTTP/HTTPS bloqueada por la protección anti-SSRF o por `ipfilter`.
    #[error("URL BitTorrent rechazada por seguridad (anti-SSRF): {0}")]
    SsrfRejected(String),
    /// Ruta de salida o persistencia no válida.
    #[error("ruta BitTorrent inválida: {0}")]
    InvalidPath(String),
    /// Fallo interno de la sesión o de E/S en el motor `librqbit`.
    #[error("error de sesión BitTorrent: {0}")]
    Session(String),
}

/// Parámetros de arranque del motor BitTorrent gobernados por la política de red del daemon.
#[derive(Debug, Clone)]
pub struct TorrentPolicyConfig {
    /// Carpeta predeterminada donde se guardan las descargas BitTorrent.
    pub default_output_dir: PathBuf,
    /// Directorio donde se escriben la lista de bloqueo `.p2p` exportada y `fastresume`.
    pub blocklist_dir: PathBuf,
    /// Activa `fastresume` y persistencia de sesión en perfiles públicos.
    pub fastresume: bool,
    /// Habilita transporte uTP (BEP 29) junto con TCP cuando el perfil lo permite.
    pub enable_utp: bool,
    /// Habilita escucha Dual-Stack IPv6+IPv4 (`ipv4_only: false`).
    pub ipv6_dual_stack: bool,
    /// Coordina el reenvío de puertos UPnP únicamente si UPnP está activo en el daemon.
    pub enable_upnp: bool,
    /// Puerto de escucha opcional (si es `None` o `0`, elige un puerto efímero).
    pub listen_port: Option<u16>,
    /// Máximo de pares simultáneos por torrent.
    pub peer_limit: usize,
    /// Identificador opcional de cliente BitTorrent.
    pub client_name_and_version: Option<String>,
}

impl TorrentPolicyConfig {
    /// Construye la configuración recomendada para 2026 con valores seguros por defecto.
    pub fn new(default_output_dir: PathBuf, blocklist_dir: PathBuf) -> Self {
        Self {
            default_output_dir,
            blocklist_dir,
            fastresume: true,
            enable_utp: true,
            ipv6_dual_stack: true,
            enable_upnp: false,
            listen_port: None,
            peer_limit: DEFAULT_TORRENT_PEER_LIMIT,
            client_name_and_version: Some(DEFAULT_TORRENT_CLIENT_NAME.to_string()),
        }
    }
}

/// Instantánea verificable de las capacidades activas del motor BitTorrent.
///
/// Refleja honestamente lo soportado en tiempo de ejecución sin exagerar capacidades
/// ausentes en la versión enlazada de `librqbit`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TorrentCapabilitiesSnapshot {
    /// Transporte uTP (BEP 29) habilitado junto a TCP.
    pub utp_enabled: bool,
    /// Escucha Dual-Stack IPv6+IPv4 operativa.
    pub ipv6_dual_stack: bool,
    /// Persistencia `fastresume` activa.
    pub fastresume_enabled: bool,
    /// Número de rangos exportados desde `ipfilter` cargados en la lista de bloqueo.
    pub ip_filter_entries: usize,
    /// Coordinación de reenvío UPnP activa según la configuración del daemon.
    pub upnp_coordinated: bool,
    /// Motor aislado estrictamente en loopback (sin DHT, rastreadores ni escucha externa).
    pub loopback_only: bool,
    /// Cifrado clásico de flujo MSE/PE (BEP 8) — `false` de forma explícita porque `librqbit 9.0.1` no lo implementa.
    pub mse_pe_supported: bool,
}

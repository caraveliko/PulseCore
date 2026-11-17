// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Tipos y constantes del receptor de conexiones entrantes.

use crate::services::identity::SuiStats;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::services::downloads::DownloadService;
use crate::services::upload_queue::UploadManager;
use mule_network_policy::NetworkPolicyManager;

/// Conexiones entrantes simultáneas máximas desde una sola IP (anti-inundación).
/// Nota: el límite global entrante no es una constante — sigue el
/// presupuesto `max_active_peer_connections` (en vivo), como en eMule donde todas las
/// conexiones de pares cuentan en `maxconnections`.
pub const MAX_INBOUND_PER_IP: usize = 5;

/// Tamaño máximo de búfer para el entramado de conexiones entrantes (256 KiB) para evitar OOM.
pub const MAX_READ_BUFFER_SIZE: usize = 256 * 1024;

/// Instantánea del estado del servicio receptor de conexiones entrantes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InboundListenerSnapshot {
    /// Indica si el receptor TCP está actualmente en ejecución.
    pub is_listening: bool,
    /// Dirección del socket de escucha si está activo.
    pub listen_address: Option<String>,
    /// Número de subidas activas que transfieren actualmente.
    pub active_uploads: usize,
    /// Número de pares en cola esperando una ranura de subida.
    pub queued_uploads: usize,
    /// Velocidad de subida actual en bytes por segundo.
    pub upload_rate_bps: u64,
}

/// Estado del servicio receptor TCP eD2K entrante.
#[derive(Clone)]
pub struct InboundListenerService {
    pub(crate) network_policy: Arc<NetworkPolicyManager>,
    pub(crate) downloads: Arc<Mutex<DownloadService>>,
    pub(crate) upload_manager: UploadManager,
    pub(crate) is_running: Arc<AtomicBool>,
    pub(crate) listen_addr: Arc<Mutex<Option<SocketAddr>>>,
    pub(crate) listener_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// User hash eD2K local — modificable en caliente: `identityRotationHours`
    /// regenera la identidad sin reiniciar el oyente.
    pub(crate) user_hash: std::sync::Arc<std::sync::RwLock<[u8; 16]>>,
    /// DER de clave pública RSA-384 local (PKCS#1) usada como clave del desafiador en cargas
    /// útiles SUI. `None` cuando el daemon no tiene identidad utilizable (p. ej. cryptkey.dat
    /// en cuarentena): no se deben emitir desafíos sin ella. Modificable en
    /// caliente para seguir la rotación de identidad.
    pub(crate) sui_pubkey_der: std::sync::Arc<std::sync::RwLock<Option<Vec<u8>>>>,
    /// Modo «paranoico» (propio de eMuleRust): solo concede ranuras de subida
    /// a pares ofuscados Y cuya firma SUI haya sido verificada.
    pub(crate) paranoid_mode: Arc<AtomicBool>,
    /// Identidad local (acceso a clave privada) para responder a desafíos de pares
    /// entrantes — el par firma nuestra clave, nosotros firmamos su clave.
    pub(crate) identity: Option<Arc<Mutex<crate::services::IdentityService>>>,
    /// Anti-inundación: IP de la última firma procesada (`sigIP` de eMule).
    pub(crate) last_sig_ip: Arc<Mutex<Option<IpAddr>>>,
    /// Contadores de anomalías SUI compartidos (objeto `sui` en identity.status).
    pub(crate) sui_stats: Arc<SuiStats>,
    pub(crate) active_conns: Arc<Mutex<HashMap<IpAddr, usize>>>,
    pub(crate) obfuscation_enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(crate) obfuscation_required: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(crate) obfuscation_padding_max: u8,
    /// Gestor de sesiones de pares para adoptar conexiones entrantes que responden a
    /// una devolución LowID pendiente (`OP_CALLBACKREQUEST` retransmitido por el servidor).
    pub(crate) peer_session:
        Arc<std::sync::RwLock<Option<crate::services::peer_session::PeerSessionManager>>>,
    /// Gestor de sesiones de servidor — proporciona nuestra información de HighID `client_id`/servidor
    /// al adoptar conexiones de callback.
    pub(crate) server_session:
        Arc<std::sync::RwLock<Option<crate::services::server_session::ServerSessionManager>>>,

    /// Servicio UDP de cliente — usado para registrar las versiones UDP anunciadas por clientes
    /// entrantes para respuestas `OP_REASKACK` correctamente conformadas.
    pub(crate) client_udp:
        Arc<std::sync::RwLock<Option<crate::services::client_udp::ClientUdpService>>>,

    /// Servicio UDP de Kad — responde a `OP_FWCHECKUDPREQ` emitiendo datagramas de
    /// sondeo `KADEMLIA2_FIREWALLUDP` en el socket UDP compartido.
    pub(crate) kad_udp:
        Arc<std::sync::RwLock<Option<Arc<crate::services::kad_udp::KadUdpService>>>>,

    /// Servicio Kad — los pares entrantes que anuncian un puerto UDP Kad pueden sembrar
    /// un arranque de la tabla de enrutamiento ("bootstrap from clients" de eMule).
    pub(crate) kad: Arc<std::sync::RwLock<Option<Arc<crate::services::kad::KadService>>>>,

    /// Nuestro puerto UDP de Kad, anunciado en `CT_EMULE_UDPPORTS` (palabra alta) y en
    /// `OP_EMULEINFO` `ET_UDPPORT` en respuestas de apretón de manos entrantes.
    pub(crate) kad_udp_port: u16,

    /// Source Exchange compartido (política + registros de limitación de tasa de eMule
    /// `SetLastSrcReqTime`/`GetLastAskedForSources`) — inyectado por `AppState`.
    pub(crate) sx_env: std::sync::Arc<
        std::sync::RwLock<Option<crate::services::peer_session::source_exchange::SxEnv>>,
    >,
}

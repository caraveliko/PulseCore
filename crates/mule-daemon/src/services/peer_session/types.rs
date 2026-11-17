// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_core::model::PeerFailureReason;
use mule_protocol::ed2k::InFlightBlock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use mule_network_policy::SocketPermit;

/// Máximo de conexiones activas de pares por hash de archivo (límite de seguridad estricto).
pub const MAX_PEERS_PER_DOWNLOAD: usize = 400;

/// Máximo de conexiones activas de pares en todo el daemon.
pub const MAX_GLOBAL_PEERS: usize = 500;

/// Capacidad máxima del búfer de lectura para datos entrantes del par (256 KiB) para evitar OOM.
pub const MAX_PEER_READ_BUFFER_SIZE: usize = 256 * 1024;

/// Intervalo mínimo entre dos eventos `DownloadUpdated`/`PeerSessionUpdated`
/// emitidos para un mismo archivo por la ruta rápida de transferencia: las sesiones
/// múltiples de una descarga comparten un presupuesto de emisión (la
/// serialización JSON de la instantánea — `parts` + `pending_ranges` — es el
/// coste dominante por evento, que de otro modo se pagaría en cada sesión).
pub(crate) const DOWNLOAD_EVENT_INTERVAL: Duration = Duration::from_secs(1);

/// Tiempos de espera granulares configurables para conexiones de pares eD2K.
#[derive(Debug, Clone, Copy)]
pub struct PeerTimeouts {
    pub connect_timeout: Duration,
    pub handshake_timeout: Duration,
    pub status_timeout: Duration,
    pub queue_timeout: Duration,
    pub first_data_timeout: Duration,
    pub inactivity_timeout: Duration,
}

impl Default for PeerTimeouts {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            handshake_timeout: Duration::from_secs(8),
            status_timeout: Duration::from_secs(20),
            // `CClientReqSocket::CheckTimeOut` de eMule (`ListenSocket.cpp:120`)
            // cierra el socket tras `CONNECTION_TIMEOUT` (40 s,
            // `Opcodes.h:64`) sin NINGÚN envío o recepción — el estado en cola
            // sobrevive lógicamente (`DS_ONQUEUE` con `socket == NULL`).
            queue_timeout: Duration::from_secs(40),
            // DOWNLOADTIMEOUT de eMule es ~100 s (medido desde
            // `m_dwLastBlockReceived`) — un límite más lento descarta fuentes lentas por goteo.
            first_data_timeout: Duration::from_secs(100),
            inactivity_timeout: Duration::from_secs(100),
        }
    }
}

/// Calcula el tiempo de enfriamiento con retroceso exponencial en milisegundos según fail_count.
/// Fórmula: min(1_200_000, 2^(fail_count - 1) * 60_000) (1m, 2m, 4m, 8m... máx 20 min)
pub fn calculate_backoff_cooldown_ms(fail_count: u32) -> i64 {
    if fail_count == 0 {
        return 0;
    }
    let exponent = (fail_count - 1).min(5);
    let base = 60_000i64;
    let factor = 1i64 << exponent;
    (base * factor).min(1_200_000)
}

/// Estado actual del ciclo de vida de una conexión de par eD2K.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PeerConnectionStatus {
    /// Ninguna conexión de par activa.
    Disconnected,
    /// Conexión de socket TCP en curso.
    Connecting,
    /// Apretón de manos (OP_HELLO / OP_HELLO_ANSWER) en curso.
    Handshaking,
    /// En cola en el par remoto (OP_QUEUERANKING / esperando ranura de subida).
    Queued,
    /// Descarga activa de bloques (OP_REQUESTPARTS / OP_SENDINGPART).
    Transferring,
    /// Desmantelamiento en curso.
    Disconnecting,
}

impl PeerConnectionStatus {
    /// Representación en cadena.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Handshaking => "handshaking",
            Self::Queued => "queued",
            Self::Transferring => "transferring",
            Self::Disconnecting => "disconnecting",
        }
    }
}

/// Instantánea de una sesión individual de par eD2K.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PeerSessionSnapshot {
    /// Estado actual de la conexión.
    pub status: PeerConnectionStatus,
    /// Hash del archivo de destino en hexadecimal.
    pub file_hash: Option<String>,
    /// Dirección del par remoto ("ip:puerto").
    pub peer_address: Option<String>,
    /// Índice de la parte solicitada actualmente, si está en transferencia.
    pub current_part: Option<u16>,
    /// Estimación de velocidad de transferencia en bytes/segundo.
    pub transfer_rate_bps: u64,
    /// Bytes totales recibidos en esta sesión.
    pub transferred_bytes: u64,
    /// Posición en cola informada por el par remoto.
    pub queue_rank: Option<u32>,
    /// `m_bRemoteQueueFull` de eMule — establecido por `OP_QUEUEFULL` (re-ask UDP),
    /// limpiado por `OP_REASKACK`. Las fuentes marcadas con cola remota llena se
    /// purgan primero cuando la lista de fuentes desborda (regla del 80 % de eMule).
    #[serde(default)]
    pub remote_queue_full: bool,
    /// Último mensaje de error o motivo de fallo.
    pub last_error: Option<String>,
    /// Motivo de fallo normalizado, si ha fallado.
    pub failure_reason: Option<PeerFailureReason>,
    /// Marca de tiempo de conexión completada (unix ms).
    pub connected_at_unix_ms: Option<i64>,
    /// Próximo plazo programado de repetición de petición TCP mientras está en cola (unix ms) — ancla
    /// de `GetTimeUntilReask()` de eMule (`DS_ONQUEUE` + `FILEREASKTIME`).
    /// El coordinador lo utiliza para disparar el re-ask UDP dentro de la
    /// ventana de `(1 s, 2 min)` de eMule antes del re-ask TCP (`PartFile.cpp:2291`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_tcp_reask_unix_ms: Option<i64>,
    /// Indica si esta sesión de par está utilizando ofuscación de protocolo cifrada.
    #[serde(default)]
    pub obfuscated: bool,
}

impl Default for PeerSessionSnapshot {
    fn default() -> Self {
        Self {
            status: PeerConnectionStatus::Disconnected,
            file_hash: None,
            peer_address: None,
            current_part: None,
            transfer_rate_bps: 0,
            transferred_bytes: 0,
            queue_rank: None,
            remote_queue_full: false,
            last_error: None,
            failure_reason: None,
            connected_at_unix_ms: None,
            next_tcp_reask_unix_ms: None,
            obfuscated: false,
        }
    }
}

/// Estado agregado del enjambre para un archivo específico o globalmente.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SwarmStatusSnapshot {
    /// Hash del archivo de destino si se filtra por archivo.
    pub file_hash: Option<String>,
    /// Conexiones activas totales de pares.
    pub active_peer_count: usize,
    /// Sesiones individuales activas de pares.
    pub peers: Vec<PeerSessionSnapshot>,
    /// Todos los rangos de bloques en vuelo activos no expirados.
    pub in_flight_blocks: Vec<InFlightBlock>,
}

pub(crate) struct PeerSessionHandle {
    pub snapshot: PeerSessionSnapshot,
    pub abort_handle: tokio::task::AbortHandle,
    /// Permiso de conexión saliente; `None` para sesiones adoptadas desde el oyente
    /// entrante (callback LowID), las cuales son contabilizadas por el propio oyente.
    pub _permit: Option<SocketPermit>,
}

pub(crate) struct SwarmInnerState {
    /// Sesiones activas indexadas por dirección del par ("ip:puerto").
    pub sessions: HashMap<String, PeerSessionHandle>,
    /// Bloques activos en vuelo asignados a pares.
    pub in_flight_blocks: Vec<InFlightBlock>,
    /// Instantánea de sesión de par más reciente (activa o desconectada), preservada para inspecciones.
    pub last_session: Option<PeerSessionSnapshot>,
    /// Conexiones activas máximas de pares por hash de archivo.
    pub max_peers_per_download: usize,
    /// Máximo de conexiones activas de pares en todo el daemon.
    pub max_global_peers: usize,
    /// Puntos de conexión UDP del par aprendidos de los apretones de manos:
    /// `"ip:puerto_tcp"` → `(puerto UDP de eMule, versión UDP del cliente)`.
    pub peer_udp: HashMap<String, (u16, u8)>,
    /// Pares lógicamente encolados en nosotros mientras estamos encolados en ellos — `DS_ONQUEUE`
    /// de eMule sobrevive al cierre del socket (`socket == NULL`): este mapa
    /// mantiene el estado de cola (plazo del próximo re-ask) tras finalizar la sesión
    /// TCP, para que `OP_REASKFILEPING` pueda dispararse en la ventana de eMule.
    /// `"ip:puerto_tcp"` → `QueuedPeerState`.
    pub queued_peers: HashMap<String, QueuedPeerState>,
    /// Último evento de transferencia emitido por archivo — `sync_transfer_rates`
    /// combina las actualizaciones a [`DOWNLOAD_EVENT_INTERVAL`] por archivo.
    pub last_download_event: HashMap<String, Instant>,
}

/// Estado de cola lógico que sobrevive a la sesión TCP — `DS_ONQUEUE` de eMule
/// con un socket cerrado (`DownloadClient.cpp:1326-1344` gestiona `!socket`).
#[derive(Debug, Clone)]
pub struct QueuedPeerState {
    /// Hash del archivo para el que el par estaba en cola.
    pub file_hash: String,
    /// Último intento de conexión TCP — `GetLastTriedToConnectTime()` de eMule
    /// (el ping UDP requiere que tenga ≥ 20 min de antigüedad, `PartFile.cpp:2294`).
    pub connected_at_unix_ms: Option<i64>,
    /// Próximo plazo de repetición de petición (`GetTimeUntilReask()` = FILEREASKTIME desde
    /// la última petición) — el ping UDP se dispara dentro de su ventana final de 2 minutos.
    pub next_tcp_reask_unix_ms: Option<i64>,
    /// `m_bReaskPending` de eMule (`DownloadClient.cpp:193`): la repetición de petición TCP
    /// de este par LowID se pospuso un ciclo porque está esperando en
    /// nuestra cola de subida — nos contactará en su propia repetición de petición. Consumido
    /// cuando el par nos contacta (`ConnectionEstablished`,
    /// `BaseClient.cpp:1552`).
    pub reask_pending: bool,
}

impl SwarmInnerState {
    /// Comprueba las cuotas del enjambre (global + por archivo) y la ausencia de una
    /// sesión ya abierta — compartido por `connect` y `accept_callback`.
    pub(crate) fn check_session_slot(
        &self,
        file_hash: &str,
        address: &str,
    ) -> Result<(), crate::error::DaemonError> {
        if self.sessions.len() >= self.max_global_peers {
            return Err(crate::error::DaemonError::Business(
                "Global peer connection limit reached".into(),
            ));
        }
        let file_peers_count = self
            .sessions
            .values()
            .filter(|s| s.snapshot.file_hash.as_deref() == Some(file_hash))
            .count();
        if file_peers_count >= self.max_peers_per_download {
            return Err(crate::error::DaemonError::Business(
                "Maximum concurrent peer connections reached for this download".into(),
            ));
        }
        if self.sessions.contains_key(address) {
            return Err(crate::error::DaemonError::Business(
                "Peer session is already running".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct PeerError {
    pub reason: PeerFailureReason,
    pub message: String,
}

impl PeerError {
    pub fn new(reason: PeerFailureReason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }
}

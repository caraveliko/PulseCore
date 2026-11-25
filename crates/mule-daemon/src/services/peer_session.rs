// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Gestor de conexiones con pares eD2K del enjambre y bloques en curso (fuente dual multipar).
//!
//! Gestiona las conexiones TCP activas con pares por descarga y a nivel global.
//! Aplica la política de red, límites de tasa por cubeta de tokens (token bucket), cancelación instantánea por kill-switch,
//! decodificación acotada de tramas en red (OP_HELLO, OP_HELLO_ANSWER, OP_FILESTATUSREQ, OP_FILESTATUS, OP_REQUESTPARTS, OP_SENDINGPART),
//! tiempos de espera granulares (connect, handshake, status, queue, first_data, inactivity),
//! enfriamiento por retroceso exponencial acotado por fuente y asignación de bloques sin solapamiento.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, info, warn, Instrument};

use mule_core::model::{PeerFailureReason, PeerSourceStatus};
use mule_network_policy::SocketPermit;

use crate::error::DaemonError;
use crate::services::downloads::DownloadService;

pub mod a4af;
pub mod bdp_pipeline;
pub mod change_client_id;
pub mod connection;
pub mod events;
pub mod file_request;
pub mod handshake;
pub mod happy_eyeballs;
pub mod peer_scoring;
pub mod source_exchange;
pub mod status_query;
pub mod sui;
pub mod transfer;
pub mod types;

pub use connection::{emit_download_update, emit_event, OP_COMPRESSEDPART_I64, OP_SENDINGPART_I64};
pub use peer_scoring::{calculate_backoff_for_reason, PeerScoreTracker};
pub use types::{
    calculate_backoff_cooldown_ms, PeerConnectionStatus, PeerSessionSnapshot, PeerTimeouts,
    SwarmStatusSnapshot, MAX_GLOBAL_PEERS, MAX_PEERS_PER_DOWNLOAD, MAX_PEER_READ_BUFFER_SIZE,
};
use types::{PeerSessionHandle, SwarmInnerState};

/// Retardo mínimo antes de volver a marcar una fuente cuyo socket en cola acaba
/// de morir cuando el límite de reconsulta ya ha pasado (protección anti-spin —
/// eMule reconsulta inmediatamente en `GetTimeUntilReask() == 0`).
const QUEUE_LOSS_REDIAL_FLOOR_MS: i64 = 60_000;

/// Seudonimizador de sesión compartido por los submódulos `peer_session`:
/// las direcciones de pares externos nunca aparecen en claro en los
/// archivos de log persistentes — `peer-NN:port` (loopback en claro,
/// inofensivo para el laboratorio).
static LOG_PSEUDONYMIZER: std::sync::OnceLock<mule_filter::SessionPseudonymizer> =
    std::sync::OnceLock::new();

/// Formatea una dirección de par para los logs: loopback en claro,
/// direcciones externas seudonimizadas de forma determinista por sesión.
/// A utilizar en cada emisión de log que contenga una dirección remota.
pub fn log_peer_addr(addr: SocketAddr) -> String {
    if addr.ip().is_loopback() {
        addr.to_string()
    } else {
        LOG_PSEUDONYMIZER
            .get_or_init(mule_filter::SessionPseudonymizer::new)
            .pseudonymize_peer(addr)
    }
}

/// Variante de servidor: `server-NN:port` para los endpoints eD2K/Kad UDP
/// del lado del servidor (tabla distinta de la de los pares).
pub fn log_server_addr(addr: SocketAddr) -> String {
    if addr.ip().is_loopback() {
        addr.to_string()
    } else {
        LOG_PSEUDONYMIZER
            .get_or_init(mule_filter::SessionPseudonymizer::new)
            .pseudonymize_server(addr)
    }
}

/// Para una dirección ya formateada (`"ip:port"`) — seudonimizada si
/// es analizable, sin cambios en caso contrario (`lowid:<id>` no es una IP).
pub fn log_addr_str(s: impl std::fmt::Display) -> String {
    let raw = s.to_string();
    match raw.parse::<SocketAddr>() {
        Ok(addr) => log_peer_addr(addr),
        Err(_) => raw,
    }
}

/// Para una IP desnuda sin puerto — `peer-NN` (comparte la tabla de pares).
/// Acepta `IpAddr` o `Ipv4Addr` (`Into<IpAddr>`).
pub fn log_peer_ip(ip: impl Into<std::net::IpAddr>) -> String {
    let ip = ip.into();
    if ip.is_loopback() {
        ip.to_string()
    } else {
        LOG_PSEUDONYMIZER
            .get_or_init(mule_filter::SessionPseudonymizer::new)
            .pseudonymize_ip(ip)
    }
}

/// Gestor seguro para subprocesos para conexiones de pares eD2K multifuente.
#[derive(Clone)]
pub struct PeerSessionManager {
    inner: Arc<Mutex<SwarmInnerState>>,
    event_tx: broadcast::Sender<String>,
    sequence: Arc<AtomicU64>,
    timeouts: PeerTimeouts,
    kad: Arc<std::sync::RwLock<Option<Arc<crate::services::KadService>>>>,
    identity: Arc<std::sync::RwLock<Option<Arc<Mutex<crate::services::IdentityService>>>>>,
    sui_stats: Arc<std::sync::RwLock<Arc<crate::services::identity::SuiStats>>>,
    /// LowID → (file_hash_hex, caducidad unix ms) para devoluciones de llamada
    /// pendientes retransmitidas por el servidor (`OP_CALLBACKREQUEST`). Las entradas expiran tras 5 minutos.
    pending_callbacks: Arc<std::sync::Mutex<std::collections::HashMap<u32, (String, i64)>>>,
    /// IP de par → (file_hash_hex, caducidad unix ms) para devoluciones de llamada
    /// pendientes mediante buddy de Kad (`KADEMLIA_CALLBACK_REQ`). La fuente LowID conecta
    /// entrante; emparejada en el listener entrante por la IP remota.
    pending_kad_callbacks:
        Arc<std::sync::Mutex<std::collections::HashMap<std::net::IpAddr, (String, i64)>>>,
    /// Limitadores de tasa de ancho de banda globales compartidos con la política de red —
    /// el bucle de transferencia modula los datos recibidos mediante `download_bucket`.
    rate_limiters: Arc<std::sync::RwLock<Option<Arc<mule_network_policy::PolicyRateLimiters>>>>,
    /// Seguidor de reputación, latencia y diversidad de subred para puntuación de candidatos.
    score_tracker: Arc<std::sync::Mutex<peer_scoring::PeerScoreTracker>>,
    /// Controlador USS para alimentar muestras pasivas de RTT (respaldo LEDBAT++).
    uss: Arc<std::sync::RwLock<Option<crate::services::UssService>>>,
}

impl PeerSessionManager {
    /// Crea un nuevo `PeerSessionManager`.
    pub fn new(event_tx: broadcast::Sender<String>, sequence: Arc<AtomicU64>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SwarmInnerState {
                sessions: HashMap::new(),
                in_flight_blocks: Vec::new(),
                last_session: None,
                max_peers_per_download: MAX_PEERS_PER_DOWNLOAD,
                max_global_peers: MAX_GLOBAL_PEERS,
                peer_udp: HashMap::new(),
                queued_peers: HashMap::new(),
                last_download_event: HashMap::new(),
            })),
            event_tx,
            sequence,
            timeouts: PeerTimeouts::default(),
            kad: Arc::new(std::sync::RwLock::new(None)),
            identity: Arc::new(std::sync::RwLock::new(None)),
            sui_stats: Arc::new(std::sync::RwLock::new(Arc::new(
                crate::services::identity::SuiStats::default(),
            ))),
            pending_callbacks: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            pending_kad_callbacks: Arc::new(
                std::sync::Mutex::new(std::collections::HashMap::new()),
            ),
            rate_limiters: Arc::new(std::sync::RwLock::new(None)),
            score_tracker: Arc::new(std::sync::Mutex::new(peer_scoring::PeerScoreTracker::new())),
            uss: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// Inyecta el servicio USS para alimentar el estimador pasivo LEDBAT++.
    pub fn set_uss(&self, uss: crate::services::UssService) {
        if let Ok(mut g) = self.uss.write() {
            *g = Some(uss);
        }
    }

    fn uss(&self) -> Option<crate::services::UssService> {
        self.uss.read().ok().and_then(|g| g.clone())
    }

    /// Inyecta los limitadores de tasa de política de red compartidos (planificador
    /// global de ancho de banda de descarga para bucles de transferencia de pares).
    pub fn set_rate_limiters(&self, limiters: Arc<mule_network_policy::PolicyRateLimiters>) {
        *self.rate_limiters.write().unwrap_or_else(|p| p.into_inner()) = Some(limiters);
    }

    /// Devuelve el seguidor compartido de reputación y puntuación de pares.
    pub fn score_tracker(&self) -> Arc<std::sync::Mutex<peer_scoring::PeerScoreTracker>> {
        Arc::clone(&self.score_tracker)
    }

    fn rate_limiters(&self) -> Option<Arc<mule_network_policy::PolicyRateLimiters>> {
        self.rate_limiters.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Registra una devolución de llamada LowID pendiente: tras enviar `OP_CALLBACKREQUEST`
    /// al servidor, se espera que el cliente LowID objetivo (`lowid`) conecte
    /// entrante dentro de `ttl_ms`. Emparejado en el listener entrante por el
    /// `client_id` del par procedente de `OP_HELLO`.
    pub fn register_pending_callback(&self, lowid: u32, file_hash: String, ttl_ms: i64) {
        let expiry = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
            + ttl_ms;
        let mut map = self.pending_callbacks.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, (_, exp)| *exp > expiry - ttl_ms);
        map.insert(lowid, (file_hash, expiry));
    }

    /// Consume un registro de devolución de llamada pendiente para `lowid`, devolviendo
    /// el hash de archivo asociado si aún es válido.
    pub fn take_pending_callback(&self, lowid: u32) -> Option<String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let mut map = self.pending_callbacks.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, (_, exp)| *exp > now);
        map.remove(&lowid).map(|(h, _)| h)
    }

    /// Devuelve true si existe una devolución de llamada pendiente (no expirada) para `lowid`.
    pub fn has_pending_callback(&self, lowid: u32) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let mut map = self.pending_callbacks.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, (_, exp)| *exp > now);
        map.contains_key(&lowid)
    }

    /// Registra una devolución de llamada pendiente mediante buddy de Kad: tras retransmitir
    /// `KADEMLIA_CALLBACK_REQ` al buddy, se espera que la fuente LowID (en `ip`) conecte
    /// de entrada dentro de `ttl_ms`.
    pub fn register_pending_kad_callback(
        &self,
        ip: std::net::IpAddr,
        file_hash: String,
        ttl_ms: i64,
    ) {
        let expiry = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
            + ttl_ms;
        let mut map = self.pending_kad_callbacks.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, (_, exp)| *exp > expiry - ttl_ms);
        map.insert(ip, (file_hash, expiry));
    }

    /// Consume un registro de devolución de llamada pendiente mediante buddy de Kad para `ip`.
    pub fn take_pending_kad_callback(&self, ip: std::net::IpAddr) -> Option<String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let mut map = self.pending_kad_callbacks.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, (_, exp)| *exp > now);
        map.remove(&ip).map(|(h, _)| h)
    }

    /// Devuelve true si existe una devolución de llamada pendiente (no expirada) mediante buddy de Kad para `ip`.
    pub fn has_pending_kad_callback(&self, ip: std::net::IpAddr) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let mut map = self.pending_kad_callbacks.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, (_, exp)| *exp > now);
        map.contains_key(&ip)
    }

    /// Establece los contadores de anomalías SUI compartidos (objeto `sui` de identity.status).
    pub fn set_sui_stats(&self, stats: Arc<crate::services::identity::SuiStats>) {
        *self.sui_stats.write().unwrap_or_else(|p| p.into_inner()) = stats;
    }

    /// Establece el servicio de identidad utilizado para la Identificación Segura
    /// de Usuario (SUI) saliente de eMule (firma RSA-384 de desafíos de pares).
    pub fn set_identity(&self, identity: Arc<Mutex<crate::services::IdentityService>>) {
        *self.identity.write().unwrap_or_else(|p| p.into_inner()) = Some(identity);
    }

    fn identity(&self) -> Option<Arc<Mutex<crate::services::IdentityService>>> {
        self.identity.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn with_limits(mut self, max_peers_per_download: usize, max_global_peers: usize) -> Self {
        self.inner = Arc::new(Mutex::new(SwarmInnerState {
            sessions: HashMap::new(),
            in_flight_blocks: Vec::new(),
            last_session: None,
            max_peers_per_download,
            max_global_peers,
            peer_udp: HashMap::new(),
            queued_peers: HashMap::new(),
            last_download_event: HashMap::new(),
        }));
        self
    }

    pub fn set_kad_service(&self, kad: Arc<crate::services::KadService>) {
        let mut g = self.kad.write().unwrap_or_else(|p| p.into_inner());
        *g = Some(kad);
    }

    pub fn set_kad(&self, kad: Arc<crate::services::KadService>) {
        self.set_kad_service(kad);
    }

    pub async fn set_swarm_limits(&self, max_per_file: usize, max_global: usize) {
        let mut g = self.inner.lock().await;
        g.max_peers_per_download = max_per_file;
        g.max_global_peers = max_global;
    }

    pub fn kad_service(&self) -> Option<Arc<crate::services::KadService>> {
        let g = self.kad.read().unwrap_or_else(|p| p.into_inner());
        g.clone()
    }

    /// Endpoints UDP de sesiones actualmente en estado `Queued` en un par remoto que
    /// anunció un puerto UDP de eMule — destinos de mantenimiento de conexión para
    /// `OP_REASKFILEPING` (`CUpDownClient::UDPReaskForDownload`).
    /// Pares en cuya cola estamos lógicamente — `UDPReaskForDownload` de eMule
    /// requiere `!(socket && socket->IsConnected())` (`DownloadClient.cpp:1344`):
    /// el ping solo tiene sentido una vez que el socket de cola murió
    /// (`DS_ONQUEUE` persiste con `socket == NULL`), por lo que las sesiones activas
    /// quedan excluidas y el estado procede de `queued_peers`.
    pub async fn queued_udp_targets(&self) -> Vec<crate::services::client_udp::UdpReaskTarget> {
        let mut g = self.inner.lock().await;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        // Descartar entradas cuyo límite de tiempo de reconsulta ha pasado ampliamente — la fuente fue
        // purgada o marcada de nuevo sin volver a ponerse en cola.
        g.queued_peers.retain(|_, q| {
            q.next_tcp_reask_unix_ms
                .map(|d| {
                    d > now_ms
                        - 2 * crate::services::peer_session::status_query::FILE_REASK_INTERVAL
                            .as_millis() as i64
                })
                .unwrap_or(true)
        });
        g.queued_peers
            .iter()
            // Sesión TCP activa → eMule no emitiría el ping UDP.
            .filter(|(addr, _)| !g.sessions.contains_key(*addr))
            .filter_map(|(addr_str, q)| {
                let (udp_port, ver) = g.peer_udp.get(addr_str)?;
                // `UDPReaskForDownload` de eMule requiere `GetUDPVersion() != 0`.
                if *ver == 0 {
                    return None;
                }
                let sock_addr = addr_str.parse::<std::net::SocketAddr>().ok()?;
                let hash_bytes = hex::decode(&q.file_hash).ok()?;
                if hash_bytes.len() != 16 {
                    return None;
                }
                let mut file_hash = [0u8; 16];
                file_hash.copy_from_slice(&hash_bytes);
                Some(crate::services::client_udp::UdpReaskTarget {
                    addr: std::net::SocketAddr::new(sock_addr.ip(), *udp_port),
                    file_hash,
                    connected_at_unix_ms: q.connected_at_unix_ms,
                    next_tcp_reask_unix_ms: q.next_tcp_reask_unix_ms,
                })
            })
            .collect()
    }

    /// `m_bReaskPending` de eMule — `CUpDownClient::AskForDownload`
    /// (`DownloadClient.cpp:185-206`): la reconsulta TCP de un par LowID que
    /// *también está esperando en nuestra cola de subida* se pospone un ciclo para que
    /// el par pueda contactarnos con su propia reconsulta de subida. Marca la
    /// entrada `queued_peers` para `ip` (indexada `"ip:port"`) y pospone su
    /// límite de tiempo de reconsulta a `next_reask_unix_ms`.
    ///
    /// Devuelve `false` cuando no existe entrada en cola para `ip`+`file_hash`
    /// (`GetLastAskedTime() > 0` de eMule — una fuente nunca contactada se
    /// marca de inmediato) o cuando el indicador ya está establecido (el aplazamiento
    /// se activa una sola vez, como `!m_bReaskPending`).
    pub async fn defer_reask_for_queued_peer(
        &self,
        ip: std::net::IpAddr,
        file_hash: &str,
        next_reask_unix_ms: i64,
    ) -> bool {
        let prefix = format!("{ip}:");
        let mut g = self.inner.lock().await;
        let mut marked = false;
        for (addr, q) in g.queued_peers.iter_mut() {
            if !addr.starts_with(&prefix)
                || !q.file_hash.eq_ignore_ascii_case(file_hash)
                || q.reask_pending
            {
                continue;
            }
            q.reask_pending = true;
            q.next_tcp_reask_unix_ms = Some(next_reask_unix_ms);
            marked = true;
        }
        marked
    }

    /// Consume el indicador `m_bReaskPending` — `ConnectionEstablished` de eMule
    /// (`BaseClient.cpp:1552`): cuando el par pospuesto abre una conexión
    /// hacia nosotros, su petición de archivo sale inmediatamente. Devuelve
    /// el estado de cola conservado (hash + límite de tiempo) para que el llamador
    /// active la devolución de llamada.
    pub async fn take_reask_pending(
        &self,
        ip: std::net::IpAddr,
    ) -> Option<crate::services::peer_session::types::QueuedPeerState> {
        let prefix = format!("{ip}:");
        let mut g = self.inner.lock().await;
        let mut found = None;
        for (addr, q) in g.queued_peers.iter_mut() {
            if addr.starts_with(&prefix) && q.reask_pending {
                q.reask_pending = false;
                found = Some(q.clone());
            }
        }
        found
    }

    fn addr_matches_ip(addr: &str, ip: &std::net::IpAddr) -> bool {
        addr.parse::<std::net::SocketAddr>()
            .ok()
            .map(|sa| sa.ip())
            == Some(*ip)
    }

    /// Aplica una posición en cola aprendida mediante `OP_REASKACK` a la
    /// sesión en cola de ese par (emparejada por IP, como
    /// `GetDownloadClientByIP_UDP`).
    pub async fn apply_udp_queue_rank(&self, ip: &std::net::IpAddr, rank: u32) {
        let mut g = self.inner.lock().await;
        for (addr, q) in g.queued_peers.iter_mut() {
            if Self::addr_matches_ip(addr, ip) {
                q.next_tcp_reask_unix_ms = Some(
                    crate::services::downloads::now_ms()
                        + crate::services::peer_session::status_query::FILE_REASK_INTERVAL
                            .as_millis() as i64,
                );
            }
        }
        for s in g.sessions.values_mut() {
            let matches = s
                .snapshot
                .peer_address
                .as_deref()
                .map(|a| Self::addr_matches_ip(a, ip))
                .unwrap_or(false);
            if matches && s.snapshot.status == PeerConnectionStatus::Queued {
                s.snapshot.queue_rank = Some(rank);
                // `UDPReaskACK` → `SetRemoteQueueFull(false)` de eMule.
                s.snapshot.remote_queue_full = false;
                emit_event(&self.event_tx, &self.sequence, &s.snapshot);
                tracing::info!(
                    "Peer {peer}: queue rank #{rank} updated via OP_REASKACK (UDP)",
                    peer = log_peer_ip(*ip)
                );
            }
        }
    }

    /// Aplica `OP_QUEUEFULL` a la sesión en cola de ese par — `SetRemoteQueueFull(true)`
    /// + `UDPReaskACK(0)` de eMule (posición restablecida a 0).
    pub async fn apply_udp_queue_full(&self, ip: &std::net::IpAddr) {
        let mut g = self.inner.lock().await;
        for s in g.sessions.values_mut() {
            let matches = s
                .snapshot
                .peer_address
                .as_deref()
                .map(|a| Self::addr_matches_ip(a, ip))
                .unwrap_or(false);
            if matches && s.snapshot.status == PeerConnectionStatus::Queued {
                s.snapshot.remote_queue_full = true;
                s.snapshot.queue_rank = Some(0);
                emit_event(&self.event_tx, &self.sequence, &s.snapshot);
                tracing::info!(
                    "Peer {peer}: remote queue full via OP_QUEUEFULL (UDP)",
                    peer = log_peer_ip(*ip)
                );
            }
        }
    }

    /// Aplica `OP_FILENOTFOUND` a la sesión en cola de ese par —
    /// `UDPReaskFNF` de eMule elimina el cliente de descarga para ese archivo.
    pub async fn apply_udp_file_not_found(&self, ip: &std::net::IpAddr) {
        let mut g = self.inner.lock().await;
        g.queued_peers.retain(|addr, _| !Self::addr_matches_ip(addr, ip));
        for s in g.sessions.values_mut() {
            let matches = s
                .snapshot
                .peer_address
                .as_deref()
                .map(|a| Self::addr_matches_ip(a, ip))
                .unwrap_or(false);
            if matches && s.snapshot.status == PeerConnectionStatus::Queued {
                s.snapshot.status = PeerConnectionStatus::Disconnected;
                s.snapshot.failure_reason = Some(mule_core::model::PeerFailureReason::FileNotFound);
                s.snapshot.last_error = Some("peer answered OP_FILENOTFOUND (UDP)".to_string());
                emit_event(&self.event_tx, &self.sequence, &s.snapshot);
                tracing::info!(
                    "Peer {peer}: queued session dropped via OP_FILENOTFOUND (UDP)",
                    peer = log_peer_ip(*ip)
                );
            }
        }
    }

    pub async fn active_peer_count_for_hash(&self, file_hash: &str) -> usize {
        let g = self.inner.lock().await;
        g.sessions
            .values()
            .filter(|s| s.snapshot.file_hash.as_deref() == Some(file_hash))
            .count()
    }

    /// Comprueba si hay alguna sesión de pares actualmente activa.
    pub async fn is_active(&self) -> bool {
        let g = self.inner.lock().await;
        !g.sessions.is_empty()
    }

    /// Desconecta la sesión de par predeterminada (primera activa) para compatibilidad hacia atrás.
    pub async fn disconnect(&self) -> Result<PeerSessionSnapshot, DaemonError> {
        self.disconnect_peer(None).await
    }

    /// Método principal de conexión utilizando un permiso previamente adquirido.
    pub async fn connect(
        &self,
        file_hash: &str,
        target_addr: SocketAddr,
        permit: SocketPermit,
        client_port: u16,
        nickname: String,
        user_hash: [u8; 16],
        downloads: Arc<Mutex<DownloadService>>,
        client_id: u32,
        server_ip: u32,
        server_port: u16,
        kad_port: u16,
        target_user_hash: Option<[u8; 16]>,
        peer_crypt_options: u8,
        obfuscation_enabled: bool,
        obfuscation_required: bool,
        obfuscation_padding_max: u8,
        sx_env: source_exchange::SxEnv,
    ) -> Result<PeerSessionSnapshot, DaemonError> {
        let address = target_addr.to_string();
        let dl_hash_bytes = hex::decode(file_hash)
            .map_err(|e| DaemonError::InvalidParams(format!("Invalid file hash hex: {e}")))?;
        if dl_hash_bytes.len() != 16 {
            return Err(DaemonError::InvalidParams(
                "File hash must be exactly 16 bytes".into(),
            ));
        }
        let mut dl_hash_arr = [0u8; 16];
        dl_hash_arr.copy_from_slice(&dl_hash_bytes);

        // Hacer cumplir el límite máximo de pares activos
        {
            let g = self.inner.lock().await;
            g.check_session_slot(file_hash, &address)?;
        }

        let cancel_token = permit.cancellation_token().clone();

        let initial_snapshot = PeerSessionSnapshot {
            peer_address: Some(address.clone()),
            file_hash: Some(file_hash.to_string()),
            status: PeerConnectionStatus::Connecting,
            transferred_bytes: 0,
            transfer_rate_bps: 0,
            current_part: None,
            queue_rank: None,
            remote_queue_full: false,
            last_error: None,
            failure_reason: None,
            connected_at_unix_ms: None,
            next_tcp_reask_unix_ms: None,
            obfuscated: false,
        };

        // Lanzar manejador de conexión asíncrono
        let inner_clone = Arc::clone(&self.inner);
        let event_tx_clone = self.event_tx.clone();
        let seq_clone = Arc::clone(&self.sequence);
        let address_str = address.clone();
        let file_hash_str = file_hash.to_string();
        let timeouts = self.timeouts;
        let kad_svc_opt = self.kad_service();
        let identity_opt = self.identity();
        let sui_stats = self.sui_stats.read().unwrap_or_else(|p| p.into_inner()).clone();
        let rate_limiters_opt = self.rate_limiters();
        let score_tracker_clone = self.score_tracker();
        let uss_opt = self.uss();

        // Span de sesión: `peer` seudonimizado se adjunta a todos los
        // eventos de la tarea en el JSONL (correlación sin IP en bruto).
        let log_addr = log_peer_addr(target_addr);
        let session_span = tracing::info_span!("peer_session", peer = %log_addr);
        let join_handle = tokio::spawn(
            async move {
                info!("Starting peer connection to {log_addr}");
                let res = connection::run_peer_connection(
                    file_hash_str.clone(),
                    dl_hash_arr,
                    address_str.clone(),
                    target_addr,
                    client_port,
                    nickname,
                    user_hash,
                    Arc::clone(&downloads),
                    cancel_token,
                    Arc::clone(&inner_clone),
                    event_tx_clone.clone(),
                    seq_clone.clone(),
                    timeouts,
                    client_id,
                    server_ip,
                    server_port,
                    kad_port,
                    kad_svc_opt,
                    target_user_hash,
                    peer_crypt_options,
                    obfuscation_enabled,
                    obfuscation_required,
                    obfuscation_padding_max,
                    identity_opt,
                    sui_stats,
                    rate_limiters_opt,
                    sx_env,
                    Arc::clone(&score_tracker_clone),
                    uss_opt,
                )
                .await;

                let mut final_status = PeerConnectionStatus::Disconnected;
                let mut err_msg = None;
                let mut failure_reason = None;

                match res {
                    Ok(bytes) => {
                        info!(
                            "Peer connection to {} completed cleanly. Total transferred: {} bytes",
                            log_addr, bytes
                        );
                        final_status = PeerConnectionStatus::Disconnected;
                    }
                    Err(e) => {
                        // Variación normal del enjambre (fuentes caídas, tiempos de espera, cola llena) -> debug;
                        // solo las anomalías de integridad permanecen en warn.
                        match e.reason {
                            PeerFailureReason::HashMismatch
                            | PeerFailureReason::InvalidRange
                            | PeerFailureReason::CorruptedData => {
                                warn!("Peer connection to {} ended with error: {:?}", log_addr, e);
                            }
                            _ => {
                                debug!("Peer connection to {} ended with error: {:?}", log_addr, e);
                            }
                        }
                        err_msg = Some(e.message.clone());
                        failure_reason = Some(e.reason);
                    }
                }

                // Limpiar sesión y liberar bloques en vuelo
                let (final_snapshot, outcome_hash, queued_deadline) = {
                    let mut g = inner_clone.lock().await;
                    g.in_flight_blocks.retain(|b| b.peer_id != address_str);

                    if let Some(mut session) = g.sessions.remove(&address_str) {
                        // `DS_ONQUEUE` de eMule sobrevive a CUALQUIER desconexión
                        // no terminal (`BaseClient.cpp:1126-1135`: solo
                        // DS_CONNECTED/DS_REQHASHSET → deadsource, los demás
                        // estados — incluido ONQUEUE — permanecen en la cola lógica).
                        // La entrada `queued_peers` solo se elimina, por tanto,
                        // para resultados que cambian el estado lógico:
                        // FNF → fuente rechazada, NNP → DS_NONEEDEDPARTS.
                        let terminal = matches!(
                            failure_reason,
                            Some(
                                PeerFailureReason::FileNotFound | PeerFailureReason::NoNeededParts
                            )
                        );
                        if terminal {
                            g.queued_peers.remove(&address_str);
                        }
                        let deadline = if session.snapshot.status == PeerConnectionStatus::Queued {
                            session.snapshot.next_tcp_reask_unix_ms
                        } else {
                            None
                        };
                        session.snapshot.status = final_status;
                        session.snapshot.transfer_rate_bps = 0;
                        session.snapshot.last_error = err_msg.clone();
                        session.snapshot.failure_reason = failure_reason;
                        g.last_session = Some(session.snapshot.clone());
                        // Tras un intercambio de socket A4AF la sesión sirvió a otro
                        // archivo — el resultado pertenece al archivo solicitado
                        // en último lugar (`RemoveSource` de eMule en `m_reqfile`),
                        // no al marcado originalmente.
                        let outcome = session.snapshot.file_hash.clone();
                        (Some(session.snapshot), outcome, deadline)
                    } else {
                        (None, None, None)
                    }
                };

                if let Some(snap) = final_snapshot {
                    connection::emit_event(&event_tx_clone, &seq_clone, &snap);
                }

                // Actualizar registro PeerSource en SQLite (enfriamiento, fail_count, estado)
                {
                    let mut dl_svc = downloads.lock().await;
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as i64;
                    let outcome_hash = outcome_hash.as_deref().unwrap_or(&file_hash_str);

                    // Recuento real de fallos consecutivos (el UPDATE lo incrementa en
                    // Failed) para que el retroceso aumente efectivamente en lugar de quedarse
                    // atascado en el primer paso.
                    let next_fail_count = dl_svc
                        .list_peer_sources(outcome_hash)
                        .unwrap_or_default()
                        .into_iter()
                        .find(|s| s.address == address_str)
                        .map(|s| s.fail_count + 1)
                        .unwrap_or(1);

                    let (new_status, new_cooldown) = if let Some(reason) = failure_reason {
                        if let Ok(mut guard) = score_tracker_clone.lock() {
                            guard.record_failure(&address_str, reason);
                        }
                        match reason {
                            PeerFailureReason::FileNotFound => (PeerSourceStatus::Rejected, None),
                            // "No Needed Parts" de eMule (DS_NONEEDEDPARTS): la
                            // fuente se conserva y se vuelve a pedir en el ciclo
                            // duplicado `FILEREASKTIME * 2` = 58 min
                            // (`DownloadClient.cpp:1888`, `PartFile.cpp:2276`).
                            PeerFailureReason::NoNeededParts => {
                                (PeerSourceStatus::Allowed, Some(now_ms + 58 * 60 * 1000))
                            }
                            _ => {
                                let is_queue_disconnect = err_msg.as_deref().is_some_and(|msg| {
                                    msg.contains("while awaiting file status")
                                        || msg.contains("while in queue")
                                        || msg.contains("Disconnected in queue")
                                        || msg.contains("Queue socket idle")
                                });

                                if is_queue_disconnect || queued_deadline.is_some() {
                                    // Comportamiento estándar de cola de eMule: el socket TCP se cerró (tiempo de espera o post-petición) estando en cola.
                                    // Permanecemos lógicamente en cola y permitimos reconsultar con prontitud (dentro de 3 minutos,
                                    // o cuando se abra la ventana de reconsulta UDP) en lugar de penalizar con enfriamientos prolongados.
                                    let soft_reask_deadline = now_ms + 180_000; // 3 minutes
                                    let effective_deadline = queued_deadline
                                        .map(|dl| dl.min(soft_reask_deadline).max(now_ms + QUEUE_LOSS_REDIAL_FLOOR_MS))
                                        .unwrap_or(soft_reask_deadline);
                                    (
                                        PeerSourceStatus::Allowed,
                                        Some(effective_deadline),
                                    )
                                } else {
                                    let cd = calculate_backoff_for_reason(
                                        next_fail_count,
                                        reason,
                                        rand::random::<f64>(),
                                    );
                                    (PeerSourceStatus::Failed, Some(now_ms + cd))
                                }
                            }
                        }
                    } else {
                        (PeerSourceStatus::Candidate, None)
                    };

                    let _ = dl_svc.update_peer_source_status(
                        outcome_hash,
                        &address_str,
                        new_status,
                        err_msg.as_deref(),
                        new_cooldown,
                    );
                }
            }
            .instrument(session_span),
        );

        let session_handle = PeerSessionHandle {
            snapshot: initial_snapshot.clone(),
            abort_handle: join_handle.abort_handle(),
            _permit: Some(permit),
        };

        {
            let mut g = self.inner.lock().await;
            g.sessions.insert(address.clone(), session_handle);
            g.last_session = Some(initial_snapshot.clone());
        }

        connection::emit_event(&self.event_tx, &self.sequence, &initial_snapshot);

        Ok(initial_snapshot)
    }

    /// Adopta una conexión entrante aceptada originada por una devolución
    /// de llamada de servidor LowID (`OP_CALLBACKREQUEST`). El par ya ha enviado
    /// `OP_HELLO` y recibido nuestro `OP_HELLOANSWER`; esto ejecuta la canalización de
    /// estado/cola/transferencia en rol de cliente sobre ella.
    /// `source_address` es la clave de fuente de par almacenada (`lowid:<id>`) utilizada
    /// para la contabilidad de estado, distinta de la `peer_addr` real del socket.
    #[allow(clippy::too_many_arguments)]
    pub async fn accept_callback(
        &self,
        stream: crate::services::obfuscation_stream::ObfuscatedStream<tokio::net::TcpStream>,
        peer_addr: SocketAddr,
        file_hash: &str,
        source_address: String,
        sx_caps: source_exchange::PeerSxCaps,
        peer_supports_aich: bool,
        peer_sec_ident: u8,
        peer_ext_req: u8,
        downloads: Arc<Mutex<DownloadService>>,
        cancel_token: mule_network_policy::CancellationToken,
        sx_env: source_exchange::SxEnv,
        sx_local: source_exchange::SxLocalIds,
    ) -> Result<PeerSessionSnapshot, DaemonError> {
        let address = peer_addr.to_string();
        let dl_hash_bytes = hex::decode(file_hash)
            .map_err(|e| DaemonError::InvalidParams(format!("Invalid file hash hex: {e}")))?;
        if dl_hash_bytes.len() != 16 {
            return Err(DaemonError::InvalidParams(
                "File hash must be exactly 16 bytes".into(),
            ));
        }
        let mut dl_hash_arr = [0u8; 16];
        dl_hash_arr.copy_from_slice(&dl_hash_bytes);

        {
            let g = self.inner.lock().await;
            g.check_session_slot(file_hash, &address)?;
        }

        let initial_snapshot = PeerSessionSnapshot {
            peer_address: Some(address.clone()),
            file_hash: Some(file_hash.to_string()),
            status: PeerConnectionStatus::Handshaking,
            transferred_bytes: 0,
            transfer_rate_bps: 0,
            current_part: None,
            queue_rank: None,
            remote_queue_full: false,
            last_error: None,
            failure_reason: None,
            connected_at_unix_ms: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64,
            ),
            next_tcp_reask_unix_ms: None,
            obfuscated: stream.is_obfuscated(),
        };

        let inner_clone = Arc::clone(&self.inner);
        let event_tx_clone = self.event_tx.clone();
        let seq_clone = Arc::clone(&self.sequence);
        let address_str = address.clone();
        let file_hash_str = file_hash.to_string();
        let timeouts = self.timeouts;
        let identity_opt = self.identity();
        let sui_stats = self.sui_stats.read().unwrap_or_else(|p| p.into_inner()).clone();
        let rate_limiters_opt = self.rate_limiters();
        let score_tracker_clone = self.score_tracker();
        let uss_opt = self.uss();

        let log_addr = log_peer_addr(peer_addr);
        let session_span = tracing::info_span!("peer_session", peer = %log_addr);
        let join_handle = tokio::spawn(
            async move {
            info!(
                "Starting callback peer session to {} for {}",
                log_addr, file_hash_str
            );
            let res = connection::run_callback_session(
                stream,
                peer_addr,
                file_hash_str.clone(),
                dl_hash_arr,
                address_str.clone(),
                sx_caps,
                peer_supports_aich,
                peer_sec_ident,
                peer_ext_req,
                Arc::clone(&downloads),
                cancel_token,
                Arc::clone(&inner_clone),
                event_tx_clone.clone(),
                seq_clone.clone(),
                timeouts,
                identity_opt,
                sui_stats,
                rate_limiters_opt,
                sx_env,
                sx_local,
                Arc::clone(&score_tracker_clone),
                uss_opt,
            )
            .await;

            let mut final_status = PeerConnectionStatus::Disconnected;
            let mut err_msg = None;
            let mut failure_reason = None;

            match res {
                Ok(bytes) => {
                    info!("Callback peer session to {} completed cleanly. Total transferred: {} bytes", log_addr, bytes);
                    final_status = PeerConnectionStatus::Disconnected;
                }
                Err(e) => {
                    warn!(
                        "Callback peer session to {} ended with error: {:?}",
                        log_addr, e
                    );
                    err_msg = Some(e.message.clone());
                    failure_reason = Some(e.reason);
                }
            }

            let (final_snapshot, outcome_hash) = {
                let mut g = inner_clone.lock().await;
                g.in_flight_blocks.retain(|b| b.peer_id != address_str);

                if let Some(mut session) = g.sessions.remove(&address_str) {
                    session.snapshot.status = final_status;
                    session.snapshot.transfer_rate_bps = 0;
                    session.snapshot.last_error = err_msg.clone();
                    session.snapshot.failure_reason = failure_reason;
                    g.last_session = Some(session.snapshot.clone());
                    // Post-A4AF: el resultado se registra en el archivo servido en
                    // último lugar (`m_reqfile` de eMule), no en el hash marcado.
                    let outcome = session.snapshot.file_hash.clone();
                    (Some(session.snapshot), outcome)
                } else {
                    (None, None)
                }
            };

            if let Some(snap) = final_snapshot {
                connection::emit_event(&event_tx_clone, &seq_clone, &snap);
            }

            // Actualizar el registro peer-source `lowid:<id>`.
            {
                let mut dl_svc = downloads.lock().await;
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                let outcome_hash = outcome_hash.as_deref().unwrap_or(&file_hash_str);

                let next_fail_count = dl_svc
                    .list_peer_sources(outcome_hash)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|s| s.address == source_address)
                    .map(|s| s.fail_count + 1)
                    .unwrap_or(1);

                let (new_status, new_cooldown) = if let Some(reason) = failure_reason {
                    if let Ok(mut guard) = score_tracker_clone.lock() {
                        guard.record_failure(&source_address, reason);
                    }
                    match reason {
                        PeerFailureReason::FileNotFound => (PeerSourceStatus::Rejected, None),
                        // DS_NONEEDEDPARTS de eMule: fuente conservada, reconsulta
                        // duplicada `FILEREASKTIME * 2` = 58 min.
                        PeerFailureReason::NoNeededParts => {
                            (PeerSourceStatus::Allowed, Some(now_ms + 58 * 60 * 1000))
                        }
                        _ => {
                            if err_msg.as_deref() == Some("Disconnected in queue") || err_msg.as_deref() == Some("Queue socket idle (eMule CONNECTION_TIMEOUT)") {
                                (PeerSourceStatus::Allowed, Some(now_ms + 180_000))
                            } else {
                                let cd = calculate_backoff_for_reason(
                                    next_fail_count,
                                    reason,
                                    rand::random::<f64>(),
                                );
                                (PeerSourceStatus::Failed, Some(now_ms + cd))
                            }
                        }
                    }
                } else {
                    (PeerSourceStatus::Candidate, None)
                };

                let _ = dl_svc.update_peer_source_status(
                    outcome_hash,
                    &source_address,
                    new_status,
                    err_msg.as_deref(),
                    new_cooldown,
                );
            }
        }
        .instrument(session_span));

        let session_handle = PeerSessionHandle {
            snapshot: initial_snapshot.clone(),
            abort_handle: join_handle.abort_handle(),
            _permit: None,
        };

        {
            let mut g = self.inner.lock().await;
            g.sessions.insert(address.clone(), session_handle);
            g.last_session = Some(initial_snapshot.clone());
        }

        connection::emit_event(&self.event_tx, &self.sequence, &initial_snapshot);

        Ok(initial_snapshot)
    }

    /// Desconecta una sesión de par activa por dirección y cancela las operaciones en vuelo.
    pub async fn disconnect_peer(
        &self,
        address: Option<&str>,
    ) -> Result<PeerSessionSnapshot, DaemonError> {
        let mut g = self.inner.lock().await;
        let addr_to_remove = if let Some(a) = address {
            a.to_string()
        } else if let Some(first_key) = g.sessions.keys().next().cloned() {
            first_key
        } else {
            return Err(DaemonError::Business(
                "No active peer session to disconnect".into(),
            ));
        };

        if let Some(session) = g.sessions.remove(&addr_to_remove) {
            session.abort_handle.abort();
            g.in_flight_blocks.retain(|b| b.peer_id != addr_to_remove);

            let mut final_snap = session.snapshot;
            final_snap.status = PeerConnectionStatus::Disconnected;
            final_snap.transfer_rate_bps = 0;
            g.last_session = Some(final_snap.clone());

            connection::emit_event(&self.event_tx, &self.sequence, &final_snap);
            Ok(final_snap)
        } else {
            Err(DaemonError::Business(format!(
                "No active peer session found for '{addr_to_remove}'"
            )))
        }
    }

    /// Inserta una sesión ficticia para pruebas — la tarea asociada es
    /// un pending sin socket; `abort_handle` la finaliza limpiamente.
    #[cfg(test)]
    pub async fn insert_test_session(&self, snapshot: PeerSessionSnapshot) {
        let key = snapshot
            .peer_address
            .clone()
            .unwrap_or_else(|| "test-peer".to_string());
        let jh = tokio::spawn(std::future::pending::<()>());
        self.inner.lock().await.sessions.insert(
            key,
            types::PeerSessionHandle {
                snapshot,
                abort_handle: jh.abort_handle(),
                _permit: None,
            },
        );
    }

    /// Devuelve una instantánea de sesión individual para compatibilidad hacia atrás.
    pub async fn snapshot(&self) -> PeerSessionSnapshot {
        let g = self.inner.lock().await;
        if let Some(s) = g.sessions.values().next() {
            s.snapshot.clone()
        } else if let Some(last) = &g.last_session {
            last.clone()
        } else {
            PeerSessionSnapshot::default()
        }
    }

    /// Devuelve la instantánea de enjambre agregada, opcionalmente filtrada por hash de archivo.
    pub async fn swarm_snapshot(&self, file_hash: Option<&str>) -> SwarmStatusSnapshot {
        let g = self.inner.lock().await;
        let peers: Vec<PeerSessionSnapshot> = g
            .sessions
            .values()
            .filter(|s| {
                if let Some(fh) = file_hash {
                    s.snapshot.file_hash.as_deref() == Some(fh)
                } else {
                    true
                }
            })
            .map(|s| s.snapshot.clone())
            .collect();

        let in_flight = if file_hash.is_some() {
            let active_peer_addrs: std::collections::HashSet<_> = peers
                .iter()
                .filter_map(|p| p.peer_address.clone())
                .collect();
            g.in_flight_blocks
                .iter()
                .filter(|b| active_peer_addrs.contains(&b.peer_id))
                .cloned()
                .collect()
        } else {
            g.in_flight_blocks.clone()
        };

        SwarmStatusSnapshot {
            file_hash: file_hash.map(|s| s.to_string()),
            active_peer_count: peers.len(),
            peers,
            in_flight_blocks: in_flight,
        }
    }

    /// Rangos de bytes de bloques actualmente solicitados pero aún no recibidos para un
    /// archivo (`requestedblocks_list` de eMule — pintados en amarillo en la barra de
    /// estado). Vacío cuando el archivo no tiene ninguna sesión activa.
    pub(crate) fn pending_ranges_locked(g: &SwarmInnerState, file_hash: &str) -> Vec<(u64, u64)> {
        g.in_flight_blocks
            .iter()
            .filter(|b| {
                g.sessions
                    .get(&b.peer_id)
                    .is_some_and(|s| s.snapshot.file_hash.as_deref() == Some(file_hash))
            })
            .map(|b| (b.start, b.end))
            .collect()
    }

    /// Variante no bloqueante para rutas de emisión síncronas (`broadcast_download_update`
    /// se ejecuta sin contexto asíncrono): devuelve vacío cuando el estado del enjambre
    /// presenta contención — el siguiente evento lo refrescará.
    pub fn try_pending_ranges(&self, file_hash: &str) -> Vec<(u64, u64)> {
        self.inner
            .try_lock()
            .map(|g| Self::pending_ranges_locked(&g, file_hash))
            .unwrap_or_default()
    }

    /// Variante asíncrona para manejadores RPC (`downloads.get`).
    pub async fn pending_ranges_for_file(&self, file_hash: &str) -> Vec<(u64, u64)> {
        let g = self.inner.lock().await;
        Self::pending_ranges_locked(&g, file_hash)
    }

    /// Todos los rangos en vuelo agrupados por hash de archivo — un único bloqueo para
    /// `downloads.list`.
    pub async fn pending_ranges_by_file(&self) -> HashMap<String, Vec<(u64, u64)>> {
        let g = self.inner.lock().await;
        let mut out: HashMap<String, Vec<(u64, u64)>> = HashMap::new();
        for b in &g.in_flight_blocks {
            if let Some(hash) = g
                .sessions
                .get(&b.peer_id)
                .and_then(|s| s.snapshot.file_hash.clone())
            {
                out.entry(hash).or_default().push((b.start, b.end));
            }
        }
        out
    }

    /// Limpia cualquier sesión en caché y bloques en vuelo.
    pub async fn clear(&self) {
        let mut g = self.inner.lock().await;
        for (_, session) in g.sessions.drain() {
            session.abort_handle.abort();
        }
        g.in_flight_blocks.clear();
        g.last_session = None;
    }
}

#[cfg(test)]
mod tests {
    use super::types::{PeerSessionHandle, PeerSessionSnapshot, QueuedPeerState};
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn manager() -> PeerSessionManager {
        let (tx, _rx) = broadcast::channel(1);
        PeerSessionManager::new(tx, Arc::new(AtomicU64::new(0)))
    }

    const PEER: &str = "85.25.13.7:49662";

    fn queue_entry(file_hash: &str) -> QueuedPeerState {
        QueuedPeerState {
            file_hash: file_hash.to_string(),
            connected_at_unix_ms: Some(1_000),
            next_tcp_reask_unix_ms: Some(i64::MAX / 2),
            reask_pending: false,
        }
    }

    /// `UDPReaskForDownload` de eMule — el ping solo sale cuando el socket está
    /// muerto: el estado `DS_ONQUEUE` procede de `queued_peers`, no de la sesión.
    #[tokio::test]
    async fn queued_udp_targets_come_from_logical_queue_state() {
        let mgr = manager();
        let file_hash = "aa".repeat(16);
        {
            let mut g = mgr.inner.lock().await;
            g.queued_peers
                .insert(PEER.to_string(), queue_entry(&file_hash));
            g.peer_udp.insert(PEER.to_string(), (49672, 4));
        }
        let targets = mgr.queued_udp_targets().await;
        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].addr,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(85, 25, 13, 7)), 49672)
        );
        assert_eq!(hex::encode(targets[0].file_hash), file_hash);
        assert_eq!(targets[0].connected_at_unix_ms, Some(1_000));
    }

    /// Sesión TCP activa → `!(socket && socket->IsConnected())`: eMule
    /// no emitiría el ping, el objetivo queda excluido.
    #[tokio::test]
    async fn queued_udp_targets_excluded_while_session_alive() {
        let mgr = manager();
        let file_hash = "bb".repeat(16);
        {
            let mut g = mgr.inner.lock().await;
            g.queued_peers
                .insert(PEER.to_string(), queue_entry(&file_hash));
            g.peer_udp.insert(PEER.to_string(), (49672, 4));
            let task = tokio::spawn(async { std::future::pending::<()>().await });
            g.sessions.insert(
                PEER.to_string(),
                PeerSessionHandle {
                    snapshot: PeerSessionSnapshot {
                        status: PeerConnectionStatus::Queued,
                        ..Default::default()
                    },
                    abort_handle: task.abort_handle(),
                    _permit: None,
                },
            );
        }
        assert!(mgr.queued_udp_targets().await.is_empty());
    }

    /// `GetUDPVersion() == 0` → sin ping (eMule exige una versión UDP).
    #[tokio::test]
    async fn queued_udp_targets_requires_udp_version() {
        let mgr = manager();
        {
            let mut g = mgr.inner.lock().await;
            g.queued_peers
                .insert(PEER.to_string(), queue_entry(&"cc".repeat(16)));
            g.peer_udp.insert(PEER.to_string(), (49672, 0));
        }
        assert!(mgr.queued_udp_targets().await.is_empty());
    }

    /// `m_bReaskPending` de eMule (`DownloadClient.cpp:193`): el aplazamiento se
    /// aplica una sola vez sobre la entrada `queued_peers` del par (clave `ip:port`),
    /// pospone el límite de tiempo y luego es consumido por `take_reask_pending`
    /// (`ConnectionEstablished`, `BaseClient.cpp:1552`).
    #[tokio::test]
    async fn reask_pending_defer_and_consume() {
        let mgr = manager();
        let ip: IpAddr = "85.25.13.7".parse().unwrap();
        let file_hash = "ee".repeat(16);
        {
            let mut g = mgr.inner.lock().await;
            g.queued_peers
                .insert(PEER.to_string(), queue_entry(&file_hash));
        }

        // Nunca solicitado o hash erróneo → sin aplazamiento
        // (`GetLastAskedTime() > 0` exige haber pasado ya por un ciclo de cola).
        assert!(
            !mgr.defer_reask_for_queued_peer(ip, &"ff".repeat(16), 5_000)
                .await
        );

        // Primer aplazamiento → indicador establecido, límite pospuesto.
        assert!(mgr.defer_reask_for_queued_peer(ip, &file_hash, 9_999).await);
        {
            let g = mgr.inner.lock().await;
            let q = g.queued_peers.get(PEER).unwrap();
            assert!(q.reask_pending);
            assert_eq!(q.next_tcp_reask_unix_ms, Some(9_999));
        }

        // `!m_bReaskPending` — el aplazamiento no se repite: el ciclo
        // siguiente procede con normalidad.
        assert!(!mgr.defer_reask_for_queued_peer(ip, &file_hash, 9_999).await);

        // Consumo al recibir la conexión entrante del par.
        let taken = mgr.take_reask_pending(ip).await.expect("pending entry");
        assert_eq!(taken.file_hash, file_hash);
        assert!(!mgr.inner.lock().await.queued_peers[PEER].reask_pending);
        // El indicador se ha consumido — una segunda llamada devuelve None.
        assert!(mgr.take_reask_pending(ip).await.is_none());
    }

    /// Entrada cuyo límite de reconsulta ha vencido hace más de dos
    /// ciclos → purgada durante la lectura (fuente caída sin vuelta a la cola).
    #[tokio::test]
    async fn queued_udp_targets_prunes_long_expired_entries() {
        let mgr = manager();
        let stale = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
            - 3 * crate::services::peer_session::status_query::FILE_REASK_INTERVAL.as_millis()
                as i64;
        {
            let mut g = mgr.inner.lock().await;
            g.queued_peers.insert(
                PEER.to_string(),
                QueuedPeerState {
                    file_hash: "dd".repeat(16),
                    connected_at_unix_ms: Some(1_000),
                    next_tcp_reask_unix_ms: Some(stale),
                    reask_pending: false,
                },
            );
            g.peer_udp.insert(PEER.to_string(), (49672, 4));
        }
        assert!(mgr.queued_udp_targets().await.is_empty());
        assert!(mgr.inner.lock().await.queued_peers.is_empty());
    }
}

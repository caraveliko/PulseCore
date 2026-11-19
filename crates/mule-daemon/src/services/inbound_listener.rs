// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Coordinador del servicio de escucha TCP eD2K entrante.

pub mod connection;
pub mod dispatcher;
pub mod share_browser;
pub mod subnet_guard;
pub mod types;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Tamaño de cola *backlog* para el socket de escucha TCP dual-stack.
const TCP_LISTEN_BACKLOG: i32 = 1024;

/// Enlaza un `TcpListener` de Tokio con soporte dual-stack IPv6+IPv4 (`IPV6_V6ONLY = false`)
/// cuando se escucha en `[::]`.
async fn bind_dual_stack_tcp_listener(bind_addr: SocketAddr) -> Result<TcpListener, String> {
    if bind_addr.is_ipv6() && bind_addr.ip().is_unspecified() {
        let domain = socket2::Domain::IPV6;
        let sock = socket2::Socket::new(
            domain,
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )
        .map_err(|e| format!("Failed to create dual-stack IPv6 socket for {bind_addr}: {e}"))?;
        let _ = sock.set_only_v6(false);
        let _ = sock.set_reuse_address(true);
        sock.set_nonblocking(true)
            .map_err(|e| format!("Failed to set nonblocking on {bind_addr}: {e}"))?;
        sock.bind(&socket2::SockAddr::from(bind_addr))
            .map_err(|e| format!("Failed to bind dual-stack listener to {bind_addr}: {e}"))?;
        sock.listen(TCP_LISTEN_BACKLOG)
            .map_err(|e| format!("Failed to listen on dual-stack socket {bind_addr}: {e}"))?;
        let std_listener: std::net::TcpListener = sock.into();
        TcpListener::from_std(std_listener)
            .map_err(|e| format!("Failed to adopt dual-stack listener {bind_addr}: {e}"))
    } else {
        TcpListener::bind(bind_addr)
            .await
            .map_err(|e| format!("Failed to bind TCP listener to {bind_addr}: {e}"))
    }
}

use mule_network_policy::{ConnectionPurpose, NetworkPolicyManager};

pub use types::{
    InboundListenerService, InboundListenerSnapshot, MAX_INBOUND_PER_IP, MAX_READ_BUFFER_SIZE,
};

use crate::services::downloads::DownloadService;
use crate::services::upload_queue::UploadManager;

impl InboundListenerService {
    /// Crea una nueva instancia de InboundListenerService.
    pub fn new(
        network_policy: Arc<NetworkPolicyManager>,
        downloads: Arc<Mutex<DownloadService>>,
        user_hash: [u8; 16],
        sui_pubkey_der: Option<Vec<u8>>,
        obfuscation_enabled: bool,
        obfuscation_required: bool,
        obfuscation_padding_max: u8,
        sui_stats: Arc<crate::services::identity::SuiStats>,
    ) -> Self {
        Self {
            network_policy,
            downloads,
            upload_manager: UploadManager::new(),
            is_running: Arc::new(AtomicBool::new(false)),
            listen_addr: Arc::new(Mutex::new(None)),
            listener_handle: Arc::new(Mutex::new(None)),
            user_hash: std::sync::Arc::new(std::sync::RwLock::new(user_hash)),
            sui_pubkey_der: std::sync::Arc::new(std::sync::RwLock::new(sui_pubkey_der)),
            paranoid_mode: Arc::new(AtomicBool::new(false)),
            identity: None,
            last_sig_ip: Arc::new(Mutex::new(None)),
            sui_stats,
            active_conns: Arc::new(Mutex::new(HashMap::new())),
            obfuscation_enabled: std::sync::Arc::new(AtomicBool::new(obfuscation_enabled)),
            obfuscation_required: std::sync::Arc::new(AtomicBool::new(obfuscation_required)),
            obfuscation_padding_max,
            peer_session: Arc::new(std::sync::RwLock::new(None)),
            server_session: Arc::new(std::sync::RwLock::new(None)),
            client_udp: Arc::new(std::sync::RwLock::new(None)),
            kad_udp: Arc::new(std::sync::RwLock::new(None)),
            kad: Arc::new(std::sync::RwLock::new(None)),
            kad_udp_port: 0,
            sx_env: Arc::new(std::sync::RwLock::new(None)),
            subnet_guard: subnet_guard::SubnetQuotaGuard::new(),
            quic_port: Arc::new(std::sync::atomic::AtomicU16::new(0)),
        }
    }

    /// Actualiza el puerto QUIC UDP local anunciado en `OP_HELLO_ANSWER`.
    pub fn set_quic_port(&self, port: u16) {
        self.quic_port.store(port, Ordering::Relaxed);
    }

    /// Inyecta el entorno de intercambio de fuentes compartido (política de red +
    /// diarios de limitación) creado por `AppState`.
    pub fn set_sx_env(&mut self, env: crate::services::peer_session::source_exchange::SxEnv) {
        *self.sx_env.write().unwrap() = Some(env);
    }

    /// `SxEnv` compartido para este listener; construye un respaldo a partir de la
    /// propia política de red del listener + hash de usuario cuando `AppState` no
    /// inyectó uno (pruebas, construcción manual).
    pub(crate) fn sx_env(&self) -> crate::services::peer_session::source_exchange::SxEnv {
        self.sx_env.read().unwrap().clone().unwrap_or_else(|| {
            crate::services::peer_session::source_exchange::SxEnv::new(
                (*self.network_policy).clone(),
                *self.user_hash.read().unwrap(),
            )
        })
    }

    /// Vista de identidad eD2K local para las autocomprobaciones de `CanAddSource` (eMule
    /// `PartFile.cpp:2380-2403`): `client_id` actual asignado por el servidor,
    /// dirección del servidor y nuestro puerto de escucha TCP.
    pub(crate) async fn sx_local_ids(
        &self,
        listen_port: u16,
    ) -> crate::services::peer_session::source_exchange::SxLocalIds {
        use crate::services::peer_session::source_exchange::SxLocalIds;
        // `CKademlia::IsConnected() && !IsFirewalled()` — socket UDP
        // activo + prueba de cortafuegos positiva.
        let kad_udp_svc = { self.kad_udp.read().unwrap().clone() };
        let kad_open = match kad_udp_svc {
            Some(svc) => {
                let snap = svc.get_snapshot().await;
                let fw_status = {
                    let tracker = svc.firewall_tracker();
                    let fw = tracker.lock().await;
                    fw.status
                };
                snap.is_active
                    && fw_status
                        == crate::services::kad_udp::types::UdpFirewallStatus::Open
            }
            None => false,
        };
        let session = self.server_session.read().unwrap().clone();
        match session {
            Some(sm) => {
                let snap = sm.snapshot().await;
                let (server_ip, server_port) = snap
                    .server_address
                    .as_deref()
                    .and_then(|a| a.parse::<SocketAddr>().ok())
                    .map(|sa| match sa.ip() {
                        std::net::IpAddr::V4(v4) => (u32::from_le_bytes(v4.octets()), sa.port()),
                        _ => (0, sa.port()),
                    })
                    .unwrap_or((0, 0));
                SxLocalIds {
                    client_id: snap.client_id.unwrap_or(0),
                    server_ip,
                    server_port,
                    tcp_port: listen_port,
                    is_high_id: snap.is_high_id,
                    kad_open,
                }
            }
            None => SxLocalIds {
                tcp_port: listen_port,
                kad_open,
                ..SxLocalIds::default()
            },
        }
    }

    /// Establece el puerto Kad UDP anunciado a pares entrantes en las respuestas de saludo.
    pub fn set_kad_udp_port(&mut self, port: u16) {
        self.kad_udp_port = port;
    }

    /// Actualiza en vivo la política de ofuscación (cifrado de protocolo) para conexiones
    /// entrantes — refleja las preferencias de eMule "Soportar conexiones ofuscadas /
    /// Requerir conexiones ofuscadas".
    pub fn set_obfuscation(&self, enabled: bool, required: bool) {
        self.obfuscation_enabled
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
        self.obfuscation_required
            .store(required, std::sync::atomic::Ordering::Relaxed);
    }

    /// Inyecta el servicio UDP de cliente (contabilidad de versión UDP entrante).
    pub fn set_client_udp(&mut self, svc: crate::services::client_udp::ClientUdpService) {
        *self.client_udp.write().unwrap() = Some(svc);
    }

    /// Inyecta el servicio UDP Kad (lado servidor de la comprobación de cortafuegos UDP).
    pub fn set_kad_udp(&mut self, svc: Arc<crate::services::kad_udp::KadUdpService>) {
        *self.kad_udp.write().unwrap() = Some(svc);
    }

    /// Inyecta el servicio Kad (bootstrap desde cliente en hellos entrantes).
    pub fn set_kad(&mut self, svc: Arc<crate::services::kad::KadService>) {
        *self.kad.write().unwrap() = Some(svc);
    }

    /// Inyecta el gestor de sesiones de pares (adopción de devolución de llamada LowID).
    pub fn set_peer_session(
        &mut self,
        peer_session: crate::services::peer_session::PeerSessionManager,
    ) {
        *self.peer_session.write().unwrap() = Some(peer_session);
    }

    /// Inyecta el gestor de sesiones de servidor (nuestro ID de cliente HighID para sesiones de devolución).
    pub fn set_server_session(
        &mut self,
        server_session: crate::services::server_session::ServerSessionManager,
    ) {
        *self.server_session.write().unwrap() = Some(server_session);
    }

    /// Inyecta el servicio de identidad local para poder firmar desafíos entrantes.
    pub fn set_identity(&mut self, identity: Arc<Mutex<crate::services::IdentityService>>) {
        self.identity = Some(identity);
    }

    /// Actualiza el material de identidad anunciado a pares entrantes tras una
    /// rotación de clave (`identity.rotate` / `identityRotationHours`).
    pub fn set_identity_material(&self, user_hash: [u8; 16], sui_pubkey_der: Option<Vec<u8>>) {
        *self.user_hash.write().unwrap() = user_hash;
        *self.sui_pubkey_der.write().unwrap() = sui_pubkey_der;
    }

    /// Activa/desactiva el modo paranoide (pares ofuscados + SUI verificados
    /// únicamente para ranuras de subida).
    pub fn set_paranoid_mode(&self, enabled: bool) {
        self.paranoid_mode.store(enabled, Ordering::Relaxed);
    }

    /// Accede al gestor de subidas subyacente.
    pub fn upload_manager(&self) -> &UploadManager {
        &self.upload_manager
    }

    /// Inicia el listener entrante en la dirección de enlace solicitada.
    pub async fn start(&self, bind_addr: SocketAddr) -> Result<SocketAddr, String> {
        if self
            .is_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("Inbound listener is already running".to_string());
        }

        let listen_permit = match self
            .network_policy
            .request_tcp_listen(bind_addr, ConnectionPurpose::Listener)
        {
            Ok(p) => p,
            Err(e) => {
                self.is_running.store(false, Ordering::SeqCst);
                return Err(format!("Policy denied listener on {bind_addr}: {e}"));
            }
        };

        let tokio_listener = match bind_dual_stack_tcp_listener(bind_addr).await {
            Ok(l) => l,
            Err(e) => {
                self.is_running.store(false, Ordering::SeqCst);
                return Err(e);
            }
        };

        let actual_addr = match tokio_listener.local_addr() {
            Ok(a) => a,
            Err(e) => {
                self.is_running.store(false, Ordering::SeqCst);
                return Err(format!("Failed to retrieve local address: {e}"));
            }
        };

        *self.listen_addr.lock().await = Some(actual_addr);

        let cancel_token = listen_permit.cancellation_token().clone();
        let service_self = self.clone();

        let handle = tokio::spawn(async move {
            info!("Inbound eD2K listener started on {}", actual_addr);
            let _permit = listen_permit;

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        info!("Inbound listener cancelled by network policy / kill switch");
                        break;
                    }
                    accept_res = tokio_listener.accept() => {
                        match accept_res {
                            Ok((socket, remote_addr)) => {
                                let ip = remote_addr.ip();
                                // Detección anti-DDoS / inundación dinámica antes de la asignación de tarea
                                if service_self.network_policy.ip_filter().record_connection_attempt(ip) {
                                    warn!(
                                        "[AUTO_BAN] Dropped inbound connection flood from {peer}",
                                        peer = crate::services::peer_session::log_peer_addr(remote_addr)
                                    );
                                    drop(socket);
                                    continue;
                                }
                                let _subnet_permit = match service_self.subnet_guard.try_acquire(ip) {
                                    Ok(p) => p,
                                    Err(subnet_guard::SubnetQuotaExceeded) => {
                                        warn!(
                                            "Rejected inbound connection from {peer}: subnet quota reached",
                                            peer = crate::services::peer_session::log_peer_addr(remote_addr)
                                        );
                                        drop(socket);
                                        continue;
                                    }
                                };
                                let mut conns = service_self.active_conns.lock().await;
                                let total_conns: usize = conns.values().sum();
                                // El límite entrante sigue el presupuesto de sockets
                                // (`maxConnections`, en vivo) al igual que en eMule —
                                // sin constantes ocultas si el usuario
                                // aumenta el presupuesto.
                                let inbound_cap = service_self
                                    .network_policy
                                    .budgets()
                                    .max_active_peer_connections;
                                if total_conns >= inbound_cap {
                                    warn!("Rejected inbound connection from {peer}: global limit reached ({inbound_cap})", peer = crate::services::peer_session::log_peer_addr(remote_addr));
                                    drop(socket);
                                    continue;
                                }
                                let ip_count = conns.entry(ip).or_insert(0);
                                if *ip_count >= MAX_INBOUND_PER_IP {
                                    warn!("Rejected inbound connection from {peer}: per-IP limit reached ({MAX_INBOUND_PER_IP})", peer = crate::services::peer_session::log_peer_addr(remote_addr));
                                    drop(socket);
                                    continue;
                                }
                                *ip_count += 1;
                                drop(conns);

                                let service_clone = service_self.clone();
                                let cancel_clone = cancel_token.clone();
                                tokio::spawn(async move {
                                    let _subnet_permit = _subnet_permit;
                                    service_clone.handle_inbound_connection(socket, remote_addr, actual_addr, cancel_clone).await;
                                    let mut conns = service_clone.active_conns.lock().await;
                                    if let std::collections::hash_map::Entry::Occupied(mut e) = conns.entry(ip) {
                                        if *e.get() <= 1 {
                                            e.remove();
                                        } else {
                                            *e.get_mut() -= 1;
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                warn!("Error accepting inbound connection on {}: {}", actual_addr, e);
                                break;
                            }
                        }
                    }
                }
            }

            service_self.is_running.store(false, Ordering::SeqCst);
            *service_self.listen_addr.lock().await = None;
            service_self.upload_manager.abort_all().await;
            service_self.active_conns.lock().await.clear();
            service_self.subnet_guard.clear();
            info!("Inbound eD2K listener stopped");
        });

        *self.listener_handle.lock().await = Some(handle);
        Ok(actual_addr)
    }

    /// Detiene el listener entrante limpiamente.
    pub async fn stop(&self) {
        self.is_running.store(false, Ordering::SeqCst);
        let handle = self.listener_handle.lock().await.take();
        if let Some(h) = handle {
            h.abort();
            let _ = h.await;
        }
        *self.listen_addr.lock().await = None;
        self.upload_manager.abort_all().await;
        self.active_conns.lock().await.clear();
        self.subnet_guard.clear();
    }

    /// Devuelve una instantánea del estado del listener.
    pub async fn snapshot(&self) -> InboundListenerSnapshot {
        let is_listening = self.is_running.load(Ordering::SeqCst);
        let addr = self.listen_addr.lock().await.map(|a| a.to_string());
        let active_snaps = self.upload_manager.active_snapshots().await;
        let queue = self.upload_manager.list_queue().await;
        let upload_rate = active_snaps.iter().map(|s| s.upload_rate_bps).sum();

        InboundListenerSnapshot {
            is_listening,
            listen_address: addr,
            active_uploads: active_snaps.len(),
            queued_uploads: queue.len(),
            upload_rate_bps: upload_rate,
        }
    }
}

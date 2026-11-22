// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Servicio UDP cliente a cliente (`ClientUDPSocket`, eMule 0.72a).
//!
//! Comparte el socket UDP del demonio con Kad (eMule multiplexa eD2K UDP y
//! Kademlia en el mismo puerto UDP): `KadUdpService` enruta aquí los
//! datagramas cuyo byte de protocolo es `PR_EMULE`/`PR_ED2K`.
//!
//! Responsabilidades:
//! - Entrante `OP_REASKFILEPING`: responde `OP_REASKACK` con la posición en la
//!   cola de subida del solicitante, `OP_FILENOTFOUND` cuando el archivo no se
//!   comparte o `OP_QUEUEFULL` cuando la cola está saturada (eMule ignora los
//!   remitentes desconocidos salvo que la cola esté casi llena).
//! - Entrante `OP_REASKACK`/`OP_QUEUEFULL`/`OP_FILENOTFOUND`: validado frente a
//!   las reconsultas pendientes y aplicado al registro de posición de la sesión
//!   del par en cola correspondiente.
//! - Saliente `send_reask_file_ping`: mantenimiento periódico de conexión hacia
//!   fuentes en cola que anunciaron un puerto UDP de eMule (deduplicado,
//!   cadencia de ~21 min como `CUpDownClient::UDPReaskForDownload`).

use crate::services::peer_session::log_peer_addr;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::sync::Mutex;

use mule_protocol::ed2k::client_udp::{self, ClientUdpMessage};
use mule_protocol::ed2k::protocol::{PR_ED2K, PR_EMULE};

use crate::services::downloads::DownloadService;
use crate::services::upload_queue::UploadManager;

/// Intervalo mínimo entre dos `OP_REASKFILEPING` hacia el mismo
/// (par, archivo) — eMule vuelve a consultar fuentes en cola aproximadamente cada 21 minutos.
pub const UDP_REASK_INTERVAL: Duration = Duration::from_secs(21 * 60);
/// Una reconsulta permanece "pendiente" este tiempo aguardando su confirmación (ack).
const PENDING_TTL: Duration = Duration::from_secs(120);
/// `UDPReaskForDownload` de eMule (`DownloadClient.cpp:1340`) — la protección
/// contra fallos solo se activa una vez que se han enviado más de este número de paquetes UDP
/// al par (`m_nTotalUDPPackets > 3`).
const UDP_GUARD_MIN_ATTEMPTS: u32 = 3;
/// `m_nFailedUDPPackets / m_nTotalUDPPackets > .3f` de eMule — por encima de esta
/// proporción de fallos (porcentaje) se abandonan las reconsultas UDP al par.
const UDP_GUARD_MAX_FAILED_PERCENT: u32 = 30;
/// `AllowCalbackRequest` de eMule (`ClientList.cpp:921`) — como máximo un
/// `OP_DIRECTCALLBACKREQ` aceptado por IP de origen en esta ventana.
const DIRECT_CALLBACK_MIN_INTERVAL: Duration = Duration::from_secs(180);

/// Resultado de una reconsulta UDP remota, registrado por IP de par.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UdpReaskResult {
    /// El par respondió `OP_REASKACK` con esta posición en cola.
    Rank(u16),
    /// El par respondió `OP_QUEUEFULL`.
    QueueFull,
    /// El par respondió `OP_FILENOTFOUND` — ya no comparte el archivo.
    FileNotFound,
}

/// Destino de mantenimiento de conexión saliente producido por `PeerSessionManager`.
#[derive(Debug, Clone)]
pub struct UdpReaskTarget {
    /// Endpoint UDP de la fuente en cola (su puerto UDP de eMule anunciado).
    pub addr: SocketAddr,
    /// Hash de archivo de la sesión en cola.
    pub file_hash: [u8; 16],
    /// Cuándo se estableció la sesión TCP subyacente — utilizado para aplicar
    /// la condición de eMule de "último intento de conexión anterior a ~20 min"
    /// antes de gastar una reconsulta UDP.
    pub connected_at_unix_ms: Option<i64>,
    /// Límite de tiempo para la siguiente reconsulta TCP de archivo en la sesión en cola —
    /// `GetTimeUntilReask()` de eMule. El coordinador lanza la reconsulta
    /// UDP únicamente dentro de la ventana de `(1 s, 2 min)` de eMule antes de este
    /// límite (`PartFile.cpp:2291`).
    pub next_tcp_reask_unix_ms: Option<i64>,
}

/// Contadores UDP por par — `m_nTotalUDPPackets` /
/// `m_nFailedUDPPackets` de eMule (`updownclient.h:561-562`).
#[derive(Debug, Default, Clone)]
struct UdpPeerStats {
    /// Total de `OP_REASKFILEPING` enviados al par.
    total: u32,
    /// Pings que quedaron sin respuesta hasta la siguiente reconsulta.
    failed: u32,
}

#[derive(Default)]
struct ClientUdpInner {
    socket: Option<Arc<UdpSocket>>,
    /// Intervalo de reconsulta: (IP de par, hash de archivo) → enviado en (limitación
    /// `UDP_REASK_INTERVAL`, independiente de la espera de ack).
    pending: HashMap<(IpAddr, [u8; 16]), Instant>,
    /// Pings a la espera de una respuesta (`m_bUDPPending` de eMule) — expirados
    /// tras `PENDING_TTL` y entonces contabilizados como fallos.
    awaiting_ack: HashMap<(IpAddr, [u8; 16]), Instant>,
    /// Contadores de fallos por IP (protección del 30 % de `UDPReaskForDownload`).
    udp_stats: HashMap<IpAddr, UdpPeerStats>,
    /// Último resultado por IP de par.
    results: HashMap<IpAddr, (UdpReaskResult, Instant)>,
    /// Versión UDP anunciada por cada cliente entrante en cola (IP → versión),
    /// utilizada para dar formato a la carga útil `OP_REASKACK` (`sender->GetUDPVersion()`).
    client_udp_versions: HashMap<IpAddr, u8>,
    /// Última solicitud de devolución de llamada directa aceptada por IP —
    /// `listDirectCallbackRequests` de eMule (ventana de 180 s).
    direct_callbacks: HashMap<IpAddr, Instant>,
}

/// Servicio UDP de cliente compartido (manejador clonable).
#[derive(Clone, Default)]
pub struct ClientUdpService {
    inner: Arc<Mutex<ClientUdpInner>>,
    downloads: Arc<std::sync::RwLock<Option<Arc<Mutex<DownloadService>>>>>,
    upload_manager: Arc<std::sync::RwLock<Option<UploadManager>>>,
    peer_session: Arc<std::sync::RwLock<Option<crate::services::peer_session::PeerSessionManager>>>,
    /// Registro de retransmisión buddy — los datagramas `OP_REASKCALLBACKUDP` para el ID de
    /// cliente de nuestro buddy se reenvían a su enlace TCP como `OP_REASKCALLBACKTCP`.
    buddy: Arc<std::sync::RwLock<Option<Arc<Mutex<crate::services::kad_udp::BuddyRegistry>>>>>,
    /// Listener entrante usado para `connect_back` cuando un par nos solicita (estando
    /// con cortafuegos en Kad) marcar hacia él (`OP_DIRECTCALLBACKREQ` → `TryToConnect`
    /// de eMule).
    inbound:
        Arc<std::sync::RwLock<Option<crate::services::inbound_listener::InboundListenerService>>>,
    /// Sesión de servidor — proporciona nuestro `client_id` de eD2K para el hello de
    /// devolución de conexión (eMule envía su ID real en `TryToConnect`).
    server_session: Arc<std::sync::RwLock<Option<crate::services::ServerSessionManager>>>,
}

impl ClientUdpService {
    /// Crea un nuevo servicio no enlazado.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inyecta el socket UDP compartido una vez que `KadUdpService` está enlazado.
    pub async fn set_socket(&self, socket: Arc<UdpSocket>) {
        self.inner.lock().await.socket = Some(socket);
    }

    /// Libera la referencia al socket compartido al detener o reenlazar.
    pub async fn clear_socket(&self) {
        self.inner.lock().await.socket = None;
    }

    /// Inyecta el servicio de descargas (búsqueda de archivos compartidos).
    pub fn set_downloads(&self, downloads: Arc<Mutex<DownloadService>>) {
        *self.downloads.write().unwrap_or_else(|e| e.into_inner()) = Some(downloads);
    }

    /// Inyecta el gestor de cola de subidas (búsqueda de rango para pings entrantes).
    pub fn set_upload_manager(&self, mgr: UploadManager) {
        *self.upload_manager.write().unwrap_or_else(|e| e.into_inner()) = Some(mgr);
    }

    /// Inyecta el gestor de sesiones de pares (aplicación de rango + contabilidad de reconsultas).
    pub fn set_peer_session(&self, mgr: crate::services::peer_session::PeerSessionManager) {
        *self.peer_session.write().unwrap_or_else(|e| e.into_inner()) = Some(mgr);
    }

    /// Inyecta el registro de retransmisión buddy compartido con `KadUdpService`.
    pub fn set_buddy_registry(&self, reg: Arc<Mutex<crate::services::kad_udp::BuddyRegistry>>) {
        *self.buddy.write().unwrap_or_else(|e| e.into_inner()) = Some(reg);
    }

    /// Inyecta el listener entrante utilizado para la devolución de conexión directa.
    pub fn set_inbound_listener(
        &self,
        svc: crate::services::inbound_listener::InboundListenerService,
    ) {
        *self.inbound.write().unwrap_or_else(|e| e.into_inner()) = Some(svc);
    }

    /// Inyecta la sesión de servidor (`client_id` eD2K para devolución de conexión).
    pub fn set_server_session(&self, svc: crate::services::ServerSessionManager) {
        *self.server_session.write().unwrap_or_else(|e| e.into_inner()) = Some(svc);
    }

    /// Registra la versión UDP que un cliente entrante anunció en su hello,
    /// de modo que las respuestas `OP_REASKACK` coincidan con la semántica de `sender->GetUDPVersion()`.
    pub async fn note_client_udp_version(&self, ip: IpAddr, version: u8) {
        self.inner
            .lock()
            .await
            .client_udp_versions
            .insert(ip, version);
    }

    /// Envía `OP_REASKFILEPING` a `target` salvo que se haya enviado una reconsulta equivalente
    /// dentro de [`UDP_REASK_INTERVAL`]. Devuelve true cuando un paquete salió.
    pub async fn send_reask_file_ping(
        &self,
        target: &UdpReaskTarget,
        complete_sources: u16,
    ) -> bool {
        let ip = target.addr.ip();
        let key = (ip, target.file_hash);
        {
            let mut inner = self.inner.lock().await;

            // Un ping sin respuesta más allá de `PENDING_TTL` es un fallo —
            // eMule incrementa `m_nFailedUDPPackets` cuando la reconsulta TCP
            // siguiente encuentra `m_bUDPPending` todavía como verdadero (`AskForDownload`).
            let mut expired: Vec<IpAddr> = Vec::new();
            inner.awaiting_ack.retain(|(eip, _), at| {
                if at.elapsed() >= PENDING_TTL {
                    expired.push(*eip);
                    false
                } else {
                    true
                }
            });
            for eip in expired {
                inner.udp_stats.entry(eip).or_default().failed += 1;
            }

            // Protección de eMule `UDPReaskForDownload` (`DownloadClient.cpp:1340`):
            // tras >3 paquetes, se cesan las reconsultas UDP cuando más del 30 %
            // quedaron sin respuesta.
            if inner.udp_stats.get(&ip).is_some_and(|st| {
                st.total > UDP_GUARD_MIN_ATTEMPTS
                    && st.failed * 100 > st.total * UDP_GUARD_MAX_FAILED_PERCENT
            }) {
                tracing::debug!(
                    "Client UDP: reask skipped for {} — UDP failure ratio > {}%",
                    log_peer_addr(target.addr),
                    UDP_GUARD_MAX_FAILED_PERCENT
                );
                return false;
            }

            // `if (!m_reqfile || m_bUDPPending) return;` de eMule — una sola
            // reconsulta UDP en vuelo por par, independientemente del archivo.
            if inner.awaiting_ack.keys().any(|(eip, _)| *eip == ip) {
                return false;
            }

            inner
                .pending
                .retain(|_, at| at.elapsed() < UDP_REASK_INTERVAL);
            if inner.pending.contains_key(&key) {
                return false;
            }
            let Some(socket) = inner.socket.clone() else {
                return false;
            };
            let now = Instant::now();
            inner.pending.insert(key, now);
            inner.awaiting_ack.insert(key, now);
            inner.udp_stats.entry(ip).or_default().total += 1;
            let pkt = client_udp::encode_reask_file_ping(&target.file_hash, complete_sources);
            tracing::debug!(
                "Client UDP: OP_REASKFILEPING -> {} (hash {})",
                log_peer_addr(target.addr),
                hex::encode(target.file_hash)
            );
            if let Err(e) = socket.send_to(&pkt, target.addr).await {
                tracing::debug!(
                    "Client UDP: send_reask_file_ping to {} failed: {e}",
                    log_peer_addr(target.addr)
                );
            }
        }
        true
    }

    /// Paso 4 de `TryToConnect` de eMule (`BaseClient.cpp:1403-1417`): envía
    /// `OP_DIRECTCALLBACKREQ` a una fuente con cortafuegos TCP y UDP abierto que
    /// anunció `SupportsDirectUDPCallback` (bit 3 de opciones de conexión en su
    /// contacto Kad publicado). Se espera que el par marque de vuelta a nuestro puerto TCP;
    /// la conexión entrante se adopta a través del registro de devolución de llamada
    /// pendiente `kadcb:`.
    ///
    /// `conn_opts` es nuestro byte `GetMyConnectOptions(true, false)` — solo indicadores
    /// de cifrado, el bit de devolución de llamada directa se mantiene a cero.
    ///
    /// Devuelve `true` cuando el socket UDP está activo y el datagrama fue
    /// despachado (disparo y olvido, como `SendPacket` de eMule — la opción
    /// se considera entonces ejercida para este intento de conexión).
    pub async fn send_direct_callback_req(
        &self,
        target: SocketAddr,
        our_tcp_port: u16,
        user_hash: &[u8; 16],
        conn_opts: u8,
    ) -> bool {
        let socket = { self.inner.lock().await.socket.clone() };
        let Some(socket) = socket else {
            return false;
        };
        let pkt = client_udp::encode_direct_callback_req(our_tcp_port, user_hash, conn_opts);
        tracing::info!(
            "Client UDP: OP_DIRECTCALLBACKREQ -> {peer} (tcp={our_tcp_port})",
            peer = log_peer_addr(target)
        );
        if let Err(e) = socket.send_to(&pkt, target).await {
            tracing::debug!(
                "Client UDP: OP_DIRECTCALLBACKREQ to {} failed: {e}",
                log_peer_addr(target)
            );
        }
        true
    }

    /// Envía un datagrama UDP de cliente ya entramado a `target`.
    /// Utilizado por la retransmisión `OP_REASKCALLBACKTCP` (`ListenSocket.cpp:1391`):
    /// la respuesta de reconsulta (`REASKACK`/`FILENOTFOUND`/`QUEUEFULL`) vuelve a
    /// la dirección del *solicitante original* portada en el paquete, no al
    /// buddy que la retransmitió. Disparo y olvido como `SendPacket` de eMule.
    pub async fn send_raw(&self, target: SocketAddr, datagram: &[u8]) -> bool {
        let socket = { self.inner.lock().await.socket.clone() };
        let Some(socket) = socket else {
            return false;
        };
        socket.send_to(datagram, target).await.is_ok()
    }

    /// Último resultado de reconsulta UDP registrado para una IP de par.
    pub async fn last_result(&self, ip: IpAddr) -> Option<UdpReaskResult> {
        let inner = self.inner.lock().await;
        inner
            .results
            .get(&ip)
            .filter(|(_, at)| at.elapsed() < UDP_REASK_INTERVAL)
            .map(|(r, _)| r.clone())
    }

    /// Devuelve true si el byte de protocolo del datagrama pertenece al canal
    /// UDP de cliente (`PR_EMULE` / `PR_ED2K`, es decir, no Kademlia).
    pub fn is_client_udp_protocol(proto: u8) -> bool {
        proto == PR_EMULE || proto == PR_ED2K
    }

    /// Gestiona un datagrama UDP de cliente entrante (ya filtrado por políticas por
    /// el bucle de socket compartido — ipfilter + puerto en `kad_udp.rs`).
    ///
    /// `ClientUDPSocket.cpp:76` de eMule descarta paquetes de clientes baneados
    /// (`clientlist->IsBannedClient`) — nuestra lista de baneos por corrupción cumple esa
    /// función: una fuente baneada tampoco puede enviarnos pings de reconsulta.
    pub async fn handle_datagram(&self, peer_addr: SocketAddr, data: &[u8]) {
        let banned = {
            let downloads = self.downloads.read().unwrap_or_else(|e| e.into_inner()).clone();
            match downloads.as_ref() {
                Some(d) => d.lock().await.is_source_banned(&peer_addr.to_string()),
                None => false,
            }
        };
        if banned {
            tracing::debug!(
                "Client UDP: dropped datagram from banned peer {}",
                log_peer_addr(peer_addr)
            );
            return;
        }

        let msg = match client_udp::decode_client_udp(data) {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(
                    "Client UDP: decode error from {peer}: {e}",
                    peer = log_peer_addr(peer_addr)
                );
                return;
            }
        };
        match msg {
            ClientUdpMessage::ReaskFilePing { file_hash, .. } => {
                self.handle_reask_file_ping(peer_addr, file_hash).await
            }
            ClientUdpMessage::ReaskAck {
                queue_rank,
                part_status,
            } => {
                self.handle_reask_ack(peer_addr, UdpReaskResult::Rank(queue_rank), part_status)
                    .await
            }
            ClientUdpMessage::QueueFull => {
                self.handle_reask_ack(peer_addr, UdpReaskResult::QueueFull, None)
                    .await
            }
            ClientUdpMessage::FileNotFound => {
                self.handle_reask_ack(peer_addr, UdpReaskResult::FileNotFound, None)
                    .await
            }
            ClientUdpMessage::ReaskCallback { buddy_id, rest } => {
                self.handle_reask_callback(peer_addr, buddy_id, rest).await
            }
            ClientUdpMessage::DirectCallbackReq {
                tcp_port,
                user_hash,
                conn_opts,
            } => {
                self.handle_direct_callback_req(peer_addr, tcp_port, user_hash, conn_opts)
                    .await
            }
            ClientUdpMessage::PortTest { request } => {
                // eMule responde a la prueba `0x12` a través de su socket TCP de prueba
                // dedicado (`SendPortTestReply('1')`) — sin respuesta UDP.
                // No implementamos la herramienta de prueba de puertos:
                // acuse registrado en log únicamente.
                tracing::debug!(
                    "Client UDP: OP_PORTTEST 0x{request:02X} from {peer} (no reply — port test tool unsupported)",
                    peer = log_peer_addr(peer_addr)
                );
            }
            ClientUdpMessage::Unknown { opcode } => {
                tracing::debug!(
                    "Client UDP: ignoring opcode 0x{opcode:02X} from {peer}",
                    peer = log_peer_addr(peer_addr)
                );
            }
        }
    }

    /// `case OP_REASKFILEPING` de eMule — responder con el rango en cola cuando el
    /// remitente espera en nuestra cola de subida para ese archivo, `OP_FILENOTFOUND`
    /// cuando el hash no está compartido, `OP_QUEUEFULL` cuando está saturada; los remitentes
    /// desconocidos son de lo contrario ignorados (forzando una conexión TCP).
    async fn handle_reask_file_ping(&self, peer_addr: SocketAddr, file_hash: [u8; 16]) {
        let hash_hex = hex::encode(file_hash);

        let shared = {
            let downloads = self.downloads.read().unwrap_or_else(|e| e.into_inner()).clone();
            match downloads.as_ref() {
                Some(d) => {
                    let d = d.lock().await;
                    d.get_shared_file(&hash_hex).unwrap_or(None).is_some()
                }
                None => false,
            }
        };

        let mgr = self.upload_manager.read().unwrap_or_else(|e| e.into_inner()).clone();
        // `GetWaitingClientByIP_UDP` + `md4equ(reqfilehash,
        // sender->GetUploadFileID())` de eMule: el rango solo se revela al cliente
        // efectivamente en cola **para este archivo** — un par en cola para
        // otro archivo (o varios clientes tras la misma IP sin
        // correspondencia de puerto UDP) no recibe ninguna respuesta.
        let lookup = match &mgr {
            Some(m) => {
                m.waiting_rank_for_udp(&peer_addr.ip(), peer_addr.port(), &hash_hex)
                    .await
            }
            None => crate::services::upload_queue::UdpRankLookup::NoSender,
        };

        use crate::services::upload_queue::UdpRankLookup;
        let response = if !shared {
            client_udp::encode_file_not_found()
        } else {
            match lookup {
                UdpRankLookup::Rank(rank) => {
                    // `sender->SetLastUpRequest()` de eMule en ping aceptado.
                    if let Some(m) = &mgr {
                        m.note_udp_reask(&peer_addr.ip()).await;
                    }
                    let include_status = {
                        let inner = self.inner.lock().await;
                        inner
                            .client_udp_versions
                            .get(&peer_addr.ip())
                            .copied()
                            .unwrap_or(0)
                            > 3
                    };
                    client_udp::encode_reask_ack(rank as u16, include_status)
                }
                UdpRankLookup::NoSender => {
                    let nearly_full = match &mgr {
                        Some(m) => m.is_nearly_full().await,
                        None => false,
                    };
                    if nearly_full {
                        client_udp::encode_queue_full()
                    } else {
                        // eMule: "No responderle. Forzarle a establecer
                        // una conexión TCP."
                        return;
                    }
                }
                UdpRankLookup::Ambiguous => {
                    tracing::debug!(
                        "Client UDP: REASKFILEPING from {peer} dropped — multiple queued clients share this IP (bSenderMultipleIpUnknown)",
                        peer = log_peer_addr(peer_addr)
                    );
                    return;
                }
                UdpRankLookup::FileMismatch => {
                    tracing::debug!(
                        "Client UDP: REASKFILEPING from {peer} dropped — reqfile does not match queued file",
                        peer = log_peer_addr(peer_addr)
                    );
                    return;
                }
            }
        };

        let socket = { self.inner.lock().await.socket.clone() };
        if let Some(socket) = socket {
            if let Err(e) = socket.send_to(&response, peer_addr).await {
                tracing::debug!(
                    "Client UDP: reask reply to {peer} failed: {e}",
                    peer = log_peer_addr(peer_addr)
                );
            }
        }
    }

    /// `case OP_REASKCALLBACKUDP` de eMule — cuando el datagrama va dirigido al ID de
    /// nuestro cliente buddy, reenviarlo en el enlace TCP del buddy como
    /// `OP_REASKCALLBACKTCP`: `[requester_ip:4][requester_udp:2][file_hash:16]
    /// [part_status..]` (eMule inserta ip/puerto en payload+10 y luego copia desde
    /// allí — se reemplazan los bytes 10..16 del ID del buddy).
    async fn handle_reask_callback(
        &self,
        peer_addr: SocketAddr,
        buddy_id: [u8; 16],
        rest: Vec<u8>,
    ) {
        let reg = self.buddy.read().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(reg) = reg else { return };
        let ip4 = match peer_addr.ip() {
            IpAddr::V4(v4) => v4.octets(),
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => v4.octets(),
                None => return,
            },
        };
        let mut payload = Vec::with_capacity(6 + rest.len());
        payload.extend_from_slice(&ip4);
        payload.extend_from_slice(&peer_addr.port().to_le_bytes());
        payload.extend_from_slice(&rest);
        let mut frame = Vec::with_capacity(6 + payload.len());
        frame.push(PR_EMULE);
        frame.extend_from_slice(&(payload.len() as u32 + 1).to_le_bytes());
        frame.push(mule_protocol::ed2k::OP_REASKCALLBACKTCP);
        frame.extend_from_slice(&payload);
        let relayed = {
            let reg = reg.lock().await;
            if reg
                .active
                .as_ref()
                .map(|b| b.buddy_id == buddy_id)
                .unwrap_or(false)
            {
                reg.send_to_buddy(frame)
            } else {
                false
            }
        };
        if relayed {
            tracing::info!(
                "Client UDP: relayed OP_REASKCALLBACKUDP from {peer} to buddy link",
                peer = log_peer_addr(peer_addr)
            );
        } else {
            tracing::debug!(
                "Client UDP: dropping OP_REASKCALLBACKUDP from {peer} — buddy_id mismatch or no buddy"
            , peer = log_peer_addr(peer_addr));
        }
    }

    /// `case OP_DIRECTCALLBACKREQ` de eMule — un par que no puede alcanzarnos
    /// por TCP nos pide (como nodo con cortafuegos Kad) que conectemos hacia él.
    /// Atendido únicamente mientras nuestro estado de cortafuegos UDP Kad sea `Firewalled`
    /// (`CKademlia::IsRunning() && IsFirewalled()`), con límite de tasa a una
    /// solicitud aceptada por IP por `DIRECT_CALLBACK_MIN_INTERVAL`
    /// (`AllowCalbackRequest`, 180 s).
    async fn handle_direct_callback_req(
        &self,
        peer_addr: SocketAddr,
        tcp_port: u16,
        user_hash: [u8; 16],
        conn_opts: u8,
    ) {
        // Límite de tasa — eMule descarta solicitudes repetidas dentro de la ventana.
        {
            let mut inner = self.inner.lock().await;
            let now = Instant::now();
            inner
                .direct_callbacks
                .retain(|_, at| now.duration_since(*at) < DIRECT_CALLBACK_MIN_INTERVAL);
            if inner.direct_callbacks.contains_key(&peer_addr.ip()) {
                tracing::debug!(
                    "Client UDP: DIRECTCALLBACKREQ from {peer} dropped — too many requests",
                    peer = log_peer_addr(peer_addr)
                );
                return;
            }
        }

        // Compuerta — eMule solo acepta cuando `CKademlia::IsRunning() &&
        // CKademlia::IsFirewalled()` (`ClientUDPSocket.cpp:373`).
        // `CKademlia::IsFirewalled()` (Kademlia.cpp:324 →
        // `CPrefs::GetFirewalled`) es el veredicto Kad-TCP — los contactos
        // confirmaron que nuestro puerto TCP era inalcanzable — NO el comprobador de cortafuegos UDP:
        // el mecanismo está destinado a nodos TCP inactivo / UDP activo (NAT full-cone),
        // por lo que el estado UDP no debe condicionar la recepción. eMule asume con cortafuegos
        // por defecto hasta que dos contactos nos reporten abiertos; nosotros no ejecutamos
        // la comprobación saliente Kad-TCP, por lo que `!is_high_id` (LowID asignado por el servidor)
        // es nuestro sustituto del lado TCP.
        let kad_running = { self.inner.lock().await.socket.is_some() };
        let server_session = { self.server_session.read().unwrap_or_else(|e| e.into_inner()).clone() };
        let (our_id, is_high_id) = match server_session {
            Some(s) => {
                let snap = s.snapshot().await;
                (snap.client_id.unwrap_or(0), snap.is_high_id)
            }
            None => (0, false),
        };
        if !kad_running || is_high_id {
            tracing::debug!(
                "Client UDP: DIRECTCALLBACKREQ from {peer} ignored — not TCP-firewalled (kad_running={kad_running}, high_id={is_high_id})",
                peer = log_peer_addr(peer_addr)
            );
            return;
        }

        let Some(inbound) = self.inbound.read().unwrap_or_else(|e| e.into_inner()).clone() else {
            return;
        };

        // `AddTrackCallbackRequests` de eMule — registrado solo una vez aceptado.
        self.inner
            .lock()
            .await
            .direct_callbacks
            .insert(peer_addr.ip(), Instant::now());

        let target = SocketAddr::new(peer_addr.ip(), tcp_port);
        tracing::info!(
            "Client UDP: DIRECTCALLBACKREQ accepted from {peer} — connect-back to {target} (conn_opts=0x{conn_opts:02X})",
            peer = log_peer_addr(peer_addr),
            target = log_peer_addr(target)
        );
        tokio::spawn(async move {
            // server_ip/port: desconocidos fuera de sesión de servidor — `0` es el
            // respaldo utilizado por `sx_local_ids` cuando no se está
            // conectado a un servidor.
            if let Err(e) = inbound
                .connect_back(target, Some(user_hash), conn_opts, our_id, 0, 0)
                .await
            {
                tracing::debug!(
                    "Client UDP: direct callback connect-back to {target} failed: {e}",
                    target = log_peer_addr(target)
                );
            }
        });
    }

    /// Valida un ack contra las reconsultas pendientes y aplica el rango a la
    /// sesión de par en cola (`sender->UDPReaskACK(nRank)` de eMule).
    async fn handle_reask_ack(
        &self,
        peer_addr: SocketAddr,
        result: UdpReaskResult,
        part_status: Option<client_udp::UdpPartStatus>,
    ) {
        // Hash del archivo para el cual el ping estaba en vuelo — eMule lo
        // localiza mediante `sender->m_reqfile`; un solo ping en vuelo por par
        // (`m_bUDPPending`), por lo que retiramos todas las entradas de la IP y
        // conservamos el hash para los procesamientos de FNF / estado de partes.
        let (had_pending, pending_hash) = {
            let mut inner = self.inner.lock().await;
            // Ack esperado = ping todavía en su ventana `PENDING_TTL` —
            // `m_bUDPPending` de eMule (restablecido a falso por `UDPReaskACK`).
            let key_exists = inner
                .awaiting_ack
                .iter()
                .any(|((ip, _), at)| *ip == peer_addr.ip() && at.elapsed() < PENDING_TTL);
            let mut hash = None;
            if key_exists {
                inner.awaiting_ack.retain(|(ip, h), _| {
                    if *ip == peer_addr.ip() {
                        hash = Some(*h);
                        false
                    } else {
                        true
                    }
                });
                inner
                    .results
                    .insert(peer_addr.ip(), (result.clone(), Instant::now()));
            }
            (key_exists, hash)
        };

        if !had_pending {
            tracing::debug!(
                "Client UDP: ignoring unsolicited {:?} from {peer} (pendingflag == false)",
                result,
                peer = log_peer_addr(peer_addr)
            );
            return;
        }

        tracing::debug!(
            "Client UDP: {:?} from {peer}",
            result,
            peer = log_peer_addr(peer_addr)
        );

        let ps = self.peer_session.read().unwrap_or_else(|e| e.into_inner()).clone();
        match result {
            UdpReaskResult::Rank(rank) => {
                // `SetRemoteQueueFull(false)` + `UDPReaskACK(nRank)` de eMule.
                if let Some(ps) = ps {
                    ps.apply_udp_queue_rank(&peer_addr.ip(), rank as u32).await;
                }
                // UDP ver > 3: `ProcessFileStatus` — el mapa de bits describe
                // las partes que el par posee del archivo solicitado; alimenta
                // la rareza (`m_SrcPartFrequency`).
                if let (Some(status), Some(hash)) = (part_status, pending_hash) {
                    self.apply_ack_part_status(peer_addr, hash, status).await;
                }
            }
            UdpReaskResult::QueueFull => {
                // `SetRemoteQueueFull(true)` + `UDPReaskACK(0)` de eMule.
                if let Some(ps) = ps {
                    ps.apply_udp_queue_full(&peer_addr.ip()).await;
                }
            }
            UdpReaskResult::FileNotFound => {
                // `UDPReaskFNF` de eMule: la fuente es marcada en la lista muerta para
                // este archivo y retirada de las fuentes.
                if let Some(ps) = ps {
                    ps.apply_udp_file_not_found(&peer_addr.ip()).await;
                }
                if let Some(hash) = pending_hash {
                    self.mark_source_file_not_found(peer_addr, hash).await;
                }
            }
        }
    }

    /// `ProcessFileStatus(true, data_in, reqfile)` de eMule: aplica el
    /// mapa de bits de partes anunciado por el par a su disponibilidad para el
    /// archivo solicitado (rareza `m_SrcPartFrequency` + estado persistido).
    async fn apply_ack_part_status(
        &self,
        peer_addr: SocketAddr,
        file_hash: [u8; 16],
        status: client_udp::UdpPartStatus,
    ) {
        let Some(dl_arc) = self.downloads.read().unwrap_or_else(|e| e.into_inner()).clone() else {
            return;
        };
        let mask = match mule_protocol::ed2k::PartAvailabilityMask::from_bytes(
            bytes::Bytes::from(status.bitfield.clone()),
            status.part_count,
        ) {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(
                    "Client UDP: malformed REASKACK part status from {peer}: {e:?}",
                    peer = log_peer_addr(peer_addr)
                );
                return;
            }
        };
        let hash_hex = hex::encode(file_hash);
        let mut dl = dl_arc.lock().await;
        let Some(dl_id) = dl.find_by_ed2k_hash(&hash_hex).map(|s| s.id) else {
            return;
        };
        // La sesión TCP del par (el puerto UDP del datagrama ≠ el puerto TCP
        // de la conexión): localizamos su `ActiveSource` por IP.
        if let Some(src_addr) = dl
            .get_sources(&dl_id)
            .and_then(|srcs| srcs.keys().find(|a| a.ip() == peer_addr.ip()).copied())
        {
            let _ = dl.record_source_availability(&dl_id, src_addr, mask);
        }
        if let Some(src) = dl
            .list_peer_sources(&hash_hex)
            .unwrap_or_default()
            .into_iter()
            .find(|s| {
                s.address
                    .rsplit_once(':')
                    .and_then(|(h, _)| h.parse::<IpAddr>().ok())
                    .is_some_and(|ip| ip == peer_addr.ip())
            })
        {
            let _ = dl.update_peer_source_details(
                &hash_hex,
                &src.address,
                None,
                Some(status.bitfield.as_slice()),
            );
        }
    }

    /// `UDPReaskFNF` de eMule — `m_DeadSourceList.AddDeadSource`: el par ha
    /// respondido `OP_FILENOTFOUND` para el archivo solicitado; la fuente queda
    /// marcada como muerta (estado `Failed`, fuera de la lista de fuentes — lista muerta de eMule).
    async fn mark_source_file_not_found(&self, peer_addr: SocketAddr, file_hash: [u8; 16]) {
        let Some(dl_arc) = self.downloads.read().unwrap_or_else(|e| e.into_inner()).clone() else {
            return;
        };
        let hash_hex = hex::encode(file_hash);
        let mut dl = dl_arc.lock().await;
        let src = dl
            .list_peer_sources(&hash_hex)
            .unwrap_or_default()
            .into_iter()
            .find(|s| {
                s.address
                    .rsplit_once(':')
                    .and_then(|(h, _)| h.parse::<IpAddr>().ok())
                    .is_some_and(|ip| ip == peer_addr.ip())
            });
        if let Some(src) = src {
            let _ = dl.update_peer_source_status(
                &hash_hex,
                &src.address,
                mule_core::model::PeerSourceStatus::Failed,
                Some("peer answered OP_FILENOTFOUND (UDP deadlist)"),
                None,
            );
            tracing::info!(
                "Client UDP: source {} marked dead for {hash_hex} (OP_FILENOTFOUND)",
                src.address
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn target(ip: IpAddr, file_hash: [u8; 16]) -> UdpReaskTarget {
        UdpReaskTarget {
            addr: SocketAddr::new(ip, 12345),
            file_hash,
            connected_at_unix_ms: Some(0),
            next_tcp_reask_unix_ms: None,
        }
    }

    async fn bound_service() -> ClientUdpService {
        let svc = ClientUdpService::new();
        let sock = UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .expect("bind loopback UDP socket");
        svc.set_socket(Arc::new(sock)).await;
        svc
    }

    /// Envejece las entradas internas como si el ping anterior no hubiera
    /// recibido respuesta jamás: `pending` más allá de `UDP_REASK_INTERVAL`
    /// (la limitación permite el reintento) y `awaiting_ack` más allá de
    /// `PENDING_TTL` (contabilizado como fallo en el próximo `send`).
    async fn age_out(svc: &ClientUdpService) {
        let mut inner = svc.inner.lock().await;
        for at in inner.pending.values_mut() {
            *at = Instant::now() - (UDP_REASK_INTERVAL + Duration::from_secs(60));
        }
        for at in inner.awaiting_ack.values_mut() {
            *at = Instant::now() - (PENDING_TTL + Duration::from_secs(60));
        }
    }

    /// `UDPReaskForDownload` de eMule: tras >3 paquetes y >30 % de fallos,
    /// el par ya no es solicitado por UDP (`DownloadClient.cpp:1340`).
    #[tokio::test]
    async fn udp_failure_ratio_guard_stops_reasks() {
        let svc = bound_service().await;
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let t = target(ip, [0xAA; 16]);

        // 4 pings que permanecieron sin respuesta → 100 % de fallos.
        for _ in 0..4 {
            assert!(svc.send_reask_file_ping(&t, 0).await);
            age_out(&svc).await;
        }
        let stats = svc
            .inner
            .lock()
            .await
            .udp_stats
            .get(&ip)
            .cloned()
            .unwrap_or_default();
        assert_eq!(stats.total, 4);
        assert_eq!(stats.failed, 3); // le 4e échec est compté au send suivant

        // 5.º intento: el 4.º ping expira → failed=4, total=4>3, ratio 100 %
        // > 30 % → la protección bloquea el envío.
        assert!(!svc.send_reask_file_ping(&t, 0).await);
    }

    /// Proporción ≤ 30 %: las reconsultas UDP continúan (1 fallo de 4 = 25 %).
    #[tokio::test]
    async fn udp_reasks_continue_below_failure_threshold() {
        let svc = bound_service().await;
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        let t = target(ip, [0xBB; 16]);
        let peer_addr = t.addr;

        // 4 pings: 3 acks + 1 silencio.
        for round in 0..4 {
            assert!(svc.send_reask_file_ping(&t, 0).await);
            if round < 3 {
                svc.handle_reask_ack(peer_addr, UdpReaskResult::Rank(7), None)
                    .await;
            }
            age_out(&svc).await;
        }

        // total=4, failed=1 → 25 % ≤ 30 %: la siguiente reconsulta está permitida.
        assert!(svc.send_reask_file_ping(&t, 0).await);
    }

    /// `m_bUDPPending`: mientras un ack pueda todavía llegar, ningún nuevo
    /// ping sale hacia este par (incluso para otro archivo).
    #[tokio::test]
    async fn udp_pending_blocks_second_ping_same_peer() {
        let svc = bound_service().await;
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 3));
        let t1 = target(ip, [0xCC; 16]);
        let t2 = target(ip, [0xDD; 16]);

        assert!(svc.send_reask_file_ping(&t1, 0).await);
        assert!(!svc.send_reask_file_ping(&t2, 0).await);
    }

    /// Un ack no solicitado (ningún ping en espera) es ignorado.
    #[tokio::test]
    async fn udp_unsolicited_ack_ignored() {
        let svc = bound_service().await;
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 4));
        svc.handle_reask_ack(SocketAddr::new(ip, 12345), UdpReaskResult::Rank(3), None)
            .await;
        assert!(svc.last_result(ip).await.is_none());
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use crate::services::peer_session::log_server_addr;
use std::net::SocketAddr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{error, info, warn};

use mule_core::model::{PeerSource, PeerSourceOrigin, PeerSourceStatus};
use mule_network_policy::SocketPermit;
use mule_protocol::ed2k::{
    decode_server_response, Ed2kLimits, ServerLoginRequest, ServerResponse, PR_ED2K,
    PR_EMULE_COMPRESSED,
};
use mule_protocol::framing::{PacketCodec, RawPacket};

use super::events::{emit_event, emit_found_sources, emit_found_sources_obfu, emit_search_results};
use super::types::{ServerConnectionStatus, ServerSessionSnapshot};
use crate::services::downloads::DownloadService;

const MAX_SERVER_READ_BUFFER_SIZE: usize = 256 * 1024;

pub struct ActiveSession {
    pub snapshot: ServerSessionSnapshot,
    pub abort_handle: Option<tokio::task::AbortHandle>,
    pub permit: Option<SocketPermit>,
    pub outgoing_tx: Option<mpsc::Sender<RawPacket>>,
    /// Dirección del servidor → (obf_tcp_port, obf_udp_port) obtenido de
    /// las etiquetas `ST_TCPPORTOBFUSCATION`/`ST_UDPPORTOBFUSCATION` de `OP_SERVERIDENT`.
    /// Persiste entre reconexiones (refleja el almacenamiento en server.met de eMule).
    pub obf_ports: std::collections::HashMap<String, (u16, u16)>,
    /// Direcciones donde el apretón de manos ofuscado ya falló en este proceso
    /// (`SetTriedCrypt` de eMule) — las conexiones posteriores van en texto claro.
    pub obf_failed: std::collections::HashSet<String>,
}

impl Default for ActiveSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ActiveSession {
    pub fn new() -> Self {
        Self {
            snapshot: ServerSessionSnapshot::default(),
            abort_handle: None,
            permit: None,
            outgoing_tx: None,
            obf_ports: std::collections::HashMap::new(),
            obf_failed: std::collections::HashSet::new(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_connection_task(
    target_addr: SocketAddr,
    cancel_token: mule_network_policy::CancellationToken,
    connect_timeout_ms: u64,
    handshake_timeout_ms: u64,
    client_port: u16,
    nickname: String,
    user_hash: [u8; 16],
    mut outgoing_rx: mpsc::Receiver<RawPacket>,
    inner_clone: Arc<Mutex<ActiveSession>>,
    event_tx_clone: broadcast::Sender<String>,
    sequence_clone: Arc<AtomicU64>,
    downloads_clone: Option<Arc<Mutex<DownloadService>>>,
    inbound_clone: Option<crate::services::inbound_listener::InboundListenerService>,
    kad_clone: Option<Arc<crate::services::kad::KadService>>,
    prefer_obfuscation: bool,
    obfuscation_padding_max: u8,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let start_connect = std::time::Instant::now();

        // eMule CServerSocket::ConnectTo: si el servidor anunció un puerto de
        // ofuscación (ST_TCPPORTOBFUSCATION en un SERVERIDENT previo) y preferimos
        // la capa criptográfica, conectamos a ese puerto con el apretón de manos
        // DH768+RC4; en caso de fallo se marca como probado y se reintenta en texto claro.
        let address_str = target_addr.to_string();
        // eMule: bEncrypt = cryptPreferred && (SupportsObfuscationTCP() || !TriedCrypt())
        // — la ofuscación se intenta al menos una vez por servidor incluso sin un
        // puerto anunciado; al primer fallo el servidor se marca como probado y las
        // conexiones posteriores van en texto claro.
        let try_obf = {
            let g = inner_clone.lock().await;
            prefer_obfuscation && !g.obf_failed.contains(&address_str)
        };
        let obf_dial_port = {
            if try_obf {
                let g = inner_clone.lock().await;
                g.obf_ports
                    .get(&address_str)
                    .map(|(tcp, _)| *tcp)
                    .filter(|p| *p != 0)
            } else {
                None
            }
        };

        let dial_addr = match obf_dial_port {
            Some(p) => SocketAddr::new(target_addr.ip(), p),
            None => target_addr,
        };
        if try_obf {
            info!(
                "Connecting to eD2K server {} via protocol obfuscation{}",
                log_server_addr(dial_addr),
                if obf_dial_port.is_some() {
                    " (dedicated port)"
                } else {
                    " (standard port)"
                }
            );
        }
        info!(
            "Attempting TCP connection to eD2K server {} (timeout: {} ms)...",
            log_server_addr(dial_addr),
            connect_timeout_ms
        );

        // 1. Conexión TCP con connect_timeout
        let stream_res = tokio::select! {
            _ = cancel_token.cancelled() => {
                warn!("Server connection to {} cancelled by policy kill switch during connect", log_server_addr(dial_addr));
                Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Cancelled by network policy"))
            }
            res = tokio::time::timeout(Duration::from_millis(connect_timeout_ms), TcpStream::connect(dial_addr)) => {
                match res {
                    Ok(Ok(stream)) => {
                        let _ = stream.set_nodelay(true);
                        info!("TCP socket connected successfully to eD2K server {} in {} ms", log_server_addr(dial_addr), start_connect.elapsed().as_millis());
                        Ok(stream)
                    }
                    Ok(Err(e)) => {
                        warn!("TCP connect failed to eD2K server {}: {}", log_server_addr(dial_addr), e);
                        Err(e)
                    }
                    Err(_) => {
                        warn!("TCP connect timed out after {} ms to eD2K server {}", connect_timeout_ms, log_server_addr(dial_addr));
                        Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "TCP connect timed out"))
                    }
                }
            }
        };

        let mut stream: crate::services::obfuscation_stream::ObfuscatedStream<TcpStream> =
            match stream_res {
                Ok(s) => {
                    if try_obf {
                        match tokio::time::timeout(
                            Duration::from_millis(handshake_timeout_ms),
                            crate::services::obfuscation_stream::ObfuscatedStream::connect_server_outgoing(
                                s,
                                obfuscation_padding_max,
                            ),
                        )
                        .await
                        {
                            Ok(Ok(obf)) => {
                                info!("Obfuscated (DH768+RC4) server handshake completed with {server}", server = log_server_addr(dial_addr));
                                obf
                            }
                            res => {
                                let why = match res {
                                    Ok(Err(e)) => e.to_string(),
                                    Err(_) => "handshake timeout".to_string(),
                                    _ => unreachable!(),
                                };
                                warn!(
                                    "Obfuscated server handshake with {server} failed ({why}) — falling back to plaintext {dial}"
                                , server = log_server_addr(target_addr), dial = log_server_addr(dial_addr));
                                let mut g = inner_clone.lock().await;
                                g.obf_failed.insert(address_str.clone());
                                drop(g);
                                match tokio::time::timeout(
                                    Duration::from_millis(connect_timeout_ms),
                                    TcpStream::connect(target_addr),
                                )
                                .await
                                {
                                    Ok(Ok(plain)) => crate::services::obfuscation_stream::ObfuscatedStream::plaintext(plain),
                                    _ => {
                                        warn!("Plaintext fallback connect to eD2K server {server} failed", server = log_server_addr(target_addr));
                                        let mut g = inner_clone.lock().await;
                                        g.snapshot = ServerSessionSnapshot::default();
                                        g.permit = None;
                                        g.abort_handle = None;
                                        g.outgoing_tx = None;
                                        emit_event(&event_tx_clone, &sequence_clone, &g.snapshot);
                                        return;
                                    }
                                }
                            }
                        }
                    } else {
                        crate::services::obfuscation_stream::ObfuscatedStream::plaintext(s)
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to establish TCP connection to eD2K server {}: {}",
                        log_server_addr(dial_addr),
                        e
                    );
                    let mut g = inner_clone.lock().await;
                    g.snapshot = ServerSessionSnapshot::default();
                    g.permit = None;
                    g.abort_handle = None;
                    g.outgoing_tx = None;
                    emit_event(&event_tx_clone, &sequence_clone, &g.snapshot);
                    return;
                }
            };

        let ping_ms = start_connect.elapsed().as_millis() as u32;

        // 2. Envío de OP_LOGINREQUEST
        let limits = Ed2kLimits::for_server();
        // eMule `ServerConnect.cpp:184-191`: cuando se prefiere la ofuscación,
        // el inicio de sesión anuncia `SUPPORTCRYPT | REQUESTCRYPT` en
        // `CT_SERVER_FLAGS` (el puerto dedicado `ST_TCPPORTOBFUSCATION` sigue
        // siendo el mecanismo principal en el lado Rust).
        let login_req = ServerLoginRequest::new(user_hash, client_port, &nickname, 0x3c)
            .with_server_flags(if prefer_obfuscation {
                mule_protocol::ed2k::tags::SRVCAP_SUPPORTCRYPT
                    | mule_protocol::ed2k::tags::SRVCAP_REQUESTCRYPT
            } else {
                0
            });
        info!(
            "Sending OP_LOGINREQUEST to eD2K server {} (nick='{}', port={}, hash={:?})",
            log_server_addr(target_addr),
            nickname,
            client_port,
            hex::encode(user_hash)
        );
        let packet = match login_req.encode(&limits) {
            Ok(p) => p,
            Err(e) => {
                error!(
                    "Failed to encode ServerLoginRequest for {}: {}",
                    log_server_addr(target_addr),
                    e
                );
                let mut g = inner_clone.lock().await;
                g.snapshot = ServerSessionSnapshot::default();
                g.permit = None;
                g.abort_handle = None;
                g.outgoing_tx = None;
                emit_event(&event_tx_clone, &sequence_clone, &g.snapshot);
                return;
            }
        };

        let mut wire_buf = Vec::with_capacity(6 + packet.payload.len());
        wire_buf.push(packet.protocol);
        wire_buf.extend_from_slice(&(packet.frame_len() as u32).to_le_bytes());
        wire_buf.push(packet.opcode);
        wire_buf.extend_from_slice(&packet.payload);

        if let Err(e) = stream.write_all(&wire_buf).await {
            warn!(
                "Failed to write OP_LOGINREQUEST bytes to eD2K server {}: {}",
                log_server_addr(target_addr),
                e
            );
            let mut g = inner_clone.lock().await;
            g.snapshot = ServerSessionSnapshot::default();
            g.permit = None;
            g.abort_handle = None;
            g.outgoing_tx = None;
            emit_event(&event_tx_clone, &sequence_clone, &g.snapshot);
            return;
        }
        info!(
            "Sent OP_LOGINREQUEST ({} wire bytes) to {}. Awaiting server response...",
            wire_buf.len(),
            log_server_addr(target_addr)
        );

        // 3. Bucle de lectura con tiempo de espera de apretón de manos y PacketCodec
        let mut read_buf = BytesMut::with_capacity(8192);
        let codec = PacketCodec::new();
        let mut temp_buf = [0u8; 4096];
        let handshake_deadline =
            tokio::time::Instant::now() + Duration::from_millis(handshake_timeout_ms);
        let mut is_connecting = true;
        let mut publish_login: Option<mule_protocol::ed2k::ServerIdChange> = None;
        // eMule `CServer::GetSoftFiles` (etiqueta `ST_SOFTFILES` 0x88 de
        // OP_SERVERIDENT) — limita cuántos archivos podemos ofrecer; eMule
        // lo acota a ≤200 (`CSharedFileList::SendListToServer`).
        let mut soft_files_limit: Option<u32> = None;
        // Credenciales del último OP_IDCHANGE, conservadas para la republicación
        // periódica de OP_OFFERFILES (eMule mantiene actualizada la lista de archivos
        // compartidos en el servidor y utiliza un paquete de oferta vacío como keep-alive).
        let mut last_idc: Option<mule_protocol::ed2k::ServerIdChange> = None;
        let republish_period = Duration::from_secs(30 * 60);
        let mut republish_timer = tokio::time::interval_at(
            tokio::time::Instant::now() + republish_period,
            republish_period,
        );

        loop {
            tokio::select! {
                            _ = cancel_token.cancelled() => {
                                info!("Server session aborted by policy kill switch");
                                break;
                            }
                            Some(out_packet) = outgoing_rx.recv() => {
                                let mut out_wire = Vec::with_capacity(6 + out_packet.payload.len());
                                out_wire.push(out_packet.protocol);
                                out_wire.extend_from_slice(&(out_packet.frame_len() as u32).to_le_bytes());
                                out_wire.push(out_packet.opcode);
                                out_wire.extend_from_slice(&out_packet.payload);
                                if let Err(e) = stream.write_all(&out_wire).await {
                                    warn!("Failed to write outgoing packet to server {}: {}", log_server_addr(target_addr), e);
                                    break;
                                }
                            }
                            read_res = stream.read(&mut temp_buf) => {
                                match read_res {
                                    Ok(0) => {
                                        info!("Server closed TCP connection cleanly: {}", log_server_addr(target_addr));
                                        break;
                                    }
                                    Ok(n) => {
                                        read_buf.extend_from_slice(&temp_buf[..n]);
                                        if read_buf.len() > MAX_SERVER_READ_BUFFER_SIZE {
                                            warn!(
                                                "Server {} exceeded maximum read buffer size ({}), disconnecting",
                                                log_server_addr(target_addr), MAX_SERVER_READ_BUFFER_SIZE
                                            );
                                            break;
                                        }

                                        loop {
                                            match codec.decode(&mut read_buf) {
                                                Ok(Some(raw_packet)) => {
                                                    tracing::debug!(
                                                        "Server TCP packet received: proto=0x{:02X}, opcode=0x{:02X}, payload_len={}",
                                                        raw_packet.protocol,
                                                        raw_packet.opcode,
                                                        raw_packet.payload.len()
                                                    );
                                                    // eMule OP_PACKEDPROT (0xD4): la carga útil está comprimida con zlib;
                                                    // descomprimir con el límite de 250 KB de servidor de eMule y luego
                                                    // despachar según el opcode como un paquete PR_ED2K normal.
                                                    let (resp_protocol, resp_payload);
                                                    if raw_packet.protocol == PR_EMULE_COMPRESSED {
                                                        match unpack_packed_packet(&raw_packet.payload) {
                                                            Some(p) => {
                                                                resp_protocol = PR_ED2K;
                                                                resp_payload = p;
                                                            }
                                                            None => {
                                                                warn!("Server {} sent an undecodable packed packet (opcode=0x{:02X})", log_server_addr(target_addr), raw_packet.opcode);
                                                                continue;
                                                            }
                                                        }
                                                    } else {
                                                        resp_protocol = raw_packet.protocol;
                                                        resp_payload = raw_packet.payload.to_vec();
                                                    }
                                                    match decode_server_response(resp_protocol, raw_packet.opcode, &resp_payload, &limits) {
                                                        Ok(resp) => {
                                                            let mut g = inner_clone.lock().await;
                                                            let mut updated = false;

                                                            match resp {
                                                                ServerResponse::Message(msg) => {
                                                                    info!("Received eD2K server message: {}", msg.message);
                                                                    let prev = g.snapshot.welcome_message.clone().unwrap_or_default();
                                                                    let combined = if prev.is_empty() {
                                                                        msg.message
                                                                    } else {
                                                                        format!("{}\n{}", prev, msg.message)
                                                                    };
                                                                    g.snapshot.welcome_message = Some(combined);
                                                                    updated = true;
                                                                }
                                                                ServerResponse::IdChange(idc) => {
                                                                    info!("Assigned ClientID: {} (HighID: {})", idc.client_id, idc.is_high_id());
                                                                    is_connecting = false;
                                                                    g.snapshot.client_id = Some(idc.client_id);
                                                                    g.snapshot.is_high_id = idc.is_high_id();
                                                                    // Una HighID codifica nuestra IPv4 pública en little-endian (convención eD2K:
                                                                    // A.B.C.D = A | B<<8 | C<<16 | D<<24) — Kad la utiliza como dirección
                                                                    // anunciada en los contactos PUBLISH_SOURCE.
                                                                    if let Some(kad) = &kad_clone {
                                                                        kad.set_public_ipv4(if idc.is_high_id() {
                                                                            Some(std::net::Ipv4Addr::from(idc.client_id.to_le_bytes()))
                                                                        } else {
                                                                            None
                                                                        });
                                                                    }
                                                                    g.snapshot.status = ServerConnectionStatus::Connected;
                                                                    if g.snapshot.connected_at_unix_ms.is_none() {
                                                                        let now = std::time::SystemTime::now()
                                                                            .duration_since(std::time::UNIX_EPOCH)
                                                                            .unwrap_or_default()
                                                                            .as_millis() as i64;
                                                                        g.snapshot.connected_at_unix_ms = Some(now);
                                                                        g.snapshot.ping_ms = Some(ping_ms);
                                                                    }
                                                                    updated = true;
                                                                    publish_login = Some(idc);
                                                                }
                                                                ServerResponse::Status(status) => {
                                                                    info!("Server status: {} users, {} files", status.current_users, status.current_files);
                                                                    g.snapshot.current_users = Some(status.current_users);
                                                                    g.snapshot.current_files = Some(status.current_files);
                                                                    updated = true;
                                                                }
                                                                ServerResponse::Ident(ident) => {
                                                                    info!("Server ident: ip={:08x}, port={}", ident.ip, ident.port);
                                                                    if let Some(name) = ident.name() {
                                                                        g.snapshot.server_name = Some(name);
                                                                    }
                                                                    if let Some(sf) = ident.soft_files() {
                                                                        soft_files_limit = Some(sf);
                                                                    }
                                                                    let obf_tcp = ident.obfuscation_tcp_port();
                                                                    let obf_udp = ident.obfuscation_udp_port();
                                                                    if obf_tcp.is_some() || obf_udp.is_some() {
                                                                        info!(
                                                                            "Server {} advertises obfuscation ports: TCP={:?} UDP={:?}",
                                                                            log_server_addr(target_addr), obf_tcp, obf_udp
                                                                        );
                                                                        g.obf_ports.insert(
                                                                            target_addr.to_string(),
                                                                            (
                                                                                obf_tcp.unwrap_or(0),
                                                                                obf_udp.unwrap_or(0),
                                                                            ),
                                                                        );
                                                                    }
                                                                    updated = true;
                                                                }
                                                                ServerResponse::SearchResult(sr) => {
                                                                    info!("Received {} search results from server", sr.files.len());
                                                                    let cancelled = super::events::search_result_cancelled_flags(&downloads_clone, &sr).await;
                                                                    emit_search_results(&event_tx_clone, &sequence_clone, &sr, &cancelled);
                                                                }
                                                                ServerResponse::FoundSources(fs) => {
                                                                    let total_rcv = fs.sources.len();
                                                                    emit_found_sources(&event_tx_clone, &sequence_clone, &fs);

                                                                    if let Some(ref dl_service) = downloads_clone {
                                                                        let file_hash_hex = hex::encode(fs.file_hash);
                                                                        let now_ms = std::time::SystemTime::now()
                                                                            .duration_since(std::time::UNIX_EPOCH)
                                                                            .unwrap_or_default()
                                                                            .as_millis() as i64;
                                                                        let mut discovered = Vec::new();
                                                                        let mut seen_addrs = std::collections::HashSet::new();
                                                                        let mut low_id_ignored = 0;
                                                                        let mut rejected_invalid = 0;

                                                                        for src in &fs.sources {
                                                                            if src.ip == 0 {
                                                                                rejected_invalid += 1;
                                                                                continue;
                                                                            }

                                                                            // Autocomprobación HighID: omitir nuestro propio anuncio repercutido por el servidor
                                                                            if let Some(our_id) = g.snapshot.client_id {
                                                                                if our_id >= 16_777_216 && src.ip == our_id && src.port == client_port {
                                                                                    continue;
                                                                                }
                                                                            }

                                                                            // Fuente LowID: conservarla — accesible solo mediante
                                                                            // devolución de llamada del servidor (OP_CALLBACKREQUEST).
                                                                            let addr_str = if src.ip < 16_777_216 {
                                                                                low_id_ignored += 1;
                                                                                format!("lowid:{}", src.ip)
                                                                            } else {
                                                                                if src.port == 0 {
                                                                                    rejected_invalid += 1;
                                                                                    continue;
                                                                                }
                                                                                let ip_addr = std::net::Ipv4Addr::from(src.ip.to_le_bytes());
                                                                                if ip_addr.is_broadcast() || ip_addr.is_multicast() {
                                                                                    rejected_invalid += 1;
                                                                                    continue;
                                                                                }
                                                                                src.to_address_string()
                                                                            };
                                                                            if !seen_addrs.insert(addr_str.clone()) {
                                                                                continue;
                                                                            }

                                                                            // Las fuentes respondidas por el servidor conectado
                                                                            // residen en él (eMule registra el `nServerIP`/`nServerPort`
                                                                            // del servidor que responde).
                                                                            let (srv_ip, srv_port) = match target_addr.ip() {
                                                                                std::net::IpAddr::V4(v4) => {
                                                                                    (u32::from_le_bytes(v4.octets()), target_addr.port())
                                                                                }
                                                                                _ => (0u32, 0u16),
                                                                            };
                                                                            discovered.push(PeerSource {
                                                                                file_hash: file_hash_hex.clone(),
                                                                                address: addr_str,
                                                                                ip: src.ip,
                                                                                port: src.port,
                                                                                origin: PeerSourceOrigin::Server,
                                                                                discovered_at_unix_ms: now_ms,
                                                                                last_seen_unix_ms: now_ms,
                                                                                status: PeerSourceStatus::Candidate,
                                                                                fail_count: 0,
                                                                                last_error: None,
                                                                                cooldown_until_unix_ms: None,
                                                                                user_hash: None,
                                                                                crypt_options: 0,
                                                                                buddy_id: None,
            software: None,
                                                                                availability: None,
                                                                                server_ip: srv_ip,
                                                                                server_port: srv_port,
                                                                                udp_port: 0,
                                                                            });
                                                                        }

                                                                        info!(
                                                                            "FoundSources for {}: received={}, accepted={}, low_id_ignored={}, deduplicated={}, rejected_invalid={}",
                                                                            file_hash_hex,
                                                                            total_rcv,
                                                                            discovered.len(),
                                                                            low_id_ignored,
                                                                            total_rcv.saturating_sub(discovered.len() + low_id_ignored + rejected_invalid),
                                                                            rejected_invalid
                                                                        );

                                                                        if !discovered.is_empty() {
                                                                            let mut dl_guard = dl_service.lock().await;
                                                                            let _ = dl_guard.upsert_peer_sources(&file_hash_hex, &discovered);
                                                                        }
                                                                    }
                                                                }
                                                                ServerResponse::FoundSourcesObfu(fs) => {
                                                                    let total_rcv = fs.sources.len();
                                                                    emit_found_sources_obfu(&event_tx_clone, &sequence_clone, &fs);
                                                                    if let Some(ref dl_service) = downloads_clone {
                                                                        let file_hash_hex = hex::encode(fs.file_hash);
                                                                        let now_ms = std::time::SystemTime::now()
                                                                            .duration_since(std::time::UNIX_EPOCH)
                                                                            .unwrap_or_default()
                                                                            .as_millis() as i64;
                                                                        let mut discovered = Vec::new();
                                                                        let mut seen_addrs = std::collections::HashSet::new();
                                                                        let mut low_id_ignored = 0;
                                                                        let mut rejected_invalid = 0;

                                                                        for src in &fs.sources {
                                                                            if src.ip == 0 {
                                                                                rejected_invalid += 1;
                                                                                continue;
                                                                            }

                                                                            // Autocomprobación HighID: omitir nuestro propio anuncio repercutido por el servidor
                                                                            if let Some(our_id) = g.snapshot.client_id {
                                                                                if our_id >= 16_777_216 && src.ip == our_id && src.port == client_port {
                                                                                    continue;
                                                                                }
                                                                            }

                                                                            // Fuente LowID: conservarla — accesible solo mediante
                                                                            // devolución de llamada del servidor (OP_CALLBACKREQUEST).
                                                                            let addr_str = if src.ip < 16_777_216 {
                                                                                low_id_ignored += 1;
                                                                                format!("lowid:{}", src.ip)
                                                                            } else {
                                                                                if src.port == 0 {
                                                                                    rejected_invalid += 1;
                                                                                    continue;
                                                                                }
                                                                                let ip_addr = std::net::Ipv4Addr::from(src.ip.to_le_bytes());
                                                                                if ip_addr.is_broadcast() || ip_addr.is_multicast() {
                                                                                    rejected_invalid += 1;
                                                                                    continue;
                                                                                }
                                                                                src.to_address_string()
                                                                            };
                                                                            if !seen_addrs.insert(addr_str.clone()) {
                                                                                continue;
                                                                            }

                                                                            // Las fuentes respondidas por el servidor conectado
                                                                            // residen en él (eMule registra `nServerIP`/`nServerPort`).
                                                                            let (srv_ip, srv_port) = match target_addr.ip() {
                                                                                std::net::IpAddr::V4(v4) => {
                                                                                    (u32::from_le_bytes(v4.octets()), target_addr.port())
                                                                                }
                                                                                _ => (0u32, 0u16),
                                                                            };
                                                                            discovered.push(PeerSource {
                                                                                file_hash: file_hash_hex.clone(),
                                                                                address: addr_str,
                                                                                ip: src.ip,
                                                                                port: src.port,
                                                                                origin: PeerSourceOrigin::Server,
                                                                                discovered_at_unix_ms: now_ms,
                                                                                last_seen_unix_ms: now_ms,
                                                                                status: PeerSourceStatus::Candidate,
                                                                                fail_count: 0,
                                                                                last_error: None,
                                                                                cooldown_until_unix_ms: None,
                                                                                user_hash: src.user_hash.map(hex::encode),
            software: None,
                                                                                availability: None,
                                                                                crypt_options: src.obf_settings,
                                                                                buddy_id: None,
                                                                                server_ip: srv_ip,
                                                                                server_port: srv_port,
                                                                                udp_port: 0,
                                                                            });
                                                                        }

                                                                        info!(
                                                                            "FoundSourcesObfu for {}: received={}, accepted={}, low_id_ignored={}, deduplicated={}, rejected_invalid={}",
                                                                            file_hash_hex,
                                                                            total_rcv,
                                                                            discovered.len(),
                                                                            low_id_ignored,
                                                                            total_rcv.saturating_sub(discovered.len() + low_id_ignored + rejected_invalid),
                                                                            rejected_invalid
                                                                        );

                                                                        if !discovered.is_empty() {
                                                                            let mut dl_guard = dl_service.lock().await;
                                                                            let _ = dl_guard.upsert_peer_sources(&file_hash_hex, &discovered);
                                                                        }
                                                                    }
                                                                }
                                                                ServerResponse::CallbackRequested(cb) => {
                                                                    // Un cliente solicitó al servidor que nos retransmitiera
                                                                    // una devolución de llamada (somos el destino LowID). eMule
                                                                    // hace `TryToConnect` de vuelta al solicitante; la conexión
                                                                    // de retorno se ejecuta como sesión de servicio de subida.
                                                                    let ip_addr = std::net::Ipv4Addr::from(cb.ip.to_le_bytes());
                                                                    info!(
                                                                        "OP_CALLBACKREQUESTED: peer {}:{} wants us to connect back (crypt_opts={:?}, user_hash={})",
                                                                        ip_addr,
                                                                        cb.port,
                                                                        cb.crypt_options,
                                                                        cb.user_hash.map(hex::encode).as_deref().unwrap_or("<none>")
                                                                    );
                                                                    if let Some(inbound) = inbound_clone.clone() {
                                                                        let target = SocketAddr::new(
                                                                            std::net::IpAddr::V4(ip_addr),
                                                                            cb.port,
                                                                        );
                                                                        let uh = cb.user_hash;
                                                                        let crypt = cb.crypt_options.unwrap_or(0);
                                                                        let our_id = g.snapshot.client_id.unwrap_or(0);
                                                                        let srv_addr = g
                                                                            .snapshot
                                                                            .server_address
                                                                            .as_deref()
                                                                            .and_then(|a| a.parse::<SocketAddr>().ok());
                                                                        let (srv_ip, srv_port) = srv_addr
                                                                            .map(|a| match a.ip() {
                                                                                std::net::IpAddr::V4(v4) => {
                                                                                    (u32::from_le_bytes(v4.octets()), a.port())
                                                                                }
                                                                                _ => (0, a.port()),
                                                                            })
                                                                            .unwrap_or((0, 0));
                                                                        tokio::spawn(async move {
                                                                            if let Err(e) = inbound
                                                                                .connect_back(
                                                                                    target, uh, crypt, our_id,
                                                                                    srv_ip, srv_port,
                                                                                )
                                                                                .await
                                                                            {
                                                                                warn!(
                                                                                    "Callback connect-back to {server} failed: {e}"
                                                                                , server = log_server_addr(target));
                                                                            }
                                                                        });
                                                                    }
                                                                }
                                                                ServerResponse::ServerList { servers } => {
                            // OP_SERVERLIST: fusionar los servidores anunciados en la BD local
                            // con baja preferencia (eMule los añade como entradas normales de
                            // server.met; se omiten las direcciones filtradas por IP).
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as i64;
                            let entries: Vec<mule_core::model::Ed2kServer> = servers
                                .iter()
                                .filter(|(ip, port)| *port != 0 && *ip != 0)
                                .map(|&(ip, port)| {
                                    let ip_be = std::net::Ipv4Addr::from(ip.to_be_bytes());
                                    mule_core::model::Ed2kServer {
                                        address: format!("{ip_be}:{port}"),
                                        ip: ip_be.to_string(),
                                        port,
                                        name: String::new(),
                                        description: "via OP_SERVERLIST".to_string(),
                                        ping: 0,
                                        preference: 0,
                                        failed_count: 0,
                                        dyn_ip: None,
                                        version: None,
                                        filter_status: mule_core::model::ServerFilterStatus::Allowed,
                                        last_seen_unix_ms: Some(now_ms),
                                        created_at_unix_ms: now_ms,
                                        updated_at_unix_ms: now_ms,
                                    }
                                })
                                .collect();
                            if !entries.is_empty() {
                                if let Some(ref dl_service) = downloads_clone {
                                    let mut dl_guard = dl_service.lock().await;
                                    match dl_guard.upsert_servers(&entries) {
                                        Ok(n) => tracing::info!(
                                            "Merged {n} server(s) advertised by OP_SERVERLIST into server list"
                                        ),
                                        Err(e) => {
                                            tracing::warn!("Failed to persist OP_SERVERLIST servers: {e}")
                                        }
                                    }
                                }
                            }
                        }
                        ServerResponse::CallbackFail { client_id } => {
                                                                    // Los servidores eMule envían OP_CALLBACK_FAIL
                                                                    // con carga útil vacía — el client_id de destino
                                                                    // no se devuelve en eco.
                                                                    match client_id {
                                                                        Some(id) => warn!(
                                                                            "OP_CALLBACK_FAIL: server could not relay callback to LowID client {id}"
                                                                        ),
                                                                        None => warn!(
                                                                            "OP_CALLBACK_FAIL: server could not relay a pending callback (target offline or unreachable)"
                                                                        ),
                                                                    }
                                                                }
                                                                ServerResponse::Rejected => {
                                                                    // eMule `ServerSocket.cpp:572` —
                                                                    // OP_REJECT solo se registra; el servidor
                                                                    // cierra el socket él mismo.
                                                                    warn!("Server rejected last command (OP_REJECT)");
                                                                }
                                                                ServerResponse::Other { opcode, .. } => {
                                                                    info!("Received other server opcode: 0x{:02x}", opcode);
                                                                }
                                                            }

                                                            if updated {
                                                                emit_event(&event_tx_clone, &sequence_clone, &g.snapshot);
                                                            }
                                                            drop(g);

                                                            // eMule `CSharedFileList::SendListToServer`:
                                                            // una vez que el servidor nos asigna ID, publicar
                                                            // la lista de archivos compartidos mediante OP_OFFERFILES.
                                                            if let Some(idc) = publish_login.take() {
                                                                last_idc = Some(idc.clone());
                                                                publish_shared_files(
                                                                    &mut stream,
                                                                    target_addr,
                                                                    idc.client_id,
                                                                    idc.tcp_flags,
                                                                    client_port,
                                                                    &limits,
                                                                    &downloads_clone,
                                                                    soft_files_limit,
                                                                )
                                                                .await;
                                                            }
                                                        }
                                                        Err(e) => {
                                                            warn!("Error decoding server packet: {}", e);
                                                        }
                                                    }
                                                }
                                                Ok(None) => break, // Se necesitan más bytes
                                                Err(e) => {
                                                    error!("Framing error from server {}: {}", log_server_addr(target_addr), e);
                                                    let mut g = inner_clone.lock().await;
                                                    g.snapshot = ServerSessionSnapshot::default();
                                                    g.outgoing_tx = None;
                                                    emit_event(&event_tx_clone, &sequence_clone, &g.snapshot);
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Error reading from server socket {}: {}", log_server_addr(target_addr), e);
                                        break;
                                    }
                                }
                            }
                            _ = republish_timer.tick(), if !is_connecting && last_idc.is_some() => {
                                if let Some(ref idc) = last_idc {
                                    publish_shared_files(
                                        &mut stream,
                                        target_addr,
                                        idc.client_id,
                                        idc.tcp_flags,
                                        client_port,
                                        &limits,
                                        &downloads_clone,
                                        soft_files_limit,
                                    )
                                    .await;
                                }
                            }
                            _ = tokio::time::sleep_until(handshake_deadline), if is_connecting => {
                                warn!("Server handshake timed out after {} ms", handshake_timeout_ms);
                                break;
                            }
                        }
        }

        // Desmontaje
        let mut g = inner_clone.lock().await;
        g.snapshot = ServerSessionSnapshot::default();
        g.permit = None;
        g.abort_handle = None;
        g.outgoing_tx = None;
        emit_event(&event_tx_clone, &sequence_clone, &g.snapshot);
    })
}

/// Publica la lista de archivos compartidos al servidor conectado mediante `OP_OFFERFILES`
/// (eMule `CSharedFileList::SendListToServer`, limitada a 200 archivos).
async fn publish_shared_files(
    stream: &mut crate::services::obfuscation_stream::ObfuscatedStream<TcpStream>,
    target_addr: SocketAddr,
    client_id: u32,
    tcp_flags: u32,
    client_port: u16,
    limits: &Ed2kLimits,
    downloads: &Option<Arc<Mutex<DownloadService>>>,
    soft_files_limit: Option<u32>,
) {
    /// Límite estricto de eMule sobre los archivos ofrecidos cuando el servidor no anuncia
    /// un límite `ST_SOFTFILES` menor (`CSharedFileList::SendListToServer`).
    const MAX_OFFER_FILES: usize = 200;

    let Some(dl_service) = downloads else { return };
    let mut shared = {
        let dl_svc = dl_service.lock().await;
        dl_svc.list_all_shared_files().unwrap_or_default()
    };
    // eMule `SendListToServer`: los archivos se ofrecen ordenados por prioridad de subida
    // (los más altos primero), limitados por el tope flexible del servidor (≤200).
    shared.sort_by_key(|sf| {
        std::cmp::Reverse(crate::services::upload_queue::file_prio_num(
            &sf.up_priority,
        ))
    });

    let offer_limit = soft_files_limit
        .map(|s| (s as usize).min(MAX_OFFER_FILES))
        .unwrap_or(MAX_OFFER_FILES);
    let mut files = Vec::with_capacity(shared.len().min(offer_limit));
    for sf in shared.iter().filter(|sf| sf.enabled) {
        if files.len() >= offer_limit {
            break;
        }
        let Ok(hash_bytes) = <[u8; 16]>::try_from(
            hex::decode(sf.file_hash.trim())
                .unwrap_or_default()
                .as_slice(),
        ) else {
            continue;
        };
        if let Some(of) = mule_protocol::ed2k::OfferedFile::for_server(
            hash_bytes,
            &sf.file_name,
            sf.file_size,
            client_id,
            client_port,
            tcp_flags,
            sf.is_partial,
        ) {
            files.push(of);
        }
    }

    // Un OP_OFFERFILES vacío actúa a la vez como paquete keep-alive hacia el servidor
    // (semántica de `KeepConnectionAlive` de eMule).
    let count = files.len();
    let offer = mule_protocol::ed2k::OfferFiles { files };
    match offer.encode(limits) {
        Ok(pkt) => {
            // eMule `Packet::PackPacket`: cuando el servidor anuncia
            // `SRV_TCPFLG_COMPRESSION`, la oferta se comprime con zlib al
            // nivel máximo y se envía bajo `PR_EMULE_COMPRESSED (0xD4)` —
            // únicamente si el resultado es más pequeño.
            let (protocol, payload): (u8, Vec<u8>) = {
                let compressed = if tcp_flags & mule_protocol::ed2k::SRV_TCPFLG_COMPRESSION != 0 {
                    pack_packet_payload(&pkt.payload)
                } else {
                    None
                };
                match compressed {
                    Some(buf) => (mule_protocol::ed2k::PR_EMULE_COMPRESSED, buf),
                    None => (pkt.protocol, pkt.payload.to_vec()),
                }
            };
            let mut out = Vec::with_capacity(6 + payload.len());
            out.push(protocol);
            out.extend_from_slice(&((payload.len() as u32 + 1).to_le_bytes()));
            out.push(pkt.opcode);
            out.extend_from_slice(&payload);
            match stream.write_all(&out).await {
                Ok(()) => info!(
                    "Published {} shared file(s) to server {} via OP_OFFERFILES",
                    count,
                    log_server_addr(target_addr)
                ),
                Err(e) => warn!(
                    "Failed to send OP_OFFERFILES to {}: {}",
                    log_server_addr(target_addr),
                    e
                ),
            }
        }
        Err(e) => warn!(
            "Failed to encode OP_OFFERFILES for {}: {}",
            log_server_addr(target_addr),
            e
        ),
    }
}

/// eMule `Packet::PackPacket`: comprime con zlib la carga útil al nivel máximo
/// (`Z_BEST_COMPRESSION`); devuelve `Some(buf)` únicamente si el resultado es
/// estrictamente más pequeño que el original (de lo contrario el paquete se envía sin comprimir).
fn pack_packet_payload(payload: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    if encoder.write_all(payload).is_err() {
        return None;
    }
    encoder
        .finish()
        .ok()
        .filter(|buf| buf.len() < payload.len())
}

/// eMule `Packet::UnPackPacket` para paquetes de servidor: descomprime con zlib la carga útil
/// empaquetada con un límite estricto de tamaño descomprimido de 250 KB (eMule pasa 250000).
/// Devuelve `None` en datos truncados, con tamaño excesivo o corruptos.
fn unpack_packed_packet(payload: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    const MAX_DECOMPRESSED: u64 = 250_000;
    let decoder = flate2::read::ZlibDecoder::new(payload).take(MAX_DECOMPRESSED + 1);
    let mut out = Vec::new();
    let mut decoder = decoder;
    decoder.read_to_end(&mut out).ok()?;
    if out.len() as u64 > MAX_DECOMPRESSED {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::unpack_packed_packet;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn unpack_packed_packet_roundtrip() {
        let payload = b"search-result-payload".repeat(64);
        assert_eq!(unpack_packed_packet(&zlib(&payload)).unwrap(), payload);
    }

    #[test]
    fn unpack_packed_packet_rejects_corrupt() {
        assert!(unpack_packed_packet(b"not zlib at all").is_none());
        let mut truncated = zlib(b"hello world");
        truncated.truncate(truncated.len() / 2);
        assert!(unpack_packed_packet(&truncated).is_none());
    }

    #[test]
    fn unpack_packed_packet_enforces_250kb_cap() {
        // ~300 KB de ceros se comprimen a unos pocos cientos de bytes — patrón de bomba de descompresión.
        let big = vec![0u8; 300_000];
        assert!(unpack_packed_packet(&zlib(&big)).is_none());
        // Justo por debajo del límite sigue pasando.
        let ok = vec![0u8; 250_000];
        assert_eq!(unpack_packed_packet(&zlib(&ok)).unwrap().len(), 250_000);
    }
}

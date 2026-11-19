// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use crate::services::peer_session::log_peer_addr;
use bytes::BytesMut;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, Mutex};

use mule_core::model::PeerFailureReason;
use mule_protocol::ed2k::{
    Ed2kDecoder, Ed2kEncoder, Ed2kHello, OP_EMULE_INFO, OP_EMULE_INFO_ANSWER, OP_HELLO,
    OP_HELLO_ANSWER, PR_ED2K, PR_EMULE,
};
use mule_protocol::framing::PacketCodec;

use super::events::emit_event;
use super::types::{
    PeerConnectionStatus, PeerError, PeerTimeouts, SwarmInnerState, MAX_PEER_READ_BUFFER_SIZE,
};
use crate::services::downloads::DownloadService;
use crate::services::hello_caps::{
    build_hello_tags, local_pc26_caps, HelloBuildParams, EMULE_VERSION_SHORT,
};
use crate::services::obfuscation_stream::ObfuscatedStream;

pub struct HandshakeResult {
    pub stream: ObfuscatedStream<TcpStream>,
    pub peer_supports_sx: bool,
    pub peer_supports_sx2: bool,
    /// `m_bySourceExchange1Ver` del par — versión SX1 anunciada en
    /// los bits 12-15 de CT_EMULE_MISCOPTIONS1 o mediante la etiqueta `ET_SOURCEEXCHANGE`
    /// de `OP_EMULEINFO` (0 = sin SX1 utilizable).
    pub peer_sx1_version: u8,
    pub peer_supports_aich: bool,
    /// `m_bySupportSecIdent` del par (0 = ninguno, bit0 = v1, bit1 = v2).
    pub peer_sec_ident: u8,
    /// Puerto UDP de eMule anunciado por el par (palabra baja de `CT_EMULE_UDPPORTS`), si existe.
    pub peer_udp_port: Option<u16>,
    /// Versión del protocolo UDP de cliente del par (`m_byUDPVer`, bits 24-27 de MiscOptions1).
    pub peer_udp_version: u8,
    /// Versión de peticiones extendidas del par (`m_byExtendedRequestsVer`,
    /// bits 8-11 de MiscOptions1). 2 = acepta `OP_REQUESTPARTS_I64`.
    pub peer_ext_req: u8,
    /// Versión de Kad anunciada por el par (bits 0-4 de MiscOptions2).
    pub peer_kad_version: u8,
    /// Envoltorio multipaquete negociado por el par
    /// (`SupportMultiPacket`/`SupportExtMultiPacket`/`SupportsFileIdentifiers`)
    /// — da forma a los lotes de `SendFileRequest` (`file_request`).
    pub peer_multipacket: super::source_exchange::MultiPacketLevel,
    /// `SupportsFileIdentifiers()` — bit 13 de MISCOPTIONS2. Condiciona
    /// `OP_HASHSETREQUEST2` en lugar de `OP_HASHSETREQUEST`
    /// (`SendHashSetRequest`, `DownloadClient.cpp:2200`).
    pub peer_file_identifiers: bool,
    /// Capacidades modernas `PC26` anunciadas por el par en `OP_HELLO_ANSWER`.
    pub peer_pc26_caps: Option<mule_protocol::ed2k::pc26::Pc26PeerCaps>,
    /// RTT inicial medido al conectar con el par (ms).
    pub initial_rtt_ms: Option<u32>,
    pub connected_ms: i64,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn perform_peer_handshake(
    target_addr: SocketAddr,
    file_hash_str: &str,
    address_str: &str,
    client_port: u16,
    nickname: &str,
    user_hash: [u8; 16],
    timeouts: &PeerTimeouts,
    cancel_token: &mule_network_policy::CancellationToken,
    inner: &Arc<Mutex<SwarmInnerState>>,
    event_tx: &broadcast::Sender<String>,
    sequence: &Arc<AtomicU64>,
    client_id: u32,
    server_ip: u32,
    server_port: u16,
    kad_port: u16,
    kad_service: &Option<Arc<crate::services::KadService>>,
    target_user_hash: Option<[u8; 16]>,
    peer_crypt_options: u8,
    obfuscation_enabled: bool,
    obfuscation_required: bool,
    obfuscation_padding_max: u8,
    downloads: &Arc<Mutex<DownloadService>>,
    sec_ident_cap: u8,
    identity: &Option<Arc<Mutex<crate::services::IdentityService>>>,
) -> Result<HandshakeResult, PeerError> {
    let stored_modern = {
        let dl_svc = downloads.lock().await;
        dl_svc.get_peer_source_modern_endpoints(file_hash_str, address_str)
    };
    let mut candidate_addrs = vec![target_addr];
    let mut peer_known_pc26 = false;
    if let Some((ipv6_opt, quic_port)) = stored_modern {
        if quic_port > 0 {
            peer_known_pc26 = true;
        }
        if let Some(ipv6_str) = ipv6_opt {
            if let Ok(v6_addr) = ipv6_str.parse::<SocketAddr>() {
                if v6_addr != target_addr {
                    candidate_addrs.push(v6_addr);
                }
            }
        }
    }

    // 1. Conexión TCP Happy Eyeballs v2 con tiempo de espera y cancel_token
    let tcp_connect_start = std::time::Instant::now();
    let (tcp_stream, connected_addr) = super::happy_eyeballs::connect_happy_eyeballs(
        &candidate_addrs,
        timeouts.connect_timeout,
        cancel_token,
    )
    .await?;
    let initial_rtt_ms = Some(
        tcp_connect_start
            .elapsed()
            .as_millis()
            .clamp(1, u128::from(u32::MAX)) as u32,
    );
    let _ = tcp_stream.set_nodelay(true);
    let connected_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    // Decisión de ofuscación: si nuestra política exige cifrado sin `user_hash` conocido o el
    // par es `PC26` conocido, iniciamos transporte autenticado oportunista `PC26-AEAD`.
    let peer_requires_crypt = (peer_crypt_options & 0x04) != 0;
    let peer_supports_crypt = (peer_crypt_options & 0x01) != 0 || (peer_crypt_options & 0x02) != 0;

    let obfu_hash = target_user_hash.filter(|_| {
        obfuscation_required || peer_requires_crypt || (obfuscation_enabled && peer_supports_crypt)
    });

    let pc26_attempt = (obfuscation_enabled && peer_known_pc26)
        || (obfuscation_required && target_user_hash.is_none());

    let mut stream = if pc26_attempt {
        match ObfuscatedStream::connect_peer_pc26_outgoing(
            tcp_stream,
            mule_crypto::pc26_handshake::Pc26Suite::HybridMlKem768ChaCha20Poly1305,
        )
        .await
        {
            Ok(s) => s,
            Err(e) if obfuscation_required && target_user_hash.is_none() => {
                return Err(PeerError::new(
                    PeerFailureReason::ObfuscationNegotiationFailed,
                    format!("PC26 obfuscation negotiation failed: {e}"),
                ));
            }
            Err(e) => {
                tracing::warn!(
                    "PC26 handshake to peer {peer} failed ({e}), falling back to legacy mode",
                    peer = log_peer_addr(connected_addr)
                );
                let (fallback_tcp, _) = super::happy_eyeballs::connect_happy_eyeballs(
                    &candidate_addrs,
                    timeouts.connect_timeout,
                    cancel_token,
                )
                .await?;
                let _ = fallback_tcp.set_nodelay(true);
                if let Some(th) = obfu_hash {
                    match ObfuscatedStream::connect_peer_outgoing(
                        fallback_tcp,
                        &th,
                        obfuscation_padding_max,
                    )
                    .await
                    {
                        Ok(s) => s,
                        Err(_) if !obfuscation_required => {
                            let (plain_stream, _) = super::happy_eyeballs::connect_happy_eyeballs(
                                &candidate_addrs,
                                timeouts.connect_timeout,
                                cancel_token,
                            )
                            .await?;
                            let _ = plain_stream.set_nodelay(true);
                            ObfuscatedStream::plaintext(plain_stream)
                        }
                        Err(rc4_err) => {
                            return Err(PeerError::new(
                                PeerFailureReason::ObfuscationNegotiationFailed,
                                format!("Protocol obfuscation negotiation failed: {rc4_err}"),
                            ));
                        }
                    }
                } else {
                    ObfuscatedStream::plaintext(fallback_tcp)
                }
            }
        }
    } else if let Some(th) = obfu_hash {
        tracing::info!(
            "Initiating obfuscated handshake to peer {peer} with target user hash {:02X?}",
            &th,
            peer = log_peer_addr(connected_addr)
        );
        match ObfuscatedStream::connect_peer_outgoing(tcp_stream, &th, obfuscation_padding_max)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                if !obfuscation_required {
                    tracing::warn!(
                        "Obfuscated handshake to peer {peer} failed ({e}), falling back to plaintext connection"
                    , peer = log_peer_addr(connected_addr));
                    match tokio::time::timeout(
                        timeouts.connect_timeout,
                        TcpStream::connect(connected_addr),
                    )
                    .await
                    {
                        Ok(Ok(plain_stream)) => {
                            let _ = plain_stream.set_nodelay(true);
                            ObfuscatedStream::plaintext(plain_stream)
                        }
                        Ok(Err(reconnect_err)) => {
                            if peer_requires_crypt {
                                return Err(PeerError::new(
                                    PeerFailureReason::ObfuscationNegotiationFailed,
                                    format!("Protocol obfuscation negotiation failed ({e}) and plaintext fallback failed: {reconnect_err}"),
                                ));
                            }
                            return Err(PeerError::new(
                                PeerFailureReason::ConnectRefused,
                                format!("Plaintext fallback connect failed: {reconnect_err}"),
                            ));
                        }
                        Err(_) => {
                            if peer_requires_crypt {
                                return Err(PeerError::new(
                                    PeerFailureReason::ObfuscationNegotiationFailed,
                                    format!("Protocol obfuscation negotiation failed ({e}) and plaintext fallback timed out"),
                                ));
                            }
                            return Err(PeerError::new(
                                PeerFailureReason::TimeoutConnect,
                                "Plaintext fallback connect timed out",
                            ));
                        }
                    }
                } else {
                    tracing::warn!(
                        "Obfuscated handshake to peer {peer} failed: {e}",
                        peer = log_peer_addr(connected_addr)
                    );
                    return Err(PeerError::new(
                        PeerFailureReason::ObfuscationNegotiationFailed,
                        format!("Protocol obfuscation negotiation failed: {e}"),
                    ));
                }
            }
        }
    } else {
        if peer_requires_crypt && target_user_hash.is_none() {
            tracing::debug!(
                "Peer {peer} advertises required crypt without user_hash — attempting plaintext fallback",
                peer = log_peer_addr(connected_addr)
            );
        }
        ObfuscatedStream::plaintext(tcp_stream)
    };

    // Actualizar estado: Handshaking
    let handshaking_snapshot = {
        let mut g = inner.lock().await;
        if let Some(session) = g.sessions.get_mut(address_str) {
            session.snapshot.status = PeerConnectionStatus::Handshaking;
            session.snapshot.connected_at_unix_ms = Some(connected_ms);
            session.snapshot.obfuscated = stream.is_obfuscated();
            Some(session.snapshot.clone())
        } else {
            None
        }
    };
    if let Some(snap) = handshaking_snapshot {
        emit_event(event_tx, sequence, &snap);
    }

    // 2. Enviar OP_HELLO
    let encoder = Ed2kEncoder::default();
    let udp_verified_open = match kad_service {
        Some(kad) => {
            let tracker = kad.udp().firewall_tracker();
            let fw = tracker.lock().await;
            fw.verified && fw.status == crate::services::kad_udp::UdpFirewallStatus::Open
        }
        None => false,
    };
    let ed25519_pubkey = match identity {
        Some(id) => id.lock().await.ed25519_public_key(),
        None => None,
    };
    let pc26 = local_pc26_caps(ed25519_pubkey, None, None, obfuscation_enabled);
    let tags = build_hello_tags(&HelloBuildParams {
        nickname,
        tcp_port: client_port,
        kad_port,
        sec_ident_cap,
        tcp_low_id: client_id < 0x0100_0000,
        udp_verified_open,
        obfuscation_enabled,
        obfuscation_required,
        pc26_caps: Some(&pc26),
    });

    let hello = Ed2kHello {
        user_hash,
        client_id,
        tcp_port: client_port,
        tags,
        unknown_tags: vec![],
        server_ip,
        server_port,
    };

    let hello_pkt = encoder.encode_hello(&hello).map_err(|e| {
        PeerError::new(
            PeerFailureReason::InvalidHandshake,
            format!("Failed to encode OP_HELLO: {e}"),
        )
    })?;

    let mut out_wire = Vec::with_capacity(6 + hello_pkt.payload.len());
    out_wire.push(hello_pkt.protocol);
    out_wire.extend_from_slice(&(hello_pkt.frame_len() as u32).to_le_bytes());
    out_wire.push(hello_pkt.opcode);
    out_wire.extend_from_slice(&hello_pkt.payload);

    tracing::info!(
        "Peer {peer}: sending OP_HELLO (client_id={client_id}, port={client_port}, server={server_ip}:{server_port}, len={}): {:02X?}",
        out_wire.len(),
        &out_wire[..]
    , peer = log_peer_addr(target_addr));

    if let Err(e) = stream.write_all(&out_wire).await {
        return Err(PeerError::new(
            PeerFailureReason::ConnectRefused,
            format!("Failed to send OP_HELLO: {e}"),
        ));
    }

    // 2b. Enviar OP_EMULEINFO justo después de OP_HELLO (SendHelloTypePacket
    // de eMule emite ambos en secuencia). Aquí es donde eMule/aMule negocian
    // capacidades extendidas: compresión, extremo UDP, intercambio de fuentes,
    // comentarios, peticiones extendidas y banderas de características.
    let emule_info = build_emule_info(kad_port, sec_ident_cap);
    match encoder.encode_emule_info(&emule_info) {
        Ok(info_pkt) => {
            let mut info_wire = Vec::with_capacity(6 + info_pkt.payload.len());
            info_wire.push(info_pkt.protocol);
            info_wire.extend_from_slice(&(info_pkt.frame_len() as u32).to_le_bytes());
            info_wire.push(info_pkt.opcode);
            info_wire.extend_from_slice(&info_pkt.payload);
            if let Err(e) = stream.write_all(&info_wire).await {
                return Err(PeerError::new(
                    PeerFailureReason::ConnectRefused,
                    format!("Failed to send OP_EMULEINFO: {e}"),
                ));
            }
        }
        Err(e) => {
            return Err(PeerError::new(
                PeerFailureReason::InvalidHandshake,
                format!("Failed to encode OP_EMULEINFO: {e}"),
            ));
        }
    }

    // 3. Recibir OP_HELLO_ANSWER con tiempo de espera de apretón de manos
    let decoder = Ed2kDecoder::default();
    let codec = PacketCodec::new();
    let mut read_buf = BytesMut::with_capacity(65536);
    let mut raw_buf = [0u8; 8192];
    let mut received_hello_answer = false;
    let mut peer_supports_sx = false;
    let mut peer_supports_sx2 = false;
    let mut peer_sx1_version: u8 = 0;
    let mut peer_supports_aich = false;
    let mut peer_sec_ident: u8 = 0;
    let mut peer_udp_port = None;
    let mut peer_udp_version: u8 = 0;
    let mut peer_ext_req: u8 = 0;
    let mut peer_kad_version: u8 = 0;
    let mut peer_supports_compression = false;
    let mut peer_multipacket = super::source_exchange::MultiPacketLevel::None;
    let mut peer_file_identifiers = false;
    let mut peer_pc26_caps: Option<mule_protocol::ed2k::pc26::Pc26PeerCaps> = None;

    let handshake_deadline = tokio::time::Instant::now() + timeouts.handshake_timeout;
    while !received_hello_answer {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                return Err(PeerError::new(PeerFailureReason::KillSwitchAborted, "Handshake aborted by kill switch"));
            }
            _ = tokio::time::sleep_until(handshake_deadline) => {
                return Err(PeerError::new(PeerFailureReason::TimeoutHandshake, "Handshake timed out waiting for OP_HELLO_ANSWER"));
            }
            read_res = stream.read(&mut raw_buf) => {
                match read_res {
                    Ok(0) => {
                        let buffered_len = read_buf.len();
                        let snippet = if buffered_len > 0 {
                            format!("{:02X?}", &read_buf[..buffered_len.min(32)])
                        } else {
                            "none".to_string()
                        };
                        tracing::warn!(
                            "Peer {peer} closed connection during handshake (bytes buffered={buffered_len}: {snippet})"
                        , peer = log_peer_addr(target_addr));
                        return Err(PeerError::new(PeerFailureReason::ConnectRefused, "Peer closed connection during handshake"));
                    }
                    Ok(n) => {
                        tracing::debug!(
                            "Peer {peer}: read {n} bytes during handshake: {:02X?}",
                            &raw_buf[..n.min(32)]
                        , peer = log_peer_addr(target_addr));
                        read_buf.extend_from_slice(&raw_buf[..n]);
                        if read_buf.len() > MAX_PEER_READ_BUFFER_SIZE {
                            return Err(PeerError::new(PeerFailureReason::InvalidHandshake, "Peer read buffer exceeded limit during handshake"));
                        }
                        while let Ok(Some(packet)) = codec.decode(&mut read_buf) {
                            // OP_PACKEDPROT (0xD4) — desempaquetado a nivel de socket,
                            // cf. Packets.cpp::UnPackPacket.
                            let Some(packet) = mule_protocol::ed2k::packed::unpack_owned(
                                packet,
                                mule_protocol::ed2k::packed::PACKED_PEER_MAX_DECOMPRESSED,
                            ) else {
                                continue;
                            };
                            tracing::debug!(
                                "Peer {peer}: decoded packet proto=0x{:02X} opcode=0x{:02X} payload_len={}",
                                packet.protocol,
                                packet.opcode,
                                packet.payload.len()
                            , peer = log_peer_addr(target_addr));
                            if packet.protocol == PR_EMULE
                                && (packet.opcode == OP_EMULE_INFO
                                    || packet.opcode == OP_EMULE_INFO_ANSWER)
                            {
                                match decoder.decode_emule_info(
                                    packet.protocol,
                                    packet.opcode,
                                    &packet.payload,
                                    0,
                                ) {
                                    Ok(info) => {
                                        let tag_desc: Vec<String> = info
                                            .tags
                                            .iter()
                                            .map(|t| t.name.display_name())
                                            .collect();
                                        tracing::info!(
                                            "Peer {peer}: {} received (client_ver=0x{:02X}, proto_ver=0x{:02X}, tags=[{}], unknown_tags={})",
                                            if packet.opcode == OP_EMULE_INFO {
                                                "OP_EMULEINFO"
                                            } else {
                                                "OP_EMULEINFO_ANSWER"
                                            },
                                            info.client_version,
                                            info.protocol_version,
                                            tag_desc.join(","),
                                            info.unknown_tags.len()
                                        , peer = log_peer_addr(target_addr));
                                        if info.has_source_exchange_capability() {
                                            peer_supports_sx = true;
                                            peer_sx1_version = peer_sx1_version.max(
                                                super::source_exchange::emule_info_sx_version(
                                                    &info,
                                                )
                                                .max(1),
                                            );
                                        }
                                        if info.has_compression_capability() {
                                            peer_supports_compression = true;
                                        }
                                        if let Some(et) = info
                                            .tags
                                            .iter()
                                            .find(|t| matches!(
                                                t.name,
                                                mule_protocol::ed2k::tags::TagName::Id(
                                                    mule_protocol::ed2k::tags::TAG_SPECIAL_EMULE_EXT_EXTENDEDREQUEST
                                                )
                                            ))
                                        {
                                            if let mule_protocol::ed2k::tags::TagValue::UInt32(v) =
                                                et.value
                                            {
                                                peer_ext_req = peer_ext_req.max(v as u8);
                                            }
                                        }
                                        // eMule responde a OP_EMULEINFO con
                                        // OP_EMULEINFOANSWER (SendMuleInfoPacket(true)).
                                        if packet.opcode == OP_EMULE_INFO {
                                            let answer = build_emule_info(kad_port, sec_ident_cap);
                                            if let Ok(ans_pkt) =
                                                encoder.encode_emule_info_answer(&answer)
                                            {
                                                let mut ans_wire =
                                                    Vec::with_capacity(6 + ans_pkt.payload.len());
                                                ans_wire.push(ans_pkt.protocol);
                                                ans_wire.extend_from_slice(
                                                    &(ans_pkt.frame_len() as u32).to_le_bytes(),
                                                );
                                                ans_wire.push(ans_pkt.opcode);
                                                ans_wire.extend_from_slice(&ans_pkt.payload);
                                                let _ = stream.write_all(&ans_wire).await;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::debug!(
                                            "Peer {peer}: malformed OP_EMULEINFO packet ignored: {e}"
                                        , peer = log_peer_addr(target_addr));
                                    }
                                }
                                continue;
                            }
                            if packet.protocol == PR_ED2K && (packet.opcode == OP_HELLO || packet.opcode == OP_HELLO_ANSWER) {
                                match decoder.decode_hello_answer(packet.protocol, packet.opcode, &packet.payload, 0) {
                                    Ok(ans) => {
                                        let peer_info = mule_protocol::ed2k::Ed2kPeerInfo::from_hello_answer(&ans);
                                        peer_supports_sx |= peer_info.supports_source_exchange();
                                        peer_supports_sx2 |= peer_info.supports_source_exchange2();
                                        peer_sx1_version = peer_sx1_version
                                            .max(peer_info.source_exchange1_version());
                                        peer_supports_aich = peer_info.supports_aich();
                                        peer_sec_ident = peer_info.secure_ident_support();
                                        peer_udp_port = peer_info.emule_udp_port;
                                        peer_udp_version = peer_info.udp_version();
                                        peer_ext_req = peer_info.extended_requests_version();
                                        peer_kad_version = peer_info.kad_version();
                                        peer_supports_compression |=
                                            peer_info.has_compression_capability();
                                        peer_multipacket = super::source_exchange::multipacket_level(
                                            &peer_info,
                                        );
                                        peer_file_identifiers =
                                            peer_info.supports_file_identifiers();
                                        peer_pc26_caps = peer_info.pc26_caps();

                                        let unknown_tag_ids: Vec<String> = peer_info
                                            .unknown_tags
                                            .iter()
                                            .map(|t| format!("{t:?}"))
                                            .collect();
                                        tracing::info!(
                                            "Peer {peer} handshake: software={} version={:?} emule_version={:?} client_id={} misc1={:?} misc2={:?} \
                                             caps: sx={} sx2={} aich={} sec_ident={} crypt(sup/req/reqd)={}/{}/{} udp_ver={} udp_port={:?} ext_req={} kad_ver={} large_files={} compression={} \
                                             unknown_tags=[{}]",
                                            peer_info.software_name(),
                                            peer_info.version,
                                            peer_info.emule_version_string(),
                                            peer_info.client_id,
                                            peer_info.misc_options1.map(|m| format!("0x{m:08X}")),
                                            peer_info.misc_options2.map(|m| format!("0x{m:08X}")),
                                            peer_supports_sx,
                                            peer_supports_sx2,
                                            peer_supports_aich,
                                            peer_sec_ident,
                                            peer_info.supports_crypt_layer(),
                                            peer_info.requests_crypt_layer(),
                                            peer_info.requires_crypt_layer(),
                                            peer_udp_version,
                                            peer_udp_port,
                                            peer_ext_req,
                                            peer_kad_version,
                                            peer_info.supports_large_files(),
                                            peer_supports_compression,
                                            unknown_tag_ids.join(",")
                                        , peer = log_peer_addr(target_addr));

                                        let mut peer_opts = 0u8;
                                        if peer_info.supports_crypt_layer() { peer_opts |= 0x01; }
                                        if peer_info.requests_crypt_layer() { peer_opts |= 0x02; }
                                        if peer_info.requires_crypt_layer() { peer_opts |= 0x04; }
                                        // Bit 12 de MISCOPTIONS2 (m_fDirectUDPCallback
                                        // de eMule) → bit 3 del byte de opciones de conexión,
                                        // como en SetConnectOptions(bCallback=true).
                                        if peer_info.supports_direct_udp_callback() { peer_opts |= 0x08; }

                                        let hex_user_hash = hex::encode(ans.user_hash);
                                        let software = format!(
                                            "{} {}",
                                            peer_info.software_name(),
                                            peer_info
                                                .emule_version_string()
                                                .unwrap_or_default()
                                        );
                                        let updated_src = mule_core::model::PeerSource {
                                            file_hash: file_hash_str.to_string(),
                                            address: address_str.to_string(),
                                            ip: match target_addr {
                                                SocketAddr::V4(v4) => u32::from_le_bytes(v4.ip().octets()),
                                                _ => 0,
                                             },
                                            port: target_addr.port(),
                                            origin: mule_core::model::PeerSourceOrigin::Manual,
                                            discovered_at_unix_ms: connected_ms,
                                            last_seen_unix_ms: connected_ms,
                                            status: mule_core::model::PeerSourceStatus::Candidate,
                                            fail_count: 0,
                                            last_error: None,
                                            cooldown_until_unix_ms: None,
                                            user_hash: Some(hex_user_hash),
                                            crypt_options: peer_opts,
                                            buddy_id: None,
                                            software: Some(software.trim().to_string()),
                                            availability: None,
                                            // Servidor del par aprendido mediante el
                                            // hello (m_dwServerIP de eMule).
                                            server_ip: ans.server_ip,
                                            server_port: ans.server_port,
                                            // Puerto UDP de Kad del par (etiqueta
                                            // CT_EMULE_UDPPORTS) — SetKadPort de eMule,
                                            // requerido para OP_DIRECTCALLBACKREQ.
                                            udp_port: peer_udp_port.unwrap_or(0),
                                        };
                                        {
                                            let mut dl_svc = downloads.lock().await;
                                            let _ = dl_svc.upsert_peer_sources(file_hash_str, &[updated_src]);
                                            if let Some(ref pc) = peer_pc26_caps {
                                                if pc.ipv6_endpoint.is_some() || pc.quic_port.is_some() {
                                                    let ipv6_str = pc.ipv6_endpoint.map(|(octets, port)| {
                                                        std::net::SocketAddrV6::new(
                                                            std::net::Ipv6Addr::from(octets),
                                                            port,
                                                            0,
                                                            0,
                                                        )
                                                        .to_string()
                                                    });
                                                    let _ = dl_svc.update_peer_source_modern_endpoints(
                                                        file_hash_str,
                                                        address_str,
                                                        ipv6_str.as_deref(),
                                                        pc.quic_port.unwrap_or(0),
                                                    );
                                                }
                                            }
                                        }

                                        for tag in &ans.tags {
                                            if let mule_protocol::ed2k::tags::TagName::Id(mule_protocol::ed2k::tags::TAG_SPECIAL_EMULE_UDPPORTS) = tag.name {
                                                if let mule_protocol::ed2k::tags::TagValue::UInt32(ports) = tag.value {
                                                    let peer_kad_port = (ports >> 16) as u16;
                                                    if peer_kad_port > 0 {
                                                        let low_id = ans.client_id < 16_777_216;
                                                        if let Some(ref kad) = kad_service {
                                                            let k_clone = Arc::clone(kad);
                                                            if let IpAddr::V4(ip4) = target_addr.ip() {
                                                                tokio::spawn(async move {
                                                                    k_clone.maybe_bootstrap_from_peer(ip4, peer_kad_port, low_id, peer_kad_version).await;
                                                                });
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        return Err(PeerError::new(PeerFailureReason::InvalidHandshake, format!("Invalid OP_HELLO_ANSWER: {e}")));
                                    }
                                }
                                received_hello_answer = true;
                                break;
                            }
                        }
                    }
                    Err(e) => return Err(PeerError::new(PeerFailureReason::ConnectRefused, format!("Read error during handshake: {e}"))),
                }
            }
        }
    }

    // Preservar paquetes posteriores recibidos en el mismo segmento TCP que OP_HELLO_ANSWER
    stream.put_back_unread(&read_buf);

    Ok(HandshakeResult {
        stream,
        peer_supports_sx,
        peer_supports_sx2,
        peer_sx1_version,
        peer_supports_aich,
        peer_sec_ident,
        peer_udp_port,
        peer_udp_version,
        peer_ext_req,
        peer_kad_version,
        peer_multipacket,
        peer_file_identifiers,
        peer_pc26_caps,
        initial_rtt_ms,
        connected_ms,
    })
}

/// Construye nuestra carga útil de `OP_EMULEINFO`/`OP_EMULEINFO_ANSWER` — el anuncio
/// de capacidades extendidas de eMule (7 etiquetas, el mismo conjunto que eMule 0.72a
/// en `SendMuleInfoPacket`): compresión, versión UDP, puerto UDP, intercambio de fuentes,
/// comentarios, peticiones extendidas y banderas de características.
pub(crate) fn build_emule_info(
    udp_port: u16,
    sec_ident_cap: u8,
) -> mule_protocol::ed2k::Ed2kEmuleInfo {
    use mule_protocol::ed2k::tags::{
        Ed2kTag, TagName, TagValue, TAG_SPECIAL_EMULE_EXT_COMMENTS,
        TAG_SPECIAL_EMULE_EXT_COMPRESSION, TAG_SPECIAL_EMULE_EXT_EXTENDEDREQUEST,
        TAG_SPECIAL_EMULE_EXT_FEATURES, TAG_SPECIAL_EMULE_EXT_SOURCEEXCHANGE,
        TAG_SPECIAL_EMULE_EXT_UDPPORT, TAG_SPECIAL_EMULE_EXT_UDPVER,
    };

    let t = |id: u8, v: u32| Ed2kTag {
        name: TagName::Id(id),
        value: TagValue::UInt32(v),
    };

    mule_protocol::ed2k::Ed2kEmuleInfo {
        client_version: EMULE_VERSION_SHORT,
        protocol_version: 0x01,
        tags: vec![
            t(TAG_SPECIAL_EMULE_EXT_COMPRESSION, 1),
            t(TAG_SPECIAL_EMULE_EXT_UDPVER, 4),
            t(TAG_SPECIAL_EMULE_EXT_UDPPORT, udp_port as u32),
            // eMule 0.72a envía 3 aquí (SX1 obsoleto) — la versión SX1 v4
            // continúa anunciándose en MiscOptions1 bits 12-15.
            t(TAG_SPECIAL_EMULE_EXT_SOURCEEXCHANGE, 3),
            t(TAG_SPECIAL_EMULE_EXT_COMMENTS, 1),
            t(TAG_SPECIAL_EMULE_EXT_EXTENDEDREQUEST, 2),
            // ET_FEATURES: bits 0-1 = nivel de secure ident (m_uEmuleFeatures de eMule).
            t(
                TAG_SPECIAL_EMULE_EXT_FEATURES,
                (sec_ident_cap & 0x03) as u32,
            ),
        ],
        unknown_tags: vec![],
    }
}

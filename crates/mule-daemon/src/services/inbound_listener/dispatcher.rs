// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Despacho del bucle de solicitudes, identificación segura y atención de subidas.

use crate::services::peer_session::log_addr_str;
use std::net::SocketAddr;

use bytes::{Bytes, BytesMut};
use md4::{Digest, Md4};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::warn;

use mule_crypto::legacy_sui::{
    build_sui_signing_payload, parse_peer_public_key, verify_sui_signature, SecureIdentVersion,
};

use mule_protocol::ed2k::limits::Ed2kLimits;
use mule_protocol::ed2k::{
    Ed2kAichAnswer, Ed2kAichFileHashRes, Ed2kAichPartHashSet, Ed2kAichRecovery, Ed2kDecoder,
    Ed2kEncoder,
    Ed2kFileIdentifier, Ed2kFileStatus, Ed2kHashSetAnswer, Ed2kHashSetAnswer2, Ed2kMessage,
    Ed2kMultiPacketAnswer, Ed2kMultiPacketSubAnswer, Ed2kMultiPacketSubRequest, Ed2kPublicKey,
    Ed2kRequestSources,
    Ed2kSecIdentState, Ed2kSendingPart, Ed2kSignature, PartAvailabilityMask, OP_ACCEPTUPLOADREQ,
    OP_AICHFILEHASHREQ, OP_AICH_REQUEST, OP_CANCELTRANSFER, OP_CHANGE_CLIENT_ID, OP_COMPRESSEDPART,
    OP_COMPRESSEDPART_I64, OP_EMULE_INFO, OP_EMULE_INFO_ANSWER, OP_END_OF_DOWNLOAD,
    OP_FILEREQANSNOFILE, OP_FILESTATUS, OP_FWCHECKUDPREQ, OP_HASHSETREQUEST, OP_HASHSETREQUEST2,
    OP_MULTIPACKET,
    OP_MULTIPACKET_EXT, OP_MULTIPACKET_EXT2, OP_OUTOFPARTS, OP_PUBLICIP_ANSWER, OP_PUBLICIP_REQ,
    OP_PUBLICKEY, OP_QUEUERANKING, OP_REQFILENAME, OP_REQFILENAME_ANSWER, OP_REQUESTPARTS,
    OP_REQUESTPARTS_EMULE, OP_REQUESTPARTS_I64, OP_REQUESTSOURCES, OP_REQUESTSOURCES2,
    OP_SECIDENTSTATE, OP_SETREQFILEID_EMULE, OP_SIGNATURE, OP_STARTUPLOADREQ, PR_ED2K, PR_EMULE,
};
use mule_protocol::framing::PacketCodec;

use super::share_browser;

use super::types::{InboundListenerService, MAX_READ_BUFFER_SIZE};
use crate::services::obfuscation_stream::ObfuscatedStream;
use crate::services::peer_session::source_exchange as sx;
use crate::services::upload_queue::UploadSlotState;

/// Tamaño máximo de fragmento de carga útil por paquete de subida
/// (`CreateStandardPackets` / `CreatePackedPackets` de eMule 0.72a).
const UPLOAD_PACKET_CHUNK_SIZE: usize = 10_240;

/// Convierte una dirección IPv4 al `u32` utilizado en la carga útil firmada SUI V2
/// (little-endian en el cable). Los pares IPv6 llevan `0`, coincidiendo con eMule que es
/// solo IPv4 para este campo.
fn ip_to_u32(ip: Option<std::net::IpAddr>) -> u32 {
    match ip {
        Some(std::net::IpAddr::V4(v4)) => u32::from_be_bytes(v4.octets()),
        _ => 0,
    }
}

/// `ExtendedRequestsVersion` anunciada en un `OP_EMULEINFO` (etiqueta
/// `ET_EXTENDEDREQUEST` `0x25`, `ProcessMuleInfoPacket` de eMule →
/// `m_byExtendedRequestsVer`). `None` cuando la etiqueta está ausente.
fn emule_info_ext_req_version(info: &mule_protocol::ed2k::Ed2kEmuleInfo) -> Option<u8> {
    use mule_protocol::ed2k::tags::{TagName, TagValue, TAG_SPECIAL_EMULE_EXT_EXTENDEDREQUEST};
    info.tags.iter().find_map(|t| {
        if !matches!(t.name, TagName::Id(TAG_SPECIAL_EMULE_EXT_EXTENDEDREQUEST)) {
            return None;
        }
        match t.value {
            TagValue::UInt32(v) => u8::try_from(v).ok(),
            TagValue::UInt16(v) => u8::try_from(v).ok(),
            TagValue::UInt8(v) => Some(v),
            _ => None,
        }
    })
}

impl InboundListenerService {
    /// Dispatches post-handshake incoming peer requests.
    pub(crate) async fn dispatch_requests(
        &self,
        mut stream: ObfuscatedStream<tokio::net::TcpStream>,
        remote_str: String,
        local_ip: std::net::IpAddr,
        cancel_token: mule_network_policy::CancellationToken,
        peer_supports_compression: bool,
        peer_user_hash: Option<[u8; 16]>,
        mut sx_caps: sx::PeerSxCaps,
        peer_ext_req: u8,
        peer_tcp_port: u16,
        peer_udp_port: Option<u16>,
        peer_supports_large_files: bool,
    ) {
        let mut read_buf = BytesMut::with_capacity(65536);
        let mut raw_buf = [0u8; 8192];
        let codec = PacketCodec::new();
        let decoder = Ed2kDecoder::default();
        let encoder = Ed2kEncoder::default();

        let mut remote_pubkey_hash: Option<String> = None;
        let mut remote_pubkey_der: Option<Vec<u8>> = None;
        let mut credit_multiplier = 1.0f64;
        // IS_IDBADGUY de eMule: un par cuya firma SUI fue explícitamente
        // rechazada obtiene puntuación 0 en la cola de subida (nunca es promocionado).
        let mut sui_rejected = false;
        // Firma SUI verificada con éxito (requerida en modo paranoico).
        let mut sui_verified = false;
        let mut active_challenge: Option<u32> = None;
        let mut pending_peer_challenge: Option<(u8, u32)> = None;
        let mut answered_peer_challenge = false;
        let peer_ip = remote_str.parse::<SocketAddr>().ok().map(|a| a.ip());
        // Identidad en cola de subida: userhash cuando el par anunció uno en
        // OP_HELLO (eMule deduplica clientes en cola por identidad, no por IP —
        // un cliente reconectado conserva su posición, las IPs compartidas por NAT permanecen distintas),
        // recurriendo a la dirección del socket en su defecto.
        let client_key = peer_user_hash
            .map(|h| format!("uh:{}", hex::encode(h)))
            .unwrap_or_else(|| remote_str.clone());
        // Señal de promoción desde la cola de subida — `AddUpNextClient` de
        // eMule envía OP_ACCEPTUPLOADREQ en el socket que el
        // cliente en espera mantiene abierto; sin ella el cliente solo sería
        // atendido en su siguiente repetición de petición (~30 min).
        let mut promote_notify: Option<std::sync::Arc<tokio::sync::Notify>> = None;
        // Señal de rotación de ranura — `process_tick` (`CheckForTimeOver`)
        // la dispara cuando la sesión activa de este par es devuelta a la
        // cola; entonces enviamos OP_OUTOFPARTREQS (eMule
        // `SendOutOfPartReqsAndAddToWaitingQueue`, UploadQueue.cpp:312).
        let mut rotated_notify: Option<std::sync::Arc<tokio::sync::Notify>> = None;

        // `ConnectionEstablished` de eMule (BaseClient.cpp:1552) — si este par
        // tenía una repetición de petición pospuesta (`m_bReaskPending` establecido por la estrategia
        // porque espera en nuestra cola de subida), su petición de
        // archivo sale inmediatamente. Nuestros sockets entrantes/salientes no
        // se comparten: emitimos el `OP_DIRECTCALLBACKREQ` de inmediato —
        // el par está en línea — y su conexión de retorno será
        // adoptada vía `pending_kad_callbacks`.
        if let Some(ip) = peer_ip {
            let mgr = { self.peer_session.read().unwrap().clone() };
            if let Some(mgr) = mgr {
                if let Some(pending) = mgr.take_reask_pending(ip).await {
                    if let Some(udp_port) = peer_udp_port {
                        use std::sync::atomic::Ordering::Relaxed;
                        let obf = self.obfuscation_enabled.load(Relaxed);
                        let conn_opts = (obf as u8)
                            | ((obf as u8) << 1)
                            | ((self.obfuscation_required.load(Relaxed) as u8) << 2);
                        let our_hash = *self.user_hash.read().unwrap();
                        let our_tcp = self.listen_addr.lock().await.map(|a| a.port()).unwrap_or(0);
                        let client_udp = { self.client_udp.read().unwrap().clone() };
                        if let Some(client_udp) = client_udp {
                            if client_udp
                                .send_direct_callback_req(
                                    SocketAddr::new(ip, udp_port),
                                    our_tcp,
                                    &our_hash,
                                    conn_opts,
                                )
                                .await
                            {
                                mgr.register_pending_kad_callback(
                                    ip,
                                    pending.file_hash.clone(),
                                    5 * 60 * 1000,
                                );
                            }
                        }
                    }
                }
            }
        }

        // Entorno SX compartido (política + registros de limitación de eMule).
        let sx_env = self.sx_env();
        // Versión de peticiones extendidas negociada (`m_byExtendedRequestsVer`
        // de eMule, acotada a nuestro máx = 2) — refinada por
        // la etiqueta `ET_EXTENDEDREQUEST` de `OP_EMULEINFO` a su llegada.
        let mut peer_ext_req = peer_ext_req.min(2);
        // Estado de partes del solicitante por archivo (`CUpDownClient::m_abyPartStatus`
        // de eMule, mediante `OP_FILESTATUS` independiente o
        // la ext-info `OP_REQFILENAME` del multipaquete) — `reqstatus` de
        // `CreateSrcInfoPacket` para servir únicamente fuentes útiles.
        let mut peer_masks: std::collections::HashMap<[u8; 16], Vec<u8>> =
            std::collections::HashMap::new();
        // `CheckFailedFileIdReqs` de eMule — contador por conexión de
        // peticiones de archivo sobre hashes totalmente desconocidos; a las 6 eMule
        // incrementa la advertencia en clientlist y corta la conexión
        // (`throwCStr`, `BaseClient.cpp:2550`).
        let mut failed_file_id_reqs: u8 = 0;

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    break;
                }
                _ = async {
                    match &promote_notify {
                        Some(n) => n.notified().await,
                        None => std::future::pending().await,
                    }
                } => {
                    // El cliente acaba de ser promocionado desde la cola — enviar
                    // OP_ACCEPTUPLOADREQ (`AddUpNextClient` de eMule escribe en
                    // el socket en espera).
                    promote_notify = None;
                    let mut out = Vec::new();
                    out.push(PR_ED2K);
                    out.extend_from_slice(&1u32.to_le_bytes());
                    out.push(OP_ACCEPTUPLOADREQ);
                    let _ = stream.write_all(&out).await;
                }
                _ = async {
                    match &rotated_notify {
                        Some(n) => n.notified().await,
                        None => std::future::pending().await,
                    }
                } => {
                    // Nuestra ranura de subida acaba de ser rotada
                    // (`CheckForTimeOver`) — el descargador vuelve a estar
                    // OnQueue en su lado en lugar de esperar sus bloques.
                    rotated_notify = None;
                    let mut out = Vec::new();
                    out.push(PR_ED2K);
                    out.extend_from_slice(&1u32.to_le_bytes());
                    out.push(OP_OUTOFPARTS);
                    let _ = stream.write_all(&out).await;
                }
                read_res = stream.read(&mut raw_buf) => {
                    let n = match read_res {
                        Ok(n) if n > 0 => n,
                        _ => break,
                    };
                    read_buf.extend_from_slice(&raw_buf[..n]);
                    if read_buf.len() > MAX_READ_BUFFER_SIZE {
                        warn!("Inbound peer {} exceeded maximum read buffer size ({}), disconnecting", log_addr_str(&remote_str), MAX_READ_BUFFER_SIZE);
                        break;
                    }

                    while let Ok(Some(pkt)) = codec.decode(&mut read_buf) {
                        // OP_PACKEDPROT (0xD4) — eMule descomprime en línea y
                        // reetiqueta el paquete como OP_EMULEPROT
                        // (ListenSocket.cpp:1742 + paso continuo); cargas útiles
                        // empaquetadas corruptas se descartan.
                        let Some(pkt) = mule_protocol::ed2k::packed::unpack_owned(
                            pkt,
                            mule_protocol::ed2k::packed::PACKED_PEER_MAX_DECOMPRESSED,
                        ) else {
                            continue;
                        };
                        if pkt.protocol == PR_EMULE && pkt.opcode != OP_REQUESTPARTS_I64 {
                            match pkt.opcode {
                                OP_PUBLICKEY => {
                                    if let Ok(pk) = decoder.decode_public_key(pkt.protocol, pkt.opcode, &pkt.payload, 0) {
                                        if pk.key_bytes.len() > mule_crypto::legacy_sui::MAX_PUBKEY_WIRE {
                                            self.sui_stats.malformed_packets.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            warn!(
                                                "[SUI] Inbound peer {} sent oversized public key ({} bytes), ignoring",
                                                log_addr_str(&remote_str), pk.key_bytes.len()
                                            );
                                            continue;
                                        }
                                        let hash = Md4::digest(&pk.key_bytes);
                                        let hash_hex = hex::encode(hash);
                                        remote_pubkey_hash = Some(hash_hex.clone());
                                        remote_pubkey_der = Some(pk.key_bytes.to_vec());
                                        // Aquí no se concede multiplicador de crédito: requiere una verificación
                                        // real de firma RSASSA-PKCS1v15-SHA1 (OP_SIGNATURE).
                                        tracing::info!(
                                            "[SUI] Inbound peer {} sent public key ({} bytes, md4={}...)",
                                            log_addr_str(&remote_str), pk.key_bytes.len(), &hash_hex[..16]
                                        );

                                        // Solo emitir un desafío si disponemos de una identidad local utilizable:
                                        // el par firma *nuestra* clave pública, por lo que sin clave no hay SUI.
                                        let Some(my_pub) =
                                            self.sui_pubkey_der.read().unwrap().clone()
                                        else {
                                            warn!(
                                                "[SUI] No local identity key, not issuing challenge to {}",
                                                log_addr_str(&remote_str)
                                            );
                                            continue;
                                        };
                                        let _ = my_pub;
                                        let challenge_val: u32 = rand::random();
                                        active_challenge = Some(challenge_val);
                                        let sec_state = Ed2kSecIdentState {
                                            state: 1,
                                            challenge: challenge_val,
                                        };
                                        if let Ok(st_pkt) = encoder.encode_secident_state(&sec_state) {
                                            let mut out = Vec::new();
                                            out.push(st_pkt.protocol);
                                            out.extend_from_slice(&(st_pkt.frame_len() as u32).to_le_bytes());
                                            out.push(st_pkt.opcode);
                                            out.extend_from_slice(&st_pkt.payload);
                                            let _ = stream.write_all(&out).await;
                                            self.sui_stats.challenges_issued_in.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            tracing::debug!(
                                                "[SUI] Challenge issued to {} (state=1)",
                                                log_addr_str(&remote_str)
                                            );
                                        }
                                        if let (Some((state, ch)), Some(pk)) =
                                            (pending_peer_challenge.take(), remote_pubkey_der.clone())
                                        {
                                            self.answer_peer_challenge(
                                                state, ch, &pk, &remote_str, &encoder, &mut stream,
                                            ).await;
                                            answered_peer_challenge = true;
                                        }
                                    }
                                }
                                OP_SECIDENTSTATE => {
                                    // El par entrante nos desafía (rol de firmante):
                                    // responder con nuestra OP_PUBLICKEY cuando se
                                    // solicite (state=2) más OP_SIGNATURE sobre
                                    // `peer_pubkey || challenge` (V1).
                                    match decoder.decode_secident_state(pkt.protocol, pkt.opcode, &pkt.payload, 0) {
                                        Ok(st) if st.state != 0 && !answered_peer_challenge => {
                                            match remote_pubkey_der.clone() {
                                                Some(pk) => {
                                                    self.answer_peer_challenge(
                                                        st.state, st.challenge, &pk,
                                                        &remote_str, &encoder, &mut stream,
                                                    ).await;
                                                    answered_peer_challenge = true;
                                                }
                                                None => {
                                                    pending_peer_challenge = Some((st.state, st.challenge));
                                                    tracing::debug!(
                                                        "[SUI] Inbound peer {} sent challenge (state={}) before its public key; deferred",
                                                        log_addr_str(&remote_str), st.state
                                                    );
                                                }
                                            }
                                        }
                                        Ok(_) => {}
                                        Err(e) => {
                                            self.sui_stats.malformed_packets.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            warn!("[SUI] Malformed OP_SECIDENTSTATE from {}: {}", log_addr_str(&remote_str), e);
                                        }
                                    }
                                }
                                OP_SIGNATURE => {
                                    // Verificación real de SUI de eMule: RSA RSASSA-PKCS1v15-SHA1 sobre
                                    // `challenger_pubkey || challenge [|| ip || kind]`.
                                    // Véase docs/ed2k/SUI_WIRE_FORMAT.md.
                                    let challenge = active_challenge.take();

                                    // Anti-inundación (conforme a eMule): una firma por IP
                                    // remota. `m_dwLastSignatureIP` solo se asigna una vez que el
                                    // paquete llega a VerifyIdent — salidas tempranas (sin pubkey, sin
                                    // desafío) NO consumen la ranura.
                                    let flooded = match peer_ip {
                                        Some(ip) => *self.last_sig_ip.lock().await == Some(ip),
                                        None => false,
                                    };

                                    let mut kind_invalid = false;
                                    let mut verified_ok = false;
                                    let reject_reason: Option<&'static str> = if flooded {
                                        Some("signature flood (duplicate source IP)")
                                    } else {
                                        let local_pubkey =
                                            self.sui_pubkey_der.read().unwrap().clone();
                                        match (challenge, local_pubkey, remote_pubkey_der.clone()) {
                                            (Some(ch), Some(my_pub), Some(peer_der)) => {
                                                match decoder.decode_signature(pkt.protocol, pkt.opcode, &pkt.payload, 0) {
                                                    Ok(sig) => {
                                                        // V2 wire: [u8 siglen][sig][u8 kind].
                                                        let sig_len = pkt.payload.first().copied().unwrap_or(0) as usize;
                                                        let kind = if pkt.payload.len() == sig_len + 2 {
                                                            Some(pkt.payload[sig_len + 1])
                                                        } else {
                                                            None
                                                        };
                                                        let ip_u32 = match kind {
                                                            Some(20) => ip_to_u32(peer_ip),  // LOCALCLIENT: signer's IP
                                                            Some(10) => ip_to_u32(Some(local_ip)), // REMOTECLIENT: verifier's IP
                                                            Some(30) => 0,                   // NONECLIENT
                                                            Some(_) => { kind_invalid = true; 0 }
                                                            None => 0,                        // V1
                                                        };
                                                        if kind_invalid {
                                                            Some("invalid IP-binding kind")
                                                        } else {
                                                            // La verificación está a punto de ejecutarse:
                                                            // consumir la ranura de firma de esta IP.
                                                            if let Some(ip) = peer_ip {
                                                                *self.last_sig_ip.lock().await = Some(ip);
                                                            }
                                                            let version = if kind.is_some() {
                                                                SecureIdentVersion::V2
                                                            } else {
                                                                SecureIdentVersion::V1
                                                            };
                                                            let payload = build_sui_signing_payload(
                                                                version, &my_pub, ch, Some(ip_u32), kind,
                                                            );
                                                            let peer_key = parse_peer_public_key(&peer_der);
                                                            match (payload, peer_key) {
                                                                (Ok(pl), Ok(pk)) => {
                                                                    match verify_sui_signature(&pk, &pl, &sig.signature_bytes) {
                                                                        Ok(()) => {
                                                                            verified_ok = true;
                                                                            None
                                                                        }
                                                                        Err(_) => Some("RSA verification failed"),
                                                                    }
                                                                }
                                                                (Err(_), _) => Some("invalid signing payload"),
                                                                (_, Err(_)) => Some("invalid peer public key"),
                                                            }
                                                        }
                                                    }
                                                    Err(_) => Some("malformed signature packet"),
                                                }
                                            }
                                            (None, _, _) => Some("no active challenge"),
                                            (_, None, _) => Some("no local identity key"),
                                            (_, _, None) => Some("no peer public key"),
                                        }
                                    };

                                    use std::sync::atomic::Ordering::Relaxed;
                                    if flooded {
                                        self.sui_stats.signatures_rejected_flood.fetch_add(1, Relaxed);
                                    }
                                    if verified_ok {
                                        sui_verified = true;
                                        self.sui_stats.signatures_verified.fetch_add(1, Relaxed);
                                        // Verificado: asociar pubkey al registro de crédito (por hash de pubkey)
                                        // y conceder el multiplicador de crédito ahora.
                                        if let Some(ref ph) = remote_pubkey_hash {
                                            let mut dl_svc = self.downloads.lock().await;
                                            // `AddTrackClient` de eMule
                                            // (`ProcessPublicKeyPacket`,
                                            // BaseClient.cpp:1899): el par
                                            // (IP, puerto anunciado) →
                                            // userhash se memoriza 2 h —
                                            // base del `ComparePriorUserhash`
                                            // anti-usurpación.
                                            if let (Some(ip), Some(uh)) =
                                                (peer_ip, peer_user_hash)
                                            {
                                                dl_svc.track_client(
                                                    ip,
                                                    peer_tcp_port,
                                                    uh,
                                                );
                                            }
                                            let pub_hex = remote_pubkey_der
                                                .as_ref()
                                                .map(hex::encode)
                                                .unwrap_or_default();
                                            if let Ok(Some(credit)) = dl_svc.get_peer_credit(ph) {
                                                credit_multiplier = credit.credit_multiplier();
                                            }
                                            let _ = dl_svc.record_credit_upload(ph, &pub_hex, 0);
                                            tracing::info!(
                                                "[SUI] Inbound peer {} signature VERIFIED (md4={}..., credit_multiplier={:.2})",
                                                log_addr_str(&remote_str), &ph[..16.min(ph.len())], credit_multiplier
                                            );
                                        }
                                    } else {
                                        self.sui_stats.signatures_rejected.fetch_add(1, Relaxed);
                                        if reject_reason == Some("malformed signature packet") {
                                            self.sui_stats.malformed_packets.fetch_add(1, Relaxed);
                                        }
                                        tracing::warn!(
                                            "[SUI] Inbound peer {} signature REJECTED: {}",
                                            log_addr_str(&remote_str),
                                            reject_reason.unwrap_or("unknown")
                                        );
                                        remote_pubkey_hash = None;
                                        remote_pubkey_der = None;
                                        credit_multiplier = 1.0;
                                        sui_rejected = true;
                                        let sec_state = Ed2kSecIdentState {
                                            state: 0,
                                            challenge: 0,
                                        };
                                        if let Ok(st_pkt) = encoder.encode_secident_state(&sec_state) {
                                            let mut out = Vec::new();
                                            out.push(st_pkt.protocol);
                                            out.extend_from_slice(&(st_pkt.frame_len() as u32).to_le_bytes());
                                            out.push(st_pkt.opcode);
                                            out.extend_from_slice(&st_pkt.payload);
                                            let _ = stream.write_all(&out).await;
                                        }
                                    }
                                }
                                OP_AICHFILEHASHREQ => {
                                    if let Ok(req) = decoder.decode_aich_file_hash_req(
                                        pkt.protocol, pkt.opcode, &pkt.payload, 0,
                                    ) {
                                        let hash_hex = hex::encode(req.file_hash);
                                        let shared_file = {
                                            let dl_svc = self.downloads.lock().await;
                                            dl_svc.get_shared_file(&hash_hex).unwrap_or(None)
                                        };
                                        let root = match shared_file {
                                            Some(sf) if sf.enabled && sf.file_size > 0 => {
                                                self.shared_aich_root(&sf, &hash_hex).await
                                            }
                                            _ => None,
                                        };
                                        // eMule solo responde cuando el hashset AICH está
                                        // disponible; de lo contrario el solicitante agota su tiempo de espera.
                                        if let Some(root) = root {
                                            let res = Ed2kAichFileHashRes {
                                                file_hash: req.file_hash,
                                                aich_hash: root,
                                            };
                                            if let Ok(res_pkt) = encoder.encode_aich_file_hash_res(&res) {
                                                let mut out = Vec::new();
                                                out.push(res_pkt.protocol);
                                                out.extend_from_slice(&(res_pkt.frame_len() as u32).to_le_bytes());
                                                out.push(res_pkt.opcode);
                                                out.extend_from_slice(&res_pkt.payload);
                                                let _ = stream.write_all(&out).await;
                                                tracing::debug!(
                                                    "[AICH] Served master hash to {} for {}",
                                                    log_addr_str(&remote_str), hash_hex
                                                );
                                            }
                                        }
                                    }
                                }
                                OP_AICH_REQUEST => {
                                    if let Ok(req) = decoder.decode_aich_request(
                                        pkt.protocol, pkt.opcode, &pkt.payload, 0,
                                    ) {
                                        let hash_hex = hex::encode(req.file_hash);
                                        let shared_file = {
                                            let dl_svc = self.downloads.lock().await;
                                            dl_svc.get_shared_file(&hash_hex).unwrap_or(None)
                                        };

                                        let recovery = match shared_file {
                                            Some(sf) if sf.enabled && sf.file_size > 0 => {
                                                match self.shared_aich_tree(&sf, &hash_hex).await {
                                                    Some((tree, root)) if root == req.master_hash => {
                                                        mule_format::create_part_recovery_data(
                                                            &tree, sf.file_size, req.part as u32,
                                                        )
                                                        .ok()
                                                    }
                                                    _ => None,
                                                }
                                            }
                                            _ => None,
                                        };

                                        let answer = Ed2kAichAnswer {
                                            file_hash: req.file_hash,
                                            recovery: recovery.map(|entries| {
                                                // eMule utiliza identificadores de 16 bits a menos que el
                                                // archivo sea suficientemente grande como para desbordarlos.
                                                let mut entries16 = Vec::new();
                                                let mut entries32 = Vec::new();
                                                for e in &entries {
                                                    if e.ident <= u16::MAX as u32 {
                                                        entries16.push((e.ident as u16, e.hash.0));
                                                    } else {
                                                        entries32.push((e.ident, e.hash.0));
                                                    }
                                                }
                                                Ed2kAichRecovery {
                                                    part: req.part,
                                                    master_hash: req.master_hash,
                                                    entries16,
                                                    entries32,
                                                }
                                            }),
                                        };
                                        if let Ok(ans_pkt) = encoder.encode_aich_answer(&answer) {
                                            let mut out = Vec::new();
                                            out.push(ans_pkt.protocol);
                                            out.extend_from_slice(&(ans_pkt.frame_len() as u32).to_le_bytes());
                                            out.push(ans_pkt.opcode);
                                            out.extend_from_slice(&ans_pkt.payload);
                                            let _ = stream.write_all(&out).await;
                                            if answer.recovery.is_some() {
                                                tracing::debug!(
                                                    "[AICH] Served recovery data for part {} of {} to {}",
                                                    req.part, hash_hex, log_addr_str(&remote_str)
                                                );
                                            } else {
                                                tracing::debug!(
                                                    "[AICH] Could not serve recovery for part {} of {} to {} (failed answer sent)",
                                                    req.part, hash_hex, log_addr_str(&remote_str)
                                                );
                                            }
                                        }
                                    }
                                }
                                OP_FWCHECKUDPREQ => {
                                    // ProcessFirewallCheckUDPRequest de eMule:
                                    // leer u16 interno + u16 externo + u32 clave,
                                    // luego enviar KADEMLIA2_FIREWALLUDP a la
                                    // IP del solicitante en su puerto interno (y
                                    // puerto externo cuando difiera). La clave de
                                    // verificación UDP solo se utiliza para UDP Kad
                                    // cifrado (no implementado) — aceptada e
                                    // ignorada.
                                    if pkt.payload.len() >= 8 {
                                        let intern = u16::from_le_bytes([pkt.payload[0], pkt.payload[1]]);
                                        let extern_p = u16::from_le_bytes([pkt.payload[2], pkt.payload[3]]);
                                        if intern != 0 {
                                            let ku = {
                                                self.kad_udp.read().unwrap().as_ref().cloned()
                                            };
                                            if let (Some(ip), Some(ku)) = (peer_ip, ku) {
                                                // eMule marca los resultados como sesgados cuando
                                                // el solicitante ya es un contacto/par
                                                // conocido; no se rastrea aquí.
                                                let error_code: u8 = 0;
                                                let t1 = SocketAddr::new(ip, intern);
                                                match ku.send_firewall_udp(t1, error_code, intern).await {
                                                    Ok(()) => tracing::debug!(
                                                        "Kad FW check: sent FIREWALLUDP to {} for peer {}",
                                                        t1, log_addr_str(&remote_str)
                                                    ),
                                                    Err(e) => tracing::debug!(
                                                        "Kad FW check: failed FIREWALLUDP to {}: {}", t1, e
                                                    ),
                                                }
                                                if extern_p != 0 && extern_p != intern {
                                                    let t2 = SocketAddr::new(ip, extern_p);
                                                    let _ = ku
                                                        .send_firewall_udp(t2, error_code, extern_p)
                                                        .await;
                                                }
                                            }
                                        }
                                    }
                                }
                                mule_protocol::ed2k::OP_CALLBACK => {
                                    // ListenSocket.cpp:1326 de eMule — el compañero
                                    // nos retransmite un KADEMLIA_CALLBACK_REQ
                                    // (somos LowID):
                                    // [check u128][fileid u128][ip u32][tcp u16]
                                    // con check = ~notre_kad_id en el cable
                                    // (el buddyID de la petición = GetBuddyID
                                    // del destinatario, ya invertido en
                                    // eMule). Sin comprobación «es nuestro
                                    // compañero»: la inversión sirve
                                    // de autenticación.
                                    let our_kad = {
                                        let kad =
                                            self.kad.read().unwrap().as_ref().cloned();
                                        kad.map(|k| k.local_id().0)
                                    };
                                    // eMule: `if (!CKademlia::IsRunning())
                                    // return;` — Kad éteint → ignorer.
                                    let Some(our_kad) = our_kad else {
                                        continue;
                                    };
                                    if pkt.payload.len() < 38 {
                                        continue;
                                    }
                                    let check_ok = pkt.payload[..16]
                                        .iter()
                                        .zip(our_kad.iter())
                                        .all(|(c, k)| *c == !*k);
                                    if !check_ok {
                                        continue;
                                    }
                                    let fileid_hex =
                                        hex::encode(&pkt.payload[16..32]);
                                    let known = {
                                        let dl = self.downloads.lock().await;
                                        dl.get_shared_file(&fileid_hex)
                                            .unwrap_or(None)
                                            .is_some()
                                            || dl.find_by_ed2k_hash(&fileid_hex).is_some()
                                    };
                                    if !known {
                                        continue;
                                    }
                                    let req_ip = std::net::Ipv4Addr::new(
                                        pkt.payload[32],
                                        pkt.payload[33],
                                        pkt.payload[34],
                                        pkt.payload[35],
                                    );
                                    let req_port = u16::from_le_bytes(
                                        [pkt.payload[36], pkt.payload[37]],
                                    );
                                    // TryToConnect(true) de eMule: inyectar el
                                    // solicitante como fuente Kad — el
                                    // coordinador establece la conexión.
                                    let now_ms = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis()
                                        as i64;
                                    let src = mule_core::model::PeerSource {
                                        file_hash: fileid_hex.clone(),
                                        address: format!("{req_ip}:{req_port}"),
                                        ip: u32::from_le_bytes(req_ip.octets()),
                                        port: req_port,
                                        origin: mule_core::model::PeerSourceOrigin::Kad,
                                        discovered_at_unix_ms: now_ms,
                                        last_seen_unix_ms: now_ms,
                                        status: mule_core::model::PeerSourceStatus::Candidate,
                                        fail_count: 0,
                                        last_error: None,
                                        cooldown_until_unix_ms: None,
                                        user_hash: None,
                                        crypt_options: 0,
                                        buddy_id: None,
                                        software: None,
                                        availability: None,
                                        server_ip: 0,
                                        server_port: 0,
                                        udp_port: 0,
                                    };
                                    let mut dl = self.downloads.lock().await;
                                    let _ = dl.upsert_peer_sources(&fileid_hex, &[src]);
                                    tracing::info!(
                                        "OP_CALLBACK from buddy {}: requester {}:{} registered for file {}",
                                        log_addr_str(&remote_str),
                                        req_ip,
                                        req_port,
                                        fileid_hex
                                    );
                                }
                                mule_protocol::ed2k::OP_REASKCALLBACKTCP => {
                                    // ListenSocket.cpp:1391-1479 de eMule — re-ask
                                    // LowID retransmitido por el enlace TCP del compañero:
                                    // [destip u32][destport u16][reqhash 16]
                                    // [extinfo…]. Solo nuestro compañero activo puede
                                    // retransmitir (`GetBuddy() != client → ignore`),
                                    // y la respuesta sale en UDP hacia el
                                    // solicitante original, no hacia el compañero.
                                    let is_buddy = {
                                        let svc =
                                            self.kad_udp.read().unwrap().as_ref().cloned();
                                        match svc {
                                            Some(s) => s
                                                .buddy_registry()
                                                .lock()
                                                .await
                                                .active
                                                .as_ref()
                                                .is_some_and(|b| {
                                                    Some(b.remote_addr.ip()) == peer_ip
                                                }),
                                            None => false,
                                        }
                                    };
                                    if !is_buddy {
                                        tracing::debug!(
                                            "Inbound peer {} sent OP_REASKCALLBACKTCP but is not our buddy — ignored",
                                            log_addr_str(&remote_str)
                                        );
                                        continue;
                                    }
                                    if pkt.payload.len() < 22 {
                                        continue;
                                    }
                                    let dest_ip = std::net::Ipv4Addr::new(
                                        pkt.payload[0],
                                        pkt.payload[1],
                                        pkt.payload[2],
                                        pkt.payload[3],
                                    );
                                    let dest_port = u16::from_le_bytes(
                                        [pkt.payload[4], pkt.payload[5]],
                                    );
                                    let req_hash = hex::encode(&pkt.payload[6..22]);
                                    let dest = SocketAddr::new(
                                        std::net::IpAddr::V4(dest_ip),
                                        dest_port,
                                    );
                                    let client_udp =
                                        self.client_udp.read().unwrap().clone();
                                    let Some(client_udp) = client_udp else {
                                        continue;
                                    };
                                    let shared = {
                                        let dl = self.downloads.lock().await;
                                        dl.get_shared_file(&req_hash)
                                            .unwrap_or(None)
                                            .is_some()
                                    };
                                    let reply = if !shared {
                                        Some(mule_protocol::ed2k::client_udp::encode_file_not_found())
                                    } else {
                                        match self
                                            .upload_manager
                                            .waiting_rank_for_udp(
                                                &dest.ip(),
                                                dest_port,
                                                &req_hash,
                                            )
                                            .await
                                        {
                                            crate::services::upload_queue::UdpRankLookup::Rank(rank) => Some(
                                                mule_protocol::ed2k::client_udp::encode_reask_ack(
                                                    rank as u16, false,
                                                ),
                                            ),
                                            crate::services::upload_queue::UdpRankLookup::NoSender
                                                if self.upload_manager.is_nearly_full().await =>
                                            {
                                                Some(mule_protocol::ed2k::client_udp::encode_queue_full())
                                            }
                                            // FileMismatch / Ambiguous /
                                            // NoSender no saturado → silencio
                                            // (eMule fuerza una conexión TCP).
                                            _ => None,
                                        }
                                    };
                                    if let Some(reply) = reply {
                                        let _ = client_udp.send_raw(dest, &reply).await;
                                    }
                                }
                                OP_PUBLICIP_REQ => {
                                    // ListenSocket.cpp:1297 de eMule — responder
                                    // OP_PUBLICIP_ANSWER (PR_EMULE) con la IP
                                    // del solicitante tal como la vemos
                                    // (`PokeUInt32(client->GetIP())` — bytes
                                    // de red en bruto).
                                    if let Some(std::net::IpAddr::V4(v4)) = peer_ip {
                                        let mut out = Vec::with_capacity(10);
                                        out.push(PR_EMULE);
                                        out.extend_from_slice(&5u32.to_le_bytes());
                                        out.push(OP_PUBLICIP_ANSWER);
                                        out.extend_from_slice(&v4.octets());
                                        let _ = stream.write_all(&out).await;
                                    }
                                }
                                OP_EMULE_INFO | OP_EMULE_INFO_ANSWER => {
                                    // ProcessMuleInfoPacket de eMule: decodificar el
                                    // anuncio de capacidades extendidas y
                                    // responder OP_EMULEINFOANSWER ante OP_EMULEINFO.
                                    match decoder.decode_emule_info(
                                        pkt.protocol,
                                        pkt.opcode,
                                        &pkt.payload,
                                        0,
                                    ) {
                                        Ok(info) => {
                                            let tag_desc: Vec<String> = info
                                                .tags
                                                .iter()
                                                .map(|t| t.name.display_name())
                                                .collect();
                                            tracing::info!(
                                                "Inbound peer {} sent {} (client_ver=0x{:02X}, proto_ver=0x{:02X}, tags=[{}], unknown_tags={})",
                                                log_addr_str(&remote_str),
                                                if pkt.opcode == OP_EMULE_INFO {
                                                    "OP_EMULEINFO"
                                                } else {
                                                    "OP_EMULEINFO_ANSWER"
                                                },
                                                info.client_version,
                                                info.protocol_version,
                                                tag_desc.join(","),
                                                info.unknown_tags.len()
                                            );
                                            // ProcessMuleInfoPacket de eMule:
                                            // las etiquetas ET_SOURCEEXCHANGE /
                                            // ET_EXTENDEDREQUEST refinan
                                            // las capacidades negociadas.
                                            sx_caps.sx1_version = sx_caps
                                                .sx1_version
                                                .max(sx::emule_info_sx_version(&info));
                                            if let Some(v) =
                                                emule_info_ext_req_version(&info)
                                            {
                                                peer_ext_req = v.min(2);
                                            }
                                            if pkt.opcode == OP_EMULE_INFO {
                                                let answer =
                                                    crate::services::peer_session::handshake::build_emule_info(
                                                        self.kad_udp_port,
                                                        if self.sui_pubkey_der.read().unwrap().is_some() { 3 } else { 0 },
                                                    );
                                                if let Ok(ans_pkt) =
                                                    encoder.encode_emule_info_answer(&answer)
                                                {
                                                    let mut out = Vec::with_capacity(
                                                        6 + ans_pkt.payload.len(),
                                                    );
                                                    out.push(ans_pkt.protocol);
                                                    out.extend_from_slice(
                                                        &(ans_pkt.frame_len() as u32).to_le_bytes(),
                                                    );
                                                    out.push(ans_pkt.opcode);
                                                    out.extend_from_slice(&ans_pkt.payload);
                                                    let _ = stream.write_all(&out).await;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::debug!(
                                                "Inbound peer {} sent malformed OP_EMULEINFO, ignored: {e}",
                                                log_addr_str(&remote_str)
                                            );
                                        }
                                    }
                                }
                                OP_REQUESTSOURCES | OP_REQUESTSOURCES2 => {
                                    // Requête SX standalone — eMule
                                    // `ListenSocket.cpp:1165-1228`.
                                    let target_addr = remote_str
                                        .parse::<SocketAddr>()
                                        .unwrap_or_else(|_| {
                                            SocketAddr::from(([0, 0, 0, 0], 0))
                                        });
                                    match decoder.decode_request_sources(
                                        pkt.protocol, pkt.opcode, &pkt.payload,
                                    ) {
                                        Ok(req) => {
                                            let mask = peer_masks
                                                .get(&req.file_hash)
                                                .map(Vec::as_slice);
                                            sx::respond_to_sx_request(
                                                &req,
                                                pkt.opcode == OP_REQUESTSOURCES2,
                                                None,
                                                target_addr,
                                                &encoder,
                                                &self.downloads,
                                                &mut stream,
                                                &sx_env,
                                                &sx_caps,
                                                mask,
                                            )
                                            .await;
                                        }
                                        Err(e) => {
                                            warn!(
                                                "Inbound peer {} sent malformed SX request: {e}",
                                                log_addr_str(&remote_str)
                                            );
                                        }
                                    }
                                }
                                OP_MULTIPACKET | OP_MULTIPACKET_EXT
                                | OP_MULTIPACKET_EXT2 => {
                                    // eMule empaqueta sus peticiones de archivo
                                    // (filename, setreqfileid, SX, AICH) en
                                    // un OP_MULTIPACKET* — ListenSocket.cpp:970.
                                    match mule_protocol::ed2k::decoder::decode_multipacket_request(
                                        pkt.protocol,
                                        pkt.opcode,
                                        &pkt.payload,
                                        0,
                                        &decoder.limits,
                                        peer_ext_req,
                                    ) {
                                        Ok(mp) => {
                                            self.handle_inbound_multipacket(
                                                &mp,
                                                &remote_str,
                                                &mut peer_masks,
                                                &sx_env,
                                                &sx_caps,
                                                &encoder,
                                                &mut stream,
                                            )
                                            .await;
                                        }
                                        Err(e) => {
                                            warn!(
                                                "Inbound peer {} sent malformed multipacket: {e}",
                                                log_addr_str(&remote_str)
                                            );
                                        }
                                    }
                                }
                                mule_protocol::ed2k::OP_KAD_FWTCPCHECK_ACK => {
                                    // `ListenSocket.cpp:1623` de eMule — veredicto
                                    // de la prueba de cortafuegos TCP de Kad: el contacto
                                    // al que solicitamos (`FIREWALLED{2}_REQ`)
                                    // se conectó a nuestro puerto y acusa
                                    // recibo. Filtro `IsKadFirewallCheckIP`
                                    // — un ACK no solicitado se ignora.
                                    let tracker = self
                                        .kad_udp
                                        .read()
                                        .unwrap()
                                        .as_ref()
                                        .map(|u| u.firewall_tracker());
                                    if let (Some(tracker), Some(ip)) = (tracker, peer_ip) {
                                        let mut fw = tracker.lock().await;
                                        if fw.is_tcp_check_ip(ip) {
                                            fw.inc_tcp_acks();
                                            tracing::debug!(
                                                "Inbound peer {} sent OP_KAD_FWTCPCHECK_ACK (ack {} in TCP fw round)",
                                                log_addr_str(&remote_str),
                                                fw.tcp_acks
                                            );
                                        } else {
                                            tracing::debug!(
                                                "Unrequested OP_KAD_FWTCPCHECK_ACK from {}",
                                                log_addr_str(&remote_str)
                                            );
                                        }
                                    }
                                }
                                OP_HASHSETREQUEST2 => {
                                    // `SendHashsetPacket(bFileIdentifiers=true)` de eMule
                                    // (`UploadClient.cpp:510`): `<FileIdentifier><opts u8>`
                                    // — bit0 solicita el hashset MD4, bit1 el hashset
                                    // AICH por partes. La respuesta `OP_HASHSETANSWER2`
                                    // repite el identificador y serializa cada conjunto
                                    // solicitado disponible (`WriteHashSetsToPacket`,
                                    // `FileIdentifier.cpp:249`).
                                    match decoder.decode_hashset_request2(
                                        pkt.protocol,
                                        pkt.opcode,
                                        &pkt.payload,
                                        0,
                                    ) {
                                        Ok(req) => {
                                            let md4 = match req.identifier.md4_hash {
                                                Some(h) => h,
                                                // `read_from(.., false)` exige el MD4;
                                                // salvaguarda defensiva.
                                                None => continue,
                                            };
                                            let hash_hex = hex::encode(md4);
                                            let shared_file = {
                                                let dl_svc = self.downloads.lock().await;
                                                dl_svc.get_shared_file(&hash_hex).unwrap_or(None)
                                            };

                                            // `GetFileByIdentifier` (no estricto) =
                                            // `CompareRelaxed` (`FileIdentifier.cpp:76`):
                                            // MD4 igual + tamaño igual cuando ambos
                                            // se conocen + raíz AICH igual idem.
                                            let matches = shared_file.as_ref().is_some_and(|sf| {
                                                req.identifier
                                                    .file_size
                                                    .is_none_or(|s| s == sf.file_size)
                                                    && req.identifier.aich_hash.is_none_or(|a| {
                                                        sf.aich_root.is_none_or(|r| r == a)
                                                    })
                                            });
                                            if !matches {
                                                tracing::debug!(
                                                    "[HASHSET2] {} requests hashset for unknown/mismatching file {}",
                                                    log_addr_str(&remote_str),
                                                    hash_hex
                                                );
                                                continue;
                                            }

                                            let md4_set = if req.want_md4 {
                                                self.resolve_part_hashes(&hash_hex, &md4)
                                                    .await
                                                    .map(|part_hashes| Ed2kHashSetAnswer {
                                                        file_hash: md4,
                                                        part_hashes,
                                                    })
                                            } else {
                                                None
                                            };

                                            // AICH: conjunto completo por partes únicamente
                                            // calculable en un archivo compartido entero
                                            // (`shared_aich_tree`); un `.part` solo sirve
                                            // su raíz — el bit 0x02 se omite entonces
                                            // como en eMule cuando el conjunto no está disponible.
                                            let mut aich_root_known = shared_file
                                                .as_ref()
                                                .and_then(|sf| sf.aich_root);
                                            let aich_set = if req.want_aich {
                                                match &shared_file {
                                                    Some(sf) if sf.enabled => {
                                                        match self.shared_aich_tree(sf, &hash_hex).await
                                                        {
                                                            Some((tree, root)) => {
                                                                aich_root_known = Some(root);
                                                                match mule_format::aich_part_hashes(
                                                                    &tree,
                                                                    sf.file_size,
                                                                ) {
                                                                    Ok(part_hashes)
                                                                        if !part_hashes.is_empty() =>
                                                                    {
                                                                        Some(Ed2kAichPartHashSet {
                                                                            master_hash: root,
                                                                            part_hashes: part_hashes
                                                                                .iter()
                                                                                .map(|h| h.0)
                                                                                .collect(),
                                                                        })
                                                                    }
                                                                    _ => None,
                                                                }
                                                            }
                                                            None => None,
                                                        }
                                                    }
                                                    _ => None,
                                                }
                                            } else {
                                                None
                                            };

                                            let mut identifier =
                                                Ed2kFileIdentifier::new(md4, shared_file.as_ref().map_or(0, |sf| sf.file_size));
                                            identifier.aich_hash = aich_root_known;
                                            let answer = Ed2kHashSetAnswer2 {
                                                identifier,
                                                md4_set,
                                                aich_set,
                                            };
                                            match encoder.encode_hashset_answer2(&answer) {
                                                Ok(ans_pkt) => {
                                                    let mut out = Vec::with_capacity(6 + ans_pkt.payload.len());
                                                    out.push(ans_pkt.protocol);
                                                    out.extend_from_slice(&(ans_pkt.frame_len() as u32).to_le_bytes());
                                                    out.push(ans_pkt.opcode);
                                                    out.extend_from_slice(&ans_pkt.payload);
                                                    let _ = stream.write_all(&out).await;
                                                    tracing::debug!(
                                                        "[HASHSET2] Served hashset2 (md4={} aich={}) to {} for {}",
                                                        answer.md4_set.is_some(),
                                                        answer.aich_set.is_some(),
                                                        log_addr_str(&remote_str),
                                                        hash_hex
                                                    );
                                                }
                                                Err(e) => {
                                                    tracing::warn!(
                                                        "[HASHSET2] Failed to encode answer for {}: {e}",
                                                        log_addr_str(&remote_str)
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::debug!(
                                                "[HASHSET2] Malformed OP_HASHSETREQUEST2 from {}: {e}",
                                                log_addr_str(&remote_str)
                                            );
                                        }
                                    }
                                }
                                _ => {}
                            }
                            continue;
                        }

                        if pkt.protocol != PR_ED2K
                            && !(pkt.protocol == PR_EMULE && pkt.opcode == OP_REQUESTPARTS_I64)
                        {
                            continue;
                        }

                        match pkt.opcode {
                            mule_protocol::ed2k::OP_PORTTEST => {
                                // ListenSocket.cpp:1311 de eMule — confirmación «TCP
                                // alcanzable» inmediata: OP_PORTTEST +
                                // carga útil 0x12. El relevo del veredicto UDP
                                // (SendPortTestReply hacia el socket marcado
                                // m_bPortTestCon) permanece no implementado —
                                // herramienta externa, cf. compat-notes §24.
                                let mut out = Vec::with_capacity(7);
                                out.push(PR_ED2K);
                                out.extend_from_slice(&2u32.to_le_bytes());
                                out.push(mule_protocol::ed2k::OP_PORTTEST);
                                out.push(0x12);
                                let _ = stream.write_all(&out).await;
                            }
                            OP_SETREQFILEID_EMULE => {
                                if let Ok(req) = decoder.decode_file_status_req(pkt.protocol, pkt.opcode, &pkt.payload, 0) {
                                    let hash_hex = hex::encode(req.file_hash);

                                    let shared_file = {
                                        let dl_svc = self.downloads.lock().await;
                                        dl_svc.get_shared_file(&hash_hex).unwrap_or(None)
                                    };

                                    // `ListenSocket.cpp:359` de eMule: archivo
                                    // servido = compartido activo, o `.part` de
                                    // downloadqueue con `partcount > 1`.
                                    // Si no `OP_FILEREQANSNOFIL` + contador
                                    // anti-inundación (únicamente si el hash es
                                    // totalmente desconocido).
                                    let servable = shared_file.as_ref().is_some_and(|sf| {
                                        sf.enabled && {
                                            let part_count = if sf.file_size == 0 {
                                                1
                                            } else {
                                                sf.file_size.div_ceil(9728000).max(1) as u16
                                            };
                                            !sf.is_partial || part_count > 1
                                        }
                                    });

                                    if servable {
                                        let sf = shared_file.as_ref().expect("checked");
                                        let part_count = if sf.file_size == 0 {
                                            1
                                        } else {
                                            sf.file_size.div_ceil(9728000).max(1) as u16
                                        };
                                        let avail: Vec<u16> = if sf.is_partial {
                                            // eMule CPartFile: advertise only
                                            // fully-received parts (PARTSTATUS).
                                            sf.completed_parts.clone()
                                        } else {
                                            (0..part_count).collect()
                                        };
                                        let file_status = Ed2kFileStatus {
                                            file_hash: req.file_hash,
                                            part_count,
                                            availability_mask: PartAvailabilityMask::from_parts(&avail, part_count),
                                        };
                                        if let Ok(st_pkt) = encoder.encode_file_status(&file_status) {
                                            let mut out = Vec::new();
                                            out.push(st_pkt.protocol);
                                            out.extend_from_slice(&(st_pkt.frame_len() as u32).to_le_bytes());
                                            out.push(st_pkt.opcode);
                                            out.extend_from_slice(&st_pkt.payload);
                                            let _ = stream.write_all(&out).await;
                                        }
                                    } else {
                                        let mut out = Vec::with_capacity(6 + 16);
                                        out.push(PR_ED2K);
                                        out.extend_from_slice(&(17u32).to_le_bytes());
                                        out.push(OP_FILEREQANSNOFILE);
                                        out.extend_from_slice(&req.file_hash);
                                        let _ = stream.write_all(&out).await;
                                        if shared_file.is_none() {
                                            failed_file_id_reqs = failed_file_id_reqs.saturating_add(1);
                                        }
                                    }
                                }
                            }
                            OP_END_OF_DOWNLOAD => {
                                // `ListenSocket.cpp:537` de eMule: `0x49` +
                                // `hash16` — el descargador ha terminado; se
                                // retira de la cola de subida si el hash
                                // coincide con su archivo servido, si no
                                // contador anti-inundación.
                                if pkt.payload.len() >= 16 {
                                    let mut file_hash = [0u8; 16];
                                    file_hash.copy_from_slice(&pkt.payload[..16]);
                                    let hash_hex = hex::encode(file_hash);
                                    if !self
                                        .upload_manager
                                        .end_of_download(&client_key, &hash_hex)
                                        .await
                                    {
                                        // El hash no coincide con ningún archivo
                                        // servido para este cliente — no incrementar
                                        // salvo si es totalmente desconocido (`CheckFailedFileIdReqs`
                                        // de eMule omite archivos conocidos no compartidos).
                                        let known = {
                                            let dl_svc = self.downloads.lock().await;
                                            dl_svc.get_shared_file(&hash_hex).unwrap_or(None).is_some()
                                        };
                                        if !known {
                                            failed_file_id_reqs = failed_file_id_reqs.saturating_add(1);
                                        }
                                    }
                                }
                            }
                            OP_CHANGE_CLIENT_ID => {
                                // `ListenSocket.cpp:590` de eMule: aplica
                                // `SetServerIP`/`SetServerPort` sobre las
                                // filas de fuentes de archivos comprometidos por
                                // este par (servidor conocido en serverlist).
                                let engaged: Vec<String> = peer_masks
                                    .keys()
                                    .map(hex::encode)
                                    .collect();
                                crate::services::peer_session::change_client_id::apply_change_client_id(
                                    &pkt.payload,
                                    &remote_str,
                                    &engaged,
                                    &self.downloads,
                                )
                                .await;
                            }
                            OP_REQFILENAME => {
                                // `ListenSocket.cpp:292` de eMule — petición de
                                // nombre independiente (ruta tomada por
                                // pares que no anunciaron MultiPacket).
                                if pkt.payload.len() >= 16 {
                                    let mut file_hash = [0u8; 16];
                                    file_hash.copy_from_slice(&pkt.payload[..16]);
                                    let hash_hex = hex::encode(file_hash);
                                    let shared_file = {
                                        let dl_svc = self.downloads.lock().await;
                                        dl_svc.get_shared_file(&hash_hex).unwrap_or(None)
                                    };
                                    // Servido si compartido activo, o `.part` con
                                    // al menos una parte completa (eMule:
                                    // `GetCompletedSize() < PARTSIZE` →
                                    // silencio + contador anti-inundación).
                                    let servable = shared_file.as_ref().is_some_and(|sf| {
                                        sf.enabled && (!sf.is_partial || !sf.completed_parts.is_empty())
                                    });
                                    if let Some(sf) = shared_file.as_ref().filter(|_| servable) {
                                        // Ext-info del solicitante (`ProcessExtendedInfo`
                                        // de eMule): estado de partes
                                        // y luego complete-sources cuando
                                        // `ExtendedRequestsVersion > 1` —
                                        // alimenta `reqstatus` SX.
                                        if peer_ext_req > 0 {
                                            let mut off = 16usize;
                                            if pkt.payload.len() >= off + 2 {
                                                let n = u16::from_le_bytes(
                                                    [pkt.payload[off], pkt.payload[off + 1]],
                                                ) as usize;
                                                off += 2;
                                                let mask_len = n.div_ceil(8);
                                                if n > 0 && pkt.payload.len() >= off + mask_len {
                                                    peer_masks.insert(
                                                        file_hash,
                                                        pkt.payload[off..off + mask_len].to_vec(),
                                                    );
                                                }
                                            }
                                        }
                                        let name = sf.file_name.as_bytes();
                                        let frame_len = 1 + 16 + 2 + name.len();
                                        let mut out = Vec::with_capacity(5 + frame_len);
                                        out.push(PR_ED2K);
                                        out.extend_from_slice(
                                            &(frame_len as u32).to_le_bytes(),
                                        );
                                        out.push(OP_REQFILENAME_ANSWER);
                                        out.extend_from_slice(&file_hash);
                                        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
                                        out.extend_from_slice(name);
                                        let _ = stream.write_all(&out).await;

                                        // `CheckAndAddKnownSource` de eMule: el
                                        // solicitante anuncia (ext-info)
                                        // partes del mismo `.part` → fuente
                                        // candidata pasiva para este archivo.
                                        if sf.is_partial
                                            && sf.file_size > 9728000
                                            && peer_tcp_port > 0
                                            && peer_masks.get(&file_hash).is_some_and(|m| {
                                                m.iter().any(|b| *b != 0)
                                            })
                                        {
                                            if let Some(ip) = peer_ip {
                                                let now_ms = std::time::SystemTime::now()
                                                    .duration_since(std::time::UNIX_EPOCH)
                                                    .map(|d| d.as_millis() as i64)
                                                    .unwrap_or(0);
                                                let src = mule_core::model::PeerSource {
                                                    file_hash: hash_hex.clone(),
                                                    address: format!("{ip}:{peer_tcp_port}"),
                                                    ip: ip_to_u32(Some(ip)),
                                                    port: peer_tcp_port,
                                                    origin: mule_core::model::PeerSourceOrigin::SourceExchange,
                                                    discovered_at_unix_ms: now_ms,
                                                    last_seen_unix_ms: now_ms,
                                                    status: mule_core::model::PeerSourceStatus::Candidate,
                                                    fail_count: 0,
                                                    last_error: None,
                                                    cooldown_until_unix_ms: None,
                                                    user_hash: peer_user_hash.map(hex::encode),
                                                    crypt_options: 0,
                                                    buddy_id: None,
                                                    software: None,
                                                    availability: peer_masks.get(&file_hash).cloned(),
                                                    server_ip: 0,
                                                    server_port: 0,
                                                    udp_port: 0,
                                                };
                                                let mut dl_svc = self.downloads.lock().await;
                                                let _ = dl_svc.upsert_peer_sources(&hash_hex, &[src]);
                                            }
                                        }
                                    } else if shared_file.is_none() {
                                        failed_file_id_reqs = failed_file_id_reqs.saturating_add(1);
                                    }
                                }
                            }
                            mule_protocol::ed2k::OP_ASKSHAREDFILES
                            | mule_protocol::ed2k::OP_ASKSHAREDDIRS
                            | mule_protocol::ed2k::OP_ASKSHAREDFILESDIR => {
                                // `ListenSocket.cpp:651-825` de eMule — exploración
                                // de compartidos (anuncio `NoViewSharedFiles=0`
                                // en MISCOPTIONS1). Filtro `CanSeeShares`:
                                // everybody/friends/nobody. Respuestas
                                // PR_ED2K: lista completa 0x4B, lista de
                                // directorios 0x5F, archivos de un
                                // seudodirectorio 0x60, rechazo 0x61.
                                use mule_protocol::ed2k::shared_files as sfbrowse;

                                let (can_see, shares) = {
                                    let dl_svc = self.downloads.lock().await;
                                    (
                                        share_browser::peer_can_view_shares(
                                            &dl_svc,
                                            peer_user_hash,
                                        ),
                                        dl_svc.list_all_shared_files().unwrap_or_default(),
                                    )
                                };
                                let listen_port =
                                    self.listen_addr.lock().await.map(|a| a.port()).unwrap_or(0);
                                let ids = self.sx_local_ids(listen_port).await;
                                let limits = Ed2kLimits::default();
                                let offer_ctx = sfbrowse::OfferPeerContext {
                                    our_client_id: ids.client_id,
                                    our_tcp_port: ids.tcp_port,
                                    our_high_id: ids.is_high_id,
                                    peer_large_files: peer_supports_large_files,
                                };

                                let frame = match pkt.opcode {
                                    mule_protocol::ed2k::OP_ASKSHAREDFILES => {
                                        // eMule responde siempre — rechazo = lista
                                        // vacía (sin ASKSHAREDDENIEDANS aquí).
                                        let entries = if can_see {
                                            share_browser::offered_entries(
                                                shares.iter().filter(|f| f.enabled),
                                                &offer_ctx,
                                            )
                                        } else {
                                            Vec::new()
                                        };
                                        sfbrowse::encode_shared_files_answer(&entries, &limits)
                                    }
                                    mule_protocol::ed2k::OP_ASKSHAREDDIRS => {
                                        if can_see {
                                            sfbrowse::encode_shared_dirs_answer(
                                                &share_browser::browse_dirs(&shares),
                                                &limits,
                                            )
                                        } else {
                                            Ok(sfbrowse::encode_shared_denied())
                                        }
                                    }
                                    _ => {
                                        // OP_ASKSHAREDFILESDIR (0x5E)
                                        let dir =
                                            sfbrowse::decode_ask_files_dir(&pkt.payload);
                                        match (can_see, dir) {
                                            (false, _) => Ok(sfbrowse::encode_shared_denied()),
                                            (true, Some(dir)) => {
                                                let files = share_browser::files_in_dir(
                                                    &shares,
                                                    &dir,
                                                    peer_supports_large_files,
                                                );
                                                let entries = share_browser::offered_entries(
                                                    files.into_iter(),
                                                    &offer_ctx,
                                                );
                                                sfbrowse::encode_shared_files_dir_answer(
                                                    &dir, &entries, &limits,
                                                )
                                            }
                                            (true, None) => {
                                                Ok(sfbrowse::encode_shared_denied())
                                            }
                                        }
                                    }
                                };
                                match frame {
                                    Ok(rp) => {
                                        let mut out =
                                            Vec::with_capacity(6 + rp.payload.len());
                                        out.push(rp.protocol);
                                        out.extend_from_slice(
                                            &((rp.payload.len() as u32 + 1).to_le_bytes()),
                                        );
                                        out.push(rp.opcode);
                                        out.extend_from_slice(&rp.payload);
                                        let _ = stream.write_all(&out).await;
                                    }
                                    Err(e) => {
                                        tracing::debug!(
                                            "Shared-files answer encode failed for {peer}: {e}",
                                            peer = log_addr_str(&remote_str)
                                        );
                                    }
                                }
                            }
                            OP_FILESTATUS => {
                                // `CUpDownClient::ProcessFileStatus` de eMule:
                                // el solicitante anuncia las partes que posee
                                // (`m_abyPartStatus`) — `reqstatus` usado
                                // para servir solo fuentes útiles en SX.
                                if let Ok(st) = decoder.decode_file_status(
                                    pkt.protocol, pkt.opcode, &pkt.payload, 0,
                                ) {
                                    let mask = &st.availability_mask;
                                    let mut raw = vec![
                                        0u8;
                                        mule_protocol::ed2k::expected_mask_bytes(
                                            mask.part_count(),
                                        )
                                    ];
                                    for p in mask.available_parts() {
                                        raw[(p / 8) as usize] |= 1 << (p % 8);
                                    }
                                    peer_masks.insert(st.file_hash, raw);
                                }
                            }
                            OP_HASHSETREQUEST => {
                                if let Ok(req) = decoder.decode_hashset_request(pkt.protocol, pkt.opcode, &pkt.payload, 0) {
                                    let hash_hex = hex::encode(req.file_hash);
                                    if let Some(part_hashes) =
                                        self.resolve_part_hashes(&hash_hex, &req.file_hash).await
                                    {
                                        let answer = Ed2kHashSetAnswer {
                                            file_hash: req.file_hash,
                                            part_hashes,
                                        };
                                        if let Ok(ans_pkt) = encoder.encode_hashset_answer(&answer) {
                                            let mut out = Vec::new();
                                            out.push(ans_pkt.protocol);
                                            out.extend_from_slice(&(ans_pkt.frame_len() as u32).to_le_bytes());
                                            out.push(ans_pkt.opcode);
                                            out.extend_from_slice(&ans_pkt.payload);
                                            let _ = stream.write_all(&out).await;
                                            tracing::debug!(
                                                "[HASHSET] Served {} part hashes to {} for {}",
                                                answer.part_hashes.len(),
                                                log_addr_str(&remote_str),
                                                hash_hex
                                            );
                                        }
                                    }
                                }
                            }
                            OP_CANCELTRANSFER => {
                                // OP_CANCELTRANSFER de eMule (carga útil vacía): el
                                // descargador aborta la sesión — liberar la ranura
                                // de subida inmediatamente y cerrar el socket.
                                tracing::info!(
                                    "[UPLOAD] Inbound peer {} sent OP_CANCELTRANSFER, releasing upload slot",
                                    log_addr_str(&remote_str)
                                );
                                self.upload_manager.release_slot(&client_key).await;
                                return;
                            }
                            OP_OUTOFPARTS => {
                                // El descargador comunica que actualmente no tiene
                                // peticiones de bloques pendientes para este archivo.
                                tracing::debug!(
                                    "Inbound peer {} sent OP_OUTOFPARTS",
                                    log_addr_str(&remote_str)
                                );
                            }
                            OP_STARTUPLOADREQ => {
                                if pkt.payload.len() >= 16 {
                                    let mut file_hash = [0u8; 16];
                                    file_hash.copy_from_slice(&pkt.payload[..16]);
                                    let hash_hex = hex::encode(file_hash);

                                    let shared_file = {
                                        let dl_svc = self.downloads.lock().await;
                                        dl_svc.get_shared_file(&hash_hex).unwrap_or(None)
                                    };

                                    let shared_file = match shared_file {
                                        Some(sf) if sf.enabled => sf,
                                        _ => continue,
                                    };

                                    tracing::info!(
                                        "[UPLOAD_REQUEST] Inbound peer {} requested upload slot for file '{}' (hash: {})",
                                        log_addr_str(&remote_str), shared_file.file_name, hash_hex
                                    );

                                    // Modo paranoico (eMuleRust): servir únicamente
                                    // a pares ofuscados y cuya firma SUI
                                    // haya sido verificada.
                                    if self.paranoid_mode.load(std::sync::atomic::Ordering::Relaxed)
                                        && !(stream.is_obfuscated() && sui_verified)
                                    {
                                        tracing::warn!(
                                            "[PARANOID] Inbound peer {} denied upload slot (obfuscated={}, sui_verified={})",
                                            log_addr_str(&remote_str), stream.is_obfuscated(), sui_verified
                                        );
                                        continue;
                                    }

                                    let slot_res = self.upload_manager.request_upload_slot(&client_key, &remote_str, &hash_hex, &shared_file.file_name, credit_multiplier, &shared_file.up_priority, sui_rejected, peer_udp_port).await;
                                    if self.drain_upload_bans(peer_ip, &remote_str).await {
                                        break;
                                    }
                                    match slot_res {
                                        Ok((UploadSlotState::Uploading, _)) => {
                                            let mut out = Vec::new();
                                            out.push(PR_ED2K);
                                            out.extend_from_slice(&1u32.to_le_bytes());
                                            out.push(OP_ACCEPTUPLOADREQ);
                                            let _ = stream.write_all(&out).await;
                                            rotated_notify = self
                                                .upload_manager
                                                .rotation_notify(&client_key)
                                                .await;
                                        }
                                        Ok((UploadSlotState::Waiting, Some(rank))) => {
                                            // eMule `SendRankingInfo`
                                            // (`UploadClient.cpp:558`) :
                                            // PR_EMULE + payload 12 octets
                                            // `[u16 rank][10 zéros]`.
                                            let mut out = Vec::new();
                                            out.push(PR_EMULE);
                                            out.extend_from_slice(&13u32.to_le_bytes());
                                            out.push(OP_QUEUERANKING);
                                            out.extend_from_slice(&(rank as u16).to_le_bytes());
                                            out.extend_from_slice(&[0u8; 10]);
                                            let _ = stream.write_all(&out).await;
                                            promote_notify = self
                                                .upload_manager
                                                .promotion_notify(&client_key)
                                                .await;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            OP_REQUESTPARTS | OP_REQUESTPARTS_EMULE | OP_REQUESTPARTS_I64 => {
                                 // Decodifica peticiones de bloques de 32 bits (0x47/0x4B) o 64 bits (0xA3)
                                 // en una forma común (hash, rangos u64).
                                 let decoded_req = if pkt.opcode == OP_REQUESTPARTS_I64 {
                                     mule_protocol::ed2k::decoder::decode_request_parts_i64(
                                         pkt.protocol, pkt.opcode, &pkt.payload, 0,
                                     )
                                     .map(|r| {
                                         (
                                             r.file_hash,
                                             r.ranges.iter().map(|rg| (rg.start, rg.end)).collect::<Vec<_>>(),
                                         )
                                     })
                                 } else {
                                     decoder
                                         .decode_request_parts(pkt.protocol, pkt.opcode, &pkt.payload, 0)
                                         .map(|r| {
                                             (
                                                 r.file_hash,
                                                 r.ranges
                                                     .iter()
                                                     .map(|rg| (rg.start as u64, rg.end as u64))
                                                     .collect::<Vec<_>>(),
                                             )
                                         })
                                 };
                                 let is_i64_req = pkt.opcode == OP_REQUESTPARTS_I64;
                                 if let Ok((req_file_hash, req_ranges)) = decoded_req {
                                     let hash_hex = hex::encode(req_file_hash);

                                     let shared_file = {
                                         let dl_svc = self.downloads.lock().await;
                                         dl_svc.get_shared_file(&hash_hex).unwrap_or(None)
                                     };

                                     let shared_file = match shared_file {
                                         Some(sf) if sf.enabled => sf,
                                         _ => {
                                             continue;
                                         }
                                     };

                                     if self.paranoid_mode.load(std::sync::atomic::Ordering::Relaxed)
                                         && !(stream.is_obfuscated() && sui_verified)
                                     {
                                         continue;
                                     }

                                     let slot_res = self.upload_manager.request_upload_slot(&client_key, &remote_str, &hash_hex, &shared_file.file_name, credit_multiplier, &shared_file.up_priority, sui_rejected, peer_udp_port).await;
                                     if self.drain_upload_bans(peer_ip, &remote_str).await {
                                         break;
                                     }
                                     match slot_res {
                                         Ok((UploadSlotState::Uploading, _)) => {
                                             rotated_notify = self
                                                 .upload_manager
                                                 .rotation_notify(&client_key)
                                                 .await;
                                             for target_range in &req_ranges {
                                                 let (start, end) = *target_range;
                                                 // Partial share (in-progress download): only
                                                 // ranges fully inside a completed eD2K part may
                                                 // be served — anything else must be refused.
                                                 if shared_file.is_partial {
                                                     let part_size =
                                                         mule_protocol::ed2k::ED2K_PART_SIZE as u64;
                                                     let covered = shared_file.completed_parts.iter().any(|&i| {
                                                         let ps = i as u64 * part_size;
                                                         let pe = ((i as u64 + 1) * part_size)
                                                             .min(shared_file.file_size);
                                                         start >= ps && end <= pe
                                                     });
                                                     if !covered {
                                                         continue;
                                                     }
                                                 }
                                                 let length = (end.saturating_sub(start)).min(184320);
                                                 if length == 0 {
                                                     continue;
                                                 }

                                                 tracing::debug!(
                                                     "[UPLOAD_REQUEST] Inbound peer {} requested block [{}..{}] for file '{}' (hash: {})",
                                                     log_addr_str(&remote_str), start, end, shared_file.file_name, hash_hex
                                                 );

                                                 // Leer bytes reales del disco — nunca inventar
                                                 // datos: un rango no legible se deja sin responder
                                                 // (eMule simplemente no sirve lo que no puede
                                                 // leer), de lo contrario el par almacenaría
                                                 // bytes corruptos y fallaría el hash de su parte.
                                                 let mut payload = None;
                                                 if !shared_file.file_path.is_empty() {
                                                     let p = std::path::Path::new(&shared_file.file_path);
                                                     if p.is_file() {
                                                         if let Ok(mut f) = tokio::fs::File::open(p).await {
                                                             use tokio::io::AsyncSeekExt;
                                                             if f.seek(std::io::SeekFrom::Start(start)).await.is_ok() {
                                                                 let mut buf = vec![0u8; length as usize];
                                                                 if let Ok(read_bytes) = f.read_exact(&mut buf).await {
                                                                     if read_bytes == length as usize {
                                                                         payload = Some(buf);
                                                                     }
                                                                 }
                                                             }
                                                         }
                                                     }
                                                 }
                                                 let Some(payload) = payload else {
                                                     tracing::warn!(
                                                         "[UPLOAD_REQUEST] Cannot read [{}..{}] from '{}' — request left unanswered",
                                                         start, end, shared_file.file_path
                                                     );
                                                     continue;
                                                 };

                                                 // CreatePackedPackets de eMule: cuando el
                                                 // descargador anunció soporte de compresión
                                                 // (DataCompVer de MiscOptions1), intentar
                                                 // OP_COMPRESSEDPART (zlib, Z_BEST_SPEED) y
                                                 // recurrir a SENDINGPART sin comprimir cuando la
                                                 // compresión no aporte ganancia. Extensiones ya
                                                 // comprimidas se omiten
                                                 // (ShouldCompressBasedOnFilename) y eMule
                                                 // solo comprime bloques mayores de 31 bytes.
                                                 let compressible_name = match shared_file
                                                     .file_name
                                                     .rsplit_once('.')
                                                 {
                                                     Some((_, ext)) => !matches!(
                                                         ext.to_ascii_lowercase().as_str(),
                                                         "zip" | "rar" | "7z" | "cbz" | "cbr" | "ogm" | "ace"
                                                     ),
                                                     None => true,
                                                 };
                                                 let compressed = if peer_supports_compression
                                                     && compressible_name
                                                     && payload.len() > 31
                                                 {
                                                     use std::io::Write as _;
                                                     let mut enc = flate2::write::ZlibEncoder::new(
                                                         Vec::new(),
                                                         flate2::Compression::new(1),
                                                     );
                                                     if enc.write_all(&payload).is_ok() {
                                                         enc.finish().ok().filter(|c| c.len() < payload.len())
                                                     } else {
                                                         None
                                                     }
                                                 } else {
                                                     None
                                                 };

                                                 let upload_bucket = &self.network_policy.rate_limiters().upload_bucket;
                                                 let is_cancelled = || cancel_token.is_cancelled();
                                                 let use_i64 = is_i64_req || (start + length) > u32::MAX as u64;
                                                 let mut throttled_ok = true;
                                                 let mut award_bytes = 0u64;

                                                 if let Some(compressed) = compressed {
                                                     // OP_COMPRESSEDPART (0x40, OP_EMULEPROT):
                                                     // hash16 + u32 block_start + u32 total
                                                     // compressed size + fragmentos de <=10240 bytes.
                                                     // Variante I64 (0xA1): u64 block_start.
                                                     let mut sent = false;
                                                     for chunk in compressed.chunks(UPLOAD_PACKET_CHUNK_SIZE) {
                                                         if upload_bucket
                                                             .acquire_tokens(chunk.len() as u64, is_cancelled)
                                                             .await
                                                             .is_err()
                                                         {
                                                             throttled_ok = false;
                                                             sent = false;
                                                             break;
                                                         }
                                                         let hdr_len = if use_i64 { 28 } else { 24 };
                                                         let frame_len = 1 + hdr_len + chunk.len();
                                                         let mut out = Vec::with_capacity(5 + frame_len);
                                                         out.push(PR_EMULE);
                                                         out.extend_from_slice(&(frame_len as u32).to_le_bytes());
                                                         out.push(if use_i64 { OP_COMPRESSEDPART_I64 } else { OP_COMPRESSEDPART });
                                                         out.extend_from_slice(&req_file_hash);
                                                         if use_i64 {
                                                             out.extend_from_slice(&start.to_le_bytes());
                                                         } else {
                                                             out.extend_from_slice(&(start as u32).to_le_bytes());
                                                         }
                                                         out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
                                                         out.extend_from_slice(chunk);
                                                         if stream.write_all(&out).await.is_err() {
                                                             sent = false;
                                                             break;
                                                         }
                                                         sent = true;
                                                     }
                                                     if sent {
                                                         let _ = stream.flush().await;
                                                         tracing::debug!(
                                                             "[UPLOAD_BLOCK] Sent compressed block ({} -> {} bytes) to {} for file '{}' (offset {}..{})",
                                                             payload.len(), compressed.len(), log_addr_str(&remote_str), shared_file.file_name, start, start + length
                                                         );
                                                         self.upload_manager.update_progress(&client_key, length).await;
                                                         award_bytes = length;
                                                     }
                                                 } else {
                                                     // CreateStandardPackets de eMule: fragmentar
                                                     // OP_SENDINGPART / OP_SENDINGPART_I64 en subpaquetes
                                                     // de hasta `UPLOAD_PACKET_CHUNK_SIZE` bytes para no
                                                     // saturar el socket del par ni retardar el refresco
                                                     // del temporizador DOWNLOADTIMEOUT.
                                                     let mut chunk_offset = 0u64;
                                                     for chunk in payload.chunks(UPLOAD_PACKET_CHUNK_SIZE) {
                                                         let chunk_len = chunk.len() as u64;
                                                         if upload_bucket
                                                             .acquire_tokens(chunk_len, is_cancelled)
                                                             .await
                                                             .is_err()
                                                         {
                                                             throttled_ok = false;
                                                             break;
                                                         }
                                                         let chunk_start = start + chunk_offset;
                                                         let chunk_end = chunk_start + chunk_len;
                                                         chunk_offset += chunk_len;

                                                         let send_pkt = if use_i64 {
                                                             encoder.encode_sending_part_i64(
                                                                 &mule_protocol::ed2k::Ed2kSendingPartI64 {
                                                                     file_hash: req_file_hash,
                                                                     start_offset: chunk_start,
                                                                     end_offset: chunk_end,
                                                                     data: Bytes::copy_from_slice(chunk),
                                                                 },
                                                             )
                                                         } else {
                                                             encoder.encode_sending_part(
                                                                 &Ed2kSendingPart::new(
                                                                     req_file_hash,
                                                                     chunk_start as u32,
                                                                     chunk_end as u32,
                                                                     Bytes::copy_from_slice(chunk),
                                                                 ),
                                                             )
                                                         };

                                                         let Ok(send_pkt) = send_pkt else {
                                                             break;
                                                         };
                                                         let mut out = Vec::with_capacity(6 + send_pkt.payload.len());
                                                         out.push(send_pkt.protocol);
                                                         out.extend_from_slice(&(send_pkt.frame_len() as u32).to_le_bytes());
                                                         out.push(send_pkt.opcode);
                                                         out.extend_from_slice(&send_pkt.payload);
                                                         if stream.write_all(&out).await.is_err() {
                                                             break;
                                                         }
                                                         self.upload_manager.update_progress(&client_key, chunk_len).await;
                                                         award_bytes += chunk_len;
                                                     }
                                                     if award_bytes > 0 {
                                                         let _ = stream.flush().await;
                                                         tracing::debug!(
                                                             "[UPLOAD_BLOCK] Sent {} bytes to {} for file '{}' (offset {}..{})",
                                                             award_bytes, log_addr_str(&remote_str), shared_file.file_name, start, start + award_bytes
                                                         );
                                                     }
                                                 }

                                                 if award_bytes > 0
                                                     && shared_file.file_size > 0
                                                     && (start + length) <= shared_file.file_size
                                                 {
                                                     if let (Some(ref ph), Some(ref pk_der)) = (&remote_pubkey_hash, &remote_pubkey_der) {
                                                         let pk_hex = hex::encode(pk_der);
                                                         let mut dl_svc = self.downloads.lock().await;
                                                         if dl_svc.record_credit_upload(ph, &pk_hex, award_bytes).is_ok() {
                                                             let total_up = dl_svc.get_peer_credit(ph).ok().flatten().map(|c| c.uploaded_bytes).unwrap_or(award_bytes);
                                                             tracing::debug!(
                                                                 "[CREDITS_AWARDED] Awarded {} uploaded bytes to verified peer {} (pubkey hash: {}, total: {} bytes)",
                                                                 award_bytes, log_addr_str(&remote_str), ph, total_up
                                                             );
                                                         }
                                                     }
                                                 }

                                                 if !throttled_ok {
                                                     break;
                                                 }
                                             }
                                         }
                                        Ok((UploadSlotState::Waiting, Some(rank))) => {
                                            let mut out = Vec::new();
                                            out.push(PR_EMULE);
                                            out.extend_from_slice(&13u32.to_le_bytes());
                                            out.push(OP_QUEUERANKING);
                                            out.extend_from_slice(&(rank as u16).to_le_bytes());
                                            out.extend_from_slice(&[0u8; 10]);
                                            let _ = stream.write_all(&out).await;
                                            promote_notify = self
                                                .upload_manager
                                                .promotion_notify(&client_key)
                                                .await;
                                        }
                                        _ => {
                                            continue;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }

                        // `CheckFailedFileIdReqs` de eMule — a las 6 peticiones sobre
                        // archivos desconocidos, la conexión se corta
                        // (`throwCStr`); la sanción en clientlist se aproxima
                        // mediante el corte estricto en el daemon.
                        if failed_file_id_reqs >= 6 {
                            warn!(
                                "Inbound peer {} flooded file-id requests ({}), dropping connection (eMule FileReq flood)",
                                log_addr_str(&remote_str),
                                failed_file_id_reqs,
                            );
                            break;
                        }
                    }
                }
            }
        }

        // Socket muerto: eMule conserva el cliente en la cola
        // (`US_ONUPLOADQUEUE` sobrevive a `Disconnected`) — la ranura activa se
        // libera pero la entrada espera `MAX_PURGEQUEUETIME` o una reconexión.
        self.upload_manager.mark_disconnected(&client_key).await;
    }

    /// Traduce las sanciones `BAD_CLIENT_BAN` de la cola de subida a
    /// baneos de IP (`DownloadService::ban_source`, 2 h — `Ban()`/`AddBannedClient`
    /// de eMule). Devuelve `true` cuando el par actual
    /// acaba de ser baneado — quien llama debe hacer `break` para cortar el socket
    /// (`Disconnected` en `Ban` de eMule).
    async fn drain_upload_bans(
        &self,
        peer_ip: Option<std::net::IpAddr>,
        remote_str: &str,
    ) -> bool {
        let bans = self.upload_manager.drain_pending_bans().await;
        if bans.is_empty() {
            return false;
        }
        let mut drop_peer = false;
        {
            let mut dl = self.downloads.lock().await;
            for ip in &bans {
                dl.ban_source(ip);
                tracing::warn!(
                    "[BAN] Client IP {} banned (upload request flood — eMule BADCLIENTBAN)",
                    ip
                );
                if peer_ip.is_some_and(|p| &p.to_string() == ip) {
                    drop_peer = true;
                }
            }
        }
        if drop_peer {
            tracing::warn!(
                "[BAN] Inbound peer {} dropped — request flood ban",
                log_addr_str(remote_str)
            );
        }
        drop_peer
    }

    /// Despacha las subpeticiones de un `OP_MULTIPACKET*` entrante — `ListenSocket.cpp:970-1011`
    /// de eMule. El hash de archivo del envoltorio se aplica a cada
    /// subpetición; las subpeticiones SX van al mismo manejador que los
    /// opcodes independientes, y las respuestas de nombre/file-status/AICH se agrupan
    /// en un único `OP_MULTIPACKETANSWER` como hace eMule.
    #[allow(clippy::too_many_arguments)]
    async fn handle_inbound_multipacket(
        &self,
        mp: &mule_protocol::ed2k::Ed2kMultiPacketRequest,
        remote_str: &str,
        peer_masks: &mut std::collections::HashMap<[u8; 16], Vec<u8>>,
        sx_env: &sx::SxEnv,
        sx_caps: &sx::PeerSxCaps,
        encoder: &Ed2kEncoder,
        stream: &mut ObfuscatedStream<tokio::net::TcpStream>,
    ) {
        let file_hash = mp.file.file_hash();
        if file_hash == [0u8; 16] {
            // `FileIdentifier` sin MD4 (contexto Kad) — ninguna respuesta
            // de archivo posible.
            return;
        }
        let target_addr = remote_str
            .parse::<SocketAddr>()
            .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        let hash_hex = hex::encode(file_hash);

        let mut want_filename = false;
        let mut want_aich = false;
        for sub in &mp.sub_requests {
            match sub {
                Ed2kMultiPacketSubRequest::FileName { ext_info } => {
                    want_filename = true;
                    // Ext-info: estado de partes del solicitante → `reqstatus` SX.
                    if let Some(ext) = ext_info {
                        peer_masks.insert(file_hash, ext.part_status.to_vec());
                    }
                }
                Ed2kMultiPacketSubRequest::SetReqFileId => {}
                Ed2kMultiPacketSubRequest::RequestSources => {
                    let req = Ed2kRequestSources::new_v1(file_hash);
                    let mask = peer_masks.get(&file_hash).map(Vec::as_slice);
                    sx::respond_to_sx_request(
                        &req,
                        false,
                        None,
                        target_addr,
                        encoder,
                        &self.downloads,
                        stream,
                        sx_env,
                        sx_caps,
                        mask,
                    )
                    .await;
                }
                Ed2kMultiPacketSubRequest::RequestSources2 { version, options } => {
                    let req = Ed2kRequestSources {
                        file_hash,
                        version: *version,
                        options: *options,
                    };
                    let mask = peer_masks.get(&file_hash).map(Vec::as_slice);
                    sx::respond_to_sx_request(
                        &req,
                        true,
                        None,
                        target_addr,
                        encoder,
                        &self.downloads,
                        stream,
                        sx_env,
                        sx_caps,
                        mask,
                    )
                    .await;
                }
                Ed2kMultiPacketSubRequest::AichFileHash => want_aich = true,
            }
        }
        if !want_filename && !want_aich {
            return;
        }

        // Respuesta agrupada de eMule (`OP_MULTIPACKETANSWER`): nombre del archivo +
        // file-status (+ hash AICH) para el archivo del envoltorio.
        let shared_file = {
            let dl_svc = self.downloads.lock().await;
            dl_svc.get_shared_file(&hash_hex).unwrap_or(None)
        };
        let mut sub_answers: Vec<Ed2kMultiPacketSubAnswer> = Vec::new();
        if want_filename {
            if let Some(sf) = shared_file.as_ref().filter(|s| s.enabled) {
                sub_answers.push(Ed2kMultiPacketSubAnswer::FileName(sf.file_name.clone()));
                let part_count = if sf.file_size == 0 {
                    1
                } else {
                    sf.file_size.div_ceil(9728000).max(1) as u16
                };
                let avail: Vec<u16> = if sf.is_partial {
                    sf.completed_parts.clone()
                } else {
                    (0..part_count).collect()
                };
                let mut mask = vec![0u8; mule_protocol::ed2k::expected_mask_bytes(part_count)];
                for p in avail {
                    mask[(p / 8) as usize] |= 1 << (p % 8);
                }
                sub_answers.push(Ed2kMultiPacketSubAnswer::FileStatus {
                    part_count,
                    part_status: Bytes::from(mask),
                });
            }
        }
        if want_aich {
            if let Some(sf) = shared_file
                .as_ref()
                .filter(|s| s.enabled && s.file_size > 0)
            {
                if let Some(root) = self.shared_aich_root(sf, &hash_hex).await {
                    sub_answers.push(Ed2kMultiPacketSubAnswer::AichFileHash(root));
                }
            }
        }
        if sub_answers.is_empty() {
            return;
        }
        let ans = Ed2kMultiPacketAnswer {
            file: mp.file.clone(),
            sub_answers,
        };
        match encoder.encode_message(&Ed2kMessage::MultiPacketAnswer(ans)) {
            Ok(ans_pkt) => {
                let mut out = Vec::with_capacity(6 + ans_pkt.payload.len());
                out.push(ans_pkt.protocol);
                out.extend_from_slice(&(ans_pkt.frame_len() as u32).to_le_bytes());
                out.push(ans_pkt.opcode);
                out.extend_from_slice(&ans_pkt.payload);
                let _ = stream.write_all(&out).await;
            }
            Err(e) => {
                warn!(
                    "Failed to encode multipacket answer for {peer}: {e}",
                    peer = log_addr_str(remote_str)
                );
            }
        }
    }

    /// Devuelve la raíz AICH de confianza de un archivo compartido, calculando (y almacenando
    /// en caché) el árbol de hashes desde el disco cuando esté ausente.
    /// Resuelve el conjunto de hashes MD4 por partes para `hash_hex`: primero la caché en BD
    /// (aprendida durante la descarga o ya calculada), si no calculada a partir del
    /// archivo compartido en disco y almacenada en caché (`hash_reader` verifica el hash raíz
    /// contra `expected_hash` antes de servir nada).
    async fn resolve_part_hashes(
        &self,
        hash_hex: &str,
        expected_hash: &[u8; 16],
    ) -> Option<Vec<[u8; 16]>> {
        let cached = {
            let dl_svc = self.downloads.lock().await;
            dl_svc.get_part_hashes_by_hash(hash_hex)
        };
        if !cached.is_empty() {
            return Some(cached);
        }
        let shared_file = {
            let dl_svc = self.downloads.lock().await;
            dl_svc.get_shared_file(hash_hex).unwrap_or(None)
        };
        match shared_file {
            Some(sf) if sf.enabled && !sf.is_partial && !sf.file_path.is_empty() => {
                let path = sf.file_path.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let mut f = std::fs::File::open(&path)?;
                    mule_format::ed2k::hash_reader(&mut f)
                })
                .await;
                match res {
                    Ok(Ok(hashed)) => {
                        if hashed.root_hash.0 == *expected_hash {
                            let mut dl_svc = self.downloads.lock().await;
                            let _ = dl_svc
                                .store_part_hashes_by_hash(hash_hex, &hashed.part_hashes);
                            Some(hashed.part_hashes)
                        } else {
                            warn!(
                                "[HASHSET] Shared file {} on disk does not match its ed2k hash {}, refusing to serve hashset",
                                sf.file_name, hash_hex
                            );
                            None
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    async fn shared_aich_root(
        &self,
        sf: &mule_core::model::SharedFile,
        hash_hex: &str,
    ) -> Option<[u8; 20]> {
        if sf.is_partial {
            return sf.aich_root;
        }
        self.shared_aich_tree(sf, hash_hex).await.map(|(_, r)| r)
    }

    /// Carga o calcula el árbol de hashes AICH completo de un archivo compartido.
    /// La raíz calculada se persiste en el registro del archivo compartido.
    async fn shared_aich_tree(
        &self,
        sf: &mule_core::model::SharedFile,
        hash_hex: &str,
    ) -> Option<(mule_format::AichNode, [u8; 20])> {
        if sf.file_path.is_empty() || sf.file_size == 0 {
            return None;
        }
        // Compartido parcial: el árbol AICH no puede calcularse a partir de un archivo
        // .part incompleto — la raíz de confianza (del .met) sigue siendo servible vía
        // `shared_aich_root`, pero no se puede construir ningún conjunto de recuperación.
        if sf.is_partial {
            return None;
        }
        let path = sf.file_path.clone();
        let size = sf.file_size;
        let tree = tokio::task::spawn_blocking(move || {
            let mut f = std::fs::File::open(&path).ok()?;
            mule_format::build_aich_tree_from_reader(&mut f, size).ok()
        })
        .await
        .ok()??;
        let root = tree.root_hash()?.0;
        if let Some(cached) = sf.aich_root {
            if cached != root {
                warn!(
                    "[AICH] Shared file {} on disk does not match its cached AICH root",
                    sf.file_name
                );
                return None;
            }
        } else {
            let mut dl_svc = self.downloads.lock().await;
            let _ = dl_svc.set_shared_aich_root(hash_hex, &root);
        }
        Some((tree, root))
    }

    /// Responde al desafío `OP_SECIDENTSTATE` de un par entrante (rol de firmante):
    /// envía nuestra `OP_PUBLICKEY` cuando se solicita (state=2), luego `OP_SIGNATURE`
    /// sobre `peer_pubkey || challenge` (carga útil V1).
    async fn answer_peer_challenge(
        &self,
        state: u8,
        challenge: u32,
        peer_pubkey_der: &[u8],
        remote_str: &str,
        encoder: &Ed2kEncoder,
        stream: &mut ObfuscatedStream<tokio::net::TcpStream>,
    ) {
        let Some(identity) = &self.identity else {
            return;
        };

        if state == 2 {
            let my_pub = self.sui_pubkey_der.read().unwrap().clone();
            if let Some(my_pub) = my_pub {
                if let Ok(pk_pkt) = encoder.encode_public_key(&Ed2kPublicKey {
                    key_bytes: Bytes::copy_from_slice(&my_pub),
                }) {
                    let mut out = Vec::new();
                    out.push(pk_pkt.protocol);
                    out.extend_from_slice(&(pk_pkt.frame_len() as u32).to_le_bytes());
                    out.push(pk_pkt.opcode);
                    out.extend_from_slice(&pk_pkt.payload);
                    if let Err(e) = stream.write_all(&out).await {
                        warn!(
                            "[SUI] Failed to send OP_PUBLICKEY to {}: {}",
                            log_addr_str(remote_str),
                            e
                        );
                        return;
                    }
                }
            }
        }

        let payload = match build_sui_signing_payload(
            SecureIdentVersion::V1,
            peer_pubkey_der,
            challenge,
            None,
            None,
        ) {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    "[SUI] Failed to build signing payload for {}: {}",
                    log_addr_str(remote_str),
                    e
                );
                return;
            }
        };
        let sig = {
            let id = identity.lock().await;
            id.sign_sui_payload(&payload)
        };
        match sig {
            Ok(sig_bytes) => {
                if let Ok(sig_pkt) = encoder.encode_signature(&Ed2kSignature {
                    signature_bytes: Bytes::from(sig_bytes),
                }) {
                    let mut out = Vec::new();
                    out.push(sig_pkt.protocol);
                    out.extend_from_slice(&(sig_pkt.frame_len() as u32).to_le_bytes());
                    out.push(sig_pkt.opcode);
                    out.extend_from_slice(&sig_pkt.payload);
                    match stream.write_all(&out).await {
                        Ok(()) => {
                            self.sui_stats
                                .signatures_sent
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            tracing::info!(
                                "[SUI] Answered challenge from inbound peer {} (state={}, V1 signature)",
                                log_addr_str(remote_str), state
                            );
                        }
                        Err(e) => {
                            warn!(
                                "[SUI] Failed to send OP_SIGNATURE to {}: {}",
                                log_addr_str(remote_str),
                                e
                            )
                        }
                    }
                }
            }
            Err(e) => warn!(
                "[SUI] Failed to sign challenge from {}: {}",
                log_addr_str(remote_str),
                e
            ),
        }
    }
}

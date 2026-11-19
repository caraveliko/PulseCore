// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use crate::services::peer_session::log_peer_addr;
use bytes::BytesMut;
use std::net::SocketAddr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, Mutex};
use tracing::info;

use mule_core::model::PeerFailureReason;
use mule_protocol::ed2k::{
    Ed2kAichRequest, Ed2kDecoder, Ed2kEncoder, Ed2kFileStatus, Ed2kHashSetReq, Ed2kHashSetReq2,
    OP_ACCEPTUPLOADREQ,
    OP_AICHFILEHASHRES, OP_AICH_ANSWER, OP_ANSWERSOURCES, OP_ANSWERSOURCES2, OP_CHANGE_CLIENT_ID,
    OP_FILEREQANSNOFILE, OP_FILESTATUS, OP_HASHSETANSWER, OP_HASHSETANSWER2, OP_QUEUERANK,
    OP_QUEUERANKING, OP_REQFILENAME_ANSWER,
    OP_REQUESTSOURCES, OP_REQUESTSOURCES2, OP_STARTUPLOADREQ, PR_ED2K, PR_EMULE,
};
use mule_protocol::framing::{PacketCodec, RawPacket};

use super::events::{emit_download_update, emit_event};
use super::source_exchange::{handle_sx_answer, handle_sx_request, PeerSxCaps, SxEnv, SxLocalIds};
use super::types::{
    PeerConnectionStatus, PeerError, PeerTimeouts, SwarmInnerState, MAX_PEER_READ_BUFFER_SIZE,
};
use crate::services::downloads::DownloadService;
use crate::services::obfuscation_stream::ObfuscatedStream;

/// Gestiona un paquete `OP_HASHSETANSWER` (`PR_ED2K` `0x52`) o `OP_HASHSETANSWER2`
/// (`PR_EMULE` `0xB2`, `ProcessHashSet(bFileIdentifiers=true)` de eMule)
/// recibido en cualquier bucle de sesión con un par: valida el hash del archivo
/// y persiste el conjunto de hashes MD4 por parte.
/// Devuelve true si el paquete era una respuesta de hashset y fue consumido.
pub(crate) async fn handle_hashset_answer(
    packet: &RawPacket,
    dl_hash_bytes: [u8; 16],
    file_hash_str: &str,
    target_addr: SocketAddr,
    decoder: &Ed2kDecoder,
    downloads: &Arc<Mutex<DownloadService>>,
) -> bool {
    // `OP_HASHSETANSWER2` es la variante FileIdentifiers (`SendHashsetPacket` en
    // `UploadClient.cpp`) — cable `PR_EMULE`, identificador compuesto + bits de
    // opciones por conjunto. eMule enruta ambos a `ProcessHashSet`.
    if packet.protocol == PR_EMULE && packet.opcode == OP_HASHSETANSWER2 {
        return match decoder.decode_hashset_answer2(
            packet.protocol,
            packet.opcode,
            &packet.payload,
            0,
        ) {
            Ok(ans2) => {
                // `CompareRelaxed` — MD4 debe coincidir; tamaño/AICH cruzados
                // cuando estén presentes en ambos lados.
                if ans2.identifier.md4_hash != Some(dl_hash_bytes) {
                    tracing::warn!(
                        "Peer {peer} sent OP_HASHSETANSWER2 for a different file hash, ignoring",
                        peer = log_peer_addr(target_addr)
                    );
                    return true;
                }
                if let Some(aich) = &ans2.aich_set {
                    let mut dl_svc = downloads.lock().await;
                    let expected_parts = dl_svc
                        .find_by_ed2k_hash(file_hash_str)
                        .map(|dl| {
                            dl.total_bytes
                                .div_ceil(mule_protocol::ed2k::ED2K_PART_SIZE as u64)
                                as usize
                        })
                        .unwrap_or(aich.part_hashes.len());
                    if expected_parts == 0 || aich.part_hashes.len() == expected_parts {
                        if let Err(e) = dl_svc.store_aich_part_hashes(file_hash_str, &aich.part_hashes) {
                            tracing::warn!(
                                "Peer {peer}: failed persisting AICH part hashes for {file_hash_str}: {e}",
                                peer = log_peer_addr(target_addr)
                            );
                        } else {
                            tracing::debug!(
                                "Peer {peer}: persisted {} AICH part hashes for {file_hash_str}",
                                aich.part_hashes.len(),
                                peer = log_peer_addr(target_addr)
                            );
                        }
                    } else {
                        tracing::warn!(
                            "Peer {peer}: AICH part count mismatch for {file_hash_str} (got {}, expected {expected_parts})",
                            aich.part_hashes.len(),
                            peer = log_peer_addr(target_addr)
                        );
                    }
                }
                match ans2.md4_set {
                    Some(ans) => store_part_hashes(ans, dl_hash_bytes, file_hash_str, target_addr, downloads).await,
                    None => true,
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Malformed OP_HASHSETANSWER2 from {peer}: {e}",
                    peer = log_peer_addr(target_addr)
                );
                true
            }
        };
    }
    if packet.protocol != PR_ED2K || packet.opcode != OP_HASHSETANSWER {
        return false;
    }

    match decoder.decode_hashset_answer(packet.protocol, packet.opcode, &packet.payload, 0) {
        Ok(ans) => {
            store_part_hashes(ans, dl_hash_bytes, file_hash_str, target_addr, downloads).await
        }
        Err(e) => {
            tracing::warn!(
                "Malformed OP_HASHSETANSWER from {peer}: {e}",
                peer = log_peer_addr(target_addr)
            );
            true
        }
    }
}

/// Gestiona `OP_QUEUERANK` (`0x5C`, PR_ED2K, `<rank u32>`) y
/// `OP_QUEUERANKING` (`0x60`, PR_EMULE, `<rank u16><10 reservados>`) — eMule
/// enruta ambos a `SetRemoteQueueRank` (`ProcessEdonkeyQueueRank` /
/// `ProcessEmuleQueueRank`, `DownloadClient.cpp:1978-2000`). Actualiza la
/// instantánea de la sesión y emite el evento de par; devuelve true cuando se consume.
pub(crate) async fn handle_queue_rank_packet(
    packet: &RawPacket,
    address_str: &str,
    target_addr: SocketAddr,
    decoder: &Ed2kDecoder,
    inner: &Arc<Mutex<SwarmInnerState>>,
    event_tx: &broadcast::Sender<String>,
    sequence: &Arc<AtomicU64>,
) -> bool {
    let rank = if packet.protocol == PR_ED2K && packet.opcode == OP_QUEUERANK {
        match decoder.decode_queue_rank(packet.protocol, packet.opcode, &packet.payload, 0) {
            Ok(qr) => qr.rank,
            Err(e) => {
                tracing::warn!(
                    "Malformed OP_QUEUERANK from {peer}: {e}",
                    peer = log_peer_addr(target_addr)
                );
                return true;
            }
        }
    } else if (packet.protocol == PR_ED2K || packet.protocol == PR_EMULE)
        && packet.opcode == OP_QUEUERANKING
    {
        match decoder.decode_queue_ranking(packet.protocol, packet.opcode, &packet.payload, 0) {
            Ok(qr) => qr.rank as u32,
            Err(e) => {
                tracing::warn!(
                    "Malformed OP_QUEUERANKING from {peer}: {e}",
                    peer = log_peer_addr(target_addr)
                );
                return true;
            }
        }
    } else {
        return false;
    };

    tracing::debug!("Queue rank from peer {}: {}", log_peer_addr(target_addr), rank);
    let queue_update = {
        let mut g = inner.lock().await;
        if let Some(session) = g.sessions.get_mut(address_str) {
            session.snapshot.queue_rank = Some(rank);
            Some(session.snapshot.clone())
        } else {
            None
        }
    };
    if let Some(snap) = queue_update {
        emit_event(event_tx, sequence, &snap);
    }
    true
}

/// Valida y persiste un conjunto de hashes MD4 de partes decodificado — cola compartida
/// de `OP_HASHSETANSWER` y `OP_HASHSETANSWER2` (porción `md4_set`).
async fn store_part_hashes(
    ans: mule_protocol::ed2k::Ed2kHashSetAnswer,
    dl_hash_bytes: [u8; 16],
    file_hash_str: &str,
    target_addr: SocketAddr,
    downloads: &Arc<Mutex<DownloadService>>,
) -> bool {
    if ans.file_hash != dl_hash_bytes {
        tracing::warn!(
            "Peer {peer} sent OP_HASHSETANSWER for a different file hash, ignoring",
            peer = log_peer_addr(target_addr)
        );
        return true;
    }
    let mut dl_svc = downloads.lock().await;
    if let Err(e) = dl_svc.store_part_hashes_by_hash(file_hash_str, &ans.part_hashes) {
        tracing::warn!(
            "Failed to persist hashset from {peer} for {file_hash_str}: {e}",
            peer = log_peer_addr(target_addr)
        );
    } else {
        info!(
            "Stored hashset ({} part hashes) for {} from peer {}",
            ans.part_hashes.len(),
            file_hash_str,
            log_peer_addr(target_addr)
        );
    }
    true
}

/// Enmascara la IP de un par como `CAICHUntrustedHash` de eMule (`dwIP &= 0x00F0FFFF`),
/// para que múltiples clientes tras el mismo rango de direcciones cuenten como un solo voto.
fn mask_peer_ip(addr: SocketAddr) -> u32 {
    match addr.ip() {
        std::net::IpAddr::V4(v4) => u32::from_le_bytes(v4.octets()) & 0x00F0FFFF,
        std::net::IpAddr::V6(v6) => {
            u32::from_le_bytes(v6.octets()[..4].try_into().unwrap_or_default()) & 0x00F0FFFF
        }
    }
}

/// Escribe un paquete eMule con entramado en el flujo del par.
async fn write_packet(
    stream: &mut ObfuscatedStream<TcpStream>,
    pkt: &RawPacket,
) -> std::io::Result<()> {
    let mut out_wire = Vec::with_capacity(pkt.payload.len() + 6);
    out_wire.push(pkt.protocol);
    out_wire.extend_from_slice(&(pkt.frame_len() as u32).to_le_bytes());
    out_wire.push(pkt.opcode);
    out_wire.extend_from_slice(&pkt.payload);
    stream.write_all(&out_wire).await
}

/// Envía `OP_AICHREQUEST` para cada parte dañada que espera recuperación AICH en
/// este archivo (`CDownloadClient::SendAICHRequest` de eMule). Sin operación cuando el par
/// no anuncia AICH o aún no se confía en ninguna raíz.
pub(crate) async fn send_pending_aich_requests(
    stream: &mut ObfuscatedStream<TcpStream>,
    encoder: &Ed2kEncoder,
    target_addr: SocketAddr,
    file_hash_str: &str,
    dl_hash_bytes: [u8; 16],
    peer_supports_aich: bool,
    downloads: &Arc<Mutex<DownloadService>>,
) {
    if !peer_supports_aich {
        return;
    }
    loop {
        let next = {
            let mut dl_svc = downloads.lock().await;
            dl_svc.next_aich_recovery_by_hash(file_hash_str)
        };
        let Some((part, master)) = next else { break };
        let req = Ed2kAichRequest {
            file_hash: dl_hash_bytes,
            part: part as u16,
            master_hash: master,
        };
        match encoder.encode_aich_request(&req) {
            Ok(pkt) => {
                if let Err(e) = write_packet(stream, &pkt).await {
                    tracing::warn!(
                        "Failed to send OP_AICHREQUEST to {peer}: {e}",
                        peer = log_peer_addr(target_addr)
                    );
                    let mut dl_svc = downloads.lock().await;
                    if let Some(snap) = dl_svc.find_by_ed2k_hash(file_hash_str) {
                        dl_svc.fail_aich_recovery(&snap.id, part);
                    }
                    break;
                }
                tracing::info!(
                    "Sent OP_AICHREQUEST for part {part} of {file_hash_str} to {peer}",
                    peer = log_peer_addr(target_addr)
                );
            }
            Err(e) => {
                tracing::warn!("Failed to encode OP_AICHREQUEST: {e}");
                break;
            }
        }
    }
}

/// Gestiona `OP_AICHFILEHASHANS` (0x9D): registra un voto para el hash maestro. Una vez
/// que la raíz alcanza consenso, se envían las peticiones de recuperación pendientes a este par.
/// Devuelve true si el paquete fue consumido.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_aich_file_hash_res(
    stream: &mut ObfuscatedStream<TcpStream>,
    packet: &RawPacket,
    dl_hash_bytes: [u8; 16],
    file_hash_str: &str,
    target_addr: SocketAddr,
    decoder: &Ed2kDecoder,
    encoder: &Ed2kEncoder,
    peer_supports_aich: bool,
    downloads: &Arc<Mutex<DownloadService>>,
) -> bool {
    if packet.protocol != PR_EMULE || packet.opcode != OP_AICHFILEHASHRES {
        return false;
    }
    match decoder.decode_aich_file_hash_res(packet.protocol, packet.opcode, &packet.payload, 0) {
        Ok(res) => {
            if res.file_hash != dl_hash_bytes {
                tracing::warn!(
                    "Peer {peer} sent OP_AICHFILEHASHANS for a different file hash, ignoring",
                    peer = log_peer_addr(target_addr)
                );
                return true;
            }
            let outcome = {
                let mut dl_svc = downloads.lock().await;
                dl_svc.find_by_ed2k_hash(file_hash_str).map(|snap| {
                    dl_svc.record_aich_root_vote(&snap.id, res.aich_hash, mask_peer_ip(target_addr))
                })
            };
            match outcome {
                Some(crate::services::downloads::operations::AichVoteOutcome::Trusted) => {
                    tracing::info!(
                        "AICH master hash for {file_hash_str} confirmed by {peer}",
                        peer = log_peer_addr(target_addr)
                    );
                    send_pending_aich_requests(
                        stream,
                        encoder,
                        target_addr,
                        file_hash_str,
                        dl_hash_bytes,
                        peer_supports_aich,
                        downloads,
                    )
                    .await;
                }
                Some(crate::services::downloads::operations::AichVoteOutcome::Mismatch) => {
                    tracing::warn!(
                        "Peer {peer} reported a different AICH master hash for {file_hash_str}",
                        peer = log_peer_addr(target_addr)
                    );
                }
                Some(crate::services::downloads::operations::AichVoteOutcome::Untrusted) => {
                    tracing::debug!(
                        "AICH root vote from {peer} recorded for {file_hash_str} (no consensus yet)"
                    , peer = log_peer_addr(target_addr));
                }
                None => {}
            }
            true
        }
        Err(e) => {
            tracing::warn!(
                "Malformed OP_AICHFILEHASHANS from {peer}: {e}",
                peer = log_peer_addr(target_addr)
            );
            true
        }
    }
}

/// Gestiona `OP_AICHANSWER` (0x9C): aplica datos de recuperación a la parte dañada
/// (desalojo a nivel de bloque) o reencola la parte ante una respuesta fallida.
/// Devuelve true si el paquete fue consumido.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_aich_answer(
    stream: &mut ObfuscatedStream<TcpStream>,
    packet: &RawPacket,
    dl_hash_bytes: [u8; 16],
    file_hash_str: &str,
    target_addr: SocketAddr,
    decoder: &Ed2kDecoder,
    encoder: &Ed2kEncoder,
    peer_supports_aich: bool,
    downloads: &Arc<Mutex<DownloadService>>,
) -> bool {
    if packet.protocol != PR_EMULE || packet.opcode != OP_AICH_ANSWER {
        return false;
    }
    match decoder.decode_aich_answer(packet.protocol, packet.opcode, &packet.payload, 0) {
        Ok(ans) => {
            if ans.file_hash != dl_hash_bytes {
                tracing::warn!(
                    "Peer {peer} sent OP_AICHANSWER for a different file hash, ignoring",
                    peer = log_peer_addr(target_addr)
                );
                return true;
            }
            let mut dl_svc = downloads.lock().await;
            let dl_id = dl_svc.find_by_ed2k_hash(file_hash_str).map(|s| s.id);
            match (dl_id, ans.recovery) {
                (Some(id), Some(rec)) => {
                    let entries: Vec<mule_format::AichRecoveryEntry> = rec
                        .entries16
                        .iter()
                        .map(|&(ident, hash)| mule_format::AichRecoveryEntry {
                            ident: ident as u32,
                            hash: mule_format::AichHash(hash),
                        })
                        .chain(rec.entries32.iter().map(|&(ident, hash)| {
                            mule_format::AichRecoveryEntry {
                                ident,
                                hash: mule_format::AichHash(hash),
                            }
                        }))
                        .collect();
                    match dl_svc.apply_aich_recovery(&id, rec.part as u32, rec.master_hash, entries)
                    {
                        Ok(report) => {
                            if report.corrupted_blocks == 0 {
                                tracing::debug!(
                                    "AICH recovery from {peer}: part {} of {file_hash_str} fully verified",
                                    report.part
                                , peer = log_peer_addr(target_addr));
                            } else {
                                tracing::debug!(
                                    "AICH recovery from {peer}: part {} of {file_hash_str}, {} corrupted block(s) evicted",
                                    report.part, report.corrupted_blocks
                                , peer = log_peer_addr(target_addr));
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "AICH recovery from {peer} rejected: {e}",
                                peer = log_peer_addr(target_addr)
                            );
                        }
                    }
                }
                (Some(id), None) => {
                    tracing::warn!(
                        "Peer {peer} could not serve AICH recovery for {file_hash_str}",
                        peer = log_peer_addr(target_addr)
                    );
                    // El índice de parte no se incluye en una respuesta fallida; se marca
                    // como fallida la petición enviada más recientemente.
                    dl_svc.fail_last_aich_request(&id);
                }
                (None, _) => {}
            }
            drop(dl_svc);
            send_pending_aich_requests(
                stream,
                encoder,
                target_addr,
                file_hash_str,
                dl_hash_bytes,
                peer_supports_aich,
                downloads,
            )
            .await;
            true
        }
        Err(e) => {
            tracing::warn!(
                "Malformed OP_AICHANSWER from {peer}: {e}",
                peer = log_peer_addr(target_addr)
            );
            true
        }
    }
}

pub(crate) async fn query_file_status_and_enter_queue(
    stream: &mut ObfuscatedStream<TcpStream>,
    target_addr: SocketAddr,
    file_hash_str: &str,
    dl_hash_bytes: [u8; 16],
    address_str: &str,
    sx_caps: &PeerSxCaps,
    peer_supports_aich: bool,
    peer_ext_req: u8,
    timeouts: &PeerTimeouts,
    cancel_token: &mule_network_policy::CancellationToken,
    inner: &Arc<Mutex<SwarmInnerState>>,
    event_tx: &broadcast::Sender<String>,
    sequence: &Arc<AtomicU64>,
    downloads: &Arc<Mutex<DownloadService>>,
    sx_env: &SxEnv,
    sx_local: &SxLocalIds,
    sui: &mut super::sui::OutboundSui,
) -> Result<Ed2kFileStatus, PeerError> {
    let encoder = Ed2kEncoder::default();
    // ExtendedRequestsVersion negociada en el apretón de manos (OP_EMULEINFO) — controla el
    // análisis determinista de la ext-info `0x58` en los MULTIPACKET*.
    let decoder = Ed2kDecoder::default().with_extended_requests_version(peer_ext_req);
    let codec = PacketCodec::new();
    let mut out_wire = Vec::with_capacity(64);
    let mut read_buf = BytesMut::with_capacity(65536);
    let mut raw_buf = [0u8; 8192];

    info!(
        "Peer {} handshake completed. Requesting file status for {file_hash_str}...",
        log_peer_addr(target_addr)
    );

    // 4a-4c. `SendFileRequest` de eMule (`DownloadClient.cpp:257-430`) — un
    // único `OP_MULTIPACKET*` cuando el par lo negoció, si no el paquete
    // fragmentado REQFILENAME(+extinfo) → SETREQFILEID → REQUESTSOURCES →
    // AICHFILEHASHREQ. `IsSourceRequestAllowed` se evalúa una vez en
    // `gather` para decidir si la subpetición SX se incrusta o emite.
    let req_inputs = super::file_request::gather_file_request_inputs(
        downloads,
        sx_env,
        sx_caps,
        target_addr,
        file_hash_str,
        dl_hash_bytes,
        peer_ext_req,
        peer_supports_aich,
    )
    .await;
    let file_part_count = req_inputs.part_count;
    let file_total_bytes = req_inputs.file_size;
    let file_aich_root = req_inputs.aich_root;
    super::file_request::send_file_request_packets(
        stream,
        &encoder,
        sx_env,
        target_addr,
        &req_inputs,
    )
    .await?;

    // 4b''. AICH: la solicitud de raíz `OP_AICHFILEHASHREQ` ya forma
    // parte del paquete `SendFileRequest` (subopcode multipaquete o
    // paquete separado). Quedan las OP_AICHREQUEST pendientes (datos de recuperación
    // de partes dañadas) — emitidas cuando el par anuncia AICH.
    if peer_supports_aich {
        send_pending_aich_requests(
            stream,
            &encoder,
            target_addr,
            file_hash_str,
            dl_hash_bytes,
            peer_supports_aich,
            downloads,
        )
        .await;
    }

    let queued_snapshot = {
        let mut g = inner.lock().await;
        if let Some(session) = g.sessions.get_mut(address_str) {
            session.snapshot.status = PeerConnectionStatus::Queued;
            Some(session.snapshot.clone())
        } else {
            None
        }
    };
    if let Some(snap) = queued_snapshot {
        emit_event(event_tx, sequence, &snap);
    }
    // Inicializa el plazo de repetición de petición desde la puesta en cola (incluso si el socket se cierra
    // antes o durante la espera del estado, para que peer_session conserve queued_deadline).
    publish_next_reask(inner, address_str, tokio::time::Instant::now() + FILE_REASK_INTERVAL).await;

    // 5. Esperar OP_FILESTATUS u OP_FILEREQANSNOFILE
    let status_deadline = tokio::time::Instant::now() + timeouts.status_timeout;
    let mut file_status_opt = None;
    let mut accepted_directly = false;

    while file_status_opt.is_none() {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                return Err(PeerError::new(PeerFailureReason::KillSwitchAborted, "Connection aborted by kill switch"));
            }
            _ = tokio::time::sleep_until(status_deadline) => {
                return Err(PeerError::new(PeerFailureReason::TimeoutStatus, "Timed out responding to OP_FILESTATUSREQ"));
            }
            read_res = stream.read(&mut raw_buf) => {
                match read_res {
                    Ok(0) => return Err(PeerError::new(PeerFailureReason::ConnectRefused, "Peer closed connection while awaiting file status")),
                    Ok(n) => {
                        read_buf.extend_from_slice(&raw_buf[..n]);
                        if read_buf.len() > MAX_PEER_READ_BUFFER_SIZE {
                            return Err(PeerError::new(PeerFailureReason::CorruptedData, "Peer read buffer exceeded limit awaiting file status"));
                        }
                        // `ListenSocket.cpp:1071` de eMule: una respuesta
                        // `OP_MULTIPACKETANSWER*` se desglosa en
                        // subrespuestas reinyectadas en este mismo bucle
                        // (el hash del envoltorio se reinserta en cada
                        // carga útil — el despacho independiente se aplica a ellas).
                        let mut expanded: std::collections::VecDeque<RawPacket> =
                            std::collections::VecDeque::new();
                        loop {
                            if expanded.is_empty() && file_status_opt.is_some() {
                                break;
                            }
                            let packet = if let Some(p) = expanded.pop_front() {
                                p
                            } else {
                                match codec.decode(&mut read_buf) {
                                    // OP_PACKEDPROT (0xD4) — eMule descomprime
                                    // en línea; el paquete descomprimido se
                                    // despacha como EMULEPROT. Las cargas útiles
                                    // comprimidas corruptas se descartan.
                                    Ok(Some(p)) => match mule_protocol::ed2k::packed::unpack_owned(
                                        p,
                                        mule_protocol::ed2k::packed::PACKED_PEER_MAX_DECOMPRESSED,
                                    ) {
                                        Some(u) => u,
                                        None => continue,
                                    },
                                    _ => break,
                                }
                            };
                            if let Some(subs) =
                                super::file_request::expand_multipacket_answer(&packet)
                            {
                                expanded.extend(subs);
                                continue;
                            }
                            if sui.handle(&packet, target_addr, &decoder, &encoder, stream).await {
                                continue;
                            }
                            if handle_hashset_answer(&packet, dl_hash_bytes, file_hash_str, target_addr, &decoder, downloads).await {
                                continue;
                            }
                            if handle_aich_file_hash_res(stream, &packet, dl_hash_bytes, file_hash_str, target_addr, &decoder, &encoder, peer_supports_aich, downloads).await {
                                continue;
                            }
                            if handle_aich_answer(stream, &packet, dl_hash_bytes, file_hash_str, target_addr, &decoder, &encoder, peer_supports_aich, downloads).await {
                                continue;
                            }
                            if packet.protocol == PR_ED2K || packet.protocol == PR_EMULE {
                                if packet.opcode == OP_FILESTATUS {
                                    let status_res = decoder.decode_file_status(packet.protocol, packet.opcode, &packet.payload, 0);
                                    match status_res {
                                        Ok(status) => {
                                            tracing::info!(
                                                "Peer {peer}: received OP_FILESTATUS for {file_hash_str} (part_count={}, complete={})",
                                                status.part_count,
                                                status.is_complete(),
                                                peer = log_peer_addr(target_addr)
                                            );
                                            if status.file_hash != dl_hash_bytes {
                                                return Err(PeerError::new(
                                                    PeerFailureReason::HashMismatch,
                                                    "OP_FILESTATUS returned unexpected file hash".to_string(),
                                                ));
                                            }
                                            // Propagar el mapa de bits de partes del par al almacén
                                            // del enjambre (`act.sources`) — alimenta
                                            // `available_source_count` (`m_SrcPartFrequency` /
                                            // `DrawStatusBar` de eMule).
                                            // `part_count == 0` = fuente completa.
                                            let mut dl_svc = downloads.lock().await;
                                            if let Some(dl_id) = dl_svc
                                                .find_by_ed2k_hash(file_hash_str)
                                                .map(|s| s.id)
                                            {
                                                if let Ok(snap) = dl_svc.record_source_availability(
                                                    &dl_id,
                                                    target_addr,
                                                    status.availability_mask.clone(),
                                                ) {
                                                    emit_download_update(event_tx, sequence, &snap);
                                                }
                                            }
                                            let mask = status.availability_mask.raw_bytes();
                                            if !mask.is_empty() {
                                                let _ = dl_svc.update_peer_source_details(
                                                    file_hash_str,
                                                    address_str,
                                                    None,
                                                    Some(mask),
                                                );
                                            }
                                            file_status_opt = Some(status);
                                            if expanded.is_empty() {
                                                break;
                                            }
                                        }
                                        Err(e) => return Err(PeerError::new(PeerFailureReason::CorruptedData, format!("Malformed OP_FILESTATUS: {e}"))),
                                    }
                                } else if packet.opcode == OP_REQFILENAME_ANSWER {
                                    // Archivo ≤ 1 parte: `OP_SETREQFILEID`
                                    // no fue enviado — la respuesta al nombre
                                    // equivale al estado «fuente completa» (eMule
                                    // `DownloadClient.cpp:362-365`).
                                    if file_part_count == 1
                                        && packet.payload.len() >= 16
                                        && packet.payload[..16] == dl_hash_bytes
                                    {
                                        file_status_opt = Some(mule_protocol::ed2k::Ed2kFileStatus {
                                            file_hash: dl_hash_bytes,
                                            part_count: 0,
                                            availability_mask: mule_protocol::ed2k::PartAvailabilityMask::from_parts(&[], 0),
                                        });
                                        if expanded.is_empty() {
                                            break;
                                        }
                                        continue;
                                    }
                                    tracing::debug!("Peer {peer} sent OP_REQFILENAME_ANSWER (len={})", packet.payload.len(), peer = log_peer_addr(target_addr));
                                } else if handle_queue_rank_packet(&packet, address_str, target_addr, &decoder, inner, event_tx, sequence).await {
                                    file_status_opt = Some(mule_protocol::ed2k::Ed2kFileStatus {
                                        file_hash: dl_hash_bytes,
                                        part_count: 0,
                                        availability_mask: mule_protocol::ed2k::PartAvailabilityMask::from_parts(&[], 0),
                                    });
                                    if expanded.is_empty() {
                                        break;
                                    }
                                } else if packet.opcode == OP_ACCEPTUPLOADREQ {
                                    tracing::info!("Peer {peer} directly accepted upload slot (OP_ACCEPTUPLOADREQ) during status query!", peer = log_peer_addr(target_addr));
                                    accepted_directly = true;
                                    file_status_opt = Some(mule_protocol::ed2k::Ed2kFileStatus {
                                        file_hash: dl_hash_bytes,
                                        part_count: 0,
                                        availability_mask: mule_protocol::ed2k::PartAvailabilityMask::from_parts(&[], 0),
                                    });
                                    if expanded.is_empty() {
                                        break;
                                    }
                                } else if packet.opcode == OP_FILEREQANSNOFILE {
                                    return Err(PeerError::new(PeerFailureReason::FileNotFound, "Remote peer does not share this file (OP_FILEREQANSNOFILE)"));
                                } else if packet.protocol == PR_ED2K && packet.opcode == OP_CHANGE_CLIENT_ID {
                                    // `ListenSocket.cpp:590` de eMule — el
                                    // par anuncia su nuevo ID/servidor
                                    // (válido en ambos sentidos).
                                    super::change_client_id::apply_change_client_id(
                                        &packet.payload,
                                        address_str,
                                        &[file_hash_str.to_string()],
                                        downloads,
                                    )
                                    .await;
                                } else if (packet.protocol == PR_EMULE)
                                    && (packet.opcode == OP_ANSWERSOURCES || packet.opcode == OP_ANSWERSOURCES2)
                                {
                                    handle_sx_answer(&packet, dl_hash_bytes, file_hash_str, target_addr, &decoder, downloads, sx_env, sx_local).await;
                                } else if packet.protocol == PR_EMULE
                                    && (packet.opcode == OP_REQUESTSOURCES || packet.opcode == OP_REQUESTSOURCES2)
                                {
                                    handle_sx_request(&packet, Some(dl_hash_bytes), target_addr, &decoder, &encoder, downloads, stream, sx_env, sx_caps, None).await;
                                } else if packet.protocol == PR_EMULE
                                {
                                    super::source_exchange::handle_sx_multipacket(
                                        &packet,
                                        dl_hash_bytes,
                                        None,
                                        target_addr,
                                        &encoder,
                                        downloads,
                                        stream,
                                        sx_env,
                                        sx_caps,
                                        peer_ext_req,
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                    Err(e) => return Err(PeerError::new(PeerFailureReason::ConnectRefused, format!("Read error awaiting file status: {e}"))),
                }
            }
        }
    }

    let Some(file_status) = file_status_opt else {
        return Err(PeerError::new(
            PeerFailureReason::TimeoutStatus,
            "File status missing after status query loop",
        ));
    };

    // "No Needed Parts" de eMule: si el par anuncia una máscara de partes pero ninguna
    // de sus partes falta localmente, la fuente es inútil para esta descarga —
    // se descarta de inmediato en lugar de ocupar una ranura de subida. Las fuentes
    // completas (part_count == 0) y sesiones aceptadas en cola omiten la comprobación.
    if !file_status.is_complete() && file_status.part_count > 0 {
        let has_needed_part = {
            let dl_svc = downloads.lock().await;
            let completed = dl_svc.get_completed_ranges_by_hash(file_hash_str);
            let total_size = dl_svc
                .find_by_ed2k_hash(file_hash_str)
                .map(|d| d.total_bytes)
                .unwrap_or(0);
            let part_size = mule_protocol::ed2k::ED2K_PART_SIZE as u64;
            let total_parts = if total_size == 0 {
                0
            } else {
                total_size.div_ceil(part_size).max(1) as u32
            };
            (0..total_parts).any(|p| {
                u16::try_from(p).is_ok_and(|pi| file_status.availability_mask.has_part(pi)) && {
                    let start = p as u64 * part_size;
                    let end = ((p as u64 + 1) * part_size).min(total_size);
                    completed.iter().fold(start, |acc, &(s, e)| {
                        if s <= acc && e > acc {
                            acc.max(e.min(end))
                        } else {
                            acc
                        }
                    }) < end
                }
            })
        };
        if !has_needed_part {
            return Err(PeerError::new(
                PeerFailureReason::NoNeededParts,
                "Peer has the file but no needed parts (eMule NNS)",
            ));
        }
    }

    // Marcar la fuente como Conectada
    {
        let mut dl_svc = downloads.lock().await;
        let _ = dl_svc.update_peer_source_status(
            file_hash_str,
            address_str,
            mule_core::model::PeerSourceStatus::Connected,
            None,
            None,
        );
    }

    // 5b. Solicitar el conjunto de hashes MD4 por partes una vez confirmado el estado del archivo (eMule
    // `DownloadClient.cpp:2200-2244`). Solo se solicita cuando se desconoce y el archivo tiene múltiples partes.
    let hashset_known = {
        let dl_svc = downloads.lock().await;
        !dl_svc.get_part_hashes_by_hash(file_hash_str).is_empty()
    };
    if !hashset_known && (file_part_count > 1 || file_total_bytes > mule_protocol::ed2k::ED2K_PART_SIZE as u64) {
        let hs_pkt = if sx_caps.file_identifiers {
            let mut identifier =
                mule_protocol::ed2k::Ed2kFileIdentifier::new(dl_hash_bytes, file_total_bytes);
            identifier.aich_hash = file_aich_root;
            encoder.encode_hashset_request2(&Ed2kHashSetReq2 {
                identifier,
                want_md4: true,
                want_aich: false,
            })
        } else {
            encoder.encode_hashset_request(&Ed2kHashSetReq {
                file_hash: dl_hash_bytes,
            })
        };
        match hs_pkt {
            Ok(hs_pkt) => {
                out_wire.clear();
                out_wire.push(hs_pkt.protocol);
                out_wire.extend_from_slice(&(hs_pkt.frame_len() as u32).to_le_bytes());
                out_wire.push(hs_pkt.opcode);
                out_wire.extend_from_slice(&hs_pkt.payload);
                if let Err(e) = stream.write_all(&out_wire).await {
                    tracing::warn!(
                        "Failed to send OP_HASHSETREQUEST to {peer}: {e}",
                        peer = log_peer_addr(target_addr)
                    );
                } else {
                    tracing::info!(
                        "Sent OP_HASHSETREQUEST (v2={}) to peer {peer} for {file_hash_str}",
                        sx_caps.file_identifiers,
                        peer = log_peer_addr(target_addr)
                    );
                }
            }
            Err(e) => {
                tracing::warn!("Failed to encode OP_HASHSETREQUEST: {e}");
            }
        }
    }

    // 6. Enviar OP_STARTUPLOADREQ
    send_start_upload_req(stream, dl_hash_bytes, file_hash_str, target_addr).await?;

    // Preservar cualquier paquete restante en `read_buf` antes de pasar a la fase de cola/transferencia
    stream.put_back_unread(&read_buf);
    if accepted_directly {
        return Ok(file_status);
    }

    // 7. Esperar en la cola de subida (DS_ONQUEUE de eMule + repeticiones de petición FILEREASKTIME)
    wait_in_queue(
        stream,
        target_addr,
        file_hash_str,
        dl_hash_bytes,
        address_str,
        sx_caps,
        timeouts,
        cancel_token,
        inner,
        event_tx,
        sequence,
        downloads,
        sx_env,
        sx_local,
        sui,
        peer_ext_req,
        peer_supports_aich,
        file_status,
        false,
    )
    .await
}

/// `OP_STARTUPLOADREQ` de eMule (carga útil de hash) — (re)entra en la cola
/// de subida remota para `file_hash`. Se repite en cada repetición de petición `FILEREASKTIME`
/// para que el `LastUpRequest` remoto permanezca fresco (el horizonte de purga es de 1 hora).
pub(crate) async fn send_start_upload_req(
    stream: &mut ObfuscatedStream<TcpStream>,
    dl_hash_bytes: [u8; 16],
    file_hash_str: &str,
    target_addr: SocketAddr,
) -> Result<(), PeerError> {
    info!(
        "Requesting upload slot via OP_STARTUPLOADREQ for file {} from peer {}",
        file_hash_str,
        log_peer_addr(target_addr)
    );
    let mut out_wire = Vec::with_capacity(23);
    out_wire.push(PR_ED2K);
    out_wire.extend_from_slice(&(17u32).to_le_bytes());
    out_wire.push(OP_STARTUPLOADREQ);
    out_wire.extend_from_slice(&dl_hash_bytes);
    stream.write_all(&out_wire).await.map_err(|e| {
        PeerError::new(
            PeerFailureReason::ConnectRefused,
            format!("Failed to send OP_STARTUPLOADREQ: {e}"),
        )
    })
}

/// Cadencia de repetición de petición de archivo de eMule: mientras está en cola, el cliente reenvía el paquete de petición de archivo
/// cada ~10 minutos. Una cadencia más corta de 10 min asegura la persistencia en cola a la vez que permite
/// al mecanismo de re-ask por UDP actualizar las posiciones con prontitud sin quedar bloqueado por tiempos de espera excesivos.
pub(crate) const FILE_REASK_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Publica el siguiente plazo de repetición de petición TCP en la instantánea de la sesión —
/// equivalente a `GetTimeUntilReask()` de eMule (`PartFile.cpp:2291`). El coordinador
/// del enjambre lo lee para disparar la repetición de petición UDP dentro de la
/// ventana `(1 s, 2 min)` anterior a la repetición TCP. Solo contabilidad: no se
/// emite ningún evento de sesión (valor interno de planificación).
async fn publish_next_reask(
    inner: &Arc<Mutex<SwarmInnerState>>,
    address_str: &str,
    next_reask: tokio::time::Instant,
) {
    let remaining = next_reask.saturating_duration_since(tokio::time::Instant::now());
    let deadline_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
        + remaining.as_millis() as i64;
    let mut g = inner.lock().await;
    // `m_bReaskPending` sobrevive a la actualización del plazo: una actualización
    // no consume la bandera (solo una conexión entrante lo hace,
    // `BaseClient.cpp:1552`).
    let reask_pending = g
        .queued_peers
        .get(address_str)
        .map(|q| q.reask_pending)
        .unwrap_or(false);
    if let Some(session) = g.sessions.get_mut(address_str) {
        session.snapshot.next_tcp_reask_unix_ms = Some(deadline_unix_ms);
        let queued = session.snapshot.file_hash.clone().map(|file_hash| {
            crate::services::peer_session::types::QueuedPeerState {
                file_hash,
                connected_at_unix_ms: session.snapshot.connected_at_unix_ms,
                next_tcp_reask_unix_ms: Some(deadline_unix_ms),
                reask_pending,
            }
        });
        if let Some(state) = queued {
            // `DS_ONQUEUE` de eMule es un estado lógico que sobrevive al socket —
            // se registra para que la repetición de petición UDP aún pueda dispararse tras
            // que `CheckTimeOut` cierre el socket inactivo (`DownloadClient.cpp:1326-1344`).
            g.queued_peers.insert(address_str.to_string(), state);
        }
    }
}

/// Espera `OP_ACCEPTUPLOADREQ` en la cola de subida remota — `DS_ONQUEUE` de
/// eMule. Compartido entre la consulta de estado inicial y la ruta de
/// recuperación por atasco (`CheckDownloadTimeout` → `SendCancelTransfer` → regreso
/// a la cola en el mismo socket).
///
/// `requeue_now` reenvía `OP_SETREQFILEID` + `OP_STARTUPLOADREQ` inmediatamente
/// (reentrada tras atasco); la ruta inicial ya los envió antes de llamar.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn wait_in_queue(
    stream: &mut ObfuscatedStream<TcpStream>,
    target_addr: SocketAddr,
    file_hash_str: &str,
    dl_hash_bytes: [u8; 16],
    address_str: &str,
    sx_caps: &PeerSxCaps,
    timeouts: &PeerTimeouts,
    cancel_token: &mule_network_policy::CancellationToken,
    inner: &Arc<Mutex<SwarmInnerState>>,
    event_tx: &broadcast::Sender<String>,
    sequence: &Arc<AtomicU64>,
    downloads: &Arc<Mutex<DownloadService>>,
    sx_env: &SxEnv,
    sx_local: &SxLocalIds,
    sui: &mut super::sui::OutboundSui,
    peer_ext_req: u8,
    peer_supports_aich: bool,
    mut file_status: Ed2kFileStatus,
    requeue_now: bool,
) -> Result<Ed2kFileStatus, PeerError> {
    let encoder = Ed2kEncoder::default();
    let decoder = Ed2kDecoder::default().with_extended_requests_version(peer_ext_req);
    let codec = PacketCodec::new();
    let mut read_buf = BytesMut::with_capacity(65536);
    let mut raw_buf = [0u8; 8192];

    if requeue_now {
        // Reentrada en cola (post-atasco): eMule reproduce `SendFileRequest`
        // completo y luego `SendStartupLoadReq`.
        let req_inputs = super::file_request::gather_file_request_inputs(
            downloads,
            sx_env,
            sx_caps,
            target_addr,
            file_hash_str,
            dl_hash_bytes,
            peer_ext_req,
            peer_supports_aich,
        )
        .await;
        super::file_request::send_file_request_packets(
            stream,
            &encoder,
            sx_env,
            target_addr,
            &req_inputs,
        )
        .await?;
        send_start_upload_req(stream, dl_hash_bytes, file_hash_str, target_addr).await?;
    }

    // Ciclo de repetición de petición de eMule: reenviar el paquete de peticiones cada FILE_REASK_INTERVAL;
    // el SOCKET se cierra tras `queue_timeout` (= CONNECTION_TIMEOUT,
    // 40 s) sin ningún envío o recepción — `CClientReqSocket::CheckTimeOut`
    // (`ListenSocket.cpp:120-155`, `ResetTimeOutTimer` en `OnSend`/`OnReceive`/
    // `SendPacket`). El estado en cola en sí sobrevive en `queued_peers`
    // (`DS_ONQUEUE` con `socket == NULL`).
    let mut next_reask = tokio::time::Instant::now() + FILE_REASK_INTERVAL;
    publish_next_reask(inner, address_str, next_reask).await;
    let mut activity_deadline = tokio::time::Instant::now() + timeouts.queue_timeout;

    'queue_loop: loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                return Err(PeerError::new(PeerFailureReason::KillSwitchAborted, "Connection aborted by kill switch while in queue"));
            }
            _ = tokio::time::sleep_until(activity_deadline) => {
                return Err(PeerError::new(PeerFailureReason::TimeoutStatus, "Queue socket idle (eMule CONNECTION_TIMEOUT)"));
            }
            _ = tokio::time::sleep_until(next_reask) => {
                next_reask = tokio::time::Instant::now() + FILE_REASK_INTERVAL;
                publish_next_reask(inner, address_str, next_reask).await;
                // Repetición de petición `SendFileRequest` de eMule: el paquete completo
                // (REQFILENAME + SETREQFILEID + REQUESTSOURCES + AICHREQ,
                // agrupado en OP_MULTIPACKET* si se negoció) y luego
                // `SendStartupLoadReq`, que refresca `LastUpRequest` en
                // la cola remota (purga a 1 h). `complete_source`: el
                // par es fuente completa cuando su estado de partes cubre
                // todo el archivo — eMule no le solicita fuentes.
                let reask_caps = PeerSxCaps {
                    complete_source: file_status.is_complete(),
                    ..*sx_caps
                };
                let req_inputs = super::file_request::gather_file_request_inputs(
                    downloads,
                    sx_env,
                    &reask_caps,
                    target_addr,
                    file_hash_str,
                    dl_hash_bytes,
                    peer_ext_req,
                    peer_supports_aich,
                )
                .await;
                super::file_request::send_file_request_packets(
                    stream,
                    &encoder,
                    sx_env,
                    target_addr,
                    &req_inputs,
                )
                .await?;
                send_start_upload_req(stream, dl_hash_bytes, file_hash_str, target_addr).await?;
                // `SendPacket` reinicia `timeout_timer` en el lado de eMule.
                activity_deadline = tokio::time::Instant::now() + timeouts.queue_timeout;
            }
            read_res = stream.read(&mut raw_buf) => {
                match read_res {
                    Ok(0) => {
                        let rank = inner.lock().await.sessions.get(address_str).and_then(|s| s.snapshot.queue_rank);
                        if let Some(r) = rank {
                            info!("Peer {peer} closed connection while we are in queue (rank {r}). Scheduled re-ask in 15 minutes.", peer = log_peer_addr(target_addr));
                            return Err(PeerError::new(PeerFailureReason::ConnectRefused, "Disconnected in queue"));
                        } else {
                            return Err(PeerError::new(PeerFailureReason::ConnectRefused, "Peer closed connection while in queue"));
                        }
                    }
                    Ok(n) => {
                        // `OnReceive` reinicia `timeout_timer` en el lado de eMule.
                        activity_deadline = tokio::time::Instant::now() + timeouts.queue_timeout;
                        read_buf.extend_from_slice(&raw_buf[..n]);
                        if read_buf.len() > MAX_PEER_READ_BUFFER_SIZE {
                            return Err(PeerError::new(PeerFailureReason::CorruptedData, "Peer read buffer exceeded limit while in queue"));
                        }
                        // Respuestas `OP_MULTIPACKETANSWER*` desglosadas en
                        // subpaquetes — mismo mecanismo que en la fase de
                        // petición inicial (`ListenSocket.cpp:1071`).
                        let mut expanded: std::collections::VecDeque<RawPacket> =
                            std::collections::VecDeque::new();
                        loop {
                            let packet = if let Some(p) = expanded.pop_front() {
                                p
                            } else {
                                match codec.decode(&mut read_buf) {
                                    // OP_PACKEDPROT (0xD4) — cf. el bucle
                                    // de transferencia: desempaquetar + reetiquetar
                                    // EMULEPROT; cargas útiles corruptas
                                    // descartadas.
                                    Ok(Some(p)) => match mule_protocol::ed2k::packed::unpack_owned(
                                        p,
                                        mule_protocol::ed2k::packed::PACKED_PEER_MAX_DECOMPRESSED,
                                    ) {
                                        Some(u) => u,
                                        None => continue,
                                    },
                                    _ => break,
                                }
                            };
                            if let Some(subs) =
                                super::file_request::expand_multipacket_answer(&packet)
                            {
                                expanded.extend(subs);
                                continue;
                            }
                            if sui.handle(&packet, target_addr, &decoder, &encoder, stream).await {
                                continue;
                            }
                            if handle_hashset_answer(&packet, dl_hash_bytes, file_hash_str, target_addr, &decoder, downloads).await {
                                continue;
                            }
                            if handle_aich_file_hash_res(stream, &packet, dl_hash_bytes, file_hash_str, target_addr, &decoder, &encoder, peer_supports_aich, downloads).await {
                                continue;
                            }
                            if handle_aich_answer(stream, &packet, dl_hash_bytes, file_hash_str, target_addr, &decoder, &encoder, peer_supports_aich, downloads).await {
                                continue;
                            }
                            if packet.protocol == PR_ED2K && packet.opcode == OP_ACCEPTUPLOADREQ {
                                info!("Upload slot accepted (OP_ACCEPTUPLOADREQ) by peer {}", log_peer_addr(target_addr));
                                break 'queue_loop;
                            } else if handle_queue_rank_packet(&packet, address_str, target_addr, &decoder, inner, event_tx, sequence).await {
                            } else if packet.protocol == PR_ED2K && packet.opcode == OP_FILESTATUS {
                                // Keep-alive de estado de partes: actualiza el mapa de bits
                                // del par (y `last_seen`) — la barra de partes
                                // refleja así la disponibilidad actual de la cola.
                                if let Ok(status) = decoder.decode_file_status(
                                    packet.protocol,
                                    packet.opcode,
                                    &packet.payload,
                                    0,
                                ) {
                                    if status.file_hash == dl_hash_bytes {
                                        // ProcessFileStatus de eMule: el mapa de bits
                                        // del par se reemplaza — el secuenciador
                                        // recalcula la disponibilidad al reanudar.
                                        file_status = status.clone();
                                        let mut dl_svc = downloads.lock().await;
                                        if let Some(dl_id) = dl_svc
                                            .find_by_ed2k_hash(file_hash_str)
                                            .map(|s| s.id)
                                        {
                                            if let Ok(snap) = dl_svc.record_source_availability(
                                                &dl_id,
                                                target_addr,
                                                status.availability_mask.clone(),
                                            ) {
                                                emit_download_update(event_tx, sequence, &snap);
                                            }
                                        }
                                    }
                                }
                            } else if packet.protocol == PR_ED2K && packet.opcode == OP_FILEREQANSNOFILE {
                                return Err(PeerError::new(PeerFailureReason::FileNotFound, "Remote peer no longer shares this file"));
                            } else if packet.protocol == PR_ED2K && packet.opcode == OP_CHANGE_CLIENT_ID {
                                super::change_client_id::apply_change_client_id(
                                    &packet.payload,
                                    address_str,
                                    &[file_hash_str.to_string()],
                                    downloads,
                                )
                                .await;
                            } else if (packet.protocol == PR_EMULE)
                                && (packet.opcode == OP_ANSWERSOURCES || packet.opcode == OP_ANSWERSOURCES2)
                            {
                                handle_sx_answer(&packet, dl_hash_bytes, file_hash_str, target_addr, &decoder, downloads, sx_env, sx_local).await;
                            } else if packet.protocol == PR_EMULE
                                && (packet.opcode == OP_REQUESTSOURCES || packet.opcode == OP_REQUESTSOURCES2)
                            {
                                // `reqstatus`: estado de partes del solicitante (su
                                // OP_FILESTATUS) — eMule solo sirve fuentes que
                                // cubran partes que le falten.
                                handle_sx_request(&packet, Some(dl_hash_bytes), target_addr, &decoder, &encoder, downloads, stream, sx_env, sx_caps, Some(file_status.availability_mask.raw_bytes())).await;
                            } else if packet.protocol == PR_EMULE
                            {
                                super::source_exchange::handle_sx_multipacket(
                                    &packet,
                                    dl_hash_bytes,
                                    Some(file_status.availability_mask.raw_bytes()),
                                    target_addr,
                                    &encoder,
                                    downloads,
                                    stream,
                                    sx_env,
                                    sx_caps,
                                    peer_ext_req,
                                )
                                .await;
                            }
                        }
                    }
                    Err(e) => return Err(PeerError::new(PeerFailureReason::ConnectRefused, format!("Read error while in queue: {e}"))),
                }
            }
        }
    }

    // Preservar paquetes concatenados tras `OP_ACCEPTUPLOADREQ` (p. ej. primer bloque) para `run_transfer_loop`
    stream.put_back_unread(&read_buf);

    Ok(file_status)
}

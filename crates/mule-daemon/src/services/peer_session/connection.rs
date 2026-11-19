// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_protocol::ed2k::Ed2kEncoder;
use std::net::SocketAddr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

pub use super::events::{emit_download_update, emit_event};
use super::handshake::perform_peer_handshake;
use super::source_exchange::{PeerSxCaps, SxEnv, SxLocalIds};
use super::status_query::query_file_status_and_enter_queue;
use super::transfer::run_transfer_loop;
pub use super::transfer::{OP_COMPRESSEDPART_I64, OP_SENDINGPART_I64};
use super::types::{PeerError, PeerTimeouts, SwarmInnerState};
use crate::services::downloads::DownloadService;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_peer_connection(
    file_hash_str: String,
    dl_hash_bytes: [u8; 16],
    address_str: String,
    target_addr: SocketAddr,
    client_port: u16,
    nickname: String,
    user_hash: [u8; 16],
    downloads: Arc<Mutex<DownloadService>>,
    cancel_token: mule_network_policy::CancellationToken,
    inner: Arc<Mutex<SwarmInnerState>>,
    event_tx: broadcast::Sender<String>,
    sequence: Arc<AtomicU64>,
    timeouts: PeerTimeouts,
    client_id: u32,
    server_ip: u32,
    server_port: u16,
    kad_port: u16,
    kad_service: Option<Arc<crate::services::KadService>>,
    target_user_hash: Option<[u8; 16]>,
    peer_crypt_options: u8,
    obfuscation_enabled: bool,
    obfuscation_required: bool,
    obfuscation_padding_max: u8,
    identity: Option<Arc<Mutex<crate::services::IdentityService>>>,
    sui_stats: Arc<crate::services::identity::SuiStats>,
    rate_limiters: Option<Arc<mule_network_policy::PolicyRateLimiters>>,
    sx_env: SxEnv,
    score_tracker: Arc<std::sync::Mutex<super::peer_scoring::PeerScoreTracker>>,
    uss: Option<crate::services::UssService>,
) -> Result<u64, PeerError> {
    // eMule anuncia SecIdent=3 (v1|v2) en los bits 16-19 de MiscOptions1 cuando una
    // clave RSA utilizable está cargada, 0 en caso contrario.
    let sec_ident_cap: u8 = match &identity {
        Some(id) if id.lock().await.has_key() => 3,
        _ => 0,
    };
    let session_started = std::time::Instant::now();
    // 1-3. Realizar conexión TCP, negociación de ofuscación, OP_HELLO y esperar OP_HELLO_ANSWER
    let mut handshake_res = perform_peer_handshake(
        target_addr,
        &file_hash_str,
        &address_str,
        client_port,
        &nickname,
        user_hash,
        &timeouts,
        &cancel_token,
        &inner,
        &event_tx,
        &sequence,
        client_id,
        server_ip,
        server_port,
        kad_port,
        &kad_service,
        target_user_hash,
        peer_crypt_options,
        obfuscation_enabled,
        obfuscation_required,
        obfuscation_padding_max,
        &downloads,
        sec_ident_cap,
        &identity,
    )
    .await?;

    if let (Some(rtt_ms), Some(u)) = (handshake_res.initial_rtt_ms, uss.as_ref()) {
        u.record_passive_rtt_sample(rtt_ms);
    }
    if let Ok(mut guard) = score_tracker.lock() {
        guard.record_handshake_profile(
            &address_str,
            handshake_res.stream.is_modern_aead() || handshake_res.peer_pc26_caps.is_some(),
            false,
            handshake_res.initial_rtt_ms,
        );
    }

    // 4-7. Consultar estado del archivo, solicitar nombre, iniciar petición de subida y esperar en cola.
    // El SUI saliente (desafíos OP_SECIDENTSTATE del extremo remoto) se gestiona
    // dentro de los bucles de estado/cola mediante `sui`.
    // Registrar el extremo UDP de eMule anunciado por el par para repeticiones de
    // petición por UDP de cliente (mensajes de mantenimiento `OP_REASKFILEPING` hacia fuentes en cola).
    if let Some(udp_port) = handshake_res.peer_udp_port {
        inner.lock().await.peer_udp.insert(
            address_str.clone(),
            (udp_port, handshake_res.peer_udp_version),
        );
    }

    let mut sui = super::sui::OutboundSui::new(identity)
        .with_stats(sui_stats)
        .with_cache(Arc::clone(&downloads), target_user_hash);
    sui.set_peer_sec_ident(handshake_res.peer_sec_ident);
    // InfoPacketsReceived de eMule: una vez intercambiados los paquetes de información, desafiar al
    // par si anunció compatibilidad con Secure Ident (rol de verificador).
    sui.initiate(
        target_addr,
        &Ed2kEncoder::default(),
        &mut handshake_res.stream,
    )
    .await;
    // Capacidades SX negociadas en el apretón de manos (GetSourceExchange1Version /
    // SupportsSourceExchange2 de eMule) + identidad local para las autocomprobaciones
    // `CanAddSource` (PartFile.cpp:2380-2403 de eMule).
    let sx_caps = PeerSxCaps {
        sx1_version: handshake_res.peer_sx1_version,
        sx2: handshake_res.peer_supports_sx2,
        complete_source: false,
        multipacket: handshake_res.peer_multipacket,
        file_identifiers: handshake_res.peer_file_identifiers,
    };
    let kad_open = match &kad_service {
        Some(kad) => {
            let udp_snap = kad.udp().get_snapshot().await;
            let fw_status = {
                let tracker = kad.udp().firewall_tracker();
                let fw = tracker.lock().await;
                fw.status
            };
            udp_snap.is_active
                && fw_status == crate::services::kad_udp::types::UdpFirewallStatus::Open
        }
        None => false,
    };
    let sx_local = SxLocalIds {
        client_id,
        server_ip,
        server_port,
        tcp_port: client_port,
        is_high_id: client_id >= 16_777_216,
        kad_open,
    };
    // 8-10. Planificador de bloques multifuente, solicitud de bloques y transferencias
    // continuas — con intercambio A4AF a nivel de socket en NNP/FNF
    // (`SwapToAnotherFile`: la conexión activa vuelve a pedir otro archivo).
    let res = run_status_transfer_with_a4af(
        &mut handshake_res.stream,
        target_addr,
        file_hash_str,
        dl_hash_bytes,
        &address_str,
        &sx_caps,
        handshake_res.peer_supports_aich,
        handshake_res.peer_ext_req,
        &timeouts,
        &cancel_token,
        &inner,
        &event_tx,
        &sequence,
        &downloads,
        &sx_env,
        &sx_local,
        &mut sui,
        rate_limiters.as_deref(),
        uss.as_ref(),
    )
    .await;
    if let Ok(bytes) = res {
        let elapsed_ms = session_started.elapsed().as_millis().max(1) as u64;
        let avg_bps = bytes.saturating_mul(1000) / elapsed_ms;
        if let Ok(mut guard) = score_tracker.lock() {
            guard.record_handshake_profile(
                &address_str,
                handshake_res.stream.is_modern_aead() || handshake_res.peer_pc26_caps.is_some(),
                sui.verified_credit_identity().is_some(),
                handshake_res.initial_rtt_ms,
            );
            if bytes > 0 {
                guard.record_transfer_success(&address_str, avg_bps);
            }
        }
    }
    res
}

/// Ejecuta `query_file_status_and_enter_queue` + `run_transfer_loop` para el
/// archivo solicitado, aplicando el A4AF a nivel de socket de eMule en NNP/FNF
/// (`CUpDownClient::SwapToAnotherFile`): en lugar de descartar la
/// conexión, el mismo socket vuelve a emitir `OP_SETREQFILEID` para el mejor
/// archivo alternativo del que este par es fuente. Limitado a
/// `a4af::max_swaps()` intercambios consecutivos.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_status_transfer_with_a4af(
    stream: &mut crate::services::obfuscation_stream::ObfuscatedStream<tokio::net::TcpStream>,
    target_addr: SocketAddr,
    file_hash_str: String,
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
    rate_limiters: Option<&mule_network_policy::PolicyRateLimiters>,
    uss: Option<&crate::services::UssService>,
) -> Result<u64, PeerError> {
    let mut cur_hash_str = file_hash_str;
    let mut cur_hash = dl_hash_bytes;
    let mut swaps = 0u32;
    loop {
        let file_status = match query_file_status_and_enter_queue(
            stream,
            target_addr,
            &cur_hash_str,
            cur_hash,
            address_str,
            sx_caps,
            peer_supports_aich,
            peer_ext_req,
            timeouts,
            cancel_token,
            inner,
            event_tx,
            sequence,
            downloads,
            sx_env,
            sx_local,
            sui,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                match try_a4af_swap(
                    downloads,
                    inner,
                    event_tx,
                    sequence,
                    address_str,
                    &cur_hash_str,
                    &e,
                    &mut swaps,
                )
                .await
                {
                    Some((h, b)) => {
                        cur_hash_str = h;
                        cur_hash = b;
                        continue;
                    }
                    None => return Err(e),
                }
            }
        };
        match run_transfer_loop(
            stream,
            target_addr,
            &cur_hash_str,
            cur_hash,
            address_str,
            file_status,
            peer_supports_aich,
            peer_ext_req,
            timeouts,
            cancel_token,
            inner,
            event_tx,
            sequence,
            downloads,
            sx_caps,
            sx_env,
            sx_local,
            sui,
            rate_limiters,
            uss,
        )
        .await
        {
            Ok(n) => return Ok(n),
            Err(e) => {
                match try_a4af_swap(
                    downloads,
                    inner,
                    event_tx,
                    sequence,
                    address_str,
                    &cur_hash_str,
                    &e,
                    &mut swaps,
                )
                .await
                {
                    Some((h, b)) => {
                        cur_hash_str = h;
                        cur_hash = b;
                        continue;
                    }
                    None => return Err(e),
                }
            }
        }
    }
}

/// Intenta un intercambio A4AF a nivel de socket: escribe la contabilidad de NNP/FNF en
/// el archivo antiguo, marca la fuente activa en el nuevo archivo y actualiza la
/// instantánea de la sesión (`DoSwap`). Devuelve los nuevos `(hash_hex, hash_bytes)`
/// o `None` cuando no existe ningún archivo alternativo viable.
#[allow(clippy::too_many_arguments)]
async fn try_a4af_swap(
    downloads: &Arc<Mutex<DownloadService>>,
    inner: &Arc<Mutex<SwarmInnerState>>,
    event_tx: &broadcast::Sender<String>,
    sequence: &Arc<AtomicU64>,
    address_str: &str,
    from_hash: &str,
    err: &PeerError,
    swaps: &mut u32,
) -> Option<(String, [u8; 16])> {
    if *swaps >= super::a4af::max_swaps() || !super::a4af::is_swap_eligible(&err.reason) {
        return None;
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let max_per_file = inner.lock().await.max_peers_per_download;
    let (to_hash, to_hash_bytes) =
        super::a4af::find_swap_target(downloads, address_str, from_hash, max_per_file, now_ms)
            .await?;
    super::a4af::apply_swap(
        downloads,
        address_str,
        from_hash,
        &to_hash,
        &err.reason,
        now_ms,
    )
    .await;
    *swaps += 1;
    tracing::info!(
        "[A4AF] Socket swap: {} re-requested for {to_hash} (was {from_hash}, reason: {})",
        address_str,
        err.reason.as_str()
    );
    // Reflejar el intercambio en la instantánea de sesión activa (`SetRequestFile` de eMule)
    // y liberar bloques en vuelo para el archivo antiguo — eMule restablece el
    // estado de descarga en `DoSwap` (se abandona `m_PendingBlocks_list`).
    let snap = {
        let mut g = inner.lock().await;
        g.in_flight_blocks.retain(|b| b.peer_id != address_str);
        if let Some(session) = g.sessions.get_mut(address_str) {
            session.snapshot.file_hash = Some(to_hash.clone());
            // La siguiente consulta de estado establece Queued/Transferring por sí misma.
            session.snapshot.queue_rank = None;
            session.snapshot.current_part = None;
            Some(session.snapshot.clone())
        } else {
            None
        }
    };
    if let Some(s) = snap {
        super::events::emit_event(event_tx, sequence, &s);
    }
    Some((to_hash, to_hash_bytes))
}

/// Ejecuta una sesión de descarga con rol de cliente sobre una conexión entrante *aceptada*
/// establecida a través de un callback de LowID del servidor (retransmisión `OP_CALLBACKREQUEST`).
/// El par ya envió `OP_HELLO` y recibió nuestro `OP_HELLOANSWER`, por lo que
/// se omite el apretón de manos de transporte — conducimos `OP_SETREQFILEID`, estado
/// de archivo, puesta en cola y transferencias exactamente como una sesión saliente.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_callback_session(
    mut stream: crate::services::obfuscation_stream::ObfuscatedStream<tokio::net::TcpStream>,
    target_addr: SocketAddr,
    file_hash_str: String,
    dl_hash_bytes: [u8; 16],
    address_str: String,
    sx_caps: PeerSxCaps,
    peer_supports_aich: bool,
    peer_sec_ident: u8,
    peer_ext_req: u8,
    downloads: Arc<Mutex<DownloadService>>,
    cancel_token: mule_network_policy::CancellationToken,
    inner: Arc<Mutex<SwarmInnerState>>,
    event_tx: broadcast::Sender<String>,
    sequence: Arc<AtomicU64>,
    timeouts: PeerTimeouts,
    identity: Option<Arc<Mutex<crate::services::IdentityService>>>,
    sui_stats: Arc<crate::services::identity::SuiStats>,
    rate_limiters: Option<Arc<mule_network_policy::PolicyRateLimiters>>,
    sx_env: SxEnv,
    sx_local: SxLocalIds,
    score_tracker: Arc<std::sync::Mutex<super::peer_scoring::PeerScoreTracker>>,
    uss: Option<crate::services::UssService>,
) -> Result<u64, PeerError> {
    let session_started = std::time::Instant::now();
    let modern = stream.is_modern_aead();
    let mut sui = super::sui::OutboundSui::new(identity)
        .with_stats(sui_stats)
        .with_cache(Arc::clone(&downloads), None);
    sui.set_peer_sec_ident(peer_sec_ident);
    sui.initiate(target_addr, &Ed2kEncoder::default(), &mut stream)
        .await;
    let res = run_status_transfer_with_a4af(
        &mut stream,
        target_addr,
        file_hash_str,
        dl_hash_bytes,
        &address_str,
        &sx_caps,
        peer_supports_aich,
        peer_ext_req,
        &timeouts,
        &cancel_token,
        &inner,
        &event_tx,
        &sequence,
        &downloads,
        &sx_env,
        &sx_local,
        &mut sui,
        rate_limiters.as_deref(),
        uss.as_ref(),
    )
    .await;
    if let Ok(bytes) = res {
        let elapsed_ms = session_started.elapsed().as_millis().max(1) as u64;
        let avg_bps = bytes.saturating_mul(1000) / elapsed_ms;
        if let Ok(mut guard) = score_tracker.lock() {
            guard.record_handshake_profile(
                &address_str,
                modern,
                sui.verified_credit_identity().is_some(),
                None,
            );
            if bytes > 0 {
                guard.record_transfer_success(&address_str, avg_bps);
            }
        }
    }
    res
}

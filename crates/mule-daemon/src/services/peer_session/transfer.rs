// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use crate::services::peer_session::log_peer_addr;
use bytes::BytesMut;
use flate2::{Decompress, FlushDecompress, Status};
use std::collections::{HashMap, VecDeque};
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
    Ed2kBlockRange, Ed2kBlockRange64, Ed2kDecoder, Ed2kDownloadSequencer, Ed2kEncoder,
    Ed2kFileStatus, Ed2kRequestParts, Ed2kRequestPartsI64, InFlightBlock,
    MAX_REQUEST_RANGES_PER_PACKET, OP_ANSWERSOURCES, OP_ANSWERSOURCES2, OP_CANCELTRANSFER,
    OP_CHANGE_CLIENT_ID, OP_COMPRESSEDPART, OP_END_OF_DOWNLOAD, OP_OUTOFPARTS, OP_REQUESTSOURCES,
    OP_REQUESTSOURCES2, OP_SENDINGPART, PR_ED2K, PR_EMULE, PR_EMULE_COMPRESSED,
};
use mule_protocol::framing::{PacketCodec, RawPacket};

use super::events::{emit_download_update, emit_event};
use super::source_exchange::{
    handle_sx_answer, handle_sx_request, maybe_send_sx_request, PeerSxCaps, SxEnv, SxLocalIds,
};
use super::types::{
    PeerConnectionStatus, PeerError, PeerTimeouts, SwarmInnerState, DOWNLOAD_EVENT_INTERVAL,
    MAX_PEER_READ_BUFFER_SIZE,
};
use super::PeerSessionManager;
use crate::services::downloads::DownloadService;
use crate::services::obfuscation_stream::ObfuscatedStream;

pub use mule_protocol::ed2k::{OP_COMPRESSEDPART_I64, OP_SENDINGPART_I64};

/// Construye un paquete de petición de bloques para `range`, eligiendo el formato
/// de cable de 64 bits (`OP_REQUESTPARTS_I64`) cuando los desplazamientos superan el límite de 32 bits.
/// Las peticiones I64 requieren `ExtendedRequestsVer >= 2` del par (`GetExtendedRequestsVersion`
/// de eMule); devuelve `None` cuando el par no puede servir el rango solicitado.
/// Extensiones de archivos previsualizables — eMule solo aplica la prioridad
/// de primera/última parte (`PR_VERYHIGH`) a medios reproducibles durante
/// la descarga.
fn is_previewable_media(file_name: &str) -> bool {
    const PREVIEW_EXTS: &[&str] = &[
        "avi", "mpg", "mpeg", "mp4", "mkv", "wmv", "mov", "flv", "ts", "vob", "mp3", "flac", "ogg",
        "wav", "m4a", "aac",
    ];
    std::path::Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| PREVIEW_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Estados para los cuales una sesión de par puede emitir nuevas peticiones
/// de bloques. Una descarga en pausa/fallida/retirada no debe pedir nada más —
/// las escrituras ya en vuelo aún se persisten en `inject_range`
/// (eMule completa los bloques en vuelo sin reanudar el partfile).
fn is_download_active_status(status: &str) -> bool {
    matches!(
        status.to_lowercase().as_str(),
        "queued" | "downloading" | "waiting_for_sources" | "waitingforsources"
    )
}

/// Petición de bloque pendiente hacia un par — `Pending_Block_Struct` de eMule:
/// el rango fue planificado (`fQueued == false`) o ya fue enviado en un paquete
/// `OP_REQUESTPARTS` (`fQueued == true`); `recv_to` es la frontera hasta
/// la cual se han entregado los datos.
struct PendingBlock {
    part_idx: u16,
    range: Ed2kBlockRange64,
    queued: bool,
    recv_to: u64,
}

/// Número de bloques pendientes de `SendBlockRequests` de eMule, adaptativo a la velocidad
/// de descarga medida de esta fuente (DownloadClient.cpp):
/// `<4 KiB/s → 1`, `<9 KiB/s → 2`, `>150 KiB/s → 9`, `>75 KiB/s → 6`, si no 3.
/// Los niveles de velocidad baja solo se aplican a clientes eMule (`IsEmuleClient()`) una vez
/// medida la tasa de transferencia; antes de medir la tasa (`peer_bps == 0`),
/// toma por defecto 3 bloques para preparar la tubería de ventana TCP de inmediato.
fn emule_pending_block_count(peer_bps: u64, is_emule_client: bool) -> usize {
    if peer_bps == 0 {
        // Estado inicial antes de medir la tasa de transferencia: comienza con 3 bloques
        // (1 paquete OP_REQUESTPARTS completo con 3 rangos) para encauzar de inmediato.
        3
    } else if is_emule_client && peer_bps < 9 * 1024 {
        if peer_bps < 4 * 1024 {
            1
        } else {
            2
        }
    } else if peer_bps > 150 * 1024 {
        9
    } else if peer_bps > 75 * 1024 {
        6
    } else {
        3
    }
}

/// Tiempo durante el cual un bloque en vuelo permanece reservado tras su última señal de vida —
/// eMule no tiene TTL en absoluto (el bloque se libera cuando la sesión termina o se
/// bloquea en cola); el enfoque de refresco con datos acota las entradas obsoletas
/// sin duplicar peticiones hacia fuentes lentas por goteo.
const INFLIGHT_BLOCK_TTL_MS: i64 = 100_000;

/// Recuperaciones consecutivas de atasco→cola permitidas en un socket antes de que
/// se descarte la sesión — eMule reencola indefinidamente; el límite evita que un
/// par patológico retenga una ranura de sesión mientras el enjambre rota fuentes.
const MAX_CONSECUTIVE_STALL_REQUEUES: u32 = 3;

/// Construye un paquete `OP_REQUESTPARTS`/`OP_REQUESTPARTS_I64` con hasta
/// [`MAX_REQUEST_RANGES_PER_PACKET`] rangos — el formato de cable de eMule empaqueta
/// tres ranuras `start`/`end` por paquete. La variante I64 es necesaria cuando algún
/// rango cruza el límite de 4 GiB y requiere `ExtendedRequestsVer >= 2`
/// (`GetExtendedRequestsVersion` de eMule); devuelve `None` cuando el par no puede
/// servir los rangos solicitados.
fn encode_block_request_packet(
    encoder: &Ed2kEncoder,
    file_hash: [u8; 16],
    ranges: &[Ed2kBlockRange64],
    peer_ext_req: u8,
    file_size: u64,
) -> Option<mule_protocol::framing::RawPacket> {
    if ranges.is_empty() || ranges.len() > MAX_REQUEST_RANGES_PER_PACKET {
        return None;
    }
    let is_large_file = file_size > u32::MAX as u64;
    let needs_i64 = ranges.iter().any(|r| r.end > u32::MAX as u64);
    if (is_large_file || needs_i64) && peer_ext_req >= 2 {
        encoder
            .encode_request_parts_i64(&Ed2kRequestPartsI64 {
                file_hash,
                ranges: ranges.to_vec(),
            })
            .ok()
    } else if !needs_i64 {
        let ranges32: Vec<Ed2kBlockRange> = ranges
            .iter()
            .map(|r| Ed2kBlockRange::new(r.start as u32, r.end as u32))
            .collect();
        encoder
            .encode_request_parts(&Ed2kRequestParts::multi(file_hash, ranges32))
            .ok()
    } else {
        tracing::warn!(
            "Block ranges cross the 4 GiB boundary but peer does not support extended requests (ext_req={peer_ext_req}) — cannot request"
        );
        None
    }
}

/// `SendBlockRequests` de eMule: completa `pending` hasta `block_count` bloques
/// disjuntos mediante el secuenciador, registra cada uno como en vuelo y luego emite
/// paquetes `OP_REQUESTPARTS` de hasta tres rangos cada uno. Devuelve `true` cuando
/// al menos un rango nuevo se envió por el cable.
#[allow(clippy::too_many_arguments)]
async fn fill_and_send_block_requests(
    stream: &mut ObfuscatedStream<TcpStream>,
    encoder: &Ed2kEncoder,
    sequencer: &mut Ed2kDownloadSequencer,
    address_str: &str,
    inner: &Arc<Mutex<SwarmInnerState>>,
    dl_hash_bytes: [u8; 16],
    peer_ext_req: u8,
    completed_ranges: &[(u64, u64)],
    pending: &mut VecDeque<PendingBlock>,
    block_count: usize,
    target_addr: SocketAddr,
    file_size: u64,
) -> Result<bool, PeerError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    // Planificar los bloques faltantes y reservarlos en la lista compartida en vuelo.
    let planned = {
        let mut g = inner.lock().await;
        let want = block_count.saturating_sub(pending.len());
        let blocks = if want == 0 {
            Vec::new()
        } else {
            sequencer
                .plan_next_blocks_for_peer64(
                    address_str,
                    completed_ranges,
                    &g.in_flight_blocks,
                    now_ms,
                    want,
                )
                .map_err(|e| {
                    PeerError::new(
                        PeerFailureReason::InvalidRange,
                        format!("Planning error: {e}"),
                    )
                })?
        };
        for (part_idx, block) in &blocks {
            g.in_flight_blocks.push(InFlightBlock::new(
                address_str.to_string(),
                *part_idx,
                block.start,
                block.end,
                now_ms,
                INFLIGHT_BLOCK_TTL_MS,
            ));
            pending.push_back(PendingBlock {
                part_idx: *part_idx,
                range: *block,
                queued: false,
                recv_to: block.start,
            });
        }
        blocks.len()
    };

    // Emitir los bloques pendientes no puestos en cola en paquetes de ≤3 rangos.
    let mut sent_any = false;
    let unqueued: Vec<Ed2kBlockRange64> = pending
        .iter()
        .filter(|p| !p.queued)
        .map(|p| p.range)
        .collect();
    for chunk in unqueued.chunks(MAX_REQUEST_RANGES_PER_PACKET) {
        let Some(pkt) = encode_block_request_packet(encoder, dl_hash_bytes, chunk, peer_ext_req, file_size)
        else {
            // Fragmento no codificable (p. ej. >4 GiB sin soporte I64): marcar los
            // rangos como enviados para que dejen de bloquear la canalización; serán
            // replanificados en la siguiente pasada para un par distinto.
            tracing::warn!(
                "Peer {peer}: dropping unencodable request chunk ({} range(s))",
                chunk.len(),
                peer = log_peer_addr(target_addr)
            );
            for r in chunk {
                if let Some(p) = pending
                    .iter_mut()
                    .find(|p| p.range.start == r.start && p.range.end == r.end)
                {
                    p.queued = true;
                }
            }
            continue;
        };
        let mut out_wire = Vec::with_capacity(6 + pkt.payload.len());
        out_wire.push(pkt.protocol);
        out_wire.extend_from_slice(&(pkt.frame_len() as u32).to_le_bytes());
        out_wire.push(pkt.opcode);
        out_wire.extend_from_slice(&pkt.payload);
        tracing::info!(
            "Peer {peer}: sending block request packet (proto=0x{:02X}, opcode=0x{:02X}, len={}, ranges={:?})",
            pkt.protocol,
            pkt.opcode,
            out_wire.len(),
            chunk,
            peer = log_peer_addr(target_addr)
        );
        if let Err(e) = stream.write_all(&out_wire).await {
            return Err(PeerError::new(
                PeerFailureReason::ConnectRefused,
                format!("Failed to send OP_REQUESTPARTS: {e}"),
            ));
        }
        for r in chunk {
            if let Some(p) = pending
                .iter_mut()
                .find(|p| p.range.start == r.start && p.range.end == r.end)
            {
                p.queued = true;
            }
        }
        sent_any = true;
    }
    if planned > 0 {
        tracing::info!(
            "Peer {peer}: {planned} new pending block(s) planned ({sent} sent, cap {block_count})",
            peer = log_peer_addr(target_addr),
            sent = pending.iter().filter(|p| p.queued).count()
        );
    }
    Ok(sent_any)
}

/// Aplica los baneos solicitados por `CorruptionBlackBox` tras
/// un `inject_range` (`Ban` + `clientlist->AddBannedClient` de eMule): las
/// fuentes infractoras pasan a `Rejected` y sus sesiones se abortan.
/// Devuelve `true` cuando el propio par de esta sesión es baneado —
/// quien llama debe entonces salir del bucle de transferencia.
async fn apply_corruption_bans(
    inner: &Arc<Mutex<SwarmInnerState>>,
    downloads: &Arc<Mutex<DownloadService>>,
    download_id: uuid::Uuid,
    file_hash_str: &str,
    self_addr: &str,
) -> bool {
    let banned = {
        let mut dl_svc = downloads.lock().await;
        dl_svc.drain_corruption_bans(&download_id)
    };
    if banned.is_empty() {
        return false;
    }
    let mut self_banned = false;
    for addr in banned {
        tracing::warn!(
            "[CORRUPTION_BLACKBOX] Banning {addr} for file {file_hash_str} (>32% corrupted data, eMule CBB_BANTHRESHOLD)"
        );
        {
            let mut dl_svc = downloads.lock().await;
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            let _ = dl_svc.update_peer_source_status(
                file_hash_str,
                &addr,
                mule_core::model::PeerSourceStatus::Allowed,
                Some("corruption blackbox: temporary cooldown"),
                Some(now_ms + 15 * 60 * 1000),
            );
        }
        if addr == self_addr {
            self_banned = true;
            continue;
        }
        let mut g = inner.lock().await;
        if let Some(session) = g.sessions.remove(&addr) {
            session.abort_handle.abort();
            g.in_flight_blocks.retain(|b| b.peer_id != addr);
        }
    }
    self_banned
}

async fn sync_transfer_rates(
    inner: &Arc<Mutex<SwarmInnerState>>,
    downloads: &Arc<Mutex<DownloadService>>,
    event_tx: &broadcast::Sender<String>,
    sequence: &Arc<AtomicU64>,
    address_str: &str,
    file_hash_str: &str,
    peer_bps: u64,
    total_received: u64,
) {
    // Coalescencia por archivo: las N sesiones de una misma descarga comparten
    // un presupuesto de emisión `DOWNLOAD_EVENT_INTERVAL` — la serialización
    // JSON de la instantánea (`parts` + `pending_ranges`) y su análisis en la UI
    // son el coste dominante, que de otro modo se pagaría por sesión en cada cadencia.
    let (progress_snapshot, total_file_rate_bps, pending_ranges, emit_due) = {
        let mut g = inner.lock().await;
        let emit_due = g
            .last_download_event
            .get(file_hash_str)
            .is_none_or(|t| t.elapsed() >= DOWNLOAD_EVENT_INTERVAL);
        let snap = if let Some(session) = g.sessions.get_mut(address_str) {
            session.snapshot.transferred_bytes = total_received;
            session.snapshot.transfer_rate_bps = peer_bps;
            if emit_due {
                Some(session.snapshot.clone())
            } else {
                None
            }
        } else {
            None
        };
        if emit_due {
            g.last_download_event
                .insert(file_hash_str.to_string(), std::time::Instant::now());
        }
        let total_rate: u64 = g
            .sessions
            .values()
            .filter(|s| s.snapshot.file_hash.as_deref() == Some(file_hash_str))
            .map(|s| s.snapshot.transfer_rate_bps)
            .sum();
        let pending = if emit_due {
            PeerSessionManager::pending_ranges_locked(&g, file_hash_str)
        } else {
            Vec::new()
        };
        (snap, total_rate, pending, emit_due)
    };

    if let Some(snap) = progress_snapshot {
        emit_event(event_tx, sequence, &snap);
    }

    let mut dl_svc = downloads.lock().await;
    if emit_due {
        if let Some(mut snap) = dl_svc.set_download_rate_by_hash(file_hash_str, total_file_rate_bps) {
            snap.pending_ranges = pending_ranges;
            emit_download_update(event_tx, sequence, &snap);
        }
    } else {
        // Tasa actualizada sin instantánea ni evento (la siguiente ventana
        // emite el valor actual; `decay_stale_rates` se mantiene informado).
        dl_svc.update_download_rate_by_hash(file_hash_str, total_file_rate_bps);
    }
}

/// Resultado de intentar reentrar en la cola remota sobre el mismo socket abierto.
enum RequeueOutcome {
    /// El par concedió una nueva ranura de subida y ya se enviaron nuevas peticiones de bloque.
    Resumed,
    /// No quedan bloques necesarios pendientes en este par (se envió `OP_END_OF_DOWNLOAD`).
    NoMoreBlocks,
}

/// Referencias inmutables compartidas para el reencolado sobre el mismo socket.
struct RequeueContext<'a> {
    target_addr: SocketAddr,
    file_hash_str: &'a str,
    dl_hash_bytes: [u8; 16],
    address_str: &'a str,
    sx_caps: &'a PeerSxCaps,
    timeouts: &'a PeerTimeouts,
    cancel_token: &'a mule_network_policy::CancellationToken,
    inner: &'a Arc<Mutex<SwarmInnerState>>,
    event_tx: &'a broadcast::Sender<String>,
    sequence: &'a Arc<AtomicU64>,
    downloads: &'a Arc<Mutex<DownloadService>>,
    sx_env: &'a SxEnv,
    sx_local: &'a SxLocalIds,
    encoder: &'a Ed2kEncoder,
    peer_ext_req: u8,
    peer_supports_aich: bool,
    is_previewable: bool,
    is_emule_client: bool,
    file_size: u64,
    part_scarcity: &'a Option<Vec<u32>>,
    file_status_fallback: &'a Ed2kFileStatus,
}

/// Libera los bloques pendientes, pasa la sesión a `Queued`, conserva bytes no leídos
/// y espera en la cola de subida remota sobre el mismo socket para reanudar la descarga.
#[allow(clippy::too_many_arguments)]
async fn requeue_and_resume_transfer(
    ctx: &RequeueContext<'_>,
    stream: &mut ObfuscatedStream<TcpStream>,
    read_buf: &mut BytesMut,
    sui: &mut super::sui::OutboundSui,
    file_status_refreshed: &mut Option<Ed2kFileStatus>,
    pending: &mut VecDeque<PendingBlock>,
    zstreams: &mut HashMap<u64, Decompress>,
    sequencer: &mut Ed2kDownloadSequencer,
    last_block_data: &mut tokio::time::Instant,
) -> Result<RequeueOutcome, PeerError> {
    // Descartar el estado pendiente local y liberar los rangos en vuelo para
    // que otros pares puedan asumirlos mientras esperamos.
    pending.clear();
    zstreams.clear();
    {
        let mut g = ctx.inner.lock().await;
        g.in_flight_blocks.retain(|b| b.peer_id != ctx.address_str);
        if let Some(session) = g.sessions.get_mut(ctx.address_str) {
            session.snapshot.status = PeerConnectionStatus::Queued;
            session.snapshot.queue_rank = None;
            let snap = session.snapshot.clone();
            emit_event(ctx.event_tx, ctx.sequence, &snap);
        }
    }
    stream.put_back_unread(read_buf);
    read_buf.clear();

    let fresh_status = super::status_query::wait_in_queue(
        stream,
        ctx.target_addr,
        ctx.file_hash_str,
        ctx.dl_hash_bytes,
        ctx.address_str,
        ctx.sx_caps,
        ctx.timeouts,
        ctx.cancel_token,
        ctx.inner,
        ctx.event_tx,
        ctx.sequence,
        ctx.downloads,
        ctx.sx_env,
        ctx.sx_local,
        sui,
        ctx.peer_ext_req,
        ctx.peer_supports_aich,
        file_status_refreshed
            .take()
            .unwrap_or_else(|| ctx.file_status_fallback.clone()),
        true,
    )
    .await?;

    *file_status_refreshed = Some(fresh_status.clone());
    // Reconstruir el secuenciador con el mapa de bits de partes actualizado — la
    // disponibilidad del par pudo haber cambiado mientras estábamos en cola.
    let mut rebuilt = Ed2kDownloadSequencer::new(ctx.dl_hash_bytes, ctx.file_size, fresh_status)
        .map_err(|e| {
            PeerError::new(
                PeerFailureReason::CorruptedData,
                format!("Sequencer init error: {e}"),
            )
        })?
        .with_preview_priority(ctx.is_previewable)
        .with_endgame_threshold(mule_protocol::ed2k::ED2K_PART_SIZE as u64);
    if let Some(counts) = ctx.part_scarcity {
        rebuilt = rebuilt.with_part_scarcity(counts.clone());
    }
    *sequencer = rebuilt;

    let completed = ctx
        .downloads
        .lock()
        .await
        .get_completed_ranges_by_hash(ctx.file_hash_str);
    let peer_bps = ctx
        .inner
        .lock()
        .await
        .sessions
        .get(ctx.address_str)
        .map(|s| s.snapshot.transfer_rate_bps)
        .unwrap_or(0);
    fill_and_send_block_requests(
        stream,
        ctx.encoder,
        sequencer,
        ctx.address_str,
        ctx.inner,
        ctx.dl_hash_bytes,
        ctx.peer_ext_req,
        &completed,
        pending,
        emule_pending_block_count(peer_bps, ctx.is_emule_client),
        ctx.target_addr,
        ctx.file_size,
    )
    .await?;

    if pending.is_empty() {
        info!(
            "Peer {peer}: no more needed blocks after re-queue",
            peer = log_peer_addr(ctx.target_addr)
        );
        let mut end_wire = Vec::with_capacity(6 + 16);
        end_wire.push(PR_ED2K);
        end_wire.extend_from_slice(&(17u32).to_le_bytes());
        end_wire.push(OP_END_OF_DOWNLOAD);
        end_wire.extend_from_slice(&ctx.dl_hash_bytes);
        let _ = stream.write_all(&end_wire).await;
        return Ok(RequeueOutcome::NoMoreBlocks);
    }

    {
        let mut g = ctx.inner.lock().await;
        if let Some(session) = g.sessions.get_mut(ctx.address_str) {
            session.snapshot.status = PeerConnectionStatus::Transferring;
            session.snapshot.current_part = pending.front().map(|p| p.part_idx);
            let snap = session.snapshot.clone();
            emit_event(ctx.event_tx, ctx.sequence, &snap);
        }
    }
    *last_block_data = tokio::time::Instant::now();
    Ok(RequeueOutcome::Resumed)
}

pub(crate) async fn run_transfer_loop(
    stream: &mut ObfuscatedStream<TcpStream>,
    target_addr: SocketAddr,
    file_hash_str: &str,
    dl_hash_bytes: [u8; 16],
    address_str: &str,
    file_status: Ed2kFileStatus,
    peer_supports_aich: bool,
    peer_ext_req: u8,
    timeouts: &PeerTimeouts,
    cancel_token: &mule_network_policy::CancellationToken,
    inner: &Arc<Mutex<SwarmInnerState>>,
    event_tx: &broadcast::Sender<String>,
    sequence: &Arc<AtomicU64>,
    downloads: &Arc<Mutex<DownloadService>>,
    sx_caps: &PeerSxCaps,
    sx_env: &SxEnv,
    sx_local: &SxLocalIds,
    sui: &mut super::sui::OutboundSui,
    rate_limiters: Option<&mule_network_policy::PolicyRateLimiters>,
) -> Result<u64, PeerError> {
    let encoder = Ed2kEncoder::default();
    let decoder = Ed2kDecoder::default().with_extended_requests_version(peer_ext_req);
    let codec = PacketCodec::new();
    let mut read_buf = BytesMut::with_capacity(65536);
    let mut raw_buf = [0u8; 8192];
    // Estado del par capturado antes de que `file_status` sea consumido por el
    // secuenciador: estado de partes del solicitante (`reqstatus` de eMule para
    // respuestas SX) y compleción (`m_bCompleteSource` — eMule no pide fuentes
    // a una fuente completa).
    let peer_part_status: Vec<u8> = file_status.availability_mask.raw_bytes().to_vec();
    let peer_is_complete = file_status.is_complete();
    // Copia conservada para la ruta atasco→cola (`file_status` es consumido
    // por el secuenciador); actualizada por `wait_in_queue` en cada reentrada.
    let file_status_fallback = file_status.clone();
    let mut file_status_refreshed: Option<Ed2kFileStatus> = None;

    // Vaciar peticiones de recuperación AICH pendientes acumuladas antes de esta sesión
    // (`SendAICHRequest` de eMule para partes dañadas).
    super::status_query::send_pending_aich_requests(
        stream,
        &encoder,
        target_addr,
        file_hash_str,
        dl_hash_bytes,
        peer_supports_aich,
        downloads,
    )
    .await;

    // Configuración del planificador de bloques multifuente
    let (file_size, completed_ranges, is_previewable, part_scarcity) = {
        let dl_svc = downloads.lock().await;
        let dl = dl_svc.find_by_ed2k_hash(file_hash_str).ok_or_else(|| {
            PeerError::new(
                PeerFailureReason::FileNotFound,
                "Download not found in local service",
            )
        })?;
        // La descarga pudo haberse pausado/detenido entre la planificación y
        // el inicio de la sesión — nunca pedir bloques para un archivo inactivo.
        if !is_download_active_status(&dl.status) {
            info!(
                "Peer {peer}: download '{}' is '{}' — no block requests will be sent",
                dl.file_name,
                dl.status,
                peer = log_peer_addr(target_addr)
            );
            return Ok(0);
        }
        let ranges = dl_svc.get_completed_ranges_by_hash(file_hash_str);
        // Sanación de enjambre (ajuste `swarm_healing` de eMuleRust): sesga la
        // elección de partes hacia las más raras del enjambre (fuentes conocidas).
        let healing = dl_svc
            .get_setting("swarm_healing")
            .ok()
            .flatten()
            .map(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "on"))
            .unwrap_or(true);
        let scarcity = if healing {
            dl_svc.part_source_counts(file_hash_str)
        } else {
            None
        };
        (
            dl.total_bytes,
            ranges,
            is_previewable_media(&dl.file_name),
            scarcity,
        )
    };

    let mut sequencer = Ed2kDownloadSequencer::new(dl_hash_bytes, file_size, file_status)
        .map_err(|e| {
            PeerError::new(
                PeerFailureReason::CorruptedData,
                format!("Sequencer init error: {e}"),
            )
        })?
        .with_preview_priority(is_previewable)
        .with_endgame_threshold(mule_protocol::ed2k::ED2K_PART_SIZE as u64);
    if let Some(counts) = &part_scarcity {
        sequencer = sequencer.with_part_scarcity(counts.clone());
    }

    // `SendBlockRequests` de eMule — canalización de bloques pendientes. `blockCount`
    // es adaptativo a la tasa medida de esta fuente (1/2/3/6/9) y cada paquete
    // OP_REQUESTPARTS empaqueta hasta tres rangos.
    let is_emule_client = peer_ext_req > 0;
    let peer_bps = inner
        .lock()
        .await
        .sessions
        .get(address_str)
        .map(|s| s.snapshot.transfer_rate_bps)
        .unwrap_or(0);
    let mut pending: VecDeque<PendingBlock> = VecDeque::new();
    fill_and_send_block_requests(
        stream,
        &encoder,
        &mut sequencer,
        address_str,
        inner,
        dl_hash_bytes,
        peer_ext_req,
        &completed_ranges,
        &mut pending,
        emule_pending_block_count(peer_bps, is_emule_client),
        target_addr,
        file_size,
    )
    .await?;

    if pending.is_empty() {
        info!(
            "No unassigned/missing blocks available for peer {}",
            log_peer_addr(target_addr)
        );
        // `CUpDownClient::SendOutOfPartReqsAndPacket` de eMule: notificar al
        // subidor que actualmente no tenemos peticiones de bloques pendientes
        // (OP_OUTOFPARTS, carga útil vacía) antes de abandonar la sesión.
        let mut out_wire = Vec::with_capacity(6);
        out_wire.push(PR_ED2K);
        out_wire.extend_from_slice(&1u32.to_le_bytes());
        out_wire.push(OP_OUTOFPARTS);
        let _ = stream.write_all(&out_wire).await;
        return Ok(0);
    }

    let transferring_snapshot = {
        let mut g = inner.lock().await;
        if let Some(session) = g.sessions.get_mut(address_str) {
            session.snapshot.status = PeerConnectionStatus::Transferring;
            session.snapshot.current_part = pending.front().map(|p| p.part_idx);
            Some(session.snapshot.clone())
        } else {
            None
        }
    };
    if let Some(snap) = transferring_snapshot {
        emit_event(event_tx, sequence, &snap);
    }

    // Bucle de recepción de fragmentos
    let mut total_received = 0u64;
    // DOWNLOADTIMEOUT (100 s) de eMule se mide desde `m_dwLastBlockReceived`,
    // que SendBlockRequests actualiza — un envío de petición cuenta como actividad.
    let mut last_block_data = tokio::time::Instant::now();
    let mut last_rate_check = std::time::Instant::now();
    let mut bytes_since_rate_check = 0u64;
    // Los flujos zlib son por bloque pendiente (eMule asocia `zStream` al
    // Pending_Block_Struct, indexado aquí por el desplazamiento inicial del bloque).
    let mut zstreams: HashMap<u64, Decompress> = HashMap::new();
    let mut last_dl_progress_log = std::time::Instant::now();
    let mut consecutive_stall_requeues = 0u32;
    // Créditos de eMule: los bytes recibidos se acumulan aquí y se persisten bajo
    // la identidad de clave pública verificada del par en lotes (semántica de clients.met).
    let mut credit_pending_bytes = 0u64;

    struct RateResetGuard<'a> {
        downloads: &'a Arc<Mutex<DownloadService>>,
        inner: &'a Arc<Mutex<SwarmInnerState>>,
        file_hash_str: &'a str,
        address_str: &'a str,
        event_tx: &'a broadcast::Sender<String>,
        sequence: &'a Arc<AtomicU64>,
    }

    impl<'a> Drop for RateResetGuard<'a> {
        fn drop(&mut self) {
            let downloads = Arc::clone(self.downloads);
            let inner = Arc::clone(self.inner);
            let file_hash_str = self.file_hash_str.to_string();
            let address_str = self.address_str.to_string();
            let event_tx = self.event_tx.clone();
            let sequence = Arc::clone(self.sequence);
            tokio::spawn(async move {
                let (total_remaining_bps, pending_ranges): (u64, Vec<(u64, u64)>) = {
                    let mut g = inner.lock().await;
                    if let Some(session) = g.sessions.get_mut(&address_str) {
                        session.snapshot.transfer_rate_bps = 0;
                    }
                    let total = g
                        .sessions
                        .values()
                        .filter(|s| s.snapshot.file_hash.as_deref() == Some(&file_hash_str))
                        .map(|s| s.snapshot.transfer_rate_bps)
                        .sum();
                    let pending = PeerSessionManager::pending_ranges_locked(&g, &file_hash_str);
                    (total, pending)
                };
                let mut dl_svc = downloads.lock().await;
                if let Some(mut snap) =
                    dl_svc.set_download_rate_by_hash(&file_hash_str, total_remaining_bps)
                {
                    snap.pending_ranges = pending_ranges;
                    emit_download_update(&event_tx, &sequence, &snap);
                }
            });
        }
    }

    let _rate_guard = RateResetGuard {
        downloads,
        inner,
        file_hash_str,
        address_str,
        event_tx,
        sequence,
    };

    // Repetición periódica de petición SX — eMule asocia `OP_REQUESTSOURCES` al ciclo
    // de re-ask del cliente (`SendFileRequest` → `IsSourceRequestAllowed`, cadencia
    // dominada por SOURCECLIENTREASKS = 40 min). Comprobado cada minuto.
    let mut next_sx_check = tokio::time::Instant::now() + Duration::from_secs(60);
    let reask_caps = PeerSxCaps {
        complete_source: peer_is_complete,
        ..*sx_caps
    };
    let requeue_ctx = RequeueContext {
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
        encoder: &encoder,
        peer_ext_req,
        peer_supports_aich,
        is_previewable,
        is_emule_client,
        file_size,
        part_scarcity: &part_scarcity,
        file_status_fallback: &file_status_fallback,
    };

    'transfer_loop: loop {
        // DOWNLOADTIMEOUT (100 s) de eMule: sin datos de bloque desde el último envío
        // de petición o paquete recibido → SendCancelTransfer + regreso a la cola en el
        // mismo socket. `first_data_timeout` cubre la ventana previa al primer bloque.
        let stall_timeout = if total_received == 0 {
            timeouts.first_data_timeout
        } else {
            timeouts.inactivity_timeout
        };

        tokio::select! {
            _ = cancel_token.cancelled() => {
                // eMule envía OP_CANCELTRANSFER (carga útil vacía) cuando se cancela
                // una transferencia en curso, para que el subidor libere la
                // ranura de inmediato en lugar de depender del cierre TCP.
                let _ = stream
                    .write_all(&[PR_ED2K, 1, 0, 0, 0, OP_CANCELTRANSFER])
                    .await;
                return Err(PeerError::new(PeerFailureReason::KillSwitchAborted, "Transfer aborted by kill switch"));
            }
            _ = tokio::time::sleep_until(last_block_data + stall_timeout) => {
                // `CUpDownClient::CheckDownloadTimeout` de eMule: el subidor
                // remoto se atascó → OP_CANCELTRANSFER y luego reentrar en su
                // cola de subida en el mismo socket (posición de cola gestionada por
                // el remoto, que nos desplaza hacia atrás como en un re-ask normal).
                consecutive_stall_requeues += 1;
                if consecutive_stall_requeues > MAX_CONSECUTIVE_STALL_REQUEUES {
                    return Err(PeerError::new(PeerFailureReason::TimeoutInactivity, "Peer stalled repeatedly — giving up on this session"));
                }
                info!(
                    "Peer {peer}: no block data for {}s — OP_CANCELTRANSFER + re-queue on same socket (stall #{consecutive_stall_requeues})",
                    stall_timeout.as_secs(),
                    peer = log_peer_addr(target_addr)
                );
                let _ = stream
                    .write_all(&[PR_ED2K, 1, 0, 0, 0, OP_CANCELTRANSFER])
                    .await;
                match requeue_and_resume_transfer(
                    &requeue_ctx,
                    stream,
                    &mut read_buf,
                    sui,
                    &mut file_status_refreshed,
                    &mut pending,
                    &mut zstreams,
                    &mut sequencer,
                    &mut last_block_data,
                )
                .await?
                {
                    RequeueOutcome::Resumed => {}
                    RequeueOutcome::NoMoreBlocks => break 'transfer_loop,
                }
            }
            _ = tokio::time::sleep_until(next_sx_check) => {
                next_sx_check = tokio::time::Instant::now() + Duration::from_secs(60);
                maybe_send_sx_request(
                    stream,
                    &encoder,
                    target_addr,
                    file_hash_str,
                    dl_hash_bytes,
                    downloads,
                    sx_env,
                    &reask_caps,
                )
                .await;
            }
            read_res = stream.read(&mut raw_buf) => {
                match read_res {
                    Ok(0) => {
                        info!("Peer {} closed stream", log_peer_addr(target_addr));
                        break;
                    }
                    Ok(n) => {
                        read_buf.extend_from_slice(&raw_buf[..n]);
                        if read_buf.len() > MAX_PEER_READ_BUFFER_SIZE {
                            return Err(PeerError::new(PeerFailureReason::CorruptedData, "Peer read buffer exceeded limit during transfer"));
                        }
                        // Respuestas `OP_MULTIPACKETANSWER*` desglosadas —
                        // `ListenSocket.cpp:1071` (cf. status_query).
                        let mut expanded: std::collections::VecDeque<RawPacket> =
                            std::collections::VecDeque::new();
                        loop {
                            let packet = if let Some(p) = expanded.pop_front() {
                                p
                            } else {
                                match codec.decode(&mut read_buf) {
                                    // OP_PACKEDPROT (0xD4) — eMule descomprime
                                    // en línea y reetiqueta como OP_EMULEPROT
                                    // (`ListenSocket.cpp:1742` →
                                    // ProcessExtPacket); cargas útiles empaquetadas
                                    // corruptas se descartan.
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
                            if super::status_query::handle_hashset_answer(&packet, dl_hash_bytes, file_hash_str, target_addr, &decoder, downloads).await {
                                continue;
                            }
                            if super::status_query::handle_aich_file_hash_res(stream, &packet, dl_hash_bytes, file_hash_str, target_addr, &decoder, &encoder, peer_supports_aich, downloads).await {
                                continue;
                            }
                            if super::status_query::handle_aich_answer(stream, &packet, dl_hash_bytes, file_hash_str, target_addr, &decoder, &encoder, peer_supports_aich, downloads).await {
                                continue;
                            }
                            // Bloque pendiente completado por este paquete — vaciado
                            // tras el despacho del opcode (elimina la entrada pendiente,
                            // libera el rango en vuelo y rellena la canalización
                            // hasta `blockCount`).
                            let mut finished_pending: Option<(u16, Ed2kBlockRange64)> = None;
                            if (packet.protocol == PR_ED2K || packet.protocol == PR_EMULE)
                                && (packet.opcode == OP_SENDINGPART || packet.opcode == OP_SENDINGPART_I64)
                            {
                                last_block_data = tokio::time::Instant::now();
                                consecutive_stall_requeues = 0;
                                let (part_hash, part_start, part_end, part_data) = if packet.opcode == OP_SENDINGPART_I64 {
                                    let part = mule_protocol::ed2k::decoder::decode_sending_part_i64(packet.protocol, packet.opcode, &packet.payload, 0)
                                        .map_err(|e| PeerError::new(PeerFailureReason::CorruptedData, format!("Malformed OP_SENDINGPART_I64: {e}")))?;
                                    (part.file_hash, part.start_offset, part.end_offset, part.data)
                                } else {
                                    let part = decoder.decode_sending_part(packet.protocol, packet.opcode, &packet.payload, 0)
                                        .map_err(|e| PeerError::new(PeerFailureReason::CorruptedData, format!("Malformed OP_SENDINGPART: {e}")))?;
                                    (part.file_hash, part.start_offset as u64, part.end_offset as u64, part.data)
                                };

                                tracing::info!(
                                    "Peer {peer}: received {} [{}, {}) ({} bytes) for {file_hash_str}",
                                    if packet.opcode == OP_SENDINGPART_I64 { "OP_SENDINGPART_I64" } else { "OP_SENDINGPART" },
                                    part_start,
                                    part_end,
                                    part_data.len(),
                                    peer = log_peer_addr(target_addr)
                                );

                                if part_hash != dl_hash_bytes {
                                    return Err(PeerError::new(PeerFailureReason::HashMismatch, "OP_SENDINGPART returned mismatched file hash"));
                                }

                                // `ProcessBlockPacket` de eMule: el rango declarado
                                // debe coincidir exactamente con el tamaño de la carga útil
                                // (IDS_ERR_BADDATABLOCK → violación de protocolo).
                                if part_start >= part_end
                                    || part_data.len() as u64 != part_end - part_start
                                {
                                    return Err(PeerError::new(PeerFailureReason::InvalidRange, format!("OP_SENDINGPART declared range [{}, {}) does not match {} bytes payload", part_start, part_end, part_data.len())));
                                }

                                // `ProcessBlockPacket` de eMule: localizar el bloque
                                // pendiente al que pertenece este paquete — cualquiera de los hasta
                                // `blockCount` rangos en cola puede ser servido.
                                let pending_idx = pending.iter().position(|p| {
                                    p.range.start <= part_start && part_start < p.range.end
                                });
                                let Some(pi) = pending_idx else {
                                    // Bloque no solicitado — eMule lo descarta
                                    // silenciosamente y mantiene viva la sesión.
                                    tracing::debug!(
                                        "Peer {peer}: OP_SENDINGPART [{}, {}) matches no pending block — ignored",
                                        part_start,
                                        part_end,
                                        peer = log_peer_addr(target_addr)
                                    );
                                    continue;
                                };
                                if part_end > pending[pi].range.end {
                                    // eMule: "exceeds requested boundaries" →
                                    // RemoveBlockFromList (libera el rango)
                                    // e ignora el paquete — sin desconexión.
                                    tracing::warn!(
                                        "Peer {peer}: OP_SENDINGPART [{}, {}) exceeds pending block [{}, {}) — block freed",
                                        part_start,
                                        part_end,
                                        pending[pi].range.start,
                                        pending[pi].range.end,
                                        peer = log_peer_addr(target_addr)
                                    );
                                    let freed = pending.remove(pi).expect("pending idx in bounds");
                                    {
                                        let mut g = inner.lock().await;
                                        g.in_flight_blocks.retain(|b| {
                                            !(b.peer_id == address_str
                                                && b.start == freed.range.start
                                                && b.end == freed.range.end)
                                        });
                                    }
                                    continue;
                                }
                                pending[pi].recv_to = pending[pi].recv_to.max(part_end);

                                if part_end >= pending[pi].range.end {
                                    finished_pending =
                                        Some((pending[pi].part_idx, pending[pi].range));
                                }

                                // Refrescar la reserva en vuelo mientras el par
                                // siga entregando — el bloque no debe
                                // replanificarse a otra fuente a mitad de la transferencia.
                                {
                                    let mut g = inner.lock().await;
                                    let now_ms = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis() as i64;
                                    for b in g.in_flight_blocks.iter_mut() {
                                        if b.peer_id == address_str {
                                            b.deadline_unix_ms = now_ms + INFLIGHT_BLOCK_TTL_MS;
                                        }
                                    }
                                }

                                let data_len = part_data.len() as u64;
                                // Planificador global de ancho de banda de descarga: regular los
                                // datos recibidos a través del cubo de fichas compartido.
                                if let Some(rl) = rate_limiters {
                                    if rl
                                        .download_bucket
                                        .acquire_tokens(data_len, || {
                                            cancel_token.is_cancelled()
                                        })
                                        .await
                                        .is_err()
                                    {
                                        break 'transfer_loop;
                                    }
                                }
                                total_received += data_len;
                                bytes_since_rate_check += data_len;
                                credit_pending_bytes += data_len;
                                if credit_pending_bytes >= CREDIT_FLUSH_BYTES {
                                    flush_download_credit(downloads, sui, &mut credit_pending_bytes).await;
                                }

                                // Las E/S de `inject_range_brief` (pwrite
                                // `.part` + transacción SQLite) son
                                // síncronas: delegadas al grupo de hilos
                                // bloqueante de Tokio para no congelar
                                // el ejecutor bajo presión de disco.
                                let (is_complete, injected_dl) = {
                                    let downloads = Arc::clone(downloads);
                                    let file_hash = file_hash_str.to_string();
                                    let sender = address_str.to_string();
                                    tokio::task::spawn_blocking(move || {
                                        let mut dl_svc = downloads.blocking_lock();
                                        let Some(dl_id) = dl_svc.id_by_ed2k_hash(&file_hash) else {
                                            return (false, None);
                                        };
                                        let done = match dl_svc.inject_range_brief(
                                            &dl_id,
                                            part_start,
                                            &part_data,
                                            Some(&sender),
                                        ) {
                                            Ok(snap) => {
                                                let s = snap.status.to_lowercase();
                                                s == "verifying"
                                                    || s == "completed"
                                                    || snap.progress >= 1.0
                                            }
                                            Err(_) => false,
                                        };
                                        (done, Some(dl_id))
                                    })
                                    .await
                                    .unwrap_or_default()
                                };
                                if let Some(dl_id) = injected_dl {
                                    if apply_corruption_bans(
                                        inner,
                                        downloads,
                                        dl_id,
                                        file_hash_str,
                                        address_str,
                                    )
                                    .await
                                    {
                                        tracing::warn!(
                                            "[CORRUPTION_BLACKBOX] Peer {peer} banned for corrupted data — closing transfer",
                                            peer = log_peer_addr(target_addr)
                                        );
                                        break 'transfer_loop;
                                    }
                                }

                                let elapsed = last_rate_check.elapsed();
                                if elapsed >= Duration::from_millis(500) || is_complete {
                                    let bps = if elapsed >= Duration::from_millis(500) {
                                        let rate = (bytes_since_rate_check as f64 / elapsed.as_secs_f64()) as u64;
                                        bytes_since_rate_check = 0;
                                        last_rate_check = std::time::Instant::now();
                                        rate
                                    } else {
                                        inner.lock().await.sessions.get(address_str).map(|s| s.snapshot.transfer_rate_bps).unwrap_or(0)
                                    };

                                    sync_transfer_rates(
                                        inner,
                                        downloads,
                                        event_tx,
                                        sequence,
                                        address_str,
                                        file_hash_str,
                                        bps,
                                        total_received,
                                    ).await;
                                }

                                if last_dl_progress_log.elapsed() >= Duration::from_secs(10) {
                                    last_dl_progress_log = std::time::Instant::now();
                                    let dl_svc = downloads.lock().await;
                                    if let Some(dl) = dl_svc.find_by_ed2k_hash(file_hash_str) {
                                        let peer_bps = inner.lock().await.sessions.get(address_str).map(|s| s.snapshot.transfer_rate_bps).unwrap_or(0);
                                        tracing::debug!(
                                            "[DL_PROGRESS] File '{}' ({:.1}% - {:.1}/{:.1} MB) | Total Rate: {:.1} KB/s (Peer: {:.1} KB/s) | Active peer: {}",
                                            dl.file_name,
                                            dl.progress * 100.0,
                                            dl.received_bytes as f64 / (1024.0 * 1024.0),
                                            dl.total_bytes as f64 / (1024.0 * 1024.0),
                                            dl.download_rate_bps as f64 / 1024.0,
                                            peer_bps as f64 / 1024.0,
                                            log_peer_addr(target_addr)
                                        );
                                    }
                                }
                            } else if (packet.protocol == PR_EMULE || packet.protocol == PR_ED2K || packet.protocol == PR_EMULE_COMPRESSED)
                                && (packet.opcode == OP_COMPRESSEDPART || packet.opcode == OP_COMPRESSEDPART_I64)
                            {
                                let is_i64 = packet.opcode == OP_COMPRESSEDPART_I64;
                                let min_len = if is_i64 { 28 } else { 24 };
                                if packet.payload.len() < min_len {
                                    return Err(PeerError::new(PeerFailureReason::CorruptedData, "Compressed part payload too short"));
                                }

                                let mut file_hash = [0u8; 16];
                                file_hash.copy_from_slice(&packet.payload[0..16]);
                                if file_hash != dl_hash_bytes {
                                    return Err(PeerError::new(PeerFailureReason::HashMismatch, "OP_COMPRESSEDPART returned mismatched file hash"));
                                }

                                let (block_start, chunk) = if is_i64 {
                                    let bs = u64::from_le_bytes(packet.payload[16..24].try_into().unwrap_or([0; 8]));
                                    (bs, &packet.payload[28..])
                                } else {
                                    let bs = u32::from_le_bytes(packet.payload[16..20].try_into().unwrap_or([0; 4])) as u64;
                                    (bs, &packet.payload[24..])
                                };

                                tracing::info!(
                                    "Peer {peer}: received {} at offset {} (chunk {} bytes) for {file_hash_str}",
                                    if is_i64 { "OP_COMPRESSEDPART_I64" } else { "OP_COMPRESSEDPART" },
                                    block_start,
                                    chunk.len(),
                                    peer = log_peer_addr(target_addr)
                                );

                                last_block_data = tokio::time::Instant::now();
                                consecutive_stall_requeues = 0;

                                // eMule asocia el flujo zlib a la
                                // Pending_Block_Struct — localizar a qué bloque
                                // pendiente pertenece este fragmento comprimido.
                                let pending_idx = pending.iter().position(|p| {
                                    p.range.start <= block_start && block_start < p.range.end
                                });
                                let Some(pi) = pending_idx else {
                                    tracing::debug!(
                                        "Peer {peer}: OP_COMPRESSEDPART at {} matches no pending block — ignored",
                                        block_start,
                                        peer = log_peer_addr(target_addr)
                                    );
                                    continue;
                                };
                                let range_key = pending[pi].range.start;
                                let range_end = pending[pi].range.end;
                                // Tomar posesión del flujo zlib del bloque para que
                                // `pending` permanezca prestable dentro del bucle;
                                // reinsertado abajo a menos que el bloque haya terminado.
                                let mut decomp = zstreams
                                    .remove(&range_key)
                                    .unwrap_or_else(|| Decompress::new(true));
                                let mut zstream_alive = true;
                                let mut out = vec![0u8; 65536];
                                let mut input_slice = chunk;

                                while !input_slice.is_empty() {
                                    let before_in = decomp.total_in();
                                    let before_out = decomp.total_out();
                                    let res = decomp
                                        .decompress(input_slice, &mut out, FlushDecompress::Sync);
                                    let produced = (decomp.total_out() - before_out) as usize;
                                    let consumed = (decomp.total_in() - before_in) as usize;
                                    input_slice = &input_slice[consumed..];

                                    // Regular los bytes recibidos del cable a través del
                                    // cubo de fichas de descarga global.
                                    if consumed > 0 {
                                        if let Some(rl) = rate_limiters {
                                            if rl
                                                .download_bucket
                                                .acquire_tokens(consumed as u64, || {
                                                    cancel_token.is_cancelled()
                                                })
                                                .await
                                                .is_err()
                                            {
                                                break 'transfer_loop;
                                            }
                                        }
                                    }

                                    if produced > 0 {
                                        total_received += produced as u64;
                                        bytes_since_rate_check += produced as u64;
                                        credit_pending_bytes += produced as u64;
                                        if credit_pending_bytes >= CREDIT_FLUSH_BYTES {
                                            flush_download_credit(downloads, sui, &mut credit_pending_bytes).await;
                                        }
                                        let write_at = pending[pi].recv_to;
                                        // Mismo desvío que en la ruta no
                                        // comprimida: E/S síncronas en el
                                        // grupo bloqueante de Tokio (`out` se
                                        // copia — búfer zlib reutilizado).
                                        let (is_complete, injected_dl) = {
                                            let downloads = Arc::clone(downloads);
                                            let file_hash = file_hash_str.to_string();
                                            let sender = address_str.to_string();
                                            let chunk = out[..produced].to_vec();
                                            tokio::task::spawn_blocking(move || {
                                                let mut dl_svc = downloads.blocking_lock();
                                                let Some(dl_id) =
                                                    dl_svc.id_by_ed2k_hash(&file_hash)
                                                else {
                                                    return (false, None);
                                                };
                                                let done = match dl_svc.inject_range_brief(
                                                    &dl_id,
                                                    write_at,
                                                    &chunk,
                                                    Some(&sender),
                                                ) {
                                                    Ok(snap) => {
                                                        let s = snap.status.to_lowercase();
                                                        s == "verifying"
                                                            || s == "completed"
                                                            || snap.progress >= 1.0
                                                    }
                                                    Err(_) => false,
                                                };
                                                (done, Some(dl_id))
                                            })
                                            .await
                                            .unwrap_or_default()
                                        };
                                        if let Some(dl_id) = injected_dl {
                                            if apply_corruption_bans(
                                                inner,
                                                downloads,
                                                dl_id,
                                                file_hash_str,
                                                address_str,
                                            )
                                            .await
                                            {
                                                tracing::warn!(
                                                    "[CORRUPTION_BLACKBOX] Peer {peer} banned for corrupted data — closing transfer",
                                                    peer = log_peer_addr(target_addr)
                                                );
                                                break 'transfer_loop;
                                            }
                                        }
                                        pending[pi].recv_to += produced as u64;

                                        let elapsed = last_rate_check.elapsed();
                                        if elapsed >= Duration::from_millis(500) || is_complete {
                                            let bps = if elapsed >= Duration::from_millis(500) {
                                                let rate = (bytes_since_rate_check as f64 / elapsed.as_secs_f64()) as u64;
                                                bytes_since_rate_check = 0;
                                                last_rate_check = std::time::Instant::now();
                                                rate
                                            } else {
                                                inner.lock().await.sessions.get(address_str).map(|s| s.snapshot.transfer_rate_bps).unwrap_or(0)
                                            };
                                            sync_transfer_rates(
                                                inner,
                                                downloads,
                                                event_tx,
                                                sequence,
                                                address_str,
                                                file_hash_str,
                                                bps,
                                                total_received,
                                            ).await;
                                        }
                                    }

                                    match res {
                                        Ok(Status::StreamEnd) => {
                                            if pending[pi].recv_to < range_end {
                                                tracing::warn!(
                                                    "Peer {peer}: Zlib stream ended at offset {} before reaching sub-block boundary {}",
                                                    pending[pi].recv_to,
                                                    range_end
                                                , peer = log_peer_addr(target_addr));
                                            } else {
                                                tracing::debug!(
                                                    "Peer {peer}: completed decompression of block at offset {}",
                                                    pending[pi].recv_to
                                                , peer = log_peer_addr(target_addr));
                                            }
                                            zstream_alive = false;
                                            if pending[pi].recv_to >= range_end {
                                                finished_pending =
                                                    Some((pending[pi].part_idx, pending[pi].range));
                                            }
                                            break;
                                        }
                                        Ok(Status::Ok) => {
                                            if pending[pi].recv_to >= range_end {
                                                tracing::debug!(
                                                    "Peer {peer}: sub-block reached requested boundary {}",
                                                    pending[pi].recv_to
                                                , peer = log_peer_addr(target_addr));
                                                zstream_alive = false;
                                                finished_pending =
                                                    Some((pending[pi].part_idx, pending[pi].range));
                                                break;
                                            }
                                            if consumed == 0 && produced == 0 {
                                                break;
                                            }
                                        }
                                        Ok(Status::BufError) => break,
                                        Err(e) => {
                                            tracing::warn!("Peer {peer}: Zlib decompression error: {e}", peer = log_peer_addr(target_addr));
                                            break;
                                        }
                                    }
                                }
                                if zstream_alive {
                                    zstreams.insert(range_key, decomp);
                                }

                                let elapsed = last_rate_check.elapsed();
                                if elapsed >= Duration::from_millis(500) {
                                    let bps = (bytes_since_rate_check as f64 / elapsed.as_secs_f64()) as u64;
                                    bytes_since_rate_check = 0;
                                    last_rate_check = std::time::Instant::now();
                                    sync_transfer_rates(
                                        inner,
                                        downloads,
                                        event_tx,
                                        sequence,
                                        address_str,
                                        file_hash_str,
                                        bps,
                                        total_received,
                                    ).await;
                                }

                                if last_dl_progress_log.elapsed() >= Duration::from_secs(10) {
                                    last_dl_progress_log = std::time::Instant::now();
                                    let dl_svc = downloads.lock().await;
                                    if let Some(dl) = dl_svc.find_by_ed2k_hash(file_hash_str) {
                                        let peer_bps = inner.lock().await.sessions.get(address_str).map(|s| s.snapshot.transfer_rate_bps).unwrap_or(0);
                                        tracing::debug!(
                                            "[DL_PROGRESS] File '{}' ({:.1}% - {:.1}/{:.1} MB) | Total Rate: {:.1} KB/s (Peer: {:.1} KB/s) | Active peer: {}",
                                            dl.file_name,
                                            dl.progress * 100.0,
                                            dl.received_bytes as f64 / (1024.0 * 1024.0),
                                            dl.total_bytes as f64 / (1024.0 * 1024.0),
                                            dl.download_rate_bps as f64 / 1024.0,
                                            peer_bps as f64 / 1024.0,
                                            log_peer_addr(target_addr)
                                        );
                                    }
                                }
                            } else if (packet.protocol == PR_ED2K || packet.protocol == PR_EMULE) && packet.opcode == OP_END_OF_DOWNLOAD {
                                info!("Peer {} sent OP_END_OF_DOWNLOAD", log_peer_addr(target_addr));
                                break;
                            } else if packet.protocol == PR_ED2K && packet.opcode == OP_OUTOFPARTS {
                                // El subidor rotó la ranura o terminó su cuota actual:
                                // si ya recibimos datos, intentar reentrar en su cola sobre
                                // el mismo socket; si cierra el socket, concluir limpiamente.
                                if total_received > 0
                                    && consecutive_stall_requeues < MAX_CONSECUTIVE_STALL_REQUEUES
                                {
                                    consecutive_stall_requeues += 1;
                                    info!(
                                        "Peer {peer} sent OP_OUTOFPARTS after {total_received} bytes — re-queuing on same socket (#{consecutive_stall_requeues})",
                                        peer = log_peer_addr(target_addr)
                                    );
                                    match requeue_and_resume_transfer(
                                        &requeue_ctx,
                                        stream,
                                        &mut read_buf,
                                        sui,
                                        &mut file_status_refreshed,
                                        &mut pending,
                                        &mut zstreams,
                                        &mut sequencer,
                                        &mut last_block_data,
                                    )
                                    .await
                                    {
                                        Ok(RequeueOutcome::Resumed) => break,
                                        Ok(RequeueOutcome::NoMoreBlocks) => break 'transfer_loop,
                                        Err(e) => {
                                            info!(
                                                "Peer {peer} ended session after OP_OUTOFPARTS ({})",
                                                e.message,
                                                peer = log_peer_addr(target_addr)
                                            );
                                            break 'transfer_loop;
                                        }
                                    }
                                }
                                info!("Peer {peer} sent OP_OUTOFPARTS, ending transfer session", peer = log_peer_addr(target_addr));
                                break 'transfer_loop;
                            } else if super::status_query::handle_queue_rank_packet(&packet, address_str, target_addr, &decoder, inner, event_tx, sequence).await {
                            } else if packet.protocol == PR_ED2K && packet.opcode == OP_CANCELTRANSFER {
                                info!("Peer {peer} cancelled the transfer (OP_CANCELTRANSFER)", peer = log_peer_addr(target_addr));
                                break 'transfer_loop;
                            } else if packet.protocol == PR_ED2K && packet.opcode == OP_CHANGE_CLIENT_ID {
                                // `ListenSocket.cpp:590` de eMule — nuevo
                                // ID/servidor del par (servidor conocido).
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
                                handle_sx_request(&packet, Some(dl_hash_bytes), target_addr, &decoder, &encoder, downloads, stream, sx_env, sx_caps, Some(peer_part_status.as_slice())).await;
                            } else if packet.protocol == PR_EMULE
                            {
                                // Subopcodes SX en OP_MULTIPACKET* — el
                                // par también puede pedirnos fuentes
                                // en el mismo socket (ListenSocket de eMule).
                                super::source_exchange::handle_sx_multipacket(
                                    &packet,
                                    dl_hash_bytes,
                                    Some(peer_part_status.as_slice()),
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

                            // Un bloque pendiente se ha completado — vaciarlo y rellenar
                            // la canalización hasta `blockCount` (`SendBlockRequests`
                            // de eMule se vuelve a ejecutar por cada paquete recibido).
                            if let Some((fin_part, fin_range)) = finished_pending {
                                pending.retain(|p| {
                                    !(p.part_idx == fin_part
                                        && p.range.start == fin_range.start
                                        && p.range.end == fin_range.end)
                                });
                                zstreams.remove(&fin_range.start);
                                {
                                    let mut g = inner.lock().await;
                                    g.in_flight_blocks.retain(|b| {
                                        !(b.peer_id == address_str
                                            && b.start == fin_range.start
                                            && b.end == fin_range.end)
                                    });
                                }

                                let (completed, dl_still_active) = {
                                    let dl_svc = downloads.lock().await;
                                    let still_active = dl_svc
                                        .find_by_ed2k_hash(file_hash_str)
                                        .map(|s| is_download_active_status(&s.status))
                                        .unwrap_or(false);
                                    (dl_svc.get_completed_ranges_by_hash(file_hash_str), still_active)
                                };

                                // La descarga fue pausada/eliminada/falló mientras
                                // este bloque estaba en vuelo — dejar de pedir
                                // nuevos bloques (eMule pausa el partfile, no
                                // solo el planificador).
                                if !dl_still_active {
                                    info!("Peer {peer}: download no longer active, ending transfer session", peer = log_peer_addr(target_addr));
                                    break 'transfer_loop;
                                }

                                let peer_bps = inner
                                    .lock()
                                    .await
                                    .sessions
                                    .get(address_str)
                                    .map(|s| s.snapshot.transfer_rate_bps)
                                    .unwrap_or(0);
                                fill_and_send_block_requests(
                                    stream,
                                    &encoder,
                                    &mut sequencer,
                                    address_str,
                                    inner,
                                    dl_hash_bytes,
                                    peer_ext_req,
                                    &completed,
                                    &mut pending,
                                    emule_pending_block_count(peer_bps, is_emule_client),
                                    target_addr,
                                    file_size,
                                )
                                .await?;
                                // Un envío de petición cuenta como actividad de transferencia
                                // (eMule refresca `m_dwLastBlockReceived` ahí).
                                last_block_data = tokio::time::Instant::now();

                                let cur_part = pending.front().map(|p| p.part_idx);
                                {
                                    let mut g = inner.lock().await;
                                    if let Some(session) = g.sessions.get_mut(address_str) {
                                        session.snapshot.current_part = cur_part;
                                    }
                                }

                                if pending.is_empty() {
                                    tracing::info!("Peer {peer}: all blocks completed or no more blocks needed", peer = log_peer_addr(target_addr));
                                    let mut end_wire = Vec::with_capacity(6 + 16);
                                    end_wire.push(PR_ED2K);
                                    end_wire.extend_from_slice(&(17u32).to_le_bytes());
                                    end_wire.push(OP_END_OF_DOWNLOAD);
                                    end_wire.extend_from_slice(&dl_hash_bytes);
                                    let _ = stream.write_all(&end_wire).await;
                                    let _ = stream.flush().await;

                                    break 'transfer_loop;
                                }
                            }
                        }
                    }
                    Err(e) => return Err(PeerError::new(PeerFailureReason::ConnectRefused, format!("Read error during transfer: {e}"))),
                }
            }
        }
    }

    // Restablecer la tasa para esta sesión de par y actualizar la tasa agregada del archivo
    {
        let (total_remaining_bps, pending_ranges): (u64, Vec<(u64, u64)>) = {
            let mut g = inner.lock().await;
            if let Some(session) = g.sessions.get_mut(address_str) {
                session.snapshot.transfer_rate_bps = 0;
            }
            let total = g
                .sessions
                .values()
                .filter(|s| s.snapshot.file_hash.as_deref() == Some(file_hash_str))
                .map(|s| s.snapshot.transfer_rate_bps)
                .sum();
            let pending = PeerSessionManager::pending_ranges_locked(&g, file_hash_str);
            (total, pending)
        };
        let mut dl_svc = downloads.lock().await;
        if let Some(mut snap) = dl_svc.set_download_rate_by_hash(file_hash_str, total_remaining_bps)
        {
            snap.pending_ranges = pending_ranges;
            emit_download_update(event_tx, sequence, &snap);
        }
    }

    // Persistir los bytes acreditados restantes para esta sesión.
    flush_download_credit(downloads, sui, &mut credit_pending_bytes).await;

    Ok(total_received)
}

/// Umbral de lote para la contabilidad de créditos de eMule (8 MiB) — acota cuántas actualizaciones
/// de `peer_credits` en SQLite realiza una sesión rápida.
const CREDIT_FLUSH_BYTES: u64 = 8 * 1024 * 1024;

/// Persiste los bytes descargados acumulados bajo la identidad de clave pública SUI
/// *verificada* del par (eMule acredita la clave de cliente, nunca la IP transitoria).
/// Los pares no verificados no acumulan nada — `peer_credits` se indexa por hash de clave pública.
async fn flush_download_credit(
    downloads: &Arc<Mutex<DownloadService>>,
    sui: &super::sui::OutboundSui,
    pending: &mut u64,
) {
    if *pending == 0 {
        return;
    }
    if let Some((pubkey_hash, pubkey_hex)) = sui.verified_credit_identity() {
        let mut dl_svc = downloads.lock().await;
        if dl_svc
            .record_credit_download(&pubkey_hash, &pubkey_hex, *pending)
            .is_ok()
        {
            tracing::debug!(
                "[CREDITS] {} bytes downloaded credited to peer {}…",
                pending,
                &pubkey_hash[..16.min(pubkey_hash.len())]
            );
        }
        *pending = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::emule_pending_block_count;

    /// Niveles de `blockCount` de `SendBlockRequests` de eMule (DownloadClient.cpp):
    /// los clientes eMule por debajo de 9 KiB/s obtienen los niveles bajos 1/2; el resto
    /// obtiene los niveles 3/6/9 independientemente de la familia de clientes.
    #[test]
    fn test_emule_pending_block_count_tiers() {
        // Tasa inicial/no medida toma por defecto 3 bloques para canalización inmediata
        assert_eq!(emule_pending_block_count(0, true), 3);
        // Cliente eMule, niveles de tasa baja tras medir la tasa
        assert_eq!(emule_pending_block_count(1, true), 1);
        assert_eq!(emule_pending_block_count(4 * 1024 - 1, true), 1);
        assert_eq!(emule_pending_block_count(4 * 1024, true), 2);
        assert_eq!(emule_pending_block_count(9 * 1024 - 1, true), 2);
        // Cliente eMule, niveles superiores
        assert_eq!(emule_pending_block_count(9 * 1024, true), 3);
        assert_eq!(emule_pending_block_count(75 * 1024 + 1, true), 6);
        assert_eq!(emule_pending_block_count(150 * 1024 + 1, true), 9);
        // Cliente no eMule: omite los niveles bajos por completo
        assert_eq!(emule_pending_block_count(0, false), 3);
        assert_eq!(emule_pending_block_count(2 * 1024, false), 3);
        assert_eq!(emule_pending_block_count(200 * 1024, false), 9);
    }
}

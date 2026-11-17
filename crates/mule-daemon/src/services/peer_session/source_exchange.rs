// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Gestión del intercambio de fuentes (Source Exchange, SX/SX2) alineada con eMule 0.72a.
//!
//! Referencias: `ListenSocket.cpp:970-1011,1165-1251` (despacho + limitación de tasa),
//! `PartFile.cpp:2368-2405` (`CanAddSource`), `PartFile.cpp:3602-3712`
//! (`CreateSrcInfoPacket`), `PartFile.cpp:3714-3890` (`AddClientSources`),
//! `DownloadClient.cpp:212-255` (`IsSourceRequestAllowed`),
//! `Opcodes.h:69-115` (`SOURCECLIENTREASKS`, `SOURCECLIENTREASKF`,
//! `MINCOMMONPENALTY`, `RARE_FILE`, `CONNECTION_LATENCY`).

use crate::services::peer_session::log_peer_addr;
use mule_protocol::ed2k::{
    Ed2kAnswerSources, Ed2kDecoder, Ed2kEncoder, Ed2kRequestSources, SxSource, OP_ANSWERSOURCES,
    OP_ANSWERSOURCES2, OP_REQUESTSOURCES, OP_REQUESTSOURCES2, PR_EMULE, PR_EMULE_COMPRESSED,
    SOURCE_EXCHANGE2_VERSION,
};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use mule_network_policy::{validate_connect_address, NetworkPolicyManager};

use crate::services::downloads::DownloadService;
use crate::services::obfuscation_stream::ObfuscatedStream;

/// `SOURCECLIENTREASKS` de eMule (`Opcodes.h:69`) — 40 min entre dos peticiones
/// SX hacia el mismo cliente.
pub const SOURCECLIENTREASKS: Duration = Duration::from_secs(40 * 60);

/// `SOURCECLIENTREASKF` de eMule (`Opcodes.h:70`) — 5 min entre dos respuestas
/// SX recibidas para el mismo archivo (ventana de «fuentes frescas»).
pub const SOURCECLIENTREASKF: Duration = Duration::from_secs(5 * 60);

/// `MINCOMMONPENALTY` de eMule (`Opcodes.h:90`) — penalización ×4 en los temporizadores
/// para archivos no raros.
pub const MINCOMMONPENALTY: u32 = 4;

/// `RARE_FILE` de eMule (`Opcodes.h:115`) — un archivo con ≤ 50 fuentes es
/// «raro» y se beneficia de temporizadores más cortos.
pub const RARE_FILE: usize = 50;

/// `CONNECTION_LATENCY` de eMule (`Opcodes.h:120`) — margen de latencia añadido
/// a las comparaciones de tiempo (22,05 s).
pub const CONNECTION_LATENCY: Duration = Duration::from_millis(22050);

/// Capacidad de cable SX de eMule: `CreateSrcInfoPacket` corta tras
/// `if (nCount > 500) break;` — 501 entradas como máximo por respuesta.
const SX_RESPONSE_MAX_SOURCES: usize = 501;

/// Límite de fuentes conocidas por archivo cuando `.part.met` no fija
/// `FT_MAXSOURCES` — `GetMaxSourcePerFileDefault() = 400` de eMule
/// (`Preferences.cpp:1976`).
const DEFAULT_MAX_SOURCES_PER_FILE: usize = 400;

/// Estado compartido de Source Exchange: política de red, identidad local y
/// registros de limitación de tasa. Una única instancia creada en `AppState`,
/// inyectada en `PeerSessionManager` e `InboundListenerService` — equivalente
/// a los contadores de eMule repartidos entre `CUpDownClient` y `CPartFile`.
#[derive(Clone)]
pub struct SxEnv {
    /// Política de red (clasificación IP + ipfilter) — equivalente a
    /// `IsGoodIP` + `theApp.ipfilter->IsFiltered` de eMule, más estricta.
    pub network_policy: NetworkPolicyManager,
    /// Última respuesta SX enviada a cada par — `CUpDownClient::m_dwLastSourceRequest`
    /// de eMule (lado servidor SX).
    pub answer_log: Arc<std::sync::Mutex<HashMap<IpAddr, Instant>>>,
    /// Última petición SX emitida hacia cada par — `CUpDownClient::GetLastAskedForSources()`
    /// de eMule (lado cliente SX).
    pub request_log: Arc<std::sync::Mutex<HashMap<IpAddr, Instant>>>,
    /// Última respuesta SX recibida por archivo — `CPartFile::m_ClientSrcAnswered`
    /// (`GetLastAnsweredTime`) de eMule.
    pub file_answer_log: Arc<std::sync::Mutex<HashMap<[u8; 16], Instant>>>,
    /// Nuestro user hash MD4 — las fuentes que lo llevan somos nosotros mismos
    /// (eMule descarta `userhash == own` al añadir, `DownloadQueue.cpp:479`).
    pub own_user_hash: [u8; 16],
}

impl SxEnv {
    /// Crea un entorno SX vacío.
    pub fn new(network_policy: NetworkPolicyManager, own_user_hash: [u8; 16]) -> Self {
        Self {
            network_policy,
            answer_log: Arc::new(std::sync::Mutex::new(HashMap::new())),
            request_log: Arc::new(std::sync::Mutex::new(HashMap::new())),
            file_answer_log: Arc::new(std::sync::Mutex::new(HashMap::new())),
            own_user_hash,
        }
    }
}

/// Identidad local eD2K para las comprobaciones `CanAddSource` de eMule
/// (`PartFile.cpp:2380-2403`): detectar fuentes que nos señalan a nosotros
/// mismos o a un destino inalcanzable.
#[derive(Clone, Copy, Debug, Default)]
pub struct SxLocalIds {
    /// `client_id` asignado por el servidor eD2K actual (0 = desconectado).
    pub client_id: u32,
    /// IPv4 del servidor actual (formato hybrid little-endian), 0 si se desconoce.
    pub server_ip: u32,
    /// Puerto del servidor actual.
    pub server_port: u16,
    /// Notre port TCP d'écoute eD2K.
    pub tcp_port: u16,
    /// `true` cuando nuestro `client_id` es un HighID (no `IsLowID`).
    pub is_high_id: bool,
    /// `CKademlia::IsConnected() && !IsFirewalled()` — junto con `is_high_id`,
    /// forma `!theApp.IsFirewalled()`: la unión determina si una LowID
    /// puede devolvernos la llamada.
    pub kad_open: bool,
}

/// Capacidades SX anunciadas por el par remoto + estado de compleción.
#[derive(Clone, Copy, Debug, Default)]
pub struct PeerSxCaps {
    /// Versión SX1 anunciada por el par (`m_bySourceExchange1Ver`,
    /// bits 12-15 de CT_EMULE_MISCOPTIONS1 o etiqueta `ET_SOURCEEXCHANGE`).
    /// `0` = el par no anunció SX1.
    pub sx1_version: u8,
    /// SX2 anunciado (`SupportsSourceExchange2`, bit 10 de MISCOPTIONS2).
    pub sx2: bool,
    /// `m_bCompleteSource`: el par posee el archivo completo — eMule no
    /// pide fuentes a una fuente completa.
    pub complete_source: bool,
    /// Envoltorio multipaquete negociado (`SendFileRequest`,
    /// `DownloadClient.cpp:266-280`).
    pub multipacket: MultiPacketLevel,
    /// `SupportsFileIdentifiers()` (bit 13 de MISCOPTIONS2) — controla la elección
    /// entre `OP_HASHSETREQUEST` y `OP_HASHSETREQUEST2`
    /// (`SendHashSetRequest`, `DownloadClient.cpp:2200`).
    pub file_identifiers: bool,
}

/// Nivel de envoltorio multipaquete anunciado por el par — `SupportMultiPacket`
/// de eMule (bit 1 de MISCOPTIONS1), `SupportExtMultiPacket` (bit 5 de MISCOPTIONS2),
/// `SupportsFileIdentifiers` (bit 13 de MISCOPTIONS2).
/// eMule elige la variante admitida más alta en `SendFileRequest`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MultiPacketLevel {
    /// Sin multipaquete — peticiones emitidas en paquetes separados.
    #[default]
    None,
    /// `OP_MULTIPACKET` (`0x92`) — envoltorio solo con hash.
    Basic,
    /// `OP_MULTIPACKET_EXT` (`0xA4`) — hash + tamaño u64.
    Ext,
    /// `OP_MULTIPACKET_EXT2` (`0xA9`) — `FileIdentifier`
    /// (hash + tamaño + raíz AICH).
    Ext2,
}

/// `CUpDownClient::IsSourceRequestAllowed` de eMule (`DownloadClient.cpp:212-255`)
/// — versión monoarchivo: nuestras sesiones están vinculadas a un único archivo
/// (`partfile == m_reqfile`, `sourceExchangeCheck = false`), por lo que las cláusulas
/// A4AF desaparecen.
///
/// - `source_count`: `GetSourceCount()` — número total de fuentes conocidas.
/// - `valid_source_count`: `GetValidSourcesCount()` — fuentes en un estado
///   viable (no filtradas/rechazadas).
/// - `max_source_soft`: `GetMaxSourcePerFileSoft()` — límite flexible del archivo
///   (`max_sources * 9/10`, por defecto 360).
pub fn is_source_request_allowed(
    env: &SxEnv,
    peer_ip: IpAddr,
    file_hash: &[u8; 16],
    source_count: usize,
    valid_source_count: usize,
    max_source_soft: usize,
    caps: &PeerSxCaps,
) -> bool {
    // `ExtProtocolAvailable() && (SupportsSourceExchange2() || sx1 > 1)`
    if !(caps.sx2 || caps.sx1_version > 1) {
        return false;
    }
    // `m_reqfile->GetMaxSourcePerFileSoft() > uSources`
    if source_count >= max_source_soft {
        return false;
    }

    let last_req = env
        .request_log
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&peer_ip)
        .copied();
    let never_asked = last_req.is_none();
    let client_elapsed = last_req
        .map(|t| t.elapsed().saturating_add(CONNECTION_LATENCY))
        .unwrap_or(Duration::MAX);
    let file_elapsed = env
        .file_answer_log
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(file_hash)
        .map(|t| t.elapsed().saturating_add(CONNECTION_LATENCY))
        .unwrap_or(Duration::MAX);

    // Fuente no completa, archivo muy raro (≤ RARE_FILE/5).
    (!caps.complete_source
        && (never_asked || client_elapsed > SOURCECLIENTREASKS)
        && source_count <= RARE_FILE / 5)
        // Fuente no completa, archivo raro (≤ RARE_FILE, o
        // ≤ RARE_FILE/2 + fuentes válidas cuando ya tenemos válidas).
        || (!caps.complete_source
            && (never_asked || client_elapsed > SOURCECLIENTREASKS)
            && (source_count <= RARE_FILE
                || source_count <= RARE_FILE / 2 + valid_source_count)
            && file_elapsed > SOURCECLIENTREASKF)
        // Archivo no raro: cliente > 160 min y archivo > 20 min.
        || ((never_asked || client_elapsed > SOURCECLIENTREASKS * MINCOMMONPENALTY)
            && file_elapsed > SOURCECLIENTREASKF * MINCOMMONPENALTY)
}

/// `SetLastAskedForSources()` + `CPartFile::SetLastAnsweredTimeTimeout()` de eMule
/// (`PartFile.h:267`) — registra la emisión de una petición SX y retrasa el
/// temporizador del archivo para no bloquear la siguiente repetición de petición
/// (`m_ClientSrcAnswered = now + 2*CONNECTION_LATENCY - SOURCECLIENTREASKS`).
pub fn mark_sx_request_sent(env: &SxEnv, peer_ip: IpAddr, file_hash: [u8; 16]) {
    env.request_log
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(peer_ip, Instant::now());
    let back = SOURCECLIENTREASKS.saturating_sub(2 * CONNECTION_LATENCY);
    let pseudo = Instant::now()
        .checked_sub(back)
        .unwrap_or_else(Instant::now);
    env.file_answer_log
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(file_hash, pseudo);
}

/// `CPartFile::SetLastAnsweredTime()` de eMule — registra la recepción de una
/// respuesta SX válida para este archivo (utilizado por `IsSourceRequestAllowed`).
pub fn mark_sx_answer_received(env: &SxEnv, file_hash: [u8; 16]) {
    env.file_answer_log
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(file_hash, Instant::now());
}

/// Versión SX1 anunciada en un `OP_EMULEINFO` — valor de la etiqueta
/// `ET_SOURCEEXCHANGE` (`ProcessMuleInfoPacket` de eMule lee el valor como
/// `m_bySourceExchange1Ver`).
pub fn emule_info_sx_version(info: &mule_protocol::ed2k::Ed2kEmuleInfo) -> u8 {
    use mule_protocol::ed2k::tags::{TagName, TagValue, TAG_SPECIAL_EMULE_EXT_SOURCEEXCHANGE};
    info.tags
        .iter()
        .find_map(|t| {
            if !matches!(
                t.name,
                TagName::Id(TAG_SPECIAL_EMULE_EXT_SOURCEEXCHANGE) | TagName::Id(0x04)
            ) {
                return None;
            }
            match t.value {
                TagValue::UInt32(v) => u8::try_from(v).ok(),
                TagValue::UInt16(v) => u8::try_from(v).ok(),
                TagValue::UInt8(v) => Some(v),
                _ => None,
            }
        })
        .unwrap_or(0)
}

/// `SendFileRequest` de eMule (`DownloadClient.cpp:266-280`) — elige
/// el envoltorio multipaquete más rico anunciado por el par:
/// `SupportsFileIdentifiers()` → EXT2, si no `SupportExtMultiPacket()` →
/// EXT, si no `SupportMultiPacket()` → BASIC.
pub fn multipacket_level(info: &mule_protocol::ed2k::Ed2kPeerInfo) -> MultiPacketLevel {
    if info.supports_file_identifiers() {
        MultiPacketLevel::Ext2
    } else if info.supports_ext_multi_packet() {
        MultiPacketLevel::Ext
    } else if info.supports_multi_packet() {
        MultiPacketLevel::Basic
    } else {
        MultiPacketLevel::None
    }
}

/// Construye las capacidades SX de un par a partir de su `Ed2kPeerInfo` (HELLO)
/// y su eventual `OP_EMULEINFO` — `GetSourceExchange1Version()` de eMule
/// (bits 12-15 de misc1) con repliegue sobre el valor de la etiqueta `ET_SOURCEEXCHANGE`.
pub fn peer_sx_caps(
    info: &mule_protocol::ed2k::Ed2kPeerInfo,
    emule_info: Option<&mule_protocol::ed2k::Ed2kEmuleInfo>,
) -> PeerSxCaps {
    PeerSxCaps {
        sx1_version: info
            .source_exchange1_version()
            .max(emule_info.map(emule_info_sx_version).unwrap_or(0)),
        sx2: info.supports_source_exchange2(),
        complete_source: false,
        multipacket: multipacket_level(info),
        file_identifiers: info.supports_file_identifiers(),
    }
}

/// Número de fuentes «válidas» en el sentido de `GetValidSourcesCount()` de eMule —
/// fuentes en un estado aprovechable (no filtradas, rechazadas o en fallo).
fn valid_source_count(sources: &[mule_core::model::PeerSource]) -> usize {
    sources
        .iter()
        .filter(|s| {
            !matches!(
                s.status,
                mule_core::model::PeerSourceStatus::Failed
                    | mule_core::model::PeerSourceStatus::Rejected
                    | mule_core::model::PeerSourceStatus::Filtered
            )
        })
        .count()
}

/// Límite flexible `GetMaxSourcePerFileSoft()` de eMule — `max_sources * 9/10`
/// (`PartFile.h:281`). Sin anulación por archivo (FT_MAXSOURCES), el
/// ajuste global `max_peers_per_download` prevalece — igual que eMule que
/// recurre a `thePrefs.maxsourceperfile`.
fn max_source_soft(
    snap: Option<&crate::services::downloads::types::DownloadSnapshot>,
    dl_svc: &crate::services::downloads::DownloadService,
) -> usize {
    let global = dl_svc
        .get_setting("max_peers_per_download")
        .ok()
        .flatten()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_SOURCES_PER_FILE);
    snap.and_then(|d| d.max_sources)
        .map(|m| (m as usize) * 9 / 10)
        .unwrap_or(global * 9 / 10)
}

/// Evalúa `IsSourceRequestAllowed` y devuelve la petición SX a emitir,
/// sin enviarla — factorización de `maybe_send_sx_request` para la
/// ruta multipaquete (`SendFileRequest` de eMule agrupa la petición SX
/// en el envoltorio `OP_MULTIPACKET*` cuando el par lo admite).
pub(crate) async fn planned_sx_request(
    target_addr: SocketAddr,
    file_hash_str: &str,
    dl_hash_bytes: [u8; 16],
    downloads: &Arc<Mutex<DownloadService>>,
    env: &SxEnv,
    caps: &PeerSxCaps,
) -> Option<Ed2kRequestSources> {
    let (source_count, valid_count, soft) = {
        let dl_svc = downloads.lock().await;
        let sources = dl_svc.list_peer_sources(file_hash_str).unwrap_or_default();
        (
            sources.len(),
            valid_source_count(&sources),
            max_source_soft(dl_svc.find_by_ed2k_hash(file_hash_str).as_ref(), &dl_svc),
        )
    };
    if !is_source_request_allowed(
        env,
        target_addr.ip(),
        &dl_hash_bytes,
        source_count,
        valid_count,
        soft,
        caps,
    ) {
        return None;
    }
    Some(if caps.sx2 {
        Ed2kRequestSources::new_v2(dl_hash_bytes)
    } else {
        Ed2kRequestSources::new_v1(dl_hash_bytes)
    })
}

/// Evalúa `IsSourceRequestAllowed` y luego emite `OP_REQUESTSOURCES(2)` hacia el
/// par — eMule envía la petición SX en cada `SendFileRequest` permitido
/// (~ciclo de re-ask); `mark_sx_request_sent` solo se llama en caso de escritura
/// exitosa (`SetLastAskedForSources` + `SetLastAnsweredTimeTimeout`).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn maybe_send_sx_request(
    stream: &mut ObfuscatedStream<TcpStream>,
    encoder: &Ed2kEncoder,
    target_addr: SocketAddr,
    file_hash_str: &str,
    dl_hash_bytes: [u8; 16],
    downloads: &Arc<Mutex<DownloadService>>,
    env: &SxEnv,
    caps: &PeerSxCaps,
) {
    let Some(sx_req) = planned_sx_request(
        target_addr,
        file_hash_str,
        dl_hash_bytes,
        downloads,
        env,
        caps,
    )
    .await
    else {
        return;
    };
    match encoder.encode_request_sources(&sx_req) {
        Ok(pkt) => {
            let mut out = Vec::with_capacity(6 + pkt.payload.len());
            out.push(pkt.protocol);
            out.extend_from_slice(&(pkt.frame_len() as u32).to_le_bytes());
            out.push(pkt.opcode);
            out.extend_from_slice(&pkt.payload);
            use tokio::io::AsyncWriteExt;
            match stream.write_all(&out).await {
                Ok(()) => {
                    mark_sx_request_sent(env, target_addr.ip(), dl_hash_bytes);
                    // `DebugSourceExchange` de eMule (verbose opcional) — la
                    // respuesta lleva el balance útil (rendimiento), la petición
                    // aislada es solo ruido (~1/fuente/40 min por archivo).
                    tracing::debug!(
                        "[SOURCE_EXCHANGE] Sent {} (version {}) to peer {peer} for file {file_hash_str}",
                        if caps.sx2 { "OP_REQUESTSOURCES2" } else { "OP_REQUESTSOURCES" },
                        sx_req.version
                    , peer = log_peer_addr(target_addr));
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to send SX request to {peer}: {e}",
                        peer = log_peer_addr(target_addr)
                    );
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                "Failed to encode SX request for {peer}: {e}",
                peer = log_peer_addr(target_addr)
            );
        }
    }
}

/// `true` si la IP/puerto de la fuente supera el filtro de red (equivalente a
/// `IsGoodIP` + `ipfilter->IsFiltered` — más estricto que eMule: rechaza
/// LAN/loopback/reservado en perfil público, permite loopback en LocalLab).
fn source_address_allowed(env: &SxEnv, ip_addr: Ipv4Addr, port: u16) -> bool {
    let sock = SocketAddr::new(IpAddr::V4(ip_addr), port);
    if validate_connect_address(&sock, env.network_policy.profile()).is_err() {
        return false;
    }
    env.network_policy
        .ip_filter()
        .check_ip(IpAddr::V4(ip_addr))
        .is_none()
}

/// Procesa una respuesta SX (`OP_ANSWERSOURCES`/`OP_ANSWERSOURCES2`) — `CPartFile::AddClientSources`
/// + `CanAddSource` de eMule (`PartFile.cpp:3714-3890, 2368-2405`).
///
/// Filtrado en la ingesta, reflejo de eMule (con nuestra política más estricta):
/// - hash del archivo esperado;
/// - `user_hash == own` → descartar;
/// - HighID: `IsGoodIP` (política), `ipfilter`, autocomprobación `client_id`+puerto;
/// - LowID: conservado como `lowid:<id>` (devolución de llamada del servidor), salvo si nosotros
///   mismos somos LowID (`IsFirewalled`) o si es nuestro propio ID;
/// - parada en `GetMaxSources()` (`max_sources` o 400 por defecto).
pub async fn handle_sx_answer(
    packet: &mule_protocol::framing::RawPacket,
    dl_hash_bytes: [u8; 16],
    file_hash_str: &str,
    target_addr: SocketAddr,
    decoder: &Ed2kDecoder,
    downloads: &Arc<Mutex<DownloadService>>,
    env: &SxEnv,
    local: &SxLocalIds,
) {
    if !((packet.protocol == PR_EMULE || packet.protocol == PR_EMULE_COMPRESSED)
        && (packet.opcode == OP_ANSWERSOURCES || packet.opcode == OP_ANSWERSOURCES2))
    {
        return;
    }

    let ans = match decoder.decode_answer_sources(packet.protocol, packet.opcode, &packet.payload) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(
                "Peer {peer}: failed to decode SX answer: {e}",
                peer = log_peer_addr(target_addr)
            );
            return;
        }
    };

    if ans.file_hash != dl_hash_bytes {
        tracing::debug!(
            "[SOURCE_EXCHANGE] Ignored SX answer from {peer}: file {} is not the requested download {file_hash_str}",
            hex::encode(ans.file_hash)
        , peer = log_peer_addr(target_addr));
        return;
    }

    // `SetLastAnsweredTime` de eMule: una respuesta válida retrasa la repetición de petición.
    mark_sx_answer_received(env, dl_hash_bytes);

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    // Límite de ingesta: `GetSourceCount() >= GetMaxSources()` detiene el
    // bucle (fuentes ya conocidas + nuevas aceptadas).
    let (existing_count, max_sources) = {
        let dl_svc = downloads.lock().await;
        let max = dl_svc
            .find_by_ed2k_hash(file_hash_str)
            .and_then(|d| d.max_sources)
            .map(|m| m as usize)
            .or_else(|| {
                // `GetMaxSources()` de eMule: sin anulación por archivo, el
                // ajuste global `max_peers_per_download` prevalece.
                dl_svc
                    .get_setting("max_peers_per_download")
                    .ok()
                    .flatten()
                    .and_then(|s| s.trim().parse::<usize>().ok())
            })
            .unwrap_or(DEFAULT_MAX_SOURCES_PER_FILE);
        (
            dl_svc
                .list_peer_sources(file_hash_str)
                .map(|v| v.len())
                .unwrap_or(0),
            max,
        )
    };

    let mut discovered_sources = Vec::new();
    let mut dropped_lowid_firewalled = 0usize;
    let mut dropped_filtered = 0usize;
    let mut dropped_self = 0usize;

    for src in ans.sources {
        if existing_count + discovered_sources.len() >= max_sources {
            break;
        }
        // Fuente que lleva nuestro user hash: somos nosotros.
        if src.user_hash == Some(env.own_user_hash) {
            dropped_self += 1;
            continue;
        }

        let addr_str = if src.is_low_id() {
            // `CanAddSource` de eMule: LowID rechazada cuando
            // `theApp.IsFirewalled()` — ni HighID eD2K ni UDP de Kad abierto,
            // por lo que es imposible recibir su devolución de conexión.
            if !local.is_high_id && !local.kad_open {
                dropped_lowid_firewalled += 1;
                continue;
            }
            // Autocomprobación LowID: nuestro propio client_id en nuestro servidor.
            if local.client_id != 0
                && src.ip == local.client_id
                && src.server_ip == local.server_ip
                && src.server_port == local.server_port
            {
                dropped_self += 1;
                continue;
            }
            format!("lowid:{}", src.ip)
        } else {
            if src.port == 0 {
                dropped_filtered += 1;
                continue;
            }
            let ip_addr = Ipv4Addr::from(src.ip_bytes);
            // `IsGoodIP` + `ipfilter->IsFiltered` (política de red del proyecto).
            if !source_address_allowed(env, ip_addr, src.port) {
                dropped_filtered += 1;
                continue;
            }
            // Autocomprobación HighID: nuestro client_id + nuestro puerto
            // (`client_id` HighID = nuestra IP en orden host/little-endian o el u32 asignado).
            if local.client_id >= 16_777_216
                && (u32::from_le_bytes(src.ip_bytes) == local.client_id
                    || u32::from_be_bytes(src.ip_bytes) == local.client_id)
                && src.port == local.tcp_port
            {
                dropped_self += 1;
                continue;
            }
            format!("{ip_addr}:{}", src.port)
        };

        let hex_user_hash = src.user_hash.map(hex::encode);
        discovered_sources.push(mule_core::model::PeerSource {
            file_hash: file_hash_str.to_string(),
            address: addr_str,
            ip: src.ip,
            port: src.port,
            origin: mule_core::model::PeerSourceOrigin::SourceExchange,
            discovered_at_unix_ms: now_ms,
            last_seen_unix_ms: now_ms,
            status: mule_core::model::PeerSourceStatus::Candidate,
            fail_count: 0,
            last_error: None,
            cooldown_until_unix_ms: None,
            user_hash: hex_user_hash,
            crypt_options: src.crypt_options,
            buddy_id: None,
            software: None,
            availability: None,
            // Servidor de la fuente (`m_dwServerIP`/`m_nServerPort` de eMule) — las
            // LowIDs solo son únicas por servidor: la devolución de llamada del servidor
            // solo es válida si `IsLocalServer` coincide con nuestro servidor.
            server_ip: src.server_ip,
            server_port: src.server_port,
            udp_port: 0,
        });
    }

    if discovered_sources.is_empty() {
        tracing::debug!(
            "[SOURCE_EXCHANGE] SX answer from {peer} for file {file_hash_str} contained no usable source (filtered={dropped_filtered}, lowid_firewalled={dropped_lowid_firewalled}, self={dropped_self})"
        , peer = log_peer_addr(target_addr));
    } else {
        let mut dl_svc = downloads.lock().await;
        let inserted = dl_svc
            .upsert_peer_sources(file_hash_str, &discovered_sources)
            .unwrap_or(0);
        let total_swarm = dl_svc
            .find_by_ed2k_hash(file_hash_str)
            .map(|d| d.source_count)
            .unwrap_or(0);
        tracing::info!(
            "[SOURCE_EXCHANGE] Received {} sources via SX from peer {} for file {} -> {} new unique sources persisted (total swarm: {} sources, dropped: filtered={} lowid_fw={} self={})",
            discovered_sources.len(),
            log_peer_addr(target_addr),
            file_hash_str,
            inserted,
            total_swarm,
            dropped_filtered,
            dropped_lowid_firewalled,
            dropped_self
        );
    }
}

/// Selección de fuentes a servir — `CreateSrcInfoPacket` de eMule
/// (`PartFile.cpp:3652-3698`):
/// - `!HasLowID() && IsValidSource()`: únicamente fuentes verificadas
///   (estados `Connected`/`Allowed` — las `Candidate` nunca contactadas no
///   se re-propagan);
/// - `srcstatus` conocido: máscara de disponibilidad presente, no vacía, y cuya
///   longitud coincide con el número de partes del archivo;
/// - ≥ 1 parte útil para el solicitante cuando se conoce su estado de partes
///   (`reqstatus`), de lo contrario ≥ 1 parte en cualquier caso;
/// - límite `nCount > 500` → 501 entradas como máximo.
fn select_sx_response_sources(
    known_sources: &[mule_core::model::PeerSource],
    file_part_count: usize,
    requester_mask: Option<&[u8]>,
    env: &SxEnv,
) -> Vec<SxSource> {
    let mut out = Vec::new();
    for src in known_sources {
        // `!cur_src->HasLowID()` — las LowID nunca se sirven en SX.
        if src.ip < 16_777_216 || src.port == 0 {
            continue;
        }
        // `IsValidSource()` — solo fuentes verificadas.
        if !matches!(
            src.status,
            mule_core::model::PeerSourceStatus::Connected
                | mule_core::model::PeerSourceStatus::Allowed
        ) {
            continue;
        }
        let ip_addr = Ipv4Addr::from(src.ip.to_le_bytes());
        if !source_address_allowed(env, ip_addr, src.port) {
            continue;
        }
        // `srcstatus && cur_src->GetPartCount() == GetPartCount()`:
        // la máscara debe conocerse y ser compatible con el archivo.
        let Some(mask) = src.availability.as_deref() else {
            continue;
        };
        if file_part_count > 0 && mask.len() != file_part_count.div_ceil(8) {
            continue;
        }
        // `srcstatus[i] && (!reqstatus || !reqstatus[i])`: la fuente debe
        // cubrir ≥ 1 parte que el solicitante no tenga (o ≥ 1 parte si se
        // desconoce su estado de partes).
        let useful = (0..mask.len() * 8).any(|i| {
            mask[i / 8] & (1 << (i % 8)) != 0
                && requester_mask
                    .is_none_or(|req| i / 8 >= req.len() || req[i / 8] & (1 << (i % 8)) == 0)
        });
        if !useful {
            continue;
        }
        out.push(SxSource {
            ip_bytes: src.ip.to_le_bytes(),
            ip: src.ip,
            port: src.port,
            // ServerIP/ServerPort no persistidos en nuestro modelo — 0 es
            // lícito en el cable (eMule los envía cuando se conocen).
            server_ip: 0,
            server_port: 0,
            user_hash: src.user_hash_bytes(),
            crypt_options: src.crypt_options,
        });
        // eMule : `if (nCount > 500) break;`
        if out.len() >= SX_RESPONSE_MAX_SOURCES {
            break;
        }
    }
    out
}

/// Procesa una petición SX entrante (`OP_REQUESTSOURCES`/`OP_REQUESTSOURCES2`)
/// — `ListenSocket.cpp:1165-1228` + `CreateSrcInfoPacket` de eMule.
///
/// - Filtro `byRequestedVersion > 0 || sx1_version > 1`;
/// - el archivo debe conocerse (compartido o en descarga);
/// - limitación `SetLastSrcReqTime`: nunca respondido → OK, part-file raro
///   → > 40 min, de lo contrario > 160 min;
/// - versión negociada: `min(requested, 4)` en SX2, si no SX1 a la versión
///   anunciada por el par; sin respuesta si `nCount == 0`;
/// - `requester_mask`: estado de partes del solicitante cuando se conoce (ext-info
///   `OP_REQFILENAME`), utilizado para servir solo fuentes útiles.
#[allow(clippy::too_many_arguments)]
pub async fn handle_sx_request(
    packet: &mule_protocol::framing::RawPacket,
    dl_hash_bytes: Option<[u8; 16]>,
    target_addr: SocketAddr,
    decoder: &Ed2kDecoder,
    encoder: &Ed2kEncoder,
    downloads: &Arc<Mutex<DownloadService>>,
    stream: &mut ObfuscatedStream<TcpStream>,
    env: &SxEnv,
    caps: &PeerSxCaps,
    requester_mask: Option<&[u8]>,
) {
    if packet.protocol != PR_EMULE
        || (packet.opcode != OP_REQUESTSOURCES && packet.opcode != OP_REQUESTSOURCES2)
    {
        return;
    }
    match decoder.decode_request_sources(packet.protocol, packet.opcode, &packet.payload) {
        Ok(req) => {
            respond_to_sx_request(
                &req,
                packet.opcode == OP_REQUESTSOURCES2,
                dl_hash_bytes,
                target_addr,
                encoder,
                downloads,
                stream,
                env,
                caps,
                requester_mask,
            )
            .await;
        }
        Err(e) => {
            tracing::warn!(
                "Peer {peer}: failed to decode SX request: {e}",
                peer = log_peer_addr(target_addr)
            );
        }
    }
}

/// Responde a una petición SX decodificada — compartido entre la ruta independiente y
/// los subopcodes `OP_MULTIPACKET*` (eMule envía la respuesta SX en un paquete
/// separado, `ListenSocket.cpp:969`).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn respond_to_sx_request(
    req: &Ed2kRequestSources,
    is_sx2_req: bool,
    dl_hash_bytes: Option<[u8; 16]>,
    target_addr: SocketAddr,
    encoder: &Ed2kEncoder,
    downloads: &Arc<Mutex<DownloadService>>,
    stream: &mut ObfuscatedStream<TcpStream>,
    env: &SxEnv,
    caps: &PeerSxCaps,
    requester_mask: Option<&[u8]>,
) {
    // eMule `ListenSocket.cpp:986` : `byRequestedVersion > 0 ||
    // client->GetSourceExchange1Version() > 1`.
    let by_requested_version = if is_sx2_req { req.version } else { 0 };
    if by_requested_version == 0 && caps.sx1_version <= 1 {
        return;
    }
    // Sesión monoarchivo: responder solo para el archivo de la sesión.
    if dl_hash_bytes.is_some() && dl_hash_bytes != Some(req.file_hash) {
        return;
    }

    let file_hash_str = hex::encode(req.file_hash);
    // eMule: sharedfiles primero, luego downloadqueue (`ListenSocket.cpp:1192`).
    let (is_part_file, file_part_count, known_sources) = {
        let dl_svc = downloads.lock().await;
        let shared = dl_svc.get_shared_file(&file_hash_str).unwrap_or(None);
        let dl_snap = dl_svc.find_by_ed2k_hash(&file_hash_str);
        let known = dl_snap.is_some() || shared.as_ref().is_some_and(|s| s.enabled);
        if !known {
            // Archivo desconocido: eMule llama a `CheckFailedFileIdReqs` —
            // aquí, silencio.
            return;
        }
        let is_part = dl_snap
            .as_ref()
            .map(|d| !matches!(d.status.to_lowercase().as_str(), "completed" | "complete"))
            .unwrap_or(false);
        let part_size = mule_protocol::ed2k::ED2K_PART_SIZE as u64;
        let parts = dl_snap
            .as_ref()
            .map(|d| d.total_bytes.div_ceil(part_size) as usize)
            .or_else(|| {
                shared
                    .as_ref()
                    .map(|s| s.file_size.div_ceil(part_size) as usize)
            })
            .unwrap_or(0);
        (
            is_part,
            parts,
            dl_svc.list_peer_sources(&file_hash_str).unwrap_or_default(),
        )
    };

    // Limitación de frecuencia de eMule (`ListenSocket.cpp:1204-1215`): nunca respondido → OK;
    // part-file raro (≤ RARE_FILE fuentes) → > SOURCECLIENTREASKS;
    // en otro caso → > SOURCECLIENTREASKS × MINCOMMONPENALTY.
    let peer_ip = target_addr.ip();
    let last_answer = env
        .answer_log
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&peer_ip)
        .copied();
    let never_asked = last_answer.is_none();
    let elapsed = last_answer
        .map(|t| t.elapsed().saturating_add(CONNECTION_LATENCY))
        .unwrap_or(Duration::MAX);
    let allowed = never_asked
        || elapsed > SOURCECLIENTREASKS * MINCOMMONPENALTY
        || (is_part_file && known_sources.len() <= RARE_FILE && elapsed > SOURCECLIENTREASKS);
    if !allowed {
        tracing::debug!(
            "[SOURCE_EXCHANGE] SX request from {peer} for {file_hash_str} rate-limited (last answer {:?} ago)",
            last_answer.map(|t| t.elapsed())
        , peer = log_peer_addr(target_addr));
        return;
    }
    // `client->SetLastSrcReqTime()`
    env.answer_log
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(peer_ip, Instant::now());

    let resp_sources =
        select_sx_response_sources(&known_sources, file_part_count, requester_mask, env);
    if resp_sources.is_empty() {
        // `if (!nCount) return 0;` de eMule — sin respuesta si no hay fuente válida.
        tracing::debug!(
            "[SOURCE_EXCHANGE] SX request from {peer} for {file_hash_str}: no valid source to offer"
        , peer = log_peer_addr(target_addr));
        return;
    }

    // Négociation eMule (`PartFile.cpp:3629-3646`) :
    // `SupportsSourceExchange2() && byRequestedVersion > 0` → SX2 à
    // `min(requested, 4)` ; sinon SX1 à `GetSourceExchange1Version()`.
    let (version, is_sx2_packet) = if caps.sx2 && by_requested_version > 0 {
        (by_requested_version.min(SOURCE_EXCHANGE2_VERSION), true)
    } else {
        (caps.sx1_version, false)
    };
    if version == 0 {
        return;
    }

    let ans = Ed2kAnswerSources::new_v(version, req.file_hash, resp_sources);
    match encoder.encode_answer_sources(&ans, is_sx2_packet, false) {
        Ok(ans_pkt) => {
            // `CreateSrcInfoPacket` de eMule (KnownFile.cpp:1149 /
            // PartFile.cpp:3708): un paquete > 354 bytes se empaqueta con zlib
            // en OP_PACKEDPROT (0xD4) cuando la compresión reduce
            // realmente el tamaño.
            let wire_pkt = mule_protocol::ed2k::packed::maybe_pack(
                &ans_pkt,
                mule_protocol::ed2k::packed::PACK_SRCINFO_MIN_SIZE,
            );
            let mut resp_wire = Vec::with_capacity(6 + wire_pkt.payload.len());
            resp_wire.push(wire_pkt.protocol);
            resp_wire.extend_from_slice(&(wire_pkt.frame_len() as u32).to_le_bytes());
            resp_wire.push(wire_pkt.opcode);
            resp_wire.extend_from_slice(&wire_pkt.payload);
            use tokio::io::AsyncWriteExt;
            if let Err(e) = stream.write_all(&resp_wire).await {
                tracing::warn!(
                    "Failed to send SX answer to {peer}: {e}",
                    peer = log_peer_addr(target_addr)
                );
            } else {
                tracing::info!(
                    "[SOURCE_EXCHANGE] Sent SX response ({} version {}, {} sources) to peer {} for file {}",
                    if is_sx2_packet { "SX2" } else { "SX1" },
                    version,
                    ans.sources.len(),
                    log_peer_addr(target_addr),
                    file_hash_str
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                "Failed to encode SX answer for {peer}: {e}",
                peer = log_peer_addr(target_addr)
            );
        }
    }
}

/// Procesa un `OP_MULTIPACKET*` recibido durante una sesión de descarga saliente —
/// el par puede encapsular en él su petición SX (`ListenSocket.cpp:970-1011` de eMule,
/// subopcodes sin hash: el del envoltorio). Solo se despachan los subopcodes SX;
/// `dl_hash_bytes` acota la respuesta al archivo de la sesión (`m_reqfile` de eMule).
/// Devuelve `true` si el paquete era un multipaquete (consumido, incluso en caso de error
/// de decodificación).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_sx_multipacket(
    packet: &mule_protocol::framing::RawPacket,
    dl_hash_bytes: [u8; 16],
    requester_mask: Option<&[u8]>,
    target_addr: SocketAddr,
    encoder: &Ed2kEncoder,
    downloads: &Arc<Mutex<DownloadService>>,
    stream: &mut ObfuscatedStream<TcpStream>,
    env: &SxEnv,
    caps: &PeerSxCaps,
    ext_req_version: u8,
) -> bool {
    use mule_protocol::ed2k::{
        Ed2kMultiPacketSubRequest, OP_MULTIPACKET, OP_MULTIPACKET_EXT, OP_MULTIPACKET_EXT2,
    };
    if packet.protocol != PR_EMULE
        || !matches!(
            packet.opcode,
            OP_MULTIPACKET | OP_MULTIPACKET_EXT | OP_MULTIPACKET_EXT2
        )
    {
        return false;
    }
    let mp = match mule_protocol::ed2k::decoder::decode_multipacket_request(
        packet.protocol,
        packet.opcode,
        &packet.payload,
        0,
        &mule_protocol::ed2k::Ed2kLimits::default(),
        ext_req_version,
    ) {
        Ok(mp) => mp,
        Err(e) => {
            tracing::debug!(
                "Peer {peer}: malformed multipacket ignored: {e}",
                peer = log_peer_addr(target_addr)
            );
            return true;
        }
    };
    let file_hash = mp.file.file_hash();
    if file_hash == [0u8; 16] {
        return true;
    }
    // La ext-info `OP_REQFILENAME` (estado de partes del solicitante) sirve como
    // `reqstatus` cuando el subpaquete SX le sigue en el envoltorio.
    let mut ext_mask: Option<Vec<u8>> = None;
    for sub in &mp.sub_requests {
        match sub {
            Ed2kMultiPacketSubRequest::FileName {
                ext_info: Some(ext),
            } => {
                ext_mask = Some(ext.part_status.to_vec());
            }
            Ed2kMultiPacketSubRequest::RequestSources => {
                let req = Ed2kRequestSources::new_v1(file_hash);
                let mask = ext_mask.as_deref().or(requester_mask);
                respond_to_sx_request(
                    &req,
                    false,
                    Some(dl_hash_bytes),
                    target_addr,
                    encoder,
                    downloads,
                    stream,
                    env,
                    caps,
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
                let mask = ext_mask.as_deref().or(requester_mask);
                respond_to_sx_request(
                    &req,
                    true,
                    Some(dl_hash_bytes),
                    target_addr,
                    encoder,
                    downloads,
                    stream,
                    env,
                    caps,
                    mask,
                )
                .await;
            }
            _ => {}
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use mule_core::model::{PeerSource, PeerSourceOrigin, PeerSourceStatus, SharedFile};
    use mule_network_policy::{NetworkProfile, ResourceBudgets};
    use mule_protocol::ed2k::tags::{
        Ed2kTag, TagName, TagValue, TAG_SPECIAL_EMULE_EXT_SOURCEEXCHANGE,
    };
    use mule_protocol::ed2k::Ed2kEmuleInfo;
    use mule_protocol::framing::{PacketCodec, RawPacket};
    use tokio::io::AsyncReadExt;

    fn test_env() -> SxEnv {
        SxEnv::new(
            NetworkPolicyManager::new(
                NetworkProfile::PublicEd2kKadPreview,
                ResourceBudgets::default(),
            ),
            [0xAA; 16],
        )
    }

    fn ip4(octets: [u8; 4]) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from(octets))
    }

    fn peer_source(
        ip_octets: [u8; 4],
        port: u16,
        status: PeerSourceStatus,
        availability: Option<Vec<u8>>,
    ) -> PeerSource {
        let ip = u32::from_le_bytes(ip_octets);
        PeerSource {
            file_hash: "00".repeat(16),
            address: format!(
                "{}.{}.{}.{}:{port}",
                ip_octets[0], ip_octets[1], ip_octets[2], ip_octets[3]
            ),
            ip,
            port,
            origin: PeerSourceOrigin::SourceExchange,
            discovered_at_unix_ms: 0,
            last_seen_unix_ms: 0,
            status,
            fail_count: 0,
            last_error: None,
            cooldown_until_unix_ms: None,
            user_hash: Some(hex::encode([0x33; 16])),
            crypt_options: 1,
            buddy_id: None,
            software: None,
            availability,
            server_ip: 0,
            server_port: 0,
            udp_port: 0,
        }
    }

    // ---- `IsSourceRequestAllowed` (DownloadClient.cpp:212-255) ----

    #[test]
    fn source_request_allowed_requires_sx_capability() {
        let env = test_env();
        let ip = ip4([85, 25, 13, 5]);
        let fh = [0x01; 16];
        // Par sin ninguna capacidad SX → nunca se solicita.
        assert!(!is_source_request_allowed(
            &env,
            ip,
            &fh,
            0,
            0,
            360,
            &PeerSxCaps::default()
        ));
    }

    #[test]
    fn source_request_allowed_rare_file_first_ask_then_client_cooldown() {
        let env = test_env();
        let ip = ip4([85, 25, 13, 5]);
        let fh = [0x01; 16];
        let caps = PeerSxCaps {
            sx1_version: 0,
            sx2: true,
            complete_source: false,
            multipacket: MultiPacketLevel::None,
        file_identifiers: false,
        };
        // Archivo raro, nunca consultado → permitido.
        assert!(is_source_request_allowed(&env, ip, &fh, 3, 1, 360, &caps));
        // Petición emitida → enfriamiento SOURCECLIENTREASKS en el lado cliente.
        mark_sx_request_sent(&env, ip, fh);
        assert!(!is_source_request_allowed(&env, ip, &fh, 3, 1, 360, &caps));
        // Enfriamiento expirado (> SOURCECLIENTREASKS) → permitido de nuevo.
        env.request_log.lock().unwrap().insert(
            ip,
            Instant::now()
                .checked_sub(Duration::from_secs(41 * 60))
                .unwrap_or_else(Instant::now),
        );
        assert!(is_source_request_allowed(&env, ip, &fh, 3, 1, 360, &caps));
    }

    #[test]
    fn source_request_allowed_soft_cap_and_complete_source() {
        let env = test_env();
        let ip = ip4([85, 25, 13, 5]);
        let fh = [0x01; 16];
        let caps = PeerSxCaps {
            sx1_version: 0,
            sx2: true,
            complete_source: false,
            multipacket: MultiPacketLevel::None,
        file_identifiers: false,
        };
        // `GetMaxSourcePerFileSoft` alcanzado → sin petición.
        assert!(!is_source_request_allowed(
            &env, ip, &fh, 360, 300, 360, &caps
        ));
        // Fuente completa: las cláusulas «raras» exigen
        // `!m_bCompleteSource` — queda la cláusula no rara (160 min),
        // que falla sin historial… salvo `never_asked` que la hace
        // verdadera: eMule pide una primera vez incluso a una fuente
        // completa en un archivo no raro. Comprobamos la condición de raro:
        // complete_source → cláusula rara imposible.
        let complete = PeerSxCaps {
            complete_source: true,
            ..caps
        };
        // Cláusula rara bloqueada; cláusula común `never_asked` aún
        // posible — eMule la autoriza realmente (cliente nunca solicitado).
        // Nos aseguramos sobre todo de que las cláusulas raras no pasen:
        // archivo raro (3 fuentes) + complete → solo la cláusula común
        // puede responder → es verdadera (never_asked). Documentado: eMule
        // pide efectivamente a fuentes completas mediante la cláusula 3.
        assert!(is_source_request_allowed(
            &env, ip, &fh, 3, 1, 360, &complete
        ));
    }

    // ---- `select_sx_response_sources` (CreateSrcInfoPacket) ----

    #[test]
    fn select_sources_filters_like_emule() {
        let env = test_env();
        let good = peer_source(
            [85, 25, 13, 7],
            49662,
            PeerSourceStatus::Connected,
            Some(vec![0x01]),
        );
        let allowed = peer_source(
            [85, 25, 13, 12],
            49662,
            PeerSourceStatus::Allowed,
            Some(vec![0x01]),
        );
        let lowid = {
            let mut s = peer_source(
                [1, 0, 0, 0],
                0,
                PeerSourceStatus::Connected,
                Some(vec![0x01]),
            );
            s.address = "lowid:1".into();
            s.ip = 1;
            s
        };
        let candidate = peer_source(
            [85, 25, 13, 8],
            49662,
            PeerSourceStatus::Candidate,
            Some(vec![0x01]),
        );
        let nomask = peer_source([85, 25, 13, 9], 49662, PeerSourceStatus::Connected, None);
        let lan = peer_source(
            [10, 0, 0, 5],
            49662,
            PeerSourceStatus::Connected,
            Some(vec![0x01]),
        );
        let wrong_mask = peer_source(
            [85, 25, 13, 10],
            49662,
            PeerSourceStatus::Connected,
            Some(vec![0xFF, 0xFF]),
        );
        let no_part = peer_source(
            [85, 25, 13, 11],
            49662,
            PeerSourceStatus::Connected,
            Some(vec![0x00]),
        );
        let out = select_sx_response_sources(
            &[
                good, allowed, lowid, candidate, nomask, lan, wrong_mask, no_part,
            ],
            1,
            None,
            &env,
        );
        let ips: Vec<[u8; 4]> = out.iter().map(|s| s.ip_bytes).collect();
        assert_eq!(ips, vec![[85, 25, 13, 7], [85, 25, 13, 12]]);
    }

    #[test]
    fn select_sources_requires_part_useful_to_requester() {
        let env = test_env();
        let good = peer_source(
            [85, 25, 13, 7],
            49662,
            PeerSourceStatus::Connected,
            Some(vec![0x01]),
        );
        // El solicitante ya posee la única parte ofrecida → nada que servir.
        assert!(
            select_sx_response_sources(std::slice::from_ref(&good), 1, Some(&[0x01]), &env)
                .is_empty()
        );
        // No la tiene → la fuente se sirve.
        assert_eq!(
            select_sx_response_sources(&[good], 1, Some(&[0x00]), &env).len(),
            1
        );
    }

    #[test]
    fn select_sources_caps_at_emule_500_limit() {
        let env = test_env();
        let mut sources = Vec::new();
        for i in 0..600u32 {
            let oct = [85, 25, (i / 250) as u8 + 10, (i % 250) as u8 + 1];
            sources.push(peer_source(
                oct,
                49662,
                PeerSourceStatus::Connected,
                Some(vec![0x01]),
            ));
        }
        let out = select_sx_response_sources(&sources, 1, None, &env);
        // eMule `if (nCount > 500) break;` → 501 entrées max.
        assert_eq!(out.len(), SX_RESPONSE_MAX_SOURCES);
    }

    // ---- `emule_info_sx_version` ----

    #[test]
    fn emule_info_sx_version_reads_et_sourceexchange() {
        let info = Ed2kEmuleInfo {
            client_version: 0x3C,
            protocol_version: 1,
            tags: vec![
                Ed2kTag {
                    name: TagName::Id(TAG_SPECIAL_EMULE_EXT_SOURCEEXCHANGE),
                    value: TagValue::UInt8(3),
                },
                Ed2kTag {
                    name: TagName::Id(0x99),
                    value: TagValue::UInt32(1),
                },
            ],
            unknown_tags: Vec::new(),
        };
        assert_eq!(emule_info_sx_version(&info), 3);
        let bare = Ed2kEmuleInfo {
            client_version: 0x3C,
            protocol_version: 1,
            tags: Vec::new(),
            unknown_tags: Vec::new(),
        };
        assert_eq!(emule_info_sx_version(&bare), 0);
    }

    // ---- Ingestion `handle_sx_answer` (AddClientSources + CanAddSource) ----

    async fn memory_downloads() -> (tempfile::TempDir, Arc<Mutex<DownloadService>>) {
        let tmp = tempfile::tempdir().unwrap();
        let dl = DownloadService::in_memory(tmp.path().to_path_buf()).unwrap();
        (tmp, Arc::new(Mutex::new(dl)))
    }

    fn sx_src(ip_octets: [u8; 4], port: u16, user_hash: Option<[u8; 16]>) -> SxSource {
        SxSource {
            ip_bytes: ip_octets,
            ip: u32::from_le_bytes(ip_octets),
            port,
            server_ip: 0,
            server_port: 0,
            user_hash,
            crypt_options: 0,
        }
    }

    #[tokio::test]
    async fn sx_answer_persists_only_valid_sources() {
        let (_tmp, downloads) = memory_downloads().await;
        let env = test_env();
        let file_hash = [0x11; 16];
        let file_hash_str = hex::encode(file_hash);
        let local = SxLocalIds {
            client_id: u32::from_be_bytes([85, 25, 13, 50]),
            server_ip: 0x0102_0304,
            server_port: 49661,
            tcp_port: 49662,
            is_high_id: true,
            kad_open: false,
        };
        // HighID → `!IsFirewalled()`: las `lowid:` se aceptan en el
        // embudo de ingesta al igual que en el filtro SX.
        downloads.lock().await.set_lowid_source_acceptance(true);

        let sources = vec![
            // Valide : IP publique HighID.
            sx_src([85, 25, 13, 7], 49662, Some([0x01; 16])),
            // Notre user hash → drop.
            sx_src([85, 25, 13, 8], 49662, Some(env.own_user_hash)),
            // Port 0 → drop.
            sx_src([85, 25, 13, 9], 0, Some([0x02; 16])),
            // LAN → drop (policy réseau).
            sx_src([10, 0, 0, 7], 49662, Some([0x03; 16])),
            // LowID → conservado como `lowid:` cuando somos HighID.
            SxSource {
                server_ip: 0x0506_0708,
                server_port: 49661,
                ..sx_src([88, 0, 0, 0], 0, Some([0x04; 16]))
            },
            // LowID == nuestro client_id en nuestro servidor → descartar.
            SxSource {
                server_ip: 0x0102_0304,
                server_port: 49661,
                ..sx_src(local.client_id.to_le_bytes(), 0, Some([0x05; 16]))
            },
            // HighID == notre client_id + notre port → drop.
            sx_src([85, 25, 13, 50], 49662, Some([0x06; 16])),
        ];
        let ans = Ed2kAnswerSources::new_v(4, file_hash, sources);
        let encoder = Ed2kEncoder::default();
        let pkt = encoder.encode_answer_sources(&ans, true, false).unwrap();
        let target: SocketAddr = "85.25.13.1:5000".parse().unwrap();
        handle_sx_answer(
            &pkt,
            file_hash,
            &file_hash_str,
            target,
            &Ed2kDecoder::default(),
            &downloads,
            &env,
            &local,
        )
        .await;

        let kept = downloads
            .lock()
            .await
            .list_peer_sources(&file_hash_str)
            .unwrap();
        let mut addrs: Vec<String> = kept.iter().map(|s| s.address.clone()).collect();
        addrs.sort();
        assert_eq!(
            addrs,
            vec!["85.25.13.7:49662".to_string(), "lowid:88".to_string()]
        );
        assert!(kept.iter().all(|s| s.status == PeerSourceStatus::Candidate));
    }

    #[tokio::test]
    async fn sx_answer_drops_lowid_when_firewalled() {
        let (_tmp, downloads) = memory_downloads().await;
        let env = test_env();
        let file_hash = [0x22; 16];
        let file_hash_str = hex::encode(file_hash);
        // Nosotros mismos somos LowID sin Kad abierto → `IsFirewalled()`, imposible
        // recibir la devolución de llamada.
        let local = SxLocalIds {
            is_high_id: false,
            kad_open: false,
            ..SxLocalIds::default()
        };
        let sources = vec![
            SxSource {
                server_ip: 1,
                server_port: 49661,
                ..sx_src([77, 0, 0, 0], 0, None)
            },
            sx_src([85, 25, 13, 7], 49662, None),
        ];
        let ans = Ed2kAnswerSources::new_v(4, file_hash, sources);
        let pkt = Ed2kEncoder::default()
            .encode_answer_sources(&ans, true, false)
            .unwrap();
        let target: SocketAddr = "85.25.13.1:5000".parse().unwrap();
        handle_sx_answer(
            &pkt,
            file_hash,
            &file_hash_str,
            target,
            &Ed2kDecoder::default(),
            &downloads,
            &env,
            &local,
        )
        .await;
        let kept = downloads
            .lock()
            .await
            .list_peer_sources(&file_hash_str)
            .unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].address, "85.25.13.7:49662");
    }

    #[tokio::test]
    async fn sx_answer_ignores_other_file_hash() {
        let (_tmp, downloads) = memory_downloads().await;
        let env = test_env();
        let file_hash = [0x33; 16];
        let other = [0x44; 16];
        let file_hash_str = hex::encode(file_hash);
        let ans = Ed2kAnswerSources::new_v(4, other, vec![sx_src([85, 25, 13, 7], 49662, None)]);
        let pkt = Ed2kEncoder::default()
            .encode_answer_sources(&ans, true, false)
            .unwrap();
        handle_sx_answer(
            &pkt,
            file_hash,
            &file_hash_str,
            "85.25.13.1:5000".parse().unwrap(),
            &Ed2kDecoder::default(),
            &downloads,
            &env,
            &SxLocalIds::default(),
        )
        .await;
        assert!(downloads
            .lock()
            .await
            .list_peer_sources(&file_hash_str)
            .unwrap()
            .is_empty());
        assert!(downloads
            .lock()
            .await
            .list_peer_sources(&hex::encode(other))
            .unwrap()
            .is_empty());
    }

    // ---- Service `respond_to_sx_request` via loopback TCP ----

    async fn socket_pair() -> (ObfuscatedStream<TcpStream>, TcpStream) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        (ObfuscatedStream::plaintext(server), client)
    }

    async fn read_wire(client: &mut TcpStream) -> RawPacket {
        let codec = PacketCodec::new();
        let mut buf = BytesMut::new();
        let mut tmp = [0u8; 8192];
        loop {
            let n = tokio::time::timeout(Duration::from_secs(3), client.read(&mut tmp))
                .await
                .expect("timeout waiting for SX answer")
                .expect("read error");
            assert!(n > 0, "connection closed before SX answer");
            buf.extend_from_slice(&tmp[..n]);
            if let Ok(Some(pkt)) = codec.decode(&mut buf) {
                return pkt;
            }
        }
    }

    async fn downloads_with_shared(
        file_hash: [u8; 16],
    ) -> (tempfile::TempDir, Arc<Mutex<DownloadService>>, String) {
        let (tmp, downloads) = memory_downloads().await;
        let hex_hash = hex::encode(file_hash);
        // El archivo debe existir: `get_shared_file` ahora ignora
        // las entradas cuya carga útil haya desaparecido del disco.
        let file_path = tmp.path().join("file.bin");
        std::fs::write(&file_path, b"shared-payload").unwrap();
        {
            let mut dl = downloads.lock().await;
            dl.add_shared_file(&SharedFile::new(
                hex_hash.clone(),
                "file.bin",
                9_728_000,
                file_path.to_string_lossy().to_string(),
            ))
            .unwrap();
            let useful = peer_source(
                [85, 25, 13, 7],
                49662,
                PeerSourceStatus::Connected,
                Some(vec![0x01]),
            );
            let useless = peer_source(
                [85, 25, 13, 8],
                49662,
                PeerSourceStatus::Connected,
                Some(vec![0x00]),
            );
            dl.upsert_peer_sources(&hex_hash, &[useful, useless])
                .unwrap();
        }
        (tmp, downloads, hex_hash)
    }

    #[tokio::test]
    async fn respond_sx2_serves_useful_sources_at_version4() {
        let file_hash = [0x42; 16];
        let (_tmp, downloads, _hex) = downloads_with_shared(file_hash).await;
        let env = test_env();
        let caps = PeerSxCaps {
            sx1_version: 3,
            sx2: true,
            complete_source: false,
            multipacket: MultiPacketLevel::None,
        file_identifiers: false,
        };
        let (mut our_end, mut peer_end) = socket_pair().await;
        let peer_addr: SocketAddr = "85.25.13.99:49662".parse().unwrap();
        let req = Ed2kRequestSources::new_v2(file_hash);
        respond_to_sx_request(
            &req,
            true,
            None,
            peer_addr,
            &Ed2kEncoder::default(),
            &downloads,
            &mut our_end,
            &env,
            &caps,
            None,
        )
        .await;
        let pkt = read_wire(&mut peer_end).await;
        assert_eq!(pkt.opcode, OP_ANSWERSOURCES2);
        let ans = Ed2kDecoder::default()
            .decode_answer_sources(pkt.protocol, pkt.opcode, &pkt.payload)
            .unwrap();
        assert_eq!(ans.version, SOURCE_EXCHANGE2_VERSION);
        assert_eq!(ans.file_hash, file_hash);
        // Solo la fuente con una parte útil es servida.
        assert_eq!(ans.sources.len(), 1);
        assert_eq!(ans.sources[0].ip_bytes, [85, 25, 13, 7]);
        assert_eq!(ans.sources[0].user_hash, Some([0x33; 16]));
    }

    #[tokio::test]
    async fn respond_sx1_request_falls_back_to_sx1_packet() {
        let file_hash = [0x43; 16];
        let (_tmp, downloads, _hex) = downloads_with_shared(file_hash).await;
        let env = test_env();
        let caps = PeerSxCaps {
            sx1_version: 3,
            sx2: true,
            complete_source: false,
            multipacket: MultiPacketLevel::None,
        file_identifiers: false,
        };
        let (mut our_end, mut peer_end) = socket_pair().await;
        let req = Ed2kRequestSources::new_v1(file_hash);
        respond_to_sx_request(
            &req,
            false,
            None,
            "85.25.13.99:49662".parse().unwrap(),
            &Ed2kEncoder::default(),
            &downloads,
            &mut our_end,
            &env,
            &caps,
            None,
        )
        .await;
        let pkt = read_wire(&mut peer_end).await;
        // eMule `byRequestedVersion == 0` → OP_ANSWERSOURCES SX1.
        assert_eq!(pkt.opcode, OP_ANSWERSOURCES);
        let ans = Ed2kDecoder::default()
            .decode_answer_sources(pkt.protocol, pkt.opcode, &pkt.payload)
            .unwrap();
        assert_eq!(ans.version, 3);
        assert_eq!(ans.sources.len(), 1);
    }

    #[tokio::test]
    async fn respond_rate_limits_second_request() {
        let file_hash = [0x44; 16];
        let (_tmp, downloads, _hex) = downloads_with_shared(file_hash).await;
        let env = test_env();
        let caps = PeerSxCaps {
            sx1_version: 3,
            sx2: true,
            complete_source: false,
            multipacket: MultiPacketLevel::None,
        file_identifiers: false,
        };
        let (mut our_end, mut peer_end) = socket_pair().await;
        let peer_addr: SocketAddr = "85.25.13.99:49662".parse().unwrap();
        let req = Ed2kRequestSources::new_v2(file_hash);
        let enc = Ed2kEncoder::default();
        for _ in 0..2 {
            respond_to_sx_request(
                &req,
                true,
                None,
                peer_addr,
                &enc,
                &downloads,
                &mut our_end,
                &env,
                &caps,
                None,
            )
            .await;
        }
        let _ = read_wire(&mut peer_end).await;
        // La 2ª petición cae bajo la limitación SOURCECLIENTREASKS.
        let mut tmp = [0u8; 512];
        let res = tokio::time::timeout(Duration::from_millis(300), peer_end.read(&mut tmp)).await;
        assert!(
            res.is_err(),
            "second SX request should have been rate-limited"
        );
    }

    #[tokio::test]
    async fn respond_silent_for_unknown_file() {
        let file_hash = [0x45; 16];
        let (_tmp, downloads, _hex) = downloads_with_shared(file_hash).await;
        let env = test_env();
        let caps = PeerSxCaps {
            sx1_version: 3,
            sx2: true,
            complete_source: false,
            multipacket: MultiPacketLevel::None,
        file_identifiers: false,
        };
        let (mut our_end, mut peer_end) = socket_pair().await;
        // Hash desconocido: eMule no responde (`ListenSocket.cpp:1192-1199`).
        let req = Ed2kRequestSources::new_v2([0x99; 16]);
        respond_to_sx_request(
            &req,
            true,
            None,
            "85.25.13.99:49662".parse().unwrap(),
            &Ed2kEncoder::default(),
            &downloads,
            &mut our_end,
            &env,
            &caps,
            None,
        )
        .await;
        let mut tmp = [0u8; 512];
        let res = tokio::time::timeout(Duration::from_millis(300), peer_end.read(&mut tmp)).await;
        assert!(res.is_err(), "unknown file must not produce an SX answer");
        // Y `answer_log` no debe haberse marcado (limitación no consumida).
        assert!(env
            .answer_log
            .lock()
            .unwrap()
            .get(&ip4([85, 25, 13, 99]))
            .is_none());
    }

    #[tokio::test]
    async fn respond_silent_when_no_source_to_offer() {
        let file_hash = [0x46; 16];
        let (_tmp, downloads, hex_hash) = downloads_with_shared(file_hash).await;
        // Ninguna fuente válida: retirar las insertadas por el auxiliar.
        downloads
            .lock()
            .await
            .clear_peer_sources(&hex_hash)
            .unwrap();
        let env = test_env();
        let caps = PeerSxCaps {
            sx1_version: 3,
            sx2: true,
            complete_source: false,
            multipacket: MultiPacketLevel::None,
        file_identifiers: false,
        };
        let (mut our_end, mut peer_end) = socket_pair().await;
        let req = Ed2kRequestSources::new_v2(file_hash);
        respond_to_sx_request(
            &req,
            true,
            None,
            "85.25.13.99:49662".parse().unwrap(),
            &Ed2kEncoder::default(),
            &downloads,
            &mut our_end,
            &env,
            &caps,
            None,
        )
        .await;
        let mut tmp = [0u8; 512];
        let res = tokio::time::timeout(Duration::from_millis(300), peer_end.read(&mut tmp)).await;
        assert!(
            res.is_err(),
            "empty answer must not be sent (eMule !nCount)"
        );
    }
}

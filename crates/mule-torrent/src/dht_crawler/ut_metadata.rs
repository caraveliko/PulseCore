// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Cliente acotado de BEP 10 (Extension Protocol) + BEP 9 (`ut_metadata`) para descargar únicamente
//! el diccionario `info` de un torrent descubierto en la DHT, verificar `SHA1(info) == info_hash`
//! y filtrar estrictamente según la política inmutable «Solo Vídeo».

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use mule_network_policy::content_policy::is_allowed_video_filename;
use mule_network_policy::validator::validate_connect_address;
use mule_network_policy::{NetworkPolicyManager, NetworkState};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::bencode::{decode, decode_prefix, BValue};
use super::content_filter::is_blocked_abusive_or_adult;
use super::wire::decode_compact_peers_v4_slice;

/// Protocolo estándar de cabecera BitTorrent (longitud 19 + cadena ASCII).
const BT_PROTOCOL_HEADER: &[u8; 20] = b"\x13BitTorrent protocol";
/// Bit de extensión BEP 10 en el sexto byte de los 8 bytes reservados del handshake BitTorrent.
const BEP10_RESERVED_BYTE_IDX: usize = 5;
const BEP10_RESERVED_FLAG: u8 = 0x10;
/// Identificador de mensaje de extensión BEP 10.
const MSG_ID_EXTENDED: u8 = 20;
/// Subidentificador BEP 10 de handshake extendido.
const EXT_HANDSHAKE_ID: u8 = 0;
/// Subidentificador local anunciado por Pulse Core para `ut_metadata` (BEP 9).
const LOCAL_UT_METADATA_ID: u8 = 1;
/// Subidentificador local anunciado por Pulse Core para `ut_pex` (BEP 11).
const LOCAL_UT_PEX_ID: u8 = 2;
/// Número máximo de pares `ut_pex` recogidos en una sola sesión BEP 10.
const MAX_PEX_PEERS_PER_FETCH: usize = 64;
/// Longitud exacta en bytes de un hash eD2K (MD4 de 128 bits) en metadatos `.torrent`.
const ED2K_HASH_BYTES_LEN: usize = 16;
/// Tamaño fijo de cada pieza de metadatos en BEP 9 (16 KiB).
pub const UT_METADATA_PIECE_SIZE: usize = 16_384;
/// Límite máximo de tamaño del diccionario `info` aceptado (512 KiB).
pub const MAX_METADATA_SIZE: usize = 512 * 1024;
/// Tamaño máximo de una trama TCP individual del protocolo de pares (64 KiB).
const MAX_PEER_MESSAGE_LEN: usize = 64 * 1024;
/// Número máximo de mensajes leídos antes de abortar una sesión `ut_metadata` lenta o hostil.
const MAX_MESSAGES_PER_FETCH: usize = 96;
/// Tiempo máximo de conexión TCP con el par remoto.
pub const UT_METADATA_CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
/// Tiempo máximo de lectura/escritura por operación individual.
pub const UT_METADATA_IO_TIMEOUT: Duration = Duration::from_secs(4);
/// Tiempo máximo total permitido para completar la descarga de metadatos de un `info_hash`.
pub const UT_METADATA_TOTAL_TIMEOUT: Duration = Duration::from_secs(10);

/// Archivo individual de vídeo extraído del diccionario `info` de un torrent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DhtVideoFile {
    pub file_index: usize,
    pub path: String,
    pub size_bytes: u64,
    pub offset_in_torrent: u64,
    /// Hash eD2K (32 hex) si el diccionario `.torrent` incluía la etiqueta nativa `"ed2k"` (16 bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ed2k_hash: Option<String>,
}

/// Metadatos verificados criptográficamente (`SHA1(info) == info_hash`) de un torrent de vídeo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredVideoTorrent {
    pub info_hash: String,
    pub name: String,
    pub total_bytes: u64,
    pub primary_video: DhtVideoFile,
    pub video_files: Vec<DhtVideoFile>,
    pub peers: Vec<SocketAddr>,
    /// Pares adicionales descubiertos en el mismo enjambre mediante `ut_pex` (BEP 11).
    #[serde(default)]
    pub pex_peers: Vec<SocketAddr>,
    /// Diccionario `info` bencoded original (verificado por SHA-1) para arranque instantáneo sin re-descargar metadatos.
    #[serde(skip)]
    pub raw_info_bytes: Vec<u8>,
    pub source: String,
}

/// Envuelve un diccionario `info` bencoded verificado en un archivo `.torrent` mínimo válido (`d4:info...e`).
pub fn build_minimal_torrent_bytes(raw_info: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw_info.len() + 8);
    out.extend_from_slice(b"d4:info");
    out.extend_from_slice(raw_info);
    out.push(b'e');
    out
}

fn extract_native_ed2k_hash(dict: &BValue) -> Option<String> {
    let raw = dict.dict_get(b"ed2k").and_then(BValue::as_bytes)?;
    if raw.len() == ED2K_HASH_BYTES_LEN {
        Some(hex::encode(raw))
    } else {
        None
    }
}

/// Analiza el diccionario `info` (ya verificado por SHA-1) y extrae únicamente los archivos de vídeo
/// conformes a `mule_network_policy::content_policy`. Devuelve `None` si el torrent no contiene vídeo.
pub fn parse_verified_video_info(
    info_hash: &[u8; 20],
    raw_info: &[u8],
    peers: Vec<SocketAddr>,
    source: &str,
) -> Option<DiscoveredVideoTorrent> {
    let digest: [u8; 20] = Sha1::digest(raw_info).into();
    if &digest != info_hash {
        return None;
    }

    let info = decode(raw_info).ok()?;
    let name = info
        .dict_get(b"name.utf-8")
        .or_else(|| info.dict_get(b"name"))
        .and_then(BValue::as_str_lossy)
        .filter(|s| !s.trim().is_empty())?;

    // Bloqueo duro e incondicional de nombres abusivos, ilegales o spam adulto antes de procesarlos.
    if is_blocked_abusive_or_adult(&name) {
        return None;
    }

    let mut total_bytes: u64 = 0;
    let mut video_files: Vec<DhtVideoFile> = Vec::new();

    if let Some(files_list) = info.dict_get(b"files").and_then(BValue::as_list) {
        let mut offset = 0u64;
        for (idx, file_entry) in files_list.iter().enumerate() {
            let len = file_entry
                .dict_get(b"length")
                .and_then(BValue::as_int)
                .and_then(|v| u64::try_from(v).ok())?;
            let path_parts = file_entry
                .dict_get(b"path.utf-8")
                .or_else(|| file_entry.dict_get(b"path"))
                .and_then(BValue::as_list)?;
            let rel_path = path_parts
                .iter()
                .filter_map(BValue::as_str_lossy)
                .collect::<Vec<_>>()
                .join("/");
            if is_blocked_abusive_or_adult(&rel_path) {
                return None;
            }
            if !rel_path.is_empty() && is_allowed_video_filename(&rel_path) {
                video_files.push(DhtVideoFile {
                    file_index: idx,
                    path: rel_path,
                    size_bytes: len,
                    offset_in_torrent: offset,
                    ed2k_hash: extract_native_ed2k_hash(file_entry),
                });
            }
            offset = offset.saturating_add(len);
        }
        total_bytes = offset;
    } else if let Some(len) = info
        .dict_get(b"length")
        .and_then(BValue::as_int)
        .and_then(|v| u64::try_from(v).ok())
    {
        total_bytes = len;
        if is_allowed_video_filename(&name) {
            video_files.push(DhtVideoFile {
                file_index: 0,
                path: name.clone(),
                size_bytes: len,
                offset_in_torrent: 0,
                ed2k_hash: extract_native_ed2k_hash(&info),
            });
        }
    }

    let primary_video = video_files
        .iter()
        .max_by_key(|f| f.size_bytes)
        .cloned()?;

    Some(DiscoveredVideoTorrent {
        info_hash: hex::encode(info_hash),
        name,
        total_bytes,
        primary_video,
        video_files,
        peers,
        pex_peers: Vec::new(),
        raw_info_bytes: raw_info.to_vec(),
        source: source.to_string(),
    })
}

/// Descarga el diccionario `info` de `info_hash` desde `peer_addr` mediante BEP 10 + BEP 9 (`ut_metadata`)
/// y recoge opcionalmente pares del enjambre vía BEP 11 (`ut_pex`), aplicando `NetworkPolicyManager` y «Solo Vídeo».
pub async fn fetch_video_metadata_from_peer(
    policy: &Arc<NetworkPolicyManager>,
    peer_addr: SocketAddr,
    info_hash: [u8; 20],
    source: &str,
) -> Option<DiscoveredVideoTorrent> {
    if matches!(
        policy.state(),
        NetworkState::Disabled | NetworkState::ShuttingDown
    ) {
        return None;
    }
    if validate_connect_address(&peer_addr, policy.profile()).is_err() {
        return None;
    }
    if policy.ip_filter().check_ip(peer_addr.ip()).is_some() {
        return None;
    }

    let (raw_info, raw_pex_peers) = tokio::time::timeout(
        UT_METADATA_TOTAL_TIMEOUT,
        fetch_raw_info_and_pex(peer_addr, info_hash),
    )
    .await
    .ok()??;

    let mut discovered = parse_verified_video_info(&info_hash, &raw_info, vec![peer_addr], source)?;
    discovered.pex_peers = raw_pex_peers
        .into_iter()
        .filter(|addr| {
            validate_connect_address(addr, policy.profile()).is_ok()
                && policy.ip_filter().check_ip(addr.ip()).is_none()
        })
        .take(MAX_PEX_PEERS_PER_FETCH)
        .collect();
    Some(discovered)
}

async fn fetch_raw_info_and_pex(
    peer_addr: SocketAddr,
    info_hash: [u8; 20],
) -> Option<(Vec<u8>, Vec<SocketAddr>)> {
    let mut stream =
        tokio::time::timeout(UT_METADATA_CONNECT_TIMEOUT, TcpStream::connect(peer_addr))
            .await
            .ok()?
            .ok()?;

    // 1. Handshake BitTorrent con bit BEP 10 habilitado.
    let mut hs = [0u8; 68];
    hs[..20].copy_from_slice(BT_PROTOCOL_HEADER);
    hs[20 + BEP10_RESERVED_BYTE_IDX] = BEP10_RESERVED_FLAG;
    hs[28..48].copy_from_slice(&info_hash);
    let mut peer_id = [0u8; 20];
    peer_id[..8].copy_from_slice(b"-PC0490-");
    rand::rng().fill_bytes(&mut peer_id[8..]);
    hs[48..68].copy_from_slice(&peer_id);

    tokio::time::timeout(UT_METADATA_IO_TIMEOUT, stream.write_all(&hs))
        .await
        .ok()?
        .ok()?;

    let mut peer_hs = [0u8; 68];
    tokio::time::timeout(UT_METADATA_IO_TIMEOUT, stream.read_exact(&mut peer_hs))
        .await
        .ok()?
        .ok()?;

    if &peer_hs[..20] != BT_PROTOCOL_HEADER {
        return None;
    }
    if peer_hs[28..48] != info_hash {
        return None;
    }
    if (peer_hs[20 + BEP10_RESERVED_BYTE_IDX] & BEP10_RESERVED_FLAG) == 0 {
        return None;
    }

    // 2. Enviar handshake extendido BEP 10 declarando soporte para `ut_metadata` (BEP 9) y `ut_pex` (BEP 11).
    let mut m_dict = BTreeMap::new();
    m_dict.insert(
        b"ut_metadata".to_vec(),
        BValue::Int(i64::from(LOCAL_UT_METADATA_ID)),
    );
    m_dict.insert(b"ut_pex".to_vec(), BValue::Int(i64::from(LOCAL_UT_PEX_ID)));
    let mut ext_hs = BTreeMap::new();
    ext_hs.insert(b"m".to_vec(), BValue::Dict(m_dict));
    send_extended_message(&mut stream, EXT_HANDSHAKE_ID, &BValue::Dict(ext_hs).encode()).await?;

    let mut peer_ut_metadata_id: Option<u8> = None;
    let mut metadata_size: Option<usize> = None;
    let mut pieces_buf: Vec<Option<Vec<u8>>> = Vec::new();
    let mut total_pieces: usize = 0;
    let mut pex_peers: Vec<SocketAddr> = Vec::new();

    for _ in 0..MAX_MESSAGES_PER_FETCH {
        let frame = read_peer_message(&mut stream).await?;
        if frame.is_empty() || frame[0] != MSG_ID_EXTENDED || frame.len() < 2 {
            continue;
        }
        let ext_id = frame[1];
        let payload = &frame[2..];

        if ext_id == EXT_HANDSHAKE_ID {
            let dict = decode(payload).ok()?;
            let remote_id = dict
                .dict_get(b"m")
                .and_then(|m| m.dict_get(b"ut_metadata"))
                .and_then(BValue::as_int)
                .and_then(|v| u8::try_from(v).ok())
                .filter(|&v| v > 0)?;
            let meta_size = dict
                .dict_get(b"metadata_size")
                .and_then(BValue::as_int)
                .and_then(|v| usize::try_from(v).ok())
                .filter(|&sz| sz > 0 && sz <= MAX_METADATA_SIZE)?;

            peer_ut_metadata_id = Some(remote_id);
            metadata_size = Some(meta_size);
            total_pieces = meta_size.div_ceil(UT_METADATA_PIECE_SIZE);
            pieces_buf = vec![None; total_pieces];

            // Solicitar todas las piezas de metadatos de 16 KiB en ráfaga acotada.
            for piece_idx in 0..total_pieces {
                let mut req = BTreeMap::new();
                req.insert(b"msg_type".to_vec(), BValue::Int(0));
                req.insert(b"piece".to_vec(), BValue::Int(piece_idx as i64));
                send_extended_message(&mut stream, remote_id, &BValue::Dict(req).encode()).await?;
            }
        } else if ext_id == LOCAL_UT_METADATA_ID {
            let (Some(_remote_id), Some(expected_size)) = (peer_ut_metadata_id, metadata_size)
            else {
                continue;
            };
            let (header, consumed) = decode_prefix(payload).ok()?;
            let msg_type = header.dict_get(b"msg_type").and_then(BValue::as_int)?;
            if msg_type == 2 {
                // Rechazo explícito de metadatos por el par.
                return None;
            }
            if msg_type != 1 {
                continue;
            }
            let piece_idx = header
                .dict_get(b"piece")
                .and_then(BValue::as_int)
                .and_then(|v| usize::try_from(v).ok())?;
            if piece_idx >= total_pieces {
                return None;
            }
            let piece_data = &payload[consumed..];
            let expected_piece_len = if piece_idx + 1 == total_pieces {
                let rem = expected_size % UT_METADATA_PIECE_SIZE;
                if rem == 0 {
                    UT_METADATA_PIECE_SIZE
                } else {
                    rem
                }
            } else {
                UT_METADATA_PIECE_SIZE
            };
            if piece_data.len() != expected_piece_len {
                return None;
            }
            pieces_buf[piece_idx] = Some(piece_data.to_vec());

            if pieces_buf.iter().all(Option::is_some) {
                let mut full = Vec::with_capacity(expected_size);
                for slot in pieces_buf.into_iter().flatten() {
                    full.extend_from_slice(&slot);
                }
                return Some((full, pex_peers));
            }
        } else if ext_id == LOCAL_UT_PEX_ID {
            if let Ok(pex_dict) = decode(payload) {
                if let Some(added) = pex_dict.dict_get(b"added").and_then(BValue::as_bytes) {
                    for addr in decode_compact_peers_v4_slice(added) {
                        if !pex_peers.contains(&addr) && pex_peers.len() < MAX_PEX_PEERS_PER_FETCH {
                            pex_peers.push(addr);
                        }
                    }
                }
            }
        }
    }

    None
}

async fn send_extended_message(stream: &mut TcpStream, ext_id: u8, payload: &[u8]) -> Option<()> {
    let msg_len = (2usize.checked_add(payload.len())?) as u32;
    let mut frame = Vec::with_capacity(4 + msg_len as usize);
    frame.extend_from_slice(&msg_len.to_be_bytes());
    frame.push(MSG_ID_EXTENDED);
    frame.push(ext_id);
    frame.extend_from_slice(payload);
    tokio::time::timeout(UT_METADATA_IO_TIMEOUT, stream.write_all(&frame))
        .await
        .ok()?
        .ok()
}

async fn read_peer_message(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    tokio::time::timeout(UT_METADATA_IO_TIMEOUT, stream.read_exact(&mut len_buf))
        .await
        .ok()?
        .ok()?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 {
        return Some(Vec::new());
    }
    if len > MAX_PEER_MESSAGE_LEN {
        return None;
    }
    let mut body = vec![0u8; len];
    tokio::time::timeout(UT_METADATA_IO_TIMEOUT, stream.read_exact(&mut body))
        .await
        .ok()?
        .ok()?;
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mule_network_policy::{NetworkProfile, ResourceBudgets};
    use tokio::net::TcpListener;

    fn build_info_dict(name: &str, length: i64, extra_pad_pieces: usize) -> Vec<u8> {
        let mut dict = BTreeMap::new();
        dict.insert(b"length".to_vec(), BValue::Int(length));
        dict.insert(b"name".to_vec(), BValue::str_bytes(name));
        dict.insert(b"piece length".to_vec(), BValue::Int(262_144));
        dict.insert(
            b"pieces".to_vec(),
            BValue::Bytes(vec![0x7Au8; 20 * extra_pad_pieces.max(1)]),
        );
        BValue::Dict(dict).encode()
    }

    #[tokio::test]
    async fn fetches_multi_piece_ut_metadata_over_loopback_and_enforces_video_policy() {
        // 1. Generar diccionario `info` mayor de 16 KiB para ejercitar varias piezas BEP 9.
        let raw_info = build_info_dict("Blade.Runner.1982.1080p.mkv", 2_500_000_000, 900);
        assert!(raw_info.len() > UT_METADATA_PIECE_SIZE);
        let info_hash: [u8; 20] = Sha1::digest(&raw_info).into();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let served_info = raw_info.clone();

        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut hs = [0u8; 68];
            sock.read_exact(&mut hs).await.unwrap();
            let mut reply_hs = [0u8; 68];
            reply_hs[..20].copy_from_slice(BT_PROTOCOL_HEADER);
            reply_hs[20 + BEP10_RESERVED_BYTE_IDX] = BEP10_RESERVED_FLAG;
            reply_hs[28..48].copy_from_slice(&info_hash);
            reply_hs[48..68].copy_from_slice(b"-MOCK01-000000000000");
            sock.write_all(&reply_hs).await.unwrap();

            // Leer handshake extendido del cliente
            let _ = read_peer_message(&mut sock).await.unwrap();

            // Responder con handshake extendido anunciando ut_metadata=7 y metadata_size
            let mut m = BTreeMap::new();
            m.insert(b"ut_metadata".to_vec(), BValue::Int(7));
            let mut ext = BTreeMap::new();
            ext.insert(b"m".to_vec(), BValue::Dict(m));
            ext.insert(
                b"metadata_size".to_vec(),
                BValue::Int(served_info.len() as i64),
            );
            send_extended_message(&mut sock, 0, &BValue::Dict(ext).encode())
                .await
                .unwrap();

            let total_pieces = served_info.len().div_ceil(UT_METADATA_PIECE_SIZE);
            for _ in 0..total_pieces {
                let msg = read_peer_message(&mut sock).await.unwrap();
                let req = decode(&msg[2..]).unwrap();
                let piece_idx = req.dict_get(b"piece").unwrap().as_int().unwrap() as usize;
                let start = piece_idx * UT_METADATA_PIECE_SIZE;
                let end = (start + UT_METADATA_PIECE_SIZE).min(served_info.len());
                let mut hdr = BTreeMap::new();
                hdr.insert(b"msg_type".to_vec(), BValue::Int(1));
                hdr.insert(b"piece".to_vec(), BValue::Int(piece_idx as i64));
                hdr.insert(
                    b"total_size".to_vec(),
                    BValue::Int(served_info.len() as i64),
                );
                let mut payload = BValue::Dict(hdr).encode();
                payload.extend_from_slice(&served_info[start..end]);
                send_extended_message(&mut sock, LOCAL_UT_METADATA_ID, &payload)
                    .await
                    .unwrap();
            }
        });

        let policy = Arc::new(NetworkPolicyManager::new(
            NetworkProfile::LocalLab,
            ResourceBudgets::default(),
        ));
        let discovered = fetch_video_metadata_from_peer(&policy, addr, info_hash, "dht_bep51")
            .await
            .expect("debe recuperar y verificar los metadatos de vídeo por loopback");

        assert_eq!(discovered.info_hash, hex::encode(info_hash));
        assert_eq!(discovered.primary_video.path, "Blade.Runner.1982.1080p.mkv");
        assert_eq!(discovered.primary_video.size_bytes, 2_500_000_000);

        // Un torrent que no contenga vídeo debe ser rechazado por la política «Solo Vídeo».
        let non_video = build_info_dict("archive.zip", 1024, 1);
        let non_video_hash: [u8; 20] = Sha1::digest(&non_video).into();
        assert!(parse_verified_video_info(&non_video_hash, &non_video, vec![], "test").is_none());

        // Extracción automática de etiqueta nativa `"ed2k"` (16 bytes) en metadatos `.torrent`.
        let expected_ed2k = [0xABu8; 16];
        let mut ed2k_dict = BTreeMap::new();
        ed2k_dict.insert(b"length".to_vec(), BValue::Int(450_000_000));
        ed2k_dict.insert(b"name".to_vec(), BValue::str_bytes("Naruto.001.Castellano.1080p.mkv"));
        ed2k_dict.insert(b"piece length".to_vec(), BValue::Int(262_144));
        ed2k_dict.insert(b"pieces".to_vec(), BValue::Bytes(vec![0x11u8; 20]));
        ed2k_dict.insert(b"ed2k".to_vec(), BValue::Bytes(expected_ed2k.to_vec()));
        let raw_ed2k_info = BValue::Dict(ed2k_dict).encode();
        let raw_ed2k_ih: [u8; 20] = Sha1::digest(&raw_ed2k_info).into();
        let parsed_ed2k = parse_verified_video_info(&raw_ed2k_ih, &raw_ed2k_info, vec![], "test")
            .expect("debe aceptar vídeo con etiqueta nativa ed2k");
        assert_eq!(
            parsed_ed2k.primary_video.ed2k_hash.as_deref(),
            Some("abababababababababababababababab")
        );
    }
}

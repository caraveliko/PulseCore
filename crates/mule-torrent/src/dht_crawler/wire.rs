// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Codificación y decodificación de paquetes KRPC Mainline DHT (BEP 5 + BEP 51 `sample_infohashes`)
//! y derivación determinista del identificador de encuentro (*Rendezvous*) eD2K → BitTorrent DHT.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use sha1::{Digest, Sha1};

use super::bencode::{decode, BValue};

/// Prefijo de dominio para derivar el `rendezvous_id` de 20 bytes en Mainline DHT a partir de un hash eD2K.
pub const PULSE_RENDEZVOUS_DOMAIN_PREFIX: &[u8] = b"pulse-ed2k-v1:";
/// Tamaño en bytes de un identificador DHT (SHA-1 / 160 bits).
pub const DHT_ID_LEN: usize = 20;
/// Tamaño en bytes de una entrada compacta de nodo IPv4 en BEP 5 (`20B id + 4B ip + 2B port`).
const COMPACT_NODE_V4_LEN: usize = 26;
/// Tamaño en bytes de una entrada compacta de par IPv4 en BEP 5 (`4B ip + 2B port`).
const COMPACT_PEER_V4_LEN: usize = 6;
/// Límite máximo de muestras BEP 51 aceptadas de una sola respuesta.
pub const MAX_BEP51_SAMPLES_PER_PACKET: usize = 64;

/// Deriva de forma determinista el `info_hash` sintético de encuentro (20 bytes) en la DHT de BitTorrent
/// para un archivo eD2K identificado por su hash MD4 de 16 bytes.
pub fn derive_ed2k_rendezvous_id(ed2k_hash: &[u8; 16]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(PULSE_RENDEZVOUS_DOMAIN_PREFIX);
    hasher.update(ed2k_hash);
    hasher.finalize().into()
}

/// Petición o respuesta KRPC decodificada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KrpcMessage {
    Query {
        tx_id: Vec<u8>,
        query_name: String,
        sender_id: [u8; 20],
        info_hash: Option<[u8; 20]>,
        target: Option<[u8; 20]>,
        port: Option<u16>,
        implied_port: bool,
    },
    Response {
        tx_id: Vec<u8>,
        sender_id: Option<[u8; 20]>,
        nodes: Vec<([u8; 20], SocketAddr)>,
        peers: Vec<SocketAddr>,
        samples: Vec<[u8; 20]>,
        token: Option<Vec<u8>>,
    },
}

/// Codifica una petición KRPC `ping` (BEP 5).
pub fn encode_ping(tx_id: &[u8], node_id: &[u8; 20]) -> Vec<u8> {
    let mut args = BTreeMap::new();
    args.insert(b"id".to_vec(), BValue::Bytes(node_id.to_vec()));
    encode_query(tx_id, b"ping", args)
}

fn encode_target_query(tx_id: &[u8], method: &[u8], node_id: &[u8; 20], target: &[u8; 20]) -> Vec<u8> {
    let mut args = BTreeMap::new();
    args.insert(b"id".to_vec(), BValue::Bytes(node_id.to_vec()));
    args.insert(b"target".to_vec(), BValue::Bytes(target.to_vec()));
    encode_query(tx_id, method, args)
}

/// Codifica una petición KRPC `find_node` (BEP 5).
pub fn encode_find_node(tx_id: &[u8], node_id: &[u8; 20], target: &[u8; 20]) -> Vec<u8> {
    encode_target_query(tx_id, b"find_node", node_id, target)
}

/// Codifica una petición KRPC `sample_infohashes` (BEP 51) para cosechar infohashes vivos sin indexadores.
pub fn encode_sample_infohashes(tx_id: &[u8], node_id: &[u8; 20], target: &[u8; 20]) -> Vec<u8> {
    encode_target_query(tx_id, b"sample_infohashes", node_id, target)
}

/// Codifica una petición KRPC `get_peers` (BEP 5) para descubrir pares de un `info_hash` o `rendezvous_id`.
pub fn encode_get_peers(tx_id: &[u8], node_id: &[u8; 20], info_hash: &[u8; 20]) -> Vec<u8> {
    let mut args = BTreeMap::new();
    args.insert(b"id".to_vec(), BValue::Bytes(node_id.to_vec()));
    args.insert(b"info_hash".to_vec(), BValue::Bytes(info_hash.to_vec()));
    encode_query(tx_id, b"get_peers", args)
}

/// Codifica una petición KRPC `announce_peer` (BEP 5) usando el `token` devuelto por `get_peers`.
pub fn encode_announce_peer(
    tx_id: &[u8],
    node_id: &[u8; 20],
    info_hash: &[u8; 20],
    port: u16,
    token: &[u8],
) -> Vec<u8> {
    let mut args = BTreeMap::new();
    args.insert(b"id".to_vec(), BValue::Bytes(node_id.to_vec()));
    args.insert(b"info_hash".to_vec(), BValue::Bytes(info_hash.to_vec()));
    args.insert(b"port".to_vec(), BValue::Int(i64::from(port)));
    args.insert(b"implied_port".to_vec(), BValue::Int(0));
    args.insert(b"token".to_vec(), BValue::Bytes(token.to_vec()));
    encode_query(tx_id, b"announce_peer", args)
}

/// Codifica una respuesta KRPC mínima (`ping` o acuse de `announce_peer`).
pub fn encode_simple_response(tx_id: &[u8], node_id: &[u8; 20]) -> Vec<u8> {
    let mut r = BTreeMap::new();
    r.insert(b"id".to_vec(), BValue::Bytes(node_id.to_vec()));
    encode_response(tx_id, r)
}

/// Codifica una respuesta `find_node` (BEP 5) devolviendo nodos reales en formato compacto IPv4
/// para que otros nodos DHT nos incorporen en sus tablas y nos envíen `announce_peer` pasivos.
pub fn encode_find_node_response(
    tx_id: &[u8],
    node_id: &[u8; 20],
    nodes: &[([u8; 20], SocketAddr)],
) -> Vec<u8> {
    let mut r = BTreeMap::new();
    r.insert(b"id".to_vec(), BValue::Bytes(node_id.to_vec()));
    r.insert(b"nodes".to_vec(), BValue::Bytes(encode_compact_nodes_v4(nodes)));
    encode_response(tx_id, r)
}

/// Codifica una respuesta `get_peers` con lista compacta de pares IPv4 y `token` de anuncio.
pub fn encode_get_peers_response(
    tx_id: &[u8],
    node_id: &[u8; 20],
    token: &[u8],
    peers: &[SocketAddr],
    nodes: &[([u8; 20], SocketAddr)],
) -> Vec<u8> {
    let mut r = BTreeMap::new();
    r.insert(b"id".to_vec(), BValue::Bytes(node_id.to_vec()));
    r.insert(b"token".to_vec(), BValue::Bytes(token.to_vec()));
    if !peers.is_empty() {
        let values: Vec<BValue> = peers
            .iter()
            .filter_map(|addr| encode_compact_peer_v4(*addr).map(BValue::Bytes))
            .collect();
        if !values.is_empty() {
            r.insert(b"values".to_vec(), BValue::List(values));
        }
    }
    if !nodes.is_empty() {
        r.insert(b"nodes".to_vec(), BValue::Bytes(encode_compact_nodes_v4(nodes)));
    }
    encode_response(tx_id, r)
}

/// Codifica una respuesta BEP 51 `sample_infohashes` (usada por nodos que sirven muestras y en pruebas loopback).
pub fn encode_sample_infohashes_response(
    tx_id: &[u8],
    node_id: &[u8; 20],
    samples: &[[u8; 20]],
    nodes: &[([u8; 20], SocketAddr)],
) -> Vec<u8> {
    let mut r = BTreeMap::new();
    r.insert(b"id".to_vec(), BValue::Bytes(node_id.to_vec()));
    r.insert(b"interval".to_vec(), BValue::Int(60));
    r.insert(b"num".to_vec(), BValue::Int(samples.len() as i64));
    let mut raw_samples = Vec::with_capacity(samples.len() * DHT_ID_LEN);
    for s in samples.iter().take(MAX_BEP51_SAMPLES_PER_PACKET) {
        raw_samples.extend_from_slice(s);
    }
    r.insert(b"samples".to_vec(), BValue::Bytes(raw_samples));
    if !nodes.is_empty() {
        r.insert(b"nodes".to_vec(), BValue::Bytes(encode_compact_nodes_v4(nodes)));
    }
    encode_response(tx_id, r)
}

fn encode_query(tx_id: &[u8], q: &[u8], args: BTreeMap<Vec<u8>, BValue>) -> Vec<u8> {
    let mut root = BTreeMap::new();
    root.insert(b"t".to_vec(), BValue::Bytes(tx_id.to_vec()));
    root.insert(b"y".to_vec(), BValue::Bytes(b"q".to_vec()));
    root.insert(b"q".to_vec(), BValue::Bytes(q.to_vec()));
    root.insert(b"a".to_vec(), BValue::Dict(args));
    BValue::Dict(root).encode()
}

fn encode_response(tx_id: &[u8], r: BTreeMap<Vec<u8>, BValue>) -> Vec<u8> {
    let mut root = BTreeMap::new();
    root.insert(b"t".to_vec(), BValue::Bytes(tx_id.to_vec()));
    root.insert(b"y".to_vec(), BValue::Bytes(b"r".to_vec()));
    root.insert(b"r".to_vec(), BValue::Dict(r));
    BValue::Dict(root).encode()
}

/// Decodifica un datagrama UDP KRPC en `KrpcMessage`.
pub fn decode_krpc_packet(bytes: &[u8]) -> Option<KrpcMessage> {
    let val = decode(bytes).ok()?;
    let tx_id = val.dict_get(b"t")?.as_bytes()?.to_vec();
    let msg_type = val.dict_get(b"y")?.as_bytes()?;

    match msg_type {
        b"q" => {
            let query_name = val.dict_get(b"q")?.as_str_lossy()?;
            let args = val.dict_get(b"a")?;
            let sender_id = parse_id20(args.dict_get(b"id")?.as_bytes()?)?;
            let info_hash = args
                .dict_get(b"info_hash")
                .and_then(BValue::as_bytes)
                .and_then(parse_id20);
            let target = args
                .dict_get(b"target")
                .and_then(BValue::as_bytes)
                .and_then(parse_id20);
            let port = args
                .dict_get(b"port")
                .and_then(BValue::as_int)
                .and_then(|p| u16::try_from(p).ok());
            let implied_port = args
                .dict_get(b"implied_port")
                .and_then(BValue::as_int)
                .is_some_and(|v| v != 0);
            Some(KrpcMessage::Query {
                tx_id,
                query_name,
                sender_id,
                info_hash,
                target,
                port,
                implied_port,
            })
        }
        b"r" => {
            let resp = val.dict_get(b"r")?;
            let sender_id = resp
                .dict_get(b"id")
                .and_then(BValue::as_bytes)
                .and_then(parse_id20);
            let nodes = resp
                .dict_get(b"nodes")
                .and_then(BValue::as_bytes)
                .map(decode_compact_nodes_v4)
                .unwrap_or_default();
            let peers = resp
                .dict_get(b"values")
                .and_then(BValue::as_list)
                .map(decode_compact_peers_v4)
                .unwrap_or_default();
            let samples = resp
                .dict_get(b"samples")
                .and_then(BValue::as_bytes)
                .map(decode_bep51_samples)
                .unwrap_or_default();
            let token = resp
                .dict_get(b"token")
                .and_then(BValue::as_bytes)
                .map(|b| b.to_vec());
            Some(KrpcMessage::Response {
                tx_id,
                sender_id,
                nodes,
                peers,
                samples,
                token,
            })
        }
        _ => None,
    }
}

fn parse_id20(slice: &[u8]) -> Option<[u8; 20]> {
    if slice.len() != DHT_ID_LEN {
        return None;
    }
    let mut id = [0u8; 20];
    id.copy_from_slice(slice);
    Some(id)
}

/// Decodifica la cadena compacta de nodos IPv4 de BEP 5 (`26` bytes por nodo).
pub fn decode_compact_nodes_v4(raw: &[u8]) -> Vec<([u8; 20], SocketAddr)> {
    let (chunks, _) = raw.as_chunks::<COMPACT_NODE_V4_LEN>();
    let mut out = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let mut id = [0u8; 20];
        id.copy_from_slice(&chunk[..20]);
        let ip = Ipv4Addr::new(chunk[20], chunk[21], chunk[22], chunk[23]);
        let port = u16::from_be_bytes([chunk[24], chunk[25]]);
        if port > 0 {
            out.push((id, SocketAddr::V4(SocketAddrV4::new(ip, port))));
        }
    }
    out
}

/// Codifica nodos IPv4 al formato compacto de 26 bytes por nodo (BEP 5).
pub fn encode_compact_nodes_v4(nodes: &[([u8; 20], SocketAddr)]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(nodes.len() * COMPACT_NODE_V4_LEN);
    for (id, addr) in nodes {
        if let SocketAddr::V4(v4) = addr {
            buf.extend_from_slice(id);
            buf.extend_from_slice(&v4.ip().octets());
            buf.extend_from_slice(&v4.port().to_be_bytes());
        }
    }
    buf
}

fn encode_compact_peer_v4(addr: SocketAddr) -> Option<Vec<u8>> {
    if let SocketAddr::V4(v4) = addr {
        let mut out = Vec::with_capacity(COMPACT_PEER_V4_LEN);
        out.extend_from_slice(&v4.ip().octets());
        out.extend_from_slice(&v4.port().to_be_bytes());
        Some(out)
    } else {
        None
    }
}

/// Decodifica una secuencia compacta de pares IPv4 (`6` bytes por par: 4 IP + 2 puerto BE),
/// reutilizada tanto por KRPC `get_peers` (BEP 5) como por `ut_pex` (BEP 11).
pub fn decode_compact_peers_v4_slice(raw: &[u8]) -> Vec<SocketAddr> {
    let (chunks, _) = raw.as_chunks::<COMPACT_PEER_V4_LEN>();
    let mut out = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let ip = Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]);
        let port = u16::from_be_bytes([chunk[4], chunk[5]]);
        if port > 0 {
            out.push(SocketAddr::V4(SocketAddrV4::new(ip, port)));
        }
    }
    out
}

fn decode_compact_peers_v4(values: &[BValue]) -> Vec<SocketAddr> {
    let mut out = Vec::with_capacity(values.len());
    for entry in values {
        if let Some(raw) = entry.as_bytes() {
            out.extend(decode_compact_peers_v4_slice(raw));
        }
    }
    out
}

fn decode_bep51_samples(raw: &[u8]) -> Vec<[u8; 20]> {
    let (chunks, _) = raw.as_chunks::<DHT_ID_LEN>();
    chunks
        .iter()
        .take(MAX_BEP51_SAMPLES_PER_PACKET)
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendezvous_id_is_deterministic_and_distinct_per_ed2k_hash() {
        let h1 = [0x11u8; 16];
        let h2 = [0x22u8; 16];
        assert_eq!(derive_ed2k_rendezvous_id(&h1), derive_ed2k_rendezvous_id(&h1));
        assert_ne!(derive_ed2k_rendezvous_id(&h1), derive_ed2k_rendezvous_id(&h2));
    }

    #[test]
    fn krpc_sample_infohashes_and_get_peers_roundtrip() {
        let node_id = [0xAAu8; 20];
        let sample_hash = [0x55u8; 20];
        let peer_addr: SocketAddr = "127.0.0.1:51413".parse().unwrap();

        let resp_bytes = encode_sample_infohashes_response(
            b"tx1",
            &node_id,
            &[sample_hash],
            &[([0xBBu8; 20], peer_addr)],
        );
        match decode_krpc_packet(&resp_bytes).unwrap() {
            KrpcMessage::Response {
                tx_id,
                sender_id,
                nodes,
                samples,
                ..
            } => {
                assert_eq!(tx_id, b"tx1");
                assert_eq!(sender_id, Some(node_id));
                assert_eq!(samples, vec![sample_hash]);
                assert_eq!(nodes.len(), 1);
                assert_eq!(nodes[0].1, peer_addr);
            }
            _ => panic!("se esperaba una respuesta KRPC"),
        }

        let peers_bytes =
            encode_get_peers_response(b"tx2", &node_id, b"tok", &[peer_addr], &[]);
        match decode_krpc_packet(&peers_bytes).unwrap() {
            KrpcMessage::Response { peers, token, .. } => {
                assert_eq!(peers, vec![peer_addr]);
                assert_eq!(token.as_deref(), Some(b"tok".as_slice()));
            }
            _ => panic!("se esperaba respuesta get_peers"),
        }
    }
}

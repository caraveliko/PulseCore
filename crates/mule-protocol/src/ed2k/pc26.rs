// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Extensión retrocompatible `PC26` para el anuncio de capacidades modernas en `OP_HELLO`.
//!
//! Todas las capacidades viajan en etiquetas TLV estándar con nombre textual, de modo
//! que los clientes clásicos de eMule/aMule las ignoran sin romper compatibilidad.

use super::tags::{Ed2kTag, TagName, TagValue};

/// Nombre de etiqueta TLV para la máscara de capacidades `PC26` (`u32`).
pub const TAG_PC26_CAPS: &str = "pc26_caps";
/// Nombre de etiqueta TLV para la clave pública Ed25519 (`[u8; 32]`).
pub const TAG_PC26_ED25519: &str = "pc26_ed25519";
/// Nombre de etiqueta TLV para el punto de enlace IPv6 (`[u8; 18]`: IPv6 + puerto u16 LE).
pub const TAG_PC26_IPV6: &str = "pc26_ipv6";
/// Nombre de etiqueta TLV para el puerto UDP QUIC del par (`u16`).
pub const TAG_PC26_QUIC_PORT: &str = "pc26_quic";

/// Longitud exacta de la clave pública Ed25519 en `pc26_ed25519`.
pub const PC26_ED25519_BLOB_LEN: usize = 32;
/// Longitud exacta del blob `pc26_ipv6` (16 bytes de dirección + 2 bytes de puerto LE).
pub const PC26_IPV6_BLOB_LEN: usize = 18;

/// Bit 0: Transporte AEAD X25519 + ChaCha20-Poly1305.
pub const PC26_CAP_X25519_CHACHA: u32 = 1 << 0;
/// Bit 1: Intercambio híbrido post-cuántico X25519 + ML-KEM-768.
pub const PC26_CAP_HYBRID_MLKEM768: u32 = 1 << 1;
/// Bit 2: SUI v3 Ed25519 con vinculación de canal sobre `transcript_hash`.
pub const PC26_CAP_ED25519_SUI_V3: u32 = 1 << 2;
/// Bit 3: Compresión Zstd acotada para bloques de datos.
pub const PC26_CAP_ZSTD: u32 = 1 << 3;
/// Bit 4: Verificación por árbol de Merkle BLAKE3.
pub const PC26_CAP_BLAKE3_MERKLE: u32 = 1 << 4;
/// Bit 5: Transporte QUIC RFC 9000 (`pulsecore-ed2k/1`).
pub const PC26_CAP_QUIC: u32 = 1 << 5;
/// Bit 6: Dual-Stack IPv6.
pub const PC26_CAP_IPV6: u32 = 1 << 6;
/// Bit 7: Coordinación de perforación simultánea UDP.
pub const PC26_CAP_UDP_HOLE_PUNCH: u32 = 1 << 7;

/// Capacidades modernas anunciadas por un par compatible con `PC26`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pc26PeerCaps {
    /// Máscara de bits `PC26_CAP_*`.
    pub flags: u32,
    /// Clave pública Ed25519 del par (32 bytes), si la anunció.
    pub ed25519_pubkey: Option<[u8; PC26_ED25519_BLOB_LEN]>,
    /// Octetos IPv6 (16 bytes) y puerto TCP alcanzable, si lo anunció.
    pub ipv6_endpoint: Option<([u8; 16], u16)>,
    /// Puerto UDP donde el par escucha conexiones QUIC `pulsecore-ed2k/1`.
    pub quic_port: Option<u16>,
}

impl Pc26PeerCaps {
    /// Indica si el par soporta el canal cifrado moderno X25519 + ChaCha20-Poly1305.
    #[inline]
    pub const fn supports_x25519_chacha(&self) -> bool {
        (self.flags & PC26_CAP_X25519_CHACHA) != 0
    }

    /// Indica si el par soporta el modo híbrido post-cuántico X25519 + ML-KEM-768.
    #[inline]
    pub const fn supports_hybrid_mlkem768(&self) -> bool {
        (self.flags & PC26_CAP_HYBRID_MLKEM768) != 0
    }

    /// Indica si el par soporta SUI v3 Ed25519 con vinculación de canal.
    #[inline]
    pub const fn supports_ed25519_sui_v3(&self) -> bool {
        (self.flags & PC26_CAP_ED25519_SUI_V3) != 0 && self.ed25519_pubkey.is_some()
    }

    /// Indica si el par soporta compresión Zstd acotada.
    #[inline]
    pub const fn supports_zstd(&self) -> bool {
        (self.flags & PC26_CAP_ZSTD) != 0
    }

    /// Indica si el par soporta verificación BLAKE3 Merkle.
    #[inline]
    pub const fn supports_blake3_merkle(&self) -> bool {
        (self.flags & PC26_CAP_BLAKE3_MERKLE) != 0
    }

    /// Indica si el par anunció soporte QUIC junto con un puerto válido.
    #[inline]
    pub const fn supports_quic(&self) -> bool {
        (self.flags & PC26_CAP_QUIC) != 0 && matches!(self.quic_port, Some(p) if p > 0)
    }

    /// Indica si el par soporta coordinación de perforación UDP.
    #[inline]
    pub const fn supports_udp_hole_punch(&self) -> bool {
        (self.flags & PC26_CAP_UDP_HOLE_PUNCH) != 0
    }
}

/// Construye las etiquetas TLV estándar correspondientes a `caps` para incluirlas en `OP_HELLO` / `OP_HELLO_ANSWER`.
pub fn build_pc26_hello_tags(caps: &Pc26PeerCaps) -> Vec<Ed2kTag> {
    let mut tags = Vec::with_capacity(4);
    if caps.flags == 0
        && caps.ed25519_pubkey.is_none()
        && caps.ipv6_endpoint.is_none()
        && caps.quic_port.is_none()
    {
        return tags;
    }

    tags.push(Ed2kTag {
        name: TagName::Text(TAG_PC26_CAPS.to_string()),
        value: TagValue::UInt32(caps.flags),
    });

    if let Some(pk) = caps.ed25519_pubkey {
        tags.push(Ed2kTag {
            name: TagName::Text(TAG_PC26_ED25519.to_string()),
            value: TagValue::Blob(pk.to_vec()),
        });
    }

    if let Some((ip6_octets, port)) = caps.ipv6_endpoint {
        let mut blob = Vec::with_capacity(PC26_IPV6_BLOB_LEN);
        blob.extend_from_slice(&ip6_octets);
        blob.extend_from_slice(&port.to_le_bytes());
        tags.push(Ed2kTag {
            name: TagName::Text(TAG_PC26_IPV6.to_string()),
            value: TagValue::Blob(blob),
        });
    }

    if let Some(port) = caps.quic_port.filter(|&p| p > 0) {
        tags.push(Ed2kTag {
            name: TagName::Text(TAG_PC26_QUIC_PORT.to_string()),
            value: TagValue::UInt16(port),
        });
    }

    tags
}

/// Extrae `Pc26PeerCaps` desde las etiquetas decodificadas de un saludo eD2K.
/// Devuelve `None` si el par no anunció ninguna etiqueta `pc26_*` (par clásico eMule/aMule).
pub fn parse_pc26_tags(tags: &[Ed2kTag]) -> Option<Pc26PeerCaps> {
    let mut saw_any = false;
    let mut caps = Pc26PeerCaps::default();

    for tag in tags {
        let TagName::Text(name) = &tag.name else {
            continue;
        };
        match name.as_str() {
            TAG_PC26_CAPS => {
                if let Some(v) = extract_u32(&tag.value) {
                    caps.flags = v;
                    saw_any = true;
                }
            }
            TAG_PC26_ED25519 => {
                if let TagValue::Blob(bytes) = &tag.value {
                    if let Ok(arr) = <[u8; PC26_ED25519_BLOB_LEN]>::try_from(bytes.as_slice()) {
                        caps.ed25519_pubkey = Some(arr);
                        saw_any = true;
                    }
                }
            }
            TAG_PC26_IPV6 => {
                if let TagValue::Blob(bytes) = &tag.value {
                    if bytes.len() == PC26_IPV6_BLOB_LEN {
                        let mut octets = [0u8; 16];
                        octets.copy_from_slice(&bytes[..16]);
                        let port = u16::from_le_bytes([bytes[16], bytes[17]]);
                        if port > 0 {
                            caps.ipv6_endpoint = Some((octets, port));
                            saw_any = true;
                        }
                    }
                }
            }
            TAG_PC26_QUIC_PORT => {
                if let Some(port) = extract_u16(&tag.value) {
                    if port > 0 {
                        caps.quic_port = Some(port);
                        saw_any = true;
                    }
                }
            }
            _ => {}
        }
    }

    saw_any.then_some(caps)
}

fn extract_u32(val: &TagValue) -> Option<u32> {
    match *val {
        TagValue::UInt32(v) => Some(v),
        TagValue::UInt16(v) => Some(u32::from(v)),
        TagValue::UInt8(v) => Some(u32::from(v)),
        _ => None,
    }
}

fn extract_u16(val: &TagValue) -> Option<u16> {
    match *val {
        TagValue::UInt16(v) => Some(v),
        TagValue::UInt32(v) => u16::try_from(v).ok(),
        TagValue::UInt8(v) => Some(u16::from(v)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ed2k::{Ed2kDecoder, Ed2kEncoder, Ed2kHello};

    #[test]
    fn pc26_hello_tags_roundtrip_and_classic_peer_safety() {
        let expected = Pc26PeerCaps {
            flags: PC26_CAP_X25519_CHACHA
                | PC26_CAP_HYBRID_MLKEM768
                | PC26_CAP_ED25519_SUI_V3
                | PC26_CAP_ZSTD
                | PC26_CAP_BLAKE3_MERKLE
                | PC26_CAP_QUIC
                | PC26_CAP_IPV6
                | PC26_CAP_UDP_HOLE_PUNCH,
            ed25519_pubkey: Some([0x42u8; 32]),
            ipv6_endpoint: Some(([0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1], 39735)),
            quic_port: Some(39738),
        };

        let hello = Ed2kHello {
            user_hash: [0x11u8; 16],
            client_id: 0x0102_0304,
            tcp_port: 39735,
            tags: build_pc26_hello_tags(&expected),
            unknown_tags: vec![],
            server_ip: 0,
            server_port: 0,
        };

        let enc = Ed2kEncoder::default();
        let dec = Ed2kDecoder::default();
        let pkt = enc.encode_hello(&hello).unwrap();
        let decoded = dec
            .decode_hello(pkt.protocol, pkt.opcode, &pkt.payload, 0)
            .unwrap();

        let parsed = parse_pc26_tags(&decoded.tags).expect("debe decodificar pc26_caps");
        assert_eq!(parsed, expected);
        assert!(parsed.supports_hybrid_mlkem768());
        assert!(parsed.supports_ed25519_sui_v3());
        assert!(parsed.supports_quic());
        assert!(parsed.supports_zstd());
        assert!(parsed.supports_blake3_merkle());
        assert!(parsed.supports_udp_hole_punch());

        // Un saludo sin etiquetas pc26_* devuelve None.
        assert!(parse_pc26_tags(&[]).is_none());
    }
}

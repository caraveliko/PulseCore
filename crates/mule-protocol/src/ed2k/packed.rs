// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Compresión de paquetes `OP_PACKEDPROT` (`0xD4`) — eMule `Packet::PackPacket`
//! / `Packet::UnPackPacket` (`Packets.cpp:200-244`).
//!
//! Una trama empaquetada conserva la misma disposición de cabecera — `[0xD4][u32 len][opcode]
//! [flujo zlib]` — el código de operación permanece sin comprimir para que el receptor pueda
//! enrutar antes de descomprimir. Una vez desempaquetada, eMule reetiqueta el paquete como
//! `OP_EMULEPROT` (las tramas Kad `0xD5` vuelven a `OP_KADEMLIAHEADER`), por lo que
//! las cargas útiles empaquetadas solo pueden transportar códigos de operación PR_EMULE en el canal entre pares.

use std::io::Read;

/// Límite por defecto de `Packet::UnPackPacket` de eMule para paquetes entre **pares**
/// (`uMaxDecompressedSize = 50000u`). Los paquetes de servidor usan 250 KB en origen;
/// los llamantes eligen el límite mediante el argumento de `unpack_payload`.
pub const PACKED_PEER_MAX_DECOMPRESSED: usize = 50_000;

/// eMule emite `ANSWERSOURCES`/`ANSWERSOURCES2` comprimidos cuando el paquete
/// supera este tamaño (`KnownFile.cpp:1149`, `PartFile.cpp:3708` —
/// `result->size > 354`, donde `size` cuenta el código de operación + carga útil).
pub const PACK_SRCINFO_MIN_SIZE: usize = 354;

/// `CKademliaUDPListener::SendPacket` de eMule empaqueta cualquier carga útil UDP de Kad
/// (datagrama menos la cabecera de 2 bytes `[proto][opcode]`) mayor que esto
/// (`KademliaUDPListener.cpp:1875/1891/1904` — `uLenData > 200`).
pub const KAD_UDP_PACK_MIN_SIZE: usize = 200;

/// Límite de descompresión de `ClientUDPSocket` de eMule para `OP_KADEMLIAPACKEDPROT`
/// (`ClientUDPSocket.cpp:93` — bucle de reintento acotado por `nNewSize < 250000`).
pub const PACKED_KAD_UDP_MAX_DECOMPRESSED: usize = 250_000;

/// Comprime `payload` con zlib en `Z_BEST_COMPRESSION` y devuelve el flujo
/// comprimido **únicamente cuando es estrictamente menor** que la entrada —
/// en caso contrario, eMule envía el paquete sin comprimir (`Packets.cpp:204-211`).
pub fn pack_payload(payload: &[u8]) -> Option<Vec<u8>> {
    let compressed = {
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        use std::io::Write;
        enc.write_all(payload).ok()?;
        enc.finish().ok()?
    };
    (compressed.len() < payload.len()).then_some(compressed)
}

/// Descomprime una carga útil empaquetada con un límite estricto de tamaño descomprimido, reflejando
/// `Packet::UnPackPacket(uMaxDecompressedSize)`. Devuelve `None` en datos
/// truncados, corruptos o de tamaño excesivo.
pub fn unpack_payload(payload: &[u8], max_decompressed: usize) -> Option<Vec<u8>> {
    let mut decoder = flate2::read::ZlibDecoder::new(payload).take(max_decompressed as u64 + 1);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).ok()?;
    // eMule acepta un flujo zlib válido incluso cuando se descomprime a cero
    // bytes (`size = unpackedsize` — el paquete se despacha vacío).
    if out.len() > max_decompressed {
        return None;
    }
    Some(out)
}

/// Desempaqueta una trama `PR_EMULE_COMPRESSED` (`0xD4`) en el paquete que ven los
/// despachadores de eMule: mismo código de operación, carga útil descomprimida, protocolo reetiquetado como
/// `PR_EMULE` (`Packets.cpp:240` — los paquetes empaquetados que no son de Kad siempre van a
/// la rama EMULEPROT). Devuelve `None` en paquetes con protocolo sin cambios o
/// cuando la descompresión falla.
pub fn unpack_packet(
    packet: &crate::framing::RawPacket,
    max_decompressed: usize,
) -> Option<crate::framing::RawPacket> {
    if packet.protocol != super::protocol::PR_EMULE_COMPRESSED {
        return None;
    }
    let payload = unpack_payload(&packet.payload, max_decompressed)?;
    Some(crate::framing::RawPacket {
        protocol: super::protocol::PR_EMULE,
        opcode: packet.opcode,
        payload: payload.into(),
    })
}

/// Normalización a nivel de socket que refleja `CEMSocket::ProcessPacket` /
/// `Packet::UnPackPacket`: deja pasar `packet` sin cambios salvo que su
/// protocolo sea `PR_EMULE_COMPRESSED`, en cuyo caso se descomprime la carga útil
/// y se reetiqueta el paquete como `PR_EMULE`. Devuelve `None` cuando una carga útil empaquetada
/// está corrupta o excede `max_decompressed` — el llamante descarta el paquete.
pub fn unpack_owned(
    packet: crate::framing::RawPacket,
    max_decompressed: usize,
) -> Option<crate::framing::RawPacket> {
    if packet.protocol != super::protocol::PR_EMULE_COMPRESSED {
        return Some(packet);
    }
    unpack_packet(&packet, max_decompressed)
}

/// Devuelve el paquete a enviar por cable: reentramado como `PR_EMULE_COMPRESSED`
/// (`0xD4`) cuando `packet.frame_len()` (código de operación + carga útil, al igual que
/// `result->size` de eMule) supera `pack_min` **y** la compresión zlib reduce
/// estrictamente la carga útil — la condición de paso de `Packet::PackPacket` aplicada por
/// `CreateSrcInfoPacket` en `OP_ANSWERSOURCES{,2}`. En caso contrario, devuelve el
/// paquete sin cambios.
#[must_use]
pub fn maybe_pack(
    packet: &crate::framing::RawPacket,
    pack_min: usize,
) -> crate::framing::RawPacket {
    if packet.frame_len() > pack_min {
        if let Some(compressed) = pack_payload(&packet.payload) {
            return crate::framing::RawPacket {
                protocol: super::protocol::PR_EMULE_COMPRESSED,
                opcode: packet.opcode,
                payload: compressed.into(),
            };
        }
    }
    packet.clone()
}

/// Empaqueta un datagrama UDP de Kad `[proto][opcode][payload]` del mismo modo
/// que lo hace `CKademliaUDPListener::SendPacket`: cuando la carga útil supera
/// `KAD_UDP_PACK_MIN_SIZE` y zlib la reduce estrictamente, el datagrama se
/// reemite como `[PR_KADEMLIA_PACKED (0xE5)][opcode][flujo zlib]`.
/// Cualquier otra cosa — protocolo no Kad, carga útil pequeña, datos no comprimibles —
/// se devuelve sin cambios (prestada, sin copia).
pub fn pack_kad_datagram(datagram: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    use super::super::kad::opcodes::{PR_KADEMLIA, PR_KADEMLIA_PACKED};
    if datagram.len() > 2
        && datagram[0] == PR_KADEMLIA
        && datagram.len() - 2 > KAD_UDP_PACK_MIN_SIZE
    {
        if let Some(compressed) = pack_payload(&datagram[2..]) {
            let mut out = Vec::with_capacity(2 + compressed.len());
            out.push(PR_KADEMLIA_PACKED);
            out.push(datagram[1]);
            out.extend_from_slice(&compressed);
            return out.into();
        }
    }
    datagram.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip() {
        // Los datos repetitivos se comprimen muy por debajo del umbral.
        let payload = vec![b'A'; 400];
        let packed = pack_payload(&payload).expect("should pack");
        assert!(packed.len() < payload.len());
        assert_eq!(
            unpack_payload(&packed, PACKED_PEER_MAX_DECOMPRESSED).unwrap(),
            payload
        );
    }

    #[test]
    fn pack_rejects_incompressible_data() {
        // La entrada de alta entropía permanece sin comprimir, al igual que en eMule.
        let mut payload = Vec::with_capacity(300);
        let mut s: u32 = 0x1234_5678;
        for _ in 0..300 {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            payload.push((s >> 24) as u8);
        }
        assert!(pack_payload(&payload).is_none());
    }

    #[test]
    fn unpack_enforces_cap() {
        // ~300 KB de ceros → unos pocos cientos de bytes comprimidos (patrón de bomba de descompresión).
        let big = vec![0u8; 300_000];
        let packed = pack_payload(&big).expect("zeros compress");
        assert!(unpack_payload(&packed, PACKED_PEER_MAX_DECOMPRESSED).is_none());
        assert!(unpack_payload(&packed, 250_000).is_none());
        // Con el tamaño descomprimido, los mismos datos deben pasar.
        assert_eq!(unpack_payload(&packed, 300_000).unwrap().len(), 300_000);
    }

    #[test]
    fn unpack_rejects_garbage_and_truncation() {
        assert!(unpack_payload(b"not zlib at all", PACKED_PEER_MAX_DECOMPRESSED).is_none());
        let mut packed = pack_payload(&vec![b'x'; 400]).unwrap();
        packed.truncate(packed.len() - 2);
        assert!(unpack_payload(&packed, PACKED_PEER_MAX_DECOMPRESSED).is_none());
        // Una carga útil de 1 byte no puede reducirse — nunca se emite empaquetada.
        assert!(pack_payload(&[0xAA]).is_none());
    }

    #[test]
    fn unpack_owned_retags_emuleprot() {
        use super::super::protocol::{PR_EMULE, PR_EMULE_COMPRESSED};
        let body = b"source exchange answer payload".repeat(20);
        let pkt = crate::framing::RawPacket {
            protocol: PR_EMULE_COMPRESSED,
            opcode: 0x99,
            payload: pack_payload(&body).unwrap().into(),
        };
        let unpacked = unpack_owned(pkt, PACKED_PEER_MAX_DECOMPRESSED).unwrap();
        assert_eq!(unpacked.protocol, PR_EMULE);
        assert_eq!(unpacked.opcode, 0x99);
        assert_eq!(&*unpacked.payload, &*body);
        // Los paquetes no empaquetados pasan intactos.
        let plain = crate::framing::RawPacket {
            protocol: PR_EMULE,
            opcode: 0x99,
            payload: body.clone().into(),
        };
        let same = unpack_owned(plain, PACKED_PEER_MAX_DECOMPRESSED).unwrap();
        assert_eq!(same.protocol, PR_EMULE);
        assert_eq!(&*same.payload, &*body);
        // Un paquete empaquetado corrupto se descarta, no se despacha.
        let bad = crate::framing::RawPacket {
            protocol: PR_EMULE_COMPRESSED,
            opcode: 0x99,
            payload: b"junk".as_slice().into(),
        };
        assert!(unpack_owned(bad, PACKED_PEER_MAX_DECOMPRESSED).is_none());
    }

    #[test]
    fn maybe_pack_gates_on_size_and_gain() {
        use super::super::protocol::{PR_EMULE, PR_EMULE_COMPRESSED};
        let mk = |payload: Vec<u8>| crate::framing::RawPacket {
            protocol: PR_EMULE,
            opcode: 0x98,
            payload: payload.into(),
        };
        // Por debajo del umbral 354: nunca se empaqueta, incluso si es comprimible.
        let small = mk(vec![b'z'; 200]);
        assert_eq!(maybe_pack(&small, PACK_SRCINFO_MIN_SIZE).protocol, PR_EMULE);
        // Grande + comprimible: se empaqueta y se descomprime de forma idéntica.
        let big = mk(vec![b'z'; 2_000]);
        let packed = maybe_pack(&big, PACK_SRCINFO_MIN_SIZE);
        assert_eq!(packed.protocol, PR_EMULE_COMPRESSED);
        assert!(packed.payload.len() < 2_000);
        let restored = unpack_owned(packed, PACKED_PEER_MAX_DECOMPRESSED).unwrap();
        assert_eq!(restored.protocol, PR_EMULE);
        assert_eq!(&*restored.payload, &vec![b'z'; 2_000][..]);
        // Grande pero no comprimible: permanece sin comprimir.
        let mut noise = Vec::with_capacity(2_000);
        let mut s: u32 = 0xDEAD_BEEF;
        for _ in 0..2_000 {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            noise.push((s >> 24) as u8);
        }
        let n = mk(noise);
        assert_eq!(maybe_pack(&n, PACK_SRCINFO_MIN_SIZE).protocol, PR_EMULE);
    }

    #[test]
    fn pack_kad_datagram_retags_packed_prot() {
        use super::super::protocol::PR_EMULE;
        use crate::kad::opcodes::{PR_KADEMLIA, PR_KADEMLIA_PACKED};
        // Datagrama Kad comprimible > 200 bytes de carga útil → 0xE5.
        let mut dgram = vec![PR_KADEMLIA, 0x3B];
        dgram.extend_from_slice(&vec![0xAA; 500]);
        let packed = pack_kad_datagram(&dgram);
        assert_eq!(packed[0], PR_KADEMLIA_PACKED);
        assert_eq!(packed[1], 0x3B);
        assert!(packed.len() < dgram.len());
        // …y se descomprime de nuevo idénticamente.
        assert_eq!(
            unpack_payload(&packed[2..], PACKED_KAD_UDP_MAX_DECOMPRESSED)
                .unwrap()
                .len(),
            500
        );
        // Datagrama pequeño → sin cambios (prestado, sin copia).
        let small = vec![PR_KADEMLIA, 0x10, 0x00];
        assert!(matches!(
            pack_kad_datagram(&small),
            std::borrow::Cow::Borrowed(_)
        ));
        // Protocolo no Kad nunca se empaqueta.
        let mut not_kad = vec![PR_EMULE, 0x99];
        not_kad.extend_from_slice(&vec![0xAA; 500]);
        assert_eq!(pack_kad_datagram(&not_kad)[0], PR_EMULE);
    }
}

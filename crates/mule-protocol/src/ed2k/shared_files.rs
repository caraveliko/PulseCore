// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Exploración de ficheros compartidos entre pares (eMule `ListenSocket.cpp:651-825`).
//!
//! Un par conectado puede explorar nuestros ficheros compartidos mediante tres peticiones PR_ED2K
//! cuando el bit 2 de `MISCOPTIONS1` (`NoViewSharedFiles`) está desactivado:
//!
//! - `OP_ASKSHAREDFILES` (`0x4A`, vacío) → [`encode_shared_files_answer`]
//!   devuelve la lista completa (`0x4B`).
//! - `OP_ASKSHAREDDIRS` (`0x5D`, vacío) → [`encode_shared_dirs_answer`]
//!   devuelve la lista de directorios (`0x5F`); denegado → [`encode_shared_denied`].
//! - `OP_ASKSHAREDFILESDIR` (`0x5E`, `<u16 len><dir>`) →
//!   [`encode_shared_files_dir_answer`] devuelve `0x60` con los ficheros del
//!   pseudodirectorio; denegado → [`encode_shared_denied`].
//!
//! Todas las respuestas se entraman sobre PR_ED2K (`Packet` de eMule usa por defecto
//! `OP_EDONKEYPROT` — `Packets.h:49`). Los códigos de operación de respuesta `0x4B`, `0x60` y
//! `0x61` colisionan con `OP_REQUESTPARTS`, `OP_QUEUERANKING` y `OP_FILEDESC`
//! respectivamente; eMule resuelve la ambigüedad por la dirección (las respuestas siempre
//! fluyen anfitrión de compartidos → explorador) y por el byte de protocolo (`QUEUERANKING`/`FILEDESC`
//! son exclusivos de PR_EMULE).
//!
//! Las entradas reutilizan la disposición `OfferedFile` de `OP_OFFERFILES`
//! (`CSharedFileList::CreateOfferedFilePacket` con `pServer == NULL`):
//! `hash[16] + client_id u32 + tcp_port u16 + tag list`. Para los clientes, el
//! ID y el puerto son nuestra dirección real solo con HighID, y los ficheros grandes llevan una
//! única etiqueta `FT_FILESIZE` de 64 bits en lugar del par de u32 divididos utilizado para
//! los servidores.

use crate::ed2k::encoder::Ed2kEncoder;
use crate::ed2k::error::Ed2kEncodeError;
use crate::ed2k::limits::Ed2kLimits;
use crate::ed2k::opcodes::{
    OP_ASKSHAREDDENIEDANS, OP_ASKSHAREDDIRSANS, OP_ASKSHAREDFILESDIRANS,
    OP_ASKSHAREDFILESANSWER,
};
use crate::ed2k::protocol::PR_ED2K;
use crate::ed2k::server::OfferedFile;
use crate::ed2k::tags::{Ed2kTag, TagName, TagValue, TAG_SPECIAL_FILESIZE, TAG_SPECIAL_FILETYPE};
use crate::framing::RawPacket;

/// Pseudodirectorio que lista los ficheros parciales en progreso
/// (`OP_INCOMPLETE_SHARED_FILES` de eMule, `opcodes.h:241`).
pub const OP_INCOMPLETE_SHARED_FILES: &str = "!Incomplete Files";

/// Pseudodirectorio para compartidos que no están bajo ningún directorio compartido
/// (`OP_OTHER_SHARED_FILES` de eMule, `opcodes.h:242`).
pub const OP_OTHER_SHARED_FILES: &str = "!Other";

/// `FT_FILERATING` (`0xF7`, `<uint8>`) — `opcodes.h:414` de eMule.
/// Para respuestas destinadas a clientes, la calificación 0..5 se escala a 0..255
/// (`SharedFileList.cpp:960`: `rating * 255/5`).
const FT_FILERATING: u8 = 0xF7;

/// Formato en cable de `WriteString` de eMule: `<longitud u16><bytes utf8>`.
/// Seguridad estilo `MAX_CLIENT_MSG_LEN`: las cadenas de directorio están acotadas por el
/// límite genérico de trama de todos modos; una longitud desmesurada errónea simplemente falla la comprobación de límites.
fn write_str(dst: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(u16::MAX as usize) as u16;
    dst.extend_from_slice(&len.to_le_bytes());
    dst.extend_from_slice(&bytes[..len as usize]);
}

/// Contexto de conexión que decide qué revela sobre nosotros una respuesta de exploración
/// (variante para pares de `CreateOfferedFilePacket`, `SharedFileList.cpp:883` con
/// `pServer == NULL`).
#[derive(Clone, Copy, Debug)]
pub struct OfferPeerContext {
    /// Nuestro ID de cliente eD2K (enviado solo cuando `our_high_id` está activo).
    pub our_client_id: u32,
    /// Nuestro puerto TCP de escucha (enviado solo cuando `our_high_id` está activo).
    pub our_tcp_port: u16,
    /// Si actualmente poseemos un HighID — LowID escribe `0`/`0`.
    pub our_high_id: bool,
    /// Si el par *explorador* anunció soporte de ficheros grandes
    /// (bit 4 de misc2) — habilita `FT_FILESIZE` de 64 bits y la entrada en sí.
    pub peer_large_files: bool,
}

/// Construye la variante para pares de `CreateOfferedFilePacket`
/// (`SharedFileList.cpp:883` con `pServer == NULL`):
///
/// - `client_id`/`client_port` portan nuestro HighID real + puerto TCP cuando estamos
///   conectados y sin cortafuegos; `0`/`0` en caso contrario;
/// - `FT_FILESIZE` es una única etiqueta `TAGTYPE_UINT64` cuando el fichero es grande
///   **y** el par anunció soporte para ficheros grandes (bit 4 de misc2);
/// - `FT_FILERATING` se envía cuando `rating > 0`, escalado `r * 51`;
/// - cadena `FT_FILETYPE` como para los servidores.
///
/// Devuelve `None` cuando el fichero es grande pero el par no admite
/// ficheros grandes (eMule omite la entrada por completo — ruta ASSERT).
pub fn offered_file_for_peer(
    file_hash: [u8; 16],
    file_name: &str,
    file_size: u64,
    rating: u8,
    ctx: &OfferPeerContext,
) -> Option<OfferedFile> {
    const LARGE_FILE: u64 = 0xFFFF_FFFF;
    let is_large = file_size > LARGE_FILE;
    if is_large && !ctx.peer_large_files {
        return None;
    }

    let (client_id, client_port) = if ctx.our_high_id {
        (ctx.our_client_id, ctx.our_tcp_port)
    } else {
        (0, 0)
    };

    let mut tags = vec![Ed2kTag {
        name: TagName::Id(crate::ed2k::tags::TAG_SPECIAL_NAME),
        value: TagValue::String(file_name.to_string()),
    }];
    if is_large {
        tags.push(Ed2kTag {
            name: TagName::Id(TAG_SPECIAL_FILESIZE),
            value: TagValue::UInt64(file_size),
        });
    } else {
        tags.push(Ed2kTag {
            name: TagName::Id(TAG_SPECIAL_FILESIZE),
            value: TagValue::UInt32(file_size as u32),
        });
    }
    if rating > 0 {
        // `SharedFileList.cpp:960` de eMule — las respuestas al cliente reescalan 0..5
        // al formato de eserver 0..255.
        tags.push(Ed2kTag {
            name: TagName::Id(FT_FILERATING),
            value: TagValue::UInt8((rating as u32 * 255 / 5) as u8),
        });
    }
    let ft = crate::ed2k::server::ed2k_file_type_term(file_name);
    if !ft.is_empty() {
        tags.push(Ed2kTag {
            name: TagName::Id(TAG_SPECIAL_FILETYPE),
            value: TagValue::String(ft.to_string()),
        });
    }
    Some(OfferedFile {
        file_hash,
        client_id,
        client_port,
        tags,
    })
}

/// Añade una entrada de fichero ofrecido (`hash + id + puerto + etiquetas`) a `buf`.
fn write_offered_entry(
    encoder: &Ed2kEncoder,
    opcode: u8,
    buf: &mut Vec<u8>,
    f: &OfferedFile,
) -> Result<(), Ed2kEncodeError> {
    buf.extend_from_slice(&f.file_hash);
    buf.extend_from_slice(&f.client_id.to_le_bytes());
    buf.extend_from_slice(&f.client_port.to_le_bytes());
    buf.extend_from_slice(&(f.tags.len() as u32).to_le_bytes());
    for tag in &f.tags {
        encoder.encode_tag(opcode, tag, buf)?;
    }
    Ok(())
}

fn finish_frame(
    opcode: u8,
    buf: Vec<u8>,
    limits: &Ed2kLimits,
) -> Result<RawPacket, Ed2kEncodeError> {
    let payload_len = buf.len() + 1;
    if payload_len > limits.max_frame_bytes {
        return Err(Ed2kEncodeError::new(
            opcode,
            "shared_files_payload",
            crate::ed2k::error::Ed2kErrorKind::FrameTooLarge {
                size: payload_len,
                limit: limits.max_frame_bytes,
            },
        ));
    }
    Ok(RawPacket::with_vec(PR_ED2K, opcode, buf))
}

/// `OP_ASKSHAREDFILESANSWER` (`0x4B`, PR_ED2K):
/// `<recuento u32>` + entradas de ficheros ofrecidos (`ListenSocket.cpp:676`).
/// Una petición denegada sigue recibiendo esta respuesta — con `count == 0`
/// (eMule envía la lista vacía en lugar de `ASKSHAREDDENIEDANS`).
pub fn encode_shared_files_answer(
    files: &[OfferedFile],
    limits: &Ed2kLimits,
) -> Result<RawPacket, Ed2kEncodeError> {
    let encoder = Ed2kEncoder::with_limits(limits.clone());
    let mut buf = Vec::new();
    buf.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for f in files {
        write_offered_entry(&encoder, OP_ASKSHAREDFILESANSWER, &mut buf, f)?;
    }
    finish_frame(OP_ASKSHAREDFILESANSWER, buf, limits)
}

/// `OP_ASKSHAREDDIRSANS` (`0x5F`, PR_ED2K):
/// `<recuento u32>(<longitud u16><directorio utf8>)*` (`SendSharedDirectories`,
/// `BaseClient.cpp:2916`).
pub fn encode_shared_dirs_answer(
    dirs: &[String],
    limits: &Ed2kLimits,
) -> Result<RawPacket, Ed2kEncodeError> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&(dirs.len() as u32).to_le_bytes());
    for d in dirs {
        write_str(&mut buf, d);
    }
    finish_frame(OP_ASKSHAREDDIRSANS, buf, limits)
}

/// `OP_ASKSHAREDFILESDIRANS` (`0x60`, PR_ED2K):
/// `<longitud u16><directorio utf8><recuento u32>` + entradas de ficheros ofrecidos
/// (`ListenSocket.cpp:766-787`). La cadena devuelta es el pseudonombre *solicitado*
/// textual (`strOrgReqDir`), incluso cuando no se resolvió en nada.
pub fn encode_shared_files_dir_answer(
    requested_dir: &str,
    files: &[OfferedFile],
    limits: &Ed2kLimits,
) -> Result<RawPacket, Ed2kEncodeError> {
    let encoder = Ed2kEncoder::with_limits(limits.clone());
    let mut buf = Vec::new();
    write_str(&mut buf, requested_dir);
    buf.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for f in files {
        write_offered_entry(&encoder, OP_ASKSHAREDFILESDIRANS, &mut buf, f)?;
    }
    finish_frame(OP_ASKSHAREDFILESDIRANS, buf, limits)
}

/// `OP_ASKSHAREDDENIEDANS` (`0x61`, PR_ED2K, carga útil vacía) — enviado cuando la
/// preferencia de visualización de compartidos deniega al solicitante (`ListenSocket.cpp:701`).
pub fn encode_shared_denied() -> RawPacket {
    RawPacket::with_vec(PR_ED2K, OP_ASKSHAREDDENIEDANS, Vec::new())
}

/// Decodifica una carga útil `OP_ASKSHAREDFILESDIR` (`0x5E`) `<longitud u16><directorio>`.
/// Devuelve `None` ante truncamiento — eMule tolera basura residual pero el
/// prefijo de longitud debe estar presente y dentro de los límites.
pub fn decode_ask_files_dir(payload: &[u8]) -> Option<String> {
    if payload.len() < 2 {
        return None;
    }
    let len = u16::from_le_bytes([payload[0], payload[1]]) as usize;
    if payload.len() < 2 + len {
        return None;
    }
    String::from_utf8(payload[2..2 + len].to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ed2k::tags::{TAG_SPECIAL_NAME, TAG_SPECIAL_FILESIZE};

    fn entry(hash: u8, id: u32, port: u16, name: &str, size: u64, large64: bool) -> OfferedFile {
        let size_tag = if large64 {
            TagValue::UInt64(size)
        } else {
            TagValue::UInt32(size as u32)
        };
        OfferedFile {
            file_hash: [hash; 16],
            client_id: id,
            client_port: port,
            tags: vec![
                Ed2kTag {
                    name: TagName::Id(TAG_SPECIAL_NAME),
                    value: TagValue::String(name.to_string()),
                },
                Ed2kTag {
                    name: TagName::Id(TAG_SPECIAL_FILESIZE),
                    value: size_tag,
                },
            ],
        }
    }

    #[test]
    fn shared_files_answer_wire_layout() {
        // eMule `ListenSocket.cpp:676` — `<u32 count><entry>*` en PR_ED2K 0x4B.
        let f = entry(0xAA, 0x01020304, 39735, "x.bin", 42, false);
        let pkt = encode_shared_files_answer(&[f], &Ed2kLimits::default()).unwrap();
        assert_eq!(pkt.protocol, PR_ED2K);
        assert_eq!(pkt.opcode, 0x4B);
        assert_eq!(&pkt.payload[..4], &1u32.to_le_bytes());
        assert_eq!(&pkt.payload[4..20], &[0xAA; 16]);
        assert_eq!(&pkt.payload[20..24], &0x01020304u32.to_le_bytes());
        assert_eq!(&pkt.payload[24..26], &39735u16.to_le_bytes());
    }

    #[test]
    fn denied_is_empty_pr_ed2k_0x61() {
        let pkt = encode_shared_denied();
        assert_eq!(pkt.protocol, PR_ED2K);
        assert_eq!(pkt.opcode, 0x61);
        assert!(pkt.payload.is_empty());
    }

    #[test]
    fn dirs_answer_strings() {
        let dirs = vec!["!Incomplete Files".to_string(), "incoming".to_string()];
        let pkt = encode_shared_dirs_answer(&dirs, &Ed2kLimits::default()).unwrap();
        assert_eq!(pkt.opcode, 0x5F);
        assert_eq!(&pkt.payload[..4], &2u32.to_le_bytes());
        // Primera cadena: len 17 + bytes
        let l = u16::from_le_bytes([pkt.payload[4], pkt.payload[5]]) as usize;
        assert_eq!(l, 17);
        assert_eq!(&pkt.payload[6..6 + l], b"!Incomplete Files");
    }

    #[test]
    fn files_dir_answer_echoes_requested_dir() {
        let pkt = encode_shared_files_dir_answer("!Other", &[], &Ed2kLimits::default()).unwrap();
        assert_eq!(pkt.opcode, 0x60);
        let l = u16::from_le_bytes([pkt.payload[0], pkt.payload[1]]) as usize;
        assert_eq!(&pkt.payload[2..2 + l], b"!Other");
        assert_eq!(
            &pkt.payload[2 + l..2 + l + 4],
            &0u32.to_le_bytes(),
            "directorio simulado desconocido -> lista vacía (eMule envía count=0)"
        );
    }

    #[test]
    fn ask_files_dir_decode() {
        assert_eq!(decode_ask_files_dir(&[4, 0, b't', b'e', b's', b't']), Some("test".into()));
        assert_eq!(decode_ask_files_dir(&[9, 0, b's']), None);
        assert_eq!(decode_ask_files_dir(&[]), None);
        // Se toleran bytes sobrantes al final (eMule ASSERT solo en debug).
        assert_eq!(
            decode_ask_files_dir(&[2, 0, b'h', b'i', 0xFF]),
            Some("hi".into())
        );
    }

    #[test]
    fn for_peer_high_id_and_large_file_rules() {
        let high = OfferPeerContext {
            our_client_id: 0x7F000001,
            our_tcp_port: 39735,
            our_high_id: true,
            peer_large_files: true,
        };
        // HighID -> id+puerto reales; archivo grande + soporte del par -> etiqueta u64.
        let f = offered_file_for_peer([1; 16], "big.iso", 5_000_000_000, 4, &high).unwrap();
        assert_eq!(f.client_id, 0x7F000001);
        assert_eq!(f.client_port, 39735);
        assert!(matches!(f.tags[1].value, TagValue::UInt64(5_000_000_000)));
        // valoración 4 -> 4*51 = 204
        assert!(matches!(f.tags[2].value, TagValue::UInt8(204)));

        // LowID -> ceros.
        let low = OfferPeerContext {
            our_high_id: false,
            ..high
        };
        let f2 = offered_file_for_peer([1; 16], "a", 1, 0, &low).unwrap();
        assert_eq!((f2.client_id, f2.client_port), (0, 0));

        // Archivo grande + sin soporte del par -> se omite por completo.
        let no_large = OfferPeerContext {
            our_client_id: 0,
            our_tcp_port: 0,
            our_high_id: false,
            peer_large_files: false,
        };
        assert!(offered_file_for_peer([1; 16], "b", 5_000_000_000, 0, &no_large).is_none());
    }
}

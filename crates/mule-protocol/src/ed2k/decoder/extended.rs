// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Decodificadores para mensajes extendidos de eMule 0.72a: conjuntos de hashes basados en `FileIdentifier`
//! (`OP_HASHSETREQUEST2`/`OP_HASHSETANSWER2`), peticiones y respuestas agrupadas `MULTIPACKET*`,
//! informes de cola y rango en cola, mensajes de chat, descripción de fichero,
//! prueba de puerto y CAPTCHA.
//!
//! Referencias: `srchybrid/opcodes.h:219-289`, `ListenSocket.cpp:834-1101`,
//! `DownloadClient.cpp:259-345`, `UploadClient.cpp` (`SendHashsetPacket`).

use bytes::Bytes;

use super::super::error::{Ed2kErrorKind, Ed2kParseError};
use super::super::file_identifier::Ed2kFileIdentifier;
use super::super::limits::Ed2kLimits;
use super::super::opcodes::{
    OP_AICHFILEHASHREQ, OP_AICHFILEHASHRES, OP_FILESTATUS, OP_MULTIPACKET, OP_MULTIPACKETANSWER,
    OP_MULTIPACKET_EXT, OP_MULTIPACKET_EXT2, OP_MULTIPACKETANSWER_EXT2, OP_REQFILENAME_ANSWER,
    OP_REQFILENAME, OP_REQUESTSOURCES, OP_REQUESTSOURCES2, OP_SETREQFILEID_EMULE,
};
use super::super::packet::{
    expected_mask_bytes, Ed2kAichPartHashSet, Ed2kCaptchaRequest, Ed2kCaptchaResponse,
    Ed2kChangeSlot, Ed2kFileDesc, Ed2kHashSetAnswer, Ed2kHashSetAnswer2, Ed2kHashSetReq2,
    Ed2kChatMessage, Ed2kMultiPacketAnswer, Ed2kMultiPacketFileRef, Ed2kMultiPacketRequest,
    Ed2kMultiPacketSubAnswer, Ed2kMultiPacketSubRequest, Ed2kPortTest, Ed2kQueueRank,
    Ed2kQueueRanking, Ed2kRequestExtInfo,
};
use super::super::tags::TagReader;

/// Límite de respaldo para recuentos de partes eD2K dentro de campos multipaquete cuando
/// el invocador no proporciona `Ed2kLimits::max_parts` (conservado por claridad en errores).
const MAX_SUBPART_COUNT: usize = 10_000;

/// Número máximo de subcódigos de operación en un cuerpo multipaquete (eMule envía como máximo 4).
const MAX_SUBOPS: usize = 32;

fn malformed(
    protocol: u8,
    opcode: u8,
    pos: usize,
    context: &'static str,
    message: String,
) -> Ed2kParseError {
    Ed2kParseError::new(
        protocol,
        Some(opcode),
        pos,
        context,
        Ed2kErrorKind::MalformedField { message },
    )
}

/// Lee la cola de estado de partes `<u16 count><ceil(count/8) bitfield>` usada por
/// la información extendida `OP_REQFILENAME` multipaquete y subrespuestas `OP_FILESTATUS`.
fn read_part_status(
    reader: &mut TagReader<'_>,
    limits: &Ed2kLimits,
    context: &'static str,
) -> Result<(u16, Bytes), Ed2kParseError> {
    let part_count = reader.read_u16_le(context)?;
    if usize::from(part_count) > MAX_SUBPART_COUNT {
        return Err(malformed(
            reader.protocol(),
            reader.opcode().unwrap_or(0),
            reader.pos() - 2,
            context,
            format!("sub-packet part count {part_count} exceeds {MAX_SUBPART_COUNT}"),
        ));
    }
    let mask_len = expected_mask_bytes(part_count);
    if mask_len > limits.max_string_bytes {
        return Err(malformed(
            reader.protocol(),
            reader.opcode().unwrap_or(0),
            reader.pos(),
            context,
            format!("part status bitfield {mask_len} bytes exceeds limit"),
        ));
    }
    let bytes = reader.read_bytes(mask_len, context)?;
    Ok((part_count, Bytes::copy_from_slice(bytes)))
}

/// Analiza la información extendida opcional que sigue a una subpetición `0x58`.
///
/// En cable no hay un marcador explícito: eMule la lee incondicionalmente cuando la
/// `ExtendedRequestsVersion` negociada no es cero
/// (`ProcessExtendedInfo`, `UploadClient.cpp`/`DownloadClient.cpp`):
/// - versión `0`  → no hay ninguna información extendida presente;
/// - versión `1`  → recuento de partes `u16` + mapa de bits de estado de partes `ceil(count/8)`;
/// - versión `>=2` → además un recuento final `u16` de fuentes completas.
///
/// `extended_requests_version` es por tanto la versión negociada del par y
/// hace que el análisis sintáctico sea completamente determinista, sin conjeturas de bytes. Bajo la versión `0`
/// cualquier byte presente es consumido por el invocador como subcódigos de operación subsiguientes.
fn read_optional_ext_info(
    reader: &mut TagReader<'_>,
    extended_requests_version: u8,
) -> Result<Option<Ed2kRequestExtInfo>, Ed2kParseError> {
    if extended_requests_version == 0 {
        return Ok(None);
    }
    let part_count = reader.read_u16_le("multipacket_ext_part_count")?;
    if usize::from(part_count) > MAX_SUBPART_COUNT {
        return Err(malformed(
            reader.protocol(),
            reader.opcode().unwrap_or(0),
            reader.pos() - 2,
            "multipacket_ext_part_count",
            format!("ext-info part count {part_count} exceeds {MAX_SUBPART_COUNT}"),
        ));
    }
    let mask_len = expected_mask_bytes(part_count);
    let part_status = Bytes::copy_from_slice(reader.read_bytes(mask_len, "multipacket_ext_mask")?);

    let complete_sources = if extended_requests_version >= 2 {
        Some(reader.read_u16_le("multipacket_ext_complete_sources")?)
    } else {
        None
    };

    Ok(Some(Ed2kRequestExtInfo {
        part_count,
        part_status,
        complete_sources,
    }))
}

/// Decodifica el flujo de subpeticiones compartido por `OP_MULTIPACKET` (`0x92`),
/// `OP_MULTIPACKET_EXT` (`0xA4`) y `OP_MULTIPACKET_EXT2` (`0xA9`).
fn decode_sub_requests(
    reader: &mut TagReader<'_>,
    _limits: &Ed2kLimits,
    extended_requests_version: u8,
) -> Result<Vec<Ed2kMultiPacketSubRequest>, Ed2kParseError> {
    let mut subs = Vec::new();
    while reader.remaining() > 0 {
        if subs.len() >= MAX_SUBOPS {
            return Err(malformed(
                reader.protocol(),
                reader.opcode().unwrap_or(0),
                reader.pos(),
                "multipacket_subops",
                format!("more than {MAX_SUBOPS} sub-opcodes in multipacket"),
            ));
        }
        let sub = reader.read_u8("multipacket_subopcode")?;
        match sub {
            OP_REQFILENAME => {
                let ext_info = read_optional_ext_info(reader, extended_requests_version)?;
                subs.push(Ed2kMultiPacketSubRequest::FileName { ext_info });
            }
            OP_SETREQFILEID_EMULE => subs.push(Ed2kMultiPacketSubRequest::SetReqFileId),
            OP_REQUESTSOURCES => subs.push(Ed2kMultiPacketSubRequest::RequestSources),
            OP_REQUESTSOURCES2 => {
                let version = reader.read_u8("multipacket_sx2_version")?;
                let options = reader.read_u16_le("multipacket_sx2_options")?;
                subs.push(Ed2kMultiPacketSubRequest::RequestSources2 { version, options });
            }
            OP_AICHFILEHASHREQ => subs.push(Ed2kMultiPacketSubRequest::AichFileHash),
            _ => {
                return Err(malformed(
                    reader.protocol(),
                    reader.opcode().unwrap_or(0),
                    reader.pos() - 1,
                    "multipacket_subopcode",
                    format!("invalid multipacket sub-opcode 0x{sub:02X}"),
                ));
            }
        }
    }
    Ok(subs)
}

/// Decodifica el flujo de subrespuestas compartido por `OP_MULTIPACKETANSWER` (`0x93`) y
/// `OP_MULTIPACKETANSWER_EXT2` (`0xB0`). Las subrespuestas nunca repiten el hash
/// de archivo (`ListenSocket.cpp:1071-1099`).
fn decode_sub_answers(
    reader: &mut TagReader<'_>,
    limits: &Ed2kLimits,
) -> Result<Vec<Ed2kMultiPacketSubAnswer>, Ed2kParseError> {
    let mut subs = Vec::new();
    while reader.remaining() > 0 {
        if subs.len() >= MAX_SUBOPS {
            return Err(malformed(
                reader.protocol(),
                reader.opcode().unwrap_or(0),
                reader.pos(),
                "multipacket_answer_subops",
                format!("more than {MAX_SUBOPS} sub-opcodes in multipacket answer"),
            ));
        }
        let sub = reader.read_u8("multipacket_answer_subopcode")?;
        match sub {
            OP_REQFILENAME_ANSWER => {
                let name = reader.read_string(limits.max_string_bytes, "mp_answer_filename")?;
                subs.push(Ed2kMultiPacketSubAnswer::FileName(name));
            }
            OP_FILESTATUS => {
                let (part_count, part_status) =
                    read_part_status(reader, limits, "mp_answer_filestatus")?;
                subs.push(Ed2kMultiPacketSubAnswer::FileStatus {
                    part_count,
                    part_status,
                });
            }
            OP_AICHFILEHASHRES => {
                let bytes = reader.read_bytes(20, "mp_answer_aich_hash")?;
                let mut h = [0u8; 20];
                h.copy_from_slice(bytes);
                subs.push(Ed2kMultiPacketSubAnswer::AichFileHash(h));
            }
            _ => {
                return Err(malformed(
                    reader.protocol(),
                    reader.opcode().unwrap_or(0),
                    reader.pos() - 1,
                    "multipacket_answer_subopcode",
                    format!("invalid multipacket answer sub-opcode 0x{sub:02X}"),
                ));
            }
        }
    }
    Ok(subs)
}

/// Decodifica una carga útil de petición `MULTIPACKET` (`0x92`/`0xA4`/`0xA9`).
///
/// `extended_requests_version` es la versión negociada del par para
/// `ExtendedRequestsVersion` (`m_byExtendedRequestsVer` de eMule, intercambiada en
/// `OP_EMULEINFO`); determina si la subpetición `0x58` porta la
/// información extendida de estado de partes — véase [`read_optional_ext_info`].
pub fn decode_multipacket_request(
    protocol: u8,
    opcode: u8,
    payload: &[u8],
    base_offset: usize,
    limits: &Ed2kLimits,
    extended_requests_version: u8,
) -> Result<Ed2kMultiPacketRequest, Ed2kParseError> {
    let mut reader = TagReader::new(payload, 0, protocol, Some(opcode));
    let file = match opcode {
        OP_MULTIPACKET => {
            let hash = reader.read_hash("multipacket_hash")?;
            Ed2kMultiPacketFileRef::Hash(hash)
        }
        OP_MULTIPACKET_EXT => {
            let hash = reader.read_hash("multipacket_ext_hash")?;
            let file_size = reader.read_u64_le("multipacket_ext_size")?;
            Ed2kMultiPacketFileRef::HashAndSize { file_hash: hash, file_size }
        }
        OP_MULTIPACKET_EXT2 => {
            Ed2kMultiPacketFileRef::Identifier(Ed2kFileIdentifier::read_from(&mut reader, false)?)
        }
        _ => {
            return Err(malformed(
                protocol,
                opcode,
                base_offset,
                "multipacket_opcode",
                format!("opcode 0x{opcode:02X} is not a multipacket request"),
            ));
        }
    };
    let sub_requests = decode_sub_requests(&mut reader, limits, extended_requests_version)?;
    let _ = base_offset;
    Ok(Ed2kMultiPacketRequest { file, sub_requests })
}

/// Decodifica una carga útil `MULTIPACKETANSWER` (`0x93`/`0xB0`).
pub fn decode_multipacket_answer(
    protocol: u8,
    opcode: u8,
    payload: &[u8],
    base_offset: usize,
    limits: &Ed2kLimits,
) -> Result<Ed2kMultiPacketAnswer, Ed2kParseError> {
    let mut reader = TagReader::new(payload, 0, protocol, Some(opcode));
    let file = match opcode {
        OP_MULTIPACKETANSWER => {
            Ed2kMultiPacketFileRef::Hash(reader.read_hash("mp_answer_hash")?)
        }
        OP_MULTIPACKETANSWER_EXT2 => {
            Ed2kMultiPacketFileRef::Identifier(Ed2kFileIdentifier::read_from(&mut reader, false)?)
        }
        _ => {
            return Err(malformed(
                protocol,
                opcode,
                base_offset,
                "multipacket_answer_opcode",
                format!("opcode 0x{opcode:02X} is not a multipacket answer"),
            ));
        }
    };
    let sub_answers = decode_sub_answers(&mut reader, limits)?;
    let _ = base_offset;
    Ok(Ed2kMultiPacketAnswer { file, sub_answers })
}

/// Bit de opción que solicita el conjunto de hashes MD4 en `OP_HASHSETREQUEST2`
/// (`UploadClient.cpp` 0.72a `SendHashsetPacket`).
pub const HASHSET2_OPT_MD4: u8 = 0x01;
/// Bit de opción que solicita el conjunto de hashes AICH en `OP_HASHSETREQUEST2`.
pub const HASHSET2_OPT_AICH: u8 = 0x02;

/// Decodifica una carga útil `OP_HASHSETREQUEST2` (`0xB1`):
/// `<FileIdentifier><opciones u8>` — debe estar activo al menos un bit conocido.
pub fn decode_hashset_request2(
    protocol: u8,
    opcode: u8,
    payload: &[u8],
    base_offset: usize,
) -> Result<Ed2kHashSetReq2, Ed2kParseError> {
    let mut reader = TagReader::new(payload, 0, protocol, Some(opcode));
    let identifier = Ed2kFileIdentifier::read_from(&mut reader, false)?;
    let opts = reader.read_u8("hashset2_options")?;
    if reader.remaining() != 0 {
        return Err(malformed(
            protocol,
            opcode,
            base_offset + reader.pos(),
            "hashset2_trailing",
            format!("{} trailing bytes after hashset2 request", reader.remaining()),
        ));
    }
    let want_md4 = opts & HASHSET2_OPT_MD4 != 0;
    let want_aich = opts & HASHSET2_OPT_AICH != 0;
    if !want_md4 && !want_aich {
        return Err(malformed(
            protocol,
            opcode,
            base_offset + reader.pos() - 1,
            "hashset2_options",
            "hashset2 request without any known option bit".to_string(),
        ));
    }
    Ok(Ed2kHashSetReq2 {
        identifier,
        want_md4,
        want_aich,
    })
}

/// Decodifica una carga útil `OP_HASHSETANSWER2` (`0xB2`):
/// `<FileIdentifier><opciones u8>[<conjunto md4>][<conjunto aich>]`.
pub fn decode_hashset_answer2(
    protocol: u8,
    opcode: u8,
    payload: &[u8],
    base_offset: usize,
) -> Result<Ed2kHashSetAnswer2, Ed2kParseError> {
    let mut reader = TagReader::new(payload, 0, protocol, Some(opcode));
    let identifier = Ed2kFileIdentifier::read_from(&mut reader, false)?;
    let opts = reader.read_u8("hashset2_answer_options")?;

    let md4_set = if opts & HASHSET2_OPT_MD4 != 0 {
        let file_hash = reader.read_hash("hashset2_md4_filehash")?;
        let part_count = reader.read_u16_le("hashset2_md4_count")? as usize;
        if part_count > MAX_SUBPART_COUNT {
            return Err(malformed(
                protocol,
                opcode,
                base_offset + reader.pos() - 2,
                "hashset2_md4_count",
                format!("md4 hashset count {part_count} exceeds {MAX_SUBPART_COUNT}"),
            ));
        }
        let mut part_hashes = Vec::with_capacity(part_count);
        for _ in 0..part_count {
            part_hashes.push(reader.read_hash("hashset2_md4_part")?);
        }
        Some(Ed2kHashSetAnswer {
            file_hash,
            part_hashes,
        })
    } else {
        None
    };

    let aich_set = if opts & HASHSET2_OPT_AICH != 0 {
        let bytes = reader.read_bytes(20, "hashset2_aich_master")?;
        let mut master_hash = [0u8; 20];
        master_hash.copy_from_slice(bytes);
        let part_count = reader.read_u16_le("hashset2_aich_count")? as usize;
        if part_count > MAX_SUBPART_COUNT {
            return Err(malformed(
                protocol,
                opcode,
                base_offset + reader.pos() - 2,
                "hashset2_aich_count",
                format!("aich hashset count {part_count} exceeds {MAX_SUBPART_COUNT}"),
            ));
        }
        let mut part_hashes = Vec::with_capacity(part_count);
        for _ in 0..part_count {
            let bytes = reader.read_bytes(20, "hashset2_aich_part")?;
            let mut h = [0u8; 20];
            h.copy_from_slice(bytes);
            part_hashes.push(h);
        }
        Some(Ed2kAichPartHashSet {
            master_hash,
            part_hashes,
        })
    } else {
        None
    };

    if reader.remaining() != 0 {
        return Err(malformed(
            protocol,
            opcode,
            base_offset + reader.pos(),
            "hashset2_answer_trailing",
            format!("{} trailing bytes after hashset2 answer", reader.remaining()),
        ));
    }

    Ok(Ed2kHashSetAnswer2 {
        identifier,
        md4_set,
        aich_set,
    })
}

/// Decodifica una carga útil `OP_MESSAGE` (`0x4E`): `<longitud u16><bytes del mensaje>`; la
/// longitud declarada debe consumir la carga útil con exactitud (comportamiento de eMule).
pub fn decode_message(
    protocol: u8,
    opcode: u8,
    payload: &[u8],
    base_offset: usize,
    limits: &Ed2kLimits,
) -> Result<Ed2kChatMessage, Ed2kParseError> {
    let mut reader = TagReader::new(payload, 0, protocol, Some(opcode));
    let text = reader.read_string(limits.max_string_bytes, "message_text")?;
    if reader.remaining() != 0 {
        return Err(malformed(
            protocol,
            opcode,
            base_offset + reader.pos(),
            "message_trailing",
            format!("{} trailing bytes after chat message", reader.remaining()),
        ));
    }
    Ok(Ed2kChatMessage { text })
}

/// Decodifica una carga útil `OP_QUEUERANK` (`0x5C`): `<rango u32>`.
pub fn decode_queue_rank(
    protocol: u8,
    opcode: u8,
    payload: &[u8],
    base_offset: usize,
) -> Result<Ed2kQueueRank, Ed2kParseError> {
    let mut reader = TagReader::new(payload, 0, protocol, Some(opcode));
    let rank = reader.read_u32_le("queue_rank")?;
    if reader.remaining() != 0 {
        return Err(malformed(
            protocol,
            opcode,
            base_offset + reader.pos(),
            "queue_rank_trailing",
            format!("{} trailing bytes after queue rank", reader.remaining()),
        ));
    }
    Ok(Ed2kQueueRank { rank })
}

/// Decodifica una carga útil `OP_QUEUERANKING` (`0x60`): exactamente 12 bytes,
/// `<rango u16><10 reservados>` (`ProcessEmuleQueueRank`, eMule 0.72a).
pub fn decode_queue_ranking(
    protocol: u8,
    opcode: u8,
    payload: &[u8],
    base_offset: usize,
) -> Result<Ed2kQueueRanking, Ed2kParseError> {
    if payload.len() != 12 {
        return Err(malformed(
            protocol,
            opcode,
            base_offset,
            "queue_ranking_len",
            format!(
                "OP_QUEUERANKING payload must be exactly 12 bytes (got {})",
                payload.len()
            ),
        ));
    }
    let mut reader = TagReader::new(payload, 0, protocol, Some(opcode));
    let rank = reader.read_u16_le("queue_ranking")?;
    let mut reserved = [0u8; 10];
    reserved.copy_from_slice(reader.read_bytes(10, "queue_ranking_reserved")?);
    Ok(Ed2kQueueRanking { rank, reserved })
}

/// Decodifica una carga útil `OP_CHANGE_SLOT` (`0x5B`): `<HASH 16>`.
pub fn decode_change_slot(
    protocol: u8,
    opcode: u8,
    payload: &[u8],
    base_offset: usize,
) -> Result<Ed2kChangeSlot, Ed2kParseError> {
    let mut reader = TagReader::new(payload, 0, protocol, Some(opcode));
    let file_hash = reader.read_hash("change_slot_hash")?;
    if reader.remaining() != 0 {
        return Err(malformed(
            protocol,
            opcode,
            base_offset + reader.pos(),
            "change_slot_trailing",
            format!("{} trailing bytes after change slot", reader.remaining()),
        ));
    }
    Ok(Ed2kChangeSlot { file_hash })
}

/// Decodifica una carga útil `OP_FILEDESC` (`0x61`): `<calificación u8><longitud u16><descripción>`.
pub fn decode_file_desc(
    protocol: u8,
    opcode: u8,
    payload: &[u8],
    base_offset: usize,
    limits: &Ed2kLimits,
) -> Result<Ed2kFileDesc, Ed2kParseError> {
    let mut reader = TagReader::new(payload, 0, protocol, Some(opcode));
    let rating = reader.read_u8("filedesc_rating")?;
    let description = reader.read_string(limits.max_string_bytes, "filedesc_text")?;
    if reader.remaining() != 0 {
        return Err(malformed(
            protocol,
            opcode,
            base_offset + reader.pos(),
            "filedesc_trailing",
            format!("{} trailing bytes after file description", reader.remaining()),
        ));
    }
    Ok(Ed2kFileDesc { rating, description })
}

/// Decodifica una carga útil `OP_PORTTEST` (`0xFE`): byte de estado único.
pub fn decode_port_test(
    protocol: u8,
    opcode: u8,
    payload: &[u8],
    base_offset: usize,
) -> Result<Ed2kPortTest, Ed2kParseError> {
    if payload.len() != 1 {
        return Err(malformed(
            protocol,
            opcode,
            base_offset,
            "porttest_len",
            format!("OP_PORTTEST payload must be 1 byte (got {})", payload.len()),
        ));
    }
    Ok(Ed2kPortTest { value: payload[0] })
}

/// Decodifica una carga útil `OP_CHATCAPTCHAREQ` (`0xA5`):
/// `<número_etiquetas u8><etiquetas><bytes del mapa de bits>`. eMule acota el mapa de bits a
/// 128..=4096 bytes; impuesto aquí cuando se aplican semánticas de `enforce_emule_bounds`.
pub fn decode_captcha_request(
    protocol: u8,
    opcode: u8,
    payload: &[u8],
    base_offset: usize,
    limits: &Ed2kLimits,
) -> Result<Ed2kCaptchaRequest, Ed2kParseError> {
    let mut reader = TagReader::new(payload, 0, protocol, Some(opcode));
    let tag_count = reader.read_u8("captcha_tag_count")? as usize;
    if tag_count > limits.max_tags {
        return Err(Ed2kParseError::new(
            protocol,
            Some(opcode),
            base_offset,
            "captcha_tag_count",
            Ed2kErrorKind::TooManyTags {
                count: tag_count,
                limit: limits.max_tags,
            },
        ));
    }
    let mut tags = Vec::new();
    let mut unknown_tags = Vec::new();
    for _ in 0..tag_count {
        let (typed, raw) = reader.read_tag(limits)?;
        if let Some(t) = typed {
            tags.push(t);
        }
        if let Some(r) = raw {
            unknown_tags.push(r);
        }
    }
    let bitmap = Bytes::copy_from_slice(reader.read_bytes(reader.remaining(), "captcha_bitmap")?);
    Ok(Ed2kCaptchaRequest {
        tags,
        unknown_tags,
        bitmap,
    })
}

/// Decodifica una carga útil `OP_CHATCAPTCHARES` (`0xA6`): `<estado u8>`.
pub fn decode_captcha_response(
    protocol: u8,
    opcode: u8,
    payload: &[u8],
    base_offset: usize,
) -> Result<Ed2kCaptchaResponse, Ed2kParseError> {
    if payload.len() != 1 {
        return Err(malformed(
            protocol,
            opcode,
            base_offset,
            "captcha_res_len",
            format!(
                "OP_CHATCAPTCHARES payload must be 1 byte (got {})",
                payload.len()
            ),
        ));
    }
    Ok(Ed2kCaptchaResponse {
        status: payload[0],
    })
}

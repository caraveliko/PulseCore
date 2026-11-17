// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Codificadores de mensajes de saludo (Hello, HelloAnswer, EmuleInfo).

use bytes::Bytes;

use super::super::error::{Ed2kEncodeError, Ed2kErrorKind};
use super::super::limits::Ed2kLimits;
use super::super::opcodes::{OP_EMULE_INFO, OP_EMULE_INFO_ANSWER, OP_HELLO, OP_HELLO_ANSWER};
use super::super::packet::{Ed2kEmuleInfo, Ed2kHello, Ed2kHelloAnswer};
use super::super::profile::Ed2kCompatibilityProfile;
use super::super::protocol::{PR_ED2K, PR_EMULE};
use super::tags::{encode_raw_tag, encode_tag};
use crate::framing::RawPacket;

/// Codifica un mensaje `Ed2kHello` en un `RawPacket`.
pub fn encode_hello(
    limits: &Ed2kLimits,
    profile: Ed2kCompatibilityProfile,
    hello: &Ed2kHello,
) -> Result<RawPacket, Ed2kEncodeError> {
    let opcode = OP_HELLO;
    let total_tags = hello.tags.len().saturating_add(hello.unknown_tags.len());
    if total_tags > limits.max_tags {
        return Err(Ed2kEncodeError::new(
            opcode,
            "hello_tag_count",
            Ed2kErrorKind::TooManyTags {
                count: total_tags,
                limit: limits.max_tags,
            },
        ));
    }

    let mut payload = Vec::with_capacity(64);
    if profile != Ed2kCompatibilityProfile::LegacyEdonkey {
        payload.push(0x10); // Prefijo de longitud de hash de usuario estándar en OP_HELLO de eD2K (16 bytes)
    }
    payload.extend_from_slice(&hello.user_hash);
    payload.extend_from_slice(&hello.client_id.to_le_bytes());
    payload.extend_from_slice(&hello.tcp_port.to_le_bytes());
    payload.extend_from_slice(&(total_tags as u32).to_le_bytes());

    for tag in &hello.tags {
        encode_tag(limits, opcode, tag, &mut payload)?;
    }
    for raw in &hello.unknown_tags {
        encode_raw_tag(limits, opcode, raw, &mut payload)?;
    }

    // El saludo estándar de eD2K siempre incluye IP del servidor (4 bytes LE) y puerto (2 bytes LE)
    payload.extend_from_slice(&hello.server_ip.to_le_bytes());
    payload.extend_from_slice(&hello.server_port.to_le_bytes());

    let frame_len = 1 + payload.len();
    let max_frame = limits.max_frame_for_opcode(opcode);
    if frame_len > max_frame {
        return Err(Ed2kEncodeError::new(
            opcode,
            "frame_len",
            Ed2kErrorKind::FrameTooLarge {
                size: frame_len,
                limit: max_frame,
            },
        ));
    }

    Ok(RawPacket::new(PR_ED2K, opcode, Bytes::from(payload)))
}

/// Codifica un mensaje `Ed2kHelloAnswer` en un `RawPacket`.
pub fn encode_hello_answer(
    limits: &Ed2kLimits,
    answer: &Ed2kHelloAnswer,
) -> Result<RawPacket, Ed2kEncodeError> {
    let opcode = OP_HELLO_ANSWER;
    let total_tags = answer.tags.len().saturating_add(answer.unknown_tags.len());
    if total_tags > limits.max_tags {
        return Err(Ed2kEncodeError::new(
            opcode,
            "hello_answer_tag_count",
            Ed2kErrorKind::TooManyTags {
                count: total_tags,
                limit: limits.max_tags,
            },
        ));
    }

    let mut payload = Vec::with_capacity(64);
    payload.extend_from_slice(&answer.user_hash);
    payload.extend_from_slice(&answer.client_id.to_le_bytes());
    payload.extend_from_slice(&answer.tcp_port.to_le_bytes());
    payload.extend_from_slice(&(total_tags as u32).to_le_bytes());

    for tag in &answer.tags {
        encode_tag(limits, opcode, tag, &mut payload)?;
    }
    for raw in &answer.unknown_tags {
        encode_raw_tag(limits, opcode, raw, &mut payload)?;
    }

    payload.extend_from_slice(&answer.server_ip.to_le_bytes());
    payload.extend_from_slice(&answer.server_port.to_le_bytes());

    let frame_len = 1 + payload.len();
    let max_frame = limits.max_frame_for_opcode(opcode);
    if frame_len > max_frame {
        return Err(Ed2kEncodeError::new(
            opcode,
            "frame_len",
            Ed2kErrorKind::FrameTooLarge {
                size: frame_len,
                limit: max_frame,
            },
        ));
    }

    Ok(RawPacket::new(PR_ED2K, opcode, Bytes::from(payload)))
}

/// Codifica una carga útil `OP_EMULE_INFO` (`0xC5 0x01`).
pub fn encode_emule_info(
    limits: &Ed2kLimits,
    info: &Ed2kEmuleInfo,
) -> Result<RawPacket, Ed2kEncodeError> {
    encode_emule_info_with_opcode(limits, info, OP_EMULE_INFO)
}

/// Codifica una carga útil `OP_EMULE_INFO_ANSWER` (`0xC5 0x02`).
///
/// `CUpDownClient::SendMuleInfoPacket(true)` de eMule envía la carga útil de etiquetas
/// idéntica a `OP_EMULE_INFO` bajo el código de operación de respuesta.
pub fn encode_emule_info_answer(
    limits: &Ed2kLimits,
    info: &Ed2kEmuleInfo,
) -> Result<RawPacket, Ed2kEncodeError> {
    encode_emule_info_with_opcode(limits, info, OP_EMULE_INFO_ANSWER)
}

fn encode_emule_info_with_opcode(
    limits: &Ed2kLimits,
    info: &Ed2kEmuleInfo,
    opcode: u8,
) -> Result<RawPacket, Ed2kEncodeError> {
    let total_tags = info.tags.len() + info.unknown_tags.len();
    if total_tags > limits.max_tags {
        return Err(Ed2kEncodeError::new(
            opcode,
            "emule_info_tag_count",
            Ed2kErrorKind::TooManyTags {
                count: total_tags,
                limit: limits.max_tags,
            },
        ));
    }

    let mut payload = Vec::with_capacity(64);
    payload.push(info.client_version);
    payload.push(info.protocol_version);
    payload.extend_from_slice(&(total_tags as u32).to_le_bytes());

    for tag in &info.tags {
        encode_tag(limits, opcode, tag, &mut payload)?;
    }
    for raw in &info.unknown_tags {
        encode_raw_tag(limits, opcode, raw, &mut payload)?;
    }

    let frame_len = 1 + payload.len();
    let max_frame = limits.max_frame_for_opcode(opcode);
    if frame_len > max_frame {
        return Err(Ed2kEncodeError::new(
            opcode,
            "emule_info_frame_len",
            Ed2kErrorKind::FrameTooLarge {
                size: frame_len,
                limit: max_frame,
            },
        ));
    }

    Ok(RawPacket::new(PR_EMULE, opcode, Bytes::from(payload)))
}

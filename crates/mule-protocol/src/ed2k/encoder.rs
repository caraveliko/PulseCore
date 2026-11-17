// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Codificadores de red eD2K seguros y deterministas con límites estrictos de recursos.

use bytes::Bytes;

use super::error::{Ed2kEncodeError, Ed2kErrorKind};
use super::limits::Ed2kLimits;
use super::packet::{
    Ed2kAichAnswer, Ed2kAichFileHashReq, Ed2kAichFileHashRes, Ed2kAichRequest, Ed2kEmuleInfo,
    Ed2kFileReqAnsNoFile, Ed2kFileStatus, Ed2kFileStatusReq, Ed2kHashSetAnswer,
    Ed2kHashSetAnswer2, Ed2kHashSetReq, Ed2kHashSetReq2,
    Ed2kHello, Ed2kHelloAnswer, Ed2kMessage, Ed2kPublicKey, Ed2kRequestParts, Ed2kRequestPartsI64,
    Ed2kSecIdentState, Ed2kSendingPart, Ed2kSendingPartI64, Ed2kSignature,
};
use super::profile::Ed2kCompatibilityProfile;
use super::tags::{Ed2kTag, RawTag, TagName};
use crate::framing::RawPacket;
use crate::limits::HEADER_SIZE;

/// Módulos de codificación de mensajes AICH (AichFileHashReq/Res, AichRequest/Answer).
pub mod aich;
/// Codificadores extendidos de eMule 0.72a (HashSet2, MultiPacket*, cola, chat, CAPTCHA…).
pub mod extended;
/// Codificadores de mensajes de saludo (Hello, HelloAnswer, EmuleInfo).
pub mod handshake;
/// Codificadores de mensajes de seguridad (PublicKey, Signature, SecIdentState).
pub mod security;
/// Codificadores de etiquetas y nombres de etiquetas eD2K.
pub mod tags;
/// Codificadores de mensajes de transferencia (FileStatus, RequestParts, SendingPart, FileReqAnsNoFile).
pub mod transfers;

pub use aich::{
    encode_aich_answer, encode_aich_file_hash_req, encode_aich_file_hash_res, encode_aich_request,
};
pub use extended::{
    encode_captcha_request, encode_captcha_response, encode_change_slot, encode_file_desc,
    encode_hashset_answer2, encode_hashset_request2, encode_message, encode_multipacket_answer,
    encode_multipacket_request, encode_port_test, encode_queue_rank, encode_queue_ranking,
};
pub use handshake::{
    encode_emule_info, encode_emule_info_answer, encode_hello, encode_hello_answer,
};
pub use security::{encode_public_key, encode_secident_state, encode_signature};
pub use tags::{encode_raw_tag, encode_tag, encode_tag_name};
pub use transfers::{
    encode_compressed_part, encode_compressed_part_i64, encode_file_req_ans_no_file,
    encode_file_status,
    encode_file_status_req, encode_file_status_req_with_opcode, encode_hashset_answer,
    encode_hashset_request, encode_req_filename, encode_request_parts, encode_request_parts_i64,
    encode_request_parts_with_opcode, encode_sending_part, encode_sending_part_i64,
};

/// Codificador determinista de red para mensajes históricos del protocolo eD2K.
#[derive(Debug, Clone, Default)]
pub struct Ed2kEncoder {
    /// Límites de seguridad y fronteras de recursos.
    pub limits: Ed2kLimits,
    /// Perfil de compatibilidad activo para la selección de códigos de operación de red.
    pub profile: Ed2kCompatibilityProfile,
}

impl Ed2kEncoder {
    /// Crea un nuevo codificador con límites de producción y perfil predeterminados (`Emule072a`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Crea un nuevo codificador con un perfil de compatibilidad explícito.
    pub fn with_profile(profile: Ed2kCompatibilityProfile) -> Self {
        Self {
            limits: Ed2kLimits::default(),
            profile,
        }
    }

    /// Crea un nuevo codificador con límites personalizados y perfil predeterminado.
    pub fn with_limits(limits: Ed2kLimits) -> Self {
        Self {
            limits,
            profile: Ed2kCompatibilityProfile::default(),
        }
    }

    /// Crea un nuevo codificador con límites personalizados y un perfil de compatibilidad explícito.
    pub fn with_limits_and_profile(limits: Ed2kLimits, profile: Ed2kCompatibilityProfile) -> Self {
        Self { limits, profile }
    }

    /// Codifica un `TagName` en el búfer de destino.
    pub fn encode_tag_name(
        &self,
        opcode: u8,
        name: &TagName,
        dst: &mut Vec<u8>,
    ) -> Result<(), Ed2kEncodeError> {
        encode_tag_name(&self.limits, opcode, name, dst)
    }

    /// Codifica una única `Ed2kTag` reconocida en el búfer de destino.
    pub fn encode_tag(
        &self,
        opcode: u8,
        tag: &Ed2kTag,
        dst: &mut Vec<u8>,
    ) -> Result<(), Ed2kEncodeError> {
        encode_tag(&self.limits, opcode, tag, dst)
    }

    /// Codifica una etiqueta desconocida sin procesar conservada en el búfer de destino.
    pub fn encode_raw_tag(
        &self,
        opcode: u8,
        tag: &RawTag,
        dst: &mut Vec<u8>,
    ) -> Result<(), Ed2kEncodeError> {
        encode_raw_tag(&self.limits, opcode, tag, dst)
    }

    /// Codifica un mensaje `Ed2kHello` en un `RawPacket`.
    pub fn encode_hello(&self, hello: &Ed2kHello) -> Result<RawPacket, Ed2kEncodeError> {
        encode_hello(&self.limits, self.profile, hello)
    }

    /// Codifica un mensaje `Ed2kHelloAnswer` en un `RawPacket`.
    pub fn encode_hello_answer(
        &self,
        answer: &Ed2kHelloAnswer,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_hello_answer(&self.limits, answer)
    }

    /// Codifica un mensaje `Ed2kFileStatusReq` usando el código de operación del perfil activo del codificador.
    pub fn encode_file_status_req(
        &self,
        req: &Ed2kFileStatusReq,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_file_status_req(&self.limits, self.profile, req)
    }

    /// Codifica un mensaje `Ed2kFileStatusReq` en un `RawPacket` con un código de operación especificado.
    pub fn encode_file_status_req_with_opcode(
        &self,
        req: &Ed2kFileStatusReq,
        opcode: u8,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_file_status_req_with_opcode(&self.limits, req, opcode)
    }

    /// Codifica un mensaje `Ed2kFileStatus` en un `RawPacket`.
    pub fn encode_file_status(
        &self,
        status: &Ed2kFileStatus,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_file_status(&self.limits, status)
    }

    /// Codifica un mensaje `Ed2kFileReqAnsNoFile` (`0x48`) en un `RawPacket`.
    pub fn encode_file_req_ans_no_file(
        &self,
        ans: &Ed2kFileReqAnsNoFile,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_file_req_ans_no_file(&self.limits, ans)
    }

    /// Codifica un mensaje `Ed2kRequestParts` usando el código de operación del perfil activo del codificador.
    pub fn encode_request_parts(
        &self,
        req: &Ed2kRequestParts,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_request_parts(&self.limits, self.profile, req)
    }

    /// Codifica un mensaje `Ed2kRequestParts` con un código de operación especificado.
    pub fn encode_request_parts_with_opcode(
        &self,
        req: &Ed2kRequestParts,
        opcode: u8,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_request_parts_with_opcode(&self.limits, req, opcode)
    }

    /// Codifica un mensaje `Ed2kRequestPartsI64` (`OP_REQUESTPARTS_I64`, `0xA3`).
    pub fn encode_request_parts_i64(
        &self,
        req: &Ed2kRequestPartsI64,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_request_parts_i64(&self.limits, req)
    }

    /// Codifica un mensaje `Ed2kSendingPart` (`0x46`).
    pub fn encode_sending_part(
        &self,
        part: &Ed2kSendingPart,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_sending_part(&self.limits, part)
    }

    /// Codifica un mensaje `Ed2kSendingPartI64` (`OP_SENDINGPART_I64`, `0xA2`).
    pub fn encode_sending_part_i64(
        &self,
        part: &Ed2kSendingPartI64,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_sending_part_i64(&self.limits, part)
    }

    /// Codifica un mensaje `Ed2kHashSetReq` (`0x51`).
    pub fn encode_hashset_request(
        &self,
        req: &Ed2kHashSetReq,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_hashset_request(&self.limits, req)
    }

    /// Codifica un mensaje `Ed2kHashSetAnswer` (`0x52`).
    pub fn encode_hashset_answer(
        &self,
        ans: &Ed2kHashSetAnswer,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_hashset_answer(&self.limits, ans)
    }

    /// Codifica un mensaje `Ed2kHashSetReq2` (`0xB1`, protocolo eMule):
    /// `<FileIdentifier><opciones u8>`.
    pub fn encode_hashset_request2(
        &self,
        req: &Ed2kHashSetReq2,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_hashset_request2(&self.limits, req)
    }

    /// Codifica un mensaje `Ed2kHashSetAnswer2` (`0xB2`, protocolo eMule):
    /// `<FileIdentifier><opciones u8>[<conjunto md4>][<conjunto aich>]`.
    pub fn encode_hashset_answer2(
        &self,
        ans: &Ed2kHashSetAnswer2,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_hashset_answer2(&self.limits, ans)
    }

    /// Codifica un mensaje `OP_AICHFILEHASHREQ` (eMule `0x14`).
    pub fn encode_aich_file_hash_req(
        &self,
        req: &Ed2kAichFileHashReq,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_aich_file_hash_req(&self.limits, req)
    }

    /// Codifica un mensaje `OP_AICHFILEHASHANS` (eMule `0x15`).
    pub fn encode_aich_file_hash_res(
        &self,
        res: &Ed2kAichFileHashRes,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_aich_file_hash_res(&self.limits, res)
    }

    /// Codifica un mensaje `OP_AICHREQUEST` (eMule `0x12`).
    pub fn encode_aich_request(&self, req: &Ed2kAichRequest) -> Result<RawPacket, Ed2kEncodeError> {
        encode_aich_request(&self.limits, req)
    }

    /// Codifica un mensaje `OP_AICHANSWER` (eMule `0x13`).
    pub fn encode_aich_answer(&self, ans: &Ed2kAichAnswer) -> Result<RawPacket, Ed2kEncodeError> {
        encode_aich_answer(&self.limits, ans)
    }

    /// Codifica una carga útil `OP_EMULE_INFO` (`0xC5 0x01`).
    pub fn encode_emule_info(&self, info: &Ed2kEmuleInfo) -> Result<RawPacket, Ed2kEncodeError> {
        encode_emule_info(&self.limits, info)
    }

    /// Codifica una carga útil `OP_EMULE_INFO_ANSWER` (`0xC5 0x02`).
    pub fn encode_emule_info_answer(
        &self,
        info: &Ed2kEmuleInfo,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_emule_info_answer(&self.limits, info)
    }

    /// Codifica un mensaje `OP_PUBLICKEY` (`0x85`) en un `RawPacket`.
    pub fn encode_public_key(&self, key: &Ed2kPublicKey) -> Result<RawPacket, Ed2kEncodeError> {
        encode_public_key(&self.limits, key)
    }

    /// Codifica un mensaje `OP_SIGNATURE` (`0x86`) en un `RawPacket`.
    pub fn encode_signature(&self, sig: &Ed2kSignature) -> Result<RawPacket, Ed2kEncodeError> {
        encode_signature(&self.limits, sig)
    }

    /// Codifica un mensaje `OP_SECIDENTSTATE` (`0x87`) en un `RawPacket`.
    pub fn encode_secident_state(
        &self,
        state: &Ed2kSecIdentState,
    ) -> Result<RawPacket, Ed2kEncodeError> {
        encode_secident_state(&self.limits, state)
    }

    /// Codifica cualquier `Ed2kMessage` en su representación `RawPacket`.
    pub fn encode_message(&self, msg: &Ed2kMessage) -> Result<RawPacket, Ed2kEncodeError> {
        match msg {
            Ed2kMessage::Hello(hello) => self.encode_hello(hello),
            Ed2kMessage::HelloAnswer(answer) => self.encode_hello_answer(answer),
            Ed2kMessage::EmuleInfo(info) => self.encode_emule_info(info),
            Ed2kMessage::FileStatusReq(req) => self.encode_file_status_req(req),
            Ed2kMessage::FileStatus(status) => self.encode_file_status(status),
            Ed2kMessage::FileReqAnsNoFile(ans) => self.encode_file_req_ans_no_file(ans),
            Ed2kMessage::RequestParts(req) => self.encode_request_parts(req),
            Ed2kMessage::RequestPartsI64(req) => encode_request_parts_i64(&self.limits, req),
            Ed2kMessage::SendingPart(part) => self.encode_sending_part(part),
            Ed2kMessage::SendingPartI64(part) => encode_sending_part_i64(&self.limits, part),
            Ed2kMessage::CompressedPartI64(part) => encode_compressed_part_i64(&self.limits, part),
            Ed2kMessage::CompressedPart(part) => encode_compressed_part(&self.limits, part),
            Ed2kMessage::HashSetReq(req) => self.encode_hashset_request(req),
            Ed2kMessage::HashSetAnswer(ans) => self.encode_hashset_answer(ans),
            Ed2kMessage::AichFileHashReq(req) => self.encode_aich_file_hash_req(req),
            Ed2kMessage::AichFileHashRes(res) => self.encode_aich_file_hash_res(res),
            Ed2kMessage::AichRequest(req) => self.encode_aich_request(req),
            Ed2kMessage::AichAnswer(ans) => self.encode_aich_answer(ans),
            Ed2kMessage::PublicKey(key) => self.encode_public_key(key),
            Ed2kMessage::Signature(sig) => self.encode_signature(sig),
            Ed2kMessage::SecIdentState(state) => self.encode_secident_state(state),
            Ed2kMessage::Message(msg) => encode_message(&self.limits, msg),
            Ed2kMessage::QueueRank(qr) => encode_queue_rank(&self.limits, qr),
            Ed2kMessage::QueueRanking(qr) => encode_queue_ranking(&self.limits, qr),
            Ed2kMessage::ChangeSlot(cs) => encode_change_slot(&self.limits, cs),
            Ed2kMessage::FileDesc(fd) => encode_file_desc(&self.limits, fd),
            Ed2kMessage::PortTest(pt) => encode_port_test(&self.limits, pt),
            Ed2kMessage::CaptchaRequest(req) => encode_captcha_request(&self.limits, req),
            Ed2kMessage::CaptchaResponse(res) => encode_captcha_response(&self.limits, res),
            Ed2kMessage::HashSetReq2(req) => encode_hashset_request2(&self.limits, req),
            Ed2kMessage::HashSetAnswer2(ans) => encode_hashset_answer2(&self.limits, ans),
            Ed2kMessage::MultiPacketRequest(req) => encode_multipacket_request(&self.limits, req),
            Ed2kMessage::MultiPacketAnswer(ans) => encode_multipacket_answer(&self.limits, ans),
            Ed2kMessage::Raw(raw) => {
                let frame_len = raw.frame_len();
                let max_frame = self.limits.max_frame_for_opcode(raw.opcode);
                if frame_len > max_frame {
                    return Err(Ed2kEncodeError::new(
                        raw.opcode,
                        "raw_frame_len",
                        Ed2kErrorKind::FrameTooLarge {
                            size: frame_len,
                            limit: max_frame,
                        },
                    ));
                }
                Ok(raw.clone())
            }
        }
    }

    /// Codifica cualquier `Ed2kMessage` directamente en un búfer de bytes listo para cable (cabecera de 5 bytes + trama).
    pub fn encode_to_bytes(&self, msg: &Ed2kMessage) -> Result<Bytes, Ed2kEncodeError> {
        let packet = self.encode_message(msg)?;
        let frame_len = packet.frame_len();
        let total = HEADER_SIZE + frame_len;
        let mut buf = Vec::with_capacity(total);
        buf.push(packet.protocol);
        buf.extend_from_slice(&(frame_len as u32).to_le_bytes());
        buf.push(packet.opcode);
        buf.extend_from_slice(&packet.payload);
        Ok(Bytes::from(buf))
    }

    /// Codifica una petición de Source Exchange (`OP_REQUESTSOURCES` u `OP_REQUESTSOURCES2`).
    pub fn encode_request_sources(
        &self,
        req: &super::source_exchange::Ed2kRequestSources,
    ) -> Result<crate::framing::RawPacket, Ed2kEncodeError> {
        super::source_exchange::encode_request_sources(req)
    }

    /// Codifica una respuesta de Source Exchange. `is_sx2` selecciona la envoltura
    /// `OP_ANSWERSOURCES2` (`0x84`) o la heredada `OP_ANSWERSOURCES` (`0x82`),
    /// coincidiendo con el código de operación que utilizó el solicitante (`bIsSX2Packet`
    /// de eMule, `PartFile.cpp:3705`).
    pub fn encode_answer_sources(
        &self,
        ans: &super::source_exchange::Ed2kAnswerSources,
        is_sx2: bool,
        force_compress: bool,
    ) -> Result<crate::framing::RawPacket, Ed2kEncodeError> {
        super::source_exchange::encode_answer_sources(ans, is_sx2, force_compress)
    }
}

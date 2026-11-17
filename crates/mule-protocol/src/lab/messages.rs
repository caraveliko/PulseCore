// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Mensajes fuertemente tipados para el protocolo determinista de laboratorio (`PR_LAB = 0xEE`).

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::error::ProtocolError;
use crate::framing::RawPacket;
use crate::limits::MAX_STRING_BYTES;
use crate::types::{PeerId, PR_LAB};

/// Código de operación para el saludo del protocolo de enlace Hello del cliente.
pub const LAB_OP_HELLO: u8 = 0x01;
/// Código de operación para la respuesta del protocolo de enlace HelloAck del servidor.
pub const LAB_OP_HELLO_ACK: u8 = 0x02;
/// Código de operación que anuncia la oferta de archivo disponible.
pub const LAB_OP_FILE_OFFER: u8 = 0x03;
/// Código de operación que solicita un bloque de datos de archivo.
pub const LAB_OP_REQUEST_BLOCK: u8 = 0x04;
/// Código de operación que transporta la carga útil de datos del bloque.
pub const LAB_OP_BLOCK_DATA: u8 = 0x05;
/// Código de operación que señaliza el rechazo de una petición de bloque.
pub const LAB_OP_BLOCK_REJECTED: u8 = 0x06;
/// Código de operación para el cierre ordenado de la conexión.
pub const LAB_OP_GOODBYE: u8 = 0x07;
/// Código de operación que informa de un error de diagnóstico del protocolo.
pub const LAB_OP_PROTOCOL_ERROR: u8 = 0x08;

/// Códigos de motivo para el rechazo de transferencia de bloques.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LabRejectReason {
    /// El archivo no está alojado ni compartido por la fuente.
    FileNotFound = 0x01,
    /// El rango de bloque solicitado está fuera de límites o no es válido.
    InvalidRange = 0x02,
    /// La fuente está ocupada o la capacidad de subida se ha agotado.
    Busy = 0x03,
    /// La sesión no se encuentra en un estado válido para transferencias de bloques.
    NotReady = 0x04,
    /// Otro fallo no especificado.
    Unspecified = 0xFF,
}

impl LabRejectReason {
    /// Convierte un byte sin procesar en un motivo de rechazo.
    pub fn from_u8(val: u8) -> Self {
        match val {
            0x01 => Self::FileNotFound,
            0x02 => Self::InvalidRange,
            0x03 => Self::Busy,
            0x04 => Self::NotReady,
            _ => Self::Unspecified,
        }
    }
}

/// Mensajes de alto nivel intercambiados durante las pruebas de bucle local de laboratorio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabMessage {
    /// Saludo del iniciador del protocolo de enlace.
    Hello {
        /// Versión compatible del protocolo de laboratorio (p. ej., 1).
        client_version: u32,
        /// Nonce de desafío efímero de 16 bytes generado por el iniciador.
        nonce: [u8; 16],
        /// Identificador efímero del par.
        peer_id: PeerId,
    },

    /// Acuse de recibo del receptor del protocolo de enlace.
    HelloAck {
        /// Versión seleccionada del protocolo de laboratorio.
        server_version: u32,
        /// Nonce que refleja el nonce del iniciador para confirmar el emparejamiento de la conexión.
        nonce_ack: [u8; 16],
        /// Identificador efímero de par del receptor.
        peer_id: PeerId,
    },

    /// Anuncio de un archivo disponible ofrecido por un par.
    FileOffer {
        /// Hash de archivo eD2K de 16 bytes.
        file_hash: [u8; 16],
        /// Tamaño total del archivo en bytes.
        file_size: u64,
        /// Nombre para mostrar del archivo.
        name: String,
    },

    /// Petición de un bloque de datos específico.
    RequestBlock {
        /// Hash de archivo eD2K de 16 bytes.
        file_hash: [u8; 16],
        /// Desplazamiento absoluto en bytes en el archivo.
        start_offset: u64,
        /// Longitud en bytes solicitada (hasta 184.320 bytes).
        length: u32,
    },

    /// Entrega de la carga útil de los datos del bloque solicitado.
    BlockData {
        /// Hash de archivo eD2K de 16 bytes.
        file_hash: [u8; 16],
        /// Desplazamiento absoluto en bytes en el archivo.
        start_offset: u64,
        /// Bytes de datos de bloque sin procesar.
        data: Bytes,
    },

    /// Rechazo de un bloque solicitado.
    BlockRejected {
        /// Hash de archivo eD2K de 16 bytes.
        file_hash: [u8; 16],
        /// Desplazamiento absoluto en bytes solicitado.
        start_offset: u64,
        /// Motivo del rechazo.
        reason: LabRejectReason,
    },

    /// Notificación de cierre limpio.
    Goodbye {
        /// Motivo descriptivo de la terminación.
        reason: String,
    },

    /// Mensaje de error de diagnóstico del protocolo.
    ProtocolError {
        /// Código de estado del error.
        code: u16,
        /// Mensaje de texto de diagnóstico.
        message: String,
    },
}

impl LabMessage {
    /// Serializa el mensaje de alto nivel en un [`RawPacket`] entramado.
    pub fn to_raw_packet(&self) -> Result<RawPacket, ProtocolError> {
        let mut buf = BytesMut::new();
        let opcode = match self {
            Self::Hello {
                client_version,
                nonce,
                peer_id,
            } => {
                buf.put_u32_le(*client_version);
                buf.put_slice(nonce);
                buf.put_slice(peer_id.as_bytes());
                LAB_OP_HELLO
            }
            Self::HelloAck {
                server_version,
                nonce_ack,
                peer_id,
            } => {
                buf.put_u32_le(*server_version);
                buf.put_slice(nonce_ack);
                buf.put_slice(peer_id.as_bytes());
                LAB_OP_HELLO_ACK
            }
            Self::FileOffer {
                file_hash,
                file_size,
                name,
            } => {
                let name_bytes = name.as_bytes();
                if name_bytes.len() > MAX_STRING_BYTES {
                    return Err(ProtocolError::OversizedString {
                        len: name_bytes.len(),
                        limit: MAX_STRING_BYTES,
                    });
                }
                buf.put_slice(file_hash);
                buf.put_u64_le(*file_size);
                buf.put_u16_le(name_bytes.len() as u16);
                buf.put_slice(name_bytes);
                LAB_OP_FILE_OFFER
            }
            Self::RequestBlock {
                file_hash,
                start_offset,
                length,
            } => {
                buf.put_slice(file_hash);
                buf.put_u64_le(*start_offset);
                buf.put_u32_le(*length);
                LAB_OP_REQUEST_BLOCK
            }
            Self::BlockData {
                file_hash,
                start_offset,
                data,
            } => {
                buf.put_slice(file_hash);
                buf.put_u64_le(*start_offset);
                buf.put_slice(data);
                LAB_OP_BLOCK_DATA
            }
            Self::BlockRejected {
                file_hash,
                start_offset,
                reason,
            } => {
                buf.put_slice(file_hash);
                buf.put_u64_le(*start_offset);
                buf.put_u8(*reason as u8);
                LAB_OP_BLOCK_REJECTED
            }
            Self::Goodbye { reason } => {
                let r_bytes = reason.as_bytes();
                if r_bytes.len() > MAX_STRING_BYTES {
                    return Err(ProtocolError::OversizedString {
                        len: r_bytes.len(),
                        limit: MAX_STRING_BYTES,
                    });
                }
                buf.put_u16_le(r_bytes.len() as u16);
                buf.put_slice(r_bytes);
                LAB_OP_GOODBYE
            }
            Self::ProtocolError { code, message } => {
                let m_bytes = message.as_bytes();
                if m_bytes.len() > MAX_STRING_BYTES {
                    return Err(ProtocolError::OversizedString {
                        len: m_bytes.len(),
                        limit: MAX_STRING_BYTES,
                    });
                }
                buf.put_u16_le(*code);
                buf.put_u16_le(m_bytes.len() as u16);
                buf.put_slice(m_bytes);
                LAB_OP_PROTOCOL_ERROR
            }
        };

        Ok(RawPacket::new(PR_LAB, opcode, buf.freeze()))
    }

    /// Deserializa un [`RawPacket`] en un [`LabMessage`].
    pub fn from_raw_packet(packet: &RawPacket) -> Result<Self, ProtocolError> {
        if packet.protocol == crate::types::PR_EMULE_COMPRESSED {
            return Err(ProtocolError::CompressedProtocolUnsupported);
        }
        if packet.protocol != PR_LAB {
            return Err(ProtocolError::Framing(
                crate::error::FramingError::UnsupportedProtocol {
                    protocol: packet.protocol,
                },
            ));
        }

        let mut payload = packet.payload.clone();

        match packet.opcode {
            LAB_OP_HELLO => {
                if payload.len() < 4 + 16 + 16 {
                    return Err(ProtocolError::MalformedPayload(
                        "Hello payload too short".into(),
                    ));
                }
                let client_version = payload.get_u32_le();
                let mut nonce = [0u8; 16];
                payload.copy_to_slice(&mut nonce);
                let mut peer_bytes = [0u8; 16];
                payload.copy_to_slice(&mut peer_bytes);
                Ok(Self::Hello {
                    client_version,
                    nonce,
                    peer_id: PeerId::new(peer_bytes),
                })
            }
            LAB_OP_HELLO_ACK => {
                if payload.len() < 4 + 16 + 16 {
                    return Err(ProtocolError::MalformedPayload(
                        "HelloAck payload too short".into(),
                    ));
                }
                let server_version = payload.get_u32_le();
                let mut nonce_ack = [0u8; 16];
                payload.copy_to_slice(&mut nonce_ack);
                let mut peer_bytes = [0u8; 16];
                payload.copy_to_slice(&mut peer_bytes);
                Ok(Self::HelloAck {
                    server_version,
                    nonce_ack,
                    peer_id: PeerId::new(peer_bytes),
                })
            }
            LAB_OP_FILE_OFFER => {
                if payload.len() < 16 + 8 + 2 {
                    return Err(ProtocolError::MalformedPayload(
                        "FileOffer payload too short".into(),
                    ));
                }
                let mut file_hash = [0u8; 16];
                payload.copy_to_slice(&mut file_hash);
                let file_size = payload.get_u64_le();
                let name_len = payload.get_u16_le() as usize;
                if name_len > MAX_STRING_BYTES {
                    return Err(ProtocolError::OversizedString {
                        len: name_len,
                        limit: MAX_STRING_BYTES,
                    });
                }
                if payload.len() < name_len {
                    return Err(ProtocolError::MalformedPayload(
                        "FileOffer truncated string".into(),
                    ));
                }
                let name_bytes = payload.split_to(name_len);
                let name = String::from_utf8(name_bytes.to_vec())
                    .map_err(|e| ProtocolError::MalformedPayload(format!("Invalid UTF-8: {e}")))?;
                Ok(Self::FileOffer {
                    file_hash,
                    file_size,
                    name,
                })
            }
            LAB_OP_REQUEST_BLOCK => {
                if payload.len() < 16 + 8 + 4 {
                    return Err(ProtocolError::MalformedPayload(
                        "RequestBlock payload too short".into(),
                    ));
                }
                let mut file_hash = [0u8; 16];
                payload.copy_to_slice(&mut file_hash);
                let start_offset = payload.get_u64_le();
                let length = payload.get_u32_le();
                Ok(Self::RequestBlock {
                    file_hash,
                    start_offset,
                    length,
                })
            }
            LAB_OP_BLOCK_DATA => {
                if payload.len() < 16 + 8 {
                    return Err(ProtocolError::MalformedPayload(
                        "BlockData payload too short".into(),
                    ));
                }
                let mut file_hash = [0u8; 16];
                payload.copy_to_slice(&mut file_hash);
                let start_offset = payload.get_u64_le();
                let data = payload;
                Ok(Self::BlockData {
                    file_hash,
                    start_offset,
                    data,
                })
            }
            LAB_OP_BLOCK_REJECTED => {
                if payload.len() < 16 + 8 + 1 {
                    return Err(ProtocolError::MalformedPayload(
                        "BlockRejected payload too short".into(),
                    ));
                }
                let mut file_hash = [0u8; 16];
                payload.copy_to_slice(&mut file_hash);
                let start_offset = payload.get_u64_le();
                let reason = LabRejectReason::from_u8(payload.get_u8());
                Ok(Self::BlockRejected {
                    file_hash,
                    start_offset,
                    reason,
                })
            }
            LAB_OP_GOODBYE => {
                if payload.len() < 2 {
                    return Err(ProtocolError::MalformedPayload(
                        "Goodbye payload too short".into(),
                    ));
                }
                let r_len = payload.get_u16_le() as usize;
                if payload.len() < r_len {
                    return Err(ProtocolError::MalformedPayload(
                        "Goodbye truncated reason".into(),
                    ));
                }
                let r_bytes = payload.split_to(r_len);
                let reason = String::from_utf8(r_bytes.to_vec())
                    .map_err(|e| ProtocolError::MalformedPayload(format!("Invalid UTF-8: {e}")))?;
                Ok(Self::Goodbye { reason })
            }
            LAB_OP_PROTOCOL_ERROR => {
                if payload.len() < 4 {
                    return Err(ProtocolError::MalformedPayload(
                        "ProtocolError payload too short".into(),
                    ));
                }
                let code = payload.get_u16_le();
                let m_len = payload.get_u16_le() as usize;
                if payload.len() < m_len {
                    return Err(ProtocolError::MalformedPayload(
                        "ProtocolError truncated message".into(),
                    ));
                }
                let m_bytes = payload.split_to(m_len);
                let message = String::from_utf8(m_bytes.to_vec())
                    .map_err(|e| ProtocolError::MalformedPayload(format!("Invalid UTF-8: {e}")))?;
                Ok(Self::ProtocolError { code, message })
            }
            unknown => Err(ProtocolError::UnexpectedOpcode {
                expected: 0,
                got: unknown,
            }),
        }
    }
}

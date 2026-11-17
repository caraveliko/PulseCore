// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Codificador y decodificador en flujo sin copias (zero-copy) para paquetes de red eD2K.

use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::packet::RawPacket;
use crate::error::FramingError;
use crate::limits::{HEADER_SIZE, MAX_BUFFERED_BYTES, MAX_FRAME_BYTES};
use crate::types::is_valid_protocol;

/// Códec de flujo para el entramado eD2K.
#[derive(Debug, Clone)]
pub struct PacketCodec {
    /// Tamaño máximo permitido de trama de paquete (código de operación + carga útil).
    pub max_frame_bytes: usize,
    /// Máximo de bytes permitidos en el búfer acumulador antes de la desconexión por desbordamiento.
    pub max_buffered_bytes: usize,
    /// Si es true, rechaza bytes mágicos de protocolo desconocidos.
    pub strict_protocol: bool,
}

impl Default for PacketCodec {
    fn default() -> Self {
        Self {
            max_frame_bytes: MAX_FRAME_BYTES,
            max_buffered_bytes: MAX_BUFFERED_BYTES,
            strict_protocol: true,
        }
    }
}

impl PacketCodec {
    /// Crea un códec con los límites predeterminados de producción.
    pub fn new() -> Self {
        Self::default()
    }

    /// Crea un códec con límites de trama y búfer personalizados.
    pub fn with_limits(max_frame_bytes: usize, max_buffered_bytes: usize) -> Self {
        Self {
            max_frame_bytes,
            max_buffered_bytes,
            strict_protocol: true,
        }
    }

    /// Configura si se aplica una validación estricta del byte de protocolo.
    pub fn strict_protocol(mut self, strict: bool) -> Self {
        self.strict_protocol = strict;
        self
    }

    /// Intenta decodificar un único `RawPacket` del búfer acumulador `src`.
    ///
    /// - Devuelve `Ok(Some(packet))` cuando se decodifica y consume una trama completa de `src`.
    /// - Devuelve `Ok(None)` cuando `src` aún no contiene una trama completa (se necesitan más bytes).
    /// - Devuelve `Err(FramingError)` si se producen límites de protocolo, tamaños no válidos o desbordamiento de búfer.
    pub fn decode(&self, src: &mut BytesMut) -> Result<Option<RawPacket>, FramingError> {
        // Aplica la capacidad máxima de búfer para evitar la acumulación ilimitada de memoria.
        if src.len() > self.max_buffered_bytes {
            return Err(FramingError::BufferOverflow {
                buffered: src.len(),
                limit: self.max_buffered_bytes,
            });
        }

        // Se necesitan al menos 5 bytes de cabecera (1 byte de protocolo + 4 bytes de longitud LE).
        if src.len() < HEADER_SIZE {
            return Ok(None);
        }

        let protocol = src[0];
        if self.strict_protocol && !is_valid_protocol(protocol) {
            return Err(FramingError::UnsupportedProtocol { protocol });
        }

        // Lee la longitud de trama en little-endian de 4 bytes sin consumirla todavía.
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&src[1..5]);
        let frame_len = u32::from_le_bytes(len_bytes) as usize;

        // En eD2K, frame_len debe ser de al menos 1 byte (para el código de operación).
        if frame_len < 1 {
            return Err(FramingError::FrameTooSmall { size: frame_len });
        }

        if frame_len > self.max_frame_bytes {
            return Err(FramingError::FrameTooLarge {
                size: frame_len,
                limit: self.max_frame_bytes,
            });
        }

        let total_required = HEADER_SIZE + frame_len;
        if src.len() < total_required {
            // Paquete parcial: esperar a los bytes restantes.
            return Ok(None);
        }

        // Avanzar más allá de la cabecera (5 bytes).
        src.advance(HEADER_SIZE);

        // El primer byte de la trama es el código de operación.
        let opcode = src.split_to(1)[0];

        // Los bytes restantes de la trama corresponden a la carga útil.
        let payload_len = frame_len - 1;
        let payload = src.split_to(payload_len).freeze();

        Ok(Some(RawPacket {
            protocol,
            opcode,
            payload,
        }))
    }

    /// Codifica un `RawPacket` en `dst`.
    pub fn encode(&self, packet: &RawPacket, dst: &mut BytesMut) -> Result<(), FramingError> {
        let frame_len = packet.frame_len();
        if frame_len > self.max_frame_bytes {
            return Err(FramingError::FrameTooLarge {
                size: frame_len,
                limit: self.max_frame_bytes,
            });
        }

        dst.reserve(HEADER_SIZE + frame_len);
        dst.put_u8(packet.protocol);
        dst.put_u32_le(frame_len as u32);
        dst.put_u8(packet.opcode);
        dst.put_slice(&packet.payload);

        Ok(())
    }

    /// Codifica un `RawPacket` directamente en un nuevo búfer inmutable `Bytes`.
    pub fn encode_to_bytes(&self, packet: &RawPacket) -> Result<Bytes, FramingError> {
        let total = packet.total_wire_len();
        let mut buf = BytesMut::with_capacity(total);
        self.encode(packet, &mut buf)?;
        Ok(buf.freeze())
    }
}

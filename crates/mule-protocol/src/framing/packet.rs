// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Representación de paquete sin procesar antes del análisis semántico.

use bytes::Bytes;

use crate::error::FramingError;
use crate::limits::HEADER_SIZE;

/// Un paquete de cable eD2K sin procesar que contiene el byte de protocolo, el byte de código de operación y la carga útil sin analizar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPacket {
    /// Byte mágico del protocolo (p. ej., 0xE3, 0xC5, 0xD4).
    pub protocol: u8,
    /// Byte de código de operación.
    pub opcode: u8,
    /// Datos de carga útil sin analizar.
    pub payload: Bytes,
}

impl RawPacket {
    /// Crea un nuevo `RawPacket`.
    pub fn new(protocol: u8, opcode: u8, payload: Bytes) -> Self {
        Self {
            protocol,
            opcode,
            payload,
        }
    }

    /// Crea un nuevo `RawPacket` a partir de un vector de bytes.
    pub fn with_vec(protocol: u8, opcode: u8, vec: Vec<u8>) -> Self {
        Self {
            protocol,
            opcode,
            payload: Bytes::from(vec),
        }
    }

    /// Crea un paquete con una carga útil vacía.
    pub fn empty(protocol: u8, opcode: u8) -> Self {
        Self {
            protocol,
            opcode,
            payload: Bytes::new(),
        }
    }

    /// Devuelve la longitud de la carga útil en bytes.
    #[inline]
    pub fn payload_len(&self) -> usize {
        self.payload.len()
    }

    /// Devuelve la longitud de la trama eD2K (1 byte para el código de operación + longitud de carga útil).
    #[inline]
    pub fn frame_len(&self) -> usize {
        1 + self.payload.len()
    }

    /// Devuelve el tamaño total en cable (5 bytes de cabecera + byte de código de operación + longitud de carga útil).
    #[inline]
    pub fn total_wire_len(&self) -> usize {
        HEADER_SIZE + self.frame_len()
    }

    /// Valida que el tamaño del paquete no viole los límites de trama.
    pub fn validate_size(&self, max_frame_bytes: usize) -> Result<(), FramingError> {
        let flen = self.frame_len();
        if flen > max_frame_bytes {
            Err(FramingError::FrameTooLarge {
                size: flen,
                limit: max_frame_bytes,
            })
        } else {
            Ok(())
        }
    }
}

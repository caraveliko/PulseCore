// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Entramado y estructuras de paquetes para saludos (handshakes) de ofuscación del protocolo de eMule.

use rand::RngCore;

use crate::ed2k::{PR_ED2K, PR_EMULE, PR_EMULE_COMPRESSED};

/// Valor mágico de sincronización para verificar que el cifrado funciona (`0x835E6FC4`).
pub const MAGICVALUE_SYNC: u32 = 0x835E6FC4;

/// Identificador del método de cifrado para la ofuscación de protocolo básica con RC4 (`0x00`).
pub const ENM_OBFUSCATION: u8 = 0x00;

/// Identificador del método moderno `PC26` con X25519 + ChaCha20-Poly1305 (`0x10`).
pub const ENM_PC26_X25519_CHACHA: u8 = 0x10;

/// Identificador del método moderno `PC26` híbrido post-cuántico X25519 + ML-KEM-768 + ChaCha20-Poly1305 (`0x11`).
pub const ENM_PC26_HYBRID_MLKEM768_CHACHA: u8 = 0x11;

/// Prefijo mágico de 4 bytes para el saludo oportunista `PC26` (`"PC26"`).
/// Su primer byte (`0x50`) no colisiona con `0xE3`, `0xC5` ni `0xD4`.
pub const PC26_OPPORTUNISTIC_MAGIC: [u8; 4] = *b"PC26";

/// Devuelve `true` si `byte` es un marcador de protocolo eD2K estándar sin cifrar.
#[inline]
pub fn is_standard_protocol_marker(byte: u8) -> bool {
    byte == PR_ED2K || byte == PR_EMULE || byte == PR_EMULE_COMPRESSED
}

/// Genera un byte semialeatorio garantizando que NO colisione con los marcadores
/// de protocolo estándar ni con el primer byte del prefijo oportunista `PC26`.
pub fn generate_semi_random_marker<R: RngCore>(rng: &mut R) -> u8 {
    loop {
        let b = (rng.next_u32() & 0xFF) as u8;
        if !is_standard_protocol_marker(b) && b != PC26_OPPORTUNISTIC_MAGIC[0] {
            return b;
        }
    }
}

/// Paquete del iniciador enviado por el Cliente A al Cliente B (cabecera sin cifrar + carga útil cifrada).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientInitiatorPacket {
    /// Byte de marcador sin cifrar (distinto de `0xE3`, `0xC5`, `0xD4`).
    pub marker: u8,
    /// Parte de clave aleatoria sin cifrar (4 bytes).
    pub random_key_part: u32,
    /// Valor mágico de sincronización cifrado (`0x835E6FC4`).
    pub magic_sync: u32,
    /// Métodos de cifrado admitidos cifrados (0x00).
    pub supported_methods: u8,
    /// Método de cifrado preferido cifrado (0x00).
    pub preferred_method: u8,
    /// Bytes de relleno aleatorios cifrados.
    pub padding: Vec<u8>,
}

impl ClientInitiatorPacket {
    /// Construye un nuevo paquete iniciador de cliente con relleno aleatorio de hasta `max_padding` bytes.
    pub fn new<R: RngCore>(rng: &mut R, max_padding: u8) -> Self {
        let marker = generate_semi_random_marker(rng);
        let random_key_part = rng.next_u32();
        let pad_len = if max_padding > 0 {
            (rng.next_u32() % (max_padding as u32 + 1)) as u8
        } else {
            0
        };
        let mut padding = vec![0u8; pad_len as usize];
        rng.fill_bytes(&mut padding);

        Self {
            marker,
            random_key_part,
            magic_sync: MAGICVALUE_SYNC,
            supported_methods: ENM_OBFUSCATION,
            preferred_method: ENM_OBFUSCATION,
            padding,
        }
    }

    /// Serializa la cabecera sin cifrar (`marker` + `random_key_part`, 5 bytes).
    pub fn serialize_unencrypted_header(&self) -> [u8; 5] {
        let mut out = [0u8; 5];
        out[0] = self.marker;
        out[1..5].copy_from_slice(&self.random_key_part.to_le_bytes());
        out
    }

    /// Serializa la carga útil sin cifrar antes del cifrado RC4:
    /// `magic_sync` (4B) + `supported` (1B) + `preferred` (1B) + `pad_len` (1B) + `padding` (NB).
    pub fn serialize_payload_plain(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(7 + self.padding.len());
        out.extend_from_slice(&self.magic_sync.to_le_bytes());
        out.push(self.supported_methods);
        out.push(self.preferred_method);
        out.push(self.padding.len() as u8);
        out.extend_from_slice(&self.padding);
        out
    }
}

/// Paquete de respuesta enviado por el Cliente B al Cliente A (completamente cifrado).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientResponderPacket {
    /// Valor mágico de sincronización cifrado (`0x835E6FC4`).
    pub magic_sync: u32,
    /// Método de cifrado seleccionado cifrado (`0x00`).
    pub selected_method: u8,
    /// Bytes de relleno aleatorios cifrados.
    pub padding: Vec<u8>,
}

impl ClientResponderPacket {
    /// Construye un nuevo paquete respondedor de cliente con relleno aleatorio de hasta `max_padding` bytes.
    pub fn new<R: RngCore>(rng: &mut R, max_padding: u8) -> Self {
        let pad_len = if max_padding > 0 {
            (rng.next_u32() % (max_padding as u32 + 1)) as u8
        } else {
            0
        };
        let mut padding = vec![0u8; pad_len as usize];
        rng.fill_bytes(&mut padding);

        Self {
            magic_sync: MAGICVALUE_SYNC,
            selected_method: ENM_OBFUSCATION,
            padding,
        }
    }

    /// Serializa el paquete respondedor antes del cifrado RC4:
    /// `magic_sync` (4B) + `selected_method` (1B) + `pad_len` (1B) + `padding` (NB).
    pub fn serialize_payload_plain(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(6 + self.padding.len());
        out.extend_from_slice(&self.magic_sync.to_le_bytes());
        out.push(self.selected_method);
        out.push(self.padding.len() as u8);
        out.extend_from_slice(&self.padding);
        out
    }
}

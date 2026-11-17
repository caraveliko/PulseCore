// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Implementación de ofuscación del protocolo eD2K conforme a las especificaciones oficiales de eMule.
//!
//! Proporciona:
//! - [`Rc4Cipher`]: cifrado de flujo RC4 de alto rendimiento con soporte para `drop_bytes(1024)`.
//! - [`dh`]: acuerdo de claves Diffie-Hellman MODP de 768 bits para conexiones a servidores.
//! - [`keys`]: derivación de claves MD5 para conexiones entre pares y con servidores.
//! - [`framing`]: codificadores/decodificadores de paquetes de red para saludos (handshakes) de ofuscación.

pub mod dh;
pub mod framing;
pub mod keys;
pub mod rc4;

pub use dh::{Dh768KeyPair, DH768_G, DH768_P};
pub use framing::{
    generate_semi_random_marker, is_standard_protocol_marker, ClientInitiatorPacket,
    ClientResponderPacket, ENM_OBFUSCATION, MAGICVALUE_SYNC,
};
pub use keys::{
    derive_initiator_peer_ciphers, derive_initiator_peer_raw_keys, derive_responder_peer_ciphers,
    derive_responder_peer_raw_keys, derive_server_ciphers, MAGICVALUE_REQUESTER, MAGICVALUE_SERVER,
};
pub use rc4::Rc4Cipher;

use thiserror::Error;

/// Errores que surgen durante los saludos (handshakes) de ofuscación del protocolo.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ObfuscationError {
    /// El extremo remoto envió un valor mágico de sincronización no válido (se esperaba `0x835E6FC4`).
    #[error("Invalid magic sync value: expected 0x835E6FC4, got 0x{0:08X}")]
    InvalidMagicSync(u32),

    /// El extremo remoto seleccionó o solicitó un método de cifrado no compatible.
    #[error("Unsupported encryption method: {0}")]
    UnsupportedMethod(u8),

    /// Iniciar una conexión ofuscada con un par requiere conocer el UserHash del par.
    #[error("Missing target client user hash for obfuscated connection")]
    MissingUserHash,

    /// El socket se cerró antes de que se recibieran todos los bytes esperados del saludo.
    #[error("Premature EOF during obfuscation handshake (expected {expected} bytes, got {got})")]
    UnexpectedEof {
        /// Número de bytes esperados.
        expected: usize,
        /// Número de bytes realmente recibidos antes de EOF.
        got: usize,
    },

    /// Ocurrió un error de E/S de nivel inferior durante la transmisión del saludo.
    #[error("I/O error during obfuscation handshake: {0}")]
    Io(String),

    /// Se agotó el tiempo de espera del saludo antes de completarse.
    #[error("Handshake timed out")]
    Timeout,
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Estructuras de datos y definiciones de constantes para eMule Source Exchange (SX/SX2).

/// Versión predeterminada de Source Exchange 2 implementada por clientes modernos de eMule.
pub const SOURCE_EXCHANGE2_VERSION: u8 = 4;

/// Versión máxima anunciada por eMule a través de la etiqueta hello SX1 (`ET_SOURCEEXCHANGE`)
/// — eMule 0.72a envía `CTag(ET_SOURCEEXCHANGE, 3)`
/// (`BaseClient.cpp:715`). Las entradas SX1 v3+ llevan el hash de usuario y utilizan
/// el formato IP "híbrido" en little-endian.
pub const SOURCE_EXCHANGE1_MAX_VERSION: u8 = 3;

/// Número máximo de fuentes permitidas en un único paquete de respuesta SX para prevenir DoS.
pub const MAX_SX_SOURCES_PER_PACKET: usize = 1000;

/// Petición de fuentes para un hash de archivo determinado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kRequestSources {
    /// Hash MD4 de 16 bytes del archivo solicitado.
    pub file_hash: [u8; 16],
    /// Versión SX solicitada (p. ej., 4 para SX2 o 1 para la heredada).
    pub version: u8,
    /// Opciones SX solicitadas (reservado, normalmente 0).
    pub options: u16,
}

impl Ed2kRequestSources {
    /// Crea una petición de fuentes SX2 estándar para un hash de archivo.
    pub fn new_v2(file_hash: [u8; 16]) -> Self {
        Self {
            file_hash,
            version: SOURCE_EXCHANGE2_VERSION,
            options: 0,
        }
    }

    /// Crea una petición de fuentes SX1 heredada para un hash de archivo.
    pub fn new_v1(file_hash: [u8; 16]) -> Self {
        Self {
            file_hash,
            version: 1,
            options: 0,
        }
    }
}

/// Representación de una única fuente de par transmitida a través de Source Exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SxSource {
    /// Dirección IPv4 en 4 octetos sin procesar [A, B, C, D] en orden de red estándar.
    pub ip_bytes: [u8; 4],
    /// Representación entera little-endian de la dirección IPv4 que coincide con el estándar eD2K.
    pub ip: u32,
    /// Puerto de escucha TCP del cliente.
    pub port: u16,
    /// Dirección IPv4 del servidor conectado como u32.
    pub server_ip: u32,
    /// Puerto del servidor conectado.
    pub server_port: u16,
    /// Hash MD4 del usuario remoto, si está disponible (SX v2+).
    pub user_hash: Option<[u8; 16]>,
    /// Opciones de ofuscación y cifrado de protocolo (SX v4):
    /// - bit 0: admite la capa de cifrado de protocolo
    /// - bit 1: solicita la capa de cifrado de protocolo
    /// - bit 2: requiere la capa de cifrado de protocolo
    pub crypt_options: u8,
}

impl SxSource {
    /// Crea una entrada de fuente con octetos IPv4, puerto, hash de usuario y opciones criptográficas.
    pub fn new(
        ip_bytes: [u8; 4],
        port: u16,
        user_hash: Option<[u8; 16]>,
        crypt_options: u8,
    ) -> Self {
        let ip = u32::from_le_bytes(ip_bytes);
        Self {
            ip_bytes,
            ip,
            port,
            server_ip: 0,
            server_port: 0,
            user_hash,
            crypt_options,
        }
    }

    /// Devuelve la cadena con la dirección IPv4 (p. ej., "192.168.1.1").
    pub fn ip_string(&self) -> String {
        format!(
            "{}.{}.{}.{}",
            self.ip_bytes[0], self.ip_bytes[1], self.ip_bytes[2], self.ip_bytes[3]
        )
    }

    /// Devuelve la cadena de dirección del socket (p. ej., "192.168.1.1:49662").
    pub fn address_string(&self) -> String {
        format!("{}:{}", self.ip_string(), self.port)
    }

    /// Devuelve verdadero si esta fuente es un LowID (< 16_777_216).
    pub fn is_low_id(&self) -> bool {
        self.ip < 16_777_216
    }
}

/// Respuesta que contiene fuentes para un hash de archivo determinado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kAnswerSources {
    /// Versión SX del paquete (p. ej., 4 para SX2 v4, o 1..3).
    pub version: u8,
    /// Hash MD4 de 16 bytes del archivo.
    pub file_hash: [u8; 16],
    /// Fuentes de pares descubiertas o compartidas.
    pub sources: Vec<SxSource>,
}

impl Ed2kAnswerSources {
    /// Crea una nueva carga útil de respuesta SX2.
    pub fn new(file_hash: [u8; 16], sources: Vec<SxSource>) -> Self {
        Self {
            version: SOURCE_EXCHANGE2_VERSION,
            file_hash,
            sources,
        }
    }

    /// Crea una carga útil de respuesta con una versión SX explícita, coincidiendo con eMule
    /// `byUsedVersion = min(byRequestedVersion, SOURCEEXCHANGE2_VERSION)` para
    /// SX2 o `GetSourceExchange1Version()` para SX1 (`PartFile.cpp:3631-3646`).
    pub fn new_v(version: u8, file_hash: [u8; 16], sources: Vec<SxSource>) -> Self {
        Self {
            version,
            file_hash,
            sources,
        }
    }
}

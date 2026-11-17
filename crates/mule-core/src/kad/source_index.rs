// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use crate::kad::KadId;
use std::net::Ipv4Addr;

/// Representa una fuente indexada para un archivo objetivo en la red Kad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadIndexedSource {
    /// Hash de 16 bytes del archivo objetivo (hash eD2K).
    pub file_hash: [u8; 16],
    /// Identidad Kad de 16 bytes del par que publica.
    pub source_kad_id: KadId,
    /// Dirección IPv4 del par que ofrece el archivo.
    pub ip: Ipv4Addr,
    /// Puerto UDP del par.
    pub udp_port: u16,
    /// Puerto TCP de cliente del par para transferencia de datos eD2K.
    pub tcp_port: u16,
    /// Tamaño total del archivo notificado por el par.
    pub file_size: u64,
    /// Indicadores de cifrado y características (p. ej. soporte de ofuscación).
    pub encryption: u8,
    /// Marca de tiempo de expiración en milisegundos (TTL).
    pub expires_at_unix_ms: u64,
    /// Marca de tiempo en la que este registro fue actualizado por última vez.
    pub updated_at_unix_ms: u64,
}

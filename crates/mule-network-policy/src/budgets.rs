// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde::{Deserialize, Serialize};

/// Límites de recursos y tiempos de espera que imponen operaciones de red acotadas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBudgets {
    /// Conexiones TCP con pares simultáneas máximas (por defecto: 200).
    pub max_active_peer_connections: usize,
    /// Conexiones TCP con servidores simultáneas máximas (por defecto: 2).
    pub max_active_server_connections: usize,
    /// Sockets de escucha simultáneos máximos (por defecto: 4 — TCP entrante,
    /// UDP de servidor y UDP de Kad consumen cada uno una ranura).
    pub max_active_listeners: usize,
    /// Tiempo de espera en milisegundos para iniciar una conexión TCP (por defecto: 10.000 ms).
    pub connect_timeout_ms: u64,
    /// Tiempo de espera en milisegundos para el intercambio de protocolo / saludo (por defecto: 15.000 ms).
    pub handshake_timeout_ms: u64,
    /// Tamaño máximo de carga útil de una sola trama / mensaje en bytes (por defecto: 10 MiB).
    pub max_frame_size_bytes: usize,
    /// Memoria de búfer agregada máxima asignada en todos los sockets de red activos (por defecto: 32 MiB).
    pub max_total_buffer_bytes: usize,
    /// Límite de tasa en intentos de conexión por minuto móvil (por defecto: 240 —
    /// eMule 0.72a `MaxConnectionsPerFiveSeconds` = 20 ≈ 240/min).
    pub rate_limit_per_min: u32,
}

impl Default for ResourceBudgets {
    fn default() -> Self {
        Self {
            max_active_peer_connections: 500,
            max_active_server_connections: 2,
            max_active_listeners: 4,
            connect_timeout_ms: 10_000,
            handshake_timeout_ms: 15_000,
            max_frame_size_bytes: 10 * 1024 * 1024,   // 10 MiB
            max_total_buffer_bytes: 32 * 1024 * 1024, // 32 MiB
            rate_limit_per_min: 240,
        }
    }
}

/// Límite de cordura para conexiones simultáneas de pares — eMule no impone
/// ningún límite fijo (`GetMaxWindowsTCPConnections` = `UNLIMITED` en
/// Windows NT+), mantenemos una salvaguarda contra errores tipográficos.
/// En la práctica el límite real sigue siendo `max_total_buffer_bytes`.
pub const MAX_PEER_CONNECTIONS_CAP: usize = 65_000;

impl ResourceBudgets {
    /// Valida que los presupuestos configurados se mantengan dentro de límites superiores e inferiores seguros.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.max_active_peer_connections == 0 {
            return Err("max_active_peer_connections must be at least 1");
        }
        if self.max_active_peer_connections > MAX_PEER_CONNECTIONS_CAP {
            return Err("max_active_peer_connections cannot exceed 65000");
        }
        if self.max_active_server_connections == 0 {
            return Err("max_active_server_connections must be at least 1");
        }
        if self.max_active_server_connections > 10 {
            return Err("max_active_server_connections cannot exceed 10");
        }
        if self.connect_timeout_ms < 500 || self.connect_timeout_ms > 120_000 {
            return Err("connect_timeout_ms must be between 500 ms and 120,000 ms");
        }
        if self.handshake_timeout_ms < 500 || self.handshake_timeout_ms > 120_000 {
            return Err("handshake_timeout_ms must be between 500 ms and 120,000 ms");
        }
        if self.max_frame_size_bytes < 1024 || self.max_frame_size_bytes > 64 * 1024 * 1024 {
            return Err("max_frame_size_bytes must be between 1 KiB and 64 MiB");
        }
        if self.max_total_buffer_bytes < self.max_frame_size_bytes {
            return Err("max_total_buffer_bytes must be at least max_frame_size_bytes");
        }
        if self.rate_limit_per_min == 0 || self.rate_limit_per_min > 1200 {
            return Err("rate_limit_per_min must be between 1 and 1200");
        }
        Ok(())
    }
}

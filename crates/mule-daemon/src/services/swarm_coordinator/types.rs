// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Constantes y estructura de estado interno del coordinador de enjambre.

use crate::state::AppState;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Intervalo entre ciclos de evaluación del coordinador.
pub const COORDINATOR_TICK_INTERVAL: Duration = Duration::from_secs(3);

/// `SERVERREASKTIME` de eMule (Opcodes.h): retraso mínimo entre dos peticiones
/// TCP de fuentes (OP_GETSOURCES) para un mismo archivo.
pub const SERVER_REASK_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// `UDPSERVERREASKTIME` de eMule (Opcodes.h): retraso mínimo entre dos
/// peticiones UDP de fuentes (OP_GLOBGETSOURCES) para un mismo archivo.
/// eMule acompasa este canal mediante un contador global (`m_lastudpsearchtime`)
/// iterando sobre todos los archivos; la cadencia por archivo es equivalente.
pub const UDP_SERVER_REASK_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// `KADEMLIAREASKTIME` de eMule (Opcodes.h): base del retroceso de búsqueda de
/// fuentes Kad. El plazo real es `now + KADEMLIA_REASK_INTERVAL *
/// total_searches` (`m_TotalSearchesKad`, limitado a `KAD_MAX_TOTAL_SEARCHES`).
pub const KADEMLIA_REASK_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// `KADEMLIAASKTIME` de eMule (Opcodes.h): espaciado mínimo entre dos
/// inicios de búsqueda de fuentes Kad, considerando todos los archivos
/// (`CDownloadQueue::DoKademliaFileRequest`).
pub const KADEMLIA_ASK_INTERVAL: Duration = Duration::from_secs(1);

/// `KADEMLIATOTALFILE` de eMule (Opcodes.h): número máximo de búsquedas de
/// fuentes Kad simultáneas (`CKademlia::GetTotalFile()`).
pub const KAD_MAX_FILE_SEARCHES: usize = 5;

/// Límite máximo de `m_TotalSearchesKad` de eMule (PartFile.cpp): el retroceso de Kad
/// deja de crecer tras 7 búsquedas (≈ 7 h entre repeticiones de consulta).
pub const KAD_MAX_TOTAL_SEARCHES: u32 = 7;

/// `MAX_SOURCES_FILE_SOFT` de eMule (Opcodes.h): límite absoluto del umbral de
/// repetición de consulta TCP (`CPartFile::GetMaxSourcePerFileSoft`).
pub const MAX_SOURCES_FILE_SOFT: usize = 750;

/// `MAX_SOURCES_FILE_UDP` de eMule (Opcodes.h): límite absoluto del umbral de
/// repetición de consulta UDP/Kad (`CPartFile::GetMaxSourcePerFileUDP`).
pub const MAX_SOURCES_FILE_UDP: usize = 50;

/// Número máximo de nuevas conexiones de pares a iniciar por archivo por ciclo
/// (anti-ráfaga por archivo). Ritmo suavizado para preservar los rúteres domésticos.
pub const MAX_CONNECT_PER_TICK: usize = 12;

/// Número máximo de nuevas conexiones totales de pares a iniciar entre todos los archivos activos por ciclo
/// (anti-ráfaga global: máx 24 conexiones / 3 s ≈ 8 conn/s).
pub const MAX_GLOBAL_CONNECT_PER_TICK: usize = 24;

/// Intervalo entre latidos periódicos del sistema (60 segundos).
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

/// Retraso mínimo entre dos intentos automáticos de (re)conexión al servidor.
pub const SERVER_RECONNECT_INTERVAL: Duration = Duration::from_secs(60);

/// Retroceso por dirección antes de reintentar un servidor cuya conexión falló.
pub const SERVER_FAIL_COOLDOWN: Duration = Duration::from_secs(300);

/// Intervalo mínimo entre pasadas de purga de exceso de fuentes por archivo
/// (los temporizadores lentos de `CPartFile::Process` de eMule se ejecutan cada ~30-60 s).
pub const SOURCE_PURGE_INTERVAL: Duration = Duration::from_secs(60);

/// Cadencia completa de reequilibrio A4AF (~8 min en `ProcessA4AFClients` de eMule).
/// Los intercambios NNP dirigidos por eventos aún se ejecutan en cada ciclo.
pub const A4AF_FULL_INTERVAL: Duration = Duration::from_secs(8 * 60);

/// Intervalo sin recibir ningún byte y sin pares activos antes de que una descarga activa se considere estancada (10 min).
pub const SMART_QUEUE_STALL_TIMEOUT: Duration = Duration::from_secs(600);

/// Duración durante la cual una descarga inactiva/estancada se coloca en enfriamiento de rotación antes de reevaluarla para ranuras activas (3 min).
pub const SMART_QUEUE_ROTATION_COOLDOWN: Duration = Duration::from_secs(180);

/// Rastreador de actividad para rotación inteligente de cola y detección de estancamiento.
#[derive(Debug, Clone)]
pub struct DownloadActivityTracker {
    pub last_received_bytes: u64,
    pub last_progress_time: Instant,
    pub activated_at: Instant,
    pub cooldown_until: Option<Instant>,
}

/// `CPartFile::GetMaxSourcePerFileSoft` de eMule: umbral de repetición de consulta
/// TCP de fuentes — 90 % del límite del archivo, acotado a
/// `MAX_SOURCES_FILE_SOFT`.
pub(crate) fn max_source_per_file_soft(max_sources: usize) -> usize {
    (max_sources * 9 / 10).min(MAX_SOURCES_FILE_SOFT)
}

/// `CPartFile::GetMaxSourcePerFileUDP` de eMule: umbral de repetición de consulta
/// UDP/Kad — 75 % del límite del archivo, acotado a `MAX_SOURCES_FILE_UDP`.
pub(crate) fn max_source_per_file_udp(max_sources: usize) -> usize {
    (max_sources * 3 / 4).min(MAX_SOURCES_FILE_UDP)
}

/// Límites de conexiones de pares leídos en caliente por el coordinador.
///
/// eMule no tiene cuota por archivo: cada archivo conecta sus
/// fuentes hasta `max_peers_per_file` (`maxsourceperfile`, o
/// `FT_MAXSOURCES` cuando el `.part.met` lo especifica), desempeñando el límite global
/// `max_global_peers` el rol de `maxconnections`
/// (`CListenSocket::TooManySockets`). `min_peers_per_file` solo sirve para el
/// reequilibrio A4AF (`ProcessA4AFClients`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwarmLimits {
    /// Límite global de conexiones activas de pares (`maxconnections` de eMule).
    pub max_global_peers: usize,
    /// Límite de fuentes conectadas por archivo (`maxsourceperfile` de eMule).
    pub max_peers_per_file: usize,
    /// Umbral de inanición que dispara un intercambio A4AF (extensión eMuleRust).
    pub min_peers_per_file: usize,
}

/// Estado de un archivo consumido por el reequilibrio A4AF
/// (`ProcessA4AFClients` de eMule): peso de prioridad y pares activos.
pub(crate) struct A4afFileState {
    pub file_hash: String,
    /// Peso de eMule: Low=1, Normal=2, High=4, Release=8.
    pub priority_weight: u32,
    /// Número de sesiones activas de pares del archivo.
    pub current_active_peers: usize,
}

impl A4afFileState {
    pub(crate) fn new(
        file_hash: String,
        priority_str: &str,
        source_count: usize,
        current_active_peers: usize,
    ) -> Self {
        let priority_weight = match priority_str.to_lowercase().as_str() {
            "low" => 1,
            "high" => 4,
            "release" => 8,
            // `PR_AUTO` de eMule — `CPartFile::UpdateAutoDownPriority`: el
            // peso efectivo depende del número de fuentes conocidas
            // (`GetSourceCount`): más de 100 → low, más de 20 → normal,
            // si no → high. Recalculado en cada ciclo del coordinador como
            // hace eMule en `CPartFile::Process`.
            "auto" => {
                if source_count > 100 {
                    1
                } else if source_count > 20 {
                    2
                } else {
                    4
                }
            }
            _ => 2, // "normal" ou non précisé
        };
        Self {
            file_hash,
            priority_weight,
            current_active_peers,
        }
    }
}

/// Estado de búsqueda de fuentes Kad de un archivo — equivalente en eMule
/// a `m_LastSearchTimeKad` + `m_TotalSearchesKad` (PartFile.h).
#[derive(Debug, Clone, Copy)]
pub(crate) struct KadSourceSearchState {
    /// Plazo absoluto antes del cual no se inicia ninguna nueva búsqueda Kad
    /// para este archivo (`now + KADEMLIA_REASK_INTERVAL *
    /// total_searches` en el momento del inicio).
    pub next_reask: Instant,
    /// Número de búsquedas Kad ya iniciadas para este archivo
    /// (`m_TotalSearchesKad` de eMule, ≤ `KAD_MAX_TOTAL_SEARCHES`).
    pub total_searches: u32,
}

/// Coordinador de enjambre que orquesta conexiones de pares y descargas en segundo plano.
pub struct SwarmCoordinator {
    pub(crate) state: AppState,
    /// Última petición TCP de fuentes por archivo (`m_LastSearchTime` de eMule).
    pub(crate) last_source_query_tcp: HashMap<String, Instant>,
    /// Última petición UDP de fuentes por archivo (ciclo de eMule
    /// `m_lastudpsearchtime` reducido a una cadencia por archivo).
    pub(crate) last_source_query_udp: HashMap<String, Instant>,
    /// Estado de búsqueda Kad por archivo (`m_LastSearchTimeKad` +
    /// `m_TotalSearchesKad` de eMule).
    pub(crate) kad_source_search: HashMap<String, KadSourceSearchState>,
    /// Marca de tiempo del último inicio de búsqueda Kad, entre todos los archivos
    /// combinados (`m_lastkademliafilerequest` de eMule — `KADEMLIAASKTIME`).
    pub(crate) last_kad_file_request: Instant,
    pub(crate) last_heartbeat: Instant,
    /// Último intento de conexión por dirección de servidor (retroceso de conmutación por error).
    pub(crate) server_attempts: HashMap<String, Instant>,
    /// Última vez que se intentó una conexión automática al servidor.
    pub(crate) last_server_reconnect: Option<Instant>,
    /// Dirección del servidor que estaba conectado antes de la última caída (excluido
    /// de la selección inmediata de conmutación por error).
    pub(crate) last_connected_server: Option<String>,
    /// Última pasada de purga por exceso de fuentes por hash de archivo (regla del 80 % de eMule).
    pub(crate) last_source_purge: HashMap<String, Instant>,
    /// Última pasada de reequilibrio completo A4AF (cadencia de ~8 min de eMule).
    pub(crate) last_a4af_full: Instant,
    /// Seguimiento de concurrencia inteligente de descargas, rotación y enfriamientos por atasco.
    pub(crate) download_trackers: HashMap<Uuid, DownloadActivityTracker>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CPartFile::UpdateAutoDownPriority` de eMule: la prioridad «auto»
    /// depende del número de fuentes conocidas — ≤20 → high (4), 21..=100
    /// → normal (2), >100 → low (1).
    #[test]
    fn a4af_auto_priority_weight_thresholds() {
        for (sources, expected) in [
            (0usize, 4u32),
            (1, 4),
            (20, 4),
            (21, 2),
            (50, 2),
            (100, 2),
            (101, 1),
            (500, 1),
        ] {
            let s = A4afFileState::new("h".to_string(), "auto", sources, 0);
            assert_eq!(
                s.priority_weight, expected,
                "auto priority with {sources} sources"
            );
        }
    }

    /// Una prioridad manual siempre ignora el recuento de fuentes.
    #[test]
    fn a4af_manual_priority_ignores_source_count() {
        for (prio, expected) in [("low", 1u32), ("normal", 2), ("high", 4), ("release", 8)] {
            for sources in [0usize, 50, 500] {
                let s = A4afFileState::new("h".to_string(), prio, sources, 0);
                assert_eq!(s.priority_weight, expected, "{prio}@{sources}");
            }
        }
    }
}

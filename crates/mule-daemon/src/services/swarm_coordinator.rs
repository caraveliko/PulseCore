// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Servicio coordinador de enjambre (Swarm Coordinator).
//!
//! Trabajador autónomo en segundo plano que orquesta continuamente:
//! 1. Descubrimiento de fuentes: consulta al servidor eD2K conectado (OP_GETSOURCES) y a Kad (search_sources).
//! 2. Gestión de conexiones con pares del enjambre: selecciona fuentes candidatas de la base de datos,
//!    solicita permisos de política de red y establece sesiones con pares hasta MAX_PEERS_PER_DOWNLOAD.
//! 3. Propagación del estado del ciclo de vida: mueve automáticamente las descargas de Queued/Waiting a Downloading.

pub mod lowid_callback;
pub mod strategy;
pub mod types;

use std::collections::HashMap;
use std::time::Instant;
pub use types::{
    SwarmCoordinator, SwarmLimits, COORDINATOR_TICK_INTERVAL, HEARTBEAT_INTERVAL,
    MAX_CONNECT_PER_TICK,
};

use crate::state::AppState;

impl SwarmCoordinator {
    /// Genera el bucle autónomo del coordinador de enjambre como una tarea en segundo plano.
    pub fn start(state: AppState) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut coordinator = Self {
                state,
                last_source_query_tcp: HashMap::new(),
                last_source_query_udp: HashMap::new(),
                kad_source_search: HashMap::new(),
                last_kad_file_request: Instant::now(),
                last_heartbeat: Instant::now(),
                server_attempts: HashMap::new(),
                last_server_reconnect: None,
                last_connected_server: None,
                last_source_purge: HashMap::new(),
                last_a4af_full: Instant::now(),
                download_trackers: HashMap::new(),
            };
            coordinator.run_loop().await;
        })
    }

    /// Bucle principal de ejecución a la escucha de ticks o del apagado del daemon.
    /// El interruptor de corte (kill switch) no detiene el bucle: `tick()` se cortocircuita
    /// mientras el estado de la red sea `Disabled`, y luego se reanuda por sí solo al
    /// emitir `network.enable` (el token cancelado sería de lo contrario definitivo).
    pub async fn run_loop(&mut self) {
        let shutdown_notify = self.state.shutdown_notify.clone();
        let mut interval = tokio::time::interval(COORDINATOR_TICK_INTERVAL);

        tracing::info!("SwarmCoordinator background task started");

        loop {
            tokio::select! {
                _ = shutdown_notify.notified() => {
                    tracing::info!("SwarmCoordinator stopped by shutdown notification");
                    break;
                }
                _ = interval.tick() => {
                    self.tick().await;
                }
            }
        }
    }
}

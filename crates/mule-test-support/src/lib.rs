// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Utilidades compartidas para las pruebas de integración del workspace.
//!
//! Todas las funciones de este crate únicamente se comunican en loopback
//! (`127.0.0.1`) y no emiten ningún tráfico de red saliente, conforme a la
//! regla «solo pruebas sin conexión» de AGENTS.md.
//!
//! El crate está destinado a las `dev-dependencies` del resto de crates: no
//! debe ser utilizado por código de producción ni publicado.

pub mod daemon;
pub mod met_fixtures;
pub mod peer_fixtures;
pub mod rpc;
pub mod state;
pub mod ws;

pub use daemon::{
    spawn_test_daemon, spawn_test_daemon_with_mode, spawn_test_daemon_with_temp_dir,
    RunningDaemon,
};
pub use rpc::make_req;
pub use state::{
    download_state, download_state_with_obfuscation, kad_state, rpc_state,
    rpc_state_with_config, rpc_state_with_profile, share_state,
};
pub use ws::{recv_response_by_id, send_json};

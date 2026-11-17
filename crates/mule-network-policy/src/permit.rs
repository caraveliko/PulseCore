// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::cancellation::CancellationToken;

/// Propósito funcional de una conexión o escucha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionPurpose {
    /// Conexión TCP saliente a un servidor eD2K.
    Ed2kServer,
    /// Conexión TCP saliente a un par eD2K.
    Ed2kPeer,
    /// Operación de enrutamiento UDP / Kademlia.
    KadNode,
    /// Socket de escucha TCP entrante.
    Listener,
}

/// Rastreador compartido de recuentos de recursos activos.
#[derive(Debug, Default)]
pub struct PermitTracker {
    pub active_peer_conns: AtomicUsize,
    pub active_server_conns: AtomicUsize,
    pub active_listeners: AtomicUsize,
}

/// Permiso RAII que autoriza una conexión de socket saliente bajo la política y presupuestos activos.
/// Libera automáticamente su presupuesto reservado al descartarse.
#[derive(Debug)]
pub struct SocketPermit {
    id: u64,
    target_addr: SocketAddr,
    purpose: ConnectionPurpose,
    cancel_token: CancellationToken,
    tracker: Arc<PermitTracker>,
}

impl SocketPermit {
    pub(crate) fn new(
        id: u64,
        target_addr: SocketAddr,
        purpose: ConnectionPurpose,
        cancel_token: CancellationToken,
        tracker: Arc<PermitTracker>,
    ) -> Self {
        match purpose {
            ConnectionPurpose::Ed2kPeer => {
                tracker.active_peer_conns.fetch_add(1, Ordering::SeqCst);
            }
            ConnectionPurpose::KadNode => {}
            ConnectionPurpose::Ed2kServer => {
                tracker.active_server_conns.fetch_add(1, Ordering::SeqCst);
            }
            ConnectionPurpose::Listener => {
                tracker.active_listeners.fetch_add(1, Ordering::SeqCst);
            }
        }
        Self {
            id,
            target_addr,
            purpose,
            cancel_token,
            tracker,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn target_addr(&self) -> SocketAddr {
        self.target_addr
    }

    pub fn purpose(&self) -> ConnectionPurpose {
        self.purpose
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    pub async fn cancelled(&self) {
        self.cancel_token.cancelled().await;
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }
}

impl Drop for SocketPermit {
    fn drop(&mut self) {
        match self.purpose {
            ConnectionPurpose::Ed2kPeer => {
                self.tracker
                    .active_peer_conns
                    .fetch_sub(1, Ordering::SeqCst);
            }
            ConnectionPurpose::KadNode => {}
            ConnectionPurpose::Ed2kServer => {
                self.tracker
                    .active_server_conns
                    .fetch_sub(1, Ordering::SeqCst);
            }
            ConnectionPurpose::Listener => {
                self.tracker.active_listeners.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }
}

/// Permiso RAII que autoriza un socket de escucha vinculado bajo la política y presupuestos activos.
/// Decrementa automáticamente el recuento de escuchas activas al descartarse.
#[derive(Debug)]
pub struct ListenPermit {
    id: u64,
    bind_addr: SocketAddr,
    cancel_token: CancellationToken,
    tracker: Arc<PermitTracker>,
}

impl ListenPermit {
    pub(crate) fn new(
        id: u64,
        bind_addr: SocketAddr,
        cancel_token: CancellationToken,
        tracker: Arc<PermitTracker>,
    ) -> Self {
        tracker.active_listeners.fetch_add(1, Ordering::SeqCst);
        Self {
            id,
            bind_addr,
            cancel_token,
            tracker,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    pub async fn cancelled(&self) {
        self.cancel_token.cancelled().await;
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }
}

impl Drop for ListenPermit {
    fn drop(&mut self) {
        self.tracker.active_listeners.fetch_sub(1, Ordering::SeqCst);
    }
}

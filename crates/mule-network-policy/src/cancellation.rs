// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

/// Token de cancelación ligero y compatible con asincronía para coordinar el cierre inmediato de sockets.
#[derive(Debug, Clone)]
pub struct CancellationToken {
    inner: Arc<TokenInner>,
}

#[derive(Debug)]
struct TokenInner {
    cancelled: AtomicBool,
    notify: Notify,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(TokenInner {
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        }
    }

    /// Activa la cancelación y notifica a todas las tareas que esperan en `cancelled()`.
    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            self.inner.notify.notify_waiters();
        }
    }

    /// Indica si este token ha sido cancelado.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// Espera de forma asíncrona hasta que se cancele el token.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        loop {
            let notified = self.inner.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
            if self.is_cancelled() {
                return;
            }
        }
    }

    /// Restablece el estado de cancelación, permitiendo la reutilización del token al volver a habilitar la red.
    pub fn reset(&self) {
        self.inner.cancelled.store(false, Ordering::SeqCst);
    }
}

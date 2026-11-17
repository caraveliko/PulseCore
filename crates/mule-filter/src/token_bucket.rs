// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenBucketError {
    #[error("Rate limit wait was cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitsConfig {
    /// Límite de tasa de descarga en bytes por segundo (0 = ilimitado)
    pub download_bytes_per_sec: u64,
    /// Límite de tasa de subida en bytes por segundo (0 = ilimitado)
    pub upload_bytes_per_sec: u64,
    /// Límite de creación de conexiones en conexiones por minuto (0 = ilimitado)
    pub connections_per_minute: u32,
    /// Peticiones de búsqueda e interacción con servidores por segundo (0 = ilimitado)
    pub server_requests_per_sec: u32,
    /// Paquetes Kad/UDP por segundo (0 = ilimitado)
    pub kad_packets_per_sec: u32,
}

impl Default for RateLimitsConfig {
    fn default() -> Self {
        Self {
            download_bytes_per_sec: 0,   // ilimitado
            upload_bytes_per_sec: 0,     // ilimitado
            connections_per_minute: 120, // 2 por segundo máx. por defecto
            server_requests_per_sec: 10,
            kad_packets_per_sec: 50,
        }
    }
}

/// Cubo de fichas (token bucket) para la limitación de tasa de bytes u operaciones.
/// Admite ráfagas, recarga dinámica y cancelación.
pub struct TokenBucket {
    rate_per_sec: AtomicU64,
    burst_capacity: AtomicU64,
    state: Mutex<BucketState>,
}

struct BucketState {
    tokens: f64,
    last_update: Instant,
}

impl TokenBucket {
    pub fn new(rate_per_sec: u64, burst_capacity: u64) -> Self {
        Self {
            rate_per_sec: AtomicU64::new(rate_per_sec),
            burst_capacity: AtomicU64::new(burst_capacity),
            state: Mutex::new(BucketState {
                tokens: burst_capacity as f64,
                last_update: Instant::now(),
            }),
        }
    }

    pub fn set_rate(&self, rate_per_sec: u64, burst_capacity: u64) {
        self.rate_per_sec.store(rate_per_sec, Ordering::Relaxed);
        self.burst_capacity.store(burst_capacity, Ordering::Relaxed);
        let mut state = self.state.lock().unwrap();
        state.last_update = Instant::now();
        if state.tokens > burst_capacity as f64 {
            state.tokens = burst_capacity as f64;
        }
    }

    pub fn rate_per_sec(&self) -> u64 {
        self.rate_per_sec.load(Ordering::Relaxed)
    }

    pub fn burst_capacity(&self) -> u64 {
        self.burst_capacity.load(Ordering::Relaxed)
    }

    /// Intenta adquirir `count` fichas inmediatamente sin esperar.
    /// Devuelve `true` si se adquirieron las fichas, `false` en caso contrario.
    pub fn try_acquire(&self, count: u64) -> bool {
        let rate = self.rate_per_sec.load(Ordering::Relaxed);
        if rate == 0 {
            // 0 significa ilimitado
            return true;
        }

        let mut state = self.state.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_update).as_secs_f64();
        let burst = self.burst_capacity.load(Ordering::Relaxed) as f64;

        state.tokens = (state.tokens + elapsed * (rate as f64)).min(burst);
        state.last_update = now;

        if state.tokens >= count as f64 {
            state.tokens -= count as f64;
            true
        } else {
            false
        }
    }

    /// Adquiere `count` fichas de forma asíncrona, durmiendo si es necesario.
    /// Si la retrollamada `cancel` o el token de cancelación se activan, devuelve `Err(TokenBucketError::Cancelled)`.
    pub async fn acquire_tokens<F>(
        &self,
        count: u64,
        is_cancelled: F,
    ) -> Result<(), TokenBucketError>
    where
        F: Fn() -> bool,
    {
        let rate = self.rate_per_sec.load(Ordering::Relaxed);
        if rate == 0 {
            return Ok(());
        }

        loop {
            if is_cancelled() {
                return Err(TokenBucketError::Cancelled);
            }

            let wait_duration = {
                let mut state = self.state.lock().unwrap();
                let now = Instant::now();
                let elapsed = now.duration_since(state.last_update).as_secs_f64();
                let burst = self.burst_capacity.load(Ordering::Relaxed) as f64;

                state.tokens = (state.tokens + elapsed * (rate as f64)).min(burst);
                state.last_update = now;

                if state.tokens >= count as f64 {
                    state.tokens -= count as f64;
                    return Ok(());
                }

                // Fichas necesarias
                let needed = (count as f64) - state.tokens;
                let wait_secs = needed / (rate as f64);
                Duration::from_secs_f64(wait_secs.clamp(0.005, 2.0))
            };

            // Dormir en pequeños incrementos o durante wait_duration
            tokio::select! {
                _ = tokio::time::sleep(wait_duration) => {
                    // repetir el bucle y volver a comprobar
                }
            }
        }
    }
}

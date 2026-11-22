// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Estimador pasivo de retardo en cola estilo LEDBAT++ (RFC 6817) para complementar USS.
//!
//! Cuando el sondeo ICMP no está disponible (entornos sin privilegios o ICMP filtrado),
//! las muestras pasivas de RTT recogidas en transferencias y saludos alimentan este
//! estimador para detectar *bufferbloat* en la subida local.

use std::collections::VecDeque;

/// Número de cubetas de un minuto para el cálculo deslizante de `base_delay` (10 minutos).
pub const BASE_DELAY_BUCKETS: usize = 10;
/// Duración de cada cubeta de `base_delay` en milisegundos (60 s).
pub const BASE_DELAY_BUCKET_MS: i64 = 60_000;
/// Número de muestras recientes para calcular `current_delay` (mínimo filtrado).
pub const CURRENT_DELAY_WINDOW: usize = 4;
/// Retardo de cola objetivo en milisegundos (LEDBAT / LEDBAT++: 60 ms).
pub const TARGET_QUEUING_DELAY_MS: i64 = 60;
/// Paso aditivo mínimo por ciclo (bytes/s) para enlaces lentos o arranque.
pub const LEDBAT_MIN_GAIN_BPS: f64 = 16_384.0;
/// Incremento aditivo proporcional al límite actual por ciclo (+5 % con cola vacía).
pub const LEDBAT_ADDITIVE_RATIO: f64 = 0.05;
/// Recorte multiplicativo máximo por ciclo ante congestión severa (-25 %).
pub const LEDBAT_MULTIPLICATIVE_MAX_DROP: f64 = 0.25;

#[derive(Debug, Clone, Copy)]
struct DelayBucket {
    bucket_id: i64,
    min_rtt_ms: u32,
}

/// Estimador pasivo LEDBAT++ basado en diferencia entre retardo actual y retardo base mínimo.
#[derive(Debug, Default)]
pub struct LedbatEstimator {
    base_buckets: VecDeque<DelayBucket>,
    recent_samples: VecDeque<u32>,
    last_sample_ms: Option<i64>,
}

impl LedbatEstimator {
    /// Crea un estimador vacío sin muestras.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra una nueva muestra de RTT en milisegundos en el instante `now_ms` (unix o relativo monótono).
    pub fn record_rtt_sample(&mut self, now_ms: i64, rtt_ms: u32) {
        let rtt_ms = rtt_ms.max(1);
        self.last_sample_ms = Some(now_ms);
        let bucket_id = now_ms.div_euclid(BASE_DELAY_BUCKET_MS);

        if let Some(back) = self.base_buckets.back_mut() {
            if back.bucket_id == bucket_id {
                back.min_rtt_ms = back.min_rtt_ms.min(rtt_ms);
            } else if bucket_id > back.bucket_id {
                self.base_buckets.push_back(DelayBucket {
                    bucket_id,
                    min_rtt_ms: rtt_ms,
                });
            }
        } else {
            self.base_buckets.push_back(DelayBucket {
                bucket_id,
                min_rtt_ms: rtt_ms,
            });
        }

        while self.base_buckets.len() > BASE_DELAY_BUCKETS {
            self.base_buckets.pop_front();
        }
        while let Some(front) = self.base_buckets.front() {
            if bucket_id - front.bucket_id >= BASE_DELAY_BUCKETS as i64 {
                self.base_buckets.pop_front();
            } else {
                break;
            }
        }

        self.recent_samples.push_back(rtt_ms);
        while self.recent_samples.len() > CURRENT_DELAY_WINDOW {
            self.recent_samples.pop_front();
        }
    }

    /// Devuelve `true` si existen muestras recientes dentro de `max_age_ms`.
    pub fn has_fresh_samples(&self, now_ms: i64, max_age_ms: i64) -> bool {
        self.last_sample_ms
            .is_some_and(|t| now_ms.saturating_sub(t) <= max_age_ms)
            && !self.recent_samples.is_empty()
    }

    /// Devuelve el retardo base mínimo observado en la ventana de 10 minutos.
    pub fn base_delay_ms(&self) -> Option<u32> {
        self.base_buckets.iter().map(|b| b.min_rtt_ms).min()
    }

    /// Devuelve el retardo actual filtrado (mínimo de las últimas `CURRENT_DELAY_WINDOW` muestras).
    pub fn current_delay_ms(&self) -> Option<u32> {
        self.recent_samples.iter().copied().min()
    }

    /// Estima el retardo de cola actual (`current_delay - base_delay`) en milisegundos.
    pub fn queuing_delay_ms(&self) -> Option<i64> {
        let base = i64::from(self.base_delay_ms()?);
        let cur = i64::from(self.current_delay_ms()?);
        Some(cur.saturating_sub(base))
    }

    /// Calcula el nuevo límite de subida (bytes/s) aplicando una ley AIMD
    /// proporcional al ancho de banda (subida aditiva escalada, bajada multiplicativa).
    pub fn compute_rate_adjustment(
        &self,
        current_limit_bps: u64,
        min_bps: u64,
        max_bps: u64,
    ) -> Option<u64> {
        let qdelay = self.queuing_delay_ms()?;
        let off_target = ((TARGET_QUEUING_DELAY_MS - qdelay) as f64
            / TARGET_QUEUING_DELAY_MS as f64)
            .clamp(-1.0, 1.0);
        let next = if off_target >= 0.0 {
            let step = (current_limit_bps as f64 * LEDBAT_ADDITIVE_RATIO)
                .max(LEDBAT_MIN_GAIN_BPS)
                * off_target;
            current_limit_bps.saturating_add(step.round() as u64)
        } else {
            let drop = (current_limit_bps as f64 * LEDBAT_MULTIPLICATIVE_MAX_DROP)
                .max(LEDBAT_MIN_GAIN_BPS)
                * (-off_target);
            current_limit_bps.saturating_sub(drop.round() as u64)
        }
        .max(min_bps);
        Some(if max_bps > 0 { next.min(max_bps) } else { next })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledbat_detects_bufferbloat_and_reduces_rate() {
        let mut est = LedbatEstimator::new();
        // Establecer línea base de 20 ms.
        for i in 0..4 {
            est.record_rtt_sample(1_000 + i * 100, 20);
        }
        assert_eq!(est.base_delay_ms(), Some(20));
        assert_eq!(est.queuing_delay_ms(), Some(0));

        // Sin retardo de cola, permite incrementar lentamente.
        let up = est.compute_rate_adjustment(200_000, 10_240, 1_000_000).unwrap();
        assert!(up > 200_000);

        // Congestión de cola (+120 ms por encima del mínimo) -> debe reducir.
        for i in 0..CURRENT_DELAY_WINDOW {
            est.record_rtt_sample(70_000 + (i as i64) * 100, 140);
        }
        assert_eq!(est.queuing_delay_ms(), Some(120));
        let down = est.compute_rate_adjustment(200_000, 10_240, 1_000_000).unwrap();
        assert!(down < 200_000);
    }

    #[test]
    fn ledbat_scales_aimd_for_fiber_rates() {
        let mut est = LedbatEstimator::new();
        for i in 0..4 {
            est.record_rtt_sample(1_000 + i * 100, 10);
        }
        // Sin congestión en fibra (8 MB/s): debe escalar proporcionalmente (+5 %).
        let up = est
            .compute_rate_adjustment(8_000_000, 10_240, 10_000_000)
            .unwrap();
        assert_eq!(up, 8_400_000);

        // Congestión severa (+120 ms): recorte multiplicativo del 25 % (6 MB/s).
        for i in 0..CURRENT_DELAY_WINDOW {
            est.record_rtt_sample(70_000 + (i as i64) * 100, 130);
        }
        let down = est
            .compute_rate_adjustment(8_000_000, 10_240, 10_000_000)
            .unwrap();
        assert_eq!(down, 6_000_000);
    }
}

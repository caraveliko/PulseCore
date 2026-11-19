// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Ventana de peticiones de bloques adaptativa por BDP (*Bandwidth-Delay Product*)
//! y seguimiento EWMA del RTT por sesión de par.
//!
//! Mantiene intactos los umbrales clásicos de eMule para enlaces lentos
//! (`<= 150 KiB/s`) y escala dinámicamente el *pipelining* para enlaces rápidos.

use std::time::Instant;

/// Tamaño estándar de bloque eD2K (180 KiB).
pub const ED2K_BLOCK_SIZE_BYTES: u64 = 184_320;
/// Mínimo de bloques en vuelo en modo adaptativo cuando no hay muestras de velocidad.
pub const MIN_ADAPTIVE_BLOCKS: usize = 3;
/// Techo máximo de bloques en vuelo por par en fibra de alta velocidad.
pub const MAX_ADAPTIVE_BLOCKS: usize = 36;
/// Umbral a partir del cual se aplica escalado dinámico por BDP (`150 KiB/s`).
pub const HIGH_SPEED_BDP_THRESHOLD_BPS: u64 = 150 * 1024;
/// Factor de suavizado EWMA para la estimación de RTT (`srtt`).
pub const RTT_EWMA_ALPHA: f64 = 0.25;
/// Suelo clásico por encima del umbral de 150 KiB/s.
const HIGH_SPEED_BASE_BLOCKS: usize = 9;
/// Colchón adicional de bloques para absorber jitter sin vaciar la ventana TCP.
const PIPELINE_HEADROOM_BLOCKS: usize = 2;

/// Seguidor de RTT por sesión de par mediante media móvil exponencial (EWMA).
#[derive(Debug, Default)]
pub struct PeerRttTracker {
    pending_request_sent_at: Option<Instant>,
    srtt_ms: Option<f64>,
    min_rtt_ms: Option<u64>,
}

impl PeerRttTracker {
    /// Crea un nuevo seguidor sin muestras iniciales.
    pub fn new() -> Self {
        Self::default()
    }

    /// Marca el instante en que se envía una petición `OP_REQUESTPARTS*` si no había ninguna pendiente de medir.
    pub fn on_request_sent(&mut self, now: Instant) {
        if self.pending_request_sent_at.is_none() {
            self.pending_request_sent_at = Some(now);
        }
    }

    /// Registra la llegada del primer fragmento de respuesta y devuelve la muestra RTT en milisegundos, si procede.
    pub fn on_chunk_received(&mut self, now: Instant) -> Option<u32> {
        let sent_at = self.pending_request_sent_at.take()?;
        let sample_ms = now.saturating_duration_since(sent_at).as_millis().max(1) as u64;
        self.srtt_ms = Some(match self.srtt_ms {
            Some(prev) => prev * (1.0 - RTT_EWMA_ALPHA) + (sample_ms as f64) * RTT_EWMA_ALPHA,
            None => sample_ms as f64,
        });
        self.min_rtt_ms = Some(match self.min_rtt_ms {
            Some(prev) => prev.min(sample_ms),
            None => sample_ms,
        });
        Some(sample_ms.min(u32::MAX as u64) as u32)
    }

    /// Devuelve el RTT suavizado actual en milisegundos redondeado al entero más cercano.
    pub fn srtt_ms(&self) -> Option<u64> {
        self.srtt_ms.map(|v| v.round().max(1.0) as u64)
    }

    /// Devuelve el RTT mínimo observado durante la sesión.
    pub fn min_rtt_ms(&self) -> Option<u64> {
        self.min_rtt_ms
    }
}

/// Calcula el número óptimo de bloques en vuelo para un par combinando compatibilidad
/// estricta con eMule (`<= 150 KiB/s`) y escalado por BDP por encima de ese umbral.
pub fn adaptive_pending_block_count(
    peer_bps: u64,
    srtt_ms: Option<u64>,
    is_emule_client: bool,
) -> usize {
    if peer_bps <= HIGH_SPEED_BDP_THRESHOLD_BPS {
        return super::transfer::emule_pending_block_count(peer_bps, is_emule_client)
            .max(if peer_bps == 0 { MIN_ADAPTIVE_BLOCKS } else { 1 });
    }
    let Some(rtt_ms) = srtt_ms.filter(|&r| r > 0) else {
        return HIGH_SPEED_BASE_BLOCKS;
    };
    let bdp_bytes = peer_bps.saturating_mul(rtt_ms) / 1000;
    let bdp_blocks = bdp_bytes.div_ceil(ED2K_BLOCK_SIZE_BYTES) as usize;
    bdp_blocks
        .saturating_add(PIPELINE_HEADROOM_BLOCKS)
        .clamp(HIGH_SPEED_BASE_BLOCKS, MAX_ADAPTIVE_BLOCKS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn adaptive_pipelining_preserves_emule_low_tiers_and_scales_bdp() {
        assert_eq!(adaptive_pending_block_count(0, None, true), 3);
        assert_eq!(adaptive_pending_block_count(2 * 1024, Some(40), true), 1);
        assert_eq!(adaptive_pending_block_count(100 * 1024, Some(40), true), 6);
        // Enlaces rápidos sin RTT conocido mantienen el suelo de 9 bloques.
        assert_eq!(adaptive_pending_block_count(2 * 1024 * 1024, None, true), 9);
        // 20 MiB/s * 100 ms = 2 MiB ≈ 12 bloques + 2 = 14.
        let blocks = adaptive_pending_block_count(20 * 1024 * 1024, Some(100), true);
        assert!(blocks > 9 && blocks <= MAX_ADAPTIVE_BLOCKS);
        // Enlace extremo queda acotado a MAX_ADAPTIVE_BLOCKS.
        assert_eq!(
            adaptive_pending_block_count(100 * 1024 * 1024, Some(300), true),
            MAX_ADAPTIVE_BLOCKS
        );
    }

    #[test]
    fn rtt_tracker_ewma_updates_cleanly() {
        let mut tracker = PeerRttTracker::new();
        let t0 = Instant::now();
        tracker.on_request_sent(t0);
        let sample = tracker.on_chunk_received(t0 + Duration::from_millis(40));
        assert_eq!(sample, Some(40));
        assert_eq!(tracker.srtt_ms(), Some(40));
        assert_eq!(tracker.min_rtt_ms(), Some(40));

        tracker.on_request_sent(t0 + Duration::from_millis(100));
        let _ = tracker.on_chunk_received(t0 + Duration::from_millis(180));
        assert_eq!(tracker.srtt_ms(), Some(50));
        assert_eq!(tracker.min_rtt_ms(), Some(40));
    }
}

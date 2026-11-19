// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Puntuación inteligente de pares (*Peer Scoring*), retroceso diferenciado con *jitter*
//! y defensa de diversidad por subred anti-Sybil/Eclipse.

use std::collections::HashMap;
use std::net::SocketAddr;

pub use crate::services::inbound_listener::subnet_guard::SubnetKey as PeerSubnetPrefix;
use mule_core::model::{PeerFailureReason, PeerSource};

/// Variación aleatoria de *jitter* (±20 %) aplicada a los enfriamientos para evitar sincronización masiva.
pub const BACKOFF_JITTER_FRACTION: f64 = 0.20;
/// Enfriamiento estándar para errores transitorios (1 min).
const STANDARD_BACKOFF_BASE_MS: i64 = 60_000;
/// Enfriamiento tope para cola perdida (10 min).
const QUEUE_BACKOFF_MAX_MS: i64 = 10 * 60_000;
/// Enfriamiento tope estándar (20 min).
const STANDARD_BACKOFF_MAX_MS: i64 = 20 * 60_000;
/// Enfriamiento base para errores de saludo/ofuscación (5 min).
const CRYPTO_BACKOFF_BASE_MS: i64 = 5 * 60_000;
/// Enfriamiento tope para errores de saludo/ofuscación (40 min).
const CRYPTO_BACKOFF_MAX_MS: i64 = 40 * 60_000;
/// Enfriamiento base para corrupción de datos o discordancia de hash (60 min).
const CORRUPTION_BACKOFF_BASE_MS: i64 = 60 * 60_000;
/// Enfriamiento tope para corrupción de datos (120 min).
const CORRUPTION_BACKOFF_MAX_MS: i64 = 120 * 60_000;
/// Enfriamiento de reconsulta NNP de eMule (`FILEREASKTIME * 2` = 58 min).
const NNP_BACKOFF_MS: i64 = 58 * 60_000;
/// Máximo de perfiles de pares retenidos simultáneamente en `PeerScoreTracker`.
pub const MAX_SCORED_PEERS: usize = 4096;
/// Penalización por candidato adicional que comparte el mismo prefijo `/24` o `/48`.
const SUBNET_CLUSTERING_PENALTY: f64 = 350.0;

/// Calcula el tiempo de enfriamiento en ms según `fail_count`, `reason` y un valor `jitter_unit` en `[0.0, 1.0]`.
pub fn calculate_backoff_for_reason(
    fail_count: u32,
    reason: PeerFailureReason,
    jitter_unit: f64,
) -> i64 {
    if reason == PeerFailureReason::NoNeededParts {
        return NNP_BACKOFF_MS;
    }
    let effective_fails = fail_count.max(1);
    let exponent = (effective_fails - 1).min(5);
    let factor = 1i64 << exponent;

    let (base_ms, max_ms) = match reason {
        PeerFailureReason::TimeoutQueue => (STANDARD_BACKOFF_BASE_MS, QUEUE_BACKOFF_MAX_MS),
        PeerFailureReason::ObfuscationNegotiationFailed
        | PeerFailureReason::InvalidHandshake
        | PeerFailureReason::MissingUserHashForObfuscation
        | PeerFailureReason::InvalidRange => (CRYPTO_BACKOFF_BASE_MS, CRYPTO_BACKOFF_MAX_MS),
        PeerFailureReason::CorruptedData | PeerFailureReason::HashMismatch => {
            (CORRUPTION_BACKOFF_BASE_MS, CORRUPTION_BACKOFF_MAX_MS)
        }
        _ => (STANDARD_BACKOFF_BASE_MS, STANDARD_BACKOFF_MAX_MS),
    };

    let raw = base_ms.saturating_mul(factor).min(max_ms);
    let clamped_jitter = jitter_unit.clamp(0.0, 1.0);
    let multiplier = (1.0 - BACKOFF_JITTER_FRACTION) + (2.0 * BACKOFF_JITTER_FRACTION * clamped_jitter);
    ((raw as f64) * multiplier).round().max(1_000.0) as i64
}

#[derive(Debug, Clone, Default)]
pub struct PeerReputation {
    pub avg_bps: u64,
    pub rtt_ms: Option<u32>,
    pub sui_verified: bool,
    pub modern_pc26: bool,
    pub success_count: u32,
    pub fail_count: u32,
    pub corruption_strikes: u32,
}

/// Seguimiento en memoria de reputación y diversidad de subred para priorizar las mejores fuentes.
#[derive(Debug, Default)]
pub struct PeerScoreTracker {
    peers: HashMap<String, PeerReputation>,
}

impl PeerScoreTracker {
    /// Crea un nuevo registrador de reputación vacío.
    pub fn new() -> Self {
        Self::default()
    }

    fn entry_mut(&mut self, addr: &str) -> &mut PeerReputation {
        if self.peers.len() >= MAX_SCORED_PEERS && !self.peers.contains_key(addr) {
            if let Some(oldest_key) = self.peers.keys().next().cloned() {
                self.peers.remove(&oldest_key);
            }
        }
        self.peers.entry(addr.to_string()).or_default()
    }

    /// Registra capacidades de saludo y RTT inicial observados para `addr`.
    pub fn record_handshake_profile(
        &mut self,
        addr: &str,
        modern_pc26: bool,
        sui_verified: bool,
        rtt_ms: Option<u32>,
    ) {
        let rep = self.entry_mut(addr);
        rep.modern_pc26 |= modern_pc26;
        rep.sui_verified |= sui_verified;
        if let Some(r) = rtt_ms {
            rep.rtt_ms = Some(r.max(1));
        }
    }

    /// Registra una transferencia o avance satisfactorio del par.
    pub fn record_transfer_success(&mut self, addr: &str, bps: u64) {
        let rep = self.entry_mut(addr);
        rep.success_count = rep.success_count.saturating_add(1);
        rep.avg_bps = if rep.avg_bps == 0 {
            bps
        } else {
            (rep.avg_bps * 3 + bps) / 4
        };
    }

    /// Registra un fallo normalizado para `addr`.
    pub fn record_failure(&mut self, addr: &str, reason: PeerFailureReason) {
        let rep = self.entry_mut(addr);
        rep.fail_count = rep.fail_count.saturating_add(1);
        if matches!(
            reason,
            PeerFailureReason::CorruptedData | PeerFailureReason::HashMismatch
        ) {
            rep.corruption_strikes = rep.corruption_strikes.saturating_add(1);
        }
    }

    /// Calcula la puntuación de prioridad de una fuente (mayor = mejor candidato),
    /// descontando concentración de la misma subred `/24` o `/48`.
    pub fn score_source(&self, src: &PeerSource, subnet_peers_count: usize) -> f64 {
        let base_freshness = (src.last_seen_unix_ms as f64) / 1_000_000_000.0;
        let db_fail_penalty = f64::from(src.fail_count) * 400.0;
        let crypt_bonus = if src.supports_crypt() { 150.0 } else { 0.0 };

        let rep_bonus = if let Some(rep) = self.peers.get(&src.address) {
            let speed_score = ((rep.avg_bps as f64) / 10_240.0).min(1_500.0);
            let rtt_bonus = rep
                .rtt_ms
                .map(|ms| (300.0 - f64::from(ms).min(300.0)) * 2.0)
                .unwrap_or(0.0);
            let trust_bonus = (if rep.sui_verified { 500.0 } else { 0.0 })
                + (if rep.modern_pc26 { 300.0 } else { 0.0 });
            let reliability = f64::from(rep.success_count) * 120.0
                - f64::from(rep.fail_count) * 200.0
                - f64::from(rep.corruption_strikes) * 2_500.0;
            speed_score + rtt_bonus + trust_bonus + reliability
        } else {
            0.0
        };

        let subnet_penalty = if subnet_peers_count > 1 {
            ((subnet_peers_count - 1) as f64) * SUBNET_CLUSTERING_PENALTY
        } else {
            0.0
        };

        base_freshness + crypt_bonus + rep_bonus - db_fail_penalty - subnet_penalty
    }

    /// Ordena las fuentes candidatas combinando puntuación individual y diversidad de subred.
    pub fn sort_candidates(&self, sources: &mut [PeerSource]) {
        let mut subnet_counts: HashMap<PeerSubnetPrefix, usize> = HashMap::new();
        for s in sources.iter() {
            if let Ok(sa) = s.address.parse::<SocketAddr>() {
                if let Some(prefix) = PeerSubnetPrefix::from_ip(sa.ip()) {
                    *subnet_counts.entry(prefix).or_insert(0) += 1;
                }
            }
        }
        sources.sort_by(|a, b| {
            let count_for = |src: &PeerSource| -> usize {
                src.address
                    .parse::<SocketAddr>()
                    .ok()
                    .and_then(|sa| PeerSubnetPrefix::from_ip(sa.ip()))
                    .and_then(|p| subnet_counts.get(&p).copied())
                    .unwrap_or(1)
            };
            let sa = self.score_source(a, count_for(a));
            let sb = self.score_source(b, count_for(b));
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mule_core::model::{PeerSourceOrigin, PeerSourceStatus};

    fn mk_source(addr: &str, fail_count: u32) -> PeerSource {
        PeerSource {
            file_hash: "aa".repeat(16),
            address: addr.to_string(),
            ip: 0,
            port: 4662,
            origin: PeerSourceOrigin::Kad,
            discovered_at_unix_ms: 1_000,
            last_seen_unix_ms: 1_000,
            status: PeerSourceStatus::Candidate,
            fail_count,
            last_error: None,
            cooldown_until_unix_ms: None,
            user_hash: None,
            crypt_options: 1,
            buddy_id: None,
            software: None,
            availability: None,
            server_ip: 0,
            server_port: 0,
            udp_port: 0,
        }
    }

    #[test]
    fn backoff_differentiates_causes_and_applies_jitter() {
        let q = calculate_backoff_for_reason(1, PeerFailureReason::TimeoutQueue, 0.5);
        let c = calculate_backoff_for_reason(1, PeerFailureReason::CorruptedData, 0.5);
        assert_eq!(q, 60_000);
        assert_eq!(c, 3_600_000);

        let low = calculate_backoff_for_reason(1, PeerFailureReason::ConnectRefused, 0.0);
        let high = calculate_backoff_for_reason(1, PeerFailureReason::ConnectRefused, 1.0);
        assert!(low < 60_000 && high > 60_000);
    }

    #[test]
    fn scoring_penalizes_sybil_subnet_and_rewards_verified_modern_peers() {
        let mut tracker = PeerScoreTracker::new();
        tracker.record_handshake_profile("198.51.100.10:4662", true, true, Some(25));
        tracker.record_transfer_success("198.51.100.10:4662", 500_000);
        tracker.record_failure("203.0.113.10:4662", PeerFailureReason::CorruptedData);

        let mut list = vec![
            mk_source("203.0.113.10:4662", 1),
            mk_source("192.0.2.1:4662", 0),
            mk_source("192.0.2.2:4662", 0),
            mk_source("192.0.2.3:4662", 0),
            mk_source("198.51.100.10:4662", 0),
        ];
        tracker.sort_candidates(&mut list);
        assert_eq!(list[0].address, "198.51.100.10:4662");
        assert_eq!(list.last().unwrap().address, "203.0.113.10:4662");
    }
}

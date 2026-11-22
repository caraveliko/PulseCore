// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Presupuesto unificado de subida eD2K + BitTorrent y cálculo de techos contextuales.

/// Porcentaje mínimo garantizado para cada protocolo cuando eD2K y BitTorrent compiten por subida.
pub const MIN_PROTOCOL_SHARE_PCT: u64 = 20;
/// Reserva preventiva para el protocolo inactivo cuando el otro está transfiriendo.
pub const IDLE_TORRENT_RESERVE_PCT: u64 = 20;
/// Caudal mínimo por protocolo al dividir un presupuesto finito (16 KiB/s).
pub const MIN_PROTOCOL_BUDGET_BPS: u64 = 16 * 1024;

/// Resultado del reparto dinámico de presupuesto entre eD2K y BitTorrent (bytes/s).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitUploadBudget {
    pub ed2k_bps: u64,
    pub torrent_bps: u64,
}

/// Reparte un presupuesto global de subida (`total_bps`; `0` = ilimitado) entre eD2K y BitTorrent
/// en función de su demanda real en el instante actual.
pub fn split_upload_budget(
    total_bps: u64,
    ed2k_active_bps: u64,
    bt_active_bps: u64,
) -> SplitUploadBudget {
    if total_bps == 0 {
        return SplitUploadBudget {
            ed2k_bps: 0,
            torrent_bps: 0,
        };
    }

    match (ed2k_active_bps > 0, bt_active_bps > 0) {
        (false, false) => SplitUploadBudget {
            ed2k_bps: total_bps,
            torrent_bps: total_bps,
        },
        (true, false) => SplitUploadBudget {
            ed2k_bps: total_bps,
            torrent_bps: ((total_bps * IDLE_TORRENT_RESERVE_PCT) / 100)
                .max(MIN_PROTOCOL_BUDGET_BPS)
                .min(total_bps),
        },
        (false, true) => SplitUploadBudget {
            ed2k_bps: ((total_bps * IDLE_TORRENT_RESERVE_PCT) / 100)
                .max(MIN_PROTOCOL_BUDGET_BPS)
                .min(total_bps),
            torrent_bps: total_bps,
        },
        (true, true) => {
            let sum = ed2k_active_bps.saturating_add(bt_active_bps).max(1);
            let ed2k_pct = ((ed2k_active_bps.saturating_mul(100)) / sum)
                .clamp(MIN_PROTOCOL_SHARE_PCT, 100 - MIN_PROTOCOL_SHARE_PCT);
            let ed2k_bps = (total_bps.saturating_mul(ed2k_pct) / 100)
                .max(MIN_PROTOCOL_BUDGET_BPS)
                .min(total_bps);
            let torrent_bps = total_bps
                .saturating_sub(ed2k_bps)
                .max(MIN_PROTOCOL_BUDGET_BPS.min(total_bps));
            SplitUploadBudget {
                ed2k_bps,
                torrent_bps,
            }
        }
    }
}

/// Entradas para evaluar el techo efectivo de subida según contexto de red, reposo, streaming y cuota.
#[derive(Debug, Clone, Copy, Default)]
pub struct CeilingInputs {
    pub max_upload_bps: u64,
    pub idle_upload_cap_bps: u64,
    pub is_idle: bool,
    pub streaming_upload_cap_bps: u64,
    pub streaming_active: bool,
    pub quota_eco_cap_bps: u64,
}

/// Resultado de evaluación del techo efectivo y su motivo prioritario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CeilingDecision {
    pub cap_bps: u64,
    pub reason: &'static str,
    pub idle_cap_active: bool,
    pub streaming_cap_active: bool,
    pub quota_eco_active: bool,
}

/// Calcula el techo efectivo mínimo entre las restricciones activas y determina el motivo dominante.
pub fn compute_effective_ceiling(inputs: CeilingInputs) -> CeilingDecision {
    let idle_cap_active = inputs.is_idle && inputs.idle_upload_cap_bps > 0;
    let streaming_cap_active = inputs.streaming_active && inputs.streaming_upload_cap_bps > 0;
    let quota_eco_active = inputs.quota_eco_cap_bps > 0;

    // Orden de desempate cuando dos techos coinciden exactamente: streaming > cuota eco > reposo > normal.
    let candidates = [
        ("streaming", streaming_cap_active, inputs.streaming_upload_cap_bps),
        ("quota_eco", quota_eco_active, inputs.quota_eco_cap_bps),
        ("idle_cap", idle_cap_active, inputs.idle_upload_cap_bps),
        ("normal", inputs.max_upload_bps > 0, inputs.max_upload_bps),
    ];

    let mut best: Option<(&'static str, u64)> = None;
    for (reason, active, cap) in candidates {
        if !active || cap == 0 {
            continue;
        }
        match best {
            None => best = Some((reason, cap)),
            Some((_, cur_cap)) if cap < cur_cap => best = Some((reason, cap)),
            _ => {}
        }
    }

    let (cap_bps, reason) = match best {
        Some((r, c)) => (c, r),
        None => (0, "normal"),
    };

    CeilingDecision {
        cap_bps,
        reason,
        idle_cap_active,
        streaming_cap_active,
        quota_eco_active,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_budget_unlimited_passes_zero_to_both() {
        assert_eq!(
            split_upload_budget(0, 5_000_000, 1_000_000),
            SplitUploadBudget {
                ed2k_bps: 0,
                torrent_bps: 0
            }
        );
    }

    #[test]
    fn split_budget_gives_full_to_single_active_protocol() {
        let s = split_upload_budget(4_000_000, 3_000_000, 0);
        assert_eq!(s.ed2k_bps, 4_000_000);
        assert_eq!(s.torrent_bps, 800_000);
    }

    #[test]
    fn split_budget_shares_proportionally_with_floor() {
        let s = split_upload_budget(10_000_000, 9_500_000, 500_000);
        // 95 % se recorta al 80 % máximo para garantizar al menos 20 % al otro protocolo.
        assert_eq!(s.ed2k_bps, 8_000_000);
        assert_eq!(s.torrent_bps, 2_000_000);
    }

    #[test]
    fn effective_ceiling_picks_most_restrictive_reason() {
        let d = compute_effective_ceiling(CeilingInputs {
            max_upload_bps: 8_000_000,
            idle_upload_cap_bps: 2_000_000,
            is_idle: true,
            streaming_upload_cap_bps: 500_000,
            streaming_active: true,
            quota_eco_cap_bps: 1_000_000,
        });
        assert_eq!(d.cap_bps, 500_000);
        assert_eq!(d.reason, "streaming");
        assert!(d.idle_cap_active);
        assert!(d.streaming_cap_active);
        assert!(d.quota_eco_active);
    }
}

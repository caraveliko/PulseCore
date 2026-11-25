// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use std::collections::VecDeque;
use std::sync::atomic::Ordering;

use super::*;

impl UploadManager {
    /// Número de ranuras que ocupan actualmente una ranura de subida completa — las ranuras
    /// de goteo (trickle) se excluyen (eMule solo cuenta subidas a velocidad normal).
    pub(super) fn uploading_count(active: &std::collections::HashMap<String, UploadSessionSnapshot>) -> usize {
        active
            .values()
            .filter(|s| s.state == UploadSlotState::Uploading)
            .count()
    }

    /// eMule `CKnownFile::UpdateAutoUpPriority`: prioridad efectiva de un
    /// archivo «auto» según la cantidad de clientes que lo suben
    /// (`GetQueuedCount` — sesiones Uploading + Trickle): más de 20
    /// → baja (6), más de 1 → normal (7), en otro caso → alta (9).
    pub(super) fn compute_auto_up_prio(
        active: &std::collections::HashMap<String, UploadSessionSnapshot>,
        lower_hash: &str,
    ) -> u32 {
        let count = active
            .values()
            .filter(|s| {
                s.file_hash.eq_ignore_ascii_case(lower_hash)
                    && matches!(s.state, UploadSlotState::Uploading | UploadSlotState::Trickle)
            })
            .count();
        if count > 20 {
            6
        } else if count > 1 {
            7
        } else {
            9
        }
    }

    /// Prioridad efectiva de un archivo «auto»: la primera petición
    /// registra el archivo en `auto_up_prio` con el valor calculado
    /// sobre el estado actual (eMule comienza en `PR_HIGH` y recalcula con
    /// cada adición o retirada de subidor).
    pub(super) fn auto_up_prio_for(
        &self,
        active: &std::collections::HashMap<String, UploadSessionSnapshot>,
        file_hash: &str,
    ) -> u32 {
        let key = file_hash.to_lowercase();
        if let Some(&v) = self.auto_up_prio.read().unwrap_or_else(|p| p.into_inner()).get(&key) {
            return v;
        }
        let v = Self::compute_auto_up_prio(active, &key);
        self.auto_up_prio.write().unwrap_or_else(|p| p.into_inner()).insert(key, v);
        v
    }

    /// Recalcula la prioridad efectiva de un archivo auto tras un cambio
    /// en las sesiones activas — no hace nada si el archivo no está en modo auto
    /// (ausente del registro `auto_up_prio`).
    pub(super) fn update_auto_up_prio(
        &self,
        active: &std::collections::HashMap<String, UploadSessionSnapshot>,
        file_hash: &str,
    ) {
        let key = file_hash.to_lowercase();
        if !self.auto_up_prio.read().unwrap_or_else(|p| p.into_inner()).contains_key(&key) {
            return;
        }
        let v = Self::compute_auto_up_prio(active, &key);
        self.auto_up_prio.write().unwrap_or_else(|p| p.into_inner()).insert(key, v);
    }

    /// El archivo vuelve a prioridad manual: sale del registro automático y
    /// reescribe `file_prio_num`/`auto_prio` en las entradas de cola correspondientes
    /// (la prioridad del archivo es una propiedad del archivo compartido, común
    /// a todos los clientes que lo solicitan).
    pub(super) fn clear_auto_file(&self, q: &mut VecDeque<QueuedUpload>, file_hash: &str, manual_num: u32) {
        let key = file_hash.to_lowercase();
        if self.auto_up_prio.write().unwrap_or_else(|p| p.into_inner()).remove(&key).is_none()
            && !q.iter().any(|e| e.auto_prio && e.file_hash.eq_ignore_ascii_case(file_hash))
        {
            return;
        }
        for e in q.iter_mut() {
            if e.auto_prio && e.file_hash.eq_ignore_ascii_case(file_hash) {
                e.auto_prio = false;
                e.file_prio_num = manual_num;
            }
        }
    }

    /// `GetFilePrioAsNumber` efectivo: `"auto"` lee el valor recalculado
    /// (`auto_up_prio`), mientras que el resto de prioridades utilizan el mapeo
    /// estático de eMule.
    pub(super) fn resolve_file_prio_num(
        &self,
        active: &std::collections::HashMap<String, UploadSessionSnapshot>,
        file_hash: &str,
        prio_str: &str,
    ) -> u32 {
        if prio_str.eq_ignore_ascii_case("auto") {
            self.auto_up_prio_for(active, file_hash)
        } else {
            file_prio_num(prio_str)
        }
    }

    /// Ordena la cola por `effective_score` decreciente — eMule compara las
    /// puntuaciones **bajo demanda** (`GetQueueRank`, `FindBestClientInQueue`);
    /// la ordenación reproduce el mismo orden para `pop_front`/posición mostrada.
    /// Las entradas «auto» actualizan primero `file_prio_num` desde
    /// `auto_up_prio`, al igual que eMule lee `GetFilePrioAsNumber()` directamente.
    pub(super) fn sort_queue_by_score(&self, q: &mut VecDeque<QueuedUpload>, now_ms: i64) {
        {
            let auto_map = self.auto_up_prio.read().unwrap_or_else(|p| p.into_inner());
            for e in q.iter_mut() {
                if e.auto_prio {
                    if let Some(&v) = auto_map.get(&e.file_hash.to_lowercase()) {
                        e.file_prio_num = v;
                    }
                }
            }
        }
        let mut entries: Vec<QueuedUpload> = q.drain(..).collect();
        entries.sort_by_key(|a| std::cmp::Reverse(a.effective_score(now_ms)));
        q.extend(entries);
    }

    /// eMule `CUploadQueue::GetTargetClientDataRate`: ≤3 ranuras → 3 KiB/s,
    /// en otro caso `slots × 1 KiB/s` con tope en `UPLOAD_CLIENT_MAXDATARATE`
    /// (25 KiB/s). La variante `min_datarate` devuelve ×3/4.
    pub(super) fn target_client_data_rate_bps(open_slots: usize, min_datarate: bool) -> u64 {
        let r = if open_slots <= 3 {
            3 * 1024
        } else {
            (open_slots as u64 * 1024).min(UPLOAD_CLIENT_MAXDATARATE_BPS)
        };
        if min_datarate {
            r * 3 / 4
        } else {
            r
        }
    }

    /// Tasa objetivo efectiva por ranura: si *Smart Slot Focus* (`target_slot_speed_bps > 0`)
    /// está configurado, concentra el caudal en menos ranuras rápidas; en caso contrario,
    /// aplica la fórmula clásica de eMule 0.72a.
    pub(super) fn effective_target_slot_rate_bps(
        &self,
        open_slots: usize,
        min_datarate: bool,
    ) -> u64 {
        let custom = self.target_slot_speed_bps.load(Ordering::Relaxed);
        if custom == 0 {
            return Self::target_client_data_rate_bps(open_slots, min_datarate);
        }
        if min_datarate {
            (custom * 3 / 4).max(1)
        } else {
            custom
        }
    }

    /// eMule `CUploadQueue::AcceptNewClient`: abre una ranura si
    /// - no hay veto de USS (`lastCommonRouteFinder->AcceptNewClient`),
    /// - `uploading < max(MIN_UP_CLIENTS_ALLOWED, 4)` → siempre,
    /// - `uploading >= MAX_UP_CLIENTS_ALLOWED` → nunca,
    /// - `uploading < datarate / minTargetRate`: la velocidad medida aún
    ///   abastece a cada cliente en el umbral mínimo,
    /// - `uploading < MaxSpeed / targetRate`: la capacidad configurada
    ///   (o medida por USS) admite una ranura adicional a la tasa objetivo.
    ///
    /// Compartido por `request_upload` (nuevo cliente), `promote_waiting`
    /// (promoción) y `process_tick` (rotación) para evitar divergencias.
    pub(super) fn can_open_slot(
        &self,
        active: &std::collections::HashMap<String, UploadSessionSnapshot>,
        now: i64,
    ) -> bool {
        if !self.uss_gate.read().unwrap_or_else(|p| p.into_inner()).load(Ordering::SeqCst) {
            return false;
        }
        let uploading = Self::uploading_count(active);
        // `MAX_UP_CLIENTS_ALLOWED` primero: eMule prueba lo garantizado antes
        // del tope, pero el tope (100) siempre supera lo garantizado
        // (4) — aquí `max_active` es configurable y puede descender de 4.
        if uploading >= self.max_active.load(Ordering::SeqCst) {
            return false;
        }
        if uploading < GUARANTEED_UPLOAD_SLOTS {
            return true;
        }
        let target = self.effective_target_slot_rate_bps(uploading, false).max(1);
        let min_target = self.effective_target_slot_rate_bps(uploading, true).max(1);
        let total_bps: u64 = active
            .values()
            .filter(|s| s.state == UploadSlotState::Uploading)
            .map(|s| s.effective_rate_bps(now))
            .sum();
        // `curUploadSlots >= datarate / minTarget` → rechazo: cada cliente
        // existente debe mantener al menos la velocidad mínima.
        if uploading as u64 >= total_bps / min_target {
            return false;
        }
        // `curUploadSlots >= MaxSpeed*1024 / target` → rechazo: la capacidad
        // de subida configurada no permite una ranura adicional a la velocidad
        // objetivo. 0 = ilimitado → únicamente el término datarate limita el crecimiento
        // (eMule añade un límite interno «graph rate» no modelado aquí).
        let max_speed = self.max_upload_speed_bps();
        if max_speed > 0 && uploading as u64 >= max_speed / target {
            return false;
        }
        // Umbral mínimo opcional `min_upload_kbs` (eMule `MinUpload` alimenta USS,
        // no `AcceptNewClient` — mantenido como salvaguarda adicional de usuario,
        // inactivo de forma predeterminada).
        let min_rate = self.min_upload_rate_bps.load(Ordering::Relaxed);
        !(min_rate > 0 && total_bps / (uploading.max(1) as u64) < min_rate)
    }

    /// Temporización de `ForceNewClient` de eMule (`UploadQueue.cpp:417-421`): `true`
    /// cuando la apertura de una ranura adicional debe esperar — menos de
    /// `PROMOTION_MIN_INTERVAL_MS` desde la última apertura y velocidad
    /// medida < `PROMOTION_UNPACED_BPS`. Por debajo de `MIN_UP_CLIENTS_ALLOWED`
    /// ranuras activas, nunca hay temporización (`return true` anticipado).
    pub(super) fn promotion_paced(
        &self,
        active: &std::collections::HashMap<String, UploadSessionSnapshot>,
        now: i64,
    ) -> bool {
        if Self::uploading_count(active) < MIN_UP_CLIENTS_ALLOWED {
            return false;
        }
        let last = self.last_upload_start_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= PROMOTION_MIN_INTERVAL_MS {
            return false;
        }
        let total_bps: u64 = active
            .values()
            .filter(|s| s.state == UploadSlotState::Uploading)
            .map(|s| s.effective_rate_bps(now))
            .sum();
        total_bps < PROMOTION_UNPACED_BPS
    }

}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Procedencia de los datos recibidos para atribuir la corrupción de una parte al
//! remitente — portado fiel de `CCorruptionBlackBox` de eMule 0.72a
//! (`CorruptionBlackBox.cpp`).
//!
//! Cada bloque recibido se registra por parte con su remitente. Cuando una
//! parte se verifica o se considera corrupta, los registros solapados se
//! marcan como `Verified`/`Corrupted`. `evaluate` reproduce
//! `CCorruptionBlackBox::EvaluateData`: para cada remitente que tenga al
//! menos un registro corrupto en la parte defectuosa, la proporción
//! `corrupto / (corrupto + verificado)` calculada sobre **todas** las partes del
//! archivo se compara con `CBB_BANTHRESHOLD` (32 %) — por encima, eMule
//! expulsa al cliente («Identified as a sender of corrupt data»).

use std::collections::{HashMap, HashSet};

/// eMule `CBB_BANTHRESHOLD` — porcentaje de datos corruptos que activa
/// la expulsión del remitente.
pub const CBB_BANTHRESHOLD_PERCENT: u64 = 32;

/// eMule `EMBLOCKSIZE` — un registro corrupto cuenta siempre como al
/// menos un bloque completo (180 KiB), incluso si el rango corrupto es más pequeño.
const EMBLOCKSIZE: u64 = 184_320;

/// Límite defensivo en los registros conservados por parte: una parte de
/// 9 728 000 bytes cubre ~54 bloques de eMule; las reescrituras multiplican los
/// registros por fragmentación, sin orden de magnitud más allá.
const MAX_RECORDS_PER_PART: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordStatus {
    /// Datos recibidos, aún no evaluados (eMule `BBR_NONE`).
    Received,
    /// Datos cuya parte/bloque ha superado la verificación (eMule `BBR_VERIFIED`).
    Verified,
    /// Datos que pertenecen a un rango corrupto (eMule `BBR_CORRUPTED`).
    Corrupted,
}

#[derive(Debug, Clone)]
struct CbbRecord {
    /// Rango semiabierto `[start, end)` en bytes absolutos del archivo.
    start: u64,
    end: u64,
    /// Dirección del remitente (`ip:port` o `lowid:N`).
    sender: String,
    status: RecordStatus,
}

/// Estado de la caja negra para una descarga (eMule `m_CorruptionBlackBox`).
#[derive(Debug, Default)]
pub struct CorruptionBlackBox {
    /// Registros por índice de parte (eMule `m_aaRecords`).
    records: HashMap<u32, Vec<CbbRecord>>,
}

impl CorruptionBlackBox {
    /// `CCorruptionBlackBox::ReceivedData`: registra la entrega de
    /// `[start, end)` por parte de `sender`. Los registros existentes pierden la
    /// porción reescrita — los bytes reescritos cambian de responsable
    /// (eMule reemplaza las estadísticas de los rangos solapados).
    pub fn received_data(&mut self, part: u32, start: u64, end: u64, sender: &str) {
        if end <= start || sender.is_empty() {
            return;
        }
        let recs = self.records.entry(part).or_default();
        subtract_range(recs, start, end);
        if recs.len() < MAX_RECORDS_PER_PART {
            recs.push(CbbRecord {
                start,
                end,
                sender: sender.to_string(),
                status: RecordStatus::Received,
            });
        }
    }

    /// `CCorruptionBlackBox::VerifiedData`: marca la porción `[start, end)`
    /// de los registros como verificada.
    pub fn verified_data(&mut self, part: u32, start: u64, end: u64) {
        self.mark(part, start, end, RecordStatus::Verified);
    }

    /// `CCorruptionBlackBox::CorruptedData`: marca la porción `[start, end)`
    /// de los registros como corrupta.
    pub fn corrupted_data(&mut self, part: u32, start: u64, end: u64) {
        self.mark(part, start, end, RecordStatus::Corrupted);
    }

    /// `CCorruptionBlackBox::EvaluateData`: devuelve los remitentes cuya
    /// proporción de datos corruptos supera `CBB_BANTHRESHOLD` (32 %).
    ///
    /// Los sospechosos son los remitentes que tienen al menos un registro
    /// corrupto en `part`. Para cada uno, la proporción se calcula sobre **todas**
    /// las partes del archivo: cada registro corrupto cuenta como
    /// `max(longitud, EMBLOCKSIZE)` bytes, cada registro verificado
    /// por su longitud real.
    pub fn evaluate(&self, part: u32) -> Vec<String> {
        let suspects: HashSet<&str> = self
            .records
            .get(&part)
            .map(|recs| {
                recs.iter()
                    .filter(|r| r.status == RecordStatus::Corrupted)
                    .map(|r| r.sender.as_str())
                    .collect()
            })
            .unwrap_or_default();
        if suspects.is_empty() {
            return Vec::new();
        }

        let mut guilty = Vec::new();
        for sender in suspects {
            let mut corrupt_bytes = 0u64;
            let mut verified_bytes = 0u64;
            let mut received_bytes = 0u64;
            for recs in self.records.values() {
                for rec in recs {
                    if rec.sender != sender {
                        continue;
                    }
                    match rec.status {
                        RecordStatus::Corrupted => {
                            corrupt_bytes += (rec.end - rec.start).max(EMBLOCKSIZE)
                        }
                        RecordStatus::Verified => verified_bytes += rec.end - rec.start,
                        RecordStatus::Received => received_bytes += rec.end - rec.start,
                    }
                }
            }
            // Dilución por el conjunto de datos entregados (verificados + recibidos en vuelo)
            let total = corrupt_bytes + verified_bytes + received_bytes;
            // Comparación entera equivalente a `100*corrupt/total > 32`.
            if total > 0 && corrupt_bytes * 100 > total * CBB_BANTHRESHOLD_PERCENT {
                guilty.push(sender.to_string());
            }
        }
        guilty
    }

    /// `m_CorruptionBlackBox.Free()` — liberación al completarse el archivo.
    pub fn clear(&mut self) {
        self.records.clear();
    }

    fn mark(&mut self, part: u32, start: u64, end: u64, status: RecordStatus) {
        if end <= start {
            return;
        }
        let Some(recs) = self.records.get_mut(&part) else {
            return;
        };
        let mut i = 0;
        while i < recs.len() {
            let rec = &recs[i];
            let ov_start = rec.start.max(start);
            let ov_end = rec.end.min(end);
            if ov_end <= ov_start {
                i += 1;
                continue;
            }
            // Divide el registro en [antes] [solapado] [después]:
            // solo la porción solapada cambia de estado (eMule Split) —
            // los fragmentos externos conservan el estado original.
            let before = (rec.start < ov_start).then_some((rec.start, ov_start));
            let after = (rec.end > ov_end).then_some((ov_end, rec.end));
            let mut rec = recs.remove(i);
            let old_status = rec.status;
            rec.start = ov_start;
            rec.end = ov_end;
            rec.status = status;
            if let Some((bs, be)) = before {
                recs.insert(
                    i,
                    CbbRecord {
                        start: bs,
                        end: be,
                        sender: rec.sender.clone(),
                        status: old_status,
                    },
                );
                i += 1;
            }
            recs.insert(i, rec);
            i += 1;
            if let Some((as_, ae)) = after {
                if recs.len() < MAX_RECORDS_PER_PART {
                    recs.insert(
                        i,
                        CbbRecord {
                            start: as_,
                            end: ae,
                            sender: recs[i - 1].sender.clone(),
                            status: old_status,
                        },
                    );
                }
                i += 1;
            }
        }
    }
}

/// Elimina la porción `[start, end)` de cada registro (eMule
/// `ReceivedData`: los bytes reescritos pierden sus estadísticas antiguas).
fn subtract_range(recs: &mut Vec<CbbRecord>, start: u64, end: u64) {
    let mut i = 0;
    while i < recs.len() {
        let rec = &recs[i];
        let ov_start = rec.start.max(start);
        let ov_end = rec.end.min(end);
        if ov_end <= ov_start {
            i += 1;
            continue;
        }
        let before = (rec.start < ov_start).then_some((rec.start, ov_start));
        let after = (rec.end > ov_end).then_some((ov_end, rec.end));
        let rec = recs.remove(i);
        match (before, after) {
            (Some((bs, be)), Some((as_, ae))) => {
                recs.insert(
                    i,
                    CbbRecord {
                        start: as_,
                        end: ae,
                        sender: rec.sender.clone(),
                        status: rec.status,
                    },
                );
                recs.insert(
                    i,
                    CbbRecord {
                        start: bs,
                        end: be,
                        sender: rec.sender,
                        status: rec.status,
                    },
                );
                i += 2;
            }
            (Some((bs, be)), None) => {
                recs.insert(
                    i,
                    CbbRecord {
                        start: bs,
                        end: be,
                        sender: rec.sender,
                        status: rec.status,
                    },
                );
                i += 1;
            }
            (None, Some((as_, ae))) => {
                recs.insert(
                    i,
                    CbbRecord {
                        start: as_,
                        end: ae,
                        sender: rec.sender,
                        status: rec.status,
                    },
                );
                i += 1;
            }
            (None, None) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// eMule `EvaluateData`: un remitente cuya entrega completa está
    /// corrupta supera el 32 % — expulsado.
    #[test]
    fn single_sender_fully_corrupt_is_guilty() {
        let mut bb = CorruptionBlackBox::default();
        bb.received_data(0, 0, 9_728_000, "1.2.3.4:49662");
        bb.corrupted_data(0, 0, 9_728_000);
        assert_eq!(bb.evaluate(0), vec!["1.2.3.4:49662".to_string()]);
    }

    /// Un remitente sospechoso solo se expulsa si `corrupto/(corrupto+verificado)`
    /// supera el 32 % — la dilución por datos verificados lo exonera.
    #[test]
    fn verified_data_dilutes_corruption_ratio() {
        let mut bb = CorruptionBlackBox::default();
        // 3 partes verificadas (~29,2 MiB) + una parte corrupta (~9,7 MiB)
        // → proporción ≈ 25 % < 32 %: sospechoso pero no expulsado.
        for part in 0..3 {
            bb.received_data(part, part as u64 * 9_728_000, (part as u64 + 1) * 9_728_000, "1.2.3.4:49662");
            bb.verified_data(part, part as u64 * 9_728_000, (part as u64 + 1) * 9_728_000);
        }
        bb.received_data(3, 3 * 9_728_000, 4 * 9_728_000, "1.2.3.4:49662");
        bb.corrupted_data(3, 3 * 9_728_000, 4 * 9_728_000);
        assert!(bb.evaluate(3).is_empty());

        // Con 2 partes verificadas solamente: ~33 % > 32 % → expulsado.
        let mut bb2 = CorruptionBlackBox::default();
        for part in 0..2 {
            bb2.received_data(part, part as u64 * 9_728_000, (part as u64 + 1) * 9_728_000, "1.2.3.4:49662");
            bb2.verified_data(part, part as u64 * 9_728_000, (part as u64 + 1) * 9_728_000);
        }
        bb2.received_data(2, 2 * 9_728_000, 3 * 9_728_000, "1.2.3.4:49662");
        bb2.corrupted_data(2, 2 * 9_728_000, 3 * 9_728_000);
        assert_eq!(bb2.evaluate(2), vec!["1.2.3.4:49662".to_string()]);
    }

    /// `ReceivedData` reemplaza la procedencia de los bytes reescritos: el
    /// nuevo remitente hereda la responsabilidad del rango solapado.
    #[test]
    fn rewritten_range_changes_attribution() {
        let mut bb = CorruptionBlackBox::default();
        bb.received_data(0, 0, 9_728_000, "bad:1");
        // El rango [100, 200) es reentregado por otro par.
        bb.received_data(0, 100, 200, "good:1");
        // Corrupción limitada al rango reescrito → solo "good" es sospechoso.
        bb.corrupted_data(0, 100, 200);
        let guilty = bb.evaluate(0);
        assert_eq!(guilty, vec!["good:1".to_string()]);
    }

    /// Un registro corrupto cuenta como al menos `EMBLOCKSIZE`:
    /// un rango corrupto muy pequeño basta para expulsar a un remitente
    /// que casi no ha enviado nada verificado.
    #[test]
    fn corrupted_record_counts_at_least_block_size() {
        let mut bb = CorruptionBlackBox::default();
        bb.received_data(0, 0, 1000, "1.2.3.4:49662");
        bb.received_data(0, 1000, 2000, "1.2.3.4:49662");
        bb.verified_data(0, 0, 1000);
        bb.corrupted_data(0, 1000, 2000);
        // corrupto contabilizado = 184 320 (no 1000) → 100 % > 32 %.
        assert_eq!(bb.evaluate(0), vec!["1.2.3.4:49662".to_string()]);
    }

    /// `evaluate` solo incrimina a los remitentes que hayan entregado datos
    /// corruptos en LA parte evaluada.
    #[test]
    fn only_suspects_of_failed_part_are_evaluated() {
        let mut bb = CorruptionBlackBox::default();
        bb.received_data(0, 0, 9_728_000, "innocent:1");
        bb.corrupted_data(0, 0, 9_728_000);
        // "innocent" es el único sospechoso → expulsado; un remitente de otra
        // parte no entra en la evaluación salvo que también sea sospechoso.
        bb.received_data(1, 9_728_000, 2 * 9_728_000, "other:1");
        let guilty = bb.evaluate(0);
        assert_eq!(guilty, vec!["innocent:1".to_string()]);
    }
}

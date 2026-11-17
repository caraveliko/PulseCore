// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! A4AF a nivel de socket — `CUpDownClient::SwapToAnotherFile`/`DoSwap` de eMule
//! (`DownloadClient.cpp:1530`).
//!
//! Cuando una sesión informa de *No Needed Parts* o *File Not Found*, eMule no
//! cierra la conexión TCP: `DoSwap` mueve el cliente a la lista de fuentes de otro
//! `CPartFile` y el mismo socket vuelve a emitir `OP_SETREQFILEID`/estado de archivo
//! para el nuevo archivo. Este módulo elige el objetivo del intercambio y realiza
//! la contabilidad asociada; el bucle de sesión en `connection.rs` reutiliza el
//! flujo activo para la nueva solicitud.

use std::sync::Arc;
use tokio::sync::Mutex;

use mule_core::model::{PeerFailureReason, PeerSourceStatus};

use crate::services::downloads::DownloadService;

/// Intercambios consecutivos máximos en un socket — eMule no tiene un límite explícito
/// (el tiempo de repetición de petición previene el ping-pong); limitamos para evitar una cadena infinita de
/// archivos NNP para un par patológico.
const MAX_A4AF_SWAPS: u32 = 4;

/// Tiempo de enfriamiento para repetir petición NNP — `FILEREASKTIME * 2` de eMule = 58 min
/// (`DownloadClient.cpp:1888`, `PartFile.cpp:2276`).
const NNP_REASK_COOLDOWN_MS: i64 = 58 * 60 * 1000;

/// Intercambios máximos por sesión (`MAX_A4AF_SWAPS`).
pub(crate) fn max_swaps() -> u32 {
    MAX_A4AF_SWAPS
}

/// Motivos de fallo que mantienen el socket activo y son aptos para un intercambio:
/// eMule llama a `SwapToAnotherFile` en NNP (`ProcessFileStatus`,
/// `SendBlockRequests`) y FNF (`UDPReaskFNF`, `OP_FILEREQANSNOFILE` mediante
/// `ListenSocket.cpp:428`). Cualquier otro error significa que la conexión está muerta —
/// nada sobre lo que intercambiar.
pub(crate) fn is_swap_eligible(reason: &PeerFailureReason) -> bool {
    matches!(
        reason,
        PeerFailureReason::NoNeededParts | PeerFailureReason::FileNotFound
    )
}

/// Elige el archivo al que una sesión debe intercambiar (escaneo de candidatos de
/// `SwapToAnotherFile` de eMule sobre `m_OtherRequests_list` y luego `m_OtherNoNeeded_list`,
/// con ordenación `SwapToRightFile` reducida a: mayor prioridad de descarga,
/// y luego orden determinista por hash).
///
/// Un archivo candidato debe ser una descarga activa (`IsPartFile`, `!IsStopped`,
/// `PS_READY || PS_EMPTY` de eMule), debe listar `address` como fuente
/// (A4AFsrclist de eMule), y no debe superar ya su límite de fuentes
/// (`cur_file->GetSourceCount() < GetMaxSources()`). Las fuentes marcadas como
/// NNP/FNF *en el archivo candidato* solo se eligen cuando no existe ningún candidato limpio
/// (nivel OtherNoNeeded de eMule).
pub(crate) async fn find_swap_target(
    downloads: &Arc<Mutex<DownloadService>>,
    address: &str,
    current_hash: &str,
    max_peers_per_file: usize,
    now_ms: i64,
) -> Option<(String, [u8; 16])> {
    struct Cand {
        hash: String,
        prio: u8,
        nnp: bool,
    }

    let mut cands: Vec<Cand> = {
        let dl = downloads.lock().await;
        let mut v = Vec::new();
        for d in dl.list_downloads() {
            if d.ed2k_hash.is_empty()
                || d.ed2k_hash.eq_ignore_ascii_case(current_hash)
                || !matches!(
                    d.status.as_str(),
                    "queued" | "waiting_for_sources" | "downloading"
                )
            {
                continue;
            }
            let cap = d
                .max_sources
                .map(|m| m as usize)
                .unwrap_or(max_peers_per_file);
            if d.source_count as usize >= cap {
                continue;
            }
            let Ok(srcs) = dl.list_peer_sources(&d.ed2k_hash) else {
                continue;
            };
            let Some(src) = srcs.iter().find(|s| s.address == address) else {
                continue;
            };
            let nnp_flag = src
                .last_error
                .as_deref()
                .map(|e| {
                    let e = e.to_lowercase();
                    e.contains("no needed parts") || e.contains("file not found")
                })
                .unwrap_or(false);
            // El enfriamiento se aplica a todos los estados — una fuente marcada como NNP
            // (`Allowed` + enfriamiento de 58 min) no debe volver a elegirse dentro de la
            // misma cadena de sockets (`DontSwapTo`/`GetTimeUntilReask` de eMule).
            if src.cooldown_until_unix_ms.is_some_and(|c| c > now_ms) {
                continue;
            }
            let viable = match src.status {
                PeerSourceStatus::Candidate
                | PeerSourceStatus::Allowed
                | PeerSourceStatus::Tried
                | PeerSourceStatus::Connected
                | PeerSourceStatus::Failed => true,
                PeerSourceStatus::Rejected | PeerSourceStatus::Filtered => false,
            };
            if !viable {
                continue;
            }
            let prio = match d.priority.as_str() {
                "high" => 2u8,
                "normal" => 1,
                _ => 0,
            };
            v.push(Cand {
                hash: d.ed2k_hash.clone(),
                prio,
                nnp: nnp_flag,
            });
        }
        v
    };

    // eMule: OtherRequests (fuentes limpias) antes de OtherNoNeeded (marcadas como NNP
    // en ese archivo), `RightFileHasHigherPrio` = prioridad de descarga primero.
    cands.sort_by(|a, b| {
        (a.nnp as u8)
            .cmp(&(b.nnp as u8))
            .then(b.prio.cmp(&a.prio))
            .then(a.hash.cmp(&b.hash))
    });
    let cand = cands.into_iter().next()?;
    let mut hash = [0u8; 16];
    let raw = hex::decode(&cand.hash).ok()?;
    if raw.len() != 16 {
        return None;
    }
    hash.copy_from_slice(&raw);
    Some((cand.hash, hash))
}

/// Registra la contabilidad del intercambio antes de que la sesión vuelva a pedir otro archivo en
/// el mismo socket (`DoSwap` de eMule: el cliente abandona la lista de fuentes del archivo antiguo —
/// lista muerta para FNF, enfriamiento NNP — y se une a la lista del nuevo archivo).
pub(crate) async fn apply_swap(
    downloads: &Arc<Mutex<DownloadService>>,
    address: &str,
    from_hash: &str,
    to_hash: &str,
    reason: &PeerFailureReason,
    now_ms: i64,
) {
    let mut dl = downloads.lock().await;
    match reason {
        PeerFailureReason::NoNeededParts => {
            let _ = dl.update_peer_source_status(
                from_hash,
                address,
                PeerSourceStatus::Allowed,
                Some("no needed parts (A4AF swap)"),
                Some(now_ms + NNP_REASK_COOLDOWN_MS),
            );
        }
        PeerFailureReason::FileNotFound => {
            let _ = dl.update_peer_source_status(
                from_hash,
                address,
                PeerSourceStatus::Rejected,
                Some("file not found (A4AF swap)"),
                None,
            );
        }
        _ => {}
    }
    // La fuente ahora se sirve activamente para `to_hash` en el socket en vivo:
    // marcarla como conectada y limpiar cualquier enfriamiento restante para que el coordinador
    // la cuente como fuente activa (`SwapTo->srclist.AddTail(this)` de eMule).
    let _ = dl.update_peer_source_status(to_hash, address, PeerSourceStatus::Connected, None, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use mule_test_support::peer_fixtures::peer_source;

    #[test]
    fn swap_eligibility() {
        assert!(is_swap_eligible(&PeerFailureReason::NoNeededParts));
        assert!(is_swap_eligible(&PeerFailureReason::FileNotFound));
        assert!(!is_swap_eligible(&PeerFailureReason::TimeoutQueue));
        assert!(!is_swap_eligible(&PeerFailureReason::ConnectRefused));
        assert!(!is_swap_eligible(&PeerFailureReason::CorruptedData));
    }

    fn hash_hex(byte: u8) -> String {
        hex::encode([byte; 16])
    }

    fn svc_with_two_files(hash_a: &str, hash_b: &str, tmp: &tempfile::TempDir) -> DownloadService {
        let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();
        for (name, h) in [("file_a.bin", hash_a), ("file_b.bin", hash_b)] {
            let link = format!("ed2k://|file|{name}|10000000|{h}|/");
            svc.add_ed2k_link(
                &link,
                tmp.path(),
                mule_core::storage::part_file::AllocationPolicy::None,
                None,
            )
            .unwrap();
        }
        svc
    }

    #[tokio::test]
    async fn swap_target_picks_other_file_source() {
        let tmp = tempfile::tempdir().unwrap();
        let (ha, hb) = (hash_hex(0x11), hash_hex(0x22));
        let mut svc = svc_with_two_files(&ha, &hb, &tmp);
        let addr = "1.2.3.4:49662";
        svc.upsert_peer_sources(&hb, &[peer_source(&hb, addr)])
            .unwrap();
        let svc = Arc::new(Mutex::new(svc));

        let got = find_swap_target(&svc, addr, &ha, 100, 0).await;
        assert_eq!(got.map(|(h, _)| h), Some(hb.clone()));
        // No se ofrece para el archivo al que ya sirve.
        assert!(find_swap_target(&svc, addr, &hb, 100, 0).await.is_none());
        // Dirección desconocida -> nada.
        assert!(find_swap_target(&svc, "9.9.9.9:1", &ha, 100, 0)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn swap_target_rejects_rejected_and_cooled_sources() {
        let tmp = tempfile::tempdir().unwrap();
        let (ha, hb) = (hash_hex(0x33), hash_hex(0x44));
        let mut svc = svc_with_two_files(&ha, &hb, &tmp);
        let addr = "5.6.7.8:49662";
        let mut src = peer_source(&hb, addr);
        src.status = PeerSourceStatus::Rejected;
        svc.upsert_peer_sources(&hb, &[src]).unwrap();
        let svc = Arc::new(Mutex::new(svc));
        assert!(find_swap_target(&svc, addr, &ha, 100, 0).await.is_none());

        // Con fallo y enfriamiento activo se omite; con enfriamiento expirado es viable.
        {
            let mut dl = svc.lock().await;
            dl.update_peer_source_status(
                &hb,
                addr,
                PeerSourceStatus::Failed,
                Some("dial failed"),
                Some(1000),
            )
            .unwrap();
        }
        assert!(find_swap_target(&svc, addr, &ha, 100, 500).await.is_none());
        assert_eq!(
            find_swap_target(&svc, addr, &ha, 100, 2000)
                .await
                .map(|(h, _)| h),
            Some(hb)
        );
    }

    #[tokio::test]
    async fn swap_target_prefers_clean_over_nnp() {
        let tmp = tempfile::tempdir().unwrap();
        let (ha, hb, hc) = (hash_hex(0x55), hash_hex(0x66), hash_hex(0x77));
        let mut svc = svc_with_two_files(&ha, &hb, &tmp);
        // Tercera descarga cuyo registro de fuente está marcado como NNP.
        let link_c = format!("ed2k://|file|file_c.bin|10000000|{hc}|/");
        svc.add_ed2k_link(
            &link_c,
            tmp.path(),
            mule_core::storage::part_file::AllocationPolicy::None,
            None,
        )
        .unwrap();
        let addr = "8.8.8.8:49662";
        let mut nnp_src = peer_source(&hb, addr);
        nnp_src.status = PeerSourceStatus::Allowed;
        nnp_src.last_error = Some("no needed parts".to_string());
        svc.upsert_peer_sources(&hb, &[nnp_src]).unwrap();
        svc.upsert_peer_sources(&hc, &[peer_source(&hc, addr)])
            .unwrap();
        let svc = Arc::new(Mutex::new(svc));
        // El candidato limpio (hc) se impone al marcado como NNP (hb).
        assert_eq!(
            find_swap_target(&svc, addr, &ha, 100, 0)
                .await
                .map(|(h, _)| h),
            Some(hc)
        );
    }

    #[tokio::test]
    async fn apply_swap_writes_deadlist_and_live_states() {
        let tmp = tempfile::tempdir().unwrap();
        let (ha, hb) = (hash_hex(0x88), hash_hex(0x99));
        let mut svc = svc_with_two_files(&ha, &hb, &tmp);
        let addr = "7.7.7.7:49662";
        svc.upsert_peer_sources(&ha, &[peer_source(&ha, addr)])
            .unwrap();
        svc.upsert_peer_sources(&hb, &[peer_source(&hb, addr)])
            .unwrap();
        let svc = Arc::new(Mutex::new(svc));

        apply_swap(
            &svc,
            addr,
            &ha,
            &hb,
            &PeerFailureReason::NoNeededParts,
            1000,
        )
        .await;
        let dl = svc.lock().await;
        let a = dl
            .list_peer_sources(&ha)
            .unwrap()
            .into_iter()
            .find(|s| s.address == addr)
            .unwrap();
        assert_eq!(a.status, PeerSourceStatus::Allowed);
        assert_eq!(a.cooldown_until_unix_ms, Some(1000 + NNP_REASK_COOLDOWN_MS));
        let b = dl
            .list_peer_sources(&hb)
            .unwrap()
            .into_iter()
            .find(|s| s.address == addr)
            .unwrap();
        assert_eq!(b.status, PeerSourceStatus::Connected);
        drop(dl);

        apply_swap(&svc, addr, &ha, &hb, &PeerFailureReason::FileNotFound, 2000).await;
        let dl = svc.lock().await;
        let a = dl
            .list_peer_sources(&ha)
            .unwrap()
            .into_iter()
            .find(|s| s.address == addr)
            .unwrap();
        assert_eq!(a.status, PeerSourceStatus::Rejected);
    }
}

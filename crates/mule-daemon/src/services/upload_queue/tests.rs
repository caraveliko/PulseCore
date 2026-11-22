// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use super::*;

/// Rebobina `m_nLastStartUpload` entre dos aperturas de ranura — las
/// pruebas evalúan las condiciones de admisión sin sufrir la espera de 1 s de
/// `ForceNewClient` (comportamiento de eMule probado por separado).
fn unpaced(mgr: &UploadManager) {
    mgr.last_upload_start_ms.store(0, Ordering::Relaxed);
}

/// eMule `ForceNewClient` (`UploadQueue.cpp:417-421`): por debajo de
/// `MIN_UP_CLIENTS_ALLOWED` nunca hay espera; por encima, 1 s mínimo
/// entre dos aperturas mientras la velocidad medida sea < 100 KB/s;
/// por encima de esa cifra, sin espaciamiento.
#[tokio::test]
async fn force_new_client_paces_slot_opens_below_100kbs() {
    let mgr = UploadManager::new();
    // Las dos primeras ranuras se conceden sin temporización
    // (`curUploadSlots < MIN_UP_CLIENTS_ALLOWED → true`).
    for i in 0..2 {
        let (state, _) = mgr
            .request_upload_slot(&format!("c{i}"), &format!("c{i}"), "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Uploading);
    }
    // 3.ª apertura < 1 s tras la anterior, velocidad 0 < 100 KB/s → cola.
    let (state, _) = mgr
        .request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Waiting);
    // Velocidad medida ≥ 100 KB/s → sin espaciamiento: el cliente en cola
    // es promocionado de inmediato.
    {
        let mut active = mgr.active_uploads.lock().await;
        for s in active.values_mut() {
            s.upload_rate_bps = 60 * 1024;
        }
    }
    let (state, _) = mgr
        .request_upload_slot("c3", "c3", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Uploading);
    // Intervalo transcurrido (> 1 s simulado) → apertura incluso por debajo de 100 KB/s.
    mgr.last_upload_start_ms.store(0, Ordering::Relaxed);
    {
        let mut active = mgr.active_uploads.lock().await;
        for s in active.values_mut() {
            s.upload_rate_bps = 0;
        }
    }
    let (state, _) = mgr
        .request_upload_slot("c4", "c4", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Uploading);
}

#[tokio::test]
async fn multiple_active_slots_up_to_limit() {
    let mgr = UploadManager::new();
    mgr.set_max_active(3);
    for i in 0..3 {
        unpaced(&mgr);
        let (state, _) = mgr
            .request_upload_slot(
                &format!("c{i}"),
                &format!("c{i}"),
                "h",
                "f",
                1.0,
                "normal",
                false,
                None,
            )
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Uploading);
    }
    // El 4.º cliente entra en cola
    let (state, rank) = mgr
        .request_upload_slot("c3", "c3", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Waiting);
    assert_eq!(rank, Some(1));
    assert_eq!(mgr.active_snapshots().await.len(), 3);
}

#[tokio::test]
async fn release_promotes_until_slots_full() {
    let mgr = UploadManager::new();
    mgr.set_max_active(2);
    mgr.request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.request_upload_slot("b", "b", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.request_upload_slot("c", "c", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.request_upload_slot("d", "d", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();

    mgr.release_slot("a").await;
    let actives = mgr.active_snapshots().await;
    assert_eq!(actives.len(), 2);
    assert!(actives.iter().any(|s| s.client_address == "c"));
    assert_eq!(mgr.list_queue().await.len(), 1);

    mgr.release_slot("b").await;
    assert!(mgr
        .active_snapshots()
        .await
        .iter()
        .any(|s| s.client_address == "d"));
    assert!(mgr.list_queue().await.is_empty());
}

#[tokio::test]
async fn userhash_key_dedups_across_ips_and_release_by_address_works() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    // El mismo userhash reconectando desde una IP diferente representa al mismo cliente.
    mgr.request_upload_slot("uh:AA", "1.1.1.1:49662", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    let (state, _) = mgr
        .request_upload_slot("uh:AA", "2.2.2.2:49662", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Uploading);
    // release_slot mediante ip:port simple se resuelve a través de client_address.
    mgr.release_slot("1.1.1.1:49662").await;
    assert_eq!(mgr.active_len(), 0);
}

#[tokio::test]
async fn slow_slot_demoted_to_trickle_and_promotes_queue() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    // c1 ocupa la única ranura.
    let (state, _) = mgr
        .request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Uploading);
    // c2 espera en cola.
    let (state, _) = mgr
        .request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Waiting);
    // c1 permanece lento tras el periodo de gracia → trickle → c2 promocionado.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let mut active = mgr.active_uploads.lock().await;
        let s = active.get_mut("c1").unwrap();
        s.started_at_unix_ms -= TRICKLE_GRACE_MS + 1;
        // Ventana de medición iniciada hace 1,5 s: la primera
        // muestra (≥ 500 ms) mide ~341 B/s < TRICKLE_RATE_BPS.
        s.rate_window_start_ms = now_ms - 1_500;
    }
    mgr.update_progress("c1", 512).await;
    let actives = mgr.active_snapshots().await;
    assert_eq!(
        actives.iter().find(|s| s.client_key == "c1").unwrap().state,
        UploadSlotState::Trickle
    );
    assert_eq!(
        actives.iter().find(|s| s.client_key == "c2").unwrap().state,
        UploadSlotState::Uploading
    );
    assert_eq!(mgr.active_len(), 1, "trickle does not occupy a full slot");
}

#[tokio::test]
async fn per_file_stats_aggregate_sessions_and_persist_totals() {
    let mgr = UploadManager::new();
    // Carga al arrancar: el archivo ya tiene 1000 B subidos.
    mgr.set_file_upload_totals(std::collections::HashMap::from([(
        "haa".to_string(),
        1000u64,
    )]));
    // Dos ranuras activas sobre el mismo hash (mayúsculas/minúsculas mixtas) + otro archivo.
    mgr.request_upload_slot("c1", "c1", "HAA", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.request_upload_slot("c2", "c2", "haa", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    unpaced(&mgr);
    mgr.request_upload_slot("c3", "c3", "hbb", "g", 1.0, "normal", false, None)
        .await
        .unwrap();
    // Ventana de medición iniciada hace 1 s → primera muestra en el
    // próximo update_progress (transcurrido ≥ RATE_FIRST_SAMPLE_MS).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let mut active = mgr.active_uploads.lock().await;
        for s in active.values_mut() {
            s.rate_window_start_ms = now - 1_000;
        }
    }
    mgr.update_progress("c1", 1500).await;
    mgr.update_progress("c2", 1500).await;
    mgr.update_progress("c3", 1500).await;

    let totals = mgr.per_file_uploaded_bytes();
    assert_eq!(totals["haa"], 4000, "hydraté 1000 + 1500 + 1500");
    assert_eq!(totals["hbb"], 1500);
    assert!(!totals.contains_key("hcc"));

    let rates = mgr.per_file_rates_bps().await;
    // ~1500 B/s por ranura → ~3000 B/s agregados para haa, ~1500 para hbb.
    let haa = rates["haa"];
    let hbb = rates["hbb"];
    assert!(haa > hbb, "deux slots agrégés ({haa}) > un slot ({hbb})");
    assert!(haa > 0 && hbb > 0);
    assert!(!rates.contains_key("hcc"));

    // Ranuras liberadas: la tasa vuelve a descender, manteniéndose el total persistido.
    mgr.release_slot("c1").await;
    mgr.release_slot("c2").await;
    assert_eq!(
        mgr.per_file_rates_bps()
            .await
            .get("haa")
            .copied()
            .unwrap_or(0),
        0
    );
    assert_eq!(mgr.per_file_uploaded_bytes()["haa"], 4000);
}

#[tokio::test]
async fn fast_slot_within_grace_is_not_demoted() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    // Tasa lenta pero dentro del periodo de gracia → permanece en Uploading.
    mgr.update_progress("c1", 512).await;
    let s = &mgr.active_snapshots().await[0];
    assert_eq!(s.state, UploadSlotState::Uploading);
}

#[tokio::test]
async fn uss_gate_keeps_new_clients_queued() {
    let mgr = UploadManager::new();
    let flag = Arc::new(AtomicBool::new(false));
    mgr.set_uss_gate(flag.clone());
    // Compuerta cerrada: existen ranuras libres pero el cliente debe esperar en cola.
    let (state, rank) = mgr
        .request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Waiting);
    assert_eq!(rank, Some(1));
    assert_eq!(mgr.active_len(), 0);
    // Compuerta abierta: la siguiente petición se concede de inmediato.
    flag.store(true, Ordering::SeqCst);
    let (state, _) = mgr
        .request_upload_slot("b", "b", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Uploading);
}

#[tokio::test]
async fn events_emitted_on_grant_queue_release() {
    let mgr = UploadManager::new();
    let (tx, mut rx) = broadcast::channel(16);
    mgr.set_event_channel(tx, Arc::new(AtomicU64::new(1)));

    mgr.set_max_active(1);
    mgr.request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    let ev = rx.recv().await.unwrap();
    assert!(ev.contains("upload.uploadsUpdated"));
    assert!(ev.contains("\"uploadRateBps\""));

    mgr.request_upload_slot("b", "b", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    let ev = rx.recv().await.unwrap();
    assert!(ev.contains("upload.uploadsUpdated"));
    assert!(ev.contains("queue"));

    mgr.release_slot("a").await;
    let ev = rx.recv().await.unwrap();
    assert!(ev.contains("upload.uploadsUpdated"));
}

#[tokio::test]
async fn progress_tracked_per_client() {
    let mgr = UploadManager::new();
    mgr.request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.request_upload_slot("b", "b", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.update_progress("a", 100).await;
    mgr.update_progress("b", 200).await;
    let actives = mgr.active_snapshots().await;
    assert_eq!(
        actives
            .iter()
            .find(|s| s.client_address == "a")
            .unwrap()
            .transferred_bytes,
        100
    );
    assert_eq!(
        actives
            .iter()
            .find(|s| s.client_address == "b")
            .unwrap()
            .transferred_bytes,
        200
    );
}

#[tokio::test]
async fn rate_is_measured_per_window_and_decays_on_stall() {
    let mgr = UploadManager::new();
    mgr.request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    let now_ms = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    };
    // Ventana iniciada hace 1 s: 200 KiB enviados → primera muestra
    // (≥ 500 ms) ≈ 200 KiB/s — la tasa medida refleja los bytes
    // realmente servidos, no el límite configurado.
    {
        let mut active = mgr.active_uploads.lock().await;
        active.get_mut("a").unwrap().rate_window_start_ms = now_ms() - 1_000;
    }
    mgr.update_progress("a", 200 * 1024).await;
    let snaps = mgr.active_snapshots().await;
    let rate = snaps[0].upload_rate_bps;
    assert!(
        (150 * 1024..=250 * 1024).contains(&rate),
        "measured rate should be ~200 KiB/s, got {rate}"
    );
    // Par inactivo desde hace > 2 s: la velocidad efectiva cae a 0 en vez de
    // quedarse congelada en el último valor.
    assert_eq!(snaps[0].effective_rate_bps(now_ms() + 10_000), 0);
}

/// eMule `AcceptNewClient`: `curUploadSlots >= MaxSpeed*1024/targetRate`
/// → rechazo. La cantidad de ranuras sigue la capacidad de subida configurada,
/// no un tope fijo de 100.
#[tokio::test]
async fn accept_new_client_caps_slots_by_configured_upload_speed() {
    let mgr = UploadManager::new();
    let speed = Arc::new(AtomicU64::new(12 * 1024));
    let speed_probe = Arc::clone(&speed);
    mgr.set_upload_speed_probe(move || speed_probe.load(Ordering::Relaxed));

    // Las 4 ranuras garantizadas se conceden siempre (max(MIN_UP_CLIENTS,4)).
    for i in 0..4 {
        unpaced(&mgr);
        let (state, _) = mgr
            .request_upload_slot(&format!("c{i}"), &format!("c{i}"), "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Uploading);
    }
    // 5.ª: 12 KiB/s / 4 KiB/s objetivo = 3 < 4 ranuras → rechazo → cola.
    let (state, _) = mgr
        .request_upload_slot("c5", "c5", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Waiting);

    // Velocidad medida real (40 KiB/s): el término `datarate/minTarget`
    // pasaría (40/3 ≈ 13 > 4) pero `MaxSpeed/target` sigue denegándolo.
    {
        let mut active = mgr.active_uploads.lock().await;
        for s in active.values_mut() {
            s.upload_rate_bps = 10 * 1024;
        }
    }
    let (state, _) = mgr
        .request_upload_slot("c6", "c6", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Waiting);

    // Límite aumentado a 200 KiB/s → 200/4 = 50 > 4 → ranura concedida.
    speed.store(200 * 1024, Ordering::Relaxed);
    unpaced(&mgr);
    let (state, _) = mgr
        .request_upload_slot("c7", "c7", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Uploading);
}

/// eMule `IsDownloading` → `OP_ACCEPTUPLOADREQ` + `SetUploadFileID`:
/// un cliente que posee una ranura conserva dicha ranura al solicitar
/// otro archivo — nunca se duplica la entrada en cola.
#[tokio::test]
async fn downloading_client_other_file_keeps_slot() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.request_upload_slot("c1", "c1", "h1", "f1", 1.0, "normal", false, None)
        .await
        .unwrap();
    let (state, _) = mgr
        .request_upload_slot("c1", "c1", "h2", "f2", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Uploading);
    let snaps = mgr.active_snapshots().await;
    assert_eq!(snaps.len(), 1);
    assert_eq!(snaps[0].file_hash, "h2");
    assert_eq!(mgr.waiting_len().await, 0);
}

/// eMule: un objeto cliente = una entrada en cola. El re-ask para
/// otro archivo actualiza la entrada existente en vez de crear una nueva.
#[tokio::test]
async fn queued_client_single_entry_and_updates_on_reask() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.request_upload_slot("c2", "c2", "h1", "f1", 1.0, "normal", false, None)
        .await
        .unwrap();
    let (state, _) = mgr
        .request_upload_slot("c2", "c2", "h2", "f2", 1.0, "high", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Waiting);
    assert_eq!(mgr.waiting_len().await, 1);
    let q = mgr.queue.lock().await;
    assert_eq!(q[0].file_hash, "h2");
    assert_eq!(q[0].file_prio_num, 9);
}

/// eMule `CheckForTimeOver` (`FullChunkTransfers`, predeterminado): una sesión
/// por encima de `SESSIONMAXTRANS` es devuelta a la cola cuando no se puede
/// abrir ninguna ranura — el inicio de espera (`credits`) sobrevive a la rotación.
#[tokio::test]
async fn process_tick_rotates_full_session_and_preserves_wait() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let mut active = mgr.active_uploads.lock().await;
        let s = active.get_mut("c1").unwrap();
        s.transferred_bytes = SESSION_MAX_TRANS_BYTES;
        s.queued_since_ms = now - 60_000; // wait start d'origine
    }
    mgr.process_tick().await;
    // c1 desalojado → c2 promocionado. c1 solo se reincorpora a la cola en su próximo
    // re-ask (comportamiento de eMule: RemoveFromUploadQueue → US_NONE).
    let snaps = mgr.active_snapshots().await;
    assert_eq!(snaps.len(), 1);
    assert_eq!(snaps[0].client_key, "c2");
    assert_eq!(mgr.waiting_len().await, 0);

    // Re-ask de c1: inicio de espera restaurado → enqueued_at anterior a now,
    // conservando la puntuación acumulada.
    let (state, rank) = mgr
        .request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Waiting);
    assert_eq!(rank, Some(1));
    let q = mgr.queue.lock().await;
    assert_eq!(
        q[0].enqueued_at_unix_ms,
        now - 60_000,
        "wait start d'origine restauré après rotation"
    );
}

/// `SendOutOfPartReqsAndAddToWaitingQueue` de eMule — tras el desalojo por
/// `CheckForTimeOver`, se notifica al despachador suscrito mediante `rotation_notify`
/// para emitir `OP_OUTOFPARTREQS` (0x57) en el socket.
#[tokio::test]
async fn rotation_notify_fires_on_timeover_eviction() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    // Sin descriptor para un cliente desconocido o que no esté subiendo.
    assert!(mgr.rotation_notify("c2").await.is_none());
    assert!(mgr.rotation_notify("nobody").await.is_none());
    let notify = mgr.rotation_notify("c1").await.unwrap();
    {
        let mut active = mgr.active_uploads.lock().await;
        active.get_mut("c1").unwrap().transferred_bytes = SESSION_MAX_TRANS_BYTES;
    }
    mgr.process_tick().await;
    tokio::time::timeout(std::time::Duration::from_millis(200), notify.notified())
        .await
        .expect("la rotation doit signaler le dispatcher");
    // Descriptor consumido — cesan las notificaciones para esta sesión.
    assert!(mgr
        .rotation_notifies
        .lock()
        .await
        .get("c1")
        .is_none());
}

/// La promoción avisa al despachador (`notify`) que emite
/// `OP_ACCEPTUPLOADREQ` — el cliente en espera no depende de su
/// siguiente re-ask.
#[tokio::test]
async fn promotion_notify_fires_when_slot_frees() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    let notify = mgr.promotion_notify("c2").await.unwrap();
    mgr.release_slot("c1").await;
    tokio::time::timeout(std::time::Duration::from_millis(200), notify.notified())
        .await
        .expect("la promotion doit signaler le dispatcher");
    assert_eq!(mgr.active_snapshots().await[0].client_key, "c2");
}

/// eMule `MAX_PURGEQUEUETIME`: una entrada sin re-ask desde hace más de 1 h
/// es purgada de la cola.
#[tokio::test]
async fn stale_queue_entries_purged_after_one_hour() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    {
        let mut q = mgr.queue.lock().await;
        q[0].last_request_unix_ms -= MAX_PURGEQUEUE_MS + 1;
    }
    mgr.process_tick().await;
    assert_eq!(mgr.waiting_len().await, 0);
}

/// Límite suave y estricto de eMule: por encima de `QueueSize`, solo los clientes
/// cuyo `combined(prio × créditos)` supere la media de la cola (o
/// los amigos) son admitidos; `hard = soft + max(soft,800)/4` supone
/// un rechazo tajante.
#[tokio::test]
async fn soft_limit_admits_only_above_average_combined_score() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.set_max_queue(10); // soft=10, hard=210
    mgr.request_upload_slot("c0", "c0", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    // Llena el límite suave con clientes de alta puntuación
    // (veryhigh=18 × 10.0 créditos → combinado 1800).
    for i in 0..10 {
        mgr.request_upload_slot(&format!("q{i}"), &format!("q{i}"), "h", "f", 10.0, "veryhigh", false, None)
            .await
            .unwrap();
    }
    assert_eq!(mgr.waiting_len().await, 10);
    // Combinado bajo (normal ×1.0 = 70 < 1800) → rechazo.
    assert!(mgr
        .request_upload_slot("low1", "low1", "h", "f", 1.0, "normal", false, None)
        .await
        .is_err());
    // Combinado igual a la media → admitido.
    let (state, _) = mgr
        .request_upload_slot("hi1", "hi1", "h", "f", 10.0, "veryhigh", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Waiting);
}

/// eMule `MIN_REQUESTTIME`/`BADCLIENTBAN`: 4 re-asks del mismo archivo en
/// menos de 10 min → cliente bloqueado (puntuación 0, nunca promocionado).
#[tokio::test]
async fn aggressive_reasks_mark_client_banned() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    for _ in 0..3 {
        mgr.request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
    }
    assert!(!mgr.queue.lock().await[0].banned);
    mgr.request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    let q = mgr.queue.lock().await;
    assert!(q[0].banned);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    assert_eq!(q[0].effective_score(now), 0);
}

/// Mapeo `GetFilePrioAsNumber` de eMule: veryhigh=18, high=9, normal=7,
/// low=6, verylow=2; powershare recae en la rama predeterminada (7);
/// «auto» = prioridad efectiva inicial `PR_HIGH` (9).
#[test]
fn file_prio_num_matches_emule_get_file_prio_as_number() {
    assert_eq!(file_prio_num("veryhigh"), 18);
    assert_eq!(file_prio_num("high"), 9);
    assert_eq!(file_prio_num("normal"), 7);
    assert_eq!(file_prio_num("low"), 6);
    assert_eq!(file_prio_num("verylow"), 2);
    assert_eq!(file_prio_num("powershare"), 7);
    assert_eq!(file_prio_num("auto"), 9);
    assert_eq!(file_prio_num("unknown"), 7);
}

/// eMule `CKnownFile::UpdateAutoUpPriority`: la prioridad efectiva
/// de un archivo «auto» sigue el número de clientes que lo suben —
/// más de 20 → baja (6), más de 1 → normal (7), en otro caso → alta (9).
#[tokio::test]
async fn auto_up_priority_tracks_uploader_count() {
    let mgr = UploadManager::new();
    let (state, _) = mgr
        .request_upload_slot("c1", "c1", "haa", "f", 1.0, "auto", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Uploading);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let fake = |key: &str| UploadSessionSnapshot {
        client_key: key.to_string(),
        client_address: key.to_string(),
        file_hash: "haa".to_string(),
        file_name: "f".to_string(),
        transferred_bytes: 0,
        upload_rate_bps: 0,
        state: UploadSlotState::Uploading,
        queue_rank: None,
        started_at_unix_ms: now,
        rate_window_start_ms: 0,
        rate_window_bytes: 0,
        rate_sampled: false,
        credit_multiplier_bps: 100,
        file_prio_num: 9,
        queued_since_ms: now,
        auto_prio: true,
    };

    let mut active = mgr.active_uploads.lock().await;
    // c1 activo → +20 simulados = 21 subidores del mismo archivo.
    for i in 0..20 {
        let k = format!("f{i}");
        active.insert(k.clone(), fake(&k));
    }
    mgr.update_auto_up_prio(&active, "haa");
    assert_eq!(
        mgr.auto_up_prio.read().unwrap()["haa"], 6,
        ">20 uploaders → low"
    );

    active.remove("f0");
    mgr.update_auto_up_prio(&active, "haa");
    assert_eq!(
        mgr.auto_up_prio.read().unwrap()["haa"], 7,
        "20 uploaders → normal"
    );

    for i in 1..19 {
        active.remove(&format!("f{i}"));
    }
    mgr.update_auto_up_prio(&active, "haa");
    assert_eq!(
        mgr.auto_up_prio.read().unwrap()["haa"], 7,
        "2 uploaders → encore normal"
    );

    active.remove("f19");
    mgr.update_auto_up_prio(&active, "haa");
    assert_eq!(
        mgr.auto_up_prio.read().unwrap()["haa"], 9,
        "≤1 uploader → high"
    );
}

/// Una entrada en cola «auto» refleja el último
/// `UpdateAutoUpPriority` en cada re-ask u ordenación; un re-ask con
/// prioridad manual saca al archivo del modo automático.
#[tokio::test]
async fn auto_prio_queue_entry_refresh_and_manual_override() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.request_upload_slot("busy", "busy", "hzz", "z", 1.0, "normal", false, None)
        .await
        .unwrap();
    let (state, _) = mgr
        .request_upload_slot("q1", "q1", "haa", "f", 1.0, "auto", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Waiting);
    {
        let q = mgr.queue.lock().await;
        assert!(q[0].auto_prio);
        assert_eq!(q[0].file_prio_num, 9, "auto initial → high");
    }

    // 21 subidores activos en haa → prioridad baja efectiva en el próximo re-ask.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let mut active = mgr.active_uploads.lock().await;
        for i in 0..21 {
            let k = format!("f{i}");
            active.insert(
                k.clone(),
                UploadSessionSnapshot {
                    client_key: k.clone(),
                    client_address: k,
                    file_hash: "haa".to_string(),
                    file_name: "f".to_string(),
                    transferred_bytes: 0,
                    upload_rate_bps: 0,
                    state: UploadSlotState::Uploading,
                    queue_rank: None,
                    started_at_unix_ms: now,
                    rate_window_start_ms: 0,
                    rate_window_bytes: 0,
                    rate_sampled: false,
                    credit_multiplier_bps: 100,
                    file_prio_num: 9,
                    queued_since_ms: now,
                    auto_prio: true,
                },
            );
        }
        mgr.update_auto_up_prio(&active, "haa");
    }
    mgr.request_upload_slot("q1", "q1", "haa", "f", 1.0, "auto", false, None)
        .await
        .unwrap();
    {
        let q = mgr.queue.lock().await;
        assert!(q[0].auto_prio);
        assert_eq!(q[0].file_prio_num, 6, "auto recalculée → low");
    }

    // Cambio a manual: salida del registro automático + peso explícito.
    mgr.request_upload_slot("q1", "q1", "haa", "f", 1.0, "high", false, None)
        .await
        .unwrap();
    {
        let q = mgr.queue.lock().await;
        assert!(!q[0].auto_prio);
        assert_eq!(q[0].file_prio_num, 9);
    }
    assert!(!mgr.auto_up_prio.read().unwrap().contains_key("haa"));
}

#[tokio::test]
async fn udp_rank_lookup_matches_emule() {
    use std::net::{IpAddr, Ipv4Addr};
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));

    // Ocupa la única ranura activa para que los siguientes clientes
    // pasen a la cola.
    mgr.request_upload_slot("uh:F", "f", "hf", "f", 1.0, "normal", false, None)
        .await
        .unwrap();

    // Un cliente en cola para "haa" con puerto UDP anunciado 49672.
    mgr.request_upload_slot(
        "uh:AAAA",
        "10.0.0.5:5000",
        "haa",
        "f",
        1.0,
        "normal",
        false,
        Some(49672),
    )
    .await
    .unwrap();

    // Hash coincidente + puerto coincidente → posición 1.
    assert_eq!(
        mgr.waiting_rank_for_udp(&ip, 49672, "haa").await,
        UdpRankLookup::Rank(1)
    );
    // Puerto desconocido pero IP única → tolerancia `bIgnorePortOnUniqueIP`.
    assert_eq!(
        mgr.waiting_rank_for_udp(&ip, 9999, "haa").await,
        UdpRankLookup::Rank(1)
    );
    // Hash diferente → silencio en eMule (`reqfile does not match`).
    assert_eq!(
        mgr.waiting_rank_for_udp(&ip, 49672, "hbb").await,
        UdpRankLookup::FileMismatch
    );
    // IP desconocida → NoSender (→ QUEUEFULL únicamente si la cola está casi llena).
    assert_eq!(
        mgr.waiting_rank_for_udp(&IpAddr::V4(Ipv4Addr::new(10, 0, 0, 6)), 49672, "haa")
            .await,
        UdpRankLookup::NoSender
    );
}

#[tokio::test]
async fn udp_rank_lookup_disambiguates_same_ip_by_port() {
    use std::net::{IpAddr, Ipv4Addr};
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 9));

    mgr.request_upload_slot("uh:F", "f", "hf", "f", 1.0, "normal", false, None)
        .await
        .unwrap();

    // Dos clientes tras la misma IP (NAT), puertos UDP diferentes.
    mgr.request_upload_slot(
        "uh:A",
        "10.0.0.9:5000",
        "haa",
        "f",
        1.0,
        "normal",
        false,
        Some(49672),
    )
    .await
    .unwrap();
    mgr.request_upload_slot(
        "uh:B",
        "10.0.0.9:5001",
        "hbb",
        "g",
        1.0,
        "normal",
        false,
        Some(49673),
    )
    .await
    .unwrap();

    // El puerto UDP desambigua → el cliente correcto y el archivo correcto.
    assert_eq!(
        mgr.waiting_rank_for_udp(&ip, 49673, "hbb").await,
        UdpRankLookup::Rank(2)
    );
    // Sin coincidencia de puerto → Ambiguous (eMule fuerza conexión TCP).
    assert_eq!(
        mgr.waiting_rank_for_udp(&ip, 1111, "haa").await,
        UdpRankLookup::Ambiguous
    );
    // Puerto exacto pero archivo diferente → FileMismatch (silencio).
    assert_eq!(
        mgr.waiting_rank_for_udp(&ip, 49672, "hcc").await,
        UdpRankLookup::FileMismatch
    );
}
/// eMule `Disconnected` (`BaseClient.cpp:1169`): `bDelete =
/// m_eUploadState != US_ONUPLOADQUEUE` — un cliente en cola sobrevive al
/// cierre de su socket conservando intacta su puntuación; `release_slot` (finalización
/// explícita) siempre lo retira.
#[tokio::test]
async fn disconnect_keeps_queue_entry_and_wait_score() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.request_upload_slot("q", "q", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    let enqueued_at = mgr.queue.lock().await[0].enqueued_at_unix_ms;

    mgr.mark_disconnected("q").await;
    {
        let q = mgr.queue.lock().await;
        assert_eq!(q.len(), 1);
        assert!(!q[0].connected);
        assert_eq!(q[0].enqueued_at_unix_ms, enqueued_at);
    }

    // Finalización explícita (CANCELTRANSFER/fin de archivo): retirada completa.
    mgr.release_slot("q").await;
    assert!(mgr.queue.lock().await.is_empty());
}

/// eMule `FindBestClientInQueue`: un cliente en espera sin socket no puede
/// ser promocionado (`ACCEPTUPLOADREQ` se escribe en la conexión) — el
/// siguiente cliente conectado obtiene la ranura liberada.
#[tokio::test]
async fn disconnected_waiter_is_skipped_for_promotion() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    // Dos clientes en espera: "q1" entra primero (mayor puntuación),
    // luego "q2".
    mgr.request_upload_slot("q1", "q1", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.request_upload_slot("q2", "q2", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.mark_disconnected("q1").await;

    mgr.release_slot("a").await;
    let actives = mgr.active_snapshots().await;
    assert_eq!(actives.len(), 1);
    assert_eq!(actives[0].client_address, "q2");
    // q1 permanece en cola, aún marcado como desconectado.
    let q = mgr.queue.lock().await;
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].client_key, "q1");
    assert!(!q[0].connected);
}

/// eMule `m_bAddNextConnect` (`UploadQueue.cpp:134-151, 537-549`): el
/// mejor cliente en espera inalcanzable que supere al mejor conectado queda
/// marcado — en su reconexión recibe una ranura inmediatamente, incluso
/// con la cola llena de clientes conectados por debajo.
#[tokio::test]
async fn add_next_connect_promotes_on_reconnect() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    // Cliente antiguo en espera (larga espera -> gran puntuación) y luego uno reciente.
    mgr.request_upload_slot("old", "old", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    {
        let mut q = mgr.queue.lock().await;
        q[0].enqueued_at_unix_ms -= 3_600_000;
        q[0].last_request_unix_ms = i64::MAX / 2; // evite la purge
    }
    mgr.request_upload_slot("new", "new", "h", "f", 1.0, "low", false, None)
        .await
        .unwrap();
    mgr.mark_disconnected("old").await;

    // "old" (desconectado, alta puntuación) supera a "new" (conectado, baja prioridad)
    // -> add_next_connect fijado por promote_locked (disparado por el
    // tick de liberación).
    mgr.release_slot("a").await;
    {
        let q = mgr.queue.lock().await;
        let old = q.iter().find(|e| e.client_key == "old").unwrap();
        assert!(old.add_next_connect);
        // "new" fue promocionado a la ranura liberada.
        assert!(mgr.active_snapshots().await.iter().any(|s| s.client_address == "new"));
    }

    // Reconexión de "old": ranura inmediata a pesar de que la cola esté llena.
    let (state, _) = mgr
        .request_upload_slot("old", "old", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    assert_eq!(state, UploadSlotState::Uploading);
    assert!(mgr
        .active_snapshots()
        .await
        .iter()
        .any(|s| s.client_address == "old"));
}

/// eMule `Ban()` → `AddBannedClient`: cuando `bad_requests` alcanza
/// `BAD_CLIENT_BAN`, la IP se incluye en `pending_bans` para un bloqueo
/// de IP de 2 h en `DownloadService` — no solo retirada de puntuación.
#[tokio::test]
async fn bad_client_ban_pushes_ip_for_two_hour_ban() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.request_upload_slot("a", "1.1.1.1:4662", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    mgr.request_upload_slot("f", "9.9.9.9:4662", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    // 4 re-asks muy seguidos para el mismo archivo por parte del par en cola.
    for _ in 0..4 {
        mgr.request_upload_slot("f", "9.9.9.9:4662", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
    }
    assert!(mgr.queue.lock().await[0].banned);
    let bans = mgr.drain_pending_bans().await;
    assert_eq!(bans, vec!["9.9.9.9".to_string()]);
    assert!(mgr.drain_pending_bans().await.is_empty());
}

/// `!GetFriendSlot()` en `AddRequestCount`: un amigo nunca se
/// contabiliza en el control anti-flood de peticiones.
#[tokio::test]
async fn friend_is_exempt_from_bad_request_count() {
    let mgr = UploadManager::new();
    mgr.set_max_active(1);
    mgr.set_friend("f", true);
    mgr.request_upload_slot("a", "1.1.1.1:4662", "h", "f", 1.0, "normal", false, None)
        .await
        .unwrap();
    // El amigo entra en cola y vuelve a solicitar 5 veces consecutivas.
    for _ in 0..6 {
        let r = mgr
            .request_upload_slot("f", "9.9.9.9:4662", "h", "f", 1.0, "normal", false, None)
            .await;
        assert!(r.is_ok());
    }
    let q = mgr.queue.lock().await;
    if let Some(e) = q.iter().find(|e| e.client_key == "f") {
        assert!(!e.banned);
        assert_eq!(e.bad_requests, 0);
    }
    drop(q);
    assert!(mgr.drain_pending_bans().await.is_empty());
}


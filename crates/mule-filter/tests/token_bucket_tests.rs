// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_filter::token_bucket::{TokenBucket, TokenBucketError};
use mule_filter::SessionPseudonymizer;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn test_token_bucket_burst_and_refill() {
    // 10 fichas por segundo, capacidad de ráfaga 20
    let bucket = TokenBucket::new(10, 20);

    // La ráfaga inicial puede adquirir 20 fichas inmediatamente
    assert!(bucket.try_acquire(20));
    // Ahora el cubo está vacío
    assert!(!bucket.try_acquire(1));

    // Esperar 200 ms -> debería recargar ~2 fichas
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(bucket.try_acquire(2));
    assert!(!bucket.try_acquire(2));
}

#[tokio::test]
async fn test_token_bucket_cancellation() {
    let bucket = TokenBucket::new(1, 1);
    assert!(bucket.try_acquire(1));

    let cancelled = Arc::new(AtomicBool::new(false));
    let c_clone = cancelled.clone();

    let task = tokio::spawn(async move {
        bucket
            .acquire_tokens(10, || c_clone.load(Ordering::Relaxed))
            .await
    });

    // Simular la activación del interruptor de corte (Kill Switch)
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancelled.store(true, Ordering::Relaxed);

    let res = task.await.unwrap();
    assert_eq!(res, Err(TokenBucketError::Cancelled));
}

#[test]
fn test_session_pseudonymizer() {
    let pseudo = SessionPseudonymizer::new();
    let addr1: std::net::SocketAddr = "192.0.2.1:39735".parse().unwrap();
    let addr2: std::net::SocketAddr = "192.0.2.1:49665".parse().unwrap();
    let addr3: std::net::SocketAddr = "198.51.100.5:49661".parse().unwrap();

    let p1 = pseudo.pseudonymize_peer(addr1);
    let p2 = pseudo.pseudonymize_peer(addr2);
    let s1 = pseudo.pseudonymize_server(addr3);

    assert_eq!(p1, "peer-01:39735");
    assert_eq!(p2, "peer-01:49665"); // La misma IP preserva el ID, el puerto se preserva
    assert_eq!(s1, "server-01:49661");
}

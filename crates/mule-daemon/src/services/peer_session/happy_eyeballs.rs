// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Conexión Dual-Stack IPv6/IPv4 mediante Happy Eyeballs v2 (RFC 8305).
//!
//! Prioriza IPv6 cuando está disponible y lanza el respaldo IPv4 tras un retardo
//! acotado de 250 ms si IPv6 aún no ha completado el saludo TCP.

use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::task::JoinSet;

use mule_core::model::PeerFailureReason;
use mule_network_policy::CancellationToken;

use super::types::PeerError;

/// Retardo estándar entre intentos de conexión de distintas familias (RFC 8305 `Connection Attempt Delay`).
pub const HAPPY_EYEBALLS_DELAY: Duration = Duration::from_millis(250);

/// Ordena direcciones candidatas intercalando familias con preferencia inicial por IPv6.
pub fn order_happy_eyeballs(addrs: &[SocketAddr]) -> Vec<SocketAddr> {
    let v6: Vec<SocketAddr> = addrs.iter().copied().filter(|a| a.is_ipv6()).collect();
    let v4: Vec<SocketAddr> = addrs.iter().copied().filter(|a| a.is_ipv4()).collect();
    let mut ordered = Vec::with_capacity(addrs.len());
    let max_len = v6.len().max(v4.len());
    for i in 0..max_len {
        if let Some(&a6) = v6.get(i) {
            ordered.push(a6);
        }
        if let Some(&a4) = v4.get(i) {
            ordered.push(a4);
        }
    }
    ordered
}

/// Conecta a la lista de direcciones candidatas aplicando Happy Eyeballs v2 (RFC 8305).
pub(crate) async fn connect_happy_eyeballs(
    addrs: &[SocketAddr],
    connect_timeout: Duration,
    cancel_token: &CancellationToken,
) -> Result<(TcpStream, SocketAddr), PeerError> {
    let ordered = order_happy_eyeballs(addrs);
    if ordered.is_empty() {
        return Err(PeerError::new(
            PeerFailureReason::ConnectRefused,
            "No candidate socket addresses provided",
        ));
    }

    // Camino rápido sin sobrecoste cuando solo existe un candidato.
    if ordered.len() == 1 {
        let addr = ordered[0];
        return tokio::select! {
            _ = cancel_token.cancelled() => {
                Err(PeerError::new(PeerFailureReason::KillSwitchAborted, "Cancelled by network policy kill switch"))
            }
            res = tokio::time::timeout(connect_timeout, TcpStream::connect(addr)) => {
                match res {
                    Ok(Ok(stream)) => Ok((stream, addr)),
                    Ok(Err(e)) => Err(PeerError::new(PeerFailureReason::ConnectRefused, format!("TCP connect refused: {e}"))),
                    Err(_) => Err(PeerError::new(PeerFailureReason::TimeoutConnect, "TCP connect timed out")),
                }
            }
        };
    }

    let mut set = JoinSet::new();
    let mut next_idx = 0usize;
    let spawn_attempt = |set: &mut JoinSet<Result<(TcpStream, SocketAddr), String>>, addr: SocketAddr| {
        set.spawn(async move {
            match tokio::time::timeout(connect_timeout, TcpStream::connect(addr)).await {
                Ok(Ok(s)) => Ok((s, addr)),
                Ok(Err(e)) => Err(format!("connect {addr}: {e}")),
                Err(_) => Err(format!("connect {addr}: timed out")),
            }
        });
    };

    spawn_attempt(&mut set, ordered[next_idx]);
    next_idx += 1;

    let mut last_err = String::from("TCP connect failed");
    let mut saw_timeout = false;

    loop {
        if next_idx < ordered.len() {
            let stagger = tokio::time::sleep(HAPPY_EYEBALLS_DELAY);
            tokio::pin!(stagger);
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    set.abort_all();
                    return Err(PeerError::new(PeerFailureReason::KillSwitchAborted, "Cancelled by network policy kill switch"));
                }
                _ = &mut stagger => {
                    spawn_attempt(&mut set, ordered[next_idx]);
                    next_idx += 1;
                }
                joined = set.join_next() => {
                    match joined {
                        Some(Ok(Ok((stream, addr)))) => {
                            set.abort_all();
                            return Ok((stream, addr));
                        }
                        Some(Ok(Err(msg))) => {
                            if msg.contains("timed out") {
                                saw_timeout = true;
                            }
                            last_err = msg;
                            // Si el intento actual falló rápido antes de los 250 ms, lanzar el siguiente de inmediato.
                            if next_idx < ordered.len() {
                                spawn_attempt(&mut set, ordered[next_idx]);
                                next_idx += 1;
                            }
                        }
                        Some(Err(_)) => {}
                        None => {}
                    }
                }
            }
        } else {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    set.abort_all();
                    return Err(PeerError::new(PeerFailureReason::KillSwitchAborted, "Cancelled by network policy kill switch"));
                }
                joined = set.join_next() => {
                    match joined {
                        Some(Ok(Ok((stream, addr)))) => {
                            set.abort_all();
                            return Ok((stream, addr));
                        }
                        Some(Ok(Err(msg))) => {
                            if msg.contains("timed out") {
                                saw_timeout = true;
                            }
                            last_err = msg;
                        }
                        Some(Err(_)) => {}
                        None => {
                            let reason = if saw_timeout {
                                PeerFailureReason::TimeoutConnect
                            } else {
                                PeerFailureReason::ConnectRefused
                            };
                            return Err(PeerError::new(reason, last_err));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn orders_ipv6_first_and_interleaves() {
        let v4_1: SocketAddr = "198.51.100.1:4662".parse().unwrap();
        let v4_2: SocketAddr = "198.51.100.2:4662".parse().unwrap();
        let v6_1: SocketAddr = "[2001:db8::1]:4662".parse().unwrap();
        let ordered = order_happy_eyeballs(&[v4_1, v6_1, v4_2]);
        assert_eq!(ordered, vec![v6_1, v4_1, v4_2]);
    }

    #[tokio::test]
    async fn happy_eyeballs_falls_back_to_v4_when_v6_fails() {
        let v4_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let v4_addr = v4_listener.local_addr().unwrap();

        // Reservar y cerrar un puerto en [::1] para garantizar fallo o rechazo inmediato.
        let dead_v6_addr = match TcpListener::bind("[::1]:0").await {
            Ok(l) => {
                let a = l.local_addr().unwrap();
                drop(l);
                a
            }
            Err(_) => {
                // Si el host de prueba no tiene interfaz IPv6 loopback, solo validar IPv4.
                let token = CancellationToken::new();
                let (_s, winner) =
                    connect_happy_eyeballs(&[v4_addr], Duration::from_secs(2), &token)
                        .await
                        .unwrap();
                assert_eq!(winner, v4_addr);
                return;
            }
        };

        let accept_task = tokio::spawn(async move {
            let (_s, _) = v4_listener.accept().await.unwrap();
        });

        let token = CancellationToken::new();
        let (_stream, winner) = connect_happy_eyeballs(
            &[dead_v6_addr, v4_addr],
            Duration::from_secs(2),
            &token,
        )
        .await
        .unwrap();
        assert_eq!(winner, v4_addr);
        accept_task.await.unwrap();
    }
}

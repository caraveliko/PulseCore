// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Coordinador ligero de perforación UDP (*UDP Hole Punching* `PCHP`).
//!
//! Facilita el establecimiento directo de conectividad UDP entre dos pares detrás de NAT
//! mediante un nodo intermedio (*buddy* / coordinador), intercambiando un pequeño token
//! de sesión seguido de ráfagas sincronizadas de sondeos `Ping`/`Ack`.

use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;

/// Marcador mágico de 4 bytes para mensajes del protocolo `PCHP`.
pub const HOLE_PUNCH_MAGIC: [u8; 4] = *b"PCHP";
/// Número de paquetes `Ping` emitidos por ráfaga de perforación NAT.
pub const DEFAULT_PUNCH_ATTEMPTS: usize = 6;
/// Intervalo entre cada sonda UDP de perforación.
const PUNCH_INTERVAL: Duration = Duration::from_millis(80);
/// Tiempo límite esperando el `Ack` tras completar las sondas.
const PUNCH_ACK_TIMEOUT: Duration = Duration::from_millis(1200);
/// Tamaño exacto del mensaje binario `PCHP`: `magic(4) + kind(1) + session_id(8)`.
pub const HOLE_PUNCH_FRAME_LEN: usize = 13;

/// Tipo de mensaje del protocolo ligero `PCHP`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PunchMsgKind {
    /// Solicitud de coordinación hacia el par / coordinador.
    Sync = 0x01,
    /// Sonda de apertura de tabla NAT hacia el extremo remoto.
    Ping = 0x02,
    /// Confirmación de recepción directa en el extremo perforado.
    Ack = 0x03,
}

impl TryFrom<u8> for PunchMsgKind {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::Sync),
            0x02 => Ok(Self::Ping),
            0x03 => Ok(Self::Ack),
            _ => Err(()),
        }
    }
}

/// Codifica una trama `PCHP` de 13 bytes.
pub fn encode_punch_frame(kind: PunchMsgKind, session_id: u64) -> [u8; HOLE_PUNCH_FRAME_LEN] {
    let mut out = [0u8; HOLE_PUNCH_FRAME_LEN];
    out[..4].copy_from_slice(&HOLE_PUNCH_MAGIC);
    out[4] = kind as u8;
    out[5..13].copy_from_slice(&session_id.to_be_bytes());
    out
}

/// Decodifica y valida una trama `PCHP` de 13 bytes.
pub fn decode_punch_frame(buf: &[u8]) -> Option<(PunchMsgKind, u64)> {
    if buf.len() != HOLE_PUNCH_FRAME_LEN || buf[..4] != HOLE_PUNCH_MAGIC {
        return None;
    }
    let kind = PunchMsgKind::try_from(buf[4]).ok()?;
    let session_id = u64::from_be_bytes(buf[5..13].try_into().ok()?);
    Some((kind, session_id))
}

/// Coordinador sin estado de sesiones de perforación UDP entre dos pares.
#[derive(Debug, Default, Clone, Copy)]
pub struct UdpHolePunchCoordinator;

impl UdpHolePunchCoordinator {
    /// Crea un coordinador de perforación UDP.
    pub fn new() -> Self {
        Self
    }

    /// Retransmite una señal `Sync(session_id)` entre `peer_a` y `peer_b` sobre `relay_sock`
    /// para sincronizar el instante de envío simultáneo.
    pub async fn coordinate_pair(
        &self,
        relay_sock: &UdpSocket,
        peer_a: SocketAddr,
        peer_b: SocketAddr,
        session_id: u64,
    ) -> Result<(), String> {
        let sync_frame = encode_punch_frame(PunchMsgKind::Sync, session_id);
        relay_sock
            .send_to(&sync_frame, peer_a)
            .await
            .map_err(|e| format!("Error enviando Sync PCHP a par A {peer_a}: {e}"))?;
        relay_sock
            .send_to(&sync_frame, peer_b)
            .await
            .map_err(|e| format!("Error enviando Sync PCHP a par B {peer_b}: {e}"))?;
        Ok(())
    }

    /// Ejecuta la ráfaga local de sondas `Ping(session_id)` hacia `remote_peer` y responde
    /// con `Ack(session_id)` al recibir el primer `Ping` o confirma al recibir `Ack`.
    pub async fn punch_bidirectional(
        &self,
        local_sock: &UdpSocket,
        remote_peer: SocketAddr,
        session_id: u64,
    ) -> Result<SocketAddr, String> {
        let ping_frame = encode_punch_frame(PunchMsgKind::Ping, session_id);
        let ack_frame = encode_punch_frame(PunchMsgKind::Ack, session_id);

        let mut buf = [0u8; 64];
        let mut saw_remote_ack_or_ping = false;

        for _ in 0..DEFAULT_PUNCH_ATTEMPTS {
            let _ = local_sock.send_to(&ping_frame, remote_peer).await;
            let recv_res = tokio::time::timeout(PUNCH_INTERVAL, local_sock.recv_from(&mut buf)).await;
            if let Ok(Ok((n, from))) = recv_res {
                if let Some((kind, sid)) = decode_punch_frame(&buf[..n]) {
                    if sid == session_id && from.ip() == remote_peer.ip() {
                        match kind {
                            PunchMsgKind::Ping => {
                                let _ = local_sock.send_to(&ack_frame, from).await;
                                saw_remote_ack_or_ping = true;
                            }
                            PunchMsgKind::Ack => {
                                let _ = local_sock.send_to(&ack_frame, from).await;
                                return Ok(from);
                            }
                            PunchMsgKind::Sync => {}
                        }
                    }
                }
            }
        }

        if saw_remote_ack_or_ping {
            return Ok(remote_peer);
        }

        let final_res = tokio::time::timeout(PUNCH_ACK_TIMEOUT, async {
            loop {
                let (n, from) = local_sock.recv_from(&mut buf).await.map_err(|e| e.to_string())?;
                if let Some((kind, sid)) = decode_punch_frame(&buf[..n]) {
                    if sid == session_id && from.ip() == remote_peer.ip() {
                        match kind {
                            PunchMsgKind::Ping => {
                                let _ = local_sock.send_to(&ack_frame, from).await;
                                return Ok::<SocketAddr, String>(from);
                            }
                            PunchMsgKind::Ack => return Ok::<SocketAddr, String>(from),
                            PunchMsgKind::Sync => {}
                        }
                    }
                }
            }
        })
        .await;

        match final_res {
            Ok(Ok(addr)) => Ok(addr),
            Ok(Err(e)) => Err(format!("Error recibiendo respuesta de perforación UDP: {e}")),
            Err(_) => Err("Tiempo de espera agotado en perforación UDP PCHP".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn three_socket_loopback_hole_punch_succeeds() {
        let coordinator_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer_a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer_b = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        let addr_a = peer_a.local_addr().unwrap();
        let addr_b = peer_b.local_addr().unwrap();
        let session_id = 0x2026_CAFE_BABEu64;

        let coord = UdpHolePunchCoordinator::new();
        coord
            .coordinate_pair(&coordinator_sock, addr_a, addr_b, session_id)
            .await
            .unwrap();

        // Verificar que ambos pares reciben la señal Sync de coordinación.
        let mut buf_a = [0u8; 32];
        let (na, _) = peer_a.recv_from(&mut buf_a).await.unwrap();
        assert_eq!(decode_punch_frame(&buf_a[..na]), Some((PunchMsgKind::Sync, session_id)));

        let mut buf_b = [0u8; 32];
        let (nb, _) = peer_b.recv_from(&mut buf_b).await.unwrap();
        assert_eq!(decode_punch_frame(&buf_b[..nb]), Some((PunchMsgKind::Sync, session_id)));

        let (res_a, res_b) = tokio::join!(
            coord.punch_bidirectional(&peer_a, addr_b, session_id),
            coord.punch_bidirectional(&peer_b, addr_a, session_id),
        );
        assert_eq!(res_a.unwrap(), addr_b);
        assert_eq!(res_b.unwrap(), addr_a);
    }
}

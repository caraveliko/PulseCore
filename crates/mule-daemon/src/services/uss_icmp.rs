// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Sondeo ICMP para USS — equivalente a `LastCommonRouteFinder` de eMule.
//!
//! eMule traza rutas ICMP (incrementando el TTL) hacia una muestra de hosts
//! remotos y luego hace ping al salto más profundo compartido por todas las
//! rutas — normalmente el router de agregación del ISP. Esto mide
//! específicamente la congestión en el enlace de subida *local*, a diferencia
//! de un sondeo de extremo a extremo hacia un servidor distante.
//!
//! Windows utiliza `IcmpSendEcho` (iphlpapi.dll) — la misma API sin
//! privilegios que `ping.exe`/`tracert.exe` (los sockets raw requerirían
//! permisos de administrador). Otras plataformas no disponen aquí de una ruta
//! ICMP sin privilegios: cada llamada devuelve `None` y USS recurre al sondeo
//! mediante conexión TCP.

use std::net::Ipv4Addr;

/// Profundidad máxima de traceroute (eMule sondea hasta 30 saltos).
const MAX_HOPS: u8 = 30;
/// Tiempo de espera de eco por salto.
const HOP_TIMEOUT_MS: u32 = 800;
/// Aborta una ruta tras este número de saltos consecutivos sin respuesta.
const MAX_CONSECUTIVE_LOSS: u8 = 3;
#[allow(dead_code)]
/// Tamaño de la carga útil ICMP.
const PAYLOAD_LEN: usize = 32;

/// Una respuesta de eco ICMP. `reached` distingue el destino en sí
/// (`IP_SUCCESS`) de un router de tránsito (`IP_TTL_EXPIRED_TRANSIT`).
#[derive(Debug, Clone)]
pub struct EchoReply {
    /// Host que responde (router de tránsito para respuestas con TTL expirado).
    pub responder: Ipv4Addr,
    pub rtt_ms: u32,
    /// true cuando la respuesta provino del destino en sí.
    pub reached: bool,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::NetworkManagement::IpHelper::{
        IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho, ICMP_ECHO_REPLY, IP_OPTION_INFORMATION,
        IP_SUCCESS,
    };

    /// Eco ICMP bloqueante — `ttl` = TTL IP (255 para ping directo,
    /// 1..MAX_HOPS para traceroute). Devuelve `None` en caso de tiempo de espera/fallo.
    pub fn echo(dest: Ipv4Addr, ttl: u8, timeout_ms: u32) -> Option<EchoReply> {
        unsafe {
            let Ok(handle) = IcmpCreateFile() else {
                return None;
            };
            let payload = [0u8; PAYLOAD_LEN];
            let mut buf = vec![0u8; std::mem::size_of::<ICMP_ECHO_REPLY>() + PAYLOAD_LEN + 8];
            let opts = IP_OPTION_INFORMATION {
                Ttl: ttl,
                ..Default::default()
            };
            // `destinationaddress` es la dirección IPv4 en orden de bytes
            // de red — `from_le_bytes` en los octetos produce exactamente eso.
            let replies = IcmpSendEcho(
                handle,
                u32::from_le_bytes(dest.octets()),
                payload.as_ptr() as *const _,
                payload.len() as u16,
                Some(&opts),
                buf.as_mut_ptr() as *mut _,
                buf.len() as u32,
                timeout_ms,
            );
            let _ = IcmpCloseHandle(handle);
            if replies == 0 {
                return None;
            }
            let reply = &*(buf.as_ptr() as *const ICMP_ECHO_REPLY);
            Some(EchoReply {
                responder: Ipv4Addr::from(reply.Address.to_le_bytes()),
                rtt_ms: reply.RoundTripTime,
                reached: reply.Status == IP_SUCCESS,
            })
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn echo(_dest: Ipv4Addr, _ttl: u8, _timeout_ms: u32) -> Option<EchoReply> {
        None
    }
}

/// Si el sondeo ICMP está disponible en esta plataforma (solo Windows).
pub fn icmp_available() -> bool {
    cfg!(windows)
}

/// Ping ICMP — devuelve el tiempo de ida y vuelta en ms, `None` en tiempo de espera.
pub async fn ping(dest: Ipv4Addr, timeout_ms: u32) -> Option<u32> {
    tokio::task::spawn_blocking(move || imp::echo(dest, 255, timeout_ms))
        .await
        .ok()
        .flatten()
        .filter(|r| r.reached)
        .map(|r| r.rtt_ms)
}

/// Traceroute ICMP a `dest` — lista ordenada de saltos (más cercano … destino).
/// Los saltos que no responden se omiten; `None` cuando nada responde.
pub async fn trace_route(dest: Ipv4Addr) -> Option<Vec<Ipv4Addr>> {
    tokio::task::spawn_blocking(move || {
        let mut hops = Vec::new();
        let mut losses = 0u8;
        for ttl in 1..=MAX_HOPS {
            match imp::echo(dest, ttl, HOP_TIMEOUT_MS) {
                Some(r) => {
                    losses = 0;
                    hops.push(r.responder);
                    if r.reached {
                        break;
                    }
                }
                None => {
                    losses += 1;
                    if losses >= MAX_CONSECUTIVE_LOSS {
                        break;
                    }
                }
            }
        }
        if hops.is_empty() {
            None
        } else {
            Some(hops)
        }
    })
    .await
    .ok()
    .flatten()
}

/// `CLastCommonRouteFinder` de eMule: el salto más profundo presente en *cada*
/// ruta — el punto de agregación del ISP. Requiere ≥ 2 rutas: con una
/// única ruta el resultado degeneraría en el propio destino.
pub fn last_common_host(routes: &[Vec<Ipv4Addr>]) -> Option<Ipv4Addr> {
    if routes.len() < 2 {
        return None;
    }
    routes[0]
        .iter()
        .rev()
        .find(|hop| routes.iter().skip(1).all(|r| r.contains(hop)))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    #[test]
    fn last_common_host_picks_deepest_shared_hop() {
        let routes = vec![
            vec![
                v4("192.168.1.1"),
                v4("80.10.1.1"),
                v4("80.10.2.5"),
                v4("9.9.9.9"),
            ],
            vec![
                v4("192.168.1.1"),
                v4("80.10.1.1"),
                v4("80.10.9.9"),
                v4("8.8.8.8"),
            ],
        ];
        // Prevalece el salto más profundo presente en cada ruta (punto de agregación del ISP),
        // no la pasarela superficial doméstica que también es común.
        assert_eq!(last_common_host(&routes), Some(v4("80.10.1.1")));
    }

    #[test]
    fn last_common_host_requires_two_routes() {
        let routes = vec![vec![v4("192.168.1.1"), v4("80.10.1.1"), v4("8.8.8.8")]];
        assert_eq!(last_common_host(&routes), None);
    }

    #[test]
    fn last_common_host_none_when_routes_diverge() {
        let routes = vec![
            vec![v4("10.0.0.1"), v4("80.10.1.1")],
            vec![v4("10.0.0.1"), v4("90.20.2.2")],
        ];
        // 10.0.0.1 ES común a ambas (pasarela superficial).
        assert_eq!(last_common_host(&routes), Some(v4("10.0.0.1")));

        let fully_diverged = vec![vec![v4("80.10.1.1")], vec![v4("90.20.2.2")]];
        assert_eq!(last_common_host(&fully_diverged), None);
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Cliente de respaldo PCP (RFC 6887) y NAT-PMP (RFC 6886) sobre UDP.
//!
//! Se invoca únicamente cuando el descubrimiento o mapeo UPnP IGD fracasa,
//! ofreciendo mapeo ligero de puertos TCP/UDP e identificación de IP externa.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use tokio::net::UdpSocket;

use super::types::is_valid_gateway_addr;

/// Puerto estándar IANA compartido por NAT-PMP (RFC 6886) y PCP (RFC 6887).
pub const PCP_NATPMP_PORT: u16 = 5351;
/// Duración de concesión solicitada en segundos (1 hora).
pub const DEFAULT_MAPPING_LEASE_SECS: u32 = 3600;
/// Tiempo máximo de espera por respuesta UDP de puerta de enlace.
const GATEWAY_UDP_TIMEOUT: Duration = Duration::from_millis(1200);
/// Versión del protocolo PCP (RFC 6887).
const PCP_VERSION: u8 = 2;
/// Versión del protocolo NAT-PMP (RFC 6886).
const NATPMP_VERSION: u8 = 0;
/// Opcode PCP `MAP`.
const PCP_OPCODE_MAP: u8 = 1;
/// Bit indicador de respuesta en cabeceras PCP y NAT-PMP.
const RESPONSE_BIT: u8 = 0x80;
/// Longitud exacta de un mensaje PCP `MAP` (cabecera 24B + carga 36B).
const PCP_MAP_MSG_LEN: usize = 60;
/// Longitud mínima de la respuesta PCP `MAP`.
const PCP_MAP_MIN_RESP_LEN: usize = 60;
/// Códigos de protocolo IANA usados en PCP `MAP`.
const PROTO_TCP: u8 = 6;
const PROTO_UDP: u8 = 17;
/// Opcodes NAT-PMP.
const NATPMP_OP_EXTERNAL_ADDR: u8 = 0;
const NATPMP_OP_MAP_UDP: u8 = 1;
const NATPMP_OP_MAP_TCP: u8 = 2;
/// Tamaños de respuesta NAT-PMP.
const NATPMP_EXT_ADDR_RESP_LEN: usize = 12;
const NATPMP_MAP_RESP_LEN: usize = 16;
/// Tamaño máximo de datagrama esperado desde la puerta de enlace.
const MAX_GATEWAY_DGRAM_LEN: usize = 256;

/// Resultado del mapeo de puertos mediante la cascada `PCP -> NAT-PMP`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayCascadeResult {
    /// Protocolo que completó al menos un mapeo (`"pcp"` o `"nat-pmp"`).
    pub protocol: &'static str,
    /// IP externa notificada por la puerta de enlace, si está disponible.
    pub external_ip: Option<IpAddr>,
    /// Indica si el puerto TCP quedó mapeado.
    pub tcp_mapped: bool,
    /// Indica si el puerto UDP quedó mapeado.
    pub udp_mapped: bool,
}

/// Detecta la dirección IPv4 de la puerta de enlace predeterminada leyendo la tabla de rutas local.
pub fn detect_default_gateway_ipv4() -> Option<Ipv4Addr> {
    #[cfg(target_os = "linux")]
    {
        let content = std::fs::read_to_string("/proc/net/route").ok()?;
        for line in content.lines().skip(1) {
            let mut cols = line.split_whitespace();
            let _iface = cols.next()?;
            let dest = cols.next()?;
            let gw_hex = cols.next()?;
            if dest == "00000000" {
                if let Ok(raw) = u32::from_str_radix(gw_hex, 16) {
                    // `/proc/net/route` almacena la IP en orden de bytes del host.
                    #[cfg(target_endian = "little")]
                    let ip = Ipv4Addr::from(raw.to_le_bytes());
                    #[cfg(target_endian = "big")]
                    let ip = Ipv4Addr::from(raw.to_be_bytes());
                    if !ip.is_unspecified() {
                        return Some(ip);
                    }
                }
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn ip_to_v6_mapped_bytes(ip: IpAddr) -> [u8; 16] {
    match ip {
        IpAddr::V4(v4) => v4.to_ipv6_mapped().octets(),
        IpAddr::V6(v6) => v6.octets(),
    }
}

fn v6_mapped_bytes_to_ip(bytes: &[u8]) -> Option<IpAddr> {
    let arr: [u8; 16] = bytes.try_into().ok()?;
    let v6 = Ipv6Addr::from(arr);
    if v6.is_unspecified() {
        return None;
    }
    Some(match v6.to_ipv4_mapped() {
        Some(v4) => IpAddr::V4(v4),
        None => IpAddr::V6(v6),
    })
}

/// Construye un mensaje PCP `MAP` de 60 bytes conforme a RFC 6887 §7.1 y §11.1.
fn encode_pcp_map_request(
    client_ip: IpAddr,
    nonce: [u8; 12],
    protocol: u8,
    internal_port: u16,
    lease_secs: u32,
) -> [u8; PCP_MAP_MSG_LEN] {
    let mut pkt = [0u8; PCP_MAP_MSG_LEN];
    pkt[0] = PCP_VERSION;
    pkt[1] = PCP_OPCODE_MAP;
    pkt[4..8].copy_from_slice(&lease_secs.to_be_bytes());
    pkt[8..24].copy_from_slice(&ip_to_v6_mapped_bytes(client_ip));
    pkt[24..36].copy_from_slice(&nonce);
    pkt[36] = protocol;
    pkt[40..42].copy_from_slice(&internal_port.to_be_bytes());
    pkt[42..44].copy_from_slice(&internal_port.to_be_bytes());
    pkt
}

/// Parsea una respuesta PCP `MAP` validando versión, opcode, nonce y código de resultado.
fn decode_pcp_map_response(buf: &[u8], expected_nonce: [u8; 12]) -> Result<Option<IpAddr>, String> {
    if buf.len() < PCP_MAP_MIN_RESP_LEN {
        return Err(format!("Respuesta PCP demasiado corta ({} bytes)", buf.len()));
    }
    if buf[0] != PCP_VERSION || buf[1] != (PCP_OPCODE_MAP | RESPONSE_BIT) {
        return Err("Cabecera PCP inválida o versión no compatible".to_string());
    }
    let result_code = buf[3];
    if result_code != 0 {
        return Err(format!("PCP devolvió código de error {result_code}"));
    }
    if buf[24..36] != expected_nonce {
        return Err("El nonce de respuesta PCP no coincide".to_string());
    }
    Ok(v6_mapped_bytes_to_ip(&buf[44..60]))
}

async fn udp_exchange(
    sock: &UdpSocket,
    req: &[u8],
    gateway: SocketAddr,
) -> Result<Vec<u8>, String> {
    sock.send_to(req, gateway)
        .await
        .map_err(|e| format!("Error al enviar datagrama a {gateway}: {e}"))?;
    let mut buf = vec![0u8; MAX_GATEWAY_DGRAM_LEN];
    let (n, peer) = tokio::time::timeout(GATEWAY_UDP_TIMEOUT, sock.recv_from(&mut buf))
        .await
        .map_err(|_| format!("Tiempo de espera agotado esperando respuesta desde {gateway}"))?
        .map_err(|e| format!("Error leyendo respuesta UDP de {gateway}: {e}"))?;
    if peer.ip() != gateway.ip() {
        return Err("Respuesta recibida desde una IP distinta a la puerta de enlace".to_string());
    }
    buf.truncate(n);
    Ok(buf)
}

async fn try_pcp_map(
    gateway: SocketAddr,
    local_ip: IpAddr,
    tcp_port: u16,
    udp_port: u16,
) -> Result<GatewayCascadeResult, String> {
    let bind_addr = if gateway.is_ipv6() {
        "[::]:0"
    } else {
        "0.0.0.0:0"
    };
    let sock = UdpSocket::bind(bind_addr)
        .await
        .map_err(|e| format!("No se pudo enlazar socket UDP PCP: {e}"))?;

    let mut tcp_mapped = false;
    let mut udp_mapped = false;
    let mut external_ip = None;

    for (proto, port, is_tcp) in [(PROTO_TCP, tcp_port, true), (PROTO_UDP, udp_port, false)] {
        if port == 0 {
            continue;
        }
        let nonce: [u8; 12] = rand::random();
        let req = encode_pcp_map_request(local_ip, nonce, proto, port, DEFAULT_MAPPING_LEASE_SECS);
        let resp = udp_exchange(&sock, &req, gateway).await?;
        let ext = decode_pcp_map_response(&resp, nonce)?;
        if ext.is_some() {
            external_ip = ext;
        }
        if is_tcp {
            tcp_mapped = true;
        } else {
            udp_mapped = true;
        }
    }

    if !tcp_mapped && !udp_mapped {
        return Err("PCP no mapeó ningún puerto solicitado".to_string());
    }

    Ok(GatewayCascadeResult {
        protocol: "pcp",
        external_ip,
        tcp_mapped,
        udp_mapped,
    })
}

async fn try_natpmp_map(
    gateway: SocketAddr,
    tcp_port: u16,
    udp_port: u16,
) -> Result<GatewayCascadeResult, String> {
    let sock = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| format!("No se pudo enlazar socket UDP NAT-PMP: {e}"))?;

    let ext_req = [NATPMP_VERSION, NATPMP_OP_EXTERNAL_ADDR];
    let ext_resp = udp_exchange(&sock, &ext_req, gateway).await?;
    if ext_resp.len() < NATPMP_EXT_ADDR_RESP_LEN
        || ext_resp[0] != NATPMP_VERSION
        || ext_resp[1] != (NATPMP_OP_EXTERNAL_ADDR | RESPONSE_BIT)
    {
        return Err("Respuesta NAT-PMP de IP externa inválida".to_string());
    }
    let ext_status = u16::from_be_bytes([ext_resp[2], ext_resp[3]]);
    if ext_status != 0 {
        return Err(format!("NAT-PMP rechazó la consulta externa (código {ext_status})"));
    }
    let external_ip = Some(IpAddr::V4(Ipv4Addr::new(
        ext_resp[8],
        ext_resp[9],
        ext_resp[10],
        ext_resp[11],
    )));

    let mut tcp_mapped = false;
    let mut udp_mapped = false;

    for (opcode, port, is_tcp) in [
        (NATPMP_OP_MAP_TCP, tcp_port, true),
        (NATPMP_OP_MAP_UDP, udp_port, false),
    ] {
        if port == 0 {
            continue;
        }
        let mut req = [0u8; 12];
        req[0] = NATPMP_VERSION;
        req[1] = opcode;
        req[4..6].copy_from_slice(&port.to_be_bytes());
        req[6..8].copy_from_slice(&port.to_be_bytes());
        req[8..12].copy_from_slice(&DEFAULT_MAPPING_LEASE_SECS.to_be_bytes());
        let resp = udp_exchange(&sock, &req, gateway).await?;
        if resp.len() < NATPMP_MAP_RESP_LEN
            || resp[0] != NATPMP_VERSION
            || resp[1] != (opcode | RESPONSE_BIT)
        {
            return Err("Respuesta NAT-PMP de mapeo inválida".to_string());
        }
        let code = u16::from_be_bytes([resp[2], resp[3]]);
        if code != 0 {
            return Err(format!("NAT-PMP falló mapeando puerto {port} (código {code})"));
        }
        if is_tcp {
            tcp_mapped = true;
        } else {
            udp_mapped = true;
        }
    }

    if !tcp_mapped && !udp_mapped {
        return Err("NAT-PMP no mapeó ningún puerto solicitado".to_string());
    }

    Ok(GatewayCascadeResult {
        protocol: "nat-pmp",
        external_ip,
        tcp_mapped,
        udp_mapped,
    })
}

/// Intenta mapear los puertos TCP y UDP contra `gateway` aplicando la cascada `PCP -> NAT-PMP`.
///
/// Valida estrictamente `gateway` contra la política anti-SSRF antes de emitir datagramas.
pub async fn map_ports_cascade_at(
    gateway: SocketAddr,
    local_ip: IpAddr,
    tcp_port: u16,
    udp_port: u16,
) -> Result<GatewayCascadeResult, String> {
    if !is_valid_gateway_addr(gateway) {
        return Err(format!(
            "Dirección de puerta de enlace PCP/NAT-PMP no válida por seguridad: {gateway}"
        ));
    }
    match try_pcp_map(gateway, local_ip, tcp_port, udp_port).await {
        Ok(ok) => Ok(ok),
        Err(pcp_err) => {
            if gateway.is_ipv4() {
                try_natpmp_map(gateway, tcp_port, udp_port)
                    .await
                    .map_err(|natpmp_err| format!("PCP: {pcp_err}; NAT-PMP: {natpmp_err}"))
            } else {
                Err(pcp_err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pcp_cascade_maps_ports_over_loopback_mock() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let gw_addr = server.local_addr().unwrap();

        let srv_handle = tokio::spawn(async move {
            let mut buf = [0u8; 128];
            for _ in 0..2 {
                let (n, peer) = server.recv_from(&mut buf).await.unwrap();
                assert_eq!(n, PCP_MAP_MSG_LEN);
                assert_eq!(buf[0], PCP_VERSION);
                let mut resp = [0u8; PCP_MAP_MSG_LEN];
                resp[..PCP_MAP_MSG_LEN].copy_from_slice(&buf[..PCP_MAP_MSG_LEN]);
                resp[1] = PCP_OPCODE_MAP | RESPONSE_BIT;
                resp[3] = 0; // Éxito
                resp[44..60].copy_from_slice(&ip_to_v6_mapped_bytes(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 77))));
                server.send_to(&resp, peer).await.unwrap();
            }
        });

        let res = map_ports_cascade_at(
            gw_addr,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
            4662,
            4672,
        )
        .await
        .unwrap();
        srv_handle.await.unwrap();

        assert_eq!(res.protocol, "pcp");
        assert!(res.tcp_mapped && res.udp_mapped);
        assert_eq!(res.external_ip, Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 77))));
    }

    #[tokio::test]
    async fn natpmp_fallback_when_pcp_unsupported() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let gw_addr = server.local_addr().unwrap();

        let srv_handle = tokio::spawn(async move {
            let mut buf = [0u8; 128];
            loop {
                let Ok((n, peer)) = server.recv_from(&mut buf).await else {
                    break;
                };
                if buf[0] == PCP_VERSION && n >= 24 {
                    let mut resp = [0u8; PCP_MAP_MSG_LEN];
                    resp[0] = PCP_VERSION;
                    resp[1] = PCP_OPCODE_MAP | RESPONSE_BIT;
                    resp[3] = 1; // UNSUPP_VERSION
                    server.send_to(&resp, peer).await.unwrap();
                } else if buf[0] == NATPMP_VERSION && buf[1] == NATPMP_OP_EXTERNAL_ADDR {
                    let mut resp = [0u8; NATPMP_EXT_ADDR_RESP_LEN];
                    resp[0] = NATPMP_VERSION;
                    resp[1] = NATPMP_OP_EXTERNAL_ADDR | RESPONSE_BIT;
                    resp[8..12].copy_from_slice(&[203, 0, 113, 9]);
                    server.send_to(&resp, peer).await.unwrap();
                } else if buf[0] == NATPMP_VERSION {
                    let mut resp = [0u8; NATPMP_MAP_RESP_LEN];
                    resp[0] = NATPMP_VERSION;
                    resp[1] = buf[1] | RESPONSE_BIT;
                    resp[8..10].copy_from_slice(&buf[4..6]);
                    resp[10..12].copy_from_slice(&buf[6..8]);
                    resp[12..16].copy_from_slice(&3600u32.to_be_bytes());
                    server.send_to(&resp, peer).await.unwrap();
                    if buf[1] == NATPMP_OP_MAP_UDP {
                        break;
                    }
                }
            }
        });

        let res = map_ports_cascade_at(
            gw_addr,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            4662,
            4672,
        )
        .await
        .unwrap();
        srv_handle.await.unwrap();

        assert_eq!(res.protocol, "nat-pmp");
        assert!(res.tcp_mapped && res.udp_mapped);
        assert_eq!(res.external_ip, Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9))));
    }

    #[tokio::test]
    async fn rejects_public_non_lan_gateway() {
        let public_gw: SocketAddr = "8.8.8.8:5351".parse().unwrap();
        let err = map_ports_cascade_at(
            public_gw,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
            4662,
            4672,
        )
        .await
        .unwrap_err();
        assert!(err.contains("no válida"));
    }
}

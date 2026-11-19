// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Tipos UPnP, instantánea de estado y utilidades de clasificación de IP.

use igd_next::PortMappingProtocol;
use mule_network_policy::validator::{classify_ip, IpCategory};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{SystemTime, UNIX_EPOCH};

/// Instantánea del estado de UPnP y de los mapeos de puertos activos.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct UpnpStatusSnapshot {
    /// Indica si UPnP está habilitado en los ajustes.
    pub enabled: bool,
    /// Indica si se ha descubierto un router/puerta de enlace UPnP.
    pub available: bool,
    /// Dirección del dispositivo de puerta de enlace si se ha encontrado.
    pub gateway_addr: Option<String>,
    /// IP pública WAN notificada por la puerta de enlace.
    pub external_ip: Option<String>,
    /// Indica si el puerto TCP está actualmente mapeado en la puerta de enlace.
    pub tcp_mapped: bool,
    /// Indica si el puerto UDP está actualmente mapeado en la puerta de enlace.
    pub udp_mapped: bool,
    /// Puerto TCP solicitado / mapeado.
    pub tcp_port: u16,
    /// Puerto UDP solicitado / mapeado.
    pub udp_port: u16,
    /// Indica si la puerta de enlace está tras un CGNAT (Carrier-Grade NAT / 100.64.x.x).
    pub cgnat_detected: bool,
    /// Último mensaje de error, si lo hubiera.
    pub last_error: Option<String>,
    /// Marca temporal de la última actualización (milisegundos unix).
    pub updated_at_unix_ms: u64,
    /// Protocolo activo usado para el mapeo de puertos (`"upnp-igd"`, `"pcp"`, `"nat-pmp"`).
    #[serde(default)]
    pub mapping_protocol: Option<String>,
    /// Indica si se ha abierto un *pinhole* IPv6 en el cortafuegos IGDv2.
    #[serde(default)]
    pub ipv6_pinhole_mapped: bool,
    /// Identificador único de concesión *pinhole* IPv6 devuelto por el router.
    #[serde(default)]
    pub ipv6_pinhole_id: Option<String>,
}

/// Devuelve los milisegundos unix actuales.
pub fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Comprueba si una dirección es una IP pública WAN enrutable válida.
pub fn is_valid_wan_ip(ip: IpAddr) -> bool {
    classify_ip(ip) == IpCategory::PublicRoutable
}

/// Comprueba si una dirección pertenece al rango CGNAT (100.64.0.0/10, RFC 6598).
pub fn is_cgnat_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            octets[0] == 100 && (octets[1] & 0xC0) == 64
        }
        IpAddr::V6(_) => false,
    }
}

/// Comprueba si una dirección es una dirección de puerta de enlace de red local válida (comprobación anti-IGD no autorizado).
pub fn is_valid_gateway_addr(addr: SocketAddr) -> bool {
    let cat = classify_ip(addr.ip());
    cat == IpCategory::PrivateLan || cat == IpCategory::Loopback
}

/// Descubre la dirección IPv4 local empleada para alcanzar la puerta de enlace predeterminada.
pub fn detect_primary_local_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("1.1.1.1:53").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_unspecified() => Some(v4),
        _ => None,
    }
}

/// Descubre la dirección IPv4 local empleada para comunicarse con la puerta de enlace dada.
pub fn detect_local_ipv4_for_gateway(gateway_addr: SocketAddr) -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect(gateway_addr).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_unspecified() => Some(v4),
        _ => None,
    }
}

/// Resultado de inspeccionar una regla de mapeo de puertos IGD existente en el router.
#[derive(Debug, PartialEq, Eq)]
pub enum MappingOwnership {
    /// La regla existente en la puerta de enlace es idéntica y ya mapea a nuestro extremo interno y descripción exactos.
    ExactMatch,
    /// La regla existente pertenece a eMuleRust en esta máquina, pero apunta a una IP local o puerto antiguos (concesión obsoleta a actualizar).
    StaleLease,
    /// La regla pertenece a otra aplicación o a otro host en la LAN. NO debe tocarse.
    ForeignConflict,
    /// No se encontró ninguna regla coincidente para este puerto externo / protocolo.
    NotFound,
}

/// Prefijos de descripción heredados («EMule-Rust» y luego «eMule-Rust», antes
/// del renombre a «eMuleRust»): las concesiones creadas por versiones
/// anteriores deben seguir reconociéndose como nuestras para poder ser
/// actualizadas o eliminadas.
const LEGACY_DESCRIPTION_PREFIXES: &[&str] = &["EMule-Rust", "eMule-Rust"];

/// Inspecciona si una entrada de mapeo de puertos específica coincide con los criterios esperados.
pub fn inspect_mapping_entry(
    entry_protocol: PortMappingProtocol,
    entry_external_port: u16,
    entry_internal_client: &str,
    entry_internal_port: u16,
    entry_description: &str,
    target_protocol: PortMappingProtocol,
    target_external_port: u16,
    expected_local: SocketAddrV4,
    description_prefix: &str,
) -> Option<MappingOwnership> {
    if entry_protocol != target_protocol || entry_external_port != target_external_port {
        return None;
    }

    let expected_client_ip = expected_local.ip().to_string();
    let expected_internal_port = expected_local.port();
    let is_emule_desc = entry_description.starts_with(description_prefix)
        || LEGACY_DESCRIPTION_PREFIXES
            .iter()
            .any(|p| entry_description.starts_with(p));

    if is_emule_desc {
        if entry_internal_client == expected_client_ip
            && entry_internal_port == expected_internal_port
        {
            Some(MappingOwnership::ExactMatch)
        } else {
            Some(MappingOwnership::StaleLease)
        }
    } else {
        Some(MappingOwnership::ForeignConflict)
    }
}

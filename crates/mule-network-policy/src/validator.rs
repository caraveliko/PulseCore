// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use thiserror::Error;

use crate::profile::NetworkProfile;

/// Infracciones generadas cuando una dirección o destino incumple las reglas de política de red.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PolicyViolation {
    #[error("Non-loopback address {0} is prohibited in LocalLab profile")]
    NonLoopbackDisallowedInLab(SocketAddr),

    #[error("Loopback address {0} is prohibited in public preview profiles")]
    LoopbackDisallowedInPublicProfile(SocketAddr),

    #[error("Private LAN address {0} is prohibited (anti-SSRF policy)")]
    PrivateLanDisallowed(SocketAddr),

    #[error("Link-local address {0} is prohibited")]
    LinkLocalDisallowed(SocketAddr),

    #[error("Multicast address {0} is prohibited")]
    MulticastDisallowed(SocketAddr),

    #[error("Broadcast address {0} is prohibited")]
    BroadcastDisallowed(SocketAddr),

    #[error("Unspecified / zero address {0} is prohibited as connection target")]
    UnspecifiedDisallowed(SocketAddr),

    #[error("Carrier-Grade NAT address {0} is prohibited")]
    CarrierGradeNatDisallowed(SocketAddr),

    #[error("Reserved / documentation address {0} is prohibited")]
    SpecialPurposeDisallowed(SocketAddr),

    #[error("Port 0 is invalid for socket connection")]
    InvalidPort(SocketAddr),

    #[error("Listening on non-loopback address {0} is prohibited in current configuration")]
    ListenOnNonLoopbackDisallowed(SocketAddr),
}

/// Clasifica una categoría de dirección IP para inspección de defensa en profundidad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpCategory {
    Loopback,
    PrivateLan,
    LinkLocal,
    Multicast,
    Broadcast,
    Unspecified,
    CarrierGradeNat,
    SpecialPurpose,
    PublicRoutable,
}

/// Desenvuelve direcciones IPv6 asignadas a IPv4 (::ffff:a.b.c.d) a IPv4 nativa.
pub fn canonicalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                IpAddr::V4(v4)
            } else {
                IpAddr::V6(v6)
            }
        }
        v4 => v4,
    }
}

/// Clasifica una dirección IP en categorías de seguridad y enrutamiento.
pub fn classify_ip(ip: IpAddr) -> IpCategory {
    let canonical = canonicalize_ip(ip);
    match canonical {
        IpAddr::V4(v4) => classify_ipv4(v4),
        IpAddr::V6(v6) => classify_ipv6(v6),
    }
}

fn classify_ipv4(v4: Ipv4Addr) -> IpCategory {
    let octets = v4.octets();

    if v4.is_loopback() {
        return IpCategory::Loopback;
    }
    if v4.is_unspecified() {
        return IpCategory::Unspecified;
    }
    if v4.is_broadcast() {
        return IpCategory::Broadcast;
    }
    if v4.is_multicast() {
        return IpCategory::Multicast;
    }

    // LAN privada RFC 1918
    // 10.0.0.0/8
    if octets[0] == 10 {
        return IpCategory::PrivateLan;
    }
    // 172.16.0.0/12
    if octets[0] == 172 && (16..=31).contains(&octets[1]) {
        return IpCategory::PrivateLan;
    }
    // 192.168.0.0/16
    if octets[0] == 192 && octets[1] == 168 {
        return IpCategory::PrivateLan;
    }

    // NAT para operadores (Carrier-Grade NAT) RFC 6598: 100.64.0.0/10 (100.64.0.0 a 100.127.255.255)
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return IpCategory::CarrierGradeNat;
    }

    // Enlace local RFC 3927: 169.254.0.0/16
    if octets[0] == 169 && octets[1] == 254 {
        return IpCategory::LinkLocal;
    }

    // Propósito especial / documentación / pruebas de rendimiento (benchmarking)
    // 192.0.2.0/24 (TEST-NET-1), 198.51.100.0/24 (TEST-NET-2), 203.0.113.0/24 (TEST-NET-3)
    if (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
        // 198.18.0.0/15 benchmarking
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        // 240.0.0.0/4 reservado
        || (octets[0] >= 240)
    {
        return IpCategory::SpecialPurpose;
    }

    IpCategory::PublicRoutable
}

fn classify_ipv6(v6: Ipv6Addr) -> IpCategory {
    if v6.is_loopback() {
        return IpCategory::Loopback;
    }
    if v6.is_unspecified() {
        return IpCategory::Unspecified;
    }
    if v6.is_multicast() {
        return IpCategory::Multicast;
    }

    let segments = v6.segments();

    // Enlace local: fe80::/10
    if (segments[0] & 0xffc0) == 0xfe80 {
        return IpCategory::LinkLocal;
    }

    // Direcciones locales únicas (ULA): fc00::/7 (RFC 4193, IPv6 privada)
    if (segments[0] & 0xfe00) == 0xfc00 {
        return IpCategory::PrivateLan;
    }

    // Documentación RFC 3849: 2001:db8::/32
    if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        return IpCategory::SpecialPurpose;
    }

    IpCategory::PublicRoutable
}

/// Valida si una dirección está autorizada para una conexión saliente bajo el perfil dado.
pub fn validate_connect_address(
    addr: &SocketAddr,
    profile: NetworkProfile,
) -> Result<(), PolicyViolation> {
    if addr.port() == 0 {
        return Err(PolicyViolation::InvalidPort(*addr));
    }

    let cat = classify_ip(addr.ip());

    match profile {
        NetworkProfile::LocalLab => {
            if cat != IpCategory::Loopback {
                return Err(PolicyViolation::NonLoopbackDisallowedInLab(*addr));
            }
            Ok(())
        }
        NetworkProfile::PublicEd2kPreview | NetworkProfile::PublicEd2kKadPreview => match cat {
            IpCategory::Loopback => Err(PolicyViolation::LoopbackDisallowedInPublicProfile(*addr)),
            IpCategory::PrivateLan => Err(PolicyViolation::PrivateLanDisallowed(*addr)),
            IpCategory::LinkLocal => Err(PolicyViolation::LinkLocalDisallowed(*addr)),
            IpCategory::Multicast => Err(PolicyViolation::MulticastDisallowed(*addr)),
            IpCategory::Broadcast => Err(PolicyViolation::BroadcastDisallowed(*addr)),
            IpCategory::Unspecified => Err(PolicyViolation::UnspecifiedDisallowed(*addr)),
            IpCategory::CarrierGradeNat => Err(PolicyViolation::CarrierGradeNatDisallowed(*addr)),
            IpCategory::SpecialPurpose => Err(PolicyViolation::SpecialPurposeDisallowed(*addr)),
            IpCategory::PublicRoutable => Ok(()),
        },
    }
}

/// Valida si una dirección está autorizada para vincularse/escuchar bajo el perfil dado.
pub fn validate_listen_address(
    addr: &SocketAddr,
    profile: NetworkProfile,
) -> Result<(), PolicyViolation> {
    let cat = classify_ip(addr.ip());

    match profile {
        NetworkProfile::LocalLab => {
            // Los listeners en perfil Lab solo deben vincularse a loopback
            if cat != IpCategory::Loopback {
                return Err(PolicyViolation::ListenOnNonLoopbackDisallowed(*addr));
            }
            Ok(())
        }
        NetworkProfile::PublicEd2kPreview | NetworkProfile::PublicEd2kKadPreview => {
            // En fase preview WAN, los listeners deben permanecer en loopback hasta que la escucha WAN se habilite específicamente
            // o si se vincula explícitamente a 0.0.0.0 / :: para tráfico entrante.
            // Para la base defensiva de la Fase 9.0, no permitir vincularse a interfaces de LAN privada o no loopback.
            if cat != IpCategory::Loopback && cat != IpCategory::Unspecified {
                return Err(PolicyViolation::ListenOnNonLoopbackDisallowed(*addr));
            }
            Ok(())
        }
    }
}

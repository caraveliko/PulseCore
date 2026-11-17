// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Guardia anti-SSRF para las descargas HTTP(S) activadas por RPC
//! (`servers.updateFromUrl`, `kad.bootstrapFromUrl`).
//!
//! La validación se realiza en dos fases:
//! 1. [`parse_fetch_url`] — sintaxis: únicamente esquema http/https, host
//!    presente, sin userinfo (se rechaza `http://user@host/`).
//! 2. [`validate_fetch_ips`] — cada IP (literal o procedente del DNS) se
//!    clasifica y se contrasta con el perfil de red, exactamente igual que
//!    `validate_connect_address`: `LocalLab` solo acepta loopback y los
//!    perfiles públicos solo aceptan `IpCategory::PublicRoutable`.
//!
//! El llamador debe a continuación **fijar** las direcciones validadas en el
//! cliente HTTP (reqwest `resolve_to`) y **prohibir las redirecciones**:
//! sin ello, una URL validada podría delegar hacia un destino interno.

use std::net::IpAddr;
use thiserror::Error;
use url::Url;

use crate::profile::NetworkProfile;
use crate::validator::{classify_ip, IpCategory};

/// Puerto HTTP implícito cuando la URL no lo especifica.
pub const DEFAULT_HTTP_PORT: u16 = 80;
/// Puerto HTTPS implícito cuando la URL no lo especifica.
pub const DEFAULT_HTTPS_PORT: u16 = 443;

/// Errores de validación de una URL de obtención externa.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FetchPolicyError {
    /// URL sintácticamente inválida o sin host.
    #[error("Invalid fetch URL: {0}")]
    InvalidUrl(String),

    /// Esquema distinto de http/https (file://, gopher://, …).
    #[error("Forbidden URL scheme '{0}' — only http/https are allowed")]
    ForbiddenScheme(String),

    /// URL que contiene datos de usuario (userinfo) (`http://user:pass@host/`).
    #[error("Userinfo in fetch URL is prohibited")]
    UserinfoDisallowed,

    /// No se proporcionó ninguna IP para un host no literal.
    #[error("Host '{0}' resolved to no IP address")]
    UnresolvableHost(String),

    /// Al menos una IP resuelta infringe la política del perfil actual.
    #[error("Resolved address {ip} for '{host}' is prohibited: {reason}")]
    AddressViolation {
        /// Host solicitado.
        host: String,
        /// IP resuelta rechazada.
        ip: IpAddr,
        /// Categoría de dirección rechazada.
        reason: &'static str,
    },
}

/// URL de obtención normalizada y validada sintácticamente.
#[derive(Debug, Clone)]
pub struct FetchUrl {
    /// URL original reanalizada (tal como se envía al cliente HTTP).
    pub url: Url,
    /// Host canónico (sin corchetes IPv6).
    pub host: String,
    /// Puerto efectivo (explícito u 80/443 por defecto).
    pub port: u16,
    /// IP literal si el host es una; omite la resolución DNS.
    pub literal_ip: Option<IpAddr>,
}

/// Analiza y valida sintácticamente una URL de obtención externa.
pub fn parse_fetch_url(raw: &str) -> Result<FetchUrl, FetchPolicyError> {
    let url = Url::parse(raw.trim())
        .map_err(|e| FetchPolicyError::InvalidUrl(format!("{raw}: {e}")))?;

    match url.scheme() {
        "http" | "https" => {}
        other => return Err(FetchPolicyError::ForbiddenScheme(other.to_string())),
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err(FetchPolicyError::UserinfoDisallowed);
    }

    let host = url
        .host_str()
        .ok_or_else(|| FetchPolicyError::InvalidUrl(format!("{raw}: missing host")))?
        .to_string();

    let port = url.port_or_known_default().unwrap_or(match url.scheme() {
        "https" => DEFAULT_HTTPS_PORT,
        _ => DEFAULT_HTTP_PORT,
    });

    let literal_ip = match url.host() {
        Some(url::Host::Ipv4(v4)) => Some(IpAddr::V4(v4)),
        Some(url::Host::Ipv6(v6)) => Some(IpAddr::V6(v6)),
        _ => None,
    };

    Ok(FetchUrl {
        url,
        host,
        port,
        literal_ip,
    })
}

/// Comprueba que cada IP candidata esté autorizada por el perfil actual.
///
/// `resolved` = direcciones DNS actuales; se ignora cuando `target` contiene una
/// IP literal. Todas las direcciones deben ser válidas: una única entrada DNS
/// pública junto con una privada basta para rechazarla (el round-robin DNS
/// podría alcanzar el host interno en caso contrario).
pub fn validate_fetch_ips(
    target: &FetchUrl,
    resolved: &[IpAddr],
    profile: NetworkProfile,
) -> Result<(), FetchPolicyError> {
    let candidates: Vec<IpAddr> = match target.literal_ip {
        Some(ip) => vec![ip],
        None => {
            if resolved.is_empty() {
                return Err(FetchPolicyError::UnresolvableHost(target.host.clone()));
            }
            resolved.to_vec()
        }
    };

    for ip in &candidates {
        let cat = classify_ip(*ip);
        let allowed = match profile {
            NetworkProfile::LocalLab => cat == IpCategory::Loopback,
            NetworkProfile::PublicEd2kPreview | NetworkProfile::PublicEd2kKadPreview => {
                cat == IpCategory::PublicRoutable
            }
        };
        if !allowed {
            return Err(FetchPolicyError::AddressViolation {
                host: target.host.clone(),
                ip: *ip,
                reason: category_reason(cat),
            });
        }
    }
    Ok(())
}

/// Etiqueta corta de la categoría de dirección para los mensajes de error.
fn category_reason(cat: IpCategory) -> &'static str {
    match cat {
        IpCategory::Loopback => "loopback",
        IpCategory::PrivateLan => "private LAN",
        IpCategory::LinkLocal => "link-local",
        IpCategory::Multicast => "multicast",
        IpCategory::Broadcast => "broadcast",
        IpCategory::Unspecified => "unspecified",
        IpCategory::CarrierGradeNat => "carrier-grade NAT",
        IpCategory::SpecialPurpose => "special-purpose/reserved",
        IpCategory::PublicRoutable => "public",
    }
}

/// Direcciones a fijar en el cliente HTTP tras la validación:
/// la lista validada tal cual, o la IP literal sola.
pub fn pinned_ips(target: &FetchUrl, resolved: &[IpAddr]) -> Vec<IpAddr> {
    match target.literal_ip {
        Some(ip) => vec![ip],
        None => resolved.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn parse_rejects_non_http_schemes() {
        for raw in [
            "file:///c:/windows/win.ini",
            "gopher://host/x",
            "ftp://host/x",
            "ed2k://|server|1.2.3.4|4661|/",
        ] {
            // Rechazo garantizado — `InvalidUrl` (sintaxis no válida como los
            // enlaces ed2k) o `ForbiddenScheme` según la capacidad de análisis.
            assert!(parse_fetch_url(raw).is_err(), "{raw} must be rejected");
        }
        assert!(matches!(
            parse_fetch_url("file:///c:/windows/win.ini"),
            Err(FetchPolicyError::ForbiddenScheme(_))
        ));
    }

    #[test]
    fn parse_rejects_userinfo() {
        assert!(matches!(
            parse_fetch_url("http://user@1.2.3.4/server.met"),
            Err(FetchPolicyError::UserinfoDisallowed)
        ));
        assert!(matches!(
            parse_fetch_url("http://user:pass@1.2.3.4/x"),
            Err(FetchPolicyError::UserinfoDisallowed)
        ));
    }

    #[test]
    fn parse_defaults_ports() {
        let http = parse_fetch_url("http://upd.emule-security.org/server.met").unwrap();
        assert_eq!(http.port, DEFAULT_HTTP_PORT);
        assert_eq!(http.host, "upd.emule-security.org");
        let https = parse_fetch_url("https://example.com:8443/nodes.dat").unwrap();
        assert_eq!(https.port, 8443);
    }

    #[test]
    fn literal_ip_detected() {
        let t = parse_fetch_url("http://169.254.169.254/latest/meta-data").unwrap();
        assert_eq!(t.literal_ip, Some(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))));
        let v6 = parse_fetch_url("http://[::1]/x").unwrap();
        assert_eq!(v6.literal_ip, Some(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn locallab_accepts_only_loopback() {
        let loop_url = parse_fetch_url("http://127.0.0.1:4661/server.met").unwrap();
        assert!(validate_fetch_ips(&loop_url, &[], NetworkProfile::LocalLab).is_ok());

        let pub_url = parse_fetch_url("http://8.8.8.8/x").unwrap();
        assert!(matches!(
            validate_fetch_ips(&pub_url, &[], NetworkProfile::LocalLab),
            Err(FetchPolicyError::AddressViolation { .. })
        ));

        let priv_url = parse_fetch_url("http://192.168.1.1/x").unwrap();
        assert!(validate_fetch_ips(&priv_url, &[], NetworkProfile::LocalLab).is_err());
    }

    #[test]
    fn public_profile_rejects_internal_targets() {
        let prof = NetworkProfile::PublicEd2kKadPreview;
        for raw in [
            "http://169.254.169.254/latest/meta-data",
            "http://192.168.1.1/server.met",
            "http://10.0.0.5/x",
            "http://127.0.0.1/x",
            "http://[fd00::1]/x",
            "http://100.64.1.1/x",
        ] {
            let t = parse_fetch_url(raw).unwrap();
            assert!(
                validate_fetch_ips(&t, &[], prof).is_err(),
                "{raw} should be rejected in public profile"
            );
        }
        let ok = parse_fetch_url("http://8.8.8.8/server.met").unwrap();
        assert!(validate_fetch_ips(&ok, &[], prof).is_ok());
    }

    #[test]
    fn hostname_requires_nonempty_resolutions() {
        let t = parse_fetch_url("http://example.org/x").unwrap();
        assert!(matches!(
            validate_fetch_ips(&t, &[], NetworkProfile::PublicEd2kKadPreview),
            Err(FetchPolicyError::UnresolvableHost(h)) if h == "example.org"
        ));
    }

    #[test]
    fn single_bad_resolved_ip_rejects_whole_host() {
        let t = parse_fetch_url("http://evil.example/x").unwrap();
        let mixed = [
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
        ];
        assert!(matches!(
            validate_fetch_ips(&t, &mixed, NetworkProfile::PublicEd2kKadPreview),
            Err(FetchPolicyError::AddressViolation { ip, .. }) if ip == IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1))
        ));
    }
}

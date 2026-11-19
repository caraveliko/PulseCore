// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Cliente SOAP IGDv2 `WANIPv6FirewallControl:1` para apertura de *pinholes* IPv6.
//!
//! Mantiene validación anti-SSRF estricta sobre la URL de control antes de emitir
//! cualquier petición HTTP, sin redirecciones permitidas y con tamaño de cuerpo acotado.

use std::net::{IpAddr, Ipv6Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use mule_network_policy::validator::{classify_ip, IpCategory};

use super::types::is_valid_gateway_addr;

/// Duración de concesión predeterminada para un *pinhole* IPv6 (en segundos).
pub const DEFAULT_PINHOLE_LEASE_SECS: u32 = 3600;
/// Tiempo máximo de espera por petición SOAP IGDv2.
const PINHOLE_HTTP_TIMEOUT: Duration = Duration::from_secs(3);
/// Tamaño máximo de cuerpo XML devuelto por el router IGDv2 (64 KiB).
const MAX_SOAP_RESPONSE_BYTES: usize = 64 * 1024;
/// Tipo de servicio SOAP IGDv2 estándar.
const SERVICE_TYPE: &str = "urn:schemas-upnp-org:service:WANIPv6FirewallControl:1";

/// Solicitud de apertura de un *pinhole* IPv6 entrante en el router.
#[derive(Debug, Clone)]
pub struct Ipv6PinholeRequest {
    /// URL HTTP de control `WANIPv6FirewallControl:1` en la LAN.
    pub gateway_control_url: String,
    /// Dirección IPv6 unicast global local del equipo.
    pub internal_client: Ipv6Addr,
    /// Puerto de destino interno a autorizar.
    pub internal_port: u16,
    /// Número de protocolo IANA (`6` = TCP, `17` = UDP).
    pub protocol: u8,
    /// Tiempo de vida solicitado en segundos.
    pub lease_secs: u32,
}

/// Detecta si el equipo dispone de una dirección IPv6 global enrutable mediante consulta
/// de ruta al kernel (sin transmitir datagramas por la red).
pub fn detect_local_global_ipv6() -> Option<Ipv6Addr> {
    let sock = UdpSocket::bind("[::]:0").ok()?;
    sock.connect("[2606:4700:4700::1111]:53").ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V6(v6) if classify_ip(IpAddr::V6(v6)) == IpCategory::PublicRoutable => Some(v6),
        _ => None,
    }
}

fn validate_control_url(raw_url: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(raw_url)
        .map_err(|e| format!("URL de control IGDv2 inválida '{raw_url}': {e}"))?;
    if url.scheme() != "http" {
        return Err(format!("Esquema IGDv2 no permitido: {}", url.scheme()));
    }
    let host = url
        .host_str()
        .ok_or_else(|| "La URL de control IGDv2 carece de host".to_string())?;
    let trimmed = host.trim_start_matches('[').trim_end_matches(']');
    let ip: IpAddr = trimmed.parse().map_err(|_| {
        format!("Host IGDv2 '{host}' rechazado: solo se admiten literales IP LAN/loopback")
    })?;
    let port = url.port_or_known_default().unwrap_or(80);
    let sock_addr = SocketAddr::new(ip, port);
    if !is_valid_gateway_addr(sock_addr) {
        return Err(format!(
            "URL de control IGDv2 rechazada por política anti-SSRF: {sock_addr}"
        ));
    }
    Ok(url)
}

async fn send_soap_action(control_url: &str, action: &str, inner_xml: &str) -> Result<String, String> {
    let validated = validate_control_url(control_url)?;
    let envelope = format!(
        "<?xml version=\"1.0\"?>\
<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
<s:Body><u:{action} xmlns:u=\"{SERVICE_TYPE}\">{inner_xml}</u:{action}></s:Body></s:Envelope>"
    );

    let client = reqwest::Client::builder()
        .timeout(PINHOLE_HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Error creando cliente HTTP IGDv2: {e}"))?;

    let resp = client
        .post(validated)
        .header("Content-Type", "text/xml; charset=\"utf-8\"")
        .header("SOAPACTION", format!("\"{SERVICE_TYPE}#{action}\""))
        .body(envelope)
        .send()
        .await
        .map_err(|e| format!("Error en petición SOAP IGDv2 {action}: {e}"))?;

    let status = resp.status();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Error leyendo respuesta SOAP {action}: {e}"))?;
    if bytes.len() > MAX_SOAP_RESPONSE_BYTES {
        return Err("Respuesta SOAP IGDv2 excede el límite de tamaño".to_string());
    }
    let body = String::from_utf8_lossy(&bytes).into_owned();
    if !status.is_success() {
        return Err(format!("SOAP {action} falló con HTTP {status}"));
    }
    Ok(body)
}

fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let rest = &xml[start..];
    let end = rest.find(&close)?;
    let val = rest[..end].trim();
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

/// Solicita abrir un *pinhole* IPv6 (`AddPinhole`) y devuelve el `UniqueID` asignado por el router.
pub async fn add_ipv6_pinhole(req: &Ipv6PinholeRequest) -> Result<String, String> {
    let lease = req.lease_secs.max(60);
    let inner = format!(
        "<RemoteHost></RemoteHost>\
<RemotePort>0</RemotePort>\
<InternalClient>{}</InternalClient>\
<InternalPort>{}</InternalPort>\
<Protocol>{}</Protocol>\
<LeaseTime>{}</LeaseTime>",
        req.internal_client, req.internal_port, req.protocol, lease
    );
    let xml = send_soap_action(&req.gateway_control_url, "AddPinhole", &inner).await?;
    extract_xml_tag(&xml, "UniqueID")
        .ok_or_else(|| "Respuesta AddPinhole sin elemento <UniqueID>".to_string())
}

/// Renueva el tiempo de concesión de un *pinhole* IPv6 existente (`UpdatePinhole`).
pub async fn update_ipv6_pinhole(
    gateway_control_url: &str,
    unique_id: &str,
    lease_secs: u32,
) -> Result<(), String> {
    let inner = format!(
        "<UniqueID>{unique_id}</UniqueID><NewLeaseTime>{}</NewLeaseTime>",
        lease_secs.max(60)
    );
    send_soap_action(gateway_control_url, "UpdatePinhole", &inner)
        .await
        .map(|_| ())
}

/// Elimina un *pinhole* IPv6 previamente abierto (`DeletePinhole`).
pub async fn delete_ipv6_pinhole(gateway_control_url: &str, unique_id: &str) -> Result<(), String> {
    let inner = format!("<UniqueID>{unique_id}</UniqueID>");
    send_soap_action(gateway_control_url, "DeletePinhole", &inner)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn adds_and_updates_ipv6_pinhole_via_loopback_soap_mock() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let srv = tokio::spawn(async move {
            for idx in 0..2 {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap();
                let req_str = String::from_utf8_lossy(&buf[..n]);
                let xml = if idx == 0 {
                    assert!(req_str.contains("AddPinhole"));
                    "<?xml version=\"1.0\"?><s:Envelope><s:Body><u:AddPinholeResponse><UniqueID>pin-42</UniqueID></u:AddPinholeResponse></s:Body></s:Envelope>"
                } else {
                    assert!(req_str.contains("UpdatePinhole"));
                    assert!(req_str.contains("pin-42"));
                    "<?xml version=\"1.0\"?><s:Envelope><s:Body><u:UpdatePinholeResponse/></s:Body></s:Envelope>"
                };
                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    xml.len(),
                    xml
                );
                sock.write_all(http_resp.as_bytes()).await.unwrap();
            }
        });

        let ctl = format!("http://{addr}/ctl/IPv6FC");
        let id = add_ipv6_pinhole(&Ipv6PinholeRequest {
            gateway_control_url: ctl.clone(),
            internal_client: "2001:db8::10".parse().unwrap(),
            internal_port: 4662,
            protocol: 6,
            lease_secs: 3600,
        })
        .await
        .unwrap();
        assert_eq!(id, "pin-42");

        update_ipv6_pinhole(&ctl, &id, 3600).await.unwrap();
        srv.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_ssrf_control_urls() {
        for bad in [
            "http://169.254.169.254/latest/meta-data",
            "http://8.8.8.8/ctl",
            "http://router.example/ctl",
            "https://192.168.1.1/ctl",
        ] {
            let err = validate_control_url(bad).unwrap_err();
            assert!(!err.is_empty());
        }
    }
}

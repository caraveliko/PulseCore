// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Obtención HTTP(S) acotada y validada para las importaciones RPC remotas
//! (`servers.updateFromUrl`, `kad.bootstrapFromUrl`).
//!
//! Flujo: [`parse_fetch_url`] → resolución DNS →
//! [`validate_fetch_ips`] → reqwest con **redirecciones deshabilitadas** e
//! **IPs fijadas** (`resolve_to`, anti DNS-rebind) → cuerpo leído en
//! streaming con límite [`MAX_FETCH_BODY_BYTES`].

use std::net::SocketAddr;

use mule_network_policy::{
    parse_fetch_url, pinned_ips, validate_fetch_ips, FetchUrl,
};

use crate::error::JsonRpcError;
use crate::state::AppState;

/// Tamaño máximo del cuerpo de una respuesta descargada (server.met /
/// nodes.dat quedan muy por debajo; 16 MiB cubre listas
/// voluminosas acotando al mismo tiempo la asignación de memoria).
pub const MAX_FETCH_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Tiempo de espera (timeout) global de la petición de descarga.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Descarga `raw_url` tras la validación anti-SSRF del perfil actual y
/// devuelve el cuerpo acotado. No se emite ningún byte antes de la validación.
pub async fn fetch_url_bytes(
    state: &AppState,
    raw_url: &str,
) -> Result<(FetchUrl, Vec<u8>), JsonRpcError> {
    if matches!(
        state.network_policy.state(),
        mule_network_policy::NetworkState::Disabled
            | mule_network_policy::NetworkState::ShuttingDown
    ) {
        return Err(JsonRpcError::business(
            "Red deshabilitada por política de red (kill switch)",
        ));
    }
    let target = parse_fetch_url(raw_url).map_err(|e| JsonRpcError::business(e.to_string()))?;

    // Resolución DNS si el host no es una IP literal — todas las
    // direcciones devueltas deben superar la directiva (una entrada
    // privada en un round-robin basta para rechazar).
    let resolved: Vec<std::net::IpAddr> = match target.literal_ip {
        Some(ip) => vec![ip],
        None => tokio::net::lookup_host(format!("{}:{}", target.host, target.port))
            .await
            .map(|addrs| addrs.map(|sa| sa.ip()).collect())
            .unwrap_or_default(),
    };

    validate_fetch_ips(&target, &resolved, state.network_policy.profile())
        .map_err(|e| JsonRpcError::business(e.to_string()))?;

    // Fija la primera IP validada: el cliente no vuelve a resolver, por lo que
    // un rebind DNS posterior no puede desviar la conexión.
    let pinned: Option<SocketAddr> = pinned_ips(&target, &resolved)
        .into_iter()
        .next()
        .map(|ip| SocketAddr::new(ip, target.port));

    let mut builder = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    if let Some(addr) = pinned {
        builder = builder.resolve(&target.host, addr);
    }
    let client = builder
        .build()
        .map_err(|e| JsonRpcError::business(format!("HTTP client build error: {e}")))?;

    let mut response = client
        .get(target.url.clone())
        .send()
        .await
        .map_err(|e| JsonRpcError::business(format!("Failed to fetch {}: {e}", target.url)))?;

    if !response.status().is_success() {
        return Err(JsonRpcError::business(format!(
            "HTTP request to {} returned status {}",
            target.url,
            response.status()
        )));
    }

    if let Some(len) = response.content_length() {
        if len > MAX_FETCH_BODY_BYTES as u64 {
            return Err(JsonRpcError::business(format!(
                "Response from {} exceeds {MAX_FETCH_BODY_BYTES} bytes",
                target.url
            )));
        }
    }

    // Lectura acotada: content-length puede estar ausente o ser engañoso.
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| JsonRpcError::business(format!("Failed to read response body: {e}")))?
    {
        if body.len() + chunk.len() > MAX_FETCH_BODY_BYTES {
            return Err(JsonRpcError::business(format!(
                "Response from {} exceeds {MAX_FETCH_BODY_BYTES} bytes",
                target.url
            )));
        }
        body.extend_from_slice(&chunk);
    }

    Ok((target, body))
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Cliente HTTP acotado y validado frente a SSRF para consultas y descargas de actualización.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use mule_network_policy::{
    parse_fetch_url, pinned_ips, validate_fetch_ips, NetworkPolicyManager, NetworkState,
};

use super::constants::MAX_UPDATE_REDIRECTS;
use super::error::UpdaterError;

/// Obtiene una respuesta HTTP(S) tras validar estado de red, esquema, IPs resueltas y redirecciones.
pub async fn fetch_validated_response(
    raw_url: &str,
    timeout: Duration,
    network_policy: &NetworkPolicyManager,
) -> Result<reqwest::Response, UpdaterError> {
    if matches!(
        network_policy.state(),
        NetworkState::Disabled | NetworkState::ShuttingDown
    ) {
        return Err(UpdaterError::NetworkBlocked(
            "La red del demonio está deshabilitada por el interruptor de corte".to_string(),
        ));
    }

    let profile = network_policy.profile();
    let mut current_url = raw_url.trim().to_string();

    for _hop in 0..=MAX_UPDATE_REDIRECTS {
        let target =
            parse_fetch_url(&current_url).map_err(|e| UpdaterError::NetworkBlocked(e.to_string()))?;

        let resolved: Vec<IpAddr> = match target.literal_ip {
            Some(ip) => vec![ip],
            None => tokio::net::lookup_host(format!("{}:{}", target.host, target.port))
                .await
                .map(|addrs| addrs.map(|sa| sa.ip()).collect())
                .unwrap_or_default(),
        };

        validate_fetch_ips(&target, &resolved, profile)
            .map_err(|e| UpdaterError::NetworkBlocked(e.to_string()))?;

        let mut builder = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(format!("PulseCore/{}", crate::rpc::methods::DAEMON_VERSION));

        if target.literal_ip.is_none() {
            if let Some(ip) = pinned_ips(&target, &resolved).into_iter().next() {
                builder = builder.resolve(&target.host, SocketAddr::new(ip, target.port));
            }
        }

        let client = builder.build()?;
        let response = client.get(target.url.clone()).send().await?;

        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|val| val.to_str().ok())
                .map(str::trim)
                .filter(|val| !val.is_empty())
                .ok_or_else(|| UpdaterError::HttpStatus {
                    status: response.status().as_u16(),
                    url: current_url.clone(),
                })?;

            let next_url = target
                .url
                .join(location)
                .map_err(|e| {
                    UpdaterError::NetworkBlocked(format!(
                        "Redirección inválida desde {}: {e}",
                        target.url
                    ))
                })?
                .to_string();
            current_url = next_url;
            continue;
        }

        if !response.status().is_success() {
            return Err(UpdaterError::HttpStatus {
                status: response.status().as_u16(),
                url: current_url,
            });
        }

        return Ok(response);
    }

    Err(UpdaterError::TooManyRedirects {
        max: MAX_UPDATE_REDIRECTS,
    })
}

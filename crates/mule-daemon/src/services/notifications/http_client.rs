// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Cliente HTTP saliente con protección anti-SSRF, fijación de IP y reintentos con retroceso.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use mule_network_policy::{
    parse_fetch_url, pinned_ips, validate_fetch_ips, NetworkPolicyManager, NetworkState,
};

use super::constants::{
    DISPATCH_HTTP_TIMEOUT_SECS, HTTP_SERVER_ERROR_MIN, HTTP_TOO_MANY_REQUESTS,
    MAX_ERROR_RESPONSE_BYTES, MAX_EXTERNAL_RETRIES, RETRY_BACKOFF_BASE_MS,
};
use super::error::NotificationError;

/// Determina si un fallo justifica un nuevo intento automático.
fn should_retry(err: &NotificationError) -> bool {
    match err {
        NotificationError::Transport(_) => true,
        NotificationError::HttpStatus { status, .. } => {
            *status == HTTP_TOO_MANY_REQUESTS || *status >= HTTP_SERVER_ERROR_MIN
        }
        NotificationError::NetworkBlocked(_)
        | NotificationError::InvalidChannel(_)
        | NotificationError::NotConfigured(_)
        | NotificationError::Storage(_)
        | NotificationError::Serialization(_) => false,
    }
}

/// Lee un extracto acotado de la respuesta de error HTTP para diagnóstico.
async fn read_bounded_error_detail(mut response: reqwest::Response) -> String {
    let mut detail = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        let remaining = MAX_ERROR_RESPONSE_BYTES.saturating_sub(detail.len());
        if remaining == 0 {
            break;
        }
        let take = remaining.min(chunk.len());
        detail.extend_from_slice(&chunk[..take]);
        if detail.len() >= MAX_ERROR_RESPONSE_BYTES {
            break;
        }
    }
    let text = String::from_utf8_lossy(&detail).trim().to_string();
    if text.is_empty() {
        "sin detalles adicionales".to_string()
    } else {
        text
    }
}

/// Envía una petición HTTP POST tras validar estado del *kill switch*, esquema, DNS y fijar IP.
pub async fn post_validated(
    raw_url: &str,
    headers: &[(String, String)],
    body: &[u8],
    content_type: &str,
    network_policy: &NetworkPolicyManager,
) -> Result<(), NotificationError> {
    if matches!(
        network_policy.state(),
        NetworkState::Disabled | NetworkState::ShuttingDown
    ) {
        return Err(NotificationError::NetworkBlocked(
            "La red del demonio está deshabilitada por el interruptor de corte".to_string(),
        ));
    }

    let target =
        parse_fetch_url(raw_url).map_err(|e| NotificationError::NetworkBlocked(e.to_string()))?;

    let resolved: Vec<IpAddr> = match target.literal_ip {
        Some(ip) => vec![ip],
        None => tokio::net::lookup_host(format!("{}:{}", target.host, target.port))
            .await
            .map(|addrs| addrs.map(|sa| sa.ip()).collect())
            .unwrap_or_default(),
    };

    validate_fetch_ips(&target, &resolved, network_policy.profile())
        .map_err(|e| NotificationError::NetworkBlocked(e.to_string()))?;

    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(DISPATCH_HTTP_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("PulseCore/{}", crate::rpc::methods::DAEMON_VERSION));

    if target.literal_ip.is_none() {
        if let Some(ip) = pinned_ips(&target, &resolved).into_iter().next() {
            builder = builder.resolve(&target.host, SocketAddr::new(ip, target.port));
        }
    }

    let client = builder.build()?;
    let mut req = client
        .post(target.url)
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .body(body.to_vec());

    for (name, value) in headers {
        req = req.header(name.as_str(), value.as_str());
    }

    let response = req.send().await?;
    let status = response.status();
    if !status.is_success() {
        let detail = read_bounded_error_detail(response).await;
        return Err(NotificationError::HttpStatus {
            status: status.as_u16(),
            detail,
        });
    }

    Ok(())
}

/// Envía una petición HTTP POST con reintentos exponenciales ante errores transitorios.
pub async fn post_with_retries(
    raw_url: &str,
    headers: &[(String, String)],
    body: &[u8],
    content_type: &str,
    network_policy: &NetworkPolicyManager,
) -> Result<(), NotificationError> {
    let mut attempt: u32 = 0;
    loop {
        match post_validated(raw_url, headers, body, content_type, network_policy).await {
            Ok(()) => return Ok(()),
            Err(err) => {
                if attempt >= MAX_EXTERNAL_RETRIES || !should_retry(&err) {
                    return Err(err);
                }
                let delay_ms = RETRY_BACKOFF_BASE_MS.saturating_mul(1u64 << attempt);
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }
}

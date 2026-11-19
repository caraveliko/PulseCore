// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Manejador HTTP autenticado para instalar actualizaciones desde un binario compilado local.

use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;

use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::json;

use crate::rpc::handlers::updater::schedule_graceful_restart;
use crate::rpc::server::{is_loopback_ip, ClientIp};
use crate::services::updater::{
    UpdaterError, HEADER_UPDATE_AUTO_RESTART, HEADER_UPDATE_FILENAME, HEADER_UPDATE_SHA256,
};
use crate::state::AppState;

/// Comprueba si la petición HTTP está autorizada para subir un binario ejecutable.
fn is_upload_authorized(state: &AppState, headers: &HeaderMap, client_ip: IpAddr) -> bool {
    let expected = state.config.auth_token.as_str();
    if expected.is_empty() {
        return !(state.has_webui_password() && !is_loopback_ip(client_ip));
    }

    let Some(auth_hdr) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };

    let Some(provided) = auth_hdr.strip_prefix("Bearer ").map(str::trim) else {
        return false;
    };

    let (a, b) = (provided.as_bytes(), expected.as_bytes());
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// Determina el código HTTP semántico correspondiente a un error del actualizador.
fn status_code_for_updater_error(err: &UpdaterError) -> StatusCode {
    match err {
        UpdaterError::EmptyPayload
        | UpdaterError::InvalidExecutableFormat(_)
        | UpdaterError::ArchitectureMismatch { .. }
        | UpdaterError::ChecksumMismatch { .. } => StatusCode::BAD_REQUEST,
        UpdaterError::PayloadTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
        UpdaterError::UpdateInProgress => StatusCode::CONFLICT,
        UpdaterError::PermissionDenied(_) => StatusCode::FORBIDDEN,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Maneja `POST /api/system/upload-update`: valida ELF/PE, arquitectura y aplica sustitución atómica.
pub(crate) async fn handle_upload_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    ext_ip: Option<axum::Extension<ClientIp>>,
    body: Bytes,
) -> impl IntoResponse {
    let client_ip = ext_ip
        .map(|e| e.0 .0)
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));

    if !is_upload_authorized(&state, &headers, client_ip) {
        tracing::warn!(
            client_ip = %client_ip,
            "[SECURITY] Rechazada subida de actualización sin token Bearer válido"
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "ok": false,
                "error": "Autenticación Bearer requerida o token no válido"
            })),
        );
    }

    let expected_sha256 = headers
        .get(HEADER_UPDATE_SHA256)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let filename = headers
        .get(HEADER_UPDATE_FILENAME)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Path::new(s).file_name())
        .and_then(|s| s.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let auto_restart = headers
        .get(HEADER_UPDATE_AUTO_RESTART)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            let clean = v.trim();
            !clean.eq_ignore_ascii_case("false") && clean != "0"
        })
        .unwrap_or(true);

    match state
        .updater
        .stage_and_apply_local_binary(&body, expected_sha256, filename)
        .await
    {
        Ok(applied) => {
            if auto_restart {
                schedule_graceful_restart(&state);
            }
            let message = if auto_restart {
                if applied.managed_by_systemd {
                    "Binario validado e instalado correctamente. systemd está reiniciando Pulse Core."
                } else {
                    "Binario validado e instalado correctamente. Pulse Core se está reiniciando."
                }
            } else {
                "Binario validado e instalado en disco. Se aplicará en el próximo reinicio."
            };
            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "restarting": auto_restart,
                    "managedBySystemd": applied.managed_by_systemd,
                    "sha256": applied.sha256,
                    "sizeBytes": applied.size_bytes,
                    "targetPath": applied.target_path,
                    "label": applied.label,
                    "message": message,
                })),
            )
        }
        Err(err) => {
            let status = status_code_for_updater_error(&err);
            tracing::warn!(error = %err, status = %status, "Error al aplicar binario local subido");
            (
                status,
                Json(json!({
                    "ok": false,
                    "error": err.to_string(),
                })),
            )
        }
    }
}

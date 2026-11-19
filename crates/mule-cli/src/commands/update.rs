// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Subcomando `mule-cli update` para consultar, comprobar, subir y revertir versiones vía HTTP/RPC.

use clap::Subcommand;
use mule_daemon::services::updater::{
    compute_sha256_hex, HEADER_UPDATE_AUTO_RESTART, HEADER_UPDATE_FILENAME, HEADER_UPDATE_SHA256,
};
use mule_daemon::DaemonSession;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// URL base por defecto del plano de control HTTP del demonio.
pub const DEFAULT_DAEMON_URL: &str = "http://127.0.0.1:4660";

/// Tiempo máximo de espera para consultas JSON-RPC de estado y comprobación.
const HTTP_RPC_TIMEOUT: Duration = Duration::from_secs(15);

/// Tiempo máximo de espera para la transferencia de subida de un binario compilado.
const HTTP_UPLOAD_TIMEOUT: Duration = Duration::from_secs(180);

/// Retraso inicial antes de consultar `/health` mientras el demonio inicia el apagado ordenado.
const HEALTH_INITIAL_DELAY: Duration = Duration::from_millis(1200);

/// Intervalo de sondeo entre intentos a `/health` tras reiniciar el demonio.
const HEALTH_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Número máximo de intentos al esperar que Pulse Core vuelva a responder en `/health`.
const HEALTH_MAX_ATTEMPTS: usize = 30;

/// Acciones disponibles bajo `mule-cli update`.
#[derive(Debug, Clone, Subcommand)]
pub enum UpdateAction {
    /// Muestra la versión instalada, plataforma, modo systemd y estado de actualización
    Status,
    /// Fuerza la consulta al canal oficial configurado
    Check,
    /// Sube e instala atómicamente un binario compilado local en el servidor
    Upload {
        /// Ruta al binario compilado (`mule-daemon`)
        file: PathBuf,
        /// No reiniciar automáticamente el demonio tras sustituir el ejecutable
        #[arg(long)]
        no_restart: bool,
        /// No esperar a que `/health` confirme el arranque tras reiniciar
        #[arg(long)]
        no_wait: bool,
    },
    /// Revierte al binario anterior conservado en `.old`
    Rollback {
        /// No reiniciar automáticamente tras restaurar la copia previa
        #[arg(long)]
        no_restart: bool,
        /// No esperar a que `/health` confirme el arranque tras reiniciar
        #[arg(long)]
        no_wait: bool,
    },
}

/// Resuelve el token de autenticación por prioridad: argumento `--token`, `MULE_AUTH_TOKEN` o `auth.token`.
pub fn resolve_token(explicit_token: Option<&str>, data_dir: &Path) -> Option<String> {
    if let Some(tok) = explicit_token.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(tok.to_string());
    }
    if let Ok(env_tok) = std::env::var("MULE_AUTH_TOKEN") {
        let trimmed = env_tok.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    DaemonSession::read_token(data_dir)
}

/// Ejecuta de forma síncrona el subcomando `update` levantando un runtime Tokio ligero.
pub fn run_update(
    base_url: &str,
    explicit_token: Option<&str>,
    data_dir: &Path,
    action: UpdateAction,
) -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(execute_update(base_url, explicit_token, data_dir, action))
}

/// Ejecuta de forma asíncrona la acción solicitada de `mule-cli update` (reutilizable en pruebas).
pub async fn execute_update(
    base_url: &str,
    explicit_token: Option<&str>,
    data_dir: &Path,
    action: UpdateAction,
) -> Result<(), Box<dyn std::error::Error>> {
    let base = base_url.trim_end_matches('/');
    let token = resolve_token(explicit_token, data_dir);

    match action {
        UpdateAction::Status => {
            let res = call_json_rpc(base, token.as_deref(), "system.getUpdateStatus", json!({}))
                .await?;
            print_update_status(&res);
        }
        UpdateAction::Check => {
            let res = call_json_rpc(
                base,
                token.as_deref(),
                "system.checkUpdate",
                json!({ "force": true }),
            )
            .await?;
            print_update_status(&res);
        }
        UpdateAction::Upload {
            file,
            no_restart,
            no_wait,
        } => {
            let auto_restart = !no_restart;
            let resp = upload_local_binary(base, token.as_deref(), &file, auto_restart).await?;
            let msg = resp
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Actualización aplicada");
            let sha = resp.get("sha256").and_then(|v| v.as_str()).unwrap_or("—");
            println!("Actualización completada: {msg}");
            println!("  SHA-256 : {sha}");
            if auto_restart && !no_wait {
                wait_for_health(base).await?;
            }
        }
        UpdateAction::Rollback {
            no_restart,
            no_wait,
        } => {
            let auto_restart = !no_restart;
            let resp = call_json_rpc(
                base,
                token.as_deref(),
                "system.rollbackUpdate",
                json!({ "autoRestart": auto_restart }),
            )
            .await?;
            let msg = resp
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Reversión completada");
            println!("{msg}");
            if auto_restart && !no_wait {
                wait_for_health(base).await?;
            }
        }
    }

    Ok(())
}

/// Realiza una llamada JSON-RPC sobre HTTP (`POST /api/rpc`) con token Bearer opcional.
async fn call_json_rpc(
    base_url: &str,
    token: Option<&str>,
    method: &str,
    params: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .timeout(HTTP_RPC_TIMEOUT)
        .no_proxy()
        .build()?;
    let url = format!("{base_url}/api/rpc");
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });

    let mut req = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(&body)?);
    if let Some(tok) = token {
        req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {tok}"));
    }

    let resp = req.send().await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        return Err(format!("El servidor devolvió HTTP {status}: {text}").into());
    }

    let parsed: Value = serde_json::from_str(&text)?;
    if let Some(err) = parsed.get("error").filter(|v| !v.is_null()) {
        let code = err.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        let msg = err
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Error RPC desconocido");
        return Err(format!("Error RPC ({code}): {msg}").into());
    }

    Ok(parsed.get("result").cloned().unwrap_or(Value::Null))
}

/// Sube un binario local compilado mediante `POST /api/system/upload-update`.
async fn upload_local_binary(
    base_url: &str,
    token: Option<&str>,
    file_path: &Path,
    auto_restart: bool,
) -> Result<Value, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(file_path)?;
    let sha256 = compute_sha256_hex(&bytes);
    let filename = file_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("mule-daemon");

    let client = reqwest::Client::builder()
        .timeout(HTTP_UPLOAD_TIMEOUT)
        .no_proxy()
        .build()?;
    let url = format!("{base_url}/api/system/upload-update");

    let mut req = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .header(HEADER_UPDATE_SHA256, &sha256)
        .header(HEADER_UPDATE_FILENAME, filename)
        .header(
            HEADER_UPDATE_AUTO_RESTART,
            if auto_restart { "true" } else { "false" },
        )
        .body(bytes);

    if let Some(tok) = token {
        req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {tok}"));
    }

    let resp = req.send().await?;
    let status = resp.status();
    let text = resp.text().await?;
    let parsed: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "error": text }));
    if !status.is_success() || parsed.get("ok") != Some(&Value::Bool(true)) {
        let err_msg = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Fallo al subir actualización");
        return Err(format!("HTTP {status}: {err_msg}").into());
    }

    Ok(parsed)
}

/// Sondea `GET /health` hasta confirmar que Pulse Core vuelve a estar en línea tras el reinicio.
async fn wait_for_health(base_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Esperando reinicio de Pulse Core...");
    tokio::time::sleep(HEALTH_INITIAL_DELAY).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()?;
    let url = format!("{base_url}/health");

    for _ in 0..HEALTH_MAX_ATTEMPTS {
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                let text = resp.text().await.unwrap_or_default();
                let val: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let ver = val
                    .get("daemonVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("desconocida");
                println!("Pulse Core en línea de nuevo (v{ver}).");
                return Ok(());
            }
        }
        tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
    }

    Err(format!(
        "Tiempo de espera agotado ({HEALTH_MAX_ATTEMPTS}s) esperando a {url}"
    )
    .into())
}

/// Imprime en consola de forma legible el estado de actualización y entorno del servidor.
fn print_update_status(val: &Value) {
    let status = val
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("desconocido");
    println!("Estado de actualización : {status}");

    if let Some(ver) = val.get("version").and_then(|v| v.as_str()) {
        println!("Versión candidata       : v{ver}");
    }
    if let Some(msg) = val.get("message").and_then(|v| v.as_str()) {
        println!("Detalle                 : {msg}");
    }

    if let Some(env) = val.get("environment").and_then(|v| v.as_object()) {
        let cur = env
            .get("currentVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("—");
        let triple = env
            .get("platformTriple")
            .and_then(|v| v.as_str())
            .unwrap_or("—");
        let systemd = env
            .get("managedBySystemd")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let writable = env
            .get("canSelfUpdate")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let backup = env
            .get("hasRollbackBackup")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        println!("Versión instalada       : v{cur}");
        println!("Plataforma del servidor : {triple}");
        println!(
            "Supervisión systemd     : {}",
            if systemd { "sí" } else { "no (proceso independiente)" }
        );
        println!(
            "Directorio escribible   : {}",
            if writable { "sí" } else { "no (solo lectura)" }
        );
        println!(
            "Respaldo rollback (.old): {}",
            if backup { "disponible" } else { "no" }
        );
    }
}

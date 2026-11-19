// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Pruebas de integración en loopback para el subcomando `mule-cli update`.

use mule_cli::commands::update::{execute_update, resolve_token, UpdateAction};
use mule_daemon::DaemonSession;
use mule_test_support::spawn_test_daemon;
use tempfile::tempdir;

#[tokio::test]
async fn test_cli_update_status_and_rollback_without_backup() {
    let (addr, token, _dir) = spawn_test_daemon("cli_secret_token_0123456789abcdef0123456789abcdef0123456789abcdef").await;
    let base_url = format!("http://{addr}");
    let tmp = tempdir().expect("directorio temporal");

    // 1. Consulta de estado exitosa
    execute_update(&base_url, Some(&token), tmp.path(), UpdateAction::Status)
        .await
        .expect("mule-cli update status debe responder correctamente en loopback");

    // 2. Rollback sin respaldo .old debe devolver un error de negocio legible
    let err = execute_update(
        &base_url,
        Some(&token),
        tmp.path(),
        UpdateAction::Rollback {
            no_restart: true,
            no_wait: true,
        },
    )
    .await
    .expect_err("rollback sin copia .old debe fallar limpiamente");
    assert!(
        err.to_string().contains(".old") || err.to_string().contains("respaldo"),
        "Error inesperado en rollback: {err}"
    );
}

#[tokio::test]
async fn test_cli_update_upload_rejects_invalid_binary_and_bad_token() {
    let valid_token = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let (addr, _token, _dir) = spawn_test_daemon(valid_token).await;
    let base_url = format!("http://{addr}");
    let tmp = tempdir().expect("directorio temporal");

    // Verificar lectura automática de auth.token desde data_dir
    DaemonSession::write_token(tmp.path(), valid_token).expect("guardar auth.token");
    assert_eq!(resolve_token(None, tmp.path()).as_deref(), Some(valid_token));

    let fake_bin = tmp.path().join("binario_invalido");
    std::fs::write(&fake_bin, b"texto-plano-no-elf").expect("escribir binario falso");

    // 1. Token incorrecto -> HTTP 401
    let err_auth = execute_update(
        &base_url,
        Some("token-incorrecto"),
        tmp.path(),
        UpdateAction::Upload {
            file: fake_bin.clone(),
            no_restart: true,
            no_wait: true,
        },
    )
    .await
    .expect_err("token incorrecto debe rechazarse");
    assert!(
        err_auth.to_string().contains("401"),
        "Se esperaba HTTP 401, recibido: {err_auth}"
    );

    // 2. Token leído automáticamente de auth.token + formato inválido -> HTTP 400
    let err_fmt = execute_update(
        &base_url,
        None,
        tmp.path(),
        UpdateAction::Upload {
            file: fake_bin,
            no_restart: true,
            no_wait: true,
        },
    )
    .await
    .expect_err("archivo no ejecutable debe rechazarse");
    assert!(
        err_fmt.to_string().contains("400"),
        "Se esperaba HTTP 400, recibido: {err_fmt}"
    );
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Pruebas de integración sin conexión del sistema híbrido de actualizaciones (loopback únicamente).

use axum::{
    extract::State,
    http::{header, StatusCode},
    routing::get,
    Router,
};
use mule_daemon::rpc::methods::handle_request;
use mule_daemon::services::updater::{
    compute_sha256_hex, UpdateManifest, UpdateStatusSnapshot, UpdaterError, ELF_CLASS_64,
    ELF_DATA_LITTLE_ENDIAN, ELF_MACHINE_AARCH64, ELF_MACHINE_X86_64, ELF_MAGIC, PE_MAGIC,
};
use mule_network_policy::NetworkProfile;
use mule_test_support::{make_req, rpc_state, rpc_state_with_profile, spawn_test_daemon};
use reqwest::header::AUTHORIZATION;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{json, Value};
use tokio::net::TcpListener;

/// Construye un binario sintético válido para la plataforma del host actual con el tamaño indicado.
fn make_valid_payload_for_triple(triple: &str, total_len: usize) -> Vec<u8> {
    let mut buf = vec![0xAAu8; total_len.max(128)];
    if triple == "x86_64-unknown-linux-gnu" {
        buf[0..4].copy_from_slice(&ELF_MAGIC);
        buf[4] = ELF_CLASS_64;
        buf[5] = ELF_DATA_LITTLE_ENDIAN;
        buf[6] = 1;
        buf[16..18].copy_from_slice(&3u16.to_le_bytes());
        buf[18..20].copy_from_slice(&ELF_MACHINE_X86_64.to_le_bytes());
    } else if triple == "aarch64-unknown-linux-gnu" {
        buf[0..4].copy_from_slice(&ELF_MAGIC);
        buf[4] = ELF_CLASS_64;
        buf[5] = ELF_DATA_LITTLE_ENDIAN;
        buf[6] = 1;
        buf[16..18].copy_from_slice(&3u16.to_le_bytes());
        buf[18..20].copy_from_slice(&ELF_MACHINE_AARCH64.to_le_bytes());
    } else {
        buf[0..2].copy_from_slice(&PE_MAGIC);
    }
    buf
}

#[tokio::test]
async fn test_get_update_status_includes_real_environment_info() {
    let (state, _dir) = rpc_state("token-entorno");
    let mut auth = true;

    let hello = handle_request(
        &state,
        make_req(
            1,
            "system.hello",
            Some(json!({ "protocolVersion": 1, "token": "token-entorno" })),
        ),
        &mut auth,
    )
    .await
    .expect("respuesta de system.hello")
    .result
    .expect("resultado exitoso de system.hello");

    let expected_triple = UpdateManifest::current_platform_triple();
    assert_eq!(hello["platformTriple"], expected_triple);
    assert!(hello["managedBySystemd"].is_boolean());

    let status_resp = handle_request(&state, make_req(2, "system.getUpdateStatus", None), &mut auth)
        .await
        .expect("respuesta de system.getUpdateStatus")
        .result
        .expect("resultado exitoso de system.getUpdateStatus");

    assert_eq!(status_resp["status"], "idle");
    let env = &status_resp["environment"];
    assert_eq!(env["platformTriple"], expected_triple);
    assert_eq!(env["currentVersion"], env!("CARGO_PKG_VERSION"));
    assert!(env["canSelfUpdate"].is_boolean());
    assert!(env["hasRollbackBackup"].is_boolean());
    assert!(env["managedBySystemd"].is_boolean());
    assert!(env["maxUploadBytes"].as_u64().unwrap_or(0) > 0);

    let rb_resp = handle_request(
        &state,
        make_req(3, "system.rollbackUpdate", Some(json!({ "autoRestart": false }))),
        &mut auth,
    )
    .await
    .expect("respuesta de system.rollbackUpdate");
    let rb_err = rb_resp.error.expect("rollback sin .old debe devolver error RPC");
    assert!(
        rb_err.message.contains(".old") || rb_err.message.contains("respaldo"),
        "Mensaje inesperado: {}",
        rb_err.message
    );
}

#[tokio::test]
async fn test_upload_update_auth_and_validation_loopback() {
    let (addr, token, _dir) = spawn_test_daemon("token-subida-secreto").await;
    let url = format!("http://{addr}/api/system/upload-update");
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("cliente HTTP local");

    // 1. Petición sin token Bearer -> 401 Unauthorized
    let resp_no_auth = client
        .post(&url)
        .body(vec![1, 2, 3, 4])
        .send()
        .await
        .expect("respuesta sin token");
    assert_eq!(resp_no_auth.status(), StatusCode::UNAUTHORIZED);

    // 2. Token Bearer incorrecto -> 401 Unauthorized
    let resp_bad_auth = client
        .post(&url)
        .header(AUTHORIZATION, "Bearer token-falso")
        .body(vec![1, 2, 3, 4])
        .send()
        .await
        .expect("respuesta con token inválido");
    assert_eq!(resp_bad_auth.status(), StatusCode::UNAUTHORIZED);

    // 3. Cuerpo vacío con token válido -> 400 Bad Request
    let resp_empty = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(Vec::<u8>::new())
        .send()
        .await
        .expect("respuesta con cuerpo vacío");
    assert_eq!(resp_empty.status(), StatusCode::BAD_REQUEST);

    // 4. Bytes aleatorios no ejecutables con token válido -> 400 Bad Request
    let resp_garbage = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(b"contenido-no-ejecutable-aleatorio".to_vec())
        .send()
        .await
        .expect("respuesta con formato inválido");
    assert_eq!(resp_garbage.status(), StatusCode::BAD_REQUEST);
    let body_garbage_txt = resp_garbage.text().await.expect("texto JSON de error");
    let body_garbage: Value = serde_json::from_str(&body_garbage_txt).expect("JSON de error");
    assert_eq!(body_garbage["ok"], false);

    // 5. Binario ELF de arquitectura opuesta -> 400 Bad Request sin tocar disco
    let wrong_triple = match UpdateManifest::current_platform_triple() {
        "aarch64-unknown-linux-gnu" => "x86_64-unknown-linux-gnu",
        _ => "aarch64-unknown-linux-gnu",
    };
    let wrong_arch_elf = make_valid_payload_for_triple(wrong_triple, 128);
    let resp_wrong_arch = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(wrong_arch_elf)
        .send()
        .await
        .expect("respuesta arquitectura incompatible");
    assert_eq!(resp_wrong_arch.status(), StatusCode::BAD_REQUEST);
    let body_wrong_txt = resp_wrong_arch.text().await.expect("texto JSON arquitectura");
    let body_wrong: Value = serde_json::from_str(&body_wrong_txt).expect("JSON de arquitectura");
    assert_eq!(body_wrong["ok"], false);
    assert!(
        body_wrong["error"]
            .as_str()
            .unwrap_or("")
            .contains("incompatible")
            || body_wrong["error"]
                .as_str()
                .unwrap_or("")
                .contains("ELF"),
        "Error inesperado: {}",
        body_wrong
    );
}

#[derive(Clone)]
struct MockReleaseState {
    manifest_json: String,
    binary_bytes: Vec<u8>,
}

#[tokio::test]
async fn test_check_update_respects_kill_switch_and_locallab_loopback() {
    let current_triple = UpdateManifest::current_platform_triple();
    let payload = make_valid_payload_for_triple(current_triple, 160 * 1024);
    let payload_sha256 = compute_sha256_hex(&payload);

    let rng = SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).expect("pkcs8");
    let keypair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("keypair");
    let pubkey_hex = hex::encode(keypair.public_key().as_ref());
    let sig_hex = hex::encode(keypair.sign(&payload).as_ref());

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback mock");
    let mock_addr = listener.local_addr().expect("mock addr");

    let manifest = json!({
        "version": "99.0.0",
        "releaseDate": "2026-11-20T00:00:00Z",
        "channel": "stable",
        "summary": "Lanzamiento de prueba loopback",
        "targets": {
            current_triple: {
                "url": format!("http://{mock_addr}/binary.bin"),
                "sha256": payload_sha256,
                "size": payload.len(),
                "signature": sig_hex
            }
        }
    })
    .to_string();

    let mock_state = MockReleaseState {
        manifest_json: manifest,
        binary_bytes: payload.clone(),
    };

    let mock_router = Router::new()
        .route(
            "/redirect-manifest",
            get(|| async {
                (
                    StatusCode::FOUND,
                    [(header::LOCATION, "/manifest.json")],
                    "",
                )
            }),
        )
        .route(
            "/manifest.json",
            get(|State(st): State<MockReleaseState>| async move {
                (StatusCode::OK, st.manifest_json)
            }),
        )
        .route(
            "/binary.bin",
            get(|State(st): State<MockReleaseState>| async move {
                (
                    [(header::CONTENT_TYPE, "application/octet-stream")],
                    st.binary_bytes,
                )
            }),
        )
        .route(
            "/missing.json",
            get(|| async { (StatusCode::NOT_FOUND, "not found") }),
        )
        .with_state(mock_state);

    tokio::spawn(async move {
        let _ = axum::serve(listener, mock_router).await;
    });

    let (state, _dir) = rpc_state_with_profile("token-policy", NetworkProfile::LocalLab);
    state.updater.set_public_key_hex(pubkey_hex).await;

    // 1. Interruptor de corte activado -> debe bloquear incluso loopback sin tocar red
    state
        .updater
        .set_manifest_url(format!("http://{mock_addr}/manifest.json"))
        .await;
    state.network_policy.disconnect_all();
    let err_kill = state
        .updater
        .check_for_updates(&state.network_policy)
        .await
        .expect_err("El interruptor de corte debe bloquear check_for_updates");
    assert!(matches!(err_kill, UpdaterError::NetworkBlocked(_)));

    // Reactivamos red en perfil LocalLab
    state.network_policy.enable();

    // 2. URL externa en perfil LocalLab -> bloqueada por anti-SSRF/aislamiento loopback
    state
        .updater
        .set_manifest_url("https://github.com/caraveliko/PulseCore/releases/latest/download/manifest.json".to_string())
        .await;
    let err_external = state
        .updater
        .check_for_updates(&state.network_policy)
        .await
        .expect_err("LocalLab debe bloquear hosts externos");
    assert!(matches!(err_external, UpdaterError::NetworkBlocked(_)));

    // 3. HTTP 404 en loopback -> error estructurado y RPC manejado limpiamente
    state
        .updater
        .set_manifest_url(format!("http://{mock_addr}/missing.json"))
        .await;
    let err_404 = state
        .updater
        .check_for_updates(&state.network_policy)
        .await
        .expect_err("404 debe devolver HttpStatus");
    assert!(matches!(err_404, UpdaterError::HttpStatus { status: 404, .. }));

    let mut auth = true;
    let rpc_404 = handle_request(&state, make_req(10, "system.checkUpdate", None), &mut auth)
        .await
        .expect("respuesta RPC")
        .result
        .expect("resultado JSON amable en HTTP 404");
    assert_eq!(rpc_404["status"], "error");
    assert!(rpc_404["message"]
        .as_str()
        .unwrap_or("")
        .contains("404"));

    // 4. Redirección 302 en loopback -> manifiesto válido -> descarga en streaming a staging
    state
        .updater
        .set_manifest_url(format!("http://{mock_addr}/redirect-manifest"))
        .await;
    let snap = state
        .updater
        .check_for_updates(&state.network_policy)
        .await
        .expect("consulta válida vía redirección loopback");
    assert!(matches!(
        snap,
        UpdateStatusSnapshot::UpdateAvailable { ref version, .. } if version == "99.0.0"
    ));

    let staging_path = state
        .updater
        .download_and_stage(&state.network_policy)
        .await
        .expect("descarga en streaming y firma Ed25519 válidas");
    assert!(staging_path.exists());
    assert_eq!(
        std::fs::read(&staging_path).expect("lectura staging"),
        payload
    );

    let final_status = state.updater.get_status().await;
    assert!(matches!(
        final_status,
        UpdateStatusSnapshot::ReadyToInstall { ref version, .. } if version == "99.0.0"
    ));

    // Limpiar el artefacto .update temporal creado junto al binario de pruebas
    let _ = std::fs::remove_file(&staging_path);
}

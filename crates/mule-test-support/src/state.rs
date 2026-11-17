// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Constructores de `AppState` de prueba: cada función corresponde a un
//! escenario recurrente de las pruebas de integración (RPC en memoria, Kad
//! persistente, par con descarga, compartido con fichero real).

use tempfile::{tempdir, TempDir};

use mule_daemon::rpc::methods::handle_request;
use mule_daemon::{AppState, DaemonConfig, DownloadService, WorkDirMode};
use mule_network_policy::NetworkProfile;
use serde_json::json;

use crate::rpc::make_req;

/// Estado mínimo en memoria: workdir temporal, modo [`WorkDirMode::Portable`],
/// almacenamiento de descargas en memoria.
pub fn rpc_state(token: &str) -> (AppState, TempDir) {
    let dir = tempdir().unwrap();
    let temp_dir = dir.path().join("temp");

    let config = DaemonConfig::new(dir.path().to_path_buf(), token)
        .with_workdir_mode(WorkDirMode::Portable);
    let downloads = DownloadService::in_memory(temp_dir).unwrap();
    let state = AppState::new(config, downloads);

    (state, dir)
}

/// Estado en memoria cuya [`DaemonConfig`] se personaliza antes de la
/// construcción (perfil de red, UPnP, ofuscación…). El modo
/// [`WorkDirMode::Portable`] sigue imponiéndose tras `customize`.
pub fn rpc_state_with_config(
    token: &str,
    customize: impl FnOnce(DaemonConfig) -> DaemonConfig,
) -> (AppState, TempDir) {
    let dir = tempdir().unwrap();
    let temp_dir = dir.path().join("temp");

    let config = customize(DaemonConfig::new(dir.path().to_path_buf(), token))
        .with_workdir_mode(WorkDirMode::Portable);
    let downloads = DownloadService::in_memory(temp_dir).unwrap();
    let state = AppState::new(config, downloads);

    (state, dir)
}

/// Estado en memoria con perfil de red forzado (pruebas de política de red).
pub fn rpc_state_with_profile(token: &str, profile: NetworkProfile) -> (AppState, TempDir) {
    let (state, dir) = rpc_state(token);
    state.network_policy.set_profile(profile);
    (state, dir)
}

/// Estado Kad persistente: estructura de directorios `data/` + `downloads/` en `tmp`, base
/// SQLite real, modo [`WorkDirMode::ExplicitCli`], token fijo `test-token`.
pub fn kad_state(tmp: &TempDir) -> AppState {
    let data_dir = tmp.path().join("data");
    let dl_dir = tmp.path().join("downloads");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&dl_dir).unwrap();

    let db_path = data_dir.join("mule.db");
    let downloads = DownloadService::new(&db_path, data_dir.join("temp")).unwrap();
    let config = DaemonConfig::new(data_dir, "test-token")
        .with_bind_addr("127.0.0.1:0".parse().unwrap())
        .with_default_download_dir(dl_dir)
        .with_workdir_mode(WorkDirMode::ExplicitCli);

    AppState::new(config, downloads)
}

/// Estado LocalLab con una descarga registrada mediante `downloads.addEd2kLink`.
/// Devuelve `(state, workdir, hash_archivo_hex)` — el hash pasado como parámetro
/// se devuelve tal cual para las aserciones.
pub async fn download_state(
    token: &str,
    file_name: &str,
    file_size: u64,
    file_hash_hex: &str,
) -> (AppState, TempDir, String) {
    download_state_inner(token, file_name, file_size, file_hash_hex, None).await
}

/// Variante de [`download_state`] que activa la ofuscación protocolaria
/// (`enabled`, `required`).
pub async fn download_state_with_obfuscation(
    token: &str,
    file_name: &str,
    file_size: u64,
    file_hash_hex: &str,
    obfuscation_enabled: bool,
    obfuscation_required: bool,
) -> (AppState, TempDir, String) {
    download_state_inner(
        token,
        file_name,
        file_size,
        file_hash_hex,
        Some((obfuscation_enabled, obfuscation_required)),
    )
    .await
}

/// Estado con un compartido `shares.add` respaldado por un archivo real escrito en disco.
/// La ruta de subida solo sirve bytes legibles: `content` es el contenido
/// físico del archivo, `declared_size` el tamaño anunciado a la red.
/// Devuelve `(state, workdir, hash_archivo_hex)`.
pub async fn share_state(
    token: &str,
    file_name: &str,
    declared_size: u64,
    content: &[u8],
    file_hash_hex: &str,
) -> (AppState, TempDir, String) {
    let (state, dir) = rpc_state(token);

    let shared_path = dir.path().join(file_name);
    std::fs::write(&shared_path, content).unwrap();

    let mut authenticated = true;
    let share_req = make_req(
        1,
        "shares.add",
        Some(json!({
            "fileHash": file_hash_hex,
            "fileName": file_name,
            "fileSize": declared_size,
            "filePath": shared_path.to_string_lossy()
        })),
    );
    let resp = handle_request(&state, share_req, &mut authenticated)
        .await
        .unwrap();
    assert!(resp.error.is_none());

    (state, dir, file_hash_hex.to_string())
}

async fn download_state_inner(
    token: &str,
    file_name: &str,
    file_size: u64,
    file_hash_hex: &str,
    obfuscation: Option<(bool, bool)>,
) -> (AppState, TempDir, String) {
    let dir = tempdir().unwrap();
    let temp_dir = dir.path().join("temp");

    let config = DaemonConfig::new(dir.path().to_path_buf(), token)
        .with_workdir_mode(WorkDirMode::Portable);
    let config = match obfuscation {
        Some((enabled, required)) => config.with_obfuscation(enabled, required),
        None => config,
    };
    let downloads = DownloadService::in_memory(temp_dir).unwrap();
    let state = AppState::new(config, downloads);
    state
        .network_policy
        .set_profile(NetworkProfile::LocalLab);

    // Registra una descarga para proporcionar un destino a las sesiones de pares.
    let link = format!("ed2k://|file|{file_name}|{file_size}|{file_hash_hex}|/");
    let req = make_req(1, "downloads.addEd2kLink", Some(json!({ "link": link })));
    let mut authenticated = true;
    let resp = handle_request(&state, req, &mut authenticated)
        .await
        .unwrap();
    assert!(resp.error.is_none());

    (state, dir, file_hash_hex.to_string())
}

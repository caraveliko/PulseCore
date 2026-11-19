// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Servicio gestor de actualizaciones en segundo plano para eMuleRust.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use mule_network_policy::NetworkPolicyManager;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use super::constants::{
    DEFAULT_RELEASE_PUBKEY_HEX, DOWNLOAD_PAYLOAD_TIMEOUT, MANIFEST_FETCH_TIMEOUT,
    MAX_UPDATE_PAYLOAD_BYTES, PROGRESS_UPDATE_MIN_BYTES,
};
use super::error::UpdaterError;
use super::http_fetch::fetch_validated_response;
use super::installer::{
    get_current_executable_path, get_staging_executable_path, has_rollback_backup,
    is_executable_dir_writable, is_managed_by_systemd, replace_current_executable,
    rollback_to_backup, write_staging_binary,
};
use super::manifest::{TargetAsset, UpdateManifest};
use super::verifier::{
    validate_executable_for_current_platform, validate_executable_for_triple,
    verify_optional_sha256, verify_payload,
};

/// Estado observable del ciclo de vida de la actualización.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UpdateStatusSnapshot {
    /// Sin actividad de actualización pendiente.
    Idle,
    /// Comprobando disponibilidad de nueva versión en el manifiesto remoto.
    Checking,
    /// Se ha detectado una nueva versión disponible para su instalación.
    UpdateAvailable {
        version: String,
        #[serde(rename = "currentVersion")]
        current_version: String,
        #[serde(rename = "releaseDate")]
        release_date: String,
        #[serde(rename = "changelogUrl", skip_serializing_if = "Option::is_none")]
        changelog_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(rename = "downloadSize")]
        download_size: usize,
    },
    /// La versión instalada coincide con la más reciente del canal.
    UpToDate {
        #[serde(rename = "currentVersion")]
        current_version: String,
    },
    /// Descarga del binario en curso hacia el área de preparación (*staging*).
    Downloading {
        #[serde(rename = "downloadedBytes")]
        downloaded_bytes: usize,
        #[serde(rename = "totalBytes")]
        total_bytes: usize,
    },
    /// Binario validado, firmado y preparado para sustitución al reiniciar.
    ReadyToInstall {
        version: String,
        #[serde(rename = "stagingPath")]
        staging_path: String,
    },
    /// Se ha producido un fallo durante la comprobación, descarga o verificación.
    Error {
        message: String,
    },
}

/// Información real del entorno de ejecución para mostrar diagnósticos y elegir actualizaciones.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEnvironmentInfo {
    /// Versión compilada actualmente en ejecución.
    pub current_version: String,
    /// Triple oficial de plataforma de este demonio (`x86_64-unknown-linux-gnu`, etc.).
    pub platform_triple: String,
    /// Sistema operativo reportado por Rust (`linux`, `windows`, etc.).
    pub os: String,
    /// Arquitectura de CPU reportada por Rust (`x86_64`, `aarch64`, etc.).
    pub arch: String,
    /// Ruta absoluta del ejecutable en disco.
    pub executable_path: String,
    /// Indica si el usuario/servicio actual tiene permisos de escritura sobre el directorio del binario.
    pub can_self_update: bool,
    /// Indica si existe una copia de respaldo `.old` disponible para rollback inmediato.
    pub has_rollback_backup: bool,
    /// Indica si el proceso está supervisado por `systemd`.
    pub managed_by_systemd: bool,
    /// URL configurada para consultar manifiestos oficiales.
    pub manifest_url: String,
    /// Canal configurado (`stable`, `beta`, etc.).
    pub channel: String,
    /// Indica si está habilitada la comprobación automática al iniciar.
    pub auto_check_enabled: bool,
    /// Límite máximo permitido en bytes para descargas y subidas locales.
    pub max_upload_bytes: usize,
}

/// Resultado de haber validado y aplicado una actualización desde un binario local compilado.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalUpdateApplied {
    /// Suma SHA-256 calculada sobre el binario instalado.
    pub sha256: String,
    /// Tamaño exacto del binario instalado en bytes.
    pub size_bytes: usize,
    /// Ruta del ejecutable reemplazado en disco.
    pub target_path: String,
    /// Etiqueta o nombre de archivo descriptivo aplicado.
    pub label: String,
    /// Indica si `systemd` relanzará el proceso tras el apagado ordenado.
    pub managed_by_systemd: bool,
}

/// Guarda RAII que garantiza la liberación de `is_busy` aunque haya retorno temprano.
struct BusyGuard<'a>(&'a AtomicBool);

impl<'a> BusyGuard<'a> {
    fn try_acquire(flag: &'a AtomicBool) -> Option<Self> {
        if flag.swap(true, Ordering::SeqCst) {
            None
        } else {
            Some(Self(flag))
        }
    }
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Servicio que orquesta la verificación, descarga y aplicación de versiones.
pub struct UpdaterService {
    manifest_url: RwLock<String>,
    public_key_hex: RwLock<String>,
    current_version: String,
    channel: String,
    auto_check_enabled: bool,
    status: RwLock<UpdateStatusSnapshot>,
    cached_asset: RwLock<Option<(String, TargetAsset)>>,
    is_busy: AtomicBool,
    restart_pending: AtomicBool,
}

impl UpdaterService {
    /// Crea una nueva instancia del servicio de actualización con valores de canal predeterminados.
    pub fn new(manifest_url: String, current_version: String) -> Self {
        Self::with_channel_config(manifest_url, current_version, "stable".to_string(), true)
    }

    /// Crea una instancia del servicio con canal y configuración automática explícitos.
    pub fn with_channel_config(
        manifest_url: String,
        current_version: String,
        channel: String,
        auto_check_enabled: bool,
    ) -> Self {
        Self {
            manifest_url: RwLock::new(manifest_url),
            public_key_hex: RwLock::new(DEFAULT_RELEASE_PUBKEY_HEX.to_string()),
            current_version,
            channel,
            auto_check_enabled,
            status: RwLock::new(UpdateStatusSnapshot::Idle),
            cached_asset: RwLock::new(None),
            is_busy: AtomicBool::new(false),
            restart_pending: AtomicBool::new(false),
        }
    }

    /// Marca que el proceso debe ser relanzado tras completarse el apagado ordenado.
    pub fn mark_restart_pending(&self) {
        self.restart_pending.store(true, Ordering::SeqCst);
    }

    /// Comprueba si hay un reinicio pendiente tras una actualización.
    pub fn is_restart_pending(&self) -> bool {
        self.restart_pending.load(Ordering::SeqCst)
    }

    /// Sobrescribe la URL del manifiesto (útil en pruebas de integración sin conexión).
    pub async fn set_manifest_url(&self, manifest_url: String) {
        let mut url = self.manifest_url.write().await;
        *url = manifest_url;
    }

    /// Sobrescribe la clave pública de verificación Ed25519 (útil para pruebas de laboratorio).
    pub async fn set_public_key_hex(&self, pubkey_hex: String) {
        let mut key = self.public_key_hex.write().await;
        *key = pubkey_hex;
    }

    /// Obtiene la instantánea actual del estado de la actualización.
    pub async fn get_status(&self) -> UpdateStatusSnapshot {
        self.status.read().await.clone()
    }

    /// Devuelve la información real del servidor y si puede autoactualizarse en disco.
    pub async fn environment_info(&self) -> UpdateEnvironmentInfo {
        let exe_path = get_current_executable_path().ok();
        let can_self_update = exe_path
            .as_deref()
            .map(is_executable_dir_writable)
            .unwrap_or(false);
        let has_rollback_backup = exe_path
            .as_deref()
            .map(has_rollback_backup)
            .unwrap_or(false);
        let executable_path = exe_path
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        UpdateEnvironmentInfo {
            current_version: self.current_version.clone(),
            platform_triple: UpdateManifest::current_platform_triple().to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            executable_path,
            can_self_update,
            has_rollback_backup,
            managed_by_systemd: is_managed_by_systemd(),
            manifest_url: self.manifest_url.read().await.clone(),
            channel: self.channel.clone(),
            auto_check_enabled: self.auto_check_enabled,
            max_upload_bytes: MAX_UPDATE_PAYLOAD_BYTES,
        }
    }

    /// Actualiza la instantánea de estado con un mensaje de error legible.
    async fn set_error_status(&self, message: String) {
        let mut st = self.status.write().await;
        *st = UpdateStatusSnapshot::Error { message };
    }

    /// Realiza una consulta al manifiesto remoto validada por la política de red.
    pub async fn check_for_updates(
        &self,
        network_policy: &NetworkPolicyManager,
    ) -> Result<UpdateStatusSnapshot, UpdaterError> {
        let Some(_busy_guard) = BusyGuard::try_acquire(&self.is_busy) else {
            return Ok(self.get_status().await);
        };

        {
            let mut st = self.status.write().await;
            *st = UpdateStatusSnapshot::Checking;
        }

        let url = self.manifest_url.read().await.clone();
        let current_ver = self.current_version.clone();

        tracing::info!(
            manifest_url = %url,
            current_version = %current_ver,
            "Comprobando actualizaciones de Pulse Core"
        );

        let mut response =
            match fetch_validated_response(&url, MANIFEST_FETCH_TIMEOUT, network_policy).await {
                Ok(resp) => resp,
                Err(err) => {
                    let msg = match &err {
                        UpdaterError::HttpStatus { status: 404, .. } => {
                            "No se encontró ningún manifiesto oficial publicado (HTTP 404). Si compilas en Linux, puedes subir tu propio binario desde la pestaña de actualización local.".to_string()
                        }
                        _ => err.to_string(),
                    };
                    tracing::warn!("{msg}");
                    self.set_error_status(msg).await;
                    return Err(err);
                }
            };

        if let Some(len) = response.content_length() {
            if len > MAX_UPDATE_PAYLOAD_BYTES as u64 {
                let err = UpdaterError::PayloadTooLarge {
                    size: len as usize,
                    max: MAX_UPDATE_PAYLOAD_BYTES,
                };
                self.set_error_status(err.to_string()).await;
                return Err(err);
            }
        }

        let mut body_bytes = Vec::new();
        while let Some(chunk) = match response.chunk().await {
            Ok(c) => c,
            Err(e) => {
                let err_msg = format!("Error al leer el cuerpo del manifiesto: {e}");
                self.set_error_status(err_msg).await;
                return Err(UpdaterError::Network(e));
            }
        } {
            if body_bytes.len().saturating_add(chunk.len()) > MAX_UPDATE_PAYLOAD_BYTES {
                let err = UpdaterError::PayloadTooLarge {
                    size: body_bytes.len().saturating_add(chunk.len()),
                    max: MAX_UPDATE_PAYLOAD_BYTES,
                };
                self.set_error_status(err.to_string()).await;
                return Err(err);
            }
            body_bytes.extend_from_slice(&chunk);
        }

        let body_text = match String::from_utf8(body_bytes) {
            Ok(txt) => txt,
            Err(e) => {
                let err_msg = format!("El manifiesto recibido no es texto UTF-8 válido: {e}");
                self.set_error_status(err_msg.clone()).await;
                return Err(UpdaterError::InvalidExecutableFormat(err_msg));
            }
        };

        let manifest = match UpdateManifest::parse_str(&body_text) {
            Ok(m) => m,
            Err(e) => {
                let err_msg = format!("Manifiesto inválido recibido: {e}");
                self.set_error_status(err_msg).await;
                return Err(e);
            }
        };

        let triple = UpdateManifest::current_platform_triple();
        let asset = match manifest.resolve_target(triple) {
            Some(a) => a.clone(),
            None => {
                let err = UpdaterError::UnsupportedPlatform(triple.to_string());
                self.set_error_status(format!(
                    "Plataforma no soportada en esta versión: {triple}"
                ))
                .await;
                return Err(err);
            }
        };

        let new_snapshot = if manifest.is_newer_than(&current_ver) {
            let snap = UpdateStatusSnapshot::UpdateAvailable {
                version: manifest.version.clone(),
                current_version: current_ver,
                release_date: manifest.release_date.clone(),
                changelog_url: manifest.changelog_url.clone(),
                summary: manifest.summary.clone(),
                download_size: asset.size,
            };
            let mut cached = self.cached_asset.write().await;
            *cached = Some((manifest.version.clone(), asset));
            snap
        } else {
            let mut cached = self.cached_asset.write().await;
            *cached = None;
            UpdateStatusSnapshot::UpToDate {
                current_version: current_ver,
            }
        };

        {
            let mut st = self.status.write().await;
            *st = new_snapshot.clone();
        }

        Ok(new_snapshot)
    }

    /// Descarga en streaming el paquete oficial, valida SHA-256, firma Ed25519 y cabecera ELF/PE, y lo guarda en staging.
    pub async fn download_and_stage(
        &self,
        network_policy: &NetworkPolicyManager,
    ) -> Result<PathBuf, UpdaterError> {
        let Some(_busy_guard) = BusyGuard::try_acquire(&self.is_busy) else {
            return Err(UpdaterError::UpdateInProgress);
        };

        let (version, asset) = {
            let cached = self.cached_asset.read().await;
            match cached.as_ref() {
                Some(val) => val.clone(),
                None => return Err(UpdaterError::NoUpdateReady),
            }
        };

        if asset.size > MAX_UPDATE_PAYLOAD_BYTES {
            let err = UpdaterError::PayloadTooLarge {
                size: asset.size,
                max: MAX_UPDATE_PAYLOAD_BYTES,
            };
            self.set_error_status(err.to_string()).await;
            return Err(err);
        }

        let mut total_bytes = asset.size;
        {
            let mut st = self.status.write().await;
            *st = UpdateStatusSnapshot::Downloading {
                downloaded_bytes: 0,
                total_bytes,
            };
        }

        tracing::info!(
            version = %version,
            url = %asset.url,
            "Iniciando descarga en streaming de actualización"
        );

        let mut response =
            match fetch_validated_response(&asset.url, DOWNLOAD_PAYLOAD_TIMEOUT, network_policy)
                .await
            {
                Ok(resp) => resp,
                Err(err) => {
                    self.set_error_status(err.to_string()).await;
                    return Err(err);
                }
            };

        if let Some(len) = response.content_length() {
            if len > MAX_UPDATE_PAYLOAD_BYTES as u64 {
                let err = UpdaterError::PayloadTooLarge {
                    size: len as usize,
                    max: MAX_UPDATE_PAYLOAD_BYTES,
                };
                self.set_error_status(err.to_string()).await;
                return Err(err);
            }
            if total_bytes == 0 {
                total_bytes = len as usize;
            }
        }

        let start_time = Instant::now();
        let mut payload_bytes = Vec::with_capacity(total_bytes);
        let mut last_reported = 0usize;

        while let Some(chunk) = match response.chunk().await {
            Ok(c) => c,
            Err(e) => {
                let err = UpdaterError::Network(e);
                self.set_error_status(err.to_string()).await;
                return Err(err);
            }
        } {
            let next_len = payload_bytes.len().saturating_add(chunk.len());
            if next_len > MAX_UPDATE_PAYLOAD_BYTES {
                let err = UpdaterError::PayloadTooLarge {
                    size: next_len,
                    max: MAX_UPDATE_PAYLOAD_BYTES,
                };
                self.set_error_status(err.to_string()).await;
                return Err(err);
            }
            payload_bytes.extend_from_slice(&chunk);
            if payload_bytes.len().saturating_sub(last_reported) >= PROGRESS_UPDATE_MIN_BYTES {
                last_reported = payload_bytes.len();
                let mut st = self.status.write().await;
                *st = UpdateStatusSnapshot::Downloading {
                    downloaded_bytes: payload_bytes.len(),
                    total_bytes: total_bytes.max(payload_bytes.len()),
                };
            }
        }

        if payload_bytes.len() != last_reported {
            let mut st = self.status.write().await;
            *st = UpdateStatusSnapshot::Downloading {
                downloaded_bytes: payload_bytes.len(),
                total_bytes: total_bytes.max(payload_bytes.len()),
            };
        }

        tracing::info!(
            bytes = payload_bytes.len(),
            duration_ms = start_time.elapsed().as_millis(),
            "Descarga finalizada. Procediendo con verificación criptográfica y de arquitectura"
        );

        let pubkey_hex = self.public_key_hex.read().await.clone();
        if let Err(err) = verify_payload(
            &payload_bytes,
            &asset.sha256,
            &asset.signature,
            &pubkey_hex,
        ) {
            self.set_error_status(err.to_string()).await;
            return Err(err);
        }

        if let Err(err) = validate_executable_for_current_platform(&payload_bytes) {
            self.set_error_status(err.to_string()).await;
            return Err(err);
        }

        let current_exe = match get_current_executable_path() {
            Ok(p) => p,
            Err(err) => {
                self.set_error_status(err.to_string()).await;
                return Err(err);
            }
        };
        let staging_path = get_staging_executable_path(&current_exe);
        if let Err(err) = write_staging_binary(&staging_path, &payload_bytes) {
            self.set_error_status(err.to_string()).await;
            return Err(err);
        }

        let ready_snapshot = UpdateStatusSnapshot::ReadyToInstall {
            version,
            staging_path: staging_path.to_string_lossy().to_string(),
        };

        {
            let mut st = self.status.write().await;
            *st = ready_snapshot;
        }

        Ok(staging_path)
    }

    /// Aplica el reemplazo del binario previamente preparado en *staging*.
    pub async fn apply_update(&self) -> Result<(), UpdaterError> {
        let current_exe = get_current_executable_path()?;
        let staging_path = get_staging_executable_path(&current_exe);

        if !staging_path.exists() {
            return Err(UpdaterError::NoUpdateReady);
        }

        tracing::info!(
            staging = %staging_path.display(),
            target = %current_exe.display(),
            "Aplicando sustitución atómica del ejecutable"
        );

        if let Err(err) = replace_current_executable(&staging_path, &current_exe) {
            self.set_error_status(err.to_string()).await;
            return Err(err);
        }

        tracing::info!("Sustitución completada con éxito. Listo para reinicio ordenado");
        Ok(())
    }

    /// Valida y aplica atómicamente un binario local compilado sobre el ejecutable en curso.
    pub async fn stage_and_apply_local_binary(
        &self,
        payload: &[u8],
        expected_sha256: Option<&str>,
        label: Option<&str>,
    ) -> Result<LocalUpdateApplied, UpdaterError> {
        let current_exe = get_current_executable_path()?;
        self.stage_and_apply_local_binary_to_target(
            payload,
            expected_sha256,
            label,
            &current_exe,
            UpdateManifest::current_platform_triple(),
        )
        .await
    }

    /// Valida un binario local para el triple indicado y lo instala atómicamente sobre `target_exe`.
    pub async fn stage_and_apply_local_binary_to_target(
        &self,
        payload: &[u8],
        expected_sha256: Option<&str>,
        label: Option<&str>,
        target_exe: &Path,
        expected_triple: &str,
    ) -> Result<LocalUpdateApplied, UpdaterError> {
        let Some(_busy_guard) = BusyGuard::try_acquire(&self.is_busy) else {
            return Err(UpdaterError::UpdateInProgress);
        };

        match self.validate_and_replace_target(
            payload,
            expected_sha256,
            label,
            target_exe,
            expected_triple,
        ) {
            Ok(applied) => {
                let mut st = self.status.write().await;
                *st = UpdateStatusSnapshot::ReadyToInstall {
                    version: applied.label.clone(),
                    staging_path: applied.target_path.clone(),
                };
                Ok(applied)
            }
            Err(err) => {
                self.set_error_status(err.to_string()).await;
                Err(err)
            }
        }
    }

    /// Ejecuta las comprobaciones síncronas de tamaño, hash, cabecera binaria y reemplazo atómico.
    fn validate_and_replace_target(
        &self,
        payload: &[u8],
        expected_sha256: Option<&str>,
        label: Option<&str>,
        target_exe: &Path,
        expected_triple: &str,
    ) -> Result<LocalUpdateApplied, UpdaterError> {
        if payload.is_empty() {
            return Err(UpdaterError::EmptyPayload);
        }
        if payload.len() > MAX_UPDATE_PAYLOAD_BYTES {
            return Err(UpdaterError::PayloadTooLarge {
                size: payload.len(),
                max: MAX_UPDATE_PAYLOAD_BYTES,
            });
        }

        let sha256 = verify_optional_sha256(payload, expected_sha256)?;
        validate_executable_for_triple(payload, expected_triple)?;

        let staging_path = get_staging_executable_path(target_exe);
        write_staging_binary(&staging_path, payload)?;
        replace_current_executable(&staging_path, target_exe)?;

        let label_str = label
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("compilacion-local")
            .to_string();

        tracing::info!(
            label = %label_str,
            size_bytes = payload.len(),
            sha256 = %sha256,
            target = %target_exe.display(),
            "Binario local validado e instalado atómicamente"
        );

        Ok(LocalUpdateApplied {
            sha256,
            size_bytes: payload.len(),
            target_path: target_exe.to_string_lossy().to_string(),
            label: label_str,
            managed_by_systemd: is_managed_by_systemd(),
        })
    }

    /// Revierte el ejecutable actual restaurando de forma atómica la copia `.old` previa.
    pub async fn rollback_update(&self) -> Result<(), UpdaterError> {
        let current_exe = get_current_executable_path()?;
        self.rollback_update_to_target(&current_exe, UpdateManifest::current_platform_triple())
            .await
    }

    /// Revierte `target_exe` hacia su copia `.old` previa validando primero la cabecera ELF/PE.
    pub async fn rollback_update_to_target(
        &self,
        target_exe: &Path,
        expected_triple: &str,
    ) -> Result<(), UpdaterError> {
        let Some(_busy_guard) = BusyGuard::try_acquire(&self.is_busy) else {
            return Err(UpdaterError::UpdateInProgress);
        };

        match rollback_to_backup(target_exe, expected_triple) {
            Ok(restored) => {
                let mut st = self.status.write().await;
                *st = UpdateStatusSnapshot::ReadyToInstall {
                    version: "rollback".to_string(),
                    staging_path: restored.to_string_lossy().to_string(),
                };
                tracing::info!(
                    target = %restored.display(),
                    "Rollback atómico completado desde copia de respaldo .old"
                );
                Ok(())
            }
            Err(err) => {
                self.set_error_status(err.to_string()).await;
                Err(err)
            }
        }
    }
}

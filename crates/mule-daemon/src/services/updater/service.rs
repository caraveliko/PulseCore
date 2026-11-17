// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Servicio gestor de actualizaciones en segundo plano para eMuleRust.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use super::constants::{
    DEFAULT_RELEASE_PUBKEY_HEX, DOWNLOAD_PAYLOAD_TIMEOUT, MANIFEST_FETCH_TIMEOUT,
    MAX_UPDATE_PAYLOAD_BYTES,
};
use super::error::UpdaterError;
use super::installer::{
    get_current_executable_path, get_staging_executable_path, replace_current_executable,
    write_staging_binary,
};
use super::manifest::{TargetAsset, UpdateManifest};
use super::verifier::verify_payload;

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

/// Servicio que orquesta la verificación, descarga y aplicación de versiones.
pub struct UpdaterService {
    manifest_url: RwLock<String>,
    public_key_hex: RwLock<String>,
    current_version: String,
    status: RwLock<UpdateStatusSnapshot>,
    cached_asset: RwLock<Option<(String, TargetAsset)>>,
    is_busy: AtomicBool,
    restart_pending: AtomicBool,
}

impl UpdaterService {
    /// Crea una nueva instancia del servicio de actualización con los valores predeterminados.
    pub fn new(manifest_url: String, current_version: String) -> Self {
        Self {
            manifest_url: RwLock::new(manifest_url),
            public_key_hex: RwLock::new(DEFAULT_RELEASE_PUBKEY_HEX.to_string()),
            current_version,
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

    /// Sobrescribe la clave pública de verificación Ed25519 (útil para pruebas de laboratorio).
    pub async fn set_public_key_hex(&self, pubkey_hex: String) {
        let mut key = self.public_key_hex.write().await;
        *key = pubkey_hex;
    }

    /// Obtiene la instantánea actual del estado de la actualización.
    pub async fn get_status(&self) -> UpdateStatusSnapshot {
        self.status.read().await.clone()
    }

    /// Realiza una consulta al manifiesto remoto para comprobar si existe una versión más reciente.
    pub async fn check_for_updates(&self) -> Result<UpdateStatusSnapshot, UpdaterError> {
        if self.is_busy.swap(true, Ordering::SeqCst) {
            return Ok(self.get_status().await);
        }

        {
            let mut st = self.status.write().await;
            *st = UpdateStatusSnapshot::Checking;
        }

        let url = self.manifest_url.read().await.clone();
        let current_ver = self.current_version.clone();

        tracing::info!(manifest_url = %url, current_version = %current_ver, "Comprobando actualizaciones de eMuleRust");

        let client = reqwest::Client::builder()
            .timeout(MANIFEST_FETCH_TIMEOUT)
            .build()
            .map_err(UpdaterError::Network)?;

        let response = match client.get(&url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                let err_msg = format!("Fallo al conectar con el servidor de versiones: {e}");
                tracing::warn!("{err_msg}");
                let mut st = self.status.write().await;
                *st = UpdateStatusSnapshot::Error { message: err_msg.clone() };
                self.is_busy.store(false, Ordering::SeqCst);
                return Err(UpdaterError::Network(e));
            }
        };

        let body_text = match response.text().await {
            Ok(txt) => txt,
            Err(e) => {
                let err_msg = format!("Error al leer el cuerpo del manifiesto: {e}");
                let mut st = self.status.write().await;
                *st = UpdateStatusSnapshot::Error { message: err_msg.clone() };
                self.is_busy.store(false, Ordering::SeqCst);
                return Err(UpdaterError::Network(e));
            }
        };

        let manifest = match UpdateManifest::parse_str(&body_text) {
            Ok(m) => m,
            Err(e) => {
                let err_msg = format!("Manifiesto inválido recibido: {e}");
                let mut st = self.status.write().await;
                *st = UpdateStatusSnapshot::Error { message: err_msg };
                self.is_busy.store(false, Ordering::SeqCst);
                return Err(e);
            }
        };

        let triple = UpdateManifest::current_platform_triple();
        let asset = match manifest.resolve_target(triple) {
            Some(a) => a.clone(),
            None => {
                let err = UpdaterError::UnsupportedPlatform(triple.to_string());
                let mut st = self.status.write().await;
                *st = UpdateStatusSnapshot::Error {
                    message: format!("Plataforma no soportada en esta versión: {triple}"),
                };
                self.is_busy.store(false, Ordering::SeqCst);
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

        self.is_busy.store(false, Ordering::SeqCst);
        Ok(new_snapshot)
    }

    /// Descarga el paquete correspondiente, valida SHA-256 y firma Ed25519, y lo escribe en staging.
    pub async fn download_and_stage(&self) -> Result<PathBuf, UpdaterError> {
        let (version, asset) = {
            let cached = self.cached_asset.read().await;
            match cached.as_ref() {
                Some(val) => val.clone(),
                None => return Err(UpdaterError::NoUpdateReady),
            }
        };

        if asset.size > MAX_UPDATE_PAYLOAD_BYTES {
            return Err(UpdaterError::PayloadTooLarge {
                size: asset.size,
                max: MAX_UPDATE_PAYLOAD_BYTES,
            });
        }

        {
            let mut st = self.status.write().await;
            *st = UpdateStatusSnapshot::Downloading {
                downloaded_bytes: 0,
                total_bytes: asset.size,
            };
        }

        tracing::info!(version = %version, url = %asset.url, "Iniciando descarga de actualización");

        let client = reqwest::Client::builder()
            .timeout(DOWNLOAD_PAYLOAD_TIMEOUT)
            .build()
            .map_err(UpdaterError::Network)?;

        let response = client
            .get(&asset.url)
            .send()
            .await
            .map_err(UpdaterError::Network)?;

        let start_time = Instant::now();
        let payload_bytes = response.bytes().await.map_err(UpdaterError::Network)?;

        if payload_bytes.len() > MAX_UPDATE_PAYLOAD_BYTES {
            return Err(UpdaterError::PayloadTooLarge {
                size: payload_bytes.len(),
                max: MAX_UPDATE_PAYLOAD_BYTES,
            });
        }

        tracing::info!(
            bytes = payload_bytes.len(),
            duration_ms = start_time.elapsed().as_millis(),
            "Descarga finalizada. Procediendo con verificación criptográfica"
        );

        // Verificación de integridad y autenticidad
        let pubkey_hex = self.public_key_hex.read().await.clone();
        verify_payload(
            &payload_bytes,
            &asset.sha256,
            &asset.signature,
            &pubkey_hex,
        )?;

        // Escritura en staging
        let current_exe = get_current_executable_path()?;
        let staging_path = get_staging_executable_path(&current_exe);
        write_staging_binary(&staging_path, &payload_bytes)?;

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

    /// Aplica el reemplazo del binario sobre el ejecutable actual.
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

        replace_current_executable(&staging_path, &current_exe)?;

        tracing::info!("Sustitución completada con éxito. Listo para reinicio ordenado");
        Ok(())
    }
}

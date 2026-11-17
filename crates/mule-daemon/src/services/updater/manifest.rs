// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Estructuras y utilidades de análisis del manifiesto de actualización de eMuleRust.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use super::error::UpdaterError;

/// Metadatos de un paquete compilado para una plataforma y arquitectura concreta.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetAsset {
    /// URL de descarga HTTPS directa del binario empaquetado o archivo comprimido.
    pub url: String,
    /// Suma de comprobación SHA-256 en formato hexadecimal en minúsculas.
    pub sha256: String,
    /// Tamaño exacto en bytes del archivo.
    pub size: usize,
    /// Firma criptográfica Ed25519 en formato hexadecimal o Base64.
    pub signature: String,
    /// Ruta interna del binario dentro del archivo comprimido, si procede.
    #[serde(rename = "binaryRelativePath", skip_serializing_if = "Option::is_none")]
    pub binary_relative_path: Option<String>,
}

/// Manifiesto descriptivo de una versión de lanzamiento publicada.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateManifest {
    /// Cadena de versión de la publicación (p. ej. "0.3.1").
    pub version: String,
    /// Versión mínima requerida para soportar actualización directa sin migración forzada.
    #[serde(rename = "minSupportedVersion", skip_serializing_if = "Option::is_none")]
    pub min_supported_version: Option<String>,
    /// Fecha de publicación en formato ISO 8601 UTC.
    #[serde(rename = "releaseDate")]
    pub release_date: String,
    /// Canal de lanzamiento ("stable", "beta", "nightly").
    pub channel: String,
    /// Enlace opcional a las notas de versión o registro de cambios (*changelog*).
    #[serde(rename = "changelogUrl", skip_serializing_if = "Option::is_none")]
    pub changelog_url: Option<String>,
    /// Resumen conciso en texto de las novedades principales.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Mapa de paquetes indexados por triple de destino de Rust.
    pub targets: HashMap<String, TargetAsset>,
}

impl UpdateManifest {
    /// Deserializa y valida el contenido de un manifiesto JSON en texto.
    pub fn parse_str(json_str: &str) -> Result<Self, UpdaterError> {
        let manifest: Self = serde_json::from_str(json_str)?;
        Ok(manifest)
    }

    /// Localiza el activo binario compatible con la plataforma indicada.
    pub fn resolve_target(&self, triple: &str) -> Option<&TargetAsset> {
        self.targets.get(triple)
    }

    /// Determina el triple de arquitectura y sistema operativo en el que se ejecuta el proceso.
    pub fn current_platform_triple() -> &'static str {
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            "x86_64-pc-windows-msvc"
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            "x86_64-unknown-linux-gnu"
        }
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        {
            "aarch64-unknown-linux-gnu"
        }
        #[cfg(not(any(
            all(target_os = "windows", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64")
        )))]
        {
            "unsupported"
        }
    }

    /// Compara si la versión del manifiesto es estrictamente más reciente que la versión actual.
    ///
    /// Soporta esquemas semánticos numéricos básicos `mayor.menor.parche` con sufijos optativos.
    pub fn is_newer_than(&self, current_version: &str) -> bool {
        let parse_version = |v: &str| -> (u64, u64, u64, bool) {
            let base = v.split('-').next().unwrap_or(v);
            let mut parts = base.split('.').filter_map(|s| s.parse::<u64>().ok());
            let major = parts.next().unwrap_or(0);
            let minor = parts.next().unwrap_or(0);
            let patch = parts.next().unwrap_or(0);
            let is_prerelease = v.contains('-');
            (major, minor, patch, is_prerelease)
        };

        let remote = parse_version(&self.version);
        let local = parse_version(current_version);

        if remote.0 != local.0 {
            return remote.0 > local.0;
        }
        if remote.1 != local.1 {
            return remote.1 > local.1;
        }
        if remote.2 != local.2 {
            return remote.2 > local.2;
        }

        // Si los tres números coinciden pero la versión local es prerrelease y la remota es final
        !remote.3 && local.3
    }
}

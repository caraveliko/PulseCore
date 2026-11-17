// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Constantes y límites del servicio de actualización automática de eMuleRust.

use std::time::Duration;

/// Clave pública maestra Ed25519 de lanzamiento (32 bytes en formato hexadecimal).
///
/// Corresponde a la clave pública autorizada para validar los binarios y
/// manifiestos oficiales de lanzamiento. En pruebas de laboratorio se puede inyectar
/// una clave efímera alternativa.
pub const DEFAULT_RELEASE_PUBKEY_HEX: &str =
    "97cbc1bbc261223375bff61abeea87359e2b98116722ba39b6962a34305bd334";

/// Tiempo máximo de espera para la consulta HTTP del manifiesto de actualización.
pub const MANIFEST_FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Tiempo máximo de espera para la descarga de un paquete de actualización.
pub const DOWNLOAD_PAYLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Tamaño máximo admisible del payload de actualización en bytes (64 MiB).
/// Evita ataques de agotamiento de memoria o almacenamiento en disco.
pub const MAX_UPDATE_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

/// Intervalo predeterminado de comprobación periódica en segundo plano (24 horas).
pub const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_secs(86400);

/// Sufijo temporal para el binario preparado en fase de verificación (*staging*).
pub const UPDATE_STAGING_SUFFIX: &str = ".update";

/// Sufijo de respaldo para el binario reemplazado en plataformas Windows (*move-aside*).
pub const UPDATE_BACKUP_SUFFIX: &str = ".old";

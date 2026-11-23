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

/// Retraso inicial antes de la primera comprobación del supervisor de autoactualización.
pub const AUTO_UPDATE_INITIAL_DELAY: Duration = Duration::from_secs(90);

/// Intervalo de comprobación periódica cuando la autoactualización desatendida está activa (6 horas).
pub const AUTO_UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);

/// Sufijo temporal para el binario preparado en fase de verificación (*staging*).
pub const UPDATE_STAGING_SUFFIX: &str = ".update";

/// Sufijo de respaldo para el binario reemplazado en plataformas Windows (*move-aside*).
pub const UPDATE_BACKUP_SUFFIX: &str = ".old";

/// Sufijo temporal utilizado al comprobar permisos de escritura en el directorio del binario.
pub const WRITE_PROBE_SUFFIX: &str = ".write-probe";

/// Número máximo de redirecciones HTTP validadas al consultar o descargar una actualización.
pub const MAX_UPDATE_REDIRECTS: usize = 5;

/// Incremento mínimo de bytes descargados antes de refrescar la instantánea de progreso en memoria.
pub const PROGRESS_UPDATE_MIN_BYTES: usize = 64 * 1024;

/// Margen de espera (ms) antes de disparar el apagado para entregar la respuesta RPC/HTTP al cliente.
pub const RESTART_RESPONSE_GRACE_MS: u64 = 250;

/// Firma mágica de los ejecutables ELF en Linux.
pub const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

/// Clase ELF correspondiente a ejecutables de 64 bits (`ELFCLASS64`).
pub const ELF_CLASS_64: u8 = 2;

/// Codificación ELF en formato *little-endian* (`ELFDATA2LSB`).
pub const ELF_DATA_LITTLE_ENDIAN: u8 = 1;

/// Identificador de arquitectura ELF para Linux x86_64 (`EM_X86_64`).
pub const ELF_MACHINE_X86_64: u16 = 0x003E;

/// Identificador de arquitectura ELF para Linux ARM64 (`EM_AARCH64`).
pub const ELF_MACHINE_AARCH64: u16 = 0x00B7;

/// Firma mágica de los ejecutables PE en Windows (`MZ`).
pub const PE_MAGIC: [u8; 2] = *b"MZ";

/// Longitud mínima de cabecera exigida para validar formato y arquitectura del binario.
pub const MIN_EXECUTABLE_HEADER_BYTES: usize = 20;

/// Cabecera HTTP opcional con la suma SHA-256 esperada en la subida de un binario local.
pub const HEADER_UPDATE_SHA256: &str = "x-update-sha256";

/// Cabecera HTTP opcional con el nombre descriptivo del binario local subido.
pub const HEADER_UPDATE_FILENAME: &str = "x-update-filename";

/// Cabecera HTTP opcional que controla si el demonio debe reiniciarse tras aplicar la subida.
pub const HEADER_UPDATE_AUTO_RESTART: &str = "x-auto-restart";

/// Endpoint oficial de la API de GitHub Releases para resolver lanzamientos Beta/prelanzamiento.
pub const DEFAULT_GITHUB_RELEASES_API_URL: &str =
    "https://api.github.com/repos/caraveliko/PulseCore/releases?per_page=15";

/// Nombre del archivo de manifiesto firmado adjunto a cada Release.
pub const MANIFEST_ASSET_NAME: &str = "manifest.json";

/// Identificador del canal estable de actualizaciones.
pub const UPDATE_CHANNEL_STABLE: &str = "stable";

/// Identificador del canal beta (incluye prelanzamientos) de actualizaciones.
pub const UPDATE_CHANNEL_BETA: &str = "beta";

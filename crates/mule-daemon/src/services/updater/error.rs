// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Errores tipados para el servicio de actualización de eMuleRust.

use thiserror::Error;

/// Errores derivados de la comprobación, descarga y aplicación de actualizaciones.
#[derive(Debug, Error)]
pub enum UpdaterError {
    /// Error en la conexión o protocolo HTTP.
    #[error("Error en la petición de red HTTP: {0}")]
    Network(#[from] reqwest::Error),

    /// El tamaño de la descarga supera el límite de seguridad presupuestado.
    #[error("El paquete excede el tamaño máximo permitido ({size} > {max})")]
    PayloadTooLarge { size: usize, max: usize },

    /// El archivo de manifiesto no tiene un formato JSON válido.
    #[error("Manifiesto de actualización no válido: {0}")]
    InvalidManifest(#[from] serde_json::Error),

    /// La arquitectura o sistema operativo actual no dispone de paquete en el manifiesto.
    #[error("No se ha encontrado un binario compatible para la plataforma '{0}'")]
    UnsupportedPlatform(String),

    /// La suma de comprobación SHA-256 no coincide con el manifiesto.
    #[error("Discrepancia en la suma SHA-256 (calculada: {calculated}, esperada: {expected})")]
    ChecksumMismatch {
        calculated: String,
        expected: String,
    },

    /// La clave pública Ed25519 no tiene un formato hexadecimal o longitud válida.
    #[error("Clave pública Ed25519 inválida: {0}")]
    InvalidPublicKey(String),

    /// La firma Ed25519 no tiene un formato o longitud válida.
    #[error("Firma Ed25519 mal formada: {0}")]
    InvalidSignatureFormat(String),

    /// La firma Ed25519 del binario es inválida o ha sido manipulada.
    #[error("Verificación criptográfica Ed25519 fallida: firma no auténtica")]
    CryptographicSignatureInvalid,

    /// Error de entrada/salida en el sistema de archivos durante la sustitución.
    #[error("Error de E/S en disco: {0}")]
    Io(#[from] std::io::Error),

    /// El proceso no dispone de permisos suficientes para sustituir el ejecutable.
    #[error("Permisos insuficientes para reemplazar el binario en '{0}'")]
    PermissionDenied(String),

    /// No se ha podido resolver la ruta absoluta del ejecutable en ejecución.
    #[error("Imposible determinar la ruta del ejecutable actual")]
    ExecutablePathUnknown,

    /// No hay ninguna actualización disponible o lista para instalar.
    #[error("No hay ninguna actualización preparada para su aplicación")]
    NoUpdateReady,

    /// El paquete binario subido o recibido no contiene datos.
    #[error("El archivo binario proporcionado está vacío")]
    EmptyPayload,

    /// El archivo no posee una cabecera ejecutable válida para la plataforma destino.
    #[error("Formato de ejecutable no válido: {0}")]
    InvalidExecutableFormat(String),

    /// El binario corresponde a una arquitectura incompatible con el servidor.
    #[error(
        "Arquitectura del binario incompatible: este servidor requiere {expected}, pero el archivo corresponde a {actual}"
    )]
    ArchitectureMismatch { expected: String, actual: String },

    /// El servidor remoto respondió con un código de estado HTTP distinto de éxito.
    #[error("El servidor de versiones respondió con HTTP {status} al consultar {url}")]
    HttpStatus { status: u16, url: String },

    /// La política de red o el interruptor de corte impidió realizar la petición saliente.
    #[error("Política de red bloqueó la solicitud de actualización: {0}")]
    NetworkBlocked(String),

    /// Ya existe una comprobación, descarga o aplicación de actualización en curso.
    #[error("Ya hay una operación de actualización en curso")]
    UpdateInProgress,

    /// Se excedió el límite seguro de redirecciones HTTP en la descarga o consulta.
    #[error("Se superó el límite de {max} redirecciones HTTP al descargar la actualización")]
    TooManyRedirects { max: usize },

    /// Canal de actualización desconocido o sin lanzamientos disponibles.
    #[error("Canal o lanzamiento de actualización no válido: {0}")]
    InvalidChannel(String),

    /// No existe ninguna copia de respaldo previa disponible para revertir la actualización.
    #[error("No existe copia de respaldo (.old) disponible para revertir la actualización")]
    NoRollbackAvailable,
}

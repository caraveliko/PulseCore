// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Etiquetas del protocolo eD2K/eMule, identificadores de pares y códigos de operación estándar.

use std::fmt;

/// Byte identificador del protocolo eD2K estándar (`0xE3`).
pub const PR_ED2K: u8 = 0xE3;

/// Byte identificador del protocolo eMule extendido (`0xC5`).
pub const PR_EMULE: u8 = 0xC5;

/// Identificador de protocolo de cable reconocido para cargas útiles comprimidas de eMule (`0xD4`).
///
/// NOTA: El soporte de entramado reconoce y enmarca estos paquetes en el límite de transporte;
/// la descompresión conforme y los límites de seguridad posteriores a la descompresión (topes de
/// tasa de descompresión, límites máximos de longitud según tipo de mensaje y mitigación de bombas
/// de descompresión) se implementan en una etapa de descompresión dedicada.
pub const PR_EMULE_COMPRESSED: u8 = 0xD4;

/// Alias de [`PR_EMULE_COMPRESSED`] (`0xD4`).
pub const PR_ZLIB: u8 = PR_EMULE_COMPRESSED;

/// Identificador de protocolo para pruebas de laboratorio y entorno de pruebas determinista en bucle local (`0xEE`).
///
/// Separado explícitamente de los formatos de cable históricos de eD2K para evitar fugas accidentales
/// o afirmaciones prematuras de interoperabilidad con la red eMule externa.
pub const PR_LAB: u8 = 0xEE;

/// Verifica si el byte proporcionado es un byte mágico de protocolo reconocido.
#[inline]
pub fn is_valid_protocol(protocol: u8) -> bool {
    matches!(protocol, PR_ED2K | PR_EMULE | PR_EMULE_COMPRESSED | PR_LAB)
}

/// Identificador de par de 16 bytes (hash de usuario).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId(pub [u8; 16]);

impl PeerId {
    /// Crea un nuevo `PeerId` a partir de una matriz de 16 bytes.
    #[inline]
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Crea un `PeerId` a partir de un fragmento de bytes si su longitud es exactamente de 16 bytes.
    pub fn from_slice(slice: &[u8]) -> Option<Self> {
        if slice.len() == 16 {
            let mut arr = [0u8; 16];
            arr.copy_from_slice(slice);
            Some(Self(arr))
        } else {
            None
        }
    }

    /// Analiza una cadena hexadecimal de 32 caracteres en un `PeerId`.
    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        let mut bytes = [0u8; 16];
        hex::decode_to_slice(s, &mut bytes)?;
        Ok(Self(bytes))
    }

    /// Da formato al `PeerId` como una cadena hexadecimal en minúsculas de 32 caracteres.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Devuelve una referencia a los 16 bytes subyacentes.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

// ---------------------------------------------------------------------------
// Códigos de operación estándar de cliente a cliente eD2K (Protocolo 0xE3)
// ---------------------------------------------------------------------------

/// Inicialización del protocolo de enlace con el par.
pub const OP_HELLO: u8 = 0x01;
/// Respuesta a OP_HELLO.
pub const OP_HELLO_ANSWER: u8 = 0x4C;
/// Petición de nombre de archivo por hash.
pub const OP_REQUESTFILENAME: u8 = 0x59;
/// Respuesta con el nombre de archivo.
pub const OP_REQFILENAME_ANSWER: u8 = 0x59;
/// Informa al par qué hash de archivo estamos solicitando.
pub const OP_SETREQFILEID: u8 = 0x47;
/// Respuesta que indica que el archivo solicitado no está compartido.
pub const OP_FILEREQANSNOFILE: u8 = 0x48;
/// Petición del estado del archivo remoto (máscara de bits de disponibilidad de partes).
pub const OP_FILESTATUSREQ: u8 = 0x49;
/// Respuesta con la máscara de bits del estado de disponibilidad de partes.
pub const OP_FILESTATUS: u8 = 0x50;
/// Petición para entrar en la cola de subida.
pub const OP_STARTUPLOADREQ: u8 = 0x54;
/// El par aceptó la petición de subida; el puesto está listo.
pub const OP_ACCEPTUPLOADREQ: u8 = 0x55;
/// Cancela la transferencia activa o retira de la cola de subida.
pub const OP_CANCELTRANSFER: u8 = 0x56;
/// Informa al par de que ya no necesitamos las partes.
pub const OP_OUTOFPARTS: u8 = 0x57;
/// Petición de un rango específico de bloques dentro de una parte.
pub const OP_REQUESTPARTS: u8 = 0x4B;
/// Respuesta con la carga útil del bloque de datos.
pub const OP_SENDINGPART: u8 = 0x46;
/// eMule: señaliza que el cliente completó la descarga de este archivo (`0x49`).
/// Mismo valor en cable que el `OP_FILESTATUSREQ` heredado anterior.
pub const OP_END_OF_DOWNLOAD: u8 = 0x49;
/// eMule: notificación de cambio de ID de cliente (`0x4D`) —
/// `<u32 newUserID><u32 newServerIP>` (`ListenSocket.cpp:590`).
pub const OP_CHANGE_CLIENT_ID: u8 = 0x4D;
/// Petición del conjunto de hashes (hashset) para un archivo multipartición.
pub const OP_HASHSETREQUEST: u8 = 0x51;
/// Respuesta con la carga útil del conjunto de hashes (hashset).
pub const OP_HASHSETANSWER: u8 = 0x52;

// ---------------------------------------------------------------------------
// Códigos de operación eMule extendidos (Protocolo 0xC5)
// ---------------------------------------------------------------------------

/// Intercambio de capacidades del protocolo de enlace extendido de eMule.
pub const OP_EMULE_INFO: u8 = 0x01;
/// Respuesta de capacidades del protocolo de enlace extendido de eMule.
pub const OP_EMULE_INFO_ANSWER: u8 = 0x02;
/// Petición de hash raíz AICH.
pub const OP_AICH_REQUEST: u8 = 0x12;
/// Respuesta de hash raíz AICH.
pub const OP_AICH_ANSWER: u8 = 0x13;
/// Petición de conjunto de hashes / árbol de prueba de recuperación AICH para una parte dañada.
pub const OP_AICHFILEHASHREQ: u8 = 0x14;
/// Respuesta con el conjunto de hashes / árbol de prueba de recuperación AICH.
pub const OP_AICHFILEHASHRES: u8 = 0x15;

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Estructuras pasivas de observación de datos que representan mensajes de red eD2K decodificados.

use super::tags::{Ed2kTag, RawTag};
use crate::framing::RawPacket;

/// Estructura de observación pasiva para `OP_HELLO` (`0x01`).
#[derive(Debug, Clone, PartialEq)]
pub struct Ed2kHello {
    /// Hash de usuario MD4 de 16 bytes.
    pub user_hash: [u8; 16],
    /// ID de cliente eD2K (IPv4 o LowID).
    pub client_id: u32,
    /// Puerto TCP de escucha.
    pub tcp_port: u16,
    /// Etiquetas de metadatos reconocidas y decodificadas.
    pub tags: Vec<Ed2kTag>,
    /// Etiquetas no reconocidas o sin procesar conservadas sin pérdida de datos.
    pub unknown_tags: Vec<RawTag>,
    /// Dirección IPv4 del servidor conectado (o 0 si no está conectado).
    pub server_ip: u32,
    /// Puerto del servidor conectado (o 0 si no está conectado).
    pub server_port: u16,
}

impl Ed2kHello {
    /// Busca una etiqueta reconocida por su cadena de visualización de nombre.
    pub fn find_tag(&self, name: &str) -> Option<&Ed2kTag> {
        crate::ed2k::tags::find_tag_by_name(&self.tags, name)
    }
}

/// Estructura de observación pasiva para `OP_HELLO_ANSWER` (`0x4C`).
#[derive(Debug, Clone, PartialEq)]
pub struct Ed2kHelloAnswer {
    /// Hash de usuario MD4 de 16 bytes del par remoto.
    pub user_hash: [u8; 16],
    /// ID de cliente eD2K del par remoto.
    pub client_id: u32,
    /// Puerto TCP de escucha del par remoto.
    pub tcp_port: u16,
    /// Etiquetas de metadatos reconocidas y decodificadas.
    pub tags: Vec<Ed2kTag>,
    /// Etiquetas no reconocidas o sin procesar conservadas sin pérdida de datos.
    pub unknown_tags: Vec<RawTag>,
    /// Dirección IPv4 del servidor conectado del par remoto.
    pub server_ip: u32,
    /// Puerto del servidor conectado del par remoto.
    pub server_port: u16,
}

impl Ed2kHelloAnswer {
    /// Busca una etiqueta reconocida por su cadena de visualización de nombre.
    pub fn find_tag(&self, name: &str) -> Option<&Ed2kTag> {
        crate::ed2k::tags::find_tag_by_name(&self.tags, name)
    }
}

/// Estructura de observación para el anuncio de extensiones de protocolo eMule `OP_EMULEINFO` (`0xC5 0x01`).
#[derive(Debug, Clone, PartialEq)]
pub struct Ed2kEmuleInfo {
    /// Código de versión del cliente eMule remoto (p. ej. 0x3C = 60).
    pub client_version: u8,
    /// Byte de versión del protocolo eMule (habitualmente 0x01).
    pub protocol_version: u8,
    /// Etiquetas de metadatos reconocidas y decodificadas.
    pub tags: Vec<Ed2kTag>,
    /// Etiquetas no reconocidas o sin procesar conservadas sin pérdida de datos.
    pub unknown_tags: Vec<RawTag>,
}

impl Ed2kEmuleInfo {
    /// Busca una etiqueta reconocida por su cadena de visualización de nombre.
    pub fn find_tag(&self, name: &str) -> Option<&Ed2kTag> {
        crate::ed2k::tags::find_tag_by_name(&self.tags, name)
    }

    /// Devuelve true si el par remoto anunció la etiqueta de capacidad de compresión.
    pub fn has_compression_capability(&self) -> bool {
        self.tags.iter().any(|t| match &t.name {
            super::tags::TagName::Id(0x20) | super::tags::TagName::Id(0x01) => true,
            super::tags::TagName::Text(s) => s.eq_ignore_ascii_case("compression"),
            _ => false,
        })
    }

    /// Devuelve true si el par remoto anunció la etiqueta de capacidad de intercambio de fuentes (source exchange).
    pub fn has_source_exchange_capability(&self) -> bool {
        self.tags.iter().any(|t| match &t.name {
            super::tags::TagName::Id(super::tags::TAG_SPECIAL_EMULE_EXT_SOURCEEXCHANGE)
            | super::tags::TagName::Id(0x04) => true,
            super::tags::TagName::Text(s) => {
                s.eq_ignore_ascii_case("sourceexchange")
                    || s.eq_ignore_ascii_case("source_exchange")
            }
            _ => false,
        })
    }
}

/// Función auxiliar para calcular el número esperado de bytes para un número de partes dado.
#[inline]
pub fn expected_mask_bytes(part_count: u16) -> usize {
    usize::from(part_count).div_ceil(8)
}

/// Máscara de disponibilidad de partes encapsulada y acotada con seguridad.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartAvailabilityMask {
    raw: bytes::Bytes,
    part_count: u16,
}

impl PartAvailabilityMask {
    /// Crea una máscara a partir de bytes sin procesar, comprobando la longitud exacta y ceros en bits no utilizados finales.
    pub fn from_bytes(
        raw: bytes::Bytes,
        part_count: u16,
    ) -> Result<Self, super::error::Ed2kErrorKind> {
        let expected = expected_mask_bytes(part_count);
        if raw.len() != expected {
            return Err(super::error::Ed2kErrorKind::MalformedField {
                message: format!(
                    "mask length {} does not match required {} for part_count {}",
                    raw.len(),
                    expected,
                    part_count
                ),
            });
        }

        // Comprobar bits sobrantes finales si part_count % 8 != 0
        let remainder = (part_count % 8) as usize;
        if remainder != 0 && !raw.is_empty() {
            let last_byte = raw[raw.len() - 1];
            // Máscara de bits válidos: bits 0..(remainder - 1)
            let valid_mask = (1u8 << remainder) - 1;
            let trailing_bits = last_byte & !valid_mask;
            if trailing_bits != 0 {
                return Err(super::error::Ed2kErrorKind::InvalidBitfieldTrailingBits {
                    remainder,
                    trailing_bits,
                });
            }
        }

        Ok(Self { raw, part_count })
    }

    /// Crea una nueva máscara a partir de un fragmento de índices de partes disponibles.
    pub fn from_parts(available_parts: &[u16], part_count: u16) -> Self {
        let expected = expected_mask_bytes(part_count);
        let mut buf = vec![0u8; expected];
        for &part in available_parts {
            if part < part_count {
                let byte_idx = (part / 8) as usize;
                let bit_idx = (part % 8) as usize;
                buf[byte_idx] |= 1 << bit_idx;
            }
        }
        Self {
            raw: bytes::Bytes::from(buf),
            part_count,
        }
    }

    /// Crea una máscara vacía (0 partes).
    pub fn empty() -> Self {
        Self {
            raw: bytes::Bytes::new(),
            part_count: 0,
        }
    }

    /// Número total de partes que cubre esta máscara.
    pub fn part_count(&self) -> u16 {
        self.part_count
    }

    /// Devuelve true si el par remoto tiene disponible la parte especificada.
    pub fn has_part(&self, part_index: u16) -> bool {
        if part_index >= self.part_count {
            return false;
        }
        let byte_idx = (part_index / 8) as usize;
        let bit_idx = (part_index % 8) as usize;
        if byte_idx < self.raw.len() {
            (self.raw[byte_idx] & (1 << bit_idx)) != 0
        } else {
            false
        }
    }

    /// Devuelve un iterador sobre todos los índices de partes disponibles.
    pub fn available_parts(&self) -> impl Iterator<Item = u16> + '_ {
        (0..self.part_count).filter(move |&idx| self.has_part(idx))
    }

    /// Devuelve los bytes sin procesar subyacentes de la máscara.
    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw
    }

    /// Devuelve el búfer `Bytes` subyacente.
    pub fn into_bytes(self) -> bytes::Bytes {
        self.raw
    }
}

/// Petición de máscara de bits de disponibilidad de partes de fichero remoto (`0x49`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kFileStatusReq {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
}

/// Estructura de observación pasiva para `OP_FILESTATUS` (`0x50`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kFileStatus {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
    /// Número total de partes del fichero.
    pub part_count: u16,
    /// Máscara de bits de disponibilidad: ceil(part_count / 8) bytes.
    pub availability_mask: PartAvailabilityMask,
}

impl Ed2kFileStatus {
    /// En el protocolo de cable eD2K, los clientes de referencia oficiales eMule/aMule escriben `part_count = 0`
    /// al responder a `OP_SETREQFILEID` para un fichero compartido completado (`CKnownFile`).
    /// En este caso canónico, no se transmite ningún mapa de bits, y `part_count = 0` indica
    /// que el par es una fuente completa que posee el 100 % de las partes del fichero.
    #[inline]
    pub fn is_complete(&self) -> bool {
        if self.part_count == 0 {
            true
        } else {
            self.availability_mask.available_parts().count() == self.part_count as usize
        }
    }

    /// Devuelve true si el par remoto tiene disponible la parte especificada.
    ///
    /// Si `self.part_count == 0`, el par remoto es una fuente completa (`CKnownFile`),
    /// y todas las partes del fichero están disponibles.
    #[inline]
    pub fn has_part(&self, part_index: u16) -> bool {
        if self.part_count == 0 {
            true
        } else {
            self.availability_mask.has_part(part_index)
        }
    }

    /// Accesor de compatibilidad para el fragmento de bytes sin procesar del mapa de bits de disponibilidad.
    #[inline]
    pub fn raw_bytes(&self) -> &[u8] {
        self.availability_mask.raw_bytes()
    }
}

/// Un rango de bytes contiguo dentro de un fichero para peticiones de bloques.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ed2kBlockRange {
    /// Desplazamiento de byte inicial.
    pub start: u32,
    /// Desplazamiento de byte final (exclusivo).
    pub end: u32,
}

impl Ed2kBlockRange {
    /// Crea un nuevo `Ed2kBlockRange`.
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// Longitud del rango en bytes.
    #[inline]
    pub fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    /// Devuelve true si start >= end.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    /// Crea un rango de bloques directamente a partir de los límites canónicos en cable de eMule `(start, end)`.
    ///
    /// En las implementaciones de referencia de eMule/aMule (`DownloadClient.cpp`, `ListenSocket.cpp`),
    /// el desplazamiento final en cable se transmite como **exclusivo**: `endpos = block->EndOffset + 1`.
    /// El recuento de bytes del bloque es estrictamente `end - start`.
    pub fn from_wire_bounds(start: u32, end: u32) -> Result<Self, super::error::Ed2kErrorKind> {
        if start >= end {
            return Err(super::error::Ed2kErrorKind::InvalidBlockOffsets {
                start: start as u64,
                end: end as u64,
            });
        }
        Ok(Self { start, end })
    }

    /// Devuelve los límites canónicos en cable `(start, end)` coincidentes directamente con la convención exclusiva de eMule.
    #[inline]
    pub fn to_wire_bounds(&self) -> (u32, u32) {
        (self.start, self.end)
    }

    /// Convierte una representación en cable de donkey heredado con un desplazamiento final inclusivo `[start, end_inclusive]`
    /// en un rango de bloques semiabierto `[start, end)` utilizado por el motor central.
    pub fn from_legacy_inclusive_wire_bounds(
        start: u32,
        end_inclusive: u32,
    ) -> Result<Self, super::error::Ed2kErrorKind> {
        let end = end_inclusive.checked_add(1).ok_or_else(|| {
            super::error::Ed2kErrorKind::MalformedField {
                message: "integer overflow in legacy inclusive end offset conversion".to_string(),
            }
        })?;
        if start >= end {
            return Err(super::error::Ed2kErrorKind::InvalidBlockOffsets {
                start: start as u64,
                end: end as u64,
            });
        }
        Ok(Self { start, end })
    }

    /// Alias de `from_legacy_inclusive_wire_bounds`.
    #[inline]
    pub fn from_inclusive_wire_bounds(
        start: u32,
        end_inclusive: u32,
    ) -> Result<Self, super::error::Ed2kErrorKind> {
        Self::from_legacy_inclusive_wire_bounds(start, end_inclusive)
    }

    /// Convierte un rango semiabierto `[start, end)` a la representación en cable inclusiva heredada `(start, end_inclusive)`.
    pub fn to_legacy_inclusive_wire_bounds(
        &self,
    ) -> Result<(u32, u32), super::error::Ed2kErrorKind> {
        if self.is_empty() {
            return Err(super::error::Ed2kErrorKind::InvalidBlockOffsets {
                start: self.start as u64,
                end: self.end as u64,
            });
        }
        Ok((self.start, self.end - 1))
    }

    /// Alias de `to_legacy_inclusive_wire_bounds`.
    #[inline]
    pub fn to_inclusive_wire_bounds(&self) -> Result<(u32, u32), super::error::Ed2kErrorKind> {
        self.to_legacy_inclusive_wire_bounds()
    }
}

/// Un rango de bytes contiguo con desplazamientos de 64 bits (familia `OP_REQUESTPARTS_I64`).
///
/// Disposición en cable de eMule `0xA3`: `<HASH 16><start[3] u64><end[3] u64>` (64 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ed2kBlockRange64 {
    /// Desplazamiento de byte inicial.
    pub start: u64,
    /// Desplazamiento de byte final (exclusivo).
    pub end: u64,
}

impl Ed2kBlockRange64 {
    /// Crea un nuevo `Ed2kBlockRange64`.
    pub fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }

    /// Longitud del rango en bytes.
    #[inline]
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// Devuelve true si start >= end.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    /// Devuelve los límites canónicos en cable `(start, end)` (final exclusivo).
    #[inline]
    pub fn to_wire_bounds(&self) -> (u64, u64) {
        (self.start, self.end)
    }
}

/// Petición de bloques de 64 bits (`OP_REQUESTPARTS_I64`, `0xA3`) para ficheros >4 GiB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kRequestPartsI64 {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
    /// Rangos solicitados (hasta 3 en el formato de cable estándar de eMule).
    pub ranges: Vec<Ed2kBlockRange64>,
}

/// Entrega de carga útil de datos de 64 bits (`OP_SENDINGPART_I64`, `0xA2`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kSendingPartI64 {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
    /// Desplazamiento de byte de inicio del bloque.
    pub start_offset: u64,
    /// Desplazamiento de byte de fin del bloque.
    pub end_offset: u64,
    /// Bytes sin procesar de los datos del bloque.
    pub data: bytes::Bytes,
}

/// Entrega de bloque comprimido con zlib de 64 bits (`OP_COMPRESSEDPART_I64`, `0xA1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kCompressedPartI64 {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
    /// Desplazamiento inicial del flujo sin comprimir.
    pub start_offset: u64,
    /// Tamaño de la carga útil comprimida.
    pub compressed_size: u32,
    /// Datos del bloque comprimidos con zlib.
    pub data: bytes::Bytes,
}

/// Entrega de bloque comprimido con zlib de 32 bits heredado (`OP_COMPRESSEDPART`,
/// `0x40`) — `UploadDiskIOThread.cpp:456` de eMule, siempre `PR_EMULE`.
///
/// El campo `compressed_size` transporta el tamaño **total** del flujo zlib
/// en cada fragmento; un único bloque comprimido puede abarcar varios
/// paquetes cuyos campos `data` son fragmentos consecutivos de un mismo flujo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kCompressedPart {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
    /// Desplazamiento inicial del flujo sin comprimir.
    pub start_offset: u32,
    /// Tamaño total del flujo comprimido (repetido en cada fragmento).
    pub compressed_size: u32,
    /// Fragmento del flujo comprimido con zlib.
    pub data: bytes::Bytes,
}

/// Petición de bloques de partes de fichero (`0x4B`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kRequestParts {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
    /// Rangos solicitados (hasta 3 en el formato de cable estándar de eMule).
    pub ranges: Vec<Ed2kBlockRange>,
}

impl Ed2kRequestParts {
    /// Crea una petición de bloque de rango único.
    pub fn single(file_hash: [u8; 16], start: u32, end: u32) -> Self {
        Self {
            file_hash,
            ranges: vec![Ed2kBlockRange::new(start, end)],
        }
    }

    /// Crea una petición de bloque de múltiples rangos.
    pub fn multi(file_hash: [u8; 16], ranges: Vec<Ed2kBlockRange>) -> Self {
        Self { file_hash, ranges }
    }
}

/// Entrega de carga útil de datos de un bloque solicitado (`0x46`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kSendingPart {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
    /// Desplazamiento de byte de inicio del bloque.
    pub start_offset: u32,
    /// Desplazamiento de byte de fin del bloque.
    pub end_offset: u32,
    /// Bytes sin procesar de los datos del bloque.
    pub data: bytes::Bytes,
}

impl Ed2kSendingPart {
    /// Crea un nuevo `Ed2kSendingPart`.
    pub fn new(
        file_hash: [u8; 16],
        start_offset: u32,
        end_offset: u32,
        data: bytes::Bytes,
    ) -> Self {
        Self {
            file_hash,
            start_offset,
            end_offset,
            data,
        }
    }

    /// Devuelve la longitud en bytes de los datos.
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Devuelve true si la carga útil del bloque está vacía.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// Estructura de observación pasiva para `OP_FILEREQANSNOFILE` (`0x48`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kFileReqAnsNoFile {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
}

/// Petición del conjunto completo de hashes MD4 por parte (hashset) de un fichero (`0x51`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kHashSetReq {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
}

/// Respuesta de conjunto de hashes MD4 por parte (`0x52`): `file_hash` + `u16 count` + `count * 16` bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kHashSetAnswer {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
    /// Hash MD4 de cada parte de 9.728.000 bytes, en orden de partes.
    pub part_hashes: Vec<[u8; 16]>,
}

/// Petición del hash maestro (raíz) AICH de un fichero — `OP_AICHFILEHASHREQ`
/// (protocolo eMule `0x9E`): carga útil simple con el hash de fichero de 16 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kAichFileHashReq {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
}

/// Respuesta de hash maestro AICH — `OP_AICHFILEHASHANS` (protocolo eMule `0x9D`):
/// `file_hash` + hash raíz AICH SHA-1 de 20 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kAichFileHashRes {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
    /// Hash maestro (raíz) AICH SHA-1 de 20 bytes.
    pub aich_hash: [u8; 20],
}

/// Petición de datos de recuperación de partes AICH — `OP_AICHREQUEST` (protocolo eMule `0x9B`): `file_hash` + `u16 part` + hash maestro de 20 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kAichRequest {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
    /// Índice de la parte solicitada (partes de 9.728.000 bytes).
    pub part: u16,
    /// Hash maestro AICH SHA-1 de 20 bytes en el que confía el solicitante.
    pub master_hash: [u8; 20],
}

/// Datos de recuperación AICH — `OP_AICHANSWER` (protocolo eMule `0x9C`).
///
/// La carga útil en cable es `file_hash` + `u16 part` + `master_hash` + datos de recuperación.
/// Una respuesta fallida solo transporta el `file_hash` simple de 16 bytes
/// (`recovery == None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kAichAnswer {
    /// Hash de fichero MD4 de 16 bytes.
    pub file_hash: [u8; 16],
    /// Datos de recuperación, ausentes cuando el par no pudo servirlos.
    pub recovery: Option<Ed2kAichRecovery>,
}

/// Entradas de hashes de recuperación de un `OP_AICHANSWER`, divididas por ancho de identificador
/// como en el cable: `u16 count` + `count * (u16 ident + hash[20])` para identificadores
/// de 16 bits, luego `u16 count` + `count * (u32 ident + hash[20])` para identificadores
/// de 32 bits (ficheros grandes). eMule puebla únicamente una de las dos listas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kAichRecovery {
    /// Índice de parte que cubren estos datos de recuperación.
    pub part: u16,
    /// Hash maestro AICH SHA-1 de 20 bytes reflejado desde la petición.
    pub master_hash: [u8; 20],
    /// Entradas `(ident, hash)` con identificadores de 16 bits.
    pub entries16: Vec<(u16, [u8; 20])>,
    /// Entradas `(ident, hash)` con identificadores de 32 bits.
    pub entries32: Vec<(u32, [u8; 20])>,
}

/// Mensaje de clave pública para identificación segura de usuario (`0x85`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kPublicKey {
    /// Bytes sin procesar de la clave pública RSA (típicamente DER o módulo/exponente sin procesar).
    pub key_bytes: bytes::Bytes,
}

/// Mensaje de firma criptográfica para identificación segura de usuario (`0x86`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kSignature {
    /// Vector de bytes de la firma.
    pub signature_bytes: bytes::Bytes,
}

/// Mensaje de estado de identificación segura (`0x87`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kSecIdentState {
    /// Indicador de estado (0 = no admitido, 1 = admitido/solicitado, 2 = autenticado).
    pub state: u8,
    /// Valor de desafío aleatorio de 4 bytes.
    pub challenge: u32,
}

/// Mensaje de chat entre pares (`OP_MESSAGE`, `0x4E`): `<len u16><bytes del mensaje>`.
///
/// eMule acota el cuerpo del mensaje (`MAX_MESSAGE_SIZE`); el prefijo de longitud en cable
/// debe coincidir exactamente con el resto de la carga útil (`ListenSocket.cpp` 0.72a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kChatMessage {
    /// Texto del mensaje (UTF-8 con pérdidas, al igual que `ReadString` de eMule).
    pub text: String,
}

/// Informe de posición en cola de eDonkey heredado (`OP_QUEUERANK`, `0x5C`): `<rank u32>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kQueueRank {
    /// Posición en cola informada por el par remoto (0 = no encolado).
    pub rank: u32,
}

/// Informe de posición en cola de eMule (`OP_QUEUERANKING`, `0x60`, protocolo eMule).
///
/// eMule 0.72a exige una carga útil exacta de 12 bytes y lee el rango como `u16`
/// (`ProcessEmuleQueueRank`, `ListenSocket.cpp`); los 10 bytes restantes están
/// reservados/relleno y se conservan tal cual para viajes de ida y vuelta deterministas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kQueueRanking {
    /// Posición en cola informada por el par remoto.
    pub rank: u16,
    /// 10 bytes reservados tras el rango en el cable.
    pub reserved: [u8; 10],
}

/// Notificación de cambio de slot de subida (`OP_CHANGE_SLOT`, `0x5B`): `<HASH 16>`.
///
/// Tolerada y prácticamente ignorada por eMule 0.72a, conservada por compatibilidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kChangeSlot {
    /// Hash de fichero MD4 de 16 bytes al que hace referencia el cambio de slot.
    pub file_hash: [u8; 16],
}

/// Valoración/comentario de fichero (`OP_FILEDESC`, `0x61`, protocolo eMule):
/// `<rating u8><len u16><bytes de descripción>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kFileDesc {
    /// Valor de puntuación (0 = ninguno/inválido, 1..=5 según convenciones de eMule).
    pub rating: u8,
    /// Descripción en texto libre (UTF-8 con pérdidas).
    pub description: String,
}

/// Prueba de puerto TCP (`OP_PORTTEST`, `0xFE`): byte de estado individual.
///
/// eMule 0.72a responde con `0x12` (`ListenSocket.cpp` caso `OP_PORTTEST`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kPortTest {
    /// Byte de estado/valor (`0x12` en respuestas de eMule).
    pub value: u8,
}

/// Petición de CAPTCHA antispam en el chat (`OP_CHATCAPTCHAREQ`, `0xA5`, protocolo eMule):
/// `<tag_count u8><tags><bytes de mapa de bits BMP>`.
///
/// eMule acota el mapa de bits entre 128 y 4096 bytes y acepta dimensiones
/// de imagen dentro de 10..150 px de ancho y 10..50 px de alto.
#[derive(Debug, Clone, PartialEq)]
pub struct Ed2kCaptchaRequest {
    /// Etiquetas reconocidas decodificadas que preceden al mapa de bits.
    pub tags: Vec<Ed2kTag>,
    /// Etiquetas no reconocidas conservadas sin pérdida de datos.
    pub unknown_tags: Vec<RawTag>,
    /// Bytes sin procesar del mapa de bits BMP.
    pub bitmap: bytes::Bytes,
}

/// Resultado de CAPTCHA de chat (`OP_CHATCAPTCHARES`, `0xA6`): `<status u8>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kCaptchaResponse {
    /// `0` = resuelto, `1` = fallido/requiere reintento (semántica de eMule).
    pub status: u8,
}

/// Petición de conjuntos de hashes de fichero mediante `FileIdentifier` (`OP_HASHSETREQUEST2`,
/// `0xB1`, protocolo eMule): `<FileIdentifier><options u8>`.
///
/// Byte de opciones (`UploadClient.cpp` 0.72a): el bit0 solicita el hashset MD4 por parte,
/// el bit1 el hashset AICH por parte; al menos un bit debe estar activo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kHashSetReq2 {
    /// Identificador compuesto de fichero (hash MD4 + tamaño, opcionalmente raíz AICH).
    pub identifier: super::file_identifier::Ed2kFileIdentifier,
    /// Solicita el conjunto de hashes MD4 por parte (bit de opción `0x01`).
    pub want_md4: bool,
    /// Solicita el conjunto de hashes AICH por parte (bit de opción `0x02`).
    pub want_aich: bool,
}

/// Conjunto de hashes AICH por parte transportado dentro de `OP_HASHSETANSWER2`:
/// `<master hash 20><u16 count><count * 20 bytes>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kAichPartHashSet {
    /// Hash maestro (raíz) AICH SHA-1 de 20 bytes.
    pub master_hash: [u8; 20],
    /// Hash AICH de cada parte de 9.728.000 bytes, en orden de partes.
    pub part_hashes: Vec<[u8; 20]>,
}

/// Respuesta de conjunto de hashes mediante `FileIdentifier` (`OP_HASHSETANSWER2`, `0xB2`):
/// `<FileIdentifier><options u8>[<md4 set>][<aich set>]`.
///
/// Cada conjunto de hashes solicitado se serializa cuando está presente; el byte de opciones refleja
/// qué conjuntos le siguen (bit0 = MD4, bit1 = AICH).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kHashSetAnswer2 {
    /// Identificador compuesto de fichero reflejado desde la petición.
    pub identifier: super::file_identifier::Ed2kFileIdentifier,
    /// Conjunto de hashes MD4 por parte cuando fue solicitado/disponible.
    pub md4_set: Option<Ed2kHashSetAnswer>,
    /// Conjunto de hashes AICH por parte cuando fue solicitado/disponible.
    pub aich_set: Option<Ed2kAichPartHashSet>,
}

/// Información de petición extendida añadida tras `OP_REQFILENAME` dentro de una
/// petición `MULTIPACKET` (`SendFileRequest` en `DownloadClient.cpp` 0.72a):
/// `<u16 part_count><ceil(count/8) bytes de estado de partes>[<u16 complete sources>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kRequestExtInfo {
    /// Número de partes eD2K para las que el solicitante informa el estado.
    pub part_count: u16,
    /// Mapa de bits de disponibilidad de partes, `ceil(part_count / 8)` bytes.
    pub part_status: bytes::Bytes,
    /// Conteo de fuentes completas (solo cuando la versión de peticiones extendidas > 1).
    pub complete_sources: Option<u16>,
}

/// Referencia de fichero transportada en un envoltorio `MULTIPACKET`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ed2kMultiPacketFileRef {
    /// `OP_MULTIPACKET`/`OP_MULTIPACKETANSWER` heredado: hash MD4 simple de 16 bytes.
    Hash([u8; 16]),
    /// `OP_MULTIPACKET_EXT`: hash + tamaño de fichero de 64 bits.
    HashAndSize {
        /// Hash de fichero MD4 de 16 bytes.
        file_hash: [u8; 16],
        /// Tamaño del fichero en bytes.
        file_size: u64,
    },
    /// `OP_MULTIPACKET_EXT2`/`OP_MULTIPACKETANSWER_EXT2`: `FileIdentifier`.
    Identifier(super::file_identifier::Ed2kFileIdentifier),
}

impl Ed2kMultiPacketFileRef {
    /// Devuelve el hash de fichero MD4 de 16 bytes con independencia de la variante de envoltorio.
    /// Para `Identifier` sin MD4 (contexto exclusivo de Kad), devuelve ceros.
    pub fn file_hash(&self) -> [u8; 16] {
        match self {
            Self::Hash(h) => *h,
            Self::HashAndSize { file_hash, .. } => *file_hash,
            Self::Identifier(id) => id.md4_hash.unwrap_or([0u8; 16]),
        }
    }
}

/// Subpetición dentro del cuerpo de una petición `MULTIPACKET`
/// (`ListenSocket.cpp` 0.72a, caso `OP_MULTIPACKET*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ed2kMultiPacketSubRequest {
    /// `OP_REQFILENAME` (`0x58`) + información extendida opcional.
    FileName {
        /// Información extendida (estado de partes + fuentes completas), cuando está presente.
        ext_info: Option<Ed2kRequestExtInfo>,
    },
    /// `OP_SETREQFILEID` (`0x4F`), sin carga útil.
    SetReqFileId,
    /// `OP_REQUESTSOURCES` (`0x81`), sin carga útil.
    RequestSources,
    /// `OP_REQUESTSOURCES2` (`0x83`): `<version u8><options u16>`.
    RequestSources2 {
        /// Versión solicitada del protocolo de intercambio de fuentes.
        version: u8,
        /// Opciones reservadas (eMule envía 0).
        options: u16,
    },
    /// `OP_AICHFILEHASHREQ` (`0x9E`), sin carga útil — obsoleta en origen cuando
    /// se utiliza `FileIdentifier`.
    AichFileHash,
}

/// Subrespuesta dentro del cuerpo de un `MULTIPACKETANSWER`. A diferencia de los mensajes
/// independientes, las subrespuestas **no** repiten el hash del fichero — este reside en el
/// envoltorio (`ListenSocket.cpp` 0.72a, caso `OP_MULTIPACKETANSWER*`).
#[derive(Debug, Clone, PartialEq)]
pub enum Ed2kMultiPacketSubAnswer {
    /// `OP_REQFILENAMEANSWER` (`0x59`): `<len u16><bytes del nombre de fichero>`.
    FileName(String),
    /// `OP_FILESTATUS` (`0x50`): `<u16 part_count><mapa de bits ceil(count/8)>`.
    FileStatus {
        /// Número de partes eD2K descritas por el mapa de bits.
        part_count: u16,
        /// Bytes del mapa de bits de disponibilidad de partes.
        part_status: bytes::Bytes,
    },
    /// `OP_AICHFILEHASHANS` (`0x9D`): hash raíz AICH simple de 20 bytes.
    AichFileHash([u8; 20]),
}

/// Petición agrupada de ficheros (`OP_MULTIPACKET` `0x92`, `OP_MULTIPACKET_EXT` `0xA4`,
/// `OP_MULTIPACKET_EXT2` `0xA9`, protocolo eMule).
#[derive(Debug, Clone, PartialEq)]
pub struct Ed2kMultiPacketRequest {
    /// Referencia de fichero (hash, hash+tamaño o `FileIdentifier` completo).
    pub file: Ed2kMultiPacketFileRef,
    /// Subpeticiones ordenadas tras la referencia.
    pub sub_requests: Vec<Ed2kMultiPacketSubRequest>,
}

/// Respuesta a petición agrupada de ficheros (`OP_MULTIPACKETANSWER` `0x93`,
/// `OP_MULTIPACKETANSWER_EXT2` `0xB0`, protocolo eMule).
#[derive(Debug, Clone, PartialEq)]
pub struct Ed2kMultiPacketAnswer {
    /// Referencia de fichero (hash o `FileIdentifier` — EXT no tiene variante de respuesta).
    pub file: Ed2kMultiPacketFileRef,
    /// Subrespuestas ordenadas tras la referencia.
    pub sub_answers: Vec<Ed2kMultiPacketSubAnswer>,
}

/// Enumeración de mensajes de observación pasiva eD2K de alto nivel.
#[derive(Debug, Clone, PartialEq)]
pub enum Ed2kMessage {
    /// Inicialización del saludo.
    Hello(Ed2kHello),
    /// Respuesta al saludo.
    HelloAnswer(Ed2kHelloAnswer),
    /// Intercambio de capacidades del saludo extendido de eMule.
    EmuleInfo(Ed2kEmuleInfo),
    /// Petición de disponibilidad de partes de fichero.
    FileStatusReq(Ed2kFileStatusReq),
    /// Respuesta de estado de disponibilidad de partes de fichero.
    FileStatus(Ed2kFileStatus),
    /// Respuesta que indica que el fichero solicitado no está compartido o es desconocido.
    FileReqAnsNoFile(Ed2kFileReqAnsNoFile),
    /// Petición de datos de bloques.
    RequestParts(Ed2kRequestParts),
    /// Petición de datos de bloques con desplazamientos de 64 bits (`OP_REQUESTPARTS_I64`).
    RequestPartsI64(Ed2kRequestPartsI64),
    /// Entrega de datos de bloques.
    SendingPart(Ed2kSendingPart),
    /// Entrega de datos de bloques con desplazamientos de 64 bits (`OP_SENDINGPART_I64`).
    SendingPartI64(Ed2kSendingPartI64),
    /// Entrega de datos de bloques comprimidos con zlib con desplazamiento de 64 bits (`OP_COMPRESSEDPART_I64`).
    CompressedPartI64(Ed2kCompressedPartI64),
    /// Entrega de datos de bloques comprimidos con zlib con desplazamiento de 32 bits (`OP_COMPRESSEDPART`).
    CompressedPart(Ed2kCompressedPart),
    /// Petición del conjunto de hashes (hashset) por parte.
    HashSetReq(Ed2kHashSetReq),
    /// Respuesta con el conjunto de hashes (hashset) por parte.
    HashSetAnswer(Ed2kHashSetAnswer),
    /// Petición de hash maestro AICH (eMule `0x9E`).
    AichFileHashReq(Ed2kAichFileHashReq),
    /// Respuesta de hash maestro AICH (eMule `0x9D`).
    AichFileHashRes(Ed2kAichFileHashRes),
    /// Petición de datos de recuperación de partes AICH (eMule `0x9B`).
    AichRequest(Ed2kAichRequest),
    /// Respuesta de datos de recuperación de partes AICH (eMule `0x9C`).
    AichAnswer(Ed2kAichAnswer),
    /// Clave pública de identificación segura de usuario.
    PublicKey(Ed2kPublicKey),
    /// Firma de identificación segura de usuario.
    Signature(Ed2kSignature),
    /// Estado/desafío de identificación segura de usuario.
    SecIdentState(Ed2kSecIdentState),
    /// Mensaje de chat entre pares (`OP_MESSAGE`).
    Message(Ed2kChatMessage),
    /// Posición en cola heredada (`OP_QUEUERANK`).
    QueueRank(Ed2kQueueRank),
    /// Posición en cola de eMule (`OP_QUEUERANKING`).
    QueueRanking(Ed2kQueueRanking),
    /// Cambio de slot de subida (`OP_CHANGE_SLOT`).
    ChangeSlot(Ed2kChangeSlot),
    /// Valoración/comentario de fichero (`OP_FILEDESC`).
    FileDesc(Ed2kFileDesc),
    /// Prueba de puerto TCP (`OP_PORTTEST`).
    PortTest(Ed2kPortTest),
    /// Petición de CAPTCHA de chat (`OP_CHATCAPTCHAREQ`).
    CaptchaRequest(Ed2kCaptchaRequest),
    /// Respuesta de CAPTCHA de chat (`OP_CHATCAPTCHARES`).
    CaptchaResponse(Ed2kCaptchaResponse),
    /// Petición de conjunto de hashes basada en `FileIdentifier` (`OP_HASHSETREQUEST2`).
    HashSetReq2(Ed2kHashSetReq2),
    /// Respuesta de conjunto de hashes basada en `FileIdentifier` (`OP_HASHSETANSWER2`).
    HashSetAnswer2(Ed2kHashSetAnswer2),
    /// Petición agrupada de ficheros (familia `OP_MULTIPACKET*`).
    MultiPacketRequest(Ed2kMultiPacketRequest),
    /// Respuesta a petición agrupada de ficheros (familia `OP_MULTIPACKETANSWER*`).
    MultiPacketAnswer(Ed2kMultiPacketAnswer),
    /// Paquete sin procesar (utilizado cuando el código de operación aún no dispone de un decodificador dedicado).
    Raw(RawPacket),
}

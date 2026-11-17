// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Análisis seguro y acotado para etiquetas históricas Tipo-Longitud-Valor (TLV) de eD2K.

use super::error::{Ed2kErrorKind, Ed2kParseError};
use super::limits::Ed2kLimits;

/// Tipos de datos de etiquetas.
pub const TAGTYPE_HASH: u8 = 0x01;
/// Tipo de etiqueta de cadena (longitud de 2 bytes + bytes UTF-8/ASCII).
pub const TAGTYPE_STRING: u8 = 0x02;
/// Tipo de etiqueta de entero sin signo de 32 bits.
pub const TAGTYPE_UINT32: u8 = 0x03;
/// Tipo de etiqueta de punto flotante de 32 bits.
pub const TAGTYPE_FLOAT32: u8 = 0x04;
/// Tipo de etiqueta booleana (1 byte, histórico 0x05).
pub const TAGTYPE_BOOL: u8 = 0x05;
/// Tipo de etiqueta de array booleano (1 byte, histórico 0x06).
pub const TAGTYPE_BOOLARRAY: u8 = 0x06;
/// Tipo de etiqueta de blob de bytes sin procesar (longitud de 4 bytes + bytes).
pub const TAGTYPE_BLOB: u8 = 0x07;
/// Tipo de etiqueta de entero sin signo de 16 bits.
pub const TAGTYPE_UINT16: u8 = 0x08;
/// Tipo de etiqueta de entero sin signo de 8 bits.
pub const TAGTYPE_UINT8: u8 = 0x09;
/// Tipo de etiqueta de blob del tamaño de un byte (longitud de 1 byte + bytes).
pub const TAGTYPE_BSOB: u8 = 0x0A;
/// Tipo de etiqueta de entero sin signo de 64 bits.
pub const TAGTYPE_UINT64: u8 = 0x0B;
/// Base del tipo de etiqueta de cadena de longitud fija (TAGTYPE_STR1 = 0x11, longitud = tag_type - 0x10).
pub const TAGTYPE_STR1: u8 = 0x11;

/// Identificadores de etiquetas especiales estándar (cuando la longitud del nombre de etiqueta es 1).
pub const TAG_SPECIAL_NAME: u8 = 0x01;
/// Identificador de etiqueta especial estándar de tamaño de fichero.
pub const TAG_SPECIAL_FILESIZE: u8 = 0x02;
/// Identificador de etiqueta especial estándar de tipo de fichero.
pub const TAG_SPECIAL_FILETYPE: u8 = 0x03;
/// 32 bits superiores del tamaño de fichero para ficheros grandes (`FT_FILESIZE_HI`, `0x3A`).
pub const TAG_SPECIAL_FILESIZE_HI: u8 = 0x3A;
/// Identificador de etiqueta especial estándar de puerto (`0x0F` = 15).
pub const TAG_SPECIAL_PORT: u8 = 0x0F;
/// Identificador de etiqueta especial estándar de versión (`0x11` = 17).
pub const TAG_SPECIAL_VERSION: u8 = 0x11;
/// Identificador de etiqueta especial canónico de indicadores de servidor eD2K / eMule (`0x20` = 32, CT_SERVER_FLAGS).
pub const TAG_SPECIAL_SERVER_FLAGS: u8 = 0x20;
/// Identificador de etiqueta especial estándar de versión de eMule (`0x5A` = 90, SO_EMULEVERSION).
pub const TAG_SPECIAL_EMULE_VERSION: u8 = 0x5A;
/// Identificador de etiqueta especial estándar de opciones misceláneas 1 (histórico 0x5B).
pub const TAG_SPECIAL_MISCOPTIONS1: u8 = 0x5B;
/// Identificador de etiqueta especial estándar de opciones misceláneas 2 (histórico 0x5C).
pub const TAG_SPECIAL_MISCOPTIONS2: u8 = 0x5C;
/// Identificador de etiqueta canónico de puertos UDP de eMule (`0xF9` = 249).
pub const TAG_SPECIAL_EMULE_UDPPORTS: u8 = 0xF9;
/// Identificador de etiqueta canónico de opciones misceláneas 1 de eMule (`0xFA` = 250).
pub const TAG_SPECIAL_EMULE_MISCOPTIONS1: u8 = 0xFA;
/// Identificador de etiqueta canónico de versión de eMule para inicio de sesión en servidor (`0xFB` = 251, CT_EMULE_VERSION).
pub const TAG_SPECIAL_CT_EMULE_VERSION: u8 = 0xFB;
/// Identificador de etiqueta canónico de opciones / indicadores UDP de eMule (`0xFB` = 251).
pub const TAG_SPECIAL_EMULE_UDPOPTIONS: u8 = 0xFB;
/// Identificador de etiqueta especial estándar de opciones / puerto UDP (alias de 0xFB).
pub const TAG_SPECIAL_UDPOPTIONS: u8 = 0xFB;
/// Identificador de etiqueta canónico de opciones misceláneas 2 de eMule (`0xFE` = 254).
pub const TAG_SPECIAL_EMULE_MISCOPTIONS2: u8 = 0xFE;
/// Identificador de etiqueta canónico de extensión de compresión de eMule (`0x20` = 32).
pub const TAG_SPECIAL_EMULE_EXT_COMPRESSION: u8 = 0x20;
/// Identificador de etiqueta canónico de extensión de versión UDP de eMule (`0x21` = 33).
pub const TAG_SPECIAL_EMULE_EXT_UDPVER: u8 = 0x21;
/// Identificador de etiqueta canónico de extensión de puerto UDP de eMule (`0x22` = 34).
pub const TAG_SPECIAL_EMULE_EXT_UDPPORT: u8 = 0x22;
/// Identificador de etiqueta canónico de extensión de intercambio de fuentes de eMule (`0x23` = 35).
pub const TAG_SPECIAL_EMULE_EXT_SOURCEEXCHANGE: u8 = 0x23;
/// Identificador de etiqueta canónico de extensión de comentarios de eMule (`0x24` = 36).
pub const TAG_SPECIAL_EMULE_EXT_COMMENTS: u8 = 0x24;
/// Identificador de etiqueta canónico de extensión de peticiones extendidas de eMule (`0x25` = 37).
pub const TAG_SPECIAL_EMULE_EXT_EXTENDEDREQUEST: u8 = 0x25;
/// Identificador de etiqueta canónico de extensión de cliente compatible de eMule (`0x26` = 38, `ET_COMPATIBLECLIENT`).
pub const TAG_SPECIAL_EMULE_EXT_COMPATIBLECLIENT: u8 = 0x26;
/// Identificador de etiqueta canónico de extensión de indicadores de características de eMule (`0x27` = 39, `ET_FEATURES`).
pub const TAG_SPECIAL_EMULE_EXT_FEATURES: u8 = 0x27;

/// Indicador de capacidad del servidor: soporte de compresión TCP con Zlib.
pub const SRVCAP_ZLIB: u32 = 0x0001;
/// Indicador de capacidad del servidor: el cliente envía la IP en el inicio de sesión.
pub const SRVCAP_IP_IN_LOGIN: u32 = 0x0002;
/// Indicador de capacidad del servidor: soporte de puerto auxiliar.
pub const SRVCAP_AUXPORT: u32 = 0x0004;
/// Indicador de capacidad del servidor: soporte de formato moderno de etiquetas.
pub const SRVCAP_NEWTAGS: u32 = 0x0008;
/// Indicador de capacidad del servidor: soporte de caracteres Unicode.
pub const SRVCAP_UNICODE: u32 = 0x0010;
/// Indicador de capacidad del servidor: soporte de ficheros mayores de 4 GiB.
pub const SRVCAP_LARGEFILES: u32 = 0x0100;
/// Indicador de capacidad del servidor: soporte de ofuscación criptográfica.
pub const SRVCAP_SUPPORTCRYPT: u32 = 0x0200;
/// Indicador de capacidad del servidor: ofuscación criptográfica solicitada.
pub const SRVCAP_REQUESTCRYPT: u32 = 0x0400;
/// Indicador de capacidad del servidor: ofuscación criptográfica requerida.
pub const SRVCAP_REQUIRECRYPT: u32 = 0x0800;

/// Identificador de nombre de etiqueta: o bien un ID especial de 1 byte o una cadena de texto.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TagName {
    /// Identificador de etiqueta estandarizado especial de 1 byte.
    Id(u8),
    /// Cadena de texto con el nombre de la etiqueta.
    Text(String),
}

impl TagName {
    /// Devuelve una cadena legible para visualización de este nombre de etiqueta.
    pub fn display_name(&self) -> String {
        match self {
            Self::Id(id) => match *id {
                TAG_SPECIAL_NAME => "name".to_string(),
                TAG_SPECIAL_FILESIZE => "filesize".to_string(),
                TAG_SPECIAL_FILETYPE => "filetype".to_string(),
                TAG_SPECIAL_PORT => "port".to_string(),
                TAG_SPECIAL_VERSION => "version".to_string(),
                TAG_SPECIAL_EMULE_VERSION => "emule_version".to_string(),
                TAG_SPECIAL_EMULE_UDPPORTS => "emule_udp_ports".to_string(),
                TAG_SPECIAL_MISCOPTIONS1 | TAG_SPECIAL_EMULE_MISCOPTIONS1 => {
                    "misc_options_1".to_string()
                }
                TAG_SPECIAL_MISCOPTIONS2 | TAG_SPECIAL_EMULE_MISCOPTIONS2 => {
                    "misc_options_2".to_string()
                }
                TAG_SPECIAL_UDPOPTIONS => "udp_options".to_string(),
                TAG_SPECIAL_EMULE_EXT_COMPRESSION => "compression".to_string(),
                TAG_SPECIAL_EMULE_EXT_UDPVER => "emule_udp_ver".to_string(),
                TAG_SPECIAL_EMULE_EXT_UDPPORT => "emule_udp_port".to_string(),
                TAG_SPECIAL_EMULE_EXT_SOURCEEXCHANGE => "source_exchange".to_string(),
                TAG_SPECIAL_EMULE_EXT_COMMENTS => "comments".to_string(),
                TAG_SPECIAL_EMULE_EXT_EXTENDEDREQUEST => "extended_request".to_string(),
                TAG_SPECIAL_EMULE_EXT_COMPATIBLECLIENT => "compatible_client".to_string(),
                TAG_SPECIAL_EMULE_EXT_FEATURES => "features".to_string(),
                other => format!("special_0x{other:02X}"),
            },
            Self::Text(s) => s.clone(),
        }
    }
}

/// Valor tipado analizado de una etiqueta eD2K.
#[derive(Debug, Clone, PartialEq)]
pub enum TagValue {
    /// Hash MD4 de 16 bytes.
    Hash([u8; 16]),
    /// Cadena UTF-8 o decodificada.
    String(String),
    /// Entero sin signo de 32 bits.
    UInt32(u32),
    /// Punto flotante de 32 bits.
    Float32(f32),
    /// Indicador booleano.
    Bool(bool),
    /// Blob de búfer de bytes.
    Blob(Vec<u8>),
    /// Entero sin signo de 16 bits.
    UInt16(u16),
    /// Entero sin signo de 8 bits.
    UInt8(u8),
    /// Entero sin signo de 64 bits.
    UInt64(u64),
}

/// Una etiqueta eD2K totalmente tipada.
#[derive(Debug, Clone, PartialEq)]
pub struct Ed2kTag {
    /// Identificador o nombre de la etiqueta.
    pub name: TagName,
    /// Valor de carga útil de la etiqueta tipado.
    pub value: TagValue,
}

/// Representación sin procesar de una etiqueta desconocida o no reconocida para garantizar que no se pierdan datos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTag {
    /// Byte de tipo sin procesar.
    pub tag_type: u8,
    /// Identificador o nombre de la etiqueta.
    pub name: TagName,
    /// Bytes de carga útil sin procesar no analizados.
    pub raw_bytes: Vec<u8>,
}

/// Busca una etiqueta reconocida en `tags` por su nombre de visualización (p. ej. `"Nick"`).
///
/// Implementación única compartida por los accesores `find_tag` de `Ed2kHello`, `Ed2kHelloAnswer`,
/// `Ed2kEmuleInfo` y `Ed2kPeerInfo`.
pub fn find_tag_by_name<'a>(tags: &'a [Ed2kTag], display_name: &str) -> Option<&'a Ed2kTag> {
    tags.iter().find(|t| t.name.display_name() == display_name)
}

/// Extrae la carga útil de cadena de una etiqueta especial (con identificador numérico) como
/// `TAG_SPECIAL_NAME`; `None` cuando está ausente o cuando la carga útil no es una cadena.
pub fn special_string_tag(tags: &[Ed2kTag], tag_id: u8) -> Option<String> {
    tags.iter().find_map(|t| match (&t.name, &t.value) {
        (TagName::Id(id), TagValue::String(s)) if *id == tag_id => Some(s.clone()),
        _ => None,
    })
}

/// Lector auxiliar para la extracción segura de etiquetas de un fragmento de bytes.
pub struct TagReader<'a> {
    buf: &'a [u8],
    pos: usize,
    protocol: u8,
    opcode: Option<u8>,
}

impl<'a> TagReader<'a> {
    /// Crea un nuevo `TagReader` comenzando en `offset`.
    pub fn new(buf: &'a [u8], offset: usize, protocol: u8, opcode: Option<u8>) -> Self {
        Self {
            buf,
            pos: offset,
            protocol,
            opcode,
        }
    }

    /// Devuelve el desplazamiento de posición de lectura actual.
    #[inline]
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Devuelve el byte de protocolo para el que se creó este lector.
    #[inline]
    pub fn protocol(&self) -> u8 {
        self.protocol
    }

    /// Devuelve el contexto del código de operación para el que se creó este lector, si existe.
    #[inline]
    pub fn opcode(&self) -> Option<u8> {
        self.opcode
    }

    /// Devuelve el número de bytes restantes.
    #[inline]
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    /// Lee exactamente `n` bytes del búfer.
    pub fn read_bytes(
        &mut self,
        n: usize,
        context: &'static str,
    ) -> Result<&'a [u8], Ed2kParseError> {
        let available = self.remaining();
        if available < n {
            return Err(Ed2kParseError::new(
                self.protocol,
                self.opcode,
                self.pos,
                context,
                Ed2kErrorKind::UnexpectedEof {
                    needed: n,
                    available,
                },
            ));
        }
        let start = self.pos;
        self.pos += n;
        Ok(&self.buf[start..self.pos])
    }

    /// Echa un vistazo al siguiente byte sin avanzar el lector.
    pub fn peek_u8(&self) -> Option<u8> {
        self.buf.get(self.pos).copied()
    }

    /// Lee un solo byte.
    pub fn read_u8(&mut self, context: &'static str) -> Result<u8, Ed2kParseError> {
        let bytes = self.read_bytes(1, context)?;
        Ok(bytes[0])
    }

    /// Lee exactamente `N` bytes en un array de tamaño fijo.
    fn read_arr<const N: usize>(&mut self, context: &'static str) -> Result<[u8; N], Ed2kParseError> {
        let bytes = self.read_bytes(N, context)?;
        let mut arr = [0u8; N];
        arr.copy_from_slice(bytes);
        Ok(arr)
    }

    /// Lee un entero sin signo de 16 bits en little-endian.
    pub fn read_u16_le(&mut self, context: &'static str) -> Result<u16, Ed2kParseError> {
        Ok(u16::from_le_bytes(self.read_arr(context)?))
    }

    /// Lee un entero sin signo de 32 bits en little-endian.
    pub fn read_u32_le(&mut self, context: &'static str) -> Result<u32, Ed2kParseError> {
        Ok(u32::from_le_bytes(self.read_arr(context)?))
    }

    /// Lee un entero sin signo de 64 bits en little-endian.
    pub fn read_u64_le(&mut self, context: &'static str) -> Result<u64, Ed2kParseError> {
        Ok(u64::from_le_bytes(self.read_arr(context)?))
    }

    /// Lee un flotante de 32 bits en little-endian.
    pub fn read_f32_le(&mut self, context: &'static str) -> Result<f32, Ed2kParseError> {
        Ok(f32::from_le_bytes(self.read_arr(context)?))
    }

    /// Lee un hash MD4 de 16 bytes.
    pub fn read_hash(&mut self, context: &'static str) -> Result<[u8; 16], Ed2kParseError> {
        self.read_arr(context)
    }

    /// Lee una cadena prefijada por su longitud (longitud LE de 16 bits + bytes UTF-8/con pérdidas), aplicando `max_string_bytes`.
    pub fn read_string(
        &mut self,
        max_string_bytes: usize,
        context: &'static str,
    ) -> Result<String, Ed2kParseError> {
        let len = self.read_u16_le(context)? as usize;
        if len > max_string_bytes {
            return Err(Ed2kParseError::new(
                self.protocol,
                self.opcode,
                self.pos - 2,
                context,
                Ed2kErrorKind::OversizedString {
                    size: len,
                    limit: max_string_bytes,
                },
            ));
        }
        let raw = self.read_bytes(len, context)?;
        Ok(String::from_utf8_lossy(raw).into_owned())
    }

    /// Lee un nombre de etiqueta (longitud 1 = ID especial, longitud > 1 = cadena).
    pub fn read_tag_name(&mut self, max_string_bytes: usize) -> Result<TagName, Ed2kParseError> {
        let name_len = self.read_u16_le("tag_name_len")? as usize;
        if name_len == 1 {
            let id = self.read_u8("tag_special_id")?;
            Ok(TagName::Id(id))
        } else {
            if name_len > max_string_bytes {
                return Err(Ed2kParseError::new(
                    self.protocol,
                    self.opcode,
                    self.pos - 2,
                    "tag_name",
                    Ed2kErrorKind::OversizedString {
                        size: name_len,
                        limit: max_string_bytes,
                    },
                ));
            }
            let bytes = self.read_bytes(name_len, "tag_name_bytes")?;
            let s = String::from_utf8_lossy(bytes).into_owned();
            Ok(TagName::Text(s))
        }
    }

    /// Analiza una sola etiqueta del flujo de entrada.
    ///
    /// Si el tipo de etiqueta es desconocido, devuelve `Ok((None, Some(raw_tag)))`.
    /// Si es reconocido, devuelve `Ok((Some(ed2k_tag), None))`.
    pub fn read_tag(
        &mut self,
        limits: &Ed2kLimits,
    ) -> Result<(Option<Ed2kTag>, Option<RawTag>), Ed2kParseError> {
        let raw_type = self.read_u8("tag_type")?;
        let (tag_type, name) = if raw_type & 0x80 != 0 {
            let tag_type = raw_type & 0x7F;
            let id = self.read_u8("tag_special_id")?;
            (tag_type, TagName::Id(id))
        } else {
            let name = self.read_tag_name(limits.max_string_bytes)?;
            (raw_type, name)
        };

        match tag_type {
            TAGTYPE_HASH => {
                let hash = self.read_hash("tag_value_hash")?;
                Ok((
                    Some(Ed2kTag {
                        name,
                        value: TagValue::Hash(hash),
                    }),
                    None,
                ))
            }
            TAGTYPE_STRING => {
                let s = self.read_string(limits.max_string_bytes, "tag_value_string")?;
                Ok((
                    Some(Ed2kTag {
                        name,
                        value: TagValue::String(s),
                    }),
                    None,
                ))
            }
            TAGTYPE_UINT32 => {
                let v = self.read_u32_le("tag_value_u32")?;
                Ok((
                    Some(Ed2kTag {
                        name,
                        value: TagValue::UInt32(v),
                    }),
                    None,
                ))
            }
            TAGTYPE_FLOAT32 => {
                let v = self.read_f32_le("tag_value_f32")?;
                Ok((
                    Some(Ed2kTag {
                        name,
                        value: TagValue::Float32(v),
                    }),
                    None,
                ))
            }
            TAGTYPE_BOOL | TAGTYPE_BOOLARRAY => {
                let b = self.read_u8("tag_value_bool")? != 0;
                Ok((
                    Some(Ed2kTag {
                        name,
                        value: TagValue::Bool(b),
                    }),
                    None,
                ))
            }
            TAGTYPE_BLOB => {
                let blob_len = self.read_u32_le("tag_value_blob_len")? as usize;
                if blob_len > limits.max_blob_bytes {
                    return Err(Ed2kParseError::new(
                        self.protocol,
                        self.opcode,
                        self.pos - 4,
                        "tag_value_blob",
                        Ed2kErrorKind::FrameTooLarge {
                            size: blob_len,
                            limit: limits.max_blob_bytes,
                        },
                    ));
                }
                let blob_bytes = self.read_bytes(blob_len, "tag_value_blob_bytes")?;
                Ok((
                    Some(Ed2kTag {
                        name,
                        value: TagValue::Blob(blob_bytes.to_vec()),
                    }),
                    None,
                ))
            }
            TAGTYPE_BSOB => {
                let blob_len = self.read_u8("tag_value_bsob_len")? as usize;
                let blob_bytes = self.read_bytes(blob_len, "tag_value_bsob_bytes")?;
                Ok((
                    Some(Ed2kTag {
                        name,
                        value: TagValue::Blob(blob_bytes.to_vec()),
                    }),
                    None,
                ))
            }
            TAGTYPE_UINT16 => {
                let v = self.read_u16_le("tag_value_u16")?;
                Ok((
                    Some(Ed2kTag {
                        name,
                        value: TagValue::UInt16(v),
                    }),
                    None,
                ))
            }
            TAGTYPE_UINT8 => {
                let v = self.read_u8("tag_value_u8")?;
                Ok((
                    Some(Ed2kTag {
                        name,
                        value: TagValue::UInt8(v),
                    }),
                    None,
                ))
            }
            TAGTYPE_UINT64 => {
                let v = self.read_u64_le("tag_value_u64")?;
                Ok((
                    Some(Ed2kTag {
                        name,
                        value: TagValue::UInt64(v),
                    }),
                    None,
                ))
            }
            t if (0x11..=0x7F).contains(&t) => {
                let len = (t - 0x10) as usize;
                let bytes = self.read_bytes(len, "tag_value_str_fixed")?;
                let s = String::from_utf8_lossy(bytes).into_owned();
                Ok((
                    Some(Ed2kTag {
                        name,
                        value: TagValue::String(s),
                    }),
                    None,
                ))
            }
            unknown => {
                // Devuelve como un RawTag si la longitud no se puede resolver o si no es válido
                Err(Ed2kParseError::new(
                    self.protocol,
                    self.opcode,
                    self.pos - 1,
                    "tag_type",
                    Ed2kErrorKind::InvalidTagType { tag_type: unknown },
                ))
            }
        }
    }
}

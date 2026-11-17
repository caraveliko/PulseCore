// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Tipos fundamentales y estructuras para la comunicación con servidores eD2K.

use crate::ed2k::encoder::Ed2kEncoder;
use crate::ed2k::error::{Ed2kEncodeError, Ed2kErrorKind};
use crate::ed2k::limits::Ed2kLimits;
use crate::ed2k::opcodes::{OP_CALLBACKREQUEST, OP_LOGINREQUEST, OP_OFFERFILES};
use crate::ed2k::protocol::PR_ED2K;
use crate::ed2k::tags::{
    Ed2kTag, RawTag, TagName, TagValue, SRVCAP_LARGEFILES, SRVCAP_NEWTAGS, SRVCAP_UNICODE,
    SRVCAP_ZLIB, TAG_SPECIAL_CT_EMULE_VERSION, TAG_SPECIAL_FILESIZE, TAG_SPECIAL_FILESIZE_HI,
    TAG_SPECIAL_FILETYPE, TAG_SPECIAL_NAME, TAG_SPECIAL_SERVER_FLAGS, TAG_SPECIAL_VERSION,
};

/// Versión de eMule anunciada en la etiqueta `CT_EMULE_VERSION` del inicio de sesión en el servidor —
/// codificación de eMule `(mjr << 17) | (min << 10) | (upd << 7)`; `0.70a` aquí,
/// coherente con la identidad anunciada por el protocolo de enlace del cliente.
// Fórmula de cable de eMule conservada tal cual (identity_op intencionado).
#[allow(clippy::identity_op)]
pub const ANNOUNCED_EMULE_VERSION_TAG: u32 = (0 << 17) | (70 << 10) | (0 << 7);
use crate::framing::RawPacket;
use bytes::Bytes;

/// Petición de inicio de sesión de cliente enviada a un servidor eD2K (`OP_LOGINREQUEST` = `0x01` bajo `PR_ED2K`).
#[derive(Debug, Clone, PartialEq)]
pub struct ServerLoginRequest {
    /// Hash de usuario MD4 de 16 bytes.
    pub user_hash: [u8; 16],
    /// ID de cliente actual (normalmente 0 antes de que el servidor asigne uno).
    pub client_id: u32,
    /// Puerto TCP de escucha local.
    pub tcp_port: u16,
    /// Etiquetas del cliente (p. ej., apodo, versión, indicadores).
    pub tags: Vec<Ed2kTag>,
}

impl ServerLoginRequest {
    /// Crea una petición de inicio de sesión con el conjunto exacto de etiquetas `OP_LOGINREQUEST` de eMule
    /// (`ServerConnect.cpp:175-202`): `CT_NAME`, `CT_VERSION`,
    /// `CT_SERVER_FLAGS` (`ZLIB | NEWTAGS | LARGEFILES | UNICODE`) y
    /// `CT_EMULE_VERSION` (identidad `0.70a` anunciada). `version` es el
    /// valor de la etiqueta `CT_VERSION` (eMule pasa `EDONKEYVERSION` = `0x3C`).
    pub fn new(user_hash: [u8; 16], tcp_port: u16, nickname: &str, version: u32) -> Self {
        let tags = vec![
            Ed2kTag {
                name: TagName::Id(TAG_SPECIAL_NAME),
                value: TagValue::String(nickname.to_string()),
            },
            Ed2kTag {
                name: TagName::Id(TAG_SPECIAL_VERSION),
                value: TagValue::UInt32(version),
            },
            Ed2kTag {
                name: TagName::Id(TAG_SPECIAL_SERVER_FLAGS),
                value: TagValue::UInt32(
                    SRVCAP_ZLIB | SRVCAP_NEWTAGS | SRVCAP_UNICODE | SRVCAP_LARGEFILES,
                ),
            },
            Ed2kTag {
                name: TagName::Id(TAG_SPECIAL_CT_EMULE_VERSION),
                value: TagValue::UInt32(ANNOUNCED_EMULE_VERSION_TAG),
            },
        ];
        Self {
            user_hash,
            client_id: 0,
            tcp_port,
            tags,
        }
    }

    /// Añade indicadores `SRVCAP_*` a la etiqueta `CT_SERVER_FLAGS` — eMule aplica
    /// OR con `SUPPORTCRYPT`/`REQUESTCRYPT`/`REQUIRECRYPT` según las preferencias
    /// de ofuscación (`ServerConnect.cpp:184-191`).
    #[must_use]
    pub fn with_server_flags(mut self, extra_flags: u32) -> Self {
        for tag in &mut self.tags {
            if tag.name == TagName::Id(TAG_SPECIAL_SERVER_FLAGS) {
                if let TagValue::UInt32(v) = &mut tag.value {
                    *v |= extra_flags;
                }
            }
        }
        self
    }

    /// Codifica la petición de inicio de sesión en un `RawPacket`.
    pub fn encode(&self, limits: &Ed2kLimits) -> Result<RawPacket, Ed2kEncodeError> {
        let encoder = Ed2kEncoder::with_limits(limits.clone());
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.user_hash);
        buf.extend_from_slice(&self.client_id.to_le_bytes());
        buf.extend_from_slice(&self.tcp_port.to_le_bytes());
        buf.extend_from_slice(&(self.tags.len() as u32).to_le_bytes());

        for tag in &self.tags {
            encoder.encode_tag(OP_LOGINREQUEST, tag, &mut buf)?;
        }

        let payload_len = buf.len() + 1;
        if payload_len > limits.max_frame_bytes {
            return Err(Ed2kEncodeError::new(
                OP_LOGINREQUEST,
                "login_request_payload",
                Ed2kErrorKind::FrameTooLarge {
                    size: payload_len,
                    limit: limits.max_frame_bytes,
                },
            ));
        }

        Ok(RawPacket::with_vec(PR_ED2K, OP_LOGINREQUEST, buf))
    }
}

/// Mensaje de servidor que contiene saludos del servidor o MOTD (`OP_SERVERMESSAGE` = `0x38`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerMessage {
    /// Cadena de mensaje enviada por el servidor.
    pub message: String,
}

/// Respuesta de cambio de ID del servidor (`OP_IDCHANGE` = `0x40`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerIdChange {
    /// ID de cliente asignado.
    /// Si es `>= 16777216`, al cliente se le asigna HighID.
    /// Si es `< 16777216`, el cliente está detrás de NAT/cortafuegos y se le asigna LowID.
    pub client_id: u32,
    /// Indicadores/capacidades TCP del servidor (histórico).
    pub tcp_flags: u32,
}

impl ServerIdChange {
    /// Devuelve verdadero si el ID de cliente califica como HighID (`>= 16_777_216`).
    #[inline]
    pub fn is_high_id(&self) -> bool {
        self.client_id >= 16_777_216
    }
}

/// Informe de estado del servidor (`OP_SERVERSTATUS` = `0x14`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerStatus {
    /// Total de usuarios activos conectados al servidor.
    pub current_users: u32,
    /// Total de archivos actualmente indexados por el servidor.
    pub current_files: u32,
}

/// Respuesta de identificación del servidor (`OP_SERVERIDENT` = `0x41`).
#[derive(Debug, Clone, PartialEq)]
pub struct ServerIdent {
    /// Hash de servidor de 16 bytes.
    pub server_hash: [u8; 16],
    /// Dirección IPv4 del servidor en orden de red.
    pub ip: u32,
    /// Puerto de escucha del servidor.
    pub port: u16,
    /// Etiquetas del servidor (nombre, descripción, etc.).
    pub tags: Vec<Ed2kTag>,
    /// Etiquetas desconocidas o sin procesar.
    pub unknown_tags: Vec<RawTag>,
}

impl ServerIdent {
    /// Auxiliar para buscar el nombre del servidor en las etiquetas.
    pub fn name(&self) -> Option<String> {
        crate::ed2k::tags::special_string_tag(&self.tags, TAG_SPECIAL_NAME)
    }

    /// eMule `ST_TCPPORTOBFUSCATION` (`0x97`, uint16) — puerto TCP dedicado
    /// que acepta conexiones de servidor ofuscadas (DH768+RC4).
    pub fn obfuscation_tcp_port(&self) -> Option<u16> {
        self.server_tag_u16(0x97)
    }

    /// eMule `ST_UDPPORTOBFUSCATION` (`0x98`, uint16) — puerto UDP dedicado
    /// que acepta datagramas de servidor ofuscados.
    pub fn obfuscation_udp_port(&self) -> Option<u16> {
        self.server_tag_u16(0x98)
    }

    /// eMule `ST_SOFTFILES` (`0x88`, uint32) — límite flexible de archivos que un cliente
    /// puede publicar mediante `OP_OFFERFILES` (`CServer::GetSoftFiles`).
    pub fn soft_files(&self) -> Option<u32> {
        self.server_tag_u32(0x88)
    }

    /// eMule `ST_HARDFILES` (`0x89`, uint32) — límite estricto de archivos indexados
    /// por cliente (`CServer::GetHardFiles`).
    pub fn hard_files(&self) -> Option<u32> {
        self.server_tag_u32(0x89)
    }

    fn server_tag_u32(&self, tag_id: u8) -> Option<u32> {
        for t in &self.tags {
            if t.name == TagName::Id(tag_id) {
                match t.value {
                    TagValue::UInt32(v) => return Some(v),
                    TagValue::UInt16(v) => return Some(u32::from(v)),
                    _ => {}
                }
            }
        }
        self.unknown_tags.iter().find_map(|t| {
            if t.name == TagName::Id(tag_id) && t.raw_bytes.len() == 4 {
                Some(u32::from_le_bytes([
                    t.raw_bytes[0],
                    t.raw_bytes[1],
                    t.raw_bytes[2],
                    t.raw_bytes[3],
                ]))
            } else {
                None
            }
        })
    }

    fn server_tag_u16(&self, tag_id: u8) -> Option<u16> {
        for t in &self.tags {
            if t.name == TagName::Id(tag_id) {
                match t.value {
                    TagValue::UInt16(v) => return Some(v),
                    TagValue::UInt32(v) => return u16::try_from(v).ok(),
                    _ => {}
                }
            }
        }
        // Recurre a etiquetas sin procesar (decodificación tipada no reconocida): una etiqueta uint16 consta de
        // 2 bytes en little-endian.
        self.unknown_tags.iter().find_map(|t| {
            if t.name == TagName::Id(tag_id) && t.raw_bytes.len() == 2 {
                Some(u16::from_le_bytes([t.raw_bytes[0], t.raw_bytes[1]]))
            } else {
                None
            }
        })
    }
}

/// eMule `SRV_TCPFLG_COMPRESSION`: el servidor acepta los marcadores de oferta
/// 0xFBFB/0xFCFC y paquetes de servidor comprimidos con zlib.
pub const SRV_TCPFLG_COMPRESSION: u32 = 0x0000_0001;
/// eMule `SRV_TCPFLG_LARGEFILES`: el servidor admite entradas de archivos >4 GiB.
pub const SRV_TCPFLG_LARGEFILES: u32 = 0x0000_0100;

/// ID de cliente marcador enviado en las entradas de `OP_OFFERFILES` cuando el servidor admite
/// compresión TCP: el archivo está completo.
pub const OFFER_COMPLETE_CLIENT_ID: u32 = 0xFBFBFBFB;
/// Puerto de cliente marcador emparejado con `OFFER_COMPLETE_CLIENT_ID`.
pub const OFFER_COMPLETE_CLIENT_PORT: u16 = 0xFBFB;
/// ID de cliente marcador para un archivo incompleto (archivo de partes .part).
pub const OFFER_INCOMPLETE_CLIENT_ID: u32 = 0xFCFCFCFC;
/// Puerto de cliente marcador emparejado con `OFFER_INCOMPLETE_CLIENT_ID`.
pub const OFFER_INCOMPLETE_CLIENT_PORT: u16 = 0xFCFC;

/// Mapea un nombre de archivo a su término de búsqueda `FT_FILETYPE` de eD2K
/// (eMule `GetED2KFileTypeSearchTerm`): `"Audio"`, `"Video"`, `"Image"`,
/// `"Doc"`, `"Pro"` — o `""` (no enviar etiqueta de tipo).
pub fn ed2k_file_type_term(filename: &str) -> &'static str {
    let ext = filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "aac" | "aif" | "aiff" | "au" | "flac" | "mid" | "midi" | "mp3" | "m4a" | "ogg" | "ogm"
        | "ra" | "ram" | "wav" | "wma" | "ape" | "mka" | "opus" => "Audio",
        "3gp" | "asf" | "avi" | "divx" | "flv" | "m1v" | "m2v" | "mkv" | "mov" | "mp1" | "mp2"
        | "mp4" | "mpe" | "mpeg" | "mpg" | "mpv" | "mpv2" | "ogv" | "qt" | "rm" | "rmvb" | "rv"
        | "ts" | "vcd" | "vob" | "webm" | "wmv" => "Video",
        "bmp" | "gif" | "jpeg" | "jpg" | "png" | "psd" | "tif" | "tiff" | "webp" => "Image",
        "doc" | "docx" | "htm" | "html" | "nfo" | "odt" | "pdf" | "pps" | "ppt" | "pptx"
        | "rtf" | "txt" | "xls" | "xlsx" | "epub" | "mobi" => "Doc",
        // eMule publica archivos comprimidos e imágenes de CD como "Pro" para su indexación.
        "7z" | "ace" | "alz" | "arj" | "bz2" | "cab" | "gz" | "lha" | "rar" | "tar" | "tgz"
        | "zip" | "zoo" | "bin" | "bwi" | "ccd" | "cue" | "img" | "iso" | "mds" | "nrg" | "sub"
        | "exe" | "msi" | "apk" | "dmg" | "com" | "bat" | "dll" => "Pro",
        _ => "",
    }
}

/// Una entrada de archivo en un paquete `OP_OFFERFILES`
/// (eMule `CSharedFileList::CreateOfferedFilePacket` para servidores).
#[derive(Debug, Clone, PartialEq)]
pub struct OfferedFile {
    /// Hash MD4 de archivo de 16 bytes.
    pub file_hash: [u8; 16],
    /// Campo ID de cliente: nuestro HighID, 0 cuando es LowID, o los marcadores
    /// `0xFBFBFBFB` / `0xFCFCFCFC` cuando el servidor admite compresión TCP.
    pub client_id: u32,
    /// Puerto TCP de escucha (o marcador `0xFBFB`/`0xFCFC`).
    pub client_port: u16,
    /// Etiquetas de metadatos (`FT_FILENAME`, `FT_FILESIZE`[, `FT_FILESIZE_HI`],
    /// `FT_FILETYPE`).
    pub tags: Vec<Ed2kTag>,
}

impl OfferedFile {
    /// Construye una entrada de archivo ofrecido para un archivo compartido siguiendo las reglas
    /// de publicación de servidor de eMule (`CreateOfferedFilePacket`): cuando el servidor
    /// admite `SRV_TCPFLG_COMPRESSION`, los archivos incompletos (archivos de partes) llevan
    /// el par marcador `0xFCFCFCFC`/`0xFCFC` y los archivos completos el par
    /// `0xFBFBFBFB`/`0xFBFB`; de lo contrario, un HighID publica su ID+puerto reales
    /// (`0` cuando es LowID). Los archivos grandes reciben el par dividido
    /// `FT_FILESIZE`/`FT_FILESIZE_HI` solo cuando el servidor los admite.
    pub fn for_server(
        file_hash: [u8; 16],
        file_name: &str,
        file_size: u64,
        our_client_id: u32,
        our_tcp_port: u16,
        server_tcp_flags: u32,
        is_partial: bool,
    ) -> Option<Self> {
        const LARGE_FILE: u64 = 0xFFFF_FFFF;
        let is_large = file_size > LARGE_FILE;
        if is_large && (server_tcp_flags & SRV_TCPFLG_LARGEFILES) == 0 {
            // eMule omite archivos grandes cuando el servidor carece de soporte TCP para archivos grandes.
            return None;
        }

        let (client_id, client_port) = if server_tcp_flags & SRV_TCPFLG_COMPRESSION != 0 {
            if is_partial {
                (OFFER_INCOMPLETE_CLIENT_ID, OFFER_INCOMPLETE_CLIENT_PORT)
            } else {
                (OFFER_COMPLETE_CLIENT_ID, OFFER_COMPLETE_CLIENT_PORT)
            }
        } else if our_client_id >= 16_777_216 {
            // HighID: publicar nuestro ID de cliente + puerto TCP reales.
            (our_client_id, our_tcp_port)
        } else {
            (0, 0)
        };

        let mut tags = vec![Ed2kTag {
            name: TagName::Id(TAG_SPECIAL_NAME),
            value: TagValue::String(file_name.to_string()),
        }];
        if is_large {
            tags.push(Ed2kTag {
                name: TagName::Id(TAG_SPECIAL_FILESIZE),
                value: TagValue::UInt32(file_size as u32),
            });
            tags.push(Ed2kTag {
                name: TagName::Id(TAG_SPECIAL_FILESIZE_HI),
                value: TagValue::UInt32((file_size >> 32) as u32),
            });
        } else {
            tags.push(Ed2kTag {
                name: TagName::Id(TAG_SPECIAL_FILESIZE),
                value: TagValue::UInt32(file_size as u32),
            });
        }
        let ft = ed2k_file_type_term(file_name);
        if !ft.is_empty() {
            tags.push(Ed2kTag {
                name: TagName::Id(TAG_SPECIAL_FILETYPE),
                value: TagValue::String(ft.to_string()),
            });
        }
        Some(Self {
            file_hash,
            client_id,
            client_port,
            tags,
        })
    }
}

/// Paquete `OP_OFFERFILES` (`0x15`): `u32 count` + por archivo
/// `hash[16]` + `u32 client_id` + `u16 client_port` + lista de etiquetas.
/// Una lista `files` vacía sirve a la vez como ping keep-alive al servidor.
#[derive(Debug, Clone, PartialEq)]
pub struct OfferFiles {
    /// Archivos a publicar (máx. ~200 según el límite flexible de archivos de eMule).
    pub files: Vec<OfferedFile>,
}

impl OfferFiles {
    /// Codifica el paquete en un `RawPacket` (PR_ED2K / `OP_OFFERFILES`).
    pub fn encode(&self, limits: &Ed2kLimits) -> Result<RawPacket, Ed2kEncodeError> {
        let encoder = Ed2kEncoder::with_limits(limits.clone());
        let mut buf = Vec::new();
        buf.extend_from_slice(&(self.files.len() as u32).to_le_bytes());
        for f in &self.files {
            buf.extend_from_slice(&f.file_hash);
            buf.extend_from_slice(&f.client_id.to_le_bytes());
            buf.extend_from_slice(&f.client_port.to_le_bytes());
            buf.extend_from_slice(&(f.tags.len() as u32).to_le_bytes());
            for tag in &f.tags {
                encoder.encode_tag(OP_OFFERFILES, tag, &mut buf)?;
            }
        }
        let payload_len = buf.len() + 1;
        if payload_len > limits.max_frame_bytes {
            return Err(Ed2kEncodeError::new(
                OP_OFFERFILES,
                "offer_files_payload",
                Ed2kErrorKind::FrameTooLarge {
                    size: payload_len,
                    limit: limits.max_frame_bytes,
                },
            ));
        }
        Ok(RawPacket::with_vec(PR_ED2K, OP_OFFERFILES, buf))
    }
}

/// `OP_CALLBACKREQUEST` (`0x1C`, PR_ED2K): solicita al servidor que retransmita una
/// petición de conexión a un cliente LowID — el destino se conecta entonces a nosotros
/// (eMule `CDownloadQueue::...` / mecanismo de devolución de llamada del servidor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackRequest {
    /// ID de cliente LowID del par al que llamar (< 16.777.216).
    pub client_id: u32,
}

impl CallbackRequest {
    /// Codifica la petición en un `RawPacket`: carga útil de `u32 client_id` simple.
    pub fn encode(&self) -> RawPacket {
        RawPacket::with_vec(
            PR_ED2K,
            OP_CALLBACKREQUEST,
            self.client_id.to_le_bytes().to_vec(),
        )
    }
}

/// Notificación de devolución de llamada retransmitida por el servidor `OP_CALLBACKREQUESTED` (`0x35`):
/// `<IP 4><PORT 2>` de un cliente que solicitó al servidor una llamada hacia nosotros,
/// opcionalmente seguido de `u8 crypt_options` + `user_hash[16]` (23+ bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackRequested {
    /// IPv4 del solicitante (little-endian como en el cable).
    pub ip: u32,
    /// Puerto TCP del solicitante.
    pub port: u16,
    /// Byte de opciones de cifrado/ofuscación, si está presente.
    pub crypt_options: Option<u8>,
    /// Hash de usuario del solicitante, si está presente.
    pub user_hash: Option<[u8; 16]>,
}

/// Mensaje decodificado de servidor a cliente.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerResponse {
    /// Notificación del servidor / cadena MOTD.
    Message(ServerMessage),
    /// ClientID asignado / HighID o LowID.
    IdChange(ServerIdChange),
    /// Estadísticas actuales de usuarios y archivos.
    Status(ServerStatus),
    /// Identificación y metadatos del servidor.
    Ident(ServerIdent),
    /// Resultados de búsqueda devueltos por el servidor.
    SearchResult(super::search::SearchResult),
    /// Fuentes devueltas por el servidor para un hash de archivo solicitado.
    FoundSources(super::sources::FoundSources),
    /// Fuentes devueltas por el servidor con metadatos de ofuscación (OP_FOUNDSOURCES_OBFU).
    FoundSourcesObfu(super::sources::FoundSourcesObfu),
    /// Un par solicitó al servidor que nos retransmitiera una devolución de llamada (`OP_CALLBACKREQUESTED`).
    CallbackRequested(CallbackRequested),
    /// El servidor rechazó el último comando (`OP_REJECT`, carga útil vacía).
    ///
    /// eMule `ServerSocket.cpp` trata esto como diagnóstico de nivel debug y
    /// mantiene el socket; normalmente el servidor lo cierra justo después.
    Rejected,
    /// El servidor no pudo retransmitir nuestra petición de devolución de llamada (`OP_CALLBACK_FAIL`).
    CallbackFail {
        /// ID de cliente de destino al que no se pudo llamar, cuando se devuelve como eco.
        client_id: Option<u32>,
    },
    /// Servidores adicionales anunciados por el servidor conectado
    /// (`OP_SERVERLIST`): entradas `(ip_le, port)`.
    ServerList {
        /// Servidores anunciados (IPv4 little-endian + puerto TCP).
        servers: Vec<(u32, u16)>,
    },
    /// Otro paquete sin analizar.
    Other {
        /// Código de operación del paquete sin analizar.
        opcode: u8,
        /// Bytes de carga útil sin procesar.
        payload: Bytes,
    },
}

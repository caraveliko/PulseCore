// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Tipos de red y constantes para paquetes UDP de Kad.

use crate::ed2k::tags::{Ed2kTag, RawTag, TagName, TagValue};

/// Alias de ID de Kad de 128 bits (16 bytes).
pub type KadIdBytes = [u8; 16];

/// Máximo de contactos en una respuesta Kad2 Bootstrap o Find Node (estándar K = 20, acotado a 50).
pub const MAX_KAD_RES_CONTACTS: usize = 50;

/// Máximo de fuentes devueltas en una respuesta Kad2 Search Source
/// (eMule `iMaxResults = 300`, `Indexed.cpp:707`).
pub const MAX_KAD_SEARCH_SOURCES: usize = 300;

/// eMule `UDP_KAD_MAXFRAGMENT` (`Opcodes.h:134`) — tamaño máximo de datagrama UDP de Kad
/// antes de que eMule divida una respuesta de múltiples entradas en varios paquetes
/// (límite conservador de 1500 de MTU dejando espacio para cabeceras IP/UDP/túnel/Kad).
pub const UDP_KAD_MAXFRAGMENT: usize = 1420;

/// Representación en cable de un contacto Kad en respuestas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadContactWire {
    /// ID de Kad de 128 bits.
    pub kad_id: KadIdBytes,
    /// Dirección IPv4 como u32 en orden de red.
    pub ip: [u8; 4],
    /// Puerto UDP.
    pub udp_port: u16,
    /// Puerto TCP (opcional, 0 si no está establecido).
    pub tcp_port: u16,
    /// Versión de Kad.
    pub version: u8,
}

/// Paquete Ping de Kad2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadPing {
    /// ID de Kad del emisor/receptor.
    pub kad_id: KadIdBytes,
    /// Puerto UDP.
    pub udp_port: u16,
}

/// Paquete Pong de Kad2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadPong {
    /// Puerto visto por el par remoto.
    pub client_port: u16,
}

/// Paquete de petición Hello de Kad2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadHelloReq {
    /// ID de Kad del emisor.
    pub kad_id: KadIdBytes,
    /// Puerto UDP del emisor anunciado mediante `TAG_SOURCEUPORT` (0 = usar origen del paquete).
    pub udp_port: u16,
    /// Versión de Kad.
    pub version: u8,
    /// Puerto TCP del emisor (0 si no tiene).
    pub tcp_port: u16,
    /// `TAG_KADMISCOPTIONS` byte if present (Kad >= 8): bit0 UDP firewalled,
    /// bit1 TCP firewalled, bit2 = requesting `KADEMLIA2_HELLO_RES_ACK`.
    pub misc_options: Option<u8>,
}

impl KadHelloReq {
    /// Verdadero si el emisor solicitó que respondamos con `KADEMLIA2_HELLO_RES_ACK`.
    pub fn requests_ack(&self) -> bool {
        self.misc_options.is_some_and(|o| o & 0x04 != 0)
    }
}

/// Paquete de respuesta Hello de Kad2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadHelloRes {
    /// ID de Kad del respondedor.
    pub kad_id: KadIdBytes,
    /// Puerto UDP del respondedor anunciado mediante `TAG_SOURCEUPORT` (0 = origen del paquete).
    pub udp_port: u16,
    /// Versión de Kad.
    pub version: u8,
    /// Puerto TCP del respondedor (0 si no tiene).
    pub tcp_port: u16,
    /// Byte `TAG_KADMISCOPTIONS` si está presente (Kad >= 8).
    pub misc_options: Option<u8>,
}

impl KadHelloRes {
    /// Verdadero si el respondedor solicitó que enviemos `KADEMLIA2_HELLO_RES_ACK`.
    pub fn requests_ack(&self) -> bool {
        self.misc_options.is_some_and(|o| o & 0x04 != 0)
    }
}

/// Reconocimiento de respuesta Hello de Kad2 (`KADEMLIA2_HELLO_RES_ACK`, 0x22).
///
/// Enviado tras un `KADEMLIA2_HELLO_RES` cuyo `TAG_KADMISCOPTIONS` lo solicita —
/// completa el apretón de manos de tres vías demostrando que nuestra IP no está falseada
/// (eMule `CKademliaUDPListener::Process_KADEMLIA2_HELLO_RES_ACK`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadHelloResAck {
    /// ID de Kad del emisor.
    pub kad_id: KadIdBytes,
}

/// Petición heredada de comprobación de cortafuegos TCP de Kad (`KADEMLIA_FIREWALLED_REQ`, 0x50):
/// el emisor solicita que nos conectemos por TCP a `tcp_port` para comprobar su alcanzabilidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadFirewalledReq {
    /// Puerto TCP que el emisor solicita probar.
    pub tcp_port: u16,
}

/// Petición v2 de comprobación de cortafuegos TCP de Kad (`KADEMLIA_FIREWALLED2_REQ`, 0x53):
/// `[tcp_port u16][user_hash 16][connect_options u8]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadFirewalledReq2 {
    /// Puerto TCP que el emisor solicita probar.
    pub tcp_port: u16,
    /// User hash eD2K del emisor.
    pub user_hash: [u8; 16],
    /// Byte de opciones de conexión del emisor (`GetMyConnectOptions`).
    pub connect_options: u8,
}

/// Respuesta de comprobación de cortafuegos de Kad (`KADEMLIA_FIREWALLED_RES`, 0x58): devuelve la
/// dirección IPv4 del solicitante tal como la ve el respondedor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadFirewalledRes {
    /// Dirección IPv4 del solicitante (little-endian en el cable).
    pub ip: u32,
}

/// Paquete de petición Bootstrap de Kad2 (0x21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadBootstrapReq {
    /// ID de Kad del emisor.
    pub kad_id: KadIdBytes,
    /// Puerto UDP del emisor.
    pub udp_port: u16,
}

/// Paquete de respuesta Bootstrap / Find Node de Kad2 (0x29).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadBootstrapRes {
    /// ID de Kad del respondedor.
    pub kad_id: KadIdBytes,
    /// Puerto UDP del respondedor.
    pub udp_port: u16,
    /// Versión de Kad.
    pub version: u8,
    /// Lista de contactos devueltos (acotada a MAX_KAD_RES_CONTACTS).
    pub contacts: Vec<KadContactWire>,
}

/// Paquete de petición de Kad2 (Find Node / Search, 0x21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadReq {
    /// Tipo de petición (ej. 0 para FIND_NODE).
    pub req_type: u8,
    /// ID de Kad de destino buscado.
    pub target_id: KadIdBytes,
    /// ID de Kad del emisor.
    pub sender_id: KadIdBytes,
}

/// Representación en cable de una fuente en una respuesta Kad2 Search Source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadSourceWire {
    /// ID de Kad/Cliente de 128 bits.
    pub kad_id: KadIdBytes,
    /// Dirección IPv4 como bytes en orden de red.
    pub ip: [u8; 4],
    /// Puerto UDP.
    pub udp_port: u16,
    /// Puerto TCP (0 si no tiene — fuente Kad-LowID alcanzable solo mediante buddy).
    pub tcp_port: u16,
    /// Flags de cifrado/capacidad.
    pub encryption: u8,
    /// Buddy Kad ID advertised via `TAG_BUDDYHASH`/`FT_BUDDYHASH` (0xF8) —
    /// presente en fuentes LowID alcanzables únicamente mediante un contacto
    /// buddy callback (`KADEMLIA_CALLBACK_REQ`).
    pub buddy_id: Option<KadIdBytes>,
}

/// Petición Kad2 Search Source (0x34).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadSearchSourceReq {
    /// Hash del archivo de destino (16 bytes).
    pub target_file_hash: KadIdBytes,
    /// ID de Kad del emisor (16 bytes).
    pub sender_id: KadIdBytes,
    /// Posición inicial para paginación.
    pub start_pos: u16,
    /// Tamaño del archivo en bytes.
    pub file_size: u64,
}

/// Respuesta Kad2 Search Source (0x3B, respuesta unificada de búsqueda).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadSearchSourceRes {
    /// Hash del archivo de destino (16 bytes).
    pub target_file_hash: KadIdBytes,
    /// Fuentes descubiertas (acotadas a MAX_KAD_SEARCH_SOURCES).
    pub sources: Vec<KadSourceWire>,
}

/// Petición Kad2 Publish Source (0x44).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadPublishSourceReq {
    /// Hash del archivo de destino (16 bytes).
    pub target_file_hash: KadIdBytes,
    /// Contacto que anuncia el archivo.
    pub contact: KadContactWire,
    /// Tamaño del archivo en bytes.
    pub file_size: u64,
    /// Publisher's connect-options byte (`TAG_ENCRYPTION` 0xF3, eMule
    /// `GetMyConnectOptions`): crypt bits 0-2, direct-UDP-callback bit 3.
    /// 0 cuando el publicador no incluyó la etiqueta.
    pub encryption: u8,
}

/// Respuesta Kad2 Publish Source (0x4B, respuesta unificada de publicación).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadPublishSourceRes {
    /// Hash del archivo de destino (16 bytes).
    pub target_file_hash: KadIdBytes,
    /// Código de estado del resultado (0 = éxito).
    pub status: u8,
    /// Trailing options byte bit 0 (`KademliaUDPListener.cpp:1438`) — the
    /// indexer requests a `KADEMLIA2_PUBLISH_RES_ACK` datagram in return.
    /// "For future use": no released client sets it today.
    pub request_ack: bool,
}

/// Kad version that introduced AICH root hashes in keyword storage —
/// eMule `KADEMLIA_VERSION9_50a` (`Opcodes.h`).
pub const KADEMLIA_VERSION_AICH: u8 = 9;

/// Current Kad protocol version — eMule `KADEMLIA_VERSION` (`Opcodes.h`),
/// advertised by eMule 0.72a.
pub const KADEMLIA_VERSION_CURRENT: u8 = 10;

/// `TAG_KADAICHHASHPUB` (`\x36`) — BSOB carrying the publisher's 20-byte
/// AICH root hash inside a `PUBLISH_KEY_REQ` keyword entry. Introduced by
/// Kad v9 (`KADEMLIA_VERSION9_50a`, eMule `Opcodes.h`).
pub const TAG_KADAICHHASH_PUB: u8 = 0x36;

/// `TAG_KADAICHHASHRESULT` (`\x37`) — BSOB carrying the AICH popularity
/// histogram synthesized by the storing node inside `SEARCH_RES` keyword
/// results (eMule `CKeyEntry::WriteTagListWithPublishInfo`).
pub const TAG_KADAICHHASH_RESULT: u8 = 0x37;

/// `TAG_PUBLISHINFO` (`\x33`) — u32 `<names u8><publishers u8><trust×100 u16>`
/// appended by the storing node to keyword search results.
pub const TAG_PUBLISHINFO: u8 = 0x33;

/// 20-byte AICH root hash (SHA-1) as published via `TAG_KADAICHHASHPUB`.
pub const AICH_ROOT_HASH_SIZE: usize = 20;

/// Maximum byte size of the `TAG_KADAICHHASHRESULT` BSOB written by eMule
/// (`CKeyEntry::WriteTagListWithPublishInfo` writes into a 255-byte buffer
/// y reserva un byte para el prefijo de recuento).
pub const MAX_AICH_RESULT_BSOB: usize = 255;

/// Maximum AICH hashes written into one `TAG_KADAICHHASHRESULT` BSOB —
/// eMule detiene el conteo cuando el siguiente par `<u8 pop><20B hash>` superaría
/// the payload past 250 bytes (`WriteTagListWithPublishInfo`), yielding at
/// most 11 entries (`1 + 21*11 = 232`).
pub const MAX_AICH_RESULT_VOTES: usize = 11;

/// One AICH root hash and its publisher popularity inside
/// `TAG_KADAICHHASHRESULT` (`<u8 count><u8 pop><20B hash>…`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AichHashVote {
    /// Number of distinct publishers claiming this root hash.
    pub popularity: u8,
    /// 20-byte AICH root hash.
    pub root: [u8; AICH_ROOT_HASH_SIZE],
}

/// Wire representation of a keyword search entry.
///
/// Not `Eq` because `tags` may contain `Float32` values.
#[derive(Debug, Clone, PartialEq)]
pub struct KadKeywordEntryWire {
    /// 128-bit file hash.
    pub file_hash: KadIdBytes,
    /// File name (UTF-8).
    pub file_name: String,
    /// Tamaño del archivo en bytes.
    pub file_size: u64,
    /// Number of sources advertising this file.
    pub source_count: u32,
    /// Published metadata tag list retained verbatim from `PUBLISH_KEY_REQ`
    /// or the `SEARCH_RES` tag list (eMule `CKeyEntry::m_listTag`). Used to
    /// evaluate restrictive search expressions
    /// (`CKeyEntry::SearchTermsMatch`). Vacío para entradas decodificadas desde
    /// legacy flat format.
    pub tags: Vec<Ed2kTag>,
    /// `TAG_KADAICHHASHPUB` (0x36) decoded from a `PUBLISH_KEY_REQ` entry —
    /// el hash raíz AICH que el publicador declara para este archivo (Kad v9+).
    /// Never set on entries received via `SEARCH_RES` (eMule ignores/strips
    /// the pub tag in results); the encoder emits it only for our own
    /// publish requests.
    pub aich_pub: Option<[u8; AICH_ROOT_HASH_SIZE]>,
    /// `TAG_KADAICHHASHRESULT` (0x37) histogram decoded from a `SEARCH_RES`
    /// entrada, o sintetizado por el nodo de almacenamiento al servir resultados.
    /// Entries with `popularity == 0` are filtered on decode (eMule
    /// `ProcessResultKeyword`).
    pub aich_votes: Vec<AichHashVote>,
    /// `TAG_PUBLISHINFO` (0x33) raw u32 decoded from a `SEARCH_RES` entry,
    /// o sintetizado por el nodo de almacenamiento al servir resultados.
    pub publish_info: Option<u32>,
}

/// Kad2 Search Keyword Request (0x33).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadSearchKeywordReq {
    /// 128-bit MD4 hash of the keyword.
    pub keyword_hash: KadIdBytes,
    /// `start_pos & 0x8000` on the wire — the requester appended a
    /// restrictive search-expression tree (`CreateSearchExpressionTree`).
    pub restrictive: bool,
    /// Start position for pagination (`start_pos & 0x7FFF` on the wire).
    pub start_pos: u16,
    /// Bytes en bruto de la expresión de búsqueda tras la petición cuando `restrictive`
    /// is set. Bounded by the frame limit. Parsed via
    /// `crate::kad::search_expr::KadSearchExpression` and applied by the
    /// daemon before pagination (eMule `StartSearchTermsMatch`).
    pub search_expression: Option<Vec<u8>>,
}

/// Kad2 Search Keyword Response (0x3B, unified search response, or legacy 0x2C).
///
/// Not Eq: entries carry metadata tags which may be floats.
#[derive(Debug, Clone, PartialEq)]
pub struct KadSearchKeywordRes {
    /// 128-bit MD4 hash of the keyword.
    pub keyword_hash: KadIdBytes,
    /// Discovered file entries.
    pub entries: Vec<KadKeywordEntryWire>,
}

/// Kad2 Publish Keyword Request (0x43).
///
/// Not Eq: entries carry metadata tags which may be floats.
#[derive(Debug, Clone, PartialEq)]
pub struct KadPublishKeywordReq {
    /// 128-bit MD4 hash of the keyword.
    pub keyword_hash: KadIdBytes,
    /// File entries published for this keyword.
    pub entries: Vec<KadKeywordEntryWire>,
}

/// Kad2 Firewall UDP test packet (0x62).
/// Sent by a Kad contact in response to a TCP `OP_FWCHECKUDPREQ`, to probe
/// whether the requester's Kad UDP port is reachable (eMule
/// `Process_KADEMLIA2_FIREWALLUDP` / `ProcessFirewallCheckUDPRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadFirewallUdp {
    /// Error/already-known flag (0 = clean result, non-zero = remote reported
    /// de que el resultado puede estar sesgado y la prueba debe descartarse).
    pub error_code: u8,
    /// Puerto UDP sondeado (puerto Kad interno o externo del solicitante).
    pub udp_port: u16,
}

/// Kad2 Publish Keyword Response (0x4B, unified publish response).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadPublishKeywordRes {
    /// 128-bit MD4 hash of the keyword.
    pub keyword_hash: KadIdBytes,
    /// Status code (0 = success).
    pub status: u8,
}

/// Kad buddy search request (0x51) — a firewalled client asks us to act as
/// its buddy (`Process_KADEMLIA_FINDBUDDY_REQ`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadFindBuddyReq {
    /// ID de amigo bajo el cual el solicitante desea registrarse (su destino de contacto).
    pub buddy_id: KadIdBytes,
    /// Requester's Kad client ID.
    pub user_id: KadIdBytes,
    /// Puerto TCP de escucha del solicitante (de donde procederá el enlace de amigo).
    pub tcp_port: u16,
}

/// Kad buddy search response (0x5A) — acceptance of a buddy request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadFindBuddyRes {
    /// Echoed buddy ID.
    pub buddy_id: KadIdBytes,
    /// Responder's Kad client ID.
    pub client_id: KadIdBytes,
    /// Responder's TCP listening port (buddy connects there).
    pub tcp_port: u16,
    /// Connection option flags (post-0.49a senders; 0 when absent).
    pub connect_options: u8,
}

/// Petición de retransmisión de callback buddy de Kad (0x52) — un solicitante HighID nos pide
/// retransmitir un callback a nuestro cliente buddy (`Process_KADEMLIA_CALLBACK_REQ`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KadCallbackReq {
    /// Campo de comprobación de ID de buddy (reenviado textualmente al cliente buddy).
    pub buddy_id: KadIdBytes,
    /// Hash de archivo que el solicitante pide al cliente buddy.
    pub file_id: KadIdBytes,
    /// Puerto TCP del solicitante al que el buddy debe conectar de vuelta.
    pub tcp_port: u16,
}

// ---------------------------------------------------------------------------
// Notas Kad (comentarios / valoraciones de archivos) — eMule `CEntry` con `m_bSource=false`.
//
// Las notas son datos suministrados por terceros, solo para visualización: eMule trata
// explícitamente las cadenas de etiquetas de notas como el único lugar donde se permite contenido no-UTF8
// «solo para visualización». Por tanto, mantenemos la lista completa de etiquetas textual
// (`Ed2kTag` tipado + `RawTag` para tipos desconocidos) y exponemos métodos de acceso cómodos
// en vez de una estructura cerrada que descartaría metadatos desconocidos.
// ---------------------------------------------------------------------------

/// `TAG_FILENAME` (`\x01`) — etiqueta de nombre de archivo dentro de una entrada de nota.
pub const TAG_NOTE_FILENAME: u8 = 0x01;
/// `TAG_FILESIZE` (`\x02`) — etiqueta de tamaño de archivo dentro de una entrada de nota.
pub const TAG_NOTE_FILESIZE: u8 = 0x02;
/// `TAG_DESCRIPTION` (`\x0B`) — texto de comentario de usuario de una entrada de nota.
pub const TAG_NOTE_DESCRIPTION: u8 = 0x0B;
/// `TAG_FILERATING` (`\xF7`) — valoración de usuario 0..5 de una entrada de nota.
pub const TAG_NOTE_FILERATING: u8 = 0xF7;

/// Límite máximo de etiquetas aceptadas en una entrada/petición de notas (límite antiabuso).
pub const MAX_NOTE_TAG_COUNT: usize = 32;

/// Máximo de entradas de notas aceptadas en una respuesta de búsqueda
/// (eMule `SendValidNoteResult` limita a 150 por paquete).
pub const MAX_NOTE_SEARCH_RESULTS: usize = 150;

/// Una entrada de nota individual (`CEntry`) transportada en resultados de notas de
/// `KADEMLIA2_SEARCH_RES` (0x3B) y `KADEMLIA2_PUBLISH_NOTES_REQ` (0x45).
#[derive(Debug, Clone, PartialEq)]
pub struct KadNoteWire {
    /// Publisher's Kad ID (`m_uSourceID`) — the "answer hash" in search
    /// results, the `source` field in publish requests.
    pub source_id: KadIdBytes,
    /// Etiquetas tipadas (filename, filesize, description, rating, …).
    pub tags: Vec<Ed2kTag>,
    /// Etiquetas no reconocidas conservadas en bruto (sin pérdida de datos).
    pub unknown_tags: Vec<RawTag>,
}

impl KadNoteWire {
    fn note_tag(&self, id: u8) -> Option<&TagValue> {
        self.tags.iter().find_map(|t| {
            if t.name == TagName::Id(id) {
                Some(&t.value)
            } else {
                None
            }
        })
    }

    /// Cadena `TAG_FILENAME`, si está presente.
    pub fn file_name(&self) -> Option<&str> {
        match self.note_tag(TAG_NOTE_FILENAME) {
            Some(TagValue::String(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Valor `TAG_FILESIZE`, si está presente.
    pub fn file_size(&self) -> Option<u64> {
        match self.note_tag(TAG_NOTE_FILESIZE) {
            Some(TagValue::UInt8(v)) => Some(*v as u64),
            Some(TagValue::UInt16(v)) => Some(*v as u64),
            Some(TagValue::UInt32(v)) => Some(*v as u64),
            Some(TagValue::UInt64(v)) => Some(*v),
            _ => None,
        }
    }

    /// `TAG_DESCRIPTION` — comentario del usuario. Texto no confiable solo para visualización.
    pub fn comment(&self) -> Option<&str> {
        match self.note_tag(TAG_NOTE_DESCRIPTION) {
            Some(TagValue::String(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// `TAG_FILERATING` — valoración 0..5, si está presente.
    pub fn rating(&self) -> Option<u8> {
        match self.note_tag(TAG_NOTE_FILERATING) {
            Some(TagValue::UInt8(v)) => Some(*v),
            Some(TagValue::UInt16(v)) => Some(*v as u8),
            Some(TagValue::UInt32(v)) => Some(*v as u8),
            _ => None,
        }
    }
}

/// Petición Kad2 Search Notes (`KADEMLIA2_SEARCH_NOTES_REQ`, 0x35):
/// `[target_file_hash:16][file_size:u64]` — eMule
/// `Process_KADEMLIA2_SEARCH_NOTES_REQ`.
#[derive(Debug, Clone, PartialEq)]
pub struct KadSearchNotesReq {
    /// Hash del archivo de destino (16 bytes).
    pub target_file_hash: KadIdBytes,
    /// Tamaño de archivo utilizado para filtrar entradas de notas (0 = aceptar cualquiera).
    pub file_size: u64,
}

/// Respuesta de búsqueda de notas. En cable es la unificada
/// `KADEMLIA2_SEARCH_RES` (0x3B) — `[sender_id:16][target:16][count:u16]
/// ([source_id:16][tag_count:u8][tags])*` — ver eMule
/// `CIndexed::SendValidNoteResult`. El opcode heredado Kad1
/// `KADEMLIA_SEARCH_NOTES_RES` (0x3A) transporta las mismas entradas sin el
/// ID del emisor.
#[derive(Debug, Clone, PartialEq)]
pub struct KadSearchNotesRes {
    /// ID de Kad del respondedor (cero para el formato heredado 0x3A).
    pub sender_id: KadIdBytes,
    /// Hash del archivo de destino (16 bytes).
    pub target_file_hash: KadIdBytes,
    /// Entradas de notas (acotadas).
    pub notes: Vec<KadNoteWire>,
}

/// Petición Kad2 Publish Notes (`KADEMLIA2_PUBLISH_NOTES_REQ`, 0x45):
/// `[target_file_hash:16][source_id:16][tag_count:u8][tags]` — eMule
/// `Process_KADEMLIA2_PUBLISH_NOTES_REQ`.
#[derive(Debug, Clone, PartialEq)]
pub struct KadPublishNotesReq {
    /// Hash del archivo de destino (16 bytes).
    pub target_file_hash: KadIdBytes,
    /// ID de Kad del publicador (`m_uSourceID`).
    pub source_id: KadIdBytes,
    /// Etiquetas de notas (tipadas).
    pub tags: Vec<Ed2kTag>,
    /// Unrecognized tags preserved raw.
    pub unknown_tags: Vec<RawTag>,
}

/// Respuesta Kad2 Publish Notes (`KADEMLIA2_PUBLISH_RES`, 0x4B):
/// `[target_file_hash:16][load:u8]` — misma forma de cable que respuestas de publicación
/// de fuente/palabra clave; el byte representa la carga del indexador.
#[derive(Debug, Clone, PartialEq)]
pub struct KadPublishNotesRes {
    /// Hash del archivo de destino (16 bytes).
    pub target_file_hash: KadIdBytes,
    /// Informe de carga del indexador (eMule `uLoad`).
    pub load: u8,
}

/// Enum que representa mensajes UDP decodificados de Kad.
#[derive(Debug, Clone, PartialEq)]
pub enum KadMessage {
    /// Petición Ping de Kad2.
    Ping(KadPing),
    /// Respuesta Pong de Kad2.
    Pong(KadPong),
    /// Petición Hello de Kad2.
    HelloReq(KadHelloReq),
    /// Respuesta Hello de Kad2.
    HelloRes(KadHelloRes),
    /// Reconocimiento de respuesta Hello de Kad2 (0x22, Kad >= 8).
    HelloResAck(KadHelloResAck),
    /// Petición heredada de comprobación de cortafuegos TCP de Kad (0x50).
    FirewalledReq(KadFirewalledReq),
    /// Petición v2 de comprobación de cortafuegos TCP de Kad (0x53, Kad >= 7).
    FirewalledReq2(KadFirewalledReq2),
    /// Respuesta de comprobación de cortafuegos de Kad (0x58) — devuelve nuestra IP pública.
    FirewalledRes(KadFirewalledRes),
    /// Acuse de recibo de cortafuegos de Kad por UDP (0x59, obsoleto desde Kad 7).
    FirewalledAckRes,
    /// Petición Bootstrap de Kad2.
    BootstrapReq(KadBootstrapReq),
    /// Respuesta Bootstrap / Find Node de Kad2.
    BootstrapRes(KadBootstrapRes),
    /// Petición genérica Find Node / Search de Kad2.
    Req(KadReq),
    /// Petición Search Source de Kad2.
    SearchSourceReq(KadSearchSourceReq),
    /// Respuesta Search Source de Kad2.
    SearchSourceRes(KadSearchSourceRes),
    /// Petición Publish Source de Kad2.
    PublishSourceReq(KadPublishSourceReq),
    /// Respuesta Publish Source de Kad2.
    PublishSourceRes(KadPublishSourceRes),
    /// Petición Search Keyword de Kad2.
    SearchKeywordReq(KadSearchKeywordReq),
    /// Respuesta Search Keyword de Kad2.
    SearchKeywordRes(KadSearchKeywordRes),
    /// Petición Publish Keyword de Kad2.
    PublishKeywordReq(KadPublishKeywordReq),
    /// Respuesta Publish Keyword de Kad2.
    PublishKeywordRes(KadPublishKeywordRes),
    /// Paquete de prueba de cortafuegos UDP de Kad2.
    FirewallUdp(KadFirewallUdp),
    /// Petición de búsqueda de buddy de Kad.
    FindBuddyReq(KadFindBuddyReq),
    /// Respuesta de búsqueda de buddy de Kad.
    FindBuddyRes(KadFindBuddyRes),
    /// Petición de retransmisión de callback buddy de Kad.
    CallbackReq(KadCallbackReq),
    /// Petición Search Notes de Kad2.
    SearchNotesReq(KadSearchNotesReq),
    /// Respuesta de búsqueda de notas (0x3B unificada o heredada 0x3A).
    SearchNotesRes(KadSearchNotesRes),
    /// Petición Publish Notes de Kad2.
    PublishNotesReq(KadPublishNotesReq),
    /// Respuesta Publish Notes de Kad2.
    PublishNotesRes(KadPublishNotesRes),
}

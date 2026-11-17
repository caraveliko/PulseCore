// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Bytes mágicos, códigos de operación y constantes del protocolo UDP de Kademlia.

/// Byte mágico estándar del protocolo UDP de Kademlia (0xE4).
pub const PR_KADEMLIA: u8 = 0xE4;

/// Byte mágico de protocolo UDP empaquetado de Kademlia (0xE5).
pub const PR_KADEMLIA_PACKED: u8 = 0xE5;

// eMule reserva `OP_UDPRESERVEDPROT1 = 0xA3` y `OP_UDPRESERVEDPROT2 = 0xB2`
// (`Opcodes.h:142-143`) para que los datagramas UDP ofuscados —cuyo primer byte es
// seudoaleatorio— puedan enrutarse a `EncryptedDatagramSocket`. Cualquier otro primer byte
// desconocido se prueba de igual forma contra la capa de ofuscación, por lo que no deben
// aceptarse bytes mágicos adicionales aquí: un datagrama que comience con ej. `0x8D` es
// candidato a paquete Kad *cifrado*, no en texto claro.

// --- Códigos de operación Kad v1.0 (Heredados) ---
/// Petición heredada de Bootstrap (0x00).
pub const KADEMLIA_BOOTSTRAP_REQ_OLD: u8 = 0x00;
/// Petición de Bootstrap (0x01).
pub const KADEMLIA_BOOTSTRAP_REQ: u8 = 0x01;
/// Respuesta de Bootstrap (0x09).
pub const KADEMLIA_BOOTSTRAP_RES: u8 = 0x09;
/// Petición Hello (0x10).
pub const KADEMLIA_HELLO_REQ: u8 = 0x10;
/// Respuesta Hello (0x18).
pub const KADEMLIA_HELLO_RES: u8 = 0x18;

// --- Códigos de operación Kad v2.0 (Especificación oficial eMule / aMule / Wireshark) ---
/// Petición Bootstrap de Kad2 (0x01).
pub const KADEMLIA2_BOOTSTRAP_REQ: u8 = 0x01;
/// Respuesta Bootstrap de Kad2 (0x09).
pub const KADEMLIA2_BOOTSTRAP_RES: u8 = 0x09;
/// Petición Hello de Kad2 (0x11).
pub const KADEMLIA2_HELLO_REQ: u8 = 0x11;
/// Respuesta Hello de Kad2 (0x19).
pub const KADEMLIA2_HELLO_RES: u8 = 0x19;
/// Acuse de recibo ACK de Hello de Kad2 (0x22).
pub const KADEMLIA2_HELLO_RES_ACK: u8 = 0x22;

// NOTA sobre los opcodes de búsqueda/publicación de Kad2 (referencia oficial eMule 0.72a):
// Peticiones:  SEARCH_KEY_REQ=0x33, SEARCH_SOURCE_REQ=0x34, SEARCH_NOTES_REQ=0x35
// Respuesta:   SEARCH_RES=0x3B (unificada para todos los tipos de búsqueda)
// Peticiones:  PUBLISH_KEY_REQ=0x43, PUBLISH_SOURCE_REQ=0x44, PUBLISH_NOTES_REQ=0x45
// Respuesta:   PUBLISH_RES=0x4B (unificada para todos los tipos de publicación)
/// Petición Find Node / Value de Kad2 (0x21).
pub const KADEMLIA2_REQ: u8 = 0x21;
/// Recuento de contactos / subtipo Find Node de Kad2 (0x0B = 11 contactos solicitados).
pub const KADEMLIA_FIND_NODE: u8 = 0x0B;
/// Recuento de contactos / subtipo Find Value de Kad2 (0x02 contactos solicitados).
pub const KADEMLIA_FIND_VALUE: u8 = 0x02;
/// Respuesta Find Node / Value de Kad2 (0x29).
pub const KADEMLIA2_RES: u8 = 0x29;
/// Petición Search Keyword de Kad2 (0x33).
pub const KADEMLIA2_SEARCH_KEY_REQ: u8 = 0x33;
/// Petición Search Source de Kad2 (0x34).
pub const KADEMLIA2_SEARCH_SOURCE_REQ: u8 = 0x34;
/// Petición Search Notes de Kad2 (0x35).
pub const KADEMLIA2_SEARCH_NOTES_REQ: u8 = 0x35;
/// Petición heredada Search Notes de Kad1 (0x32) — solo `[target:16]`.
pub const KADEMLIA_SEARCH_NOTES_REQ_LEGACY: u8 = 0x32;
/// Respuesta heredada Search Notes de Kad1 (0x3A) — `[target:16][count:u16]`
/// y luego las mismas entradas `[answer:16][taglist]` que la forma unificada 0x3B.
/// NOTA: eMule 0.72a responde a búsquedas de notas Kad2 con `KADEMLIA2_SEARCH_RES`
/// (0x3B); 0x3A solo se observa en nodos Kad1.
pub const KADEMLIA_SEARCH_NOTES_RES: u8 = 0x3A;
/// Respuesta Search de Kad2 (0x3B) — respuesta unificada para búsqueda de palabras clave, fuentes y notas.
pub const KADEMLIA2_SEARCH_RES: u8 = 0x3B;
/// Alias: la respuesta Search Source de Kad2 es la unificada KADEMLIA2_SEARCH_RES (0x3B).
pub const KADEMLIA2_SEARCH_SOURCE_RES: u8 = KADEMLIA2_SEARCH_RES;
/// Alias: la petición Search Keyword de Kad2 es KADEMLIA2_SEARCH_KEY_REQ (0x33).
pub const KADEMLIA2_SEARCH_KEYWORD_REQ: u8 = KADEMLIA2_SEARCH_KEY_REQ;
/// Alias: la respuesta Search Keyword de Kad2 es la unificada KADEMLIA2_SEARCH_RES (0x3B).
pub const KADEMLIA2_SEARCH_KEYWORD_RES: u8 = KADEMLIA2_SEARCH_RES;
/// Opcode interno heredado de respuesta Search Keyword (0x2C, preservado para respaldo/pruebas).
pub const KADEMLIA2_SEARCH_KEYWORD_RES_LEGACY: u8 = 0x2C;
/// Petición Publish Keyword de Kad2 (0x43).
pub const KADEMLIA2_PUBLISH_KEY_REQ: u8 = 0x43;
/// Petición Publish Source de Kad2 (0x44).
pub const KADEMLIA2_PUBLISH_SOURCE_REQ: u8 = 0x44;
/// Petición Publish Notes de Kad2 (0x45).
pub const KADEMLIA2_PUBLISH_NOTES_REQ: u8 = 0x45;
/// Respuesta Publish de Kad2 (0x4B) — respuesta unificada para publicación de palabras clave, fuentes y notas.
pub const KADEMLIA2_PUBLISH_RES: u8 = 0x4B;
/// Acuse de recibo de resultado de publicación de Kad2 (0x4C, carga útil vacía).
///
/// eMule `KademliaUDPListener.cpp:1444`: devuelto cuando un `KADEMLIA2_PUBLISH_RES`
/// entrante incluye un byte de opciones final con el bit 0 activo
/// ("para uso futuro" — ningún cliente publicado lo activa hoy). eMule
/// no procesa 0x4C entrante; recibirlo se registra y se descarta.
pub const KADEMLIA2_PUBLISH_RES_ACK: u8 = 0x4C;
/// Alias: la respuesta Publish Source de Kad2 es la unificada KADEMLIA2_PUBLISH_RES (0x4B).
pub const KADEMLIA2_PUBLISH_SOURCE_RES: u8 = KADEMLIA2_PUBLISH_RES;
/// Alias: la petición Publish Keyword de Kad2 es KADEMLIA2_PUBLISH_KEY_REQ (0x43).
pub const KADEMLIA2_PUBLISH_KEYWORD_REQ: u8 = KADEMLIA2_PUBLISH_KEY_REQ;
/// Alias: la respuesta Publish Keyword de Kad2 es la unificada KADEMLIA2_PUBLISH_RES (0x4B).
/// NOTA: Previamente era 0x2D, un código inventado que no existe en el eMule oficial.
pub const KADEMLIA2_PUBLISH_KEYWORD_RES: u8 = KADEMLIA2_PUBLISH_RES;
/// Petición Ping de Kad2 (0x60).
pub const KADEMLIA2_PING: u8 = 0x60;
/// Respuesta Pong de Kad2 (0x61).
pub const KADEMLIA2_PONG: u8 = 0x61;
/// Prueba de cortafuegos UDP de Kad2 (0x62).
pub const KADEMLIA2_FIREWALLUDP: u8 = 0x62;
/// Petición de búsqueda de buddy de Kad (0x51): [buddy_id:16][user_id:16][tcp_port:2].
pub const KADEMLIA_FINDBUDDY_REQ: u8 = 0x51;
/// Petición de retransmisión de callback buddy de Kad (0x52): [buddy_id:16][file_id:16][tcp_port:2].
pub const KADEMLIA_CALLBACK_REQ: u8 = 0x52;
/// Respuesta de búsqueda de buddy de Kad (0x5A): [buddy_id:16][client_id:16][tcp_port:2][connect_opts:1].
pub const KADEMLIA_FINDBUDDY_RES: u8 = 0x5A;
/// Petición de comprobación de cortafuegos TCP de Kad (0x50): `[tcp_port u16]` (heredada, previa a Kad7).
pub const KADEMLIA_FIREWALLED_REQ: u8 = 0x50;
/// Petición v2 de comprobación de cortafuegos TCP de Kad (0x53, eMule `KADEMLIA_FIREWALLED2_REQ`):
/// `[tcp_port u16][user_hash 16][connect_options u8]` — 19 bytes.
/// Requerida para versiones de Kad >= 7.
pub const KADEMLIA_FIREWALLED2_REQ: u8 = 0x53;
/// Respuesta de comprobación de cortafuegos de Kad (0x58): `[ip u32]` — devuelve la IP del solicitante.
pub const KADEMLIA_FIREWALLED_RES: u8 = 0x58;
/// Acuse de recibo de cortafuegos de Kad por UDP (0x59, sustituido por `OP_KAD_FWTCPCHECK_ACK`
/// sobre TCP desde la versión 7 de Kad).
pub const KADEMLIA_FIREWALLED_ACK_RES: u8 = 0x59;

/// `TAG_KADMISCOPTIONS` de eMule (`0xF2`, uint8) transportado en KADEMLIA2_HELLO_*:
/// bit 0 = cortafuegos UDP, bit 1 = cortafuegos TCP, bit 2 = solicita
/// `KADEMLIA2_HELLO_RES_ACK`. Requerido para versiones de Kad >= 8.
pub const TAG_KADMISCOPTIONS: u8 = 0xF2;
/// `TAG_SOURCEUPORT` de eMule (`0xFC`, uint16) — puerto UDP de Kad anunciado.
pub const TAG_SOURCEUPORT: u8 = 0xFC;

/// Máximo de entradas de búsqueda servidas/enviadas por petición de búsqueda Kad.
///
/// eMule `CIndexed::SendValidSourceResult`/`SendValidKeywordResult`
/// (`iMaxResults = 300`, `Indexed.cpp:619/707`): hasta 300 resultados repartidos
/// en tantos datagramas como sea necesario para mantenerse bajo `UDP_KAD_MAXFRAGMENT`.
/// Este es también el límite de decodificación — un par no puede enviar legalmente más.
pub const MAX_KAD_SEARCH_KEYWORDS: usize = 300;

/// Tamaño máximo permitido de carga útil de datagrama UDP en bytes (la carga útil máxima UDP IPv4 es 65507 bytes).
pub const MAX_KAD_UDP_PACKET_SIZE: usize = 65507;

/// Comprueba si el byte de protocolo es un byte mágico reconocido del protocolo UDP de Kademlia.
#[inline]
pub fn is_valid_kad_protocol(protocol: u8) -> bool {
    matches!(protocol, PR_KADEMLIA | PR_KADEMLIA_PACKED)
}

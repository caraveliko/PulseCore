// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Modelo del índice de notas de Kad (comentarios / valoraciones de ficheros, rol de nodo de almacenamiento).

/// Una entrada de nota indexada para un archivo objetivo, reflejando `CEntry` de eMule con
/// `m_bSource = false`. Las etiquetas se mantienen como su blob codificado en el cable
/// (`[tag_count:u8][tags]`) de modo que etiquetas arbitrarias/desconocidas sobrevivan a un reinicio;
/// la capa de protocolo las redecodifica al hidratar.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KadIndexedNote {
    /// Hexadecimal en minúsculas de 32 caracteres del hash del archivo objetivo.
    pub file_hash: String,
    /// Hexadecimal en minúsculas de 32 caracteres del Kad ID del publicador (`m_uSourceID`).
    pub source_id: String,
    /// IPv4 del publicador (cadena con puntos).
    pub publisher_ip: String,
    /// Puerto UDP de Kad del publicador.
    pub publisher_udp_port: u16,
    /// Blob de lista de etiquetas codificado en el cable (datos acotados, solo para visualización).
    pub tags_blob: Vec<u8>,
    /// Marca de tiempo de cuando se almacenó por primera vez.
    pub discovered_at_unix_ms: i64,
    /// Marca de tiempo de caducidad (TTL ~5h, como en `KADEMLIAREPUBLISHTIMEN` de eMule).
    pub expires_at_unix_ms: i64,
}

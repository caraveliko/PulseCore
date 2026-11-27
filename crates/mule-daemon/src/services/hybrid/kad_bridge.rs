// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Puente descentralizado sobre notas Kad (`KADEMLIA2_PUBLISH_NOTES_REQ` / `SEARCH_NOTES_REQ`).
//!
//! Usa la propia red Kad existente (incluidos nodos eMule 0.72a clásicos) como almacén distribuido
//! clave-valor `ed2k_hash → bt_info_hash` sin servidores ni indexadores externos.

use std::collections::HashSet;

use crate::state::AppState;

/// Prefijo compacto y versionado para vínculos híbridos embebidos en comentarios de notas Kad.
pub const KAD_HYBRID_NOTE_PREFIX: &str = "pc1:btih:";
/// Marcador opcional de tamaño exacto dentro de la nota Kad híbrida.
const KAD_HYBRID_SIZE_MARKER: &str = ":sz:";
/// Valoración máxima asignada a notas Kad verificadas criptográficamente (escala eMule 1..=5).
const KAD_HYBRID_NOTE_RATING: u8 = 5;
/// Longitud exacta de un infohash BitTorrent v1 en hexadecimal.
const BTIH_HEX_LEN: usize = 40;
/// Longitud exacta de un infohash BitTorrent v1 codificado en Base32 RFC 4648 (160 bits = 32 caracteres).
const BTIH_BASE32_LEN: usize = 32;
/// Marcador estándar de infohash dentro de enlaces `magnet:` o comentarios de usuarios en Kad.
const URN_BTIH_MARKER: &str = "urn:btih:";
/// Marcador abreviado ocasional en comentarios Kad clásicos.
const SHORT_BTIH_MARKER: &str = "btih:";

/// Formatea un puente verificado en una nota Kad compacta compatible con clientes eMule clásicos.
pub fn format_kad_hybrid_note(bt_info_hash: &str, file_size: u64) -> Option<String> {
    let clean = bt_info_hash.trim().to_ascii_lowercase();
    if clean.len() != BTIH_HEX_LEN || !clean.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    if file_size == 0 {
        return None;
    }
    Some(format!(
        "{KAD_HYBRID_NOTE_PREFIX}{clean}{KAD_HYBRID_SIZE_MARKER}{file_size}"
    ))
}

/// Extrae un `bt_info_hash` candidato (40 hex minúsculas) de cualquier comentario de nota Kad:
/// 1. Formato estructurado de Pulse Core (`pc1:btih:<40-hex>[:sz:<bytes>]`, exigiendo tamaño exacto).
/// 2. Enlaces `magnet:?xt=urn:btih:<40-hex | 32-base32>` o referencias `btih:<40-hex>` en comentarios clásicos.
///
/// Todo candidato extraído se somete posteriormente a validación criptográfica MD4 antes de fusionar bytes.
pub fn parse_kad_hybrid_note(comment: &str, expected_file_size: u64) -> Option<String> {
    if let Some(idx) = comment.find(KAD_HYBRID_NOTE_PREFIX) {
        let after_prefix = &comment[idx + KAD_HYBRID_NOTE_PREFIX.len()..];
        if after_prefix.len() >= BTIH_HEX_LEN {
            let hex_part = &after_prefix[..BTIH_HEX_LEN];
            if hex_part.bytes().all(|b| b.is_ascii_hexdigit()) {
                let rest = &after_prefix[BTIH_HEX_LEN..];
                if let Some(after_sz) = rest.strip_prefix(KAD_HYBRID_SIZE_MARKER) {
                    let digits: String = after_sz
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    let declared_size = digits.parse::<u64>().ok()?;
                    if expected_file_size > 0 && declared_size != expected_file_size {
                        return None;
                    }
                }
                return Some(hex_part.to_ascii_lowercase());
            }
        }
    }

    extract_btih_from_free_comment(comment)
}

fn extract_btih_from_free_comment(comment: &str) -> Option<String> {
    let lower = comment.to_ascii_lowercase();
    for marker in [URN_BTIH_MARKER, SHORT_BTIH_MARKER] {
        if let Some(pos) = lower.find(marker) {
            let rest = &comment[pos + marker.len()..];
            let token: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            if let Some(hex_hash) = normalize_btih_token(&token) {
                return Some(hex_hash);
            }
        }
    }
    None
}

/// Normaliza un identificador BitTorrent v1 (40 hex o 32 Base32) a 40 caracteres hexadecimales minúsculas.
pub fn normalize_btih_token(token: &str) -> Option<String> {
    if token.len() == BTIH_HEX_LEN && token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(token.to_ascii_lowercase());
    }
    if token.len() == BTIH_BASE32_LEN {
        let upper = token.to_ascii_uppercase();
        if let Ok(decoded) = data_encoding::BASE32_NOPAD.decode(upper.as_bytes()) {
            if decoded.len() == 20 {
                return Some(hex::encode(decoded));
            }
        }
    }
    None
}

/// Consulta notas Kad asociadas a `ed2k_hash_hex` y devuelve los `bt_info_hash` candidatos descubiertos.
pub async fn discover_bridges_via_kad_notes(
    state: &AppState,
    ed2k_hash_hex: &str,
    file_size: u64,
) -> Vec<String> {
    let Ok(notes) = state.kad.search_notes(ed2k_hash_hex, file_size).await else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for note in notes {
        let Some(comment) = note.get("comment").and_then(|v| v.as_str()) else {
            continue;
        };
        if let Some(btih) = parse_kad_hybrid_note(comment, file_size) {
            if seen.insert(btih.clone()) {
                out.push(btih);
            }
        }
    }
    out
}

/// Publica en Kad un puente ya verificado criptográficamente para que otros nodos híbridos lo descubran.
pub async fn publish_verified_bridge_to_kad(
    state: &AppState,
    ed2k_hash_hex: &str,
    file_name: &str,
    file_size: u64,
    bt_info_hash: &str,
) -> Result<usize, String> {
    let note_text = format_kad_hybrid_note(bt_info_hash, file_size)
        .ok_or_else(|| "info_hash o tamaño inválidos para publicar nota Kad híbrida".to_string())?;
    state
        .kad
        .publish_note(
            ed2k_hash_hex,
            file_name,
            file_size,
            &note_text,
            Some(KAD_HYBRID_NOTE_RATING),
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_parses_kad_hybrid_notes_strictly() {
        let ih = "0123456789abcdef0123456789abcdef01234567";
        let note = format_kad_hybrid_note(ih, 4_721_883_900).unwrap();
        assert_eq!(note, format!("pc1:btih:{ih}:sz:4721883900"));

        assert_eq!(
            parse_kad_hybrid_note(&note, 4_721_883_900).as_deref(),
            Some(ih)
        );
        // Tamaño distinto debe rechazarse para evitar falsos enlaces.
        assert_eq!(parse_kad_hybrid_note(&note, 9_999_999), None);
        // Comentario normal de usuario eMule se ignora.
        assert_eq!(
            parse_kad_hybrid_note("Buena calidad de imagen y audio", 4_721_883_900),
            None
        );
        // Enlace magnet estándar en comentario clásico (hex 40 caracteres).
        let magnet_hex = format!("Descargado de magnet:?xt=urn:btih:{ih}&dn=Pelicula.mkv");
        assert_eq!(
            parse_kad_hybrid_note(&magnet_hex, 4_721_883_900).as_deref(),
            Some(ih)
        );
        // Enlace magnet en Base32 RFC 4648 (32 caracteres) en comentario clásico.
        let raw_bytes = hex::decode(ih).unwrap();
        let b32 = data_encoding::BASE32_NOPAD.encode(&raw_bytes);
        let magnet_b32 = format!("magnet:?xt=urn:btih:{b32}&dn=Naruto.mkv");
        assert_eq!(
            parse_kad_hybrid_note(&magnet_b32, 4_721_883_900).as_deref(),
            Some(ih)
        );
    }
}

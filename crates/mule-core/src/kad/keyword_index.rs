// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Modelos y utilidades del índice de palabras clave de Kad.

use super::KadId;
use md4::{Digest, Md4};

/// Una asociación de palabra clave-archivo indexada en la DHT de Kad.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KadKeywordEntry {
    /// Hexadecimal en minúsculas de 32 caracteres del hash MD4 de la palabra clave.
    pub keyword_hash: String,
    /// Hexadecimal en minúsculas de 32 caracteres del hash MD4 del archivo.
    pub file_hash: String,
    /// Nombre del archivo.
    pub file_name: String,
    /// Tamaño del archivo en bytes.
    pub file_size: u64,
    /// Número de fuentes que anuncian este archivo.
    pub source_count: u32,
    /// Marca de tiempo cuando fue descubierto.
    pub discovered_at_unix_ms: i64,
    /// Marca de tiempo de caducidad (TTL).
    pub expires_at_unix_ms: i64,
}

/// Normaliza una cadena de palabra clave (minúsculas, recortada) y calcula su KadId MD4 de 128 bits.
pub fn hash_kad_keyword(keyword: &str) -> (String, KadId) {
    let normalized = keyword.trim().to_lowercase();
    let mut hasher = Md4::new();
    hasher.update(normalized.as_bytes());
    let digest: [u8; 16] = hasher.finalize().into();
    (normalized, KadId::from_hash_be(&digest))
}

/// Extrae palabras clave de búsqueda (longitud >= 3) de un nombre de archivo y devuelve pares de (palabra, KadId).
pub fn extract_kad_keywords(file_name: &str) -> Vec<(String, KadId)> {
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Dividir según delimitadores de palabras típicos
    let words = file_name.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '.' | '_'
                    | '-'
                    | '['
                    | ']'
                    | '('
                    | ')'
                    | '{'
                    | '}'
                    | '+'
                    | ','
                    | ';'
                    | '!'
                    | '?'
                    | ':'
                    | '\''
                    | '"'
                    | '@'
                    | '#'
                    | '$'
                    | '%'
                    | '^'
                    | '&'
                    | '*'
                    | '='
                    | '~'
                    | '`'
                    | '|'
                    | '/'
                    | '\\'
            )
    });

    for word in words {
        let trimmed = word.trim();
        if trimmed.len() >= 3 {
            let (normalized, kad_id) = hash_kad_keyword(trimmed);
            if seen.insert(normalized.clone()) {
                results.push((normalized, kad_id));
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_kad_keywords() {
        let name = "Ubuntu-22.04.3-live-server-amd64.iso";
        let keywords = extract_kad_keywords(name);
        let words: Vec<&str> = keywords.iter().map(|(w, _)| w.as_str()).collect();

        assert!(words.contains(&"ubuntu"));
        assert!(words.contains(&"live"));
        assert!(words.contains(&"server"));
        assert!(words.contains(&"amd64"));
        assert!(words.contains(&"iso"));
        // Los números como "22" o "04" (< 3 caracteres) quedan excluidos
        assert!(!words.contains(&"22"));
    }

    #[test]
    fn test_hash_kad_keyword() {
        let (norm, id) = hash_kad_keyword("Linux");
        assert_eq!(norm, "linux");
        let (norm2, id2) = hash_kad_keyword("  LINUX  ");
        assert_eq!(norm2, "linux");
        assert_eq!(id, id2);
    }
}

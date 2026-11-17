// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Árboles de expresiones de búsqueda restrictiva de Kad.
//!
//! Implementa el formato de red y la semántica de evaluación de eMule
//! `CreateSearchExpressionTree` (`KademliaUDPListener.cpp:883`) y
//! `CKeyEntry::StartSearchTermsMatch` (`Entry.cpp:224`):
//!
//! - `0x00` + operador booleano (`0x00` AND / `0x01` OR / `0x02` NOT) + dos expresiones
//!   hijas. `NOT` significa *izquierda y no derecha*.
//! - `0x01` + cadena UTF-8 — término de nombre de fichero: la cadena se pasa a minúsculas y
//!   se pretokeniza (conservando las partes entrecomilladas); cada token debe ser una subcadena
//!   del nombre de fichero en minúsculas (`"a b"` se comporta como `a AND b`).
//! - `0x02` + valor UTF-8 + `u16` longitud de nombre de etiqueta + bytes de nombre de etiqueta — igualdad
//!   insensible a mayúsculas frente a una etiqueta de cadena almacenada; `TAG_FILEFORMAT`
//!   (`0x04`) compara frente a la extensión del nombre de fichero en su lugar.
//! - `0x03`/`0x08` + valor `u32`/`u64` LE + `u8` operador de comparación + `u16`
//!   longitud de nombre de etiqueta + bytes de nombre de etiqueta — relación entera frente a una etiqueta
//!   entera almacenada; `TAG_FILESIZE` (`0x02`) es una etiqueta virtual resuelta al tamaño de fichero
//!   de la entrada (eMule `CEntry::GetIntTagValue` con `bIncludeVirtualTags`).

use crate::ed2k::tags::{Ed2kTag, TagName, TagValue};
use crate::ed2k::{Ed2kErrorKind, Ed2kParseError};
use crate::kad::types::KadKeywordEntryWire;

/// Profundidad máxima del árbol de expresiones — eMule rechaza árboles con `iLevel >= 24`
/// (`KademliaUDPListener.cpp:887`).
pub const MAX_SEARCH_EXPR_DEPTH: usize = 24;

/// Id de etiqueta comparado contra la extensión del nombre de archivo sin almacenarse
/// (caso especial `TAG_FILEFORMAT` de eMule, `Entry.cpp:260`).
const TAG_ID_FILEFORMAT: u8 = 0x04;

/// Id de etiqueta resuelto al campo `file_size` de la entrada
/// (etiqueta virtual `TAG_FILESIZE` de eMule, `Entry.cpp:101`).
const TAG_ID_FILESIZE: u8 = 0x02;

/// Conjunto de delimitadores para dividir términos de nombres sin entrecomillar
/// (eMule `INV_KAD_KEYWORD_CHARS`, `SearchManager.cpp:54`).
const INV_KAD_KEYWORD_CHARS: [char; 20] = [
    ' ', '(', ')', '[', ']', '{', '}', '<', '>', ',', '.', '_', '-', '!', '?', ':', ';', '\\', '/',
    '"',
];

/// Tipos de nodos hoja de expresión analizados desde el cable.
#[derive(Debug, Clone, PartialEq)]
pub enum KadSearchTerm {
    /// Filename term — every pre-tokenized word must be a substring of the
    /// lowercased file name (eMule `SSearchTerm::String`).
    Str(Vec<String>),
    /// Igualdad de cadena de metadatos (eMule `SSearchTerm::MetaTag`).
    MetaStr {
        /// Bytes en bruto del nombre de etiqueta (1 byte = id de etiqueta, más largo = cadena de nombre).
        name: Vec<u8>,
        /// Valor esperado en minúsculas.
        value: String,
    },
    /// Integer metadata relation (eMule `OpEqual`…`OpNotEqual` over
    /// `CKadTagUInt64`; float expression terms are never produced by
    /// `CreateSearchExpressionTree`, so only int terms exist on the wire).
    Num {
        /// Bytes en bruto del nombre de etiqueta (1 byte = id de etiqueta, más largo = cadena de nombre).
        name: Vec<u8>,
        /// Operador de comparación (códigos de cable 0x00–0x05).
        op: KadSearchNumOp,
        /// Valor de comparación derecho.
        value: u64,
    },
}

/// Operadores de comparación entera (códigos de cable `mmop`, `Entry.cpp:284`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KadSearchNumOp {
    /// `0x00` — igual.
    Equal,
    /// `0x01` — estrictamente mayor.
    Greater,
    /// `0x02` — estrictamente menor.
    Less,
    /// `0x03` — mayor o igual.
    GreaterEqual,
    /// `0x04` — menor o igual.
    LessEqual,
    /// `0x05` — distinto.
    NotEqual,
}

/// Árbol analizado de expresiones de búsqueda restrictivas.
#[derive(Debug, Clone, PartialEq)]
pub enum KadSearchExpression {
    /// `AND` — ambos hijos deben coincidir.
    And(Box<KadSearchExpression>, Box<KadSearchExpression>),
    /// `OR` — al menos un hijo debe coincidir.
    Or(Box<KadSearchExpression>, Box<KadSearchExpression>),
    /// `NOT` — el hijo izquierdo debe coincidir y el derecho no.
    Not(Box<KadSearchExpression>, Box<KadSearchExpression>),
    /// Término hoja.
    Term(KadSearchTerm),
}

impl KadSearchExpression {
    /// Parses a trailing search-expression tree from `data`
    /// (`CreateSearchExpressionTree`). Trailing bytes after a complete tree
    /// are ignored, like eMule reading from a stream. Returns an
    /// `Ed2kParseError` on malformed or truncated input, unknown operators,
    /// o árboles más profundos que `MAX_SEARCH_EXPR_DEPTH` — el llamador debe descartar
    /// the datagram, like eMule's `try/catch` in
    /// `Process_KADEMLIA2_SEARCH_KEY_REQ`.
    pub fn parse(protocol: u8, opcode: u8, data: &[u8]) -> Result<Self, Ed2kParseError> {
        let mut cur = Cursor {
            data,
            pos: 0,
            protocol,
            opcode,
        };
        parse_term(&mut cur, 0)
    }

    /// Evaluates the expression against a stored keyword entry
    /// (`CKeyEntry::StartSearchTermsMatch`). The entry's lowercased file name
    /// is computed once by the caller via `KadKeywordSearchView`.
    pub fn matches(&self, view: &KadKeywordSearchView<'_>) -> bool {
        match self {
            Self::And(l, r) => l.matches(view) && r.matches(view),
            Self::Or(l, r) => l.matches(view) || r.matches(view),
            Self::Not(l, r) => l.matches(view) && !r.matches(view),
            Self::Term(t) => t.matches(view),
        }
    }
}

/// Contexto de evaluación para una entrada de palabra clave almacenada: nombre de archivo en minúsculas,
/// tamaño de archivo y lista de etiquetas publicada (campos de eMule `CKeyEntry`:
/// `m_strSearchTermCacheCommonFileNameLowerCase`, `m_uSize`, `m_listTag`).
#[derive(Debug)]
pub struct KadKeywordSearchView<'a> {
    /// File name lowercased once per evaluation (eMule caches it on the
    /// entry for the duration of the match).
    file_name_lower: String,
    /// Tamaño del archivo de la entrada — valor virtual de `TAG_FILESIZE`.
    file_size: u64,
    /// Lista de etiquetas de metadatos publicada.
    tags: &'a [Ed2kTag],
}

impl<'a> KadKeywordSearchView<'a> {
    /// Construye la vista de evaluación de una entrada de palabra clave almacenada.
    pub fn new(entry: &'a KadKeywordEntryWire) -> Self {
        Self {
            file_name_lower: entry.file_name.to_lowercase(),
            file_size: entry.file_size,
            tags: &entry.tags,
        }
    }
}

impl KadSearchTerm {
    fn matches(&self, view: &KadKeywordSearchView<'_>) -> bool {
        match self {
            Self::Str(tokens) => {
                if tokens.is_empty() {
                    return false;
                }
                tokens
                    .iter()
                    .all(|t| view.file_name_lower.contains(t.as_str()))
            }
            Self::MetaStr { name, value } => {
                if name.as_slice() == [TAG_ID_FILEFORMAT] {
                    // eMule: `fileformat` is part of the file name — compare
                    // la extensión (tras el último '.') sin distinguir mayúsculas.
                    if let Some(ext) = view.file_name_lower.rsplit('.').next() {
                        if view.file_name_lower.contains('.') && ext == value {
                            return true;
                        }
                    }
                }
                view.tags.iter().any(|tag| {
                    tag_name_matches(&tag.name, name)
                        && matches!(&tag.value, TagValue::String(s) if s.to_lowercase() == *value)
                })
            }
            Self::Num { name, op, value } => {
                // eMule `GetIntTagValue`: first matching int tag by exact
                // nombre; `TAG_FILESIZE` resuelve al tamaño de entrada cuando no hay
                // stored tag matches.
                let entry_value = view
                    .tags
                    .iter()
                    .find_map(|tag| match (&tag.name, &tag.value) {
                        (n, v) if tag_name_matches(n, name) => tag_int_value(v),
                        _ => None,
                    })
                    .or_else(|| {
                        (name.as_slice() == [TAG_ID_FILESIZE]).then_some(view.file_size)
                    });
                let Some(entry_value) = entry_value else {
                    return false;
                };
                match op {
                    KadSearchNumOp::Equal => entry_value == *value,
                    KadSearchNumOp::Greater => entry_value > *value,
                    KadSearchNumOp::Less => entry_value < *value,
                    KadSearchNumOp::GreaterEqual => entry_value >= *value,
                    KadSearchNumOp::LessEqual => entry_value <= *value,
                    KadSearchNumOp::NotEqual => entry_value != *value,
                }
            }
        }
    }
}

/// eMule `pTag->m_name.Compare(name) == 0` — comparación exacta de bytes: un nombre
/// de expresión de 1 byte coincide con `TagName::Id`, nombres más largos coinciden con `TagName::Text`
/// byte a byte.
fn tag_name_matches(tag_name: &TagName, expr_name: &[u8]) -> bool {
    match tag_name {
        TagName::Id(id) => expr_name == [*id].as_slice(),
        TagName::Text(s) => expr_name == s.as_bytes(),
    }
}

/// Valor entero de una etiqueta almacenada (eMule `CKadTag::IsInt()`).
fn tag_int_value(value: &TagValue) -> Option<u64> {
    match value {
        TagValue::UInt64(v) => Some(*v),
        TagValue::UInt32(v) => Some(u64::from(*v)),
        TagValue::UInt16(v) => Some(u64::from(*v)),
        TagValue::UInt8(v) => Some(u64::from(*v)),
        _ => None,
    }
}

/// Cursor acotado sobre el flujo de bytes de la expresión.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    protocol: u8,
    opcode: u8,
}

impl<'a> Cursor<'a> {
    fn err(&self, field: &'static str, kind: Ed2kErrorKind) -> Ed2kParseError {
        Ed2kParseError::new(self.protocol, Some(self.opcode), self.pos, field, kind)
    }

    fn eof(&self, field: &'static str, needed: usize) -> Ed2kParseError {
        self.err(
            field,
            Ed2kErrorKind::UnexpectedEof {
                needed,
                available: self.data.len().saturating_sub(self.pos),
            },
        )
    }

    fn u8(&mut self, field: &'static str) -> Result<u8, Ed2kParseError> {
        if self.pos >= self.data.len() {
            return Err(self.eof(field, 1));
        }
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    fn u16(&mut self, field: &'static str) -> Result<u16, Ed2kParseError> {
        if self.pos + 2 > self.data.len() {
            return Err(self.eof(field, 2));
        }
        let v = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    fn take(&mut self, len: usize, field: &'static str) -> Result<&'a [u8], Ed2kParseError> {
        if self.pos + len > self.data.len() {
            return Err(self.eof(field, len));
        }
        let s = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(s)
    }

    /// `CFileDataIO::ReadStringUTF8` — `u16` raw length + bytes, decoded as
    /// UTF-8 con reserva con pérdida (eMule recurre a la página de códigos local).
    fn utf8_string(&mut self, field: &'static str) -> Result<String, Ed2kParseError> {
        let len = self.u16(field)? as usize;
        let bytes = self.take(len, field)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

fn parse_term(cur: &mut Cursor<'_>, level: usize) -> Result<KadSearchExpression, Ed2kParseError> {
    // eMule `iLevel >= 24` -> NULL (el nivel 0 del llamador es la raíz).
    if level >= MAX_SEARCH_EXPR_DEPTH {
        return Err(cur.err(
            "search_expr_depth",
            Ed2kErrorKind::MalformedField {
                message: "search expression tree exceeds depth limit".to_string(),
            },
        ));
    }
    let level = level + 1;
    let op = cur.u8("search_expr_op")?;
    match op {
        0x00 => {
            let bool_op = cur.u8("search_expr_bool_op")?;
            let make = match bool_op {
                0x00 => KadSearchExpression::And,
                0x01 => KadSearchExpression::Or,
                0x02 => KadSearchExpression::Not,
                _ => {
                    return Err(cur.err(
                        "search_expr_bool_op",
                        Ed2kErrorKind::MalformedField {
                            message: "unknown boolean search operator".to_string(),
                        },
                    ))
                }
            };
            let left = parse_term(cur, level)?;
            let right = parse_term(cur, level)?;
            Ok(make(Box::new(left), Box::new(right)))
        }
        0x01 => {
            let raw = cur.utf8_string("search_expr_string")?;
            // eMule `KadTagStrMakeLower` + `TokenizeOptQuotedSearchTerm`.
            let tokens = tokenize_search_term(&raw.to_lowercase());
            Ok(KadSearchExpression::Term(KadSearchTerm::Str(tokens)))
        }
        0x02 => {
            let value = cur.utf8_string("search_expr_meta_value")?.to_lowercase();
            let name = read_tag_name(cur)?;
            Ok(KadSearchExpression::Term(KadSearchTerm::MetaStr {
                name,
                value,
            }))
        }
        0x03 | 0x08 => {
            let width = if op == 0x03 { 4 } else { 8 };
            let bytes = cur.take(width, "search_expr_num_value")?;
            let mut buf = [0u8; 8];
            buf[..width].copy_from_slice(bytes);
            let value = u64::from_le_bytes(buf);
            let mmop = cur.u8("search_expr_num_op")?;
            let op = match mmop {
                0x00 => KadSearchNumOp::Equal,
                0x01 => KadSearchNumOp::Greater,
                0x02 => KadSearchNumOp::Less,
                0x03 => KadSearchNumOp::GreaterEqual,
                0x04 => KadSearchNumOp::LessEqual,
                0x05 => KadSearchNumOp::NotEqual,
                _ => {
                    return Err(cur.err(
                        "search_expr_num_op",
                        Ed2kErrorKind::MalformedField {
                            message: "unknown integer search operator".to_string(),
                        },
                    ))
                }
            };
            let name = read_tag_name(cur)?;
            Ok(KadSearchExpression::Term(KadSearchTerm::Num {
                name,
                op,
                value,
            }))
        }
        _ => Err(cur.err(
            "search_expr_op",
            Ed2kErrorKind::MalformedField {
                message: "unknown search expression opcode".to_string(),
            },
        )),
    }
}

/// Longitud `u16` del nombre de etiqueta + bytes en bruto (no UTF-8 — eMule los lee en un `CStringA`).
fn read_tag_name(cur: &mut Cursor<'_>) -> Result<Vec<u8>, Ed2kParseError> {
    let len = cur.u16("search_expr_tag_name_len")? as usize;
    Ok(cur.take(len, "search_expr_tag_name")?.to_vec())
}

/// eMule `TokenizeOptQuotedSearchTerm` (`KademliaUDPListener.cpp:848`):
/// divide por `INV_KAD_KEYWORD_CHARS`; una comilla `"` inicia una sección entrecomillada
/// literal hasta la siguiente `"` (las comillas sin cerrar se omiten); se descartan
/// los tokens vacíos. La entrada ya está en minúsculas.
fn tokenize_search_term(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"' {
            // Quoted section: take until the next '"'. If none, the start
            // quote is just skipped.
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != '"' {
                i += 1;
            }
            if i < chars.len() {
                if i > start {
                    tokens.push(chars[start..i].iter().collect());
                }
                i += 1;
            }
        } else {
            let start = i;
            while i < chars.len() && !INV_KAD_KEYWORD_CHARS.contains(&chars[i]) {
                i += 1;
            }
            if i > start {
                tokens.push(chars[start..i].iter().collect());
                if i == chars.len() {
                    break;
                }
                if chars[i] == '"' {
                    continue;
                }
            }
            i += 1;
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ed2k::tags::{TagName, TagValue};

    const PROTO: u8 = 0xE4;
    const OP: u8 = 0x33;

    fn str_leaf(s: &str) -> Vec<u8> {
        let mut v = vec![0x01u8];
        v.extend_from_slice(&(s.len() as u16).to_le_bytes());
        v.extend_from_slice(s.as_bytes());
        v
    }

    fn meta_str_leaf(name: &[u8], value: &str) -> Vec<u8> {
        let mut v = vec![0x02u8];
        v.extend_from_slice(&(value.len() as u16).to_le_bytes());
        v.extend_from_slice(value.as_bytes());
        v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        v.extend_from_slice(name);
        v
    }

    fn num_leaf(name: &[u8], op: u8, value: u64, wide: bool) -> Vec<u8> {
        let mut v = vec![if wide { 0x08u8 } else { 0x03u8 }];
        if wide {
            v.extend_from_slice(&value.to_le_bytes());
        } else {
            v.extend_from_slice(&(value as u32).to_le_bytes());
        }
        v.push(op);
        v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        v.extend_from_slice(name);
        v
    }

    fn bool_node(op: u8, l: &[u8], r: &[u8]) -> Vec<u8> {
        let mut v = vec![0x00u8, op];
        v.extend_from_slice(l);
        v.extend_from_slice(r);
        v
    }

    fn entry(name: &str, size: u64, sources: u32, tags: Vec<Ed2kTag>) -> KadKeywordEntryWire {
        KadKeywordEntryWire {
            file_hash: [1u8; 16],
            file_name: name.to_string(),
            file_size: size,
            source_count: sources,
            tags,
            aich_pub: None,
            aich_votes: Vec::new(),
            publish_info: None,
        }
    }

    fn eval(expr: &KadSearchExpression, entry: &KadKeywordEntryWire) -> bool {
        expr.matches(&KadKeywordSearchView::new(entry))
    }

    fn assert_str_term(term: &str, hit: &str, miss: &str) {
        let e = KadSearchExpression::parse(PROTO, OP, &str_leaf(term)).unwrap();
        assert!(eval(&e, &entry(hit, 1, 1, vec![])), "expected match: {hit}");
        assert!(
            !eval(&e, &entry(miss, 1, 1, vec![])),
            "expected no match: {miss}"
        );
    }

    #[test]
    fn str_term_substring_case_insensitive() {
        assert_str_term("Movie", "A.MOVIE.2024.x264.mkv", "other.bin");
    }

    #[test]
    fn str_term_multiple_words_are_anded() {
        // "a b" se comporta como a AND b (subcadena del nombre en minúsculas).
        assert_str_term("movie x264", "a.movie.2024.x264.mkv", "a.movie.2024.mkv");
    }

    #[test]
    fn str_term_quoted_part_kept_verbatim() {
        // El entrecomillado "movie x" permanece como un único token; b sin entrecomillar es un segundo token.
        let e = KadSearchExpression::parse(PROTO, OP, &str_leaf("\"movie x\" b")).unwrap();
        assert!(eval(&e, &entry("the movie x b.mkv", 1, 1, vec![])));
        // eMule matches on substrings: "movie x" is a prefix of "movie xb"
        // and "b" appears in "xb" — both tokens match here.
        assert!(eval(&e, &entry("movie xb.mkv", 1, 1, vec![])));
        // Sin token "b" en absoluto -> sin coincidencia.
        assert!(!eval(&e, &entry("movie x cd.mkv", 1, 1, vec![])));
    }

    #[test]
    fn meta_str_matches_stored_tag_case_insensitive() {
        let e =
            KadSearchExpression::parse(PROTO, OP, &meta_str_leaf(b"codec", "X264")).unwrap();
        let tag = Ed2kTag {
            name: TagName::Text("codec".to_string()),
            value: TagValue::String("x264".to_string()),
        };
        assert!(eval(&e, &entry("f.mkv", 1, 1, vec![tag])));
        assert!(!eval(&e, &entry("f.mkv", 1, 1, vec![])));
    }

    #[test]
    fn meta_str_fileformat_matches_extension() {
        let e =
            KadSearchExpression::parse(PROTO, OP, &meta_str_leaf(&[0x04], "avi")).unwrap();
        assert!(eval(&e, &entry("clip.AVI", 1, 1, vec![])));
        assert!(!eval(&e, &entry("clip.mkv", 1, 1, vec![])));
        assert!(!eval(&e, &entry("noext", 1, 1, vec![])));
    }

    #[test]
    fn num_filesize_virtual_tag() {
        let e = KadSearchExpression::parse(PROTO, OP, &num_leaf(&[0x02], 0x03, 700, false)).unwrap();
        assert!(eval(&e, &entry("f.bin", 700, 1, vec![])));
        assert!(eval(&e, &entry("f.bin", 800, 1, vec![])));
        assert!(!eval(&e, &entry("f.bin", 699, 1, vec![])));
    }

    #[test]
    fn num_sources_stored_tag() {
        let e = KadSearchExpression::parse(PROTO, OP, &num_leaf(&[0x15], 0x01, 5, false)).unwrap();
        let tag = Ed2kTag {
            name: TagName::Id(0x15),
            value: TagValue::UInt32(6),
        };
        assert!(eval(&e, &entry("f.bin", 1, 1, vec![tag])));
        let tag_low = Ed2kTag {
            name: TagName::Id(0x15),
            value: TagValue::UInt32(5),
        };
        assert!(!eval(&e, &entry("f.bin", 1, 1, vec![tag_low])));
    }

    #[test]
    fn num_operators_all() {
        let tag = Ed2kTag {
            name: TagName::Text("bitrate".to_string()),
            value: TagValue::UInt32(100),
        };
        for (wire_op, expect) in [
            (0x00u8, true),  // = 100
            (0x01, false),   // > 100
            (0x02, false),   // < 100
            (0x03, true),    // >= 100
            (0x04, true),    // <= 100
            (0x05, false),   // <> 100
        ] {
            let e = KadSearchExpression::parse(
                PROTO,
                OP,
                &num_leaf(b"bitrate", wire_op, 100, false),
            )
            .unwrap();
            let t = Ed2kTag {
                name: tag.name.clone(),
                value: tag.value.clone(),
            };
            assert_eq!(eval(&e, &entry("f.bin", 1, 1, vec![t])), expect, "op {wire_op}");
        }
    }

    #[test]
    fn num_64bit_value() {
        let e =
            KadSearchExpression::parse(PROTO, OP, &num_leaf(&[0x02], 0x01, u32::MAX as u64, true))
                .unwrap();
        assert!(eval(&e, &entry("big.iso", u64::from(u32::MAX) + 1, 1, vec![])));
        assert!(!eval(&e, &entry("small.bin", 10, 1, vec![])));
    }

    #[test]
    fn boolean_and_or_not() {
        let a = str_leaf("movie");
        let b = str_leaf("x264");
        let c = str_leaf("cam");

        let and = KadSearchExpression::parse(PROTO, OP, &bool_node(0x00, &a, &b)).unwrap();
        let or = KadSearchExpression::parse(PROTO, OP, &bool_node(0x01, &b, &c)).unwrap();
        let not = KadSearchExpression::parse(PROTO, OP, &bool_node(0x02, &a, &c)).unwrap();

        let hit = entry("a.movie.x264.mkv", 1, 1, vec![]);
        let cam = entry("a.movie.cam.mkv", 1, 1, vec![]);
        assert!(eval(&and, &hit));
        assert!(!eval(&and, &cam));
        assert!(eval(&or, &hit));
        assert!(eval(&or, &cam));
        assert!(eval(&not, &hit));
        assert!(!eval(&not, &cam));
    }

    #[test]
    fn nested_expression() {
        // ("movie" AND "x264") OR ("doc" AND "pdf")
        let l = bool_node(0x00, &str_leaf("movie"), &str_leaf("x264"));
        let r = bool_node(0x00, &str_leaf("doc"), &str_leaf("pdf"));
        let e = KadSearchExpression::parse(PROTO, OP, &bool_node(0x01, &l, &r)).unwrap();
        assert!(eval(&e, &entry("a.movie.x264.mkv", 1, 1, vec![])));
        assert!(eval(&e, &entry("my doc file.pdf", 1, 1, vec![])));
        assert!(!eval(&e, &entry("a.movie.avi", 1, 1, vec![])));
    }

    #[test]
    fn malformed_inputs_rejected() {
        // Empty.
        assert!(KadSearchExpression::parse(PROTO, OP, &[]).is_err());
        // Unknown node op.
        assert!(KadSearchExpression::parse(PROTO, OP, &[0x7F]).is_err());
        // Unknown bool op.
        assert!(KadSearchExpression::parse(PROTO, OP, &[0x00, 0x09, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00]).is_err());
        // Unknown num op.
        let mut bad = vec![0x03u8, 1, 0, 0, 0, 0x09];
        bad.extend_from_slice(&1u16.to_le_bytes());
        bad.push(0x02);
        assert!(KadSearchExpression::parse(PROTO, OP, &bad).is_err());
        // Truncated string.
        assert!(KadSearchExpression::parse(PROTO, OP, &[0x01, 0x05, 0x00, b'a']).is_err());
        // Truncated tag name.
        assert!(KadSearchExpression::parse(PROTO, OP, &[0x02, 0x01, 0x00, b'x', 0x04, 0x00, 0x04]).is_err());
    }

    #[test]
    fn depth_limit_rejected() {
        // Nodos AND anidados de profundidad 24+ -> error (eMule iLevel >= 24).
        let mut expr = str_leaf("x");
        for _ in 0..MAX_SEARCH_EXPR_DEPTH + 2 {
            expr = bool_node(0x00, &expr.clone(), &str_leaf("y"));
        }
        assert!(KadSearchExpression::parse(PROTO, OP, &expr).is_err());
        // Profundidad de exactamente 23 es aceptada.
        let mut ok = str_leaf("x");
        for _ in 0..22 {
            ok = bool_node(0x00, &ok.clone(), &str_leaf("y"));
        }
        assert!(KadSearchExpression::parse(PROTO, OP, &ok).is_ok());
    }

    #[test]
    fn trailing_bytes_ignored() {
        let mut v = str_leaf("x");
        v.extend_from_slice(&[0xAA, 0xBB]);
        assert!(KadSearchExpression::parse(PROTO, OP, &v).is_ok());
    }

    #[test]
    fn empty_token_list_does_not_match() {
        // A string leaf that yields no tokens (delimiters only) never matches
        // (eMule returns false on empty m_pastr).
        let e = KadSearchExpression::parse(PROTO, OP, &str_leaf("...")).unwrap();
        assert!(!eval(&e, &entry("...file.mkv", 1, 1, vec![])));
    }
}

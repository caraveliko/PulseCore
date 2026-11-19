// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Consultas y resultados de búsqueda para servidores eD2K.

use crate::ed2k::error::{Ed2kEncodeError, Ed2kErrorKind};
use crate::ed2k::limits::Ed2kLimits;
use crate::ed2k::opcodes::OP_SEARCHREQUEST;
use crate::ed2k::protocol::PR_ED2K;
use crate::ed2k::tags::{
    Ed2kTag, TagName, TagValue, TAG_SPECIAL_FILESIZE, TAG_SPECIAL_FILETYPE, TAG_SPECIAL_NAME,
};
use crate::framing::RawPacket;

/// Petición para buscar archivos en un servidor eD2K (`OP_SEARCHREQUEST` = `0x16`).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchQuery {
    /// Cadena de texto de consulta de búsqueda.
    pub term: String,
    /// Filtro opcional de tamaño mínimo de archivo en bytes.
    pub min_size: Option<u64>,
    /// Filtro opcional de tamaño máximo de archivo en bytes.
    pub max_size: Option<u64>,
    /// Filtro opcional de extensión / formato de archivo (p. ej., "mkv", "mp4").
    pub extension: Option<String>,
    /// Filtro opcional de tipo de archivo eD2K (`FT_FILETYPE`, p. ej., "Video").
    pub file_type: Option<String>,
}

impl SearchQuery {
    /// Crea una consulta de búsqueda simple por palabra clave.
    pub fn new(term: impl Into<String>) -> Self {
        Self {
            term: term.into(),
            min_size: None,
            max_size: None,
            extension: None,
            file_type: None,
        }
    }

    /// Codifica la petición de búsqueda en un `RawPacket` como un árbol de consultas canónico de eD2K.
    pub fn encode(&self, limits: &Ed2kLimits) -> Result<RawPacket, Ed2kEncodeError> {
        let term_bytes = self.term.as_bytes();
        if term_bytes.len() > limits.max_string_bytes {
            return Err(Ed2kEncodeError::new(
                OP_SEARCHREQUEST,
                "term",
                Ed2kErrorKind::OversizedString {
                    size: term_bytes.len(),
                    limit: limits.max_string_bytes,
                },
            ));
        }

        // 1. Codificar la hoja del término: 0x01 (EDONKEY_SEARCH_NAME) + longitud u16 + bytes del término
        let mut term_leaf = Vec::with_capacity(3 + term_bytes.len());
        term_leaf.push(0x01);
        term_leaf.extend_from_slice(&(term_bytes.len() as u16).to_le_bytes());
        term_leaf.extend_from_slice(term_bytes);

        // 2. Preparar hojas de filtro
        let mut filter_leaves = Vec::new();

        if let Some(min) = self.min_size {
            let mut leaf = Vec::with_capacity(9);
            leaf.push(0x03);
            let val = (min.min(u32::MAX as u64)) as u32;
            leaf.extend_from_slice(&val.to_le_bytes());
            leaf.push(0x01); // MIN
            leaf.extend_from_slice(&1u16.to_le_bytes());
            leaf.push(TAG_SPECIAL_FILESIZE);
            filter_leaves.push(leaf);
        }

        if let Some(max) = self.max_size {
            let mut leaf = Vec::with_capacity(9);
            leaf.push(0x03);
            let val = (max.min(u32::MAX as u64)) as u32;
            leaf.extend_from_slice(&val.to_le_bytes());
            leaf.push(0x02); // MAX
            leaf.extend_from_slice(&1u16.to_le_bytes());
            leaf.push(TAG_SPECIAL_FILESIZE);
            filter_leaves.push(leaf);
        }

        if let Some(ext) = &self.extension {
            let ext_bytes = ext.as_bytes();
            if ext_bytes.len() > limits.max_string_bytes {
                return Err(Ed2kEncodeError::new(
                    OP_SEARCHREQUEST,
                    "extension",
                    Ed2kErrorKind::OversizedString {
                        size: ext_bytes.len(),
                        limit: limits.max_string_bytes,
                    },
                ));
            }
            let mut leaf = Vec::with_capacity(6 + ext_bytes.len());
            leaf.push(0x02); // META
            leaf.extend_from_slice(&(ext_bytes.len() as u16).to_le_bytes());
            leaf.extend_from_slice(ext_bytes);
            leaf.extend_from_slice(&1u16.to_le_bytes());
            leaf.push(0x04); // EDONKEY_STAG_FORMAT
            filter_leaves.push(leaf);
        }

        if let Some(ft) = self.file_type.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let ft_bytes = ft.as_bytes();
            if ft_bytes.len() > limits.max_string_bytes {
                return Err(Ed2kEncodeError::new(
                    OP_SEARCHREQUEST,
                    "file_type",
                    Ed2kErrorKind::OversizedString {
                        size: ft_bytes.len(),
                        limit: limits.max_string_bytes,
                    },
                ));
            }
            let mut leaf = Vec::with_capacity(6 + ft_bytes.len());
            leaf.push(0x02); // EDONKEY_SEARCH_META
            leaf.extend_from_slice(&(ft_bytes.len() as u16).to_le_bytes());
            leaf.extend_from_slice(ft_bytes);
            leaf.extend_from_slice(&1u16.to_le_bytes());
            leaf.push(TAG_SPECIAL_FILETYPE); // 0x03 = FT_FILETYPE
            filter_leaves.push(leaf);
        }

        // 3. Ensamblar árbol binario en recorrido en preorden:
        let mut current_tree = term_leaf;
        for filter in filter_leaves {
            let mut new_tree = Vec::with_capacity(2 + current_tree.len() + filter.len());
            new_tree.push(0x00); // EDONKEY_SEARCH_BOOL
            new_tree.push(0x00); // EDONKEY_SEARCH_AND
            new_tree.extend_from_slice(&current_tree);
            new_tree.extend_from_slice(&filter);
            current_tree = new_tree;
        }

        let buf = current_tree;
        let payload_len = buf.len() + 1;
        if payload_len > limits.max_frame_bytes {
            return Err(Ed2kEncodeError::new(
                OP_SEARCHREQUEST,
                "search_query",
                Ed2kErrorKind::FrameTooLarge {
                    size: payload_len,
                    limit: limits.max_frame_bytes,
                },
            ));
        }

        Ok(RawPacket::with_vec(PR_ED2K, OP_SEARCHREQUEST, buf))
    }
}

/// Un único resultado de archivo devuelto por la búsqueda del servidor.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResultFile {
    /// Hash de archivo de 16 bytes.
    pub file_hash: [u8; 16],
    /// ID de cliente del publicador o 0.
    pub client_id: u32,
    /// Puerto de cliente del publicador o 0.
    pub client_port: u16,
    /// Etiquetas de archivo (nombre, tamaño, recuento de fuentes, etc.).
    pub tags: Vec<Ed2kTag>,
}

impl SearchResultFile {
    /// Auxiliar para extraer el nombre de archivo de las etiquetas.
    pub fn name(&self) -> Option<String> {
        crate::ed2k::tags::special_string_tag(&self.tags, TAG_SPECIAL_NAME)
    }

    /// Auxiliar para extraer el tamaño del archivo en bytes de las etiquetas.
    pub fn size(&self) -> Option<u64> {
        self.tags.iter().find_map(|t| match (&t.name, &t.value) {
            (TagName::Id(TAG_SPECIAL_FILESIZE), TagValue::UInt32(v)) => Some(*v as u64),
            (TagName::Id(TAG_SPECIAL_FILESIZE), TagValue::UInt64(v)) => Some(*v),
            _ => None,
        })
    }

    /// Auxiliar para extraer el recuento de fuentes de las etiquetas.
    pub fn source_count(&self) -> Option<u32> {
        self.tags.iter().find_map(|t| {
            let is_source_tag = match &t.name {
                TagName::Id(0x15) | TagName::Id(0x30) => true,
                TagName::Text(txt) => {
                    let lower = txt.to_lowercase();
                    lower.contains("source")
                        || lower.contains("availability")
                        || lower.contains("complete")
                }
                _ => false,
            };
            if is_source_tag {
                match &t.value {
                    TagValue::UInt8(v) => Some(*v as u32),
                    TagValue::UInt16(v) => Some(*v as u32),
                    TagValue::UInt32(v) => Some(*v),
                    TagValue::UInt64(v) => Some(*v as u32),
                    _ => None,
                }
            } else {
                None
            }
        })
    }
}

/// Carga útil del paquete de resultados de búsqueda (`OP_SEARCH_RESULT` = `0x33`).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    /// Lista de archivos coincidentes.
    pub files: Vec<SearchResultFile>,
}

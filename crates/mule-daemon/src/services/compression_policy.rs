// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Política de compresión inteligente por extensión de archivo y capacidades del par.
//!
//! Amplía la exclusión clásica de archivos ya comprimidos en eMule con formatos
//! multimedia y de archivado modernos para ahorrar CPU, eligiendo Zstd cuando ambos
//! pares anuncian soporte `PC26`.

/// Tamaño mínimo de carga útil para compensar la cabecera de compresión (eMule: > 31 bytes).
pub const MIN_COMPRESS_PAYLOAD_BYTES: usize = 32;

/// Algoritmo de compresión seleccionado para un bloque de subida.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionAlgo {
    /// Enviar bloque sin comprimir (`OP_SENDINGPART*`).
    None,
    /// Enviar bloque comprimido con zlib estándar de eMule (`OP_COMPRESSEDPART*`).
    Zlib,
    /// Compresión Zstd acotada para extensiones `PC26` futuras.
    Zstd,
}

/// Extensiones ya comprimidas o de alta entropía que no deben recomprimirse en vuelo.
const INCOMPRESSIBLE_EXTENSIONS: &[&str] = &[
    "zip", "rar", "7z", "cbz", "cbr", "ogm", "ace", "gz", "tgz", "bz2", "tbz2", "xz", "txz",
    "zst", "lz4", "lzma", "mkv", "mp4", "m4v", "mov", "avi", "webm", "wmv", "ts", "m2ts",
    "mp3", "aac", "m4a", "ogg", "oga", "opus", "flac", "wma", "jpg", "jpeg", "png", "webp",
    "avif", "heic", "heif", "gif", "pdf", "docx", "xlsx", "pptx", "epub", "iso", "dmg",
];

/// Devuelve `true` si el nombre de archivo es candidato a compresión antes del envío.
pub fn is_compressible_filename(file_name: &str) -> bool {
    match file_name.rsplit_once('.') {
        Some((_, ext)) => !INCOMPRESSIBLE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()),
        None => true,
    }
}

/// Selecciona el algoritmo óptimo de compresión para un bloque saliente.
pub fn select_compression(
    file_name: &str,
    peer_supports_zlib: bool,
    peer_supports_zstd: bool,
    payload_len: usize,
) -> CompressionAlgo {
    if payload_len < MIN_COMPRESS_PAYLOAD_BYTES || !is_compressible_filename(file_name) {
        return CompressionAlgo::None;
    }
    if peer_supports_zstd {
        CompressionAlgo::Zstd
    } else if peer_supports_zlib {
        CompressionAlgo::Zlib
    } else {
        CompressionAlgo::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_modern_media_and_archives_and_prefers_zstd_when_negotiated() {
        assert!(!is_compressible_filename("pelicula.4k.mkv"));
        assert!(!is_compressible_filename("archivo.tar.zst"));
        assert!(!is_compressible_filename("imagen.avif"));
        assert!(is_compressible_filename("registro_texto.log"));
        assert!(is_compressible_filename("README"));

        assert_eq!(
            select_compression("pelicula.mp4", true, true, 10_240),
            CompressionAlgo::None
        );
        assert_eq!(
            select_compression("volcado.sql", true, false, 10_240),
            CompressionAlgo::Zlib
        );
        assert_eq!(
            select_compression("volcado.sql", true, true, 10_240),
            CompressionAlgo::Zstd
        );
        assert_eq!(
            select_compression("volcado.sql", true, true, 16),
            CompressionAlgo::None
        );
    }
}

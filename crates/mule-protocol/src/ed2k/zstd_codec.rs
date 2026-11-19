// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Códec puro y acotado de compresión Zstd para bloques compatibles con `PC26`.
//!
//! Protege frente a bombas de descompresión deteniendo la lectura inmediatamente
//! cuando los bytes descomprimidos superan `max_out`.

use std::io::{Cursor, Read};

/// Nivel predeterminado de compresión Zstd (equilibrio óptimo velocidad/ratio).
pub const DEFAULT_ZSTD_LEVEL: i32 = 3;
/// Límite predeterminado de descompresión Zstd por bloque (256 KiB).
pub const ZSTD_MAX_DECOMPRESSED_DEFAULT: usize = 256 * 1024;

/// Errores de compresión o descompresión acotada Zstd.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ZstdCodecError {
    /// Error subyacente del codificador/decodificador Zstd.
    #[error("error de códec Zstd: {0}")]
    Io(String),
    /// El flujo descomprimido supera el límite máximo permitido (bomba de descompresión).
    #[error("carga descomprimida Zstd supera el límite de {limit} bytes")]
    DecompressedExceedsLimit {
        /// Límite configurado en bytes.
        limit: usize,
    },
}

/// Comprime `data` usando Zstd al nivel indicado.
pub fn compress_zstd_bounded(data: &[u8], level: i32) -> Result<Vec<u8>, ZstdCodecError> {
    zstd::stream::encode_all(Cursor::new(data), level)
        .map_err(|e| ZstdCodecError::Io(e.to_string()))
}

/// Descomprime `data` con Zstd asegurando que la salida no supere `max_out` bytes.
pub fn decompress_zstd_bounded(data: &[u8], max_out: usize) -> Result<Vec<u8>, ZstdCodecError> {
    let decoder =
        zstd::stream::read::Decoder::new(Cursor::new(data)).map_err(|e| ZstdCodecError::Io(e.to_string()))?;
    let mut limited = decoder.take((max_out as u64).saturating_add(1));
    let mut out = Vec::with_capacity(data.len().min(max_out));
    limited
        .read_to_end(&mut out)
        .map_err(|e| ZstdCodecError::Io(e.to_string()))?;
    if out.len() > max_out {
        return Err(ZstdCodecError::DecompressedExceedsLimit { limit: max_out });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zstd_roundtrip_and_bomb_protection() {
        let payload = b"bloque repetitivo de prueba eMuleRust 2026 ".repeat(200);
        let compressed = compress_zstd_bounded(&payload, DEFAULT_ZSTD_LEVEL).unwrap();
        assert!(compressed.len() < payload.len());

        let restored = decompress_zstd_bounded(&compressed, payload.len()).unwrap();
        assert_eq!(restored, payload);

        let err = decompress_zstd_bounded(&compressed, 100).unwrap_err();
        assert_eq!(err, ZstdCodecError::DecompressedExceedsLimit { limit: 100 });
    }
}

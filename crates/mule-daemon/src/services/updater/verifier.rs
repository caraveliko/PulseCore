// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Verificación criptográfica de integridad y autenticidad para las actualizaciones.

use ring::signature::{UnparsedPublicKey, ED25519};
use sha2::{Digest, Sha256};

use super::error::UpdaterError;

/// Longitud exacta en bytes de una clave pública Ed25519 estándar.
pub const ED25519_PUBLIC_KEY_LEN: usize = 32;

/// Longitud exacta en bytes de una firma Ed25519 estándar.
pub const ED25519_SIGNATURE_LEN: usize = 64;

/// Calcula la suma de comprobación SHA-256 de una secuencia de bytes en formato hexadecimal.
pub fn compute_sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Decodifica una cadena hexadecimal de longitud fija en un vector de bytes.
fn decode_hex_exact(hex_str: &str, expected_len: usize) -> Result<Vec<u8>, String> {
    let clean = hex_str.trim();
    let bytes = hex::decode(clean).map_err(|e| format!("Codificación hexadecimal errónea: {e}"))?;
    if bytes.len() != expected_len {
        return Err(format!(
            "Longitud inesperada: se esperaban {expected_len} bytes, se obtuvieron {}",
            bytes.len()
        ));
    }
    Ok(bytes)
}

/// Valida la integridad (SHA-256) y autenticidad (firma Ed25519) del contenido descargado.
///
/// La firma cubre directamente los bytes del archivo binario descargado.
pub fn verify_payload(
    payload_bytes: &[u8],
    expected_sha256_hex: &str,
    signature_hex: &str,
    public_key_hex: &str,
) -> Result<(), UpdaterError> {
    // 1. Verificación de suma de comprobación SHA-256
    let calculated_sha256 = compute_sha256_hex(payload_bytes);
    if !calculated_sha256.eq_ignore_ascii_case(expected_sha256_hex.trim()) {
        return Err(UpdaterError::ChecksumMismatch {
            calculated: calculated_sha256,
            expected: expected_sha256_hex.trim().to_string(),
        });
    }

    // 2. Parseo de clave pública Ed25519
    let pubkey_bytes = decode_hex_exact(public_key_hex, ED25519_PUBLIC_KEY_LEN)
        .map_err(UpdaterError::InvalidPublicKey)?;

    // 3. Parseo de firma Ed25519
    let signature_bytes = decode_hex_exact(signature_hex, ED25519_SIGNATURE_LEN)
        .map_err(UpdaterError::InvalidSignatureFormat)?;

    // 4. Verificación criptográfica con la primitiva de ring
    let peer_public_key = UnparsedPublicKey::new(&ED25519, &pubkey_bytes);
    peer_public_key
        .verify(payload_bytes, &signature_bytes)
        .map_err(|_| UpdaterError::CryptographicSignatureInvalid)?;

    Ok(())
}

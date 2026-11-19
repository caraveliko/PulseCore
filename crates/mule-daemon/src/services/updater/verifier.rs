// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Verificación criptográfica de integridad y autenticidad para las actualizaciones.

use ring::signature::{UnparsedPublicKey, ED25519};
use sha2::{Digest, Sha256};

use super::constants::{
    ELF_CLASS_64, ELF_DATA_LITTLE_ENDIAN, ELF_MACHINE_AARCH64, ELF_MACHINE_X86_64, ELF_MAGIC,
    MIN_EXECUTABLE_HEADER_BYTES, PE_MAGIC,
};
use super::error::UpdaterError;
use super::manifest::UpdateManifest;

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

/// Verifica el archivo binario y, si se facilita una suma SHA-256 esperada, exige coincidencia exacta.
/// Devuelve la suma SHA-256 calculada en formato hexadecimal en minúsculas.
pub fn verify_optional_sha256(
    payload_bytes: &[u8],
    expected_sha256: Option<&str>,
) -> Result<String, UpdaterError> {
    let calculated = compute_sha256_hex(payload_bytes);
    if let Some(expected) = expected_sha256.map(str::trim).filter(|s| !s.is_empty()) {
        if !calculated.eq_ignore_ascii_case(expected) {
            return Err(UpdaterError::ChecksumMismatch {
                calculated,
                expected: expected.to_string(),
            });
        }
    }
    Ok(calculated)
}

/// Convierte el campo ELF `e_machine` en una etiqueta legible de arquitectura.
fn elf_machine_label(machine: u16) -> String {
    match machine {
        ELF_MACHINE_X86_64 => "x86_64-unknown-linux-gnu".to_string(),
        ELF_MACHINE_AARCH64 => "aarch64-unknown-linux-gnu".to_string(),
        other => format!("ELF e_machine=0x{other:04x}"),
    }
}

/// Valida que el binario recibido tenga cabecera ejecutable compatible con la plataforma objetivo.
pub fn validate_executable_for_triple(
    payload: &[u8],
    expected_triple: &str,
) -> Result<(), UpdaterError> {
    if payload.is_empty() {
        return Err(UpdaterError::EmptyPayload);
    }
    if payload.len() < MIN_EXECUTABLE_HEADER_BYTES {
        return Err(UpdaterError::InvalidExecutableFormat(format!(
            "archivo demasiado corto ({} bytes, mínimo {MIN_EXECUTABLE_HEADER_BYTES})",
            payload.len()
        )));
    }

    if expected_triple.contains("linux") {
        if payload[..2] == PE_MAGIC {
            return Err(UpdaterError::InvalidExecutableFormat(
                "el archivo es un ejecutable PE de Windows (.exe) y este servidor ejecuta Linux"
                    .to_string(),
            ));
        }
        if payload[..4] != ELF_MAGIC {
            return Err(UpdaterError::InvalidExecutableFormat(
                "el archivo no comienza por la firma mágica ELF (0x7F 'ELF')".to_string(),
            ));
        }
        if payload[4] != ELF_CLASS_64 {
            return Err(UpdaterError::InvalidExecutableFormat(format!(
                "solo se admiten binarios ELF de 64 bits (EI_CLASS={}, esperado {ELF_CLASS_64})",
                payload[4]
            )));
        }
        if payload[5] != ELF_DATA_LITTLE_ENDIAN {
            return Err(UpdaterError::InvalidExecutableFormat(format!(
                "codificación ELF no soportada (EI_DATA={}, esperado little-endian {ELF_DATA_LITTLE_ENDIAN})",
                payload[5]
            )));
        }
        let machine = u16::from_le_bytes([payload[18], payload[19]]);
        let expected_machine = match expected_triple {
            "x86_64-unknown-linux-gnu" => Some(ELF_MACHINE_X86_64),
            "aarch64-unknown-linux-gnu" => Some(ELF_MACHINE_AARCH64),
            _ => None,
        };
        if let Some(req_machine) = expected_machine {
            if machine != req_machine {
                return Err(UpdaterError::ArchitectureMismatch {
                    expected: expected_triple.to_string(),
                    actual: elf_machine_label(machine),
                });
            }
        }
        return Ok(());
    }

    if expected_triple.contains("windows") {
        if payload[..4] == ELF_MAGIC {
            return Err(UpdaterError::InvalidExecutableFormat(
                "el archivo es un binario ELF de Linux y este servidor ejecuta Windows".to_string(),
            ));
        }
        if payload[..2] != PE_MAGIC {
            return Err(UpdaterError::InvalidExecutableFormat(
                "el archivo no comienza por la firma mágica PE de Windows ('MZ')".to_string(),
            ));
        }
        return Ok(());
    }

    Err(UpdaterError::UnsupportedPlatform(expected_triple.to_string()))
}

/// Valida que el binario recibido sea ejecutable en la arquitectura y SO actuales del daemon.
pub fn validate_executable_for_current_platform(payload: &[u8]) -> Result<(), UpdaterError> {
    validate_executable_for_triple(payload, UpdateManifest::current_platform_triple())
}

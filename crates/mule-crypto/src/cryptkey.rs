// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Persistencia de `cryptkey.dat` — fichero de clave privada RSA compatible con eMule.
//!
//! Confirmado frente a eMule `ClientCredits.cpp:474` / aMule `ClientCreditsList.cpp:CreateKeyPair`:
//! el fichero contiene la clave privada `InvertibleRSAFunction` de Crypto++, codificada en DER
//! (es decir, PKCS#1 `RSAPrivateKey` = `SEQUENCE { v, n, e, d, p, q, dp, dq, qInv }`)
//! y envuelta en texto Base64. Se toleran saltos de línea durante la lectura.
//!
//! Reglas de seguridad:
//! - Un fichero corrupto nunca se sobrescribe: se pone en cuarentena con un sufijo
//!   de marca temporal antes de cualquier regeneración, preservando la recuperación
//!   histórica de créditos.
//! - Las escrituras son atómicas (fichero temporal + `sync_all` + renombrado).

use std::path::{Path, PathBuf};

use rsa::pkcs1::{DecodeRsaPrivateKey, EncodeRsaPrivateKey};
use rsa::pkcs8::DecodePrivateKey;
use rsa::RsaPrivateKey;

/// Nombre utilizado por eMule/aMule para el archivo de clave privada.
pub const CRYPTKEY_FILENAME: &str = "cryptkey.dat";

#[derive(Debug)]
pub enum LegacyKeyLoad {
    /// Archivo de clave existente analizado correctamente.
    Loaded(Box<RsaPrivateKey>),
    /// No hay ningún archivo de clave presente.
    Missing,
    /// El archivo existe pero no pudo ser analizado; fue movido a cuarentena.
    Invalid {
        quarantine_path: PathBuf,
        reason: CryptkeyError,
    },
    /// El archivo contenía el blob aleatorio de 80 bytes previo a RSA escrito por
    /// versiones antiguas del daemon. No contiene material de clave recuperable;
    /// fue puesto en cuarentena y el llamador debe generar una nueva clave (esto
    /// es una migración, no la corrupción de una clave real).
    LegacyBlob { quarantine_path: PathBuf },
}

#[derive(Debug, thiserror::Error)]
pub enum CryptkeyError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("base64 decode failed: {0}")]
    Base64(#[from] data_encoding::DecodeError),
    #[error("pkcs1 der decode failed: {0}")]
    Pkcs1(#[from] rsa::pkcs1::Error),
    #[error("key validation failed: {0}")]
    Rsa(#[from] rsa::Error),
    #[error("key size is {0} bits, expected {EMULE_RSA_BITS}")]
    WrongKeySize(usize),
}

use crate::legacy_sui::EMULE_RSA_BITS;

/// Decodifica un búfer de bytes de `cryptkey.dat` (PKCS#1 DER envuelto en Base64).
/// Se ignora todo espacio en blanco ASCII dentro del archivo.
pub fn parse_cryptkey_bytes(raw: &[u8]) -> Result<RsaPrivateKey, CryptkeyError> {
    let compact: Vec<u8> = raw
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    let der = data_encoding::BASE64.decode(&compact)?;
    let key = match RsaPrivateKey::from_pkcs1_der(&der) {
        Ok(k) => k,
        Err(pkcs1_err) => match RsaPrivateKey::from_pkcs8_der(&der) {
            Ok(k) => k,
            Err(_) => return Err(CryptkeyError::Pkcs1(pkcs1_err)),
        },
    };
    key.validate()?;
    let bits = rsa::traits::PublicKeyParts::n(&key).bits();
    if bits != EMULE_RSA_BITS {
        return Err(CryptkeyError::WrongKeySize(bits));
    }
    Ok(key)
}

/// Serializa una clave privada en el formato de archivo de eMule: `Base64(PKCS#1 DER)`,
/// dividida en líneas de 64 columnas como la salida de `Base64Encoder` de CryptoPP.
pub fn serialize_cryptkey_bytes(key: &RsaPrivateKey) -> Result<Vec<u8>, CryptkeyError> {
    let der = key.to_pkcs1_der()?;
    let b64 = data_encoding::BASE64.encode(der.as_bytes());
    let mut out = Vec::with_capacity(b64.len() + b64.len() / 64 + 2);
    for chunk in b64.as_bytes().chunks(64) {
        out.extend_from_slice(chunk);
        out.push(b'\n');
    }
    Ok(out)
}

/// Carga `cryptkey.dat` desde `data_dir`.
///
/// - ausente → `Missing` (el llamador decide si generar una nueva)
/// - válido  → `Loaded`
/// - inválido → el archivo se renombra a `cryptkey.dat.corrupt-<unix_ts>` y
///   se devuelve `Invalid`; el llamador no debe regenerar silenciosamente.
pub fn load_cryptkey(data_dir: &Path) -> LegacyKeyLoad {
    let path = data_dir.join(CRYPTKEY_FILENAME);
    let raw = match std::fs::read(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LegacyKeyLoad::Missing,
        Err(e) => {
            let quarantine_path = quarantine_path_with_prefix(&path, CRYPTKEY_FILENAME);
            let _ = std::fs::rename(&path, &quarantine_path);
            return LegacyKeyLoad::Invalid {
                quarantine_path,
                reason: e.into(),
            };
        }
    };
    match parse_cryptkey_bytes(&raw) {
        Ok(key) => LegacyKeyLoad::Loaded(Box::new(key)),
        Err(reason) => {
            let quarantine_path = quarantine_path_with_prefix(&path, CRYPTKEY_FILENAME);
            let _ = std::fs::rename(&path, &quarantine_path);
            if is_legacy_blob(&raw) {
                return LegacyKeyLoad::LegacyBlob { quarantine_path };
            }
            LegacyKeyLoad::Invalid {
                quarantine_path,
                reason,
            }
        }
    }
}

/// Las versiones antiguas del daemon almacenaban un blob aleatorio de 80 bytes en `cryptkey.dat`
/// en lugar de una clave RSA real. Detección: exactamente 80 bytes y sin texto Base64
/// imprimible.
fn is_legacy_blob(raw: &[u8]) -> bool {
    raw.len() == 80
        && !raw.iter().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=') || b.is_ascii_whitespace()
        })
}

/// Escribe atómicamente `cryptkey.dat` en `data_dir`.
pub fn save_cryptkey(data_dir: &Path, key: &RsaPrivateKey) -> Result<(), CryptkeyError> {
    std::fs::create_dir_all(data_dir)?;
    let bytes = serialize_cryptkey_bytes(key)?;
    let target = data_dir.join(CRYPTKEY_FILENAME);
    let tmp = data_dir.join(format!("{CRYPTKEY_FILENAME}.tmp"));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    } // cerrar el descriptor antes de renombrar (requerido en Windows)
    std::fs::rename(&tmp, &target)?;
    Ok(())
}

/// Genera una clave RSA-384 nueva y la persiste atómicamente.
pub fn generate_cryptkey(data_dir: &Path) -> Result<RsaPrivateKey, CryptkeyError> {
    let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, EMULE_RSA_BITS)?;
    save_cryptkey(data_dir, &key)?;
    Ok(key)
}

pub(crate) fn quarantine_path_with_prefix(path: &Path, filename: &str) -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    path.with_file_name(format!("{filename}.corrupt-{ts}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::traits::{PrivateKeyParts, PublicKeyParts};

    #[test]
    fn cryptkey_roundtrip_preserves_key() {
        let dir = tempfile::tempdir().unwrap();
        let key = generate_cryptkey(dir.path()).unwrap();
        match load_cryptkey(dir.path()) {
            LegacyKeyLoad::Loaded(loaded) => {
                assert_eq!(loaded.n(), key.n());
                assert_eq!(loaded.e(), key.e());
                assert_eq!(loaded.d(), key.d());
            }
            _ => panic!("expected Loaded"),
        }
        // El contenido del archivo debe ser texto Base64 (compatible con eMule), no DER sin formato.
        let raw = std::fs::read(dir.path().join(CRYPTKEY_FILENAME)).unwrap();
        assert!(raw.iter().all(|b| b.is_ascii_alphanumeric()
            || matches!(b, b'+' | b'/' | b'=')
            || b.is_ascii_whitespace()));
    }

    #[test]
    fn cryptkey_missing_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(load_cryptkey(dir.path()), LegacyKeyLoad::Missing));
    }

    #[test]
    fn cryptkey_corrupt_is_quarantined_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CRYPTKEY_FILENAME);
        std::fs::write(&path, b"not-base64!!!").unwrap();
        match load_cryptkey(dir.path()) {
            LegacyKeyLoad::Invalid {
                quarantine_path, ..
            } => {
                assert!(quarantine_path.exists());
                assert_eq!(std::fs::read(&quarantine_path).unwrap(), b"not-base64!!!");
                assert!(!path.exists());
            }
            _ => panic!("expected Invalid"),
        }
    }

    #[test]
    fn cryptkey_legacy_blob_detected_and_quarantined() {
        // Las versiones del daemon previas a RSA escribían un blob aleatorio de 80 bytes (no texto).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CRYPTKEY_FILENAME);
        let blob: Vec<u8> = (0u8..80)
            .map(|i| i.wrapping_mul(37).wrapping_add(11))
            .collect();
        assert!(is_legacy_blob(&blob));
        std::fs::write(&path, &blob).unwrap();
        match load_cryptkey(dir.path()) {
            LegacyKeyLoad::LegacyBlob { quarantine_path } => {
                assert!(quarantine_path.exists());
                assert_eq!(std::fs::read(&quarantine_path).unwrap(), blob);
                assert!(!path.exists());
            }
            _ => panic!("expected LegacyBlob"),
        }
    }

    #[test]
    fn cryptkey_80_byte_text_file_is_invalid_not_legacy() {
        // Un archivo de 80 bytes que *es* texto del alfabeto Base64 no es el blob
        // binario heredado; tratar como corrupción ordinaria.
        let raw = vec![b'A'; 80];
        assert!(!is_legacy_blob(&raw));
    }

    #[test]
    fn cryptkey_wrong_size_key_rejected() {
        // Una clave RSA-512 no debe aceptarse como clave SUI de eMule.
        let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 512).unwrap();
        let bytes = serialize_cryptkey_bytes(&key).unwrap();
        let err = parse_cryptkey_bytes(&bytes).unwrap_err();
        assert!(matches!(err, CryptkeyError::WrongKeySize(512)));
    }
}

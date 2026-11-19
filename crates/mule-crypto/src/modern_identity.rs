// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Identidad criptográfica moderna del nodo (Ed25519) para SUI v3 y QUIC P2P.
//!
//! La clave privada PKCS#8 se persiste en `pulse_identity.key` codificada en Base64
//! con escritura atómica y puesta en cuarentena si el archivo existente resulta
//! corrupto, manteniendo la seguridad histórica del repositorio.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};

/// Nombre del archivo de identidad moderna persistido en `data_dir`.
pub const PULSE_IDENTITY_FILENAME: &str = "pulse_identity.key";
/// Longitud en bytes de una clave pública Ed25519 sin comprimir.
pub const ED25519_PUBKEY_LEN: usize = 32;
/// Longitud en bytes de una firma Ed25519.
pub const ED25519_SIG_LEN: usize = 64;
/// Prefijo de separación de dominio para las firmas SUI v3 con vinculación de canal.
pub const SUI_V3_DOMAIN: &[u8] = b"pulsecore-sui-v3:";
/// Longitud de línea usada al serializar Base64 a disco.
const BASE64_LINE_WIDTH: usize = 64;

/// Rol de la parte firmante dentro del saludo criptográfico para evitar ataques de reflexión.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelBindingRole {
    /// El iniciador de la conexión firma el transcript.
    Initiator,
    /// El respondedor de la conexión firma el transcript.
    Responder,
}

impl ChannelBindingRole {
    /// Etiqueta de un byte codificada en la carga útil firmada.
    #[inline]
    pub const fn wire_tag(self) -> u8 {
        match self {
            Self::Initiator => 0x01,
            Self::Responder => 0x02,
        }
    }
}

/// Errores de las operaciones de identidad moderna Ed25519.
#[derive(Debug, thiserror::Error)]
pub enum ModernIdentityError {
    /// Error de E/S al leer o escribir `pulse_identity.key`.
    #[error("error de E/S en identidad moderna: {0}")]
    Io(#[from] std::io::Error),
    /// El texto Base64 almacenado en disco es inválido.
    #[error("error al decodificar Base64 de identidad moderna: {0}")]
    Base64(#[from] data_encoding::DecodeError),
    /// El material PKCS#8 de Ed25519 es inválido o fue rechazado por `ring`.
    #[error("material PKCS#8 Ed25519 inválido")]
    InvalidKeyMaterial,
    /// Fallo al generar entropía o un par de claves Ed25519.
    #[error("error generando par de claves Ed25519")]
    KeyGeneration,
    /// La firma Ed25519 proporcionada no verifica la vinculación de canal.
    #[error("firma Ed25519 inválida para la vinculación de canal")]
    InvalidSignature,
}

/// Resultado de cargar `pulse_identity.key` desde el disco.
#[derive(Debug)]
pub enum ModernIdentityLoad {
    /// Identidad cargada correctamente desde disco.
    Loaded(Box<ModernIdentityKey>),
    /// No existía el archivo de identidad en `data_dir`.
    Missing,
    /// El archivo estaba corrupto y fue puesto en cuarentena sin regenerarse automáticamente.
    Invalid {
        /// Ruta del archivo puesto en cuarentena.
        quarantine_path: PathBuf,
        /// Motivo del fallo.
        reason: ModernIdentityError,
    },
}

/// Par de claves Ed25519 moderno con su codificación PKCS#8 para persistencia y QUIC.
#[derive(Clone)]
pub struct ModernIdentityKey {
    key_pair: Arc<Ed25519KeyPair>,
    pkcs8_bytes: Arc<Vec<u8>>,
    public_key: [u8; ED25519_PUBKEY_LEN],
}

impl fmt::Debug for ModernIdentityKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModernIdentityKey")
            .field("public_key_prefix", &&self.public_key[..4])
            .finish_non_exhaustive()
    }
}

impl ModernIdentityKey {
    /// Genera un par de claves Ed25519 nuevo en memoria utilizando el CSPRNG del sistema.
    pub fn generate() -> Result<Self, ModernIdentityError> {
        let rng = SystemRandom::new();
        let doc = Ed25519KeyPair::generate_pkcs8(&rng)
            .map_err(|_| ModernIdentityError::KeyGeneration)?;
        Self::from_pkcs8(doc.as_ref())
    }

    /// Reconstruye una identidad Ed25519 desde bytes DER PKCS#8.
    pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Self, ModernIdentityError> {
        let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8)
            .map_err(|_| ModernIdentityError::InvalidKeyMaterial)?;
        let pub_slice = key_pair.public_key().as_ref();
        let public_key: [u8; ED25519_PUBKEY_LEN] = pub_slice
            .try_into()
            .map_err(|_| ModernIdentityError::InvalidKeyMaterial)?;
        Ok(Self {
            key_pair: Arc::new(key_pair),
            pkcs8_bytes: Arc::new(pkcs8.to_vec()),
            public_key,
        })
    }

    /// Devuelve la clave pública Ed25519 de 32 bytes.
    #[inline]
    pub fn public_key(&self) -> [u8; ED25519_PUBKEY_LEN] {
        self.public_key
    }

    /// Devuelve los bytes DER PKCS#8 completos (para derivar certificados QUIC efímeros en memoria).
    #[inline]
    pub fn pkcs8_bytes(&self) -> &[u8] {
        &self.pkcs8_bytes
    }

    /// Firma la vinculación de canal SUI v3 sobre `transcript_hash` y el rol indicado.
    pub fn sign_channel_binding(
        &self,
        transcript_hash: &[u8; 32],
        role: ChannelBindingRole,
    ) -> [u8; ED25519_SIG_LEN] {
        let msg = build_channel_binding_message(transcript_hash, role);
        let sig = self.key_pair.sign(&msg);
        let mut out = [0u8; ED25519_SIG_LEN];
        out.copy_from_slice(sig.as_ref());
        out
    }
}

/// Construye el búfer canónico firmado por SUI v3:
/// `"pulsecore-sui-v3:" || role_tag || transcript_hash[32]`.
pub fn build_channel_binding_message(
    transcript_hash: &[u8; 32],
    role: ChannelBindingRole,
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(SUI_V3_DOMAIN.len() + 1 + 32);
    msg.extend_from_slice(SUI_V3_DOMAIN);
    msg.push(role.wire_tag());
    msg.extend_from_slice(transcript_hash);
    msg
}

/// Verifica una firma SUI v3 Ed25519 frente a la clave pública y el transcript de la sesión.
pub fn verify_channel_binding(
    peer_pubkey: &[u8; ED25519_PUBKEY_LEN],
    transcript_hash: &[u8; 32],
    role: ChannelBindingRole,
    signature: &[u8; ED25519_SIG_LEN],
) -> Result<(), ModernIdentityError> {
    let msg = build_channel_binding_message(transcript_hash, role);
    let verifier = UnparsedPublicKey::new(&ED25519, peer_pubkey);
    verifier
        .verify(&msg, signature)
        .map_err(|_| ModernIdentityError::InvalidSignature)
}

/// Serializa el PKCS#8 en texto Base64 dividido en líneas de 64 caracteres.
pub fn serialize_modern_identity(key: &ModernIdentityKey) -> Vec<u8> {
    let b64 = data_encoding::BASE64.encode(key.pkcs8_bytes());
    let mut out = Vec::with_capacity(b64.len() + b64.len() / BASE64_LINE_WIDTH + 2);
    for chunk in b64.as_bytes().chunks(BASE64_LINE_WIDTH) {
        out.extend_from_slice(chunk);
        out.push(b'\n');
    }
    out
}

/// Analiza los bytes de `pulse_identity.key` ignorando espacios en blanco ASCII.
pub fn parse_modern_identity_bytes(raw: &[u8]) -> Result<ModernIdentityKey, ModernIdentityError> {
    let filtered: Vec<u8> = raw
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    let pkcs8 = data_encoding::BASE64.decode(&filtered)?;
    ModernIdentityKey::from_pkcs8(&pkcs8)
}

/// Carga `pulse_identity.key` desde `data_dir` sin sobrescribir archivos corruptos.
pub fn load_modern_identity(data_dir: &Path) -> ModernIdentityLoad {
    let path = data_dir.join(PULSE_IDENTITY_FILENAME);
    let raw = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ModernIdentityLoad::Missing,
        Err(e) => {
            let quarantine_path =
                crate::cryptkey::quarantine_path_with_prefix(&path, PULSE_IDENTITY_FILENAME);
            let _ = std::fs::rename(&path, &quarantine_path);
            return ModernIdentityLoad::Invalid {
                quarantine_path,
                reason: e.into(),
            };
        }
    };
    match parse_modern_identity_bytes(&raw) {
        Ok(key) => ModernIdentityLoad::Loaded(Box::new(key)),
        Err(reason) => {
            let quarantine_path =
                crate::cryptkey::quarantine_path_with_prefix(&path, PULSE_IDENTITY_FILENAME);
            let _ = std::fs::rename(&path, &quarantine_path);
            ModernIdentityLoad::Invalid {
                quarantine_path,
                reason,
            }
        }
    }
}

/// Persiste atómicamente `pulse_identity.key` en `data_dir`.
pub fn save_modern_identity(
    data_dir: &Path,
    key: &ModernIdentityKey,
) -> Result<(), ModernIdentityError> {
    std::fs::create_dir_all(data_dir)?;
    let encoded = serialize_modern_identity(key);
    let target = data_dir.join(PULSE_IDENTITY_FILENAME);
    let tmp = data_dir.join(format!("{PULSE_IDENTITY_FILENAME}.tmp"));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&encoded)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &target)?;
    Ok(())
}

/// Genera una nueva identidad Ed25519 y la guarda atómicamente en `data_dir`.
pub fn generate_and_save_modern_identity(
    data_dir: &Path,
) -> Result<ModernIdentityKey, ModernIdentityError> {
    let key = ModernIdentityKey::generate()?;
    save_modern_identity(data_dir, &key)?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_save_load_and_channel_binding() {
        let dir = tempfile::tempdir().unwrap();
        let key = generate_and_save_modern_identity(dir.path()).unwrap();

        let loaded = match load_modern_identity(dir.path()) {
            ModernIdentityLoad::Loaded(k) => *k,
            other => panic!("se esperaba Loaded, obtenido: {other:?}"),
        };
        assert_eq!(loaded.public_key(), key.public_key());

        let transcript = [0xA5u8; 32];
        let sig = loaded.sign_channel_binding(&transcript, ChannelBindingRole::Initiator);
        verify_channel_binding(
            &loaded.public_key(),
            &transcript,
            ChannelBindingRole::Initiator,
            &sig,
        )
        .expect("la firma válida debe verificarse");

        // Rol distinto o transcript distinto deben fallar.
        assert!(verify_channel_binding(
            &loaded.public_key(),
            &transcript,
            ChannelBindingRole::Responder,
            &sig,
        )
        .is_err());

        let mut bad_transcript = transcript;
        bad_transcript[0] ^= 0x01;
        assert!(verify_channel_binding(
            &loaded.public_key(),
            &bad_transcript,
            ChannelBindingRole::Initiator,
            &sig,
        )
        .is_err());
    }

    #[test]
    fn corrupt_modern_identity_is_quarantined() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PULSE_IDENTITY_FILENAME);
        std::fs::write(&path, b"corrupto!!!").unwrap();

        match load_modern_identity(dir.path()) {
            ModernIdentityLoad::Invalid {
                quarantine_path, ..
            } => {
                assert!(quarantine_path.exists());
                assert!(!path.exists());
            }
            other => panic!("se esperaba Invalid, obtenido: {other:?}"),
        }
    }
}

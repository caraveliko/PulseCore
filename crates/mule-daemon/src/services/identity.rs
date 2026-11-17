// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use md4::{Digest, Md4};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

use mule_crypto::cryptkey::{self, LegacyKeyLoad};
use mule_crypto::legacy_sui::EMULE_RSA_BITS;

/// Contadores SUI compartidos para observabilidad de anomalías — una instancia por daemon,
/// incrementados por el listener entrante y las sesiones de pares salientes, expuestos
/// en modo solo lectura mediante el RPC `identity.status`. Atómicos sin bloqueos, nunca reiniciados.
#[derive(Debug, Default)]
pub struct SuiStats {
    /// Desafíos OP_SECIDENTSTATE enviados a pares entrantes (nosotros los verificamos).
    pub challenges_issued_in: AtomicU64,
    /// Desafíos OP_SECIDENTSTATE enviados salientes (verificamos al par).
    pub challenges_issued_out: AtomicU64,
    /// OP_SIGNATURE enviado hacia afuera respondiendo al desafío de un par.
    pub signatures_sent: AtomicU64,
    /// Firmas de pares que superaron la verificación RSA (entrantes + salientes).
    pub signatures_verified: AtomicU64,
    /// Firmas de pares rechazadas por cualquier motivo (RSA incorrecto, desafío erróneo, ...).
    pub signatures_rejected: AtomicU64,
    /// Rechazos específicamente debidos a la protección anti-inundación de una firma por IP.
    pub signatures_rejected_flood: AtomicU64,
    /// Paquetes SUI malformados o de tamaño excesivo descartados antes de la verificación.
    pub malformed_packets: AtomicU64,
}

/// Instantánea serializable de `SuiStats` para RPC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SuiStatsSnapshot {
    pub challenges_issued_in: u64,
    pub challenges_issued_out: u64,
    pub signatures_sent: u64,
    pub signatures_verified: u64,
    pub signatures_rejected: u64,
    pub signatures_rejected_flood: u64,
    pub malformed_packets: u64,
}

impl SuiStats {
    pub fn snapshot(&self) -> SuiStatsSnapshot {
        use std::sync::atomic::Ordering::Relaxed;
        SuiStatsSnapshot {
            challenges_issued_in: self.challenges_issued_in.load(Relaxed),
            challenges_issued_out: self.challenges_issued_out.load(Relaxed),
            signatures_sent: self.signatures_sent.load(Relaxed),
            signatures_verified: self.signatures_verified.load(Relaxed),
            signatures_rejected: self.signatures_rejected.load(Relaxed),
            signatures_rejected_flood: self.signatures_rejected_flood.load(Relaxed),
            malformed_packets: self.malformed_packets.load(Relaxed),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IdentityStatus {
    pub enabled: bool,
    pub has_key: bool,
    pub pubkey_hash: Option<String>,
    pub key_bits: Option<u32>,
    /// Establecido cuando `cryptkey.dat` era ilegible o estaba corrupto y fue puesto en cuarentena.
    pub quarantined_key: Option<String>,
}

/// `userhash.dat` — `m_abyUserHash` importado de eMule (16 bytes sin procesar), mantenido
/// separado de `cryptkey.dat` al igual que eMule lo mantiene en `preferences.dat`.
const USERHASH_FILENAME: &str = "userhash.dat";

/// Resultado de `import_emule_identity` — migración de `cryptkey.dat` +
/// `preferences.dat` de eMule.
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityImportReport {
    /// `cryptkey.dat` importado (clave privada RSA-384, continuidad de créditos).
    pub key_imported: bool,
    /// Hash de usuario de `preferences.dat` importado y activado.
    pub userhash_imported: bool,
    /// Hash de usuario activo tras la importación (hexadecimal).
    pub userhash: Option<String>,
}

/// Servicio que gestiona la identidad heredada SUI de eMule del daemon.
///
/// La clave privada es una clave RSA-384 real almacenada en `cryptkey.dat`
/// (Base64(PKCS#1 DER), formato de eMule). La forma de cable de la clave pública es la
/// secuencia DER PKCS#1 `RSAPublicKey` (<= 80 bytes) enviada en `OP_PUBLICKEY`.
///
/// La clave privada nunca abandona este servicio: los llamadores solo pueden solicitar
/// firmas mediante `sign_sui_payload`.
#[derive(Clone)]
pub struct IdentityService {
    key_path: PathBuf,
    /// Clave privada RSA-384; `None` si la clave almacenada estaba corrupta (puesta en cuarentena).
    legacy_key: Option<rsa::RsaPrivateKey>,
    /// Codificación DER PKCS#1 de la clave pública (forma en cable).
    public_key: Vec<u8>,
    pubkey_hash: String,
    quarantined_key: Option<String>,
    /// Hash de usuario importado de eMule (`preferences.dat`); cuando está presente tiene
    /// prioridad sobre el hash derivado `MD4(pubkey)` — los créditos de eMule se indexan en él.
    userhash_override: Option<[u8; 16]>,
}

// Debug omite intencionadamente el material de la clave privada.
impl std::fmt::Debug for IdentityService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityService")
            .field("key_path", &self.key_path)
            .field("has_key", &self.legacy_key.is_some())
            .field("pubkey_hash", &self.pubkey_hash)
            .finish()
    }
}

impl IdentityService {
    /// Carga un `cryptkey.dat` existente o genera una nueva clave RSA-384.
    ///
    /// Un archivo de clave corrupto se pone en cuarentena (`cryptkey.dat.corrupt-<ts>`) y el
    /// servicio se ejecuta con SUI deshabilitado en lugar de destruir silenciosamente la
    /// identidad histórica y los créditos del usuario.
    pub fn load_or_generate(data_dir: &Path) -> Self {
        let key_path = data_dir.join(cryptkey::CRYPTKEY_FILENAME);
        let mut quarantined_key = None;

        let legacy_key = match cryptkey::load_cryptkey(data_dir) {
            LegacyKeyLoad::Loaded(key) => {
                tracing::info!("[SUI] Loaded legacy RSA-384 key from {:?}", key_path);
                Some(*key)
            }
            LegacyKeyLoad::Missing => match cryptkey::generate_cryptkey(data_dir) {
                Ok(key) => {
                    tracing::info!("[SUI] Generated new RSA-384 keypair at {:?}", key_path);
                    Some(key)
                }
                Err(e) => {
                    tracing::error!("[SUI] Failed to generate cryptkey.dat: {}", e);
                    None
                }
            },
            LegacyKeyLoad::Invalid {
                quarantine_path,
                reason,
            } => {
                tracing::error!(
                    "[SUI] cryptkey.dat invalid ({}), quarantined to {:?} — SUI disabled, identity NOT regenerated",
                    reason, quarantine_path
                );
                quarantined_key = Some(quarantine_path.display().to_string());
                None
            }
            LegacyKeyLoad::LegacyBlob { quarantine_path } => {
                // Blob de identidad aleatorio de 80 bytes anterior a RSA de una compilación antigua del daemon:
                // no hay material de clave que preservar — seguro de migrar a una clave real.
                quarantined_key = Some(quarantine_path.display().to_string());
                match cryptkey::generate_cryptkey(data_dir) {
                    Ok(key) => {
                        tracing::warn!(
                            "[SUI] Migrated legacy 80-byte identity blob to {:?} — generated new RSA-384 keypair at {:?}",
                            quarantine_path, key_path
                        );
                        Some(key)
                    }
                    Err(e) => {
                        tracing::error!("[SUI] Failed to generate cryptkey.dat: {}", e);
                        None
                    }
                }
            }
        };

        let public_key = legacy_key.as_ref().map(public_key_wire).unwrap_or_default();
        let pubkey_hash = hex::encode(Md4::digest(&public_key));
        let userhash_override = Self::load_userhash_override(data_dir);

        tracing::info!(
            "[SUI] Identity ready (sui={}, pubkey md4={}...)",
            legacy_key.is_some(),
            &pubkey_hash[..16.min(pubkey_hash.len())]
        );

        Self {
            key_path,
            legacy_key,
            public_key,
            pubkey_hash,
            quarantined_key,
            userhash_override,
        }
    }

    /// Lee `data_dir/userhash.dat` (16 bytes sin procesar). Un archivo degenerado o
    /// malformado se ignora con una advertencia — nunca se destruye, el
    /// usuario aún puede conservar una copia de su `preferences.dat` de eMule.
    fn load_userhash_override(data_dir: &Path) -> Option<[u8; 16]> {
        let path = data_dir.join(USERHASH_FILENAME);
        let raw = std::fs::read(&path).ok()?;
        let Ok(bytes) = <[u8; 16]>::try_from(raw.as_slice()) else {
            tracing::warn!(
                "[SUI] {:?} malformed ({} bytes, expected 16) — ignored",
                path,
                raw.len()
            );
            return None;
        };
        if mule_format::met::is_bad_hash(&bytes) {
            tracing::warn!("[SUI] {:?} holds a degenerate user hash — ignored", path);
            return None;
        }
        tracing::info!("[SUI] Imported eMule user hash loaded from {:?}", path);
        Some(bytes)
    }

    /// Genera un `cryptkey.dat` nuevo, reemplazando la clave actual.
    /// Todos los créditos vinculados a la identidad previa se vuelven irreclamables —
    /// incluido un hash de usuario de eMule importado, eliminado junto con la clave antigua.
    pub fn rotate_key(data_dir: &Path) -> std::io::Result<Self> {
        let key_path = data_dir.join(cryptkey::CRYPTKEY_FILENAME);
        let userhash_path = data_dir.join(USERHASH_FILENAME);
        let key = cryptkey::generate_cryptkey(data_dir).map_err(std::io::Error::other)?;
        // Nueva identidad ⇒ el hash de `preferences.dat` importado ya no
        // aplica (pertenece a la clave de eMule que acabamos de reemplazar).
        if userhash_path.exists() {
            let _ = std::fs::remove_file(&userhash_path);
        }
        let public_key = public_key_wire(&key);
        let pubkey_hash = hex::encode(Md4::digest(&public_key));
        tracing::warn!(
            "[SUI] Identity rotated at {:?} (new pubkey md4={}...) — previous credits no longer claimable",
            key_path, &pubkey_hash[..16]
        );
        Ok(Self {
            key_path,
            legacy_key: Some(key),
            public_key,
            pubkey_hash,
            quarantined_key: None,
            userhash_override: None,
        })
    }

    /// Importa una identidad de eMule: `cryptkey.dat` (clave privada RSA-384)
    /// y/o el hash de usuario de `preferences.dat`. Ambos archivos preservan los
    /// créditos que los pares otorgaron a esa identidad — eMule indexa los créditos en
    /// `m_abyUserHash` y los verifica con la clave pública SUI.
    ///
    /// Se requiere `replace` cuando los archivos de destino ya existen —
    /// sobrescribir una identidad descarta los créditos vinculados a ella.
    pub fn import_emule_identity(
        data_dir: &Path,
        cryptkey_src: Option<&Path>,
        preferences_src: Option<&Path>,
        replace: bool,
    ) -> Result<IdentityImportReport, crate::error::DaemonError> {
        use crate::error::DaemonError;
        let key_path = data_dir.join(cryptkey::CRYPTKEY_FILENAME);
        let userhash_path = data_dir.join(USERHASH_FILENAME);

        let mut report = IdentityImportReport::default();

        if let Some(src) = cryptkey_src {
            if key_path.exists() && !replace {
                return Err(DaemonError::Business(
                    "cryptkey.dat already exists — pass replace:true to overwrite the current identity"
                        .to_string(),
                ));
            }
            // Validar antes de tocar nada — el `cryptkey.dat` de eMule es
            // `Base64(PKCS#1 DER)` RSA-384.
            let raw = std::fs::read(src)?;
            let key = cryptkey::parse_cryptkey_bytes(&raw)
                .map_err(|e| DaemonError::Business(format!("cryptkey.dat: {e}")))?;
            cryptkey::save_cryptkey(data_dir, &key)
                .map_err(|e| DaemonError::Business(format!("writing cryptkey.dat: {e}")))?;
            report.key_imported = true;
        }

        if let Some(src) = preferences_src {
            if userhash_path.exists() && !replace {
                return Err(DaemonError::Business(
                    "userhash.dat already exists — pass replace:true to overwrite the imported user hash"
                        .to_string(),
                ));
            }
            let prefs = mule_format::met::PreferencesDat::open(src)
                .map_err(|e| DaemonError::Business(format!("preferences.dat: {e}")))?;
            std::fs::create_dir_all(data_dir)?;
            let tmp = data_dir.join(format!("{USERHASH_FILENAME}.tmp"));
            {
                use std::io::Write;
                let mut f = std::fs::File::create(&tmp)?;
                f.write_all(&prefs.user_hash)?;
                f.sync_all()?;
            }
            std::fs::rename(&tmp, &userhash_path)?;
            report.userhash_imported = true;
            report.userhash = Some(hex::encode(prefs.user_hash));
        }

        Ok(report)
    }

    /// Si hay cargada una clave privada RSA heredada utilizable.
    pub fn has_key(&self) -> bool {
        self.legacy_key.is_some()
    }

    /// Devuelve la ruta al archivo cryptkey almacenado.
    pub fn key_path(&self) -> &Path {
        &self.key_path
    }

    /// Devuelve los bytes de clave pública DER PKCS#1 para la transmisión de `OP_PUBLICKEY` en eD2K.
    pub fn public_key_bytes(&self) -> &[u8] {
        &self.public_key
    }

    /// Devuelve el hash MD4 de la clave pública (32 caracteres hexadecimales).
    pub fn pubkey_hash(&self) -> &str {
        &self.pubkey_hash
    }

    /// Firma una carga útil SUI (`pubkey || challenge [|| ip || kind]`) con la
    /// clave heredada RSA-384 (RSASSA-PKCS1v15-SHA1). Falla si no hay clave cargada.
    pub fn sign_sui_payload(
        &self,
        payload: &[u8],
    ) -> Result<Vec<u8>, mule_crypto::legacy_sui::LegacySuiError> {
        let key = self.legacy_key.as_ref().ok_or_else(|| {
            mule_crypto::legacy_sui::LegacySuiError::Rsa(rsa::Error::Verification)
        })?;
        mule_crypto::legacy_sui::sign_sui_payload(key, payload)
    }

    /// Devuelve el hash de usuario de 16 bytes.
    /// Orden (`Preferences.cpp:1484-1492`): el hash de `preferences.dat` importado
    /// de eMule prevalece literalmente — ya porta los bytes marcadores y es
    /// la clave bajo la cual los pares almacenan los créditos.
    /// En caso contrario, el hash `MD4(pubkey)` derivado marcado con los bytes mágicos
    /// de eMule (una clave rotada desvincula los créditos).
    pub fn user_hash(&self) -> [u8; 16] {
        if let Some(imported) = self.userhash_override {
            return imported;
        }
        let hash = Md4::digest(&self.public_key);
        let mut out = [0u8; 16];
        out.copy_from_slice(&hash[..16]);
        // Bytes mágicos oficiales de identificación de cliente eMule (Preferences.cpp:1491-1492)
        out[5] = 14; // 0x0E
        out[14] = 111; // 0x6F
        out
    }

    /// Devuelve el objeto de estado público de identidad.
    pub fn status(&self) -> IdentityStatus {
        IdentityStatus {
            enabled: self.legacy_key.is_some(),
            has_key: self.legacy_key.is_some(),
            pubkey_hash: Some(self.pubkey_hash.clone()),
            key_bits: self.legacy_key.as_ref().map(|_| EMULE_RSA_BITS as u32),
            quarantined_key: self.quarantined_key.clone(),
        }
    }
}

/// DER PKCS#1 (`SEQUENCE { n INTEGER, e INTEGER }`) — mismos bytes que CryptoPP
/// `GetMaterial().Save()` usado para la clave pública de eMule en el cable.
fn public_key_wire(key: &rsa::RsaPrivateKey) -> Vec<u8> {
    use rsa::pkcs1::EncodeRsaPublicKey;
    rsa::RsaPublicKey::from(key)
        .to_pkcs1_der()
        .map(|d| d.as_bytes().to_vec())
        .unwrap_or_default()
}

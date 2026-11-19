// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Acuerdo criptográfico moderno `PC26` (X25519 clásico o híbrido post-cuántico
//! X25519 + ML-KEM-768 con HKDF-SHA256 y registros AEAD ChaCha20-Poly1305).
//!
//! Este módulo implementa únicamente la máquina de estados y el cifrado puro en
//! memoria sin realizar ninguna operación de E/S de red.

use ml_kem::kem::{Decapsulate, Encapsulate, Kem};
use ml_kem::{Ciphertext, DecapsulationKey768, EncapsulationKey768, KeyExport, MlKem768};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
use ring::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey, X25519};
use ring::hkdf::{KeyType, Salt, HKDF_SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};

/// Versión del protocolo criptográfico de saludo `PC26`.
pub const PC26_HANDSHAKE_VERSION: u8 = 1;
/// Suite clásica con PFS y AEAD: X25519 + HKDF-SHA256 + ChaCha20-Poly1305.
pub const PC26_SUITE_X25519_CHACHA: u8 = 0x10;
/// Suite híbrida post-cuántica: X25519 + ML-KEM-768 (FIPS 203) + HKDF-SHA256 + ChaCha20-Poly1305.
pub const PC26_SUITE_HYBRID_MLKEM768_CHACHA: u8 = 0x11;
/// Tamaño de una clave pública efímera X25519.
pub const PC26_X25519_PUB_LEN: usize = 32;
/// Tamaño del nonce aleatorio incluido en cada saludo.
pub const PC26_NONCE_LEN: usize = 16;
/// Tamaño en bytes de la clave pública (encapsulación) ML-KEM-768.
pub const PC26_MLKEM768_EK_LEN: usize = 1184;
/// Tamaño en bytes del texto cifrado KEM de ML-KEM-768.
pub const PC26_MLKEM768_CT_LEN: usize = 1088;
/// Tamaño de clave simétrica ChaCha20-Poly1305.
pub const PC26_AEAD_KEY_LEN: usize = 32;
/// Tamaño de etiqueta de autenticación Poly1305.
pub const PC26_AEAD_TAG_LEN: usize = 16;
/// Tamaño máximo de texto plano por registro AEAD (16 KiB).
pub const PC26_MAX_RECORD_PLAINTEXT: usize = 16_384;
/// Identificador de dirección iniciador → respondedor para el AAD y las claves.
pub const PC26_DIR_INIT_TO_RESP: u8 = 0x01;
/// Identificador de dirección respondedor → iniciador para el AAD y las claves.
pub const PC26_DIR_RESP_TO_INIT: u8 = 0x02;

const CLIENT_HELLO_BASE_LEN: usize = 1 + 1 + PC26_X25519_PUB_LEN + PC26_NONCE_LEN;
const SERVER_HELLO_BASE_LEN: usize = 1 + 1 + PC26_X25519_PUB_LEN + PC26_NONCE_LEN;
const PC26_HKDF_INFO_INIT: &[u8] = b"pc26-init-to-resp-v1";
const PC26_HKDF_INFO_RESP: &[u8] = b"pc26-resp-to-init-v1";

/// Suite criptográfica negociable en `PC26`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pc26Suite {
    /// X25519 + ChaCha20-Poly1305.
    X25519ChaCha20Poly1305,
    /// X25519 + ML-KEM-768 + ChaCha20-Poly1305.
    HybridMlKem768ChaCha20Poly1305,
}

impl Pc26Suite {
    /// Identificador de suite en el cable.
    #[inline]
    pub const fn wire_id(self) -> u8 {
        match self {
            Self::X25519ChaCha20Poly1305 => PC26_SUITE_X25519_CHACHA,
            Self::HybridMlKem768ChaCha20Poly1305 => PC26_SUITE_HYBRID_MLKEM768_CHACHA,
        }
    }

    /// Analiza un identificador de suite recibido del cable.
    pub fn from_wire_id(id: u8) -> Result<Self, Pc26Error> {
        match id {
            PC26_SUITE_X25519_CHACHA => Ok(Self::X25519ChaCha20Poly1305),
            PC26_SUITE_HYBRID_MLKEM768_CHACHA => Ok(Self::HybridMlKem768ChaCha20Poly1305),
            other => Err(Pc26Error::UnsupportedSuite(other)),
        }
    }
}

/// Errores del protocolo de intercambio y cifrado `PC26`.
#[derive(Debug, thiserror::Error)]
pub enum Pc26Error {
    /// Error de formato o longitud en un mensaje del saludo.
    #[error("formato de mensaje PC26 inválido: {0}")]
    InvalidWireFormat(&'static str),
    /// Versión de saludo `PC26` no compatible.
    #[error("versión PC26 no admitida: {0}")]
    UnsupportedVersion(u8),
    /// Suite criptográfica desconocida o no compatible.
    #[error("suite PC26 no compatible: 0x{0:02X}")]
    UnsupportedSuite(u8),
    /// El respondedor intentó cambiar a una suite no ofertada.
    #[error("negociación de suite PC26 incoherente")]
    InvalidSuiteNegotiation,
    /// Error en generación de claves o acuerdo Diffie-Hellman X25519.
    #[error("fallo en intercambio de claves X25519")]
    KeyExchangeFailed,
    /// Clave pública o texto cifrado ML-KEM-768 inválido.
    #[error("material ML-KEM-768 inválido")]
    InvalidMlKemMaterial,
    /// Secreto compartido degenerado (todo ceros).
    #[error("secreto compartido criptográfico degenerado")]
    DegenerateSharedSecret,
    /// Error derivando claves con HKDF-SHA256.
    #[error("error en derivación HKDF-SHA256")]
    HkdfFailed,
    /// Tamaño de registro fuera de límites (`1..=16384`).
    #[error("longitud de registro PC26 fuera de rango: {0}")]
    RecordLengthOutOfBounds(usize),
    /// El contador de nonces AEAD alcanzó su límite de seguridad.
    #[error("espacio de nonces PC26 agotado")]
    NonceExhausted,
    /// Error de cifrado o autenticación AEAD ChaCha20-Poly1305.
    #[error("fallo de autenticación o cifrado AEAD PC26")]
    AeadFailed,
}

/// Claves simétricas derivadas tras completar el saludo `PC26`.
#[derive(Clone)]
pub struct Pc26SessionKeys {
    /// Suite acordada.
    pub suite: Pc26Suite,
    /// Huella SHA-256 de la transcripción completa (`ClientHello || ServerHello`).
    pub transcript_hash: [u8; 32],
    /// Clave AEAD para sentido iniciador → respondedor.
    init_to_resp_key: [u8; PC26_AEAD_KEY_LEN],
    /// Clave AEAD para sentido respondedor → iniciador.
    resp_to_init_key: [u8; PC26_AEAD_KEY_LEN],
}

impl std::fmt::Debug for Pc26SessionKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pc26SessionKeys")
            .field("suite", &self.suite)
            .field("transcript_prefix", &&self.transcript_hash[..4])
            .finish_non_exhaustive()
    }
}

impl Pc26SessionKeys {
    /// Construye el par `(cifrador_envío, cifrador_recepción)` para el iniciador.
    pub fn into_initiator_ciphers(self) -> Result<(Pc26AeadCipher, Pc26AeadCipher), Pc26Error> {
        let send = Pc26AeadCipher::new(self.suite, PC26_DIR_INIT_TO_RESP, &self.init_to_resp_key)?;
        let recv = Pc26AeadCipher::new(self.suite, PC26_DIR_RESP_TO_INIT, &self.resp_to_init_key)?;
        Ok((send, recv))
    }

    /// Construye el par `(cifrador_envío, cifrador_recepción)` para el respondedor.
    pub fn into_responder_ciphers(self) -> Result<(Pc26AeadCipher, Pc26AeadCipher), Pc26Error> {
        let send = Pc26AeadCipher::new(self.suite, PC26_DIR_RESP_TO_INIT, &self.resp_to_init_key)?;
        let recv = Pc26AeadCipher::new(self.suite, PC26_DIR_INIT_TO_RESP, &self.init_to_resp_key)?;
        Ok((send, recv))
    }
}

/// Estado efímero del iniciador `PC26` a la espera de `ServerHello`.
pub struct Pc26Initiator {
    offered_suite: Pc26Suite,
    x25519_priv: EphemeralPrivateKey,
    mlkem_dk: Option<DecapsulationKey768>,
    client_hello_bytes: Vec<u8>,
}

impl Pc26Initiator {
    /// Inicia un saludo `PC26` generando claves efímeras y serializando `ClientHello`.
    pub fn new(suite: Pc26Suite) -> Result<(Self, Vec<u8>), Pc26Error> {
        let rng = SystemRandom::new();
        let x25519_priv = EphemeralPrivateKey::generate(&X25519, &rng)
            .map_err(|_| Pc26Error::KeyExchangeFailed)?;
        let x25519_pub = x25519_priv
            .compute_public_key()
            .map_err(|_| Pc26Error::KeyExchangeFailed)?;

        let mut client_nonce = [0u8; PC26_NONCE_LEN];
        rng.fill(&mut client_nonce)
            .map_err(|_| Pc26Error::KeyExchangeFailed)?;

        let (mlkem_dk, mlkem_ek_bytes) = match suite {
            Pc26Suite::X25519ChaCha20Poly1305 => (None, Vec::new()),
            Pc26Suite::HybridMlKem768ChaCha20Poly1305 => {
                let (dk, ek) = MlKem768::generate_keypair();
                let ek_bytes = ek.to_bytes();
                (Some(dk), ek_bytes.as_slice().to_vec())
            }
        };

        let mut client_hello =
            Vec::with_capacity(CLIENT_HELLO_BASE_LEN + mlkem_ek_bytes.len());
        client_hello.push(PC26_HANDSHAKE_VERSION);
        client_hello.push(suite.wire_id());
        client_hello.extend_from_slice(x25519_pub.as_ref());
        client_hello.extend_from_slice(&client_nonce);
        client_hello.extend_from_slice(&mlkem_ek_bytes);

        Ok((
            Self {
                offered_suite: suite,
                x25519_priv,
                mlkem_dk,
                client_hello_bytes: client_hello.clone(),
            },
            client_hello,
        ))
    }

    /// Completa el acuerdo de claves al recibir `ServerHello` del respondedor.
    pub fn finish(self, server_hello: &[u8]) -> Result<Pc26SessionKeys, Pc26Error> {
        if server_hello.len() < SERVER_HELLO_BASE_LEN {
            return Err(Pc26Error::InvalidWireFormat("ServerHello truncado"));
        }
        let version = server_hello[0];
        if version != PC26_HANDSHAKE_VERSION {
            return Err(Pc26Error::UnsupportedVersion(version));
        }
        let selected_suite = Pc26Suite::from_wire_id(server_hello[1])?;
        let server_x25519_pub =
            &server_hello[2..2 + PC26_X25519_PUB_LEN];
        let rest = &server_hello[SERVER_HELLO_BASE_LEN..];

        match (self.offered_suite, selected_suite) {
            (Pc26Suite::X25519ChaCha20Poly1305, Pc26Suite::HybridMlKem768ChaCha20Poly1305) => {
                return Err(Pc26Error::InvalidSuiteNegotiation);
            }
            (_, Pc26Suite::X25519ChaCha20Poly1305) if !rest.is_empty() => {
                return Err(Pc26Error::InvalidWireFormat(
                    "bytes inesperados en ServerHello clásico",
                ));
            }
            (_, Pc26Suite::HybridMlKem768ChaCha20Poly1305)
                if rest.len() != PC26_MLKEM768_CT_LEN =>
            {
                return Err(Pc26Error::InvalidWireFormat(
                    "longitud de texto cifrado ML-KEM-768 inválida",
                ));
            }
            _ => {}
        }

        let ss_x25519 = agree_x25519(self.x25519_priv, server_x25519_pub)?;
        let mut ikm = Vec::with_capacity(64);
        ikm.extend_from_slice(&ss_x25519);

        if selected_suite == Pc26Suite::HybridMlKem768ChaCha20Poly1305 {
            let dk = self.mlkem_dk.as_ref().ok_or(Pc26Error::InvalidSuiteNegotiation)?;
            let ct = Ciphertext::<MlKem768>::try_from(rest)
                .map_err(|_| Pc26Error::InvalidMlKemMaterial)?;
            let ss_pq = dk.decapsulate(&ct);
            ikm.extend_from_slice(ss_pq.as_slice());
        }

        derive_session_keys(
            selected_suite,
            &self.client_hello_bytes,
            server_hello,
            &ikm,
        )
    }
}

/// Procesador sin estado del lado respondedor para saludos `PC26`.
pub struct Pc26Responder;

impl Pc26Responder {
    /// Acepta un `ClientHello`, genera la respuesta efímera y deriva las claves de sesión.
    pub fn accept(
        client_hello: &[u8],
        allow_pq: bool,
    ) -> Result<(Vec<u8>, Pc26SessionKeys), Pc26Error> {
        if client_hello.len() < CLIENT_HELLO_BASE_LEN {
            return Err(Pc26Error::InvalidWireFormat("ClientHello truncado"));
        }
        let version = client_hello[0];
        if version != PC26_HANDSHAKE_VERSION {
            return Err(Pc26Error::UnsupportedVersion(version));
        }
        let offered_suite = Pc26Suite::from_wire_id(client_hello[1])?;
        let client_x25519_pub =
            &client_hello[2..2 + PC26_X25519_PUB_LEN];
        let ek_slice = &client_hello[CLIENT_HELLO_BASE_LEN..];

        let selected_suite = match offered_suite {
            Pc26Suite::X25519ChaCha20Poly1305 => {
                if !ek_slice.is_empty() {
                    return Err(Pc26Error::InvalidWireFormat(
                        "bytes extra en ClientHello clásico",
                    ));
                }
                Pc26Suite::X25519ChaCha20Poly1305
            }
            Pc26Suite::HybridMlKem768ChaCha20Poly1305 => {
                if ek_slice.len() != PC26_MLKEM768_EK_LEN {
                    return Err(Pc26Error::InvalidWireFormat(
                        "longitud de clave pública ML-KEM-768 inválida",
                    ));
                }
                if allow_pq {
                    Pc26Suite::HybridMlKem768ChaCha20Poly1305
                } else {
                    Pc26Suite::X25519ChaCha20Poly1305
                }
            }
        };

        let rng = SystemRandom::new();
        let x25519_priv = EphemeralPrivateKey::generate(&X25519, &rng)
            .map_err(|_| Pc26Error::KeyExchangeFailed)?;
        let x25519_pub = x25519_priv
            .compute_public_key()
            .map_err(|_| Pc26Error::KeyExchangeFailed)?;
        let ss_x25519 = agree_x25519(x25519_priv, client_x25519_pub)?;

        let mut server_nonce = [0u8; PC26_NONCE_LEN];
        rng.fill(&mut server_nonce)
            .map_err(|_| Pc26Error::KeyExchangeFailed)?;

        let mut ikm = Vec::with_capacity(64);
        ikm.extend_from_slice(&ss_x25519);

        let ct_bytes = if selected_suite == Pc26Suite::HybridMlKem768ChaCha20Poly1305 {
            let ek_key = <&ml_kem::Key<EncapsulationKey768>>::try_from(ek_slice)
                .map_err(|_| Pc26Error::InvalidMlKemMaterial)?;
            let ek = EncapsulationKey768::new(ek_key)
                .map_err(|_| Pc26Error::InvalidMlKemMaterial)?;
            let (ct, ss_pq) = ek.encapsulate();
            ikm.extend_from_slice(ss_pq.as_slice());
            ct.as_slice().to_vec()
        } else {
            Vec::new()
        };

        let mut server_hello = Vec::with_capacity(SERVER_HELLO_BASE_LEN + ct_bytes.len());
        server_hello.push(PC26_HANDSHAKE_VERSION);
        server_hello.push(selected_suite.wire_id());
        server_hello.extend_from_slice(x25519_pub.as_ref());
        server_hello.extend_from_slice(&server_nonce);
        server_hello.extend_from_slice(&ct_bytes);

        let keys = derive_session_keys(selected_suite, client_hello, &server_hello, &ikm)?;
        Ok((server_hello, keys))
    }
}

fn agree_x25519(
    priv_key: EphemeralPrivateKey,
    peer_pub: &[u8],
) -> Result<[u8; 32], Pc26Error> {
    let peer = UnparsedPublicKey::new(&X25519, peer_pub);
    let ss = agreement::agree_ephemeral(priv_key, &peer, |shared| {
        let mut out = [0u8; 32];
        if shared.len() == 32 {
            out.copy_from_slice(shared);
        }
        out
    })
    .map_err(|_| Pc26Error::KeyExchangeFailed)?;
    if ss.iter().all(|&b| b == 0) {
        return Err(Pc26Error::DegenerateSharedSecret);
    }
    Ok(ss)
}

struct AeadKeyLen;

impl KeyType for AeadKeyLen {
    fn len(&self) -> usize {
        PC26_AEAD_KEY_LEN
    }
}

fn derive_session_keys(
    suite: Pc26Suite,
    client_hello: &[u8],
    server_hello: &[u8],
    ikm: &[u8],
) -> Result<Pc26SessionKeys, Pc26Error> {
    let mut hasher = Sha256::new();
    hasher.update(client_hello);
    hasher.update(server_hello);
    let transcript_hash: [u8; 32] = hasher.finalize().into();

    let salt = Salt::new(HKDF_SHA256, &transcript_hash);
    let prk = salt.extract(ikm);

    let mut init_to_resp_key = [0u8; PC26_AEAD_KEY_LEN];
    let mut resp_to_init_key = [0u8; PC26_AEAD_KEY_LEN];
    prk.expand(&[PC26_HKDF_INFO_INIT], AeadKeyLen)
        .and_then(|okm| okm.fill(&mut init_to_resp_key))
        .map_err(|_| Pc26Error::HkdfFailed)?;
    prk.expand(&[PC26_HKDF_INFO_RESP], AeadKeyLen)
        .and_then(|okm| okm.fill(&mut resp_to_init_key))
        .map_err(|_| Pc26Error::HkdfFailed)?;

    Ok(Pc26SessionKeys {
        suite,
        transcript_hash,
        init_to_resp_key,
        resp_to_init_key,
    })
}

/// Cifrador/descifrador de registros AEAD ChaCha20-Poly1305 para un único sentido de flujo.
pub struct Pc26AeadCipher {
    key: LessSafeKey,
    suite: Pc26Suite,
    direction: u8,
    seq: u64,
}

impl Pc26AeadCipher {
    /// Construye un cifrador AEAD para `suite` y sentido (`PC26_DIR_*`).
    pub fn new(
        suite: Pc26Suite,
        direction: u8,
        raw_key: &[u8; PC26_AEAD_KEY_LEN],
    ) -> Result<Self, Pc26Error> {
        let unbound = UnboundKey::new(&CHACHA20_POLY1305, raw_key)
            .map_err(|_| Pc26Error::AeadFailed)?;
        Ok(Self {
            key: LessSafeKey::new(unbound),
            suite,
            direction,
            seq: 0,
        })
    }

    fn next_nonce(&mut self) -> Result<Nonce, Pc26Error> {
        if self.seq == u64::MAX {
            return Err(Pc26Error::NonceExhausted);
        }
        let cur = self.seq;
        self.seq += 1;
        let mut raw = [0u8; 12];
        raw[4..12].copy_from_slice(&cur.to_le_bytes());
        Ok(Nonce::assume_unique_for_key(raw))
    }

    fn record_aad(&self, plaintext_len: u16) -> [u8; 5] {
        let len_bytes = plaintext_len.to_le_bytes();
        [
            PC26_HANDSHAKE_VERSION,
            self.suite.wire_id(),
            self.direction,
            len_bytes[0],
            len_bytes[1],
        ]
    }

    /// Cifra `plaintext` (entre `1` y `PC26_MAX_RECORD_PLAINTEXT` bytes) y devuelve el registro completo:
    /// `[len_u16_le][ciphertext][tag(16)]`.
    pub fn seal_record(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, Pc26Error> {
        if plaintext.is_empty() || plaintext.len() > PC26_MAX_RECORD_PLAINTEXT {
            return Err(Pc26Error::RecordLengthOutOfBounds(plaintext.len()));
        }
        let plain_len_u16 = plaintext.len() as u16;
        let nonce = self.next_nonce()?;
        let aad_bytes = self.record_aad(plain_len_u16);

        let mut out = Vec::with_capacity(2 + plaintext.len() + PC26_AEAD_TAG_LEN);
        out.extend_from_slice(&plain_len_u16.to_le_bytes());
        out.extend_from_slice(plaintext);

        let tag = self
            .key
            .seal_in_place_separate_tag(nonce, Aad::from(&aad_bytes), &mut out[2..])
            .map_err(|_| Pc26Error::AeadFailed)?;
        out.extend_from_slice(tag.as_ref());
        Ok(out)
    }

    /// Descifra y verifica el cuerpo `[ciphertext][tag(16)]` para un registro cuya longitud
    /// declarada es `plaintext_len`.
    pub fn open_record(
        &mut self,
        ciphertext_and_tag: &[u8],
        plaintext_len: u16,
    ) -> Result<Vec<u8>, Pc26Error> {
        let plain_len = plaintext_len as usize;
        if plain_len == 0 || plain_len > PC26_MAX_RECORD_PLAINTEXT {
            return Err(Pc26Error::RecordLengthOutOfBounds(plain_len));
        }
        if ciphertext_and_tag.len() != plain_len + PC26_AEAD_TAG_LEN {
            return Err(Pc26Error::AeadFailed);
        }
        let nonce = self.next_nonce()?;
        let aad_bytes = self.record_aad(plaintext_len);
        let mut buf = ciphertext_and_tag.to_vec();
        let plain = self
            .key
            .open_in_place(nonce, Aad::from(&aad_bytes), &mut buf)
            .map_err(|_| Pc26Error::AeadFailed)?;
        Ok(plain.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_x25519_and_aead() {
        let (init, ch) = Pc26Initiator::new(Pc26Suite::X25519ChaCha20Poly1305).unwrap();
        let (sh, resp_keys) = Pc26Responder::accept(&ch, true).unwrap();
        let init_keys = init.finish(&sh).unwrap();

        assert_eq!(init_keys.suite, Pc26Suite::X25519ChaCha20Poly1305);
        assert_eq!(init_keys.transcript_hash, resp_keys.transcript_hash);

        let (mut i_send, mut i_recv) = init_keys.into_initiator_ciphers().unwrap();
        let (mut r_send, mut r_recv) = resp_keys.into_responder_ciphers().unwrap();

        let rec = i_send.seal_record(b"hola-ed2k-2026").unwrap();
        let plain_len = u16::from_le_bytes([rec[0], rec[1]]);
        let dec = r_recv.open_record(&rec[2..], plain_len).unwrap();
        assert_eq!(dec, b"hola-ed2k-2026");

        let rec2 = r_send.seal_record(b"respuesta-segura").unwrap();
        let plain_len2 = u16::from_le_bytes([rec2[0], rec2[1]]);
        let dec2 = i_recv.open_record(&rec2[2..], plain_len2).unwrap();
        assert_eq!(dec2, b"respuesta-segura");
    }

    #[test]
    fn roundtrip_hybrid_mlkem768_and_tamper_detection() {
        let (init, ch) = Pc26Initiator::new(Pc26Suite::HybridMlKem768ChaCha20Poly1305).unwrap();
        let (sh, resp_keys) = Pc26Responder::accept(&ch, true).unwrap();
        let init_keys = init.finish(&sh).unwrap();

        assert_eq!(
            init_keys.suite,
            Pc26Suite::HybridMlKem768ChaCha20Poly1305
        );
        assert_eq!(init_keys.transcript_hash, resp_keys.transcript_hash);

        let (mut i_send, _) = init_keys.into_initiator_ciphers().unwrap();
        let (_, mut r_recv) = resp_keys.into_responder_ciphers().unwrap();

        let mut rec = i_send.seal_record(b"pq-hybrid-payload").unwrap();
        let plain_len = u16::from_le_bytes([rec[0], rec[1]]);
        // Corromper un bit del texto cifrado/tag debe invalidar el registro.
        rec[5] ^= 0x80;
        assert!(r_recv.open_record(&rec[2..], plain_len).is_err());
    }

    #[test]
    fn hybrid_downgrades_cleanly_when_pq_disabled() {
        let (init, ch) = Pc26Initiator::new(Pc26Suite::HybridMlKem768ChaCha20Poly1305).unwrap();
        let (sh, resp_keys) = Pc26Responder::accept(&ch, false).unwrap();
        let init_keys = init.finish(&sh).unwrap();

        assert_eq!(init_keys.suite, Pc26Suite::X25519ChaCha20Poly1305);
        assert_eq!(init_keys.transcript_hash, resp_keys.transcript_hash);
    }
}

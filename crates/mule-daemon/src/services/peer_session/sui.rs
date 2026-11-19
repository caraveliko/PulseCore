// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Identificación segura de usuario de eMule saliente (lado del firmante).
//!
//! Cuando un cliente remoto nos desafía con `OP_SECIDENTSTATE`, respondemos con
//! nuestra clave pública RSA-384 (si se solicita, state=2) seguida de `OP_SIGNATURE`
//! sobre `peer_pubkey || challenge` (carga útil V1 — véase
//! docs/ed2k/SUI_WIRE_FORMAT.md). La clave pública del par se obtiene de su
//! paquete `OP_PUBLICKEY`; sin ella no se puede construir la carga útil, lo cual
//! coincide con eMule, que requiere la clave del verificador.

use crate::services::peer_session::log_peer_addr;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use mule_crypto::legacy_sui::{
    build_sui_signing_payload, parse_peer_public_key, verify_sui_signature, SecureIdentVersion,
    MAX_PUBKEY_WIRE,
};
use mule_protocol::ed2k::{
    Ed2kDecoder, Ed2kEncoder, Ed2kPublicKey, Ed2kSecIdentState, Ed2kSignature, OP_PUBLICKEY,
    OP_SECIDENTSTATE, OP_SIGNATURE, PR_EMULE,
};

use crate::services::downloads::DownloadService;
use crate::services::obfuscation_stream::ObfuscatedStream;
use crate::services::IdentityService;
use md4::Digest;
use mule_crypto::modern_identity::{verify_channel_binding, ChannelBindingRole};

pub(crate) struct OutboundSui {
    identity: Option<Arc<Mutex<IdentityService>>>,
    /// DER de la clave pública del desafiador (par remoto), aprendida de su OP_PUBLICKEY
    /// o precargada desde caché SQLite.
    peer_pubkey_der: Option<Vec<u8>>,
    /// Solo una firma por conexión (semántica anti-inundación de eMule).
    answered: bool,
    /// `m_bySupportSecIdent` del par anunciado en el apretón de manos (0 = ninguno).
    peer_sec_ident: u8,
    /// Desafío emitido por nosotros al par (rol de verificador), consumido una sola vez.
    challenge_issued: Option<u32>,
    /// Indica si ya enviamos nuestro desafío OP_SECIDENTSTATE.
    challenge_sent: bool,
    /// Desafío que el par nos pidió responder, aparcado hasta que llegue
    /// su OP_PUBLICKEY — los pares de eMule reales pueden enviar primero OP_SECIDENTSTATE.
    pending_challenge: Option<(u8, u32)>,
    /// Contadores de anomalías SUI compartidos (objeto `sui` en identity.status).
    sui_stats: Option<Arc<crate::services::identity::SuiStats>>,
    /// Firma del par verificada — su identidad de clave pública puede acumular
    /// créditos de descarga (eMule solo acredita a clientes criptográficamente verificados).
    peer_verified: bool,
    /// Servicio de descargas para leer/persistir claves SUI cacheadas por `user_hash`.
    downloads: Option<Arc<Mutex<DownloadService>>>,
    /// User hash del par remoto en hexadecimal (cuando se conoce).
    peer_user_hash_hex: Option<String>,
}

impl OutboundSui {
    pub fn new(identity: Option<Arc<Mutex<IdentityService>>>) -> Self {
        Self {
            identity,
            peer_pubkey_der: None,
            answered: false,
            peer_sec_ident: 0,
            challenge_issued: None,
            challenge_sent: false,
            pending_challenge: None,
            sui_stats: None,
            peer_verified: false,
            downloads: None,
            peer_user_hash_hex: None,
        }
    }

    pub fn with_stats(mut self, stats: Arc<crate::services::identity::SuiStats>) -> Self {
        self.sui_stats = Some(stats);
        self
    }

    /// Conecta caché SQLite de claves públicas verificadas por `user_hash`.
    pub fn with_cache(
        mut self,
        downloads: Arc<Mutex<DownloadService>>,
        peer_user_hash: Option<[u8; 16]>,
    ) -> Self {
        self.downloads = Some(downloads);
        self.peer_user_hash_hex = peer_user_hash.map(hex::encode);
        self
    }

    /// Verifica una firma Ed25519 de vinculación de canal SUI v3 sobre `transcript_hash`.
    #[allow(dead_code)]
    pub(crate) fn verify_modern_channel_binding(
        transcript_hash: &[u8; 32],
        peer_ed25519: &[u8; 32],
        sig: &[u8; 64],
        role: ChannelBindingRole,
    ) -> bool {
        verify_channel_binding(peer_ed25519, transcript_hash, role, sig).is_ok()
    }

    fn bump(
        &self,
        f: impl Fn(&crate::services::identity::SuiStats) -> &std::sync::atomic::AtomicU64,
    ) {
        if let Some(s) = &self.sui_stats {
            f(s).fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn set_peer_sec_ident(&mut self, cap: u8) {
        self.peer_sec_ident = cap;
    }

    /// `SendSecIdentStatePacket` / `InfoPacketsReceived` de eMule: si la clave pública
    /// del par ya está cacheada en SQLite solicita únicamente firma (`state=1`),
    /// o clave + firma (`state=2`) en caso contrario.
    pub async fn initiate(
        &mut self,
        target_addr: SocketAddr,
        encoder: &Ed2kEncoder,
        stream: &mut ObfuscatedStream<TcpStream>,
    ) {
        if self.challenge_sent || self.peer_sec_ident == 0 {
            return;
        }
        let has_key = match &self.identity {
            Some(id) => id.lock().await.has_key(),
            None => false,
        };
        if !has_key {
            return;
        }
        if self.peer_pubkey_der.is_none() {
            if let (Some(dl), Some(uh)) = (&self.downloads, &self.peer_user_hash_hex) {
                if let Some(cached) = dl.lock().await.get_verified_peer_pubkey(uh) {
                    if parse_peer_public_key(&cached).is_ok() {
                        self.peer_pubkey_der = Some(cached);
                    }
                }
            }
        }
        let state = if self.peer_pubkey_der.is_some() { 1 } else { 2 };
        let challenge: u32 = rand::random::<u32>().wrapping_add(1);
        match encoder.encode_secident_state(&Ed2kSecIdentState { state, challenge }) {
            Ok(pkt) => {
                let mut out = Vec::with_capacity(6 + pkt.payload.len());
                out.push(pkt.protocol);
                out.extend_from_slice(&(pkt.frame_len() as u32).to_le_bytes());
                out.push(pkt.opcode);
                out.extend_from_slice(&pkt.payload);
                match stream.write_all(&out).await {
                    Ok(()) => {
                        self.challenge_sent = true;
                        self.challenge_issued = Some(challenge);
                        self.bump(|s| &s.challenges_issued_out);
                        tracing::debug!(
                            "[SUI] Sent OP_SECIDENTSTATE challenge to {peer} (state={state})",
                            peer = log_peer_addr(target_addr)
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[SUI] Failed to send OP_SECIDENTSTATE to {peer}: {e}",
                            peer = log_peer_addr(target_addr)
                        )
                    }
                }
            }
            Err(e) => tracing::warn!("[SUI] Failed to encode OP_SECIDENTSTATE: {e}"),
        }
    }

    /// Responde a un desafío del par (rol de firmante en `OP_SECIDENTSTATE`): envía nuestra
    /// `OP_PUBLICKEY` cuando el par la solicitó (state=2), y luego `OP_SIGNATURE`
    /// en formato V1 o V2 según `peer_sec_ident`. Requiere `peer_pubkey_der`.
    async fn answer_challenge(
        &mut self,
        state: u8,
        challenge: u32,
        target_addr: SocketAddr,
        encoder: &Ed2kEncoder,
        stream: &mut ObfuscatedStream<TcpStream>,
    ) {
        let Some(identity) = &self.identity else {
            return;
        };
        let Some(peer_pub) = &self.peer_pubkey_der else {
            return;
        };

        if state == 2 {
            let my_pub = {
                let id = identity.lock().await;
                id.public_key_bytes().to_vec()
            };
            if let Ok(pk_pkt) = encoder.encode_public_key(&Ed2kPublicKey {
                key_bytes: bytes::Bytes::from(my_pub),
            }) {
                let mut out = Vec::new();
                out.push(pk_pkt.protocol);
                out.extend_from_slice(&(pk_pkt.frame_len() as u32).to_le_bytes());
                out.push(pk_pkt.opcode);
                out.extend_from_slice(&pk_pkt.payload);
                if let Err(e) = stream.write_all(&out).await {
                    tracing::warn!(
                        "[SUI] Failed to send OP_PUBLICKEY to {peer}: {e}",
                        peer = log_peer_addr(target_addr)
                    );
                    return;
                }
            }
        }

        let use_v2 = (self.peer_sec_ident & 0x01) == 0 && (self.peer_sec_ident & 0x02) != 0;
        let (v2_kind, payload_res) = if use_v2 {
            let (ip_u32, kind_byte) = match target_addr.ip() {
                std::net::IpAddr::V4(v4) => (u32::from_be_bytes(v4.octets()), 10u8),
                std::net::IpAddr::V6(_) => (0u32, 30u8),
            };
            (
                Some(kind_byte),
                build_sui_signing_payload(
                    SecureIdentVersion::V2,
                    peer_pub,
                    challenge,
                    Some(ip_u32),
                    Some(kind_byte),
                ),
            )
        } else {
            (
                None,
                build_sui_signing_payload(
                    SecureIdentVersion::V1,
                    peer_pub,
                    challenge,
                    None,
                    None,
                ),
            )
        };

        let payload = match payload_res {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    "[SUI] Failed to build signing payload for {peer}: {e}",
                    peer = log_peer_addr(target_addr)
                );
                return;
            }
        };
        let sig = {
            let id = identity.lock().await;
            id.sign_sui_payload(&payload)
        };
        match sig {
            Ok(sig_bytes) => {
                if let Ok(sig_pkt) = encoder.encode_signature(&Ed2kSignature {
                    signature_bytes: bytes::Bytes::from(sig_bytes),
                }) {
                    let extra = usize::from(v2_kind.is_some());
                    let mut out = Vec::with_capacity(6 + sig_pkt.payload.len() + extra);
                    out.push(sig_pkt.protocol);
                    out.extend_from_slice(&((sig_pkt.frame_len() + extra) as u32).to_le_bytes());
                    out.push(sig_pkt.opcode);
                    out.extend_from_slice(&sig_pkt.payload);
                    if let Some(kind) = v2_kind {
                        out.push(kind);
                    }
                    match stream.write_all(&out).await {
                        Ok(()) => {
                            self.answered = true;
                            self.bump(|s| &s.signatures_sent);
                            tracing::info!(
                                "[SUI] Answered challenge from peer {peer} (state={}, {} signature)",
                                state,
                                if v2_kind.is_some() { "V2" } else { "V1" },
                                peer = log_peer_addr(target_addr)
                            );
                        }
                        Err(e) => tracing::warn!(
                            "[SUI] Failed to send OP_SIGNATURE to {peer}: {e}",
                            peer = log_peer_addr(target_addr)
                        ),
                    }
                }
            }
            Err(e) => tracing::warn!(
                "[SUI] Failed to sign challenge from {peer}: {e}",
                peer = log_peer_addr(target_addr)
            ),
        }
    }

    /// Gestiona un paquete SUI del lado saliente. Devuelve true cuando el paquete fue
    /// consumido (los opcodes SUI nunca son fatales para la transferencia).
    pub async fn handle(
        &mut self,
        packet: &mule_protocol::framing::RawPacket,
        target_addr: SocketAddr,
        decoder: &Ed2kDecoder,
        encoder: &Ed2kEncoder,
        stream: &mut ObfuscatedStream<TcpStream>,
    ) -> bool {
        if packet.protocol != PR_EMULE {
            return false;
        }
        match packet.opcode {
            OP_PUBLICKEY => {
                match decoder.decode_public_key(packet.protocol, packet.opcode, &packet.payload, 0)
                {
                    Ok(pk) if pk.key_bytes.len() <= MAX_PUBKEY_WIRE => {
                        tracing::debug!(
                            "[SUI] Peer {peer} sent its public key ({} bytes)",
                            pk.key_bytes.len(),
                            peer = log_peer_addr(target_addr)
                        );
                        self.peer_pubkey_der = Some(pk.key_bytes.to_vec());
                        if let Some((state, challenge)) = self.pending_challenge.take() {
                            self.answer_challenge(state, challenge, target_addr, encoder, stream)
                                .await;
                        }
                    }
                    Ok(pk) => {
                        self.bump(|s| &s.malformed_packets);
                        tracing::warn!(
                            "[SUI] Peer {peer} sent oversized public key ({} bytes), ignoring",
                            pk.key_bytes.len(),
                            peer = log_peer_addr(target_addr)
                        );
                    }
                    Err(e) => {
                        self.bump(|s| &s.malformed_packets);
                        tracing::warn!(
                            "[SUI] Malformed OP_PUBLICKEY from {peer}: {e}",
                            peer = log_peer_addr(target_addr)
                        );
                    }
                }
                true
            }
            OP_SECIDENTSTATE => {
                let st = match decoder.decode_secident_state(
                    packet.protocol,
                    packet.opcode,
                    &packet.payload,
                    0,
                ) {
                    Ok(st) => st,
                    Err(e) => {
                        tracing::warn!(
                            "[SUI] Malformed OP_SECIDENTSTATE from {peer}: {e}",
                            peer = log_peer_addr(target_addr)
                        );
                        return true;
                    }
                };
                if st.state == 0 || self.answered {
                    return true;
                }
                if self.identity.is_none() {
                    tracing::debug!(
                        "[SUI] Peer {peer} challenged us but no local identity key",
                        peer = log_peer_addr(target_addr)
                    );
                    return true;
                }
                if self.peer_pubkey_der.is_none() {
                    // eMule puede enviar el desafío antes de su OP_PUBLICKEY —
                    // aparcarlo y responder cuando llegue la clave.
                    self.pending_challenge = Some((st.state, st.challenge));
                    tracing::debug!(
                        "[SUI] Peer {peer} sent challenge (state={}) before its public key; deferred",
                        st.state
                    , peer = log_peer_addr(target_addr));
                    return true;
                }
                self.answer_challenge(st.state, st.challenge, target_addr, encoder, stream)
                    .await;
                true
            }
            OP_SIGNATURE => {
                // Rol de verificador: el par respondió a nuestro desafío
                // OP_SECIDENTSTATE. Carga útil: `our_pubkey || challenge [|| ip || kind]`.
                let Some(challenge) = self.challenge_issued.take() else {
                    tracing::debug!(
                        "[SUI] Peer {peer} sent OP_SIGNATURE but we issued no challenge",
                        peer = log_peer_addr(target_addr)
                    );
                    return true;
                };
                let Some(identity) = &self.identity else {
                    return true;
                };
                let Some(peer_der) = self.peer_pubkey_der.clone() else {
                    tracing::warn!(
                        "[SUI] Peer {peer} signed but never sent its public key",
                        peer = log_peer_addr(target_addr)
                    );
                    return true;
                };
                let sig = match decoder.decode_signature(
                    packet.protocol,
                    packet.opcode,
                    &packet.payload,
                    0,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        self.bump(|s| &s.malformed_packets);
                        self.bump(|s| &s.signatures_rejected);
                        tracing::warn!(
                            "[SUI] Malformed OP_SIGNATURE from {peer}: {e}",
                            peer = log_peer_addr(target_addr)
                        );
                        return true;
                    }
                };
                // Cable V2: [u8 siglen][sig][u8 kind].
                let sig_len = packet.payload.first().copied().unwrap_or(0) as usize;
                let kind = if packet.payload.len() == sig_len + 2 {
                    Some(packet.payload[sig_len + 1])
                } else {
                    None
                };
                let peer_ip_u32 = match target_addr.ip() {
                    std::net::IpAddr::V4(v4) => u32::from_be_bytes(v4.octets()),
                    _ => 0,
                };
                let local_ip_u32 = match stream.inner().local_addr().map(|a| a.ip()) {
                    Ok(std::net::IpAddr::V4(v4)) => u32::from_be_bytes(v4.octets()),
                    _ => 0,
                };
                let (ip_u32, version) = match kind {
                    Some(20) => (peer_ip_u32, SecureIdentVersion::V2), // LOCALCLIENT: signer's IP
                    Some(10) => (local_ip_u32, SecureIdentVersion::V2), // REMOTECLIENT: our IP
                    Some(30) => (0, SecureIdentVersion::V2),           // NONECLIENT
                    Some(_) => {
                        tracing::warn!(
                            "[SUI] Peer {peer} used invalid IP-binding kind",
                            peer = log_peer_addr(target_addr)
                        );
                        return true;
                    }
                    None => (0, SecureIdentVersion::V1),
                };
                let my_pub = {
                    let id = identity.lock().await;
                    id.public_key_bytes().to_vec()
                };
                let result =
                    build_sui_signing_payload(version, &my_pub, challenge, Some(ip_u32), kind)
                        .and_then(|payload| {
                            parse_peer_public_key(&peer_der).and_then(|pk| {
                                verify_sui_signature(&pk, &payload, &sig.signature_bytes)
                            })
                        });
                match result {
                    Ok(()) => {
                        self.bump(|s| &s.signatures_verified);
                        self.peer_verified = true;
                        if let (Some(dl), Some(uh)) = (&self.downloads, &self.peer_user_hash_hex) {
                            let ph = hex::encode(md4::Md4::digest(&peer_der));
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as i64;
                            let ip_str = target_addr.ip().to_string();
                            let _ = dl.lock().await.upsert_verified_peer_pubkey(
                                uh,
                                &peer_der,
                                &ph,
                                now_ms,
                                Some(&ip_str),
                            );
                        }
                        tracing::info!(
                            "[SUI] Peer {peer} signature VERIFIED ({version:?})",
                            peer = log_peer_addr(target_addr)
                        );
                    }
                    Err(e) => {
                        self.bump(|s| &s.signatures_rejected);
                        tracing::warn!(
                            "[SUI] Peer {peer} signature REJECTED: {e}",
                            peer = log_peer_addr(target_addr)
                        );
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Devuelve la identidad de crédito verificada `(pubkey_md4_hex, pubkey_hex)` del
    /// par remoto, o `None` cuando la firma no fue verificada.
    /// Quienes llaman persisten los bytes descargados bajo esta identidad (eMule acredita
    /// la *clave del cliente*, nunca la IP transitoria).
    pub fn verified_credit_identity(&self) -> Option<(String, String)> {
        if !self.peer_verified {
            return None;
        }
        self.peer_pubkey_der.as_ref().map(|der| {
            (
                hex::encode(<md4::Md4 as md4::Digest>::digest(der)),
                hex::encode(der),
            )
        })
    }
}

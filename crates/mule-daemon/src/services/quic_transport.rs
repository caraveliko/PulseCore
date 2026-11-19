// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Transporte QUIC P2P autenticado mediante Ed25519 (RFC 9000 / TLS 1.3).
//!
//! Permite multiplexar flujos bidireccionales independientes sin bloqueo HOL,
//! autenticando criptográficamente la clave Ed25519 del par a partir de su
//! certificado X.509 efímero generado con la misma clave privada (`pulse_identity.key`).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use mule_crypto::modern_identity::{ModernIdentityKey, ED25519_PUBKEY_LEN};
use mule_network_policy::{ConnectionPurpose, ListenPermit, NetworkPolicyManager, SocketPermit};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ClientConfig, Endpoint, ServerConfig, TransportConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

/// Protocolo ALPN negociado para sesiones QUIC entre nodos eMuleRust `PC26`.
pub const QUIC_ALPN_PC26: &[u8] = b"pulse-ed2k-2026";
/// Máximo de flujos bidireccionales simultáneos por conexión QUIC entre pares.
const QUIC_MAX_CONCURRENT_BIDI_STREAMS: u32 = 64;
/// Tiempo máximo de inactividad tolerado en una conexión QUIC P2P.
const QUIC_MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Tiempo límite para establecer una conexión QUIC saliente con un par.
const QUIC_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
/// Prefijo canónico ASN.1 DER de `SubjectPublicKeyInfo` para Ed25519 (RFC 8410).
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2A, 0x30, 0x05, 0x06, 0x03, 0x2B, 0x65, 0x70, 0x03, 0x21, 0x00,
];

fn parse_der_tlv(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *input.first()?;
    let len_byte = *input.get(1)?;
    let (len, hdr_len) = if len_byte < 0x80 {
        (usize::from(len_byte), 2)
    } else {
        let count = usize::from(len_byte & 0x7F);
        if count == 0 || count > 4 || input.len() < 2 + count {
            return None;
        }
        let mut val = 0usize;
        for &b in &input[2..2 + count] {
            val = (val << 8) | usize::from(b);
        }
        (val, 2 + count)
    };
    let end = hdr_len.checked_add(len)?;
    if end > input.len() {
        return None;
    }
    Some((tag, &input[hdr_len..end], &input[end..]))
}

/// Extrae exactamente los 32 bytes de clave pública Ed25519 desde el campo
/// `subjectPublicKeyInfo` canónico de un certificado X.509 DER (sin búsqueda difusa).
pub fn extract_ed25519_spki_from_cert(cert_der: &[u8]) -> Option<[u8; ED25519_PUBKEY_LEN]> {
    let (cert_tag, cert_body, _) = parse_der_tlv(cert_der)?;
    if cert_tag != 0x30 {
        return None;
    }
    let (tbs_tag, mut tbs, _) = parse_der_tlv(cert_body)?;
    if tbs_tag != 0x30 {
        return None;
    }
    // Saltar `[0] EXPLICIT Version` opcional.
    if tbs.first().copied() == Some(0xA0) {
        let (_, _, rest) = parse_der_tlv(tbs)?;
        tbs = rest;
    }
    // Saltar serialNumber (0x02), signature (0x30), issuer (0x30), validity (0x30), subject (0x30).
    for expected_tag in [0x02u8, 0x30, 0x30, 0x30, 0x30] {
        let (tag, _, rest) = parse_der_tlv(tbs)?;
        if tag != expected_tag {
            return None;
        }
        tbs = rest;
    }
    // El siguiente campo es `subjectPublicKeyInfo` (`SEQUENCE`, 44 bytes totales).
    let (spki_tag, spki_body, _) = parse_der_tlv(tbs)?;
    if spki_tag != 0x30 || spki_body.len() != 42 {
        return None;
    }
    let expected_body = &ED25519_SPKI_PREFIX[2..];
    if spki_body[..expected_body.len()] != *expected_body {
        return None;
    }
    spki_body[expected_body.len()..].try_into().ok()
}

#[derive(Debug)]
struct Ed25519PeerCertVerifier {
    expected_pubkey: Option<[u8; ED25519_PUBKEY_LEN]>,
}

impl ServerCertVerifier for Ed25519PeerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let Some(peer_key) = extract_ed25519_spki_from_cert(end_entity.as_ref()) else {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::BadEncoding,
            ));
        };
        if let Some(expected) = self.expected_pubkey {
            if peer_key != expected {
                return Err(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::ApplicationVerificationFailure,
                ));
            }
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::InvalidCertificate(
            rustls::CertificateError::BadSignature,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        if dss.scheme != SignatureScheme::ED25519 {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::BadSignature,
            ));
        }
        let Some(pubkey) = extract_ed25519_spki_from_cert(cert.as_ref()) else {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::BadEncoding,
            ));
        };
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &pubkey)
            .verify(message, dss.signature())
            .map(|()| HandshakeSignatureValid::assertion())
            .map_err(|_| {
                rustls::Error::InvalidCertificate(rustls::CertificateError::BadSignature)
            })
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

fn build_ephemeral_cert(
    identity: &ModernIdentityKey,
) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), String> {
    let pkcs8 = PrivatePkcs8KeyDer::from(identity.pkcs8_bytes().to_vec());
    let key_pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&pkcs8, &rcgen::PKCS_ED25519)
        .map_err(|e| format!("Error importando clave Ed25519 en rcgen: {e}"))?;
    let mut params = rcgen::CertificateParams::new(vec!["pulse-core.local".to_string()])
        .map_err(|e| format!("Error creando parámetros de certificado QUIC: {e}"))?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("Error firmando certificado X.509 Ed25519: {e}"))?;
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.pkcs8_bytes().to_vec()));
    Ok((cert.der().clone(), key_der))
}

fn build_transport_config() -> Arc<TransportConfig> {
    let mut cfg = TransportConfig::default();
    cfg.max_concurrent_bidi_streams(QUIC_MAX_CONCURRENT_BIDI_STREAMS.into());
    cfg.max_concurrent_uni_streams(QUIC_MAX_CONCURRENT_BIDI_STREAMS.into());
    if let Ok(idle) = QUIC_MAX_IDLE_TIMEOUT.try_into() {
        cfg.max_idle_timeout(Some(idle));
    }
    Arc::new(cfg)
}

fn bind_udp_socket_dual_stack(addr: SocketAddr) -> Result<std::net::UdpSocket, String> {
    if addr.is_ipv6() && addr.ip().is_unspecified() {
        let sock = socket2::Socket::new(
            socket2::Domain::IPV6,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )
        .map_err(|e| format!("Error creando socket UDP IPv6 dual-stack: {e}"))?;
        let _ = sock.set_only_v6(false);
        sock.set_nonblocking(true)
            .map_err(|e| format!("Error configurando no-bloqueo UDP QUIC: {e}"))?;
        sock.bind(&socket2::SockAddr::from(addr))
            .map_err(|e| format!("Error enlazando socket UDP QUIC en {addr}: {e}"))?;
        Ok(sock.into())
    } else {
        let sock = std::net::UdpSocket::bind(addr)
            .map_err(|e| format!("Error enlazando socket UDP QUIC en {addr}: {e}"))?;
        sock.set_nonblocking(true)
            .map_err(|e| format!("Error configurando no-bloqueo UDP QUIC: {e}"))?;
        Ok(sock)
    }
}

/// Conexión QUIC saliente autenticada vinculada a su permiso de política de red.
pub struct QuicPeerConnection {
    /// Conexión QUIC subyacente para abrir flujos multiplexados sin bloqueo HOL.
    pub conn: quinn::Connection,
    _permit: SocketPermit,
}

impl std::fmt::Debug for QuicPeerConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuicPeerConnection")
            .field("remote_addr", &self.conn.remote_address())
            .finish_non_exhaustive()
    }
}

impl std::ops::Deref for QuicPeerConnection {
    type Target = quinn::Connection;

    fn deref(&self) -> &Self::Target {
        &self.conn
    }
}

/// Transporte QUIC P2P autenticado por identidad Ed25519 para conexiones de pares `PC26`.
#[derive(Clone)]
pub struct QuicPeerTransport {
    endpoint: Endpoint,
    local_addr: SocketAddr,
    policy: Arc<NetworkPolicyManager>,
    crypto_provider: Arc<rustls::crypto::CryptoProvider>,
    transport_cfg: Arc<TransportConfig>,
    _listen_permit: Arc<ListenPermit>,
}

impl QuicPeerTransport {
    /// Enlaza un extremo QUIC P2P respetando `NetworkPolicyManager`.
    pub fn bind(
        bind_addr: SocketAddr,
        identity: &ModernIdentityKey,
        policy: Arc<NetworkPolicyManager>,
    ) -> Result<Self, String> {
        let listen_permit = policy
            .request_tcp_listen(bind_addr, ConnectionPurpose::KadNode)
            .map_err(|e| format!("Política de red rechazó enlace QUIC en {bind_addr}: {e}"))?;

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let (cert, key_der) = build_ephemeral_cert(identity)?;

        let mut server_crypto = rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| format!("Error configurando TLS 1.3 en servidor QUIC: {e}"))?
            .with_no_client_auth()
            .with_single_cert(vec![cert], key_der)
            .map_err(|e| format!("Error asignando certificado QUIC: {e}"))?;
        server_crypto.alpn_protocols = vec![QUIC_ALPN_PC26.to_vec()];

        let quic_server_crypto = QuicServerConfig::try_from(server_crypto)
            .map_err(|e| format!("Error adaptando configuración QUIC servidor: {e}"))?;
        let transport_cfg = build_transport_config();
        let mut server_config = ServerConfig::with_crypto(Arc::new(quic_server_crypto));
        server_config.transport_config(Arc::clone(&transport_cfg));

        let udp_sock = bind_udp_socket_dual_stack(bind_addr)?;
        let runtime = Arc::new(quinn::TokioRuntime);
        let endpoint = Endpoint::new(
            Default::default(),
            Some(server_config),
            udp_sock,
            runtime,
        )
        .map_err(|e| format!("Error inicializando endpoint QUIC: {e}"))?;

        let local_addr = endpoint
            .local_addr()
            .map_err(|e| format!("Error obteniendo dirección local QUIC: {e}"))?;

        Ok(Self {
            endpoint,
            local_addr,
            policy,
            crypto_provider: provider,
            transport_cfg,
            _listen_permit: Arc::new(listen_permit),
        })
    }

    /// Devuelve la dirección UDP local efectivamente asignada al endpoint QUIC.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Acepta una conexión QUIC entrante verificando protocolo ALPN.
    pub async fn accept(&self) -> Option<quinn::Connection> {
        let incoming = self.endpoint.accept().await?;
        incoming.await.ok()
    }

    /// Conecta por QUIC contra `target` autenticando criptográficamente la clave Ed25519
    /// esperada (`expected_peer_pubkey`) si fue anunciada por el par.
    pub async fn connect_peer(
        &self,
        target: SocketAddr,
        expected_peer_pubkey: Option<[u8; ED25519_PUBKEY_LEN]>,
    ) -> Result<QuicPeerConnection, String> {
        let permit = self
            .policy
            .request_tcp_connect(target, ConnectionPurpose::Ed2kPeer)
            .map_err(|e| format!("Política de red rechazó conexión QUIC a {target}: {e}"))?;

        let verifier = Arc::new(Ed25519PeerCertVerifier {
            expected_pubkey: expected_peer_pubkey,
        });
        let mut client_crypto =
            rustls::ClientConfig::builder_with_provider(Arc::clone(&self.crypto_provider))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .map_err(|e| format!("Error TLS 1.3 cliente QUIC: {e}"))?
                .dangerous()
                .with_custom_certificate_verifier(verifier)
                .with_no_client_auth();
        client_crypto.alpn_protocols = vec![QUIC_ALPN_PC26.to_vec()];

        let quic_client_crypto = QuicClientConfig::try_from(client_crypto)
            .map_err(|e| format!("Error adaptando configuración QUIC cliente: {e}"))?;
        let mut client_cfg = ClientConfig::new(Arc::new(quic_client_crypto));
        client_cfg.transport_config(Arc::clone(&self.transport_cfg));

        let connecting = self
            .endpoint
            .connect_with(client_cfg, target, "pulse-core.local")
            .map_err(|e| format!("Error iniciando conexión QUIC hacia {target}: {e}"))?;

        let cancel = self.policy.cancellation_token();
        tokio::select! {
            _ = cancel.cancelled() => Err("Conexión QUIC abortada por interruptor de corte".to_string()),
            res = tokio::time::timeout(QUIC_CONNECT_TIMEOUT, connecting) => {
                match res {
                    Ok(Ok(conn)) => Ok(QuicPeerConnection {
                        conn,
                        _permit: permit,
                    }),
                    Ok(Err(e)) => Err(format!("Fallo en saludo QUIC con {target}: {e}")),
                    Err(_) => Err(format!("Tiempo de espera agotado en saludo QUIC con {target}")),
                }
            }
        }
    }

    /// Cierra limpiamente todas las conexiones QUIC activas.
    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"shutdown");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mule_network_policy::{NetworkProfile, ResourceBudgets};

    fn test_policy() -> Arc<NetworkPolicyManager> {
        Arc::new(NetworkPolicyManager::new(
            NetworkProfile::LocalLab,
            ResourceBudgets::default(),
        ))
    }

    #[tokio::test]
    async fn quic_multiplexes_streams_with_verified_ed25519_identity() {
        let policy = test_policy();
        let id_server = ModernIdentityKey::generate().unwrap();
        let id_client = ModernIdentityKey::generate().unwrap();

        let server = QuicPeerTransport::bind(
            "127.0.0.1:0".parse().unwrap(),
            &id_server,
            Arc::clone(&policy),
        )
        .unwrap();
        let client = QuicPeerTransport::bind(
            "127.0.0.1:0".parse().unwrap(),
            &id_client,
            Arc::clone(&policy),
        )
        .unwrap();

        let srv_clone = server.clone();
        let srv_task = tokio::spawn(async move {
            let conn = srv_clone.accept().await.unwrap();
            let mut workers = Vec::new();
            for _ in 0..2 {
                let (mut send, mut recv) = conn.accept_bi().await.unwrap();
                workers.push(tokio::spawn(async move {
                    let data = recv.read_to_end(1024).await.unwrap();
                    send.write_all(&data).await.unwrap();
                    send.finish().unwrap();
                    let _ = send.stopped().await;
                }));
            }
            for w in workers {
                w.await.unwrap();
            }
            let _ = conn.closed().await;
        });

        let conn = client
            .connect_peer(server.local_addr(), Some(id_server.public_key()))
            .await
            .unwrap();

        // Dos flujos bidireccionales concurrentes independientes (sin bloqueo HOL).
        let (mut s1, mut r1) = conn.open_bi().await.unwrap();
        let (mut s2, mut r2) = conn.open_bi().await.unwrap();
        s1.write_all(b"stream-control").await.unwrap();
        s1.finish().unwrap();
        s2.write_all(b"stream-data-payload").await.unwrap();
        s2.finish().unwrap();

        let e1 = r1.read_to_end(1024).await.unwrap();
        let e2 = r2.read_to_end(1024).await.unwrap();
        assert_eq!(e1, b"stream-control");
        assert_eq!(e2, b"stream-data-payload");

        drop(conn);
        client.close();
        srv_task.await.unwrap();
        server.close();
    }

    #[tokio::test]
    async fn quic_rejects_mitm_with_mismatched_ed25519_pubkey() {
        let policy = test_policy();
        let real_id = ModernIdentityKey::generate().unwrap();
        let mitm_id = ModernIdentityKey::generate().unwrap();
        let client_id = ModernIdentityKey::generate().unwrap();

        let mitm_server = QuicPeerTransport::bind(
            "127.0.0.1:0".parse().unwrap(),
            &mitm_id,
            Arc::clone(&policy),
        )
        .unwrap();
        let client = QuicPeerTransport::bind(
            "127.0.0.1:0".parse().unwrap(),
            &client_id,
            Arc::clone(&policy),
        )
        .unwrap();

        let srv_clone = mitm_server.clone();
        tokio::spawn(async move {
            let _ = srv_clone.accept().await;
        });

        let err = client
            .connect_peer(mitm_server.local_addr(), Some(real_id.public_key()))
            .await
            .unwrap_err();
        assert!(!err.is_empty());

        mitm_server.close();
        client.close();
    }
}

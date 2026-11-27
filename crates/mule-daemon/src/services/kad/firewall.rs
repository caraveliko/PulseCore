// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Prueba de cortafuegos/alcanzabilidad UDP de Kad (KADEMLIA2_FIREWALLUDP / OP_FWCHECKUDPREQ).
//!
//! Refleja `CUDPFirewallTester` de eMule (UDPFirewallTester.cpp): el nodo local
//! solicita a hasta 2 contactos Kad adecuados mediante una conexión TCP eD2K que envíen
//! datagramas `KADEMLIA2_FIREWALLUDP` a nuestros puertos UDP de Kad anunciados. Un
//! datagrama positivo demuestra que el puerto está abierto; si todos los candidatos fallan, el
//! puerto se considera tras cortafuegos (tiempo de espera global en eMule de ~6 min).

use crate::services::peer_session::log_peer_addr;
use crate::services::peer_session::log_peer_ip;
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use mule_core::kad::{KadContact, KadId};
use mule_network_policy::ConnectionPurpose;
use mule_protocol::ed2k::{
    Ed2kEncoder, Ed2kHello, OP_FWCHECKUDPREQ, OP_HELLO, OP_HELLO_ANSWER, PR_ED2K, PR_EMULE,
};
use mule_protocol::framing::PacketCodec;

use super::KadService;
use crate::services::kad_udp::{FirewallUdpResult, UdpFirewallStatus};

/// Número de contactos consultados por ronda de comprobación (eMule
/// `UDP_FIREWALLTEST_CLIENTSTOASK = 2`).
const UDP_FW_CLIENTS_TO_ASK: usize = 2;
/// Máximo de contactos candidatos examinados por ronda para conseguir hasta
/// `UDP_FW_CLIENTS_TO_ASK` sondeos TCP válidos.
const UDP_FW_CANDIDATE_POOL: usize = 10;
/// Versión mínima de Kad que un candidato debe anunciar para atender una
/// comprobación de cortafuegos UDP (eMule `KADEMLIA_VERSION6_49aBETA`).
const MIN_FW_TEST_KAD_VERSION: u8 = 6;
/// Versión de Kad que anunciamos — eMule `KADEMLIA_VERSION` (0x0a, eMule
/// 0.72a). Incluye la característica v9 (`TAG_KADAICHHASHPUB` /
/// almacenamiento de palabras clave AICH `TAG_KADAICHHASHRESULT`); v10 en sí no añade
/// nuevo comportamiento de red más allá del incremento de identidad.
pub(crate) const OUR_KAD_VERSION: u8 = mule_protocol::kad::KADEMLIA_VERSION_CURRENT;

const FW_TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FW_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Cuánto tiempo esperar el datagrama FIREWALLUDP tras enviar la solicitud.
const FW_UDP_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
/// Presupuesto global de comprobación (eMule: ~6 minutos antes de declarar
/// agotado el tiempo y clasificar como tras cortafuegos).
const FW_OVERALL_TIMEOUT: Duration = Duration::from_secs(360);

/// Clasificación del fallo en una comprobación individual de cortafuegos UDP.
enum CandidateTestError {
    /// Fallo previo al envío de `OP_FWCHECKUDPREQ` (conexión TCP o handshake con el par remoto).
    PreProbe(String),
    /// `OP_FWCHECKUDPREQ` se envió al par, pero no llegó respuesta UDP dentro del tiempo límite.
    ProbeNoResponse(String),
}

/// Resuelve el veredicto de una ronda cuando ningún candidato devolvió un datagrama UDP válido.
fn resolve_fw_round_verdict(probes_completed: usize) -> UdpFirewallStatus {
    if probes_completed > 0 {
        UdpFirewallStatus::Firewalled
    } else {
        UdpFirewallStatus::Unverified
    }
}

impl KadService {
    /// Establece el hash de usuario local empleado en los saludos de conexiones de prueba.
    pub fn set_fw_check_identity(&self, user_hash: [u8; 16]) {
        *self.fw_user_hash.write().unwrap_or_else(|p| p.into_inner()) = user_hash;
    }

    /// Estado actual del cortafuegos UDP (Unverified/Testing/Open/Firewalled).
    pub async fn udp_firewall_status(&self) -> UdpFirewallStatus {
        self.udp.firewall_tracker().lock().await.status
    }

    /// Ejecuta una ronda de comprobación de cortafuegos UDP contra contactos de enrutamiento adecuados.
    /// Devuelve el estado resultante. Sin efecto si ya hay una comprobación en curso o
    /// si el socket UDP está inactivo.
    pub async fn run_udp_firewall_check(&self) -> UdpFirewallStatus {
        let tracker = self.udp.firewall_tracker();

        // Marcar en pruebas (o salir si ya hay una ronda en curso).
        {
            let mut fw = tracker.lock().await;
            if fw.status == UdpFirewallStatus::Testing {
                return fw.status;
            }
            fw.status = UdpFirewallStatus::Testing;
        }

        let result =
            match tokio::time::timeout(FW_OVERALL_TIMEOUT, self.fw_check_round(&tracker)).await {
                Ok(st) => st,
                Err(_) => {
                    tracing::warn!(
                        "Kad UDP firewall check timed out after {:?} — declaring firewalled",
                        FW_OVERALL_TIMEOUT
                    );
                    UdpFirewallStatus::Firewalled
                }
            };

        let mut fw = tracker.lock().await;
        fw.status = result;
        if result == UdpFirewallStatus::Open || result == UdpFirewallStatus::Firewalled {
            fw.verified = true;
            fw.last_check = Some(std::time::Instant::now());
        }
        result
    }

    async fn fw_check_round(
        &self,
        tracker: &Arc<tokio::sync::Mutex<crate::services::kad_udp::FirewallTestTracker>>,
    ) -> UdpFirewallStatus {
        let intern_port = match self.udp.local_udp_port().await {
            Some(p) if p != 0 => p,
            _ => {
                tracing::debug!("Kad UDP firewall check skipped: UDP socket not bound");
                return UdpFirewallStatus::Unverified;
            }
        };
        let extern_port = tracker.lock().await.extern_port;

        // Selección de candidatos (FindNodeFWCheckUDP de eMule + reglas de idoneidad):
        // contactos verificados, versión Kad >= 6, puerto TCP conocido, IPs distintas.
        let candidates: Vec<KadContact> = {
            let table = self.routing_table.lock().await;
            let mut seen = HashSet::new();
            table
                .find_closest(&KadId::random(), 20)
                .into_iter()
                .filter(|c| {
                    c.verified
                        && c.version >= MIN_FW_TEST_KAD_VERSION
                        && c.tcp_port.is_some()
                        && c.kad_id != self.local_id()
                        && seen.insert(c.ip)
                })
                .take(UDP_FW_CANDIDATE_POOL)
                .collect()
        };

        if candidates.is_empty() {
            tracing::debug!(
                "Kad UDP firewall check: no suitable candidate contacts (need verified Kad>=6 with TCP port)"
            );
            return UdpFirewallStatus::Unverified;
        }

        let mut probes_completed: usize = 0;
        for contact in &candidates {
            if probes_completed >= UDP_FW_CLIENTS_TO_ASK {
                break;
            }
            match self
                .fw_test_candidate(contact, tracker, intern_port, extern_port)
                .await
            {
                Ok(res) => {
                    // SetUDPFWCheckResult de eMule: un datagrama solo es concluyente
                    // cuando error_code == 0 y el puerto sondeado es un puerto que
                    // realmente hayamos anunciado (interno o externo conocido).
                    let valid_port =
                        res.port_tested == intern_port || (extern_port == Some(res.port_tested));
                    if res.error_code == 0 && valid_port {
                        let mut fw = tracker.lock().await;
                        if res.port_tested != intern_port {
                            // Nos alcanzó a través del puerto externo mapeado por NAT.
                            fw.extern_port = Some(res.port_tested);
                            fw.use_extern = true;
                            tracing::info!(
                                "Kad UDP firewall check: OPEN via extern port {} (from {})",
                                res.port_tested,
                                log_peer_ip(contact.ip)
                            );
                        } else {
                            fw.use_extern = false;
                            tracing::info!(
                                "Kad UDP firewall check: OPEN on intern port {} (from {})",
                                intern_port,
                                log_peer_ip(contact.ip)
                            );
                        }
                        return UdpFirewallStatus::Open;
                    }
                    probes_completed += 1;
                    tracing::warn!(
                        "Kad UDP firewall check from {} returned unusable result (error={}, port={}) — trying next candidate",
                        log_peer_ip(contact.ip),
                        res.error_code,
                        res.port_tested
                    );
                }
                Err(CandidateTestError::ProbeNoResponse(e)) => {
                    probes_completed += 1;
                    tracing::debug!(
                        "Kad UDP firewall check via {} sent probe without UDP reply: {} — trying next candidate",
                        log_peer_ip(contact.ip),
                        e
                    );
                }
                Err(CandidateTestError::PreProbe(e)) => {
                    tracing::debug!(
                        "Kad UDP firewall check pre-probe TCP/handshake via {} failed: {} — trying next candidate",
                        log_peer_ip(contact.ip),
                        e
                    );
                }
            }
        }

        let verdict = resolve_fw_round_verdict(probes_completed);
        if verdict == UdpFirewallStatus::Unverified {
            tracing::info!(
                "Kad UDP firewall check inconclusive: 0/{} candidates reachable over TCP (keeping unverified)",
                candidates.len()
            );
        }
        verdict
    }

    /// Ejecuta una comprobación individual contra un contacto: conexión TCP → OP_HELLO →
    /// OP_FWCHECKUDPREQ (PR_EMULE) → esperar el datagrama FIREWALLUDP.
    async fn fw_test_candidate(
        &self,
        contact: &KadContact,
        tracker: &Arc<tokio::sync::Mutex<crate::services::kad_udp::FirewallTestTracker>>,
        intern_port: u16,
        extern_port: Option<u16>,
    ) -> Result<FirewallUdpResult, CandidateTestError> {
        let ip = IpAddr::V4(contact.ip);
        let tcp_port = contact
            .tcp_port
            .ok_or_else(|| CandidateTestError::PreProbe("candidate has no TCP port".to_string()))?;
        let addr = SocketAddr::new(ip, tcp_port);

        // Clave de verificación UDP por candidato (GetUDPVerifyKey de eMule): los pares remotos
        // cifran su respuesta FIREWALLUDP con ella — almacenada en el rastreador para
        // que el bucle de recepción UDP pueda intentar descifrarla.
        let udp_verify_key: u32 = rand::random::<u32>().wrapping_add(1);

        // Registrar antes de conectar para no perder nunca un datagrama temprano.
        let rx = tracker.lock().await.register_candidate(ip, udp_verify_key);

        let result = self
            .fw_test_candidate_inner(addr, rx, udp_verify_key, intern_port, extern_port)
            .await;

        tracker.lock().await.unregister_candidate(&ip);
        result
    }

    async fn fw_test_candidate_inner(
        &self,
        addr: SocketAddr,
        rx: tokio::sync::oneshot::Receiver<FirewallUdpResult>,
        udp_verify_key: u32,
        intern_port: u16,
        extern_port: Option<u16>,
    ) -> Result<FirewallUdpResult, CandidateTestError> {
        let _permit = self
            .policy_mgr
            .request_tcp_connect(addr, ConnectionPurpose::Ed2kPeer)
            .map_err(|e| CandidateTestError::PreProbe(format!("policy denied connect to {addr}: {e}")))?;

        let mut stream = tokio::time::timeout(FW_TCP_CONNECT_TIMEOUT, TcpStream::connect(addr))
            .await
            .map_err(|_| CandidateTestError::PreProbe(format!("TCP connect to {addr} timed out")))?
            .map_err(|e| CandidateTestError::PreProbe(format!("TCP connect to {addr} refused: {e}")))?;

        // OP_HELLO — el par remoto comprueba nuestra versión Kad anunciada
        // (bits 0-3 de MiscOptions2) antes de aceptar OP_FWCHECKUDPREQ.
        let hello = self.build_fw_hello(intern_port);
        let encoder = Ed2kEncoder::default();
        let hello_pkt = encoder
            .encode_hello(&hello)
            .map_err(|e| CandidateTestError::PreProbe(format!("failed to encode OP_HELLO: {e}")))?;
        let mut out = Vec::with_capacity(6 + hello_pkt.payload.len());
        out.push(hello_pkt.protocol);
        out.extend_from_slice(&(hello_pkt.frame_len() as u32).to_le_bytes());
        out.push(hello_pkt.opcode);
        out.extend_from_slice(&hello_pkt.payload);
        stream
            .write_all(&out)
            .await
            .map_err(|e| CandidateTestError::PreProbe(format!("failed to send OP_HELLO: {e}")))?;

        // Esperar OP_HELLO_ANSWER (o un OP_HELLO simple — algunos clientes
        // inician su propio saludo primero).
        let codec = PacketCodec::new();
        let mut read_buf = BytesMut::with_capacity(8192);
        let mut raw_buf = [0u8; 4096];
        let deadline = tokio::time::Instant::now() + FW_HANDSHAKE_TIMEOUT;
        let mut hello_done = false;
        while !hello_done {
            let read_res = tokio::time::timeout_at(deadline, stream.read(&mut raw_buf))
                .await
                .map_err(|_| {
                    CandidateTestError::PreProbe("handshake timed out waiting for OP_HELLO_ANSWER".to_string())
                })?
                .map_err(|e| {
                    CandidateTestError::PreProbe(format!("read error during handshake: {e}"))
                })?;
            if read_res == 0 {
                return Err(CandidateTestError::PreProbe(
                    "peer closed connection during handshake".to_string(),
                ));
            }
            read_buf.extend_from_slice(&raw_buf[..read_res]);
            while let Ok(Some(pkt)) = codec.decode(&mut read_buf) {
                let Some(pkt) = mule_protocol::ed2k::packed::unpack_owned(
                    pkt,
                    mule_protocol::ed2k::packed::PACKED_PEER_MAX_DECOMPRESSED,
                ) else {
                    continue;
                };
                if pkt.protocol == PR_ED2K
                    && (pkt.opcode == OP_HELLO_ANSWER || pkt.opcode == OP_HELLO)
                {
                    hello_done = true;
                    break;
                }
            }
        }

        // OP_FWCHECKUDPREQ (PR_EMULE): u16 interno + u16 externo + clave u32.
        let mut payload = Vec::with_capacity(8);
        payload.extend_from_slice(&intern_port.to_le_bytes());
        payload.extend_from_slice(&extern_port.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&udp_verify_key.to_le_bytes());
        let mut req = Vec::with_capacity(6 + payload.len());
        req.push(PR_EMULE);
        req.extend_from_slice(&((payload.len() as u32) + 1).to_le_bytes());
        req.push(OP_FWCHECKUDPREQ);
        req.extend_from_slice(&payload);
        stream.write_all(&req).await.map_err(|e| {
            CandidateTestError::PreProbe(format!("failed to send OP_FWCHECKUDPREQ: {e}"))
        })?;

        tracing::info!(
            "Kad UDP firewall check: OP_FWCHECKUDPREQ sent to {peer} (intern={intern_port}, extern={})",
            extern_port.unwrap_or(0)
        , peer = log_peer_addr(addr));

        // Mantener abierta la conexión TCP mientras se espera el datagrama (eMule
        // descarta la prueba si el cliente solicitante se desconecta).
        tokio::time::timeout(FW_UDP_WAIT_TIMEOUT, rx)
            .await
            .map_err(|_| {
                CandidateTestError::ProbeNoResponse(
                    "no FIREWALLUDP datagram within timeout".to_string(),
                )
            })?
            .map_err(|_| {
                CandidateTestError::ProbeNoResponse("firewall test channel dropped".to_string())
            })
    }

    /// OP_HELLO mínimo para conexiones de prueba de cortafuegos — mismos flags de capacidad
    /// que el saludo normal de pares, con versión Kad >= 6 en MiscOptions2
    /// (los pares remotos rechazan OP_FWCHECKUDPREQ procedente de clientes más antiguos).
    fn build_fw_hello(&self, intern_port: u16) -> Ed2kHello {
        use mule_protocol::ed2k::tags::*;
        let user_hash = *self.fw_user_hash.read().unwrap_or_else(|p| p.into_inner());
        let ed2k_tcp_port = self
            .ed2k_tcp_port
            .load(std::sync::atomic::Ordering::Relaxed);

        let mut tags = Vec::with_capacity(6);
        tags.push(Ed2kTag {
            name: TagName::Id(TAG_SPECIAL_NAME),
            value: TagValue::String("eMuleRust".to_string()),
        });
        tags.push(Ed2kTag {
            name: TagName::Id(TAG_SPECIAL_VERSION),
            value: TagValue::UInt32(0x3C),
        });
        let udp_ports = ((intern_port as u32) << 16) | (intern_port as u32);
        tags.push(Ed2kTag {
            name: TagName::Id(TAG_SPECIAL_EMULE_UDPPORTS),
            value: TagValue::UInt32(udp_ports),
        });
        tags.push(Ed2kTag {
            name: TagName::Id(TAG_SPECIAL_CT_EMULE_VERSION),
            value: TagValue::UInt32(50 << 10),
        });
        let misc1: u32 =
            (1 << 29) | (1 << 28) | (4 << 24) | (1 << 20) | (3 << 16) | (4 << 12) | (1 << 4) | 1;
        tags.push(Ed2kTag {
            name: TagName::Id(TAG_SPECIAL_EMULE_MISCOPTIONS1),
            value: TagValue::UInt32(misc1),
        });
        // MiscOptions2: SX2<<10 | ExtMultiPkt<<5 | LargeFiles<<4 | KadVersion<<0
        let misc2: u32 = (1 << 10) | (1 << 5) | (1 << 4) | (OUR_KAD_VERSION as u32);
        tags.push(Ed2kTag {
            name: TagName::Id(TAG_SPECIAL_EMULE_MISCOPTIONS2),
            value: TagValue::UInt32(misc2),
        });

        Ed2kHello {
            user_hash,
            client_id: 0,
            tcp_port: ed2k_tcp_port,
            tags,
            unknown_tags: vec![],
            server_ip: 0,
            server_port: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sin_sondeos_completados_mantiene_unverified() {
        assert_eq!(resolve_fw_round_verdict(0), UdpFirewallStatus::Unverified);
    }

    #[test]
    fn con_sondeos_completados_sin_datagrama_declara_firewalled() {
        assert_eq!(resolve_fw_round_verdict(1), UdpFirewallStatus::Firewalled);
        assert_eq!(
            resolve_fw_round_verdict(UDP_FW_CLIENTS_TO_ASK),
            UdpFirewallStatus::Firewalled
        );
    }
}

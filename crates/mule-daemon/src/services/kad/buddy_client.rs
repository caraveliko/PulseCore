// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Cliente compañero (*buddy*) LowID saliente para Kad (`KADEMLIA_FINDBUDDY_REQ` + enlace TCP).
//!
//! Cuando este nodo está tras cortafuegos TCP (LowID) pero conoce contactos Kad verificados,
//! solicita a un par abierto actuar como repetidor (`buddy`), establece el enlace TCP eD2K
//! persistente, lo mantiene vivo mediante `OP_BUDDYPING` ↔ `OP_BUDDYPONG` y atiende los
//! relés `OP_CALLBACK` entrantes iniciando `connect_back`.

use crate::services::peer_session::log_peer_addr;
use bytes::BytesMut;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use mule_core::kad::{KadContact, KadId};
use mule_network_policy::{ConnectionPurpose, CancellationToken};
use mule_protocol::ed2k::{
    Ed2kDecoder, Ed2kEncoder, Ed2kHello, OP_BUDDYPING, OP_BUDDYPONG, OP_CALLBACK,
    OP_HELLO_ANSWER, PR_ED2K, PR_EMULE,
};
use mule_protocol::framing::PacketCodec;

use super::KadService;

/// Intervalo mínimo entre intentos de búsqueda de compañero saliente.
const BUDDY_FIND_RETRY_INTERVAL: Duration = Duration::from_secs(60);
/// Cadencia de keep-alive `OP_BUDDYPING` sobre el enlace TCP de compañero (20 min).
const BUDDY_PING_INTERVAL: Duration = Duration::from_secs(20 * 60);
/// Tiempo máximo para establecer la conexión TCP con el repetidor de compañero.
const BUDDY_TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Tiempo máximo esperando `OP_HELLO_ANSWER` durante el saludo del enlace de compañero.
const BUDDY_HELLO_TIMEOUT: Duration = Duration::from_secs(8);
/// Número máximo de contactos verificados candidatos por ronda de búsqueda de compañero.
const BUDDY_FIND_CANDIDATES: usize = 5;
/// Capacidad inicial del búfer de lectura del enlace TCP de compañero.
const BUDDY_READ_BUF_CAP: usize = 8192;
/// Límite máximo acumulado en el búfer de lectura del enlace TCP de compañero.
const BUDDY_MAX_READ_BUF: usize = 65_536;
/// Longitud mínima esperada en carga útil de `OP_CALLBACK`: `[buddy_id:16][file:16][ip:4][tcp:2]`.
const BUDDY_CALLBACK_PAYLOAD_LEN: usize = 38;
/// Relleno máximo de ofuscación para el enlace TCP saliente de compañero.
const BUDDY_OBFU_PADDING_MAX: u8 = 16;

/// Estado del enlace saliente de compañero mantenido por este nodo cuando está tras cortafuegos.
#[derive(Debug, Default)]
pub struct OutgoingBuddyState {
    pub active_addr: Option<SocketAddr>,
    pub active_buddy_id: Option<[u8; 16]>,
    pub last_attempt: Option<Instant>,
    pub last_pong_unix_ms: Option<u64>,
    pub task: Option<tokio::task::JoinHandle<()>>,
}

/// Calcula el `buddy_id` eMule (`~kad_id`, complemento bit a bit del `KadId` local).
pub(crate) fn compute_buddy_id(local_id: &KadId) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (dst, src) in out.iter_mut().zip(local_id.0.iter()) {
        *dst = !*src;
    }
    out
}

fn frame_emule_tcp(protocol: u8, opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + payload.len());
    out.push(protocol);
    out.extend_from_slice(&(payload.len() as u32 + 1).to_le_bytes());
    out.push(opcode);
    out.extend_from_slice(payload);
    out
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl KadService {
    /// Instantánea simplificada para diagnóstico/pruebas: `(active_addr, last_pong_unix_ms)`.
    pub async fn outgoing_buddy_status(&self) -> (Option<SocketAddr>, Option<u64>) {
        let st = self.outgoing_buddy.lock().await;
        (st.active_addr, st.last_pong_unix_ms)
    }

    /// Negocia `FINDBUDDY_REQ` por UDP y establece el enlace TCP persistente `KS_CONNECTED_BUDDY`.
    pub async fn establish_buddy_link(
        &self,
        buddy_udp_addr: SocketAddr,
        cancel_token: CancellationToken,
    ) -> Result<SocketAddr, String> {
        let tcp_port = self.ed2k_tcp_port.load(Ordering::Relaxed);
        if tcp_port == 0 {
            return Err("Puerto TCP eD2K local desconocido; no se puede solicitar compañero".to_string());
        }

        let local_id = self.local_id();
        let buddy_id = compute_buddy_id(&local_id);
        let user_hash = *self.fw_user_hash.read().unwrap_or_else(|p| p.into_inner());
        let wire_user_id = if user_hash != [0u8; 16] {
            KadId::from_hash_be(&user_hash).0
        } else {
            local_id.0
        };

        let res = self
            .udp
            .send_find_buddy(buddy_udp_addr, &buddy_id, &wire_user_id, tcp_port)
            .await?;

        if res.tcp_port == 0 {
            return Err("FINDBUDDY_RES devolvió puerto TCP 0".to_string());
        }
        let buddy_tcp_addr = SocketAddr::new(buddy_udp_addr.ip(), res.tcp_port);

        let permit = self
            .policy_mgr
            .request_tcp_connect(buddy_tcp_addr, ConnectionPurpose::KadNode)
            .map_err(|e| format!("Política de red denegó enlace compañero TCP a {buddy_tcp_addr}: {e}"))?;

        let raw_stream = tokio::select! {
            _ = cancel_token.cancelled() => {
                return Err("Cancelado antes de abrir enlace TCP de compañero".to_string());
            }
            conn = tokio::time::timeout(BUDDY_TCP_CONNECT_TIMEOUT, TcpStream::connect(buddy_tcp_addr)) => {
                conn.map_err(|_| format!("Tiempo de espera agotado conectando TCP con buddy {buddy_tcp_addr}"))?
                    .map_err(|e| format!("Fallo TCP conectando con buddy {buddy_tcp_addr}: {e}"))?
            }
        };
        let _ = raw_stream.set_nodelay(true);

        let obf_requested = (res.connect_options & 0x01) != 0;
        let obf_required = (res.connect_options & 0x04) != 0;
        let mut stream = if obf_requested {
            let peer_hash = KadId(res.client_id).to_hash_be();
            crate::services::obfuscation_stream::ObfuscatedStream::connect_peer_outgoing(
                raw_stream,
                &peer_hash,
                BUDDY_OBFU_PADDING_MAX,
            )
            .await
            .map_err(|e| format!("Fallo de ofuscación con buddy {buddy_tcp_addr}: {e}"))?
        } else {
            crate::services::obfuscation_stream::ObfuscatedStream::plaintext(raw_stream)
        };

        let kad_port = self.udp.local_udp_port().await.unwrap_or(0);
        let udp_open = {
            let tracker = self.udp.firewall_tracker();
            let fw = tracker.lock().await;
            fw.verified && fw.status == crate::services::kad_udp::UdpFirewallStatus::Open
        };
        let tags = crate::services::hello_caps::build_hello_tags(
            &crate::services::hello_caps::HelloBuildParams {
                nickname: "eMuleRust",
                tcp_port,
                kad_port,
                sec_ident_cap: 0,
                tcp_low_id: true,
                udp_verified_open: udp_open,
                obfuscation_enabled: obf_requested,
                obfuscation_required: obf_required,
                pc26_caps: None,
            },
        );
        let hello = Ed2kHello {
            user_hash,
            client_id: 1,
            tcp_port,
            tags,
            unknown_tags: vec![],
            server_ip: 0,
            server_port: 0,
        };
        let encoder = Ed2kEncoder::default();
        let pkt = encoder
            .encode_hello(&hello)
            .map_err(|e| format!("No se pudo codificar OP_HELLO de compañero: {e}"))?;
        let wire_hello = frame_emule_tcp(pkt.protocol, pkt.opcode, &pkt.payload);
        stream
            .write_all(&wire_hello)
            .await
            .map_err(|e| format!("Fallo enviando OP_HELLO al buddy {buddy_tcp_addr}: {e}"))?;

        let codec = PacketCodec::new();
        let decoder = Ed2kDecoder::default();
        let mut read_buf = BytesMut::with_capacity(BUDDY_READ_BUF_CAP);
        let mut raw_buf = [0u8; BUDDY_READ_BUF_CAP];

        let hello_ok = tokio::time::timeout(BUDDY_HELLO_TIMEOUT, async {
            loop {
                let n = stream.read(&mut raw_buf).await.map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("EOF esperando OP_HELLO_ANSWER del buddy".to_string());
                }
                read_buf.extend_from_slice(&raw_buf[..n]);
                if read_buf.len() > BUDDY_MAX_READ_BUF {
                    return Err("Límite de búfer excedido durante handshake buddy".to_string());
                }
                while let Ok(Some(p)) = codec.decode(&mut read_buf) {
                    let Some(p) = mule_protocol::ed2k::packed::unpack_owned(
                        p,
                        mule_protocol::ed2k::packed::PACKED_PEER_MAX_DECOMPRESSED,
                    ) else {
                        continue;
                    };
                    if p.protocol == PR_ED2K && p.opcode == OP_HELLO_ANSWER {
                        let _ = decoder
                            .decode_hello_answer(p.protocol, p.opcode, &p.payload, 0)
                            .map_err(|e| format!("OP_HELLO_ANSWER inválido del buddy: {e}"))?;
                        return Ok(());
                    }
                }
            }
        })
        .await;

        match hello_ok {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(format!(
                    "Tiempo de espera agotado esperando OP_HELLO_ANSWER de {buddy_tcp_addr}"
                ));
            }
        }

        // Enviar OP_BUDDYPING inicial para validar inmediatamente el relé antes de entrar en el bucle.
        let ping_frame = frame_emule_tcp(PR_EMULE, OP_BUDDYPING, &[]);
        stream
            .write_all(&ping_frame)
            .await
            .map_err(|e| format!("Error enviando OP_BUDDYPING inicial a {buddy_tcp_addr}: {e}"))?;

        let state_arc = Arc::clone(&self.outgoing_buddy);
        let inbound_arc = self.inbound_listener.read().unwrap_or_else(|p| p.into_inner()).clone();
        let policy_cancel = permit.cancellation_token().clone();

        {
            let mut st = state_arc.lock().await;
            if let Some(prev) = st.task.take() {
                prev.abort();
            }
            st.active_addr = Some(buddy_tcp_addr);
            st.active_buddy_id = Some(buddy_id);
        }

        let state_for_task = Arc::clone(&state_arc);
        let handle = tokio::spawn(async move {
            let _permit = permit;
            let mut ping_interval = tokio::time::interval(BUDDY_PING_INTERVAL);
            ping_interval.tick().await;
            let mut tmp_buf = [0u8; BUDDY_READ_BUF_CAP];

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => break,
                    _ = policy_cancel.cancelled() => break,
                    _ = ping_interval.tick() => {
                        let ping = frame_emule_tcp(PR_EMULE, OP_BUDDYPING, &[]);
                        if stream.write_all(&ping).await.is_err() {
                            break;
                        }
                    }
                    read_res = stream.read(&mut tmp_buf) => {
                        match read_res {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                read_buf.extend_from_slice(&tmp_buf[..n]);
                                if read_buf.len() > BUDDY_MAX_READ_BUF {
                                    break;
                                }
                                while let Ok(Some(pkt)) = codec.decode(&mut read_buf) {
                                    let Some(pkt) = mule_protocol::ed2k::packed::unpack_owned(
                                        pkt,
                                        mule_protocol::ed2k::packed::PACKED_PEER_MAX_DECOMPRESSED,
                                    ) else {
                                        continue;
                                    };
                                    if pkt.protocol == PR_EMULE && pkt.opcode == OP_BUDDYPONG {
                                        let mut st = state_for_task.lock().await;
                                        st.last_pong_unix_ms = Some(now_unix_ms());
                                    } else if (pkt.protocol == PR_EMULE || pkt.protocol == PR_ED2K)
                                        && pkt.opcode == OP_CALLBACK
                                        && pkt.payload.len() >= BUDDY_CALLBACK_PAYLOAD_LEN
                                    {
                                        if pkt.payload[..16] != buddy_id {
                                            continue;
                                        }
                                        let req_ip = Ipv4Addr::new(
                                            pkt.payload[32],
                                            pkt.payload[33],
                                            pkt.payload[34],
                                            pkt.payload[35],
                                        );
                                        let req_port = u16::from_le_bytes([
                                            pkt.payload[36],
                                            pkt.payload[37],
                                        ]);
                                        if req_port == 0 {
                                            continue;
                                        }
                                        let cb_target = SocketAddr::new(IpAddr::V4(req_ip), req_port);
                                        if let Some(ref listener) = inbound_arc {
                                            let listener_c = Arc::clone(listener);
                                            tokio::spawn(async move {
                                                if let Err(e) = listener_c
                                                    .connect_back(cb_target, None, 0, 1, 0, 0)
                                                    .await
                                                {
                                                    tracing::debug!(
                                                        "Buddy connect_back hacia {} falló: {}",
                                                        log_peer_addr(cb_target),
                                                        e
                                                    );
                                                }
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let mut st = state_for_task.lock().await;
            if st.active_addr == Some(buddy_tcp_addr) {
                st.active_addr = None;
                st.active_buddy_id = None;
                st.task = None;
            }
            tracing::info!(
                "Kad: enlace compañero saliente con {} cerrado",
                log_peer_addr(buddy_tcp_addr)
            );
        });

        state_arc.lock().await.task = Some(handle);
        tracing::info!(
            "Kad: enlace compañero saliente establecido con {} (buddy_id={})",
            log_peer_addr(buddy_tcp_addr),
            hex::encode(buddy_id)
        );
        Ok(buddy_tcp_addr)
    }

    /// Si este nodo está tras cortafuegos TCP (sin IP pública HighID) y no posee compañero activo,
    /// intenta negociar `KADEMLIA_FINDBUDDY_REQ` con contactos verificados adecuados.
    pub async fn ensure_outgoing_buddy(&self, cancel_token: CancellationToken) {
        if self.public_ipv4.read().unwrap_or_else(|p| p.into_inner()).is_some() {
            return;
        }
        let tcp_firewalled = {
            let tracker = self.udp.firewall_tracker();
            let fw = tracker.lock().await;
            fw.tcp_is_firewalled()
        };
        if !tcp_firewalled || self.ed2k_tcp_port.load(Ordering::Relaxed) == 0 {
            return;
        }

        {
            let mut st = self.outgoing_buddy.lock().await;
            if let Some(ref task) = st.task {
                if !task.is_finished() && st.active_addr.is_some() {
                    return;
                }
                st.active_addr = None;
                st.task = None;
            }
            if st
                .last_attempt
                .is_some_and(|t| t.elapsed() < BUDDY_FIND_RETRY_INTERVAL)
            {
                return;
            }
            st.last_attempt = Some(Instant::now());
        }

        let local_id = self.local_id();
        let candidates: Vec<KadContact> = {
            let table = self.routing_table.lock().await;
            table
                .find_verified_closest(&local_id, 24)
                .into_iter()
                .filter(|c| c.tcp_port.is_some() && !c.ip.is_unspecified())
                .take(BUDDY_FIND_CANDIDATES)
                .collect()
        };

        for contact in candidates {
            if cancel_token.is_cancelled() {
                break;
            }
            let addr = SocketAddr::new(IpAddr::V4(contact.ip), contact.udp_port);
            match self.establish_buddy_link(addr, cancel_token.clone()).await {
                Ok(_) => break,
                Err(e) => {
                    tracing::debug!(
                        "Kad: intento de compañero hacia {} descartado: {}",
                        log_peer_addr(addr),
                        e
                    );
                }
            }
        }
    }
}

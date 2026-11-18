// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use crate::services::peer_session::log_peer_addr;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::net::UdpSocket;

use mule_core::kad::{KBucketTable, KadContact, KadId};
use mule_protocol::ed2k::OP_CALLBACK;
use mule_protocol::kad::*;

use super::storage::{KadStorage, KAD_KEYWORD_TTL, KAD_NOTE_TTL, KAD_SOURCE_TTL};
use super::types::{
    BuddyRegistry, FirewallTestTracker, FirewallUdpResult, InFlightMap, KadExpectedResponse,
    UdpFirewallStatus,
};
use crate::services::downloads::DownloadService;
use crate::services::kad::firewall::OUR_KAD_VERSION;
use mule_protocol::ed2k::packed::pack_kad_datagram;

/// `KADEMLIA_VERSION7_49a` de eMule (`Opcodes.h:29`) — los contactos estrictamente superiores
/// a `KADEMLIA_VERSION6_49aBETA` (6) reciben el `FIREWALLED2_REQ` extendido y
/// responden con `OP_KAD_FWTCPCHECK_ACK` por TCP.
const KADEMLIA_VERSION7_49A: u8 = 7;

type DownloadsSlot = Arc<tokio::sync::RwLock<Option<Arc<tokio::sync::Mutex<DownloadService>>>>>;

/// Internal dispatcher for incoming messages.
#[allow(clippy::too_many_arguments)]
pub async fn handle_incoming_message(
    socket: &Arc<UdpSocket>,
    peer_addr: SocketAddr,
    msg: KadMessage,
    in_flight: &InFlightMap,
    local_kad_id: &[u8; 16],
    // Nuestro hash de usuario eD2K — campo `clientHash` de `KADEMLIA_FIREWALLED2_REQ`
    // (`CPrefs::GetClientHash()` de eMule = el hash de usuario, no el ID Kad).
    our_user_hash: &[u8; 16],
    local_port: u16,
    storage: &Arc<tokio::sync::Mutex<KadStorage>>,
    downloads: &DownloadsSlot,
    firewall: &Arc<tokio::sync::Mutex<FirewallTestTracker>>,
    buddy: &Arc<tokio::sync::Mutex<BuddyRegistry>>,
    // Local eD2K TCP port — also advertised in `BOOTSTRAP_RES`/`HELLO_RES`
    // headers (eMule `thePrefs.GetPort()`).
    buddy_tcp_port: u16,
    buddy_connect_options: u8,
    // Shared routing table (`None` until `KadService` injects it).
    routing_table: Option<Arc<tokio::sync::Mutex<KBucketTable>>>,
) {
    match msg {
        KadMessage::Ping(_ping) => {
            tracing::debug!(
                "Kad UDP: received Ping from {peer}, replying with Pong",
                peer = log_peer_addr(peer_addr)
            );
            let pong_bytes = KadCodec::encode_pong(peer_addr.port());
            let _ = socket
                .send_to(&pack_kad_datagram(&pong_bytes), peer_addr)
                .await;
        }
        KadMessage::Pong(pong) => {
            let mut inf = in_flight.lock().await;
            let mut matched_tx = None;
            for (id, tx) in inf.iter() {
                if (tx.target == peer_addr || tx.target.ip() == peer_addr.ip())
                    && tx.expected_response == KadExpectedResponse::Pong
                {
                    matched_tx = Some(*id);
                    break;
                }
            }
            tracing::debug!(
                "Kad UDP: received Pong from {peer} (matched_tx={matched_tx:?})",
                peer = log_peer_addr(peer_addr)
            );
            if let Some(id) = matched_tx {
                if let Some(tx) = inf.remove(&id) {
                    let _ = tx.sender.send(Ok(KadMessage::Pong(pong)));
                }
            }
        }
        KadMessage::HelloReq(hello) => {
            tracing::debug!(
                "Kad UDP: received HelloReq from {peer} (kad_v{}, tcp={}, misc={:?}), replying with HelloRes",
                hello.version,
                hello.tcp_port,
                hello.misc_options
            , peer = log_peer_addr(peer_addr));
            // AddContact_KADEMLIA2 de eMule: cada HELLO_REQ alimenta la tabla de
            // enrutamiento. El contacto permanece no verificado hasta que responde a una
            // de nuestras solicitudes o completa el saludo HELLO_RES_ACK.
            if let IpAddr::V4(ip) = peer_addr.ip() {
                learn_contact(
                    &routing_table,
                    hello.kad_id,
                    ip,
                    if hello.udp_port != 0 {
                        hello.udp_port
                    } else {
                        peer_addr.port()
                    },
                    if hello.tcp_port != 0 {
                        Some(hello.tcp_port)
                    } else {
                        None
                    },
                    hello.version,
                    false,
                )
                .await;
            }
            // KADMISCOPTIONS de eMule (KademliaUDPListener.cpp:131-137):
            // bit0 = cortafuegos UDP, bit1 = cortafuegos TCP (GetFirewalled de
            // CPrefs), bit2 = solicitar al receptor completar el
            // saludo con HELLO_RES_ACK.
            let mut misc = 0x04u8;
            {
                let fw = firewall.lock().await;
                if fw.verified && fw.status == UdpFirewallStatus::Firewalled {
                    misc |= 0x01;
                }
                if fw.tcp_is_firewalled() {
                    misc |= 0x02;
                }
            }
            let hello_res = KadCodec::encode_hello_res(
                local_kad_id,
                local_port,
                OUR_KAD_VERSION,
                buddy_tcp_port,
                Some(misc),
            );
            let _ = socket
                .send_to(&pack_kad_datagram(&hello_res), peer_addr)
                .await;
            // Final de Process_KADEMLIA2_HELLO_REQ en eMule — una ronda de recomprobación en
            // curso transforma cada HELLO entrante en una solicitud de sondeo TCP.
            maybe_tcp_firewall_check(
                socket,
                peer_addr,
                hello.version,
                firewall,
                buddy_tcp_port,
                buddy_connect_options,
                our_user_hash,
            )
            .await;
        }
        KadMessage::HelloRes(hello_res) => {
            // Kad >= 8: el respondedor puede solicitarnos demostrar nuestra IP con un
            // HELLO_RES_ACK (bit 2 de KADMISCOPTIONS).
            let wants_ack = hello_res.requests_ack();
            // Capturado antes de mover `hello_res` al remitente de transacción
            // inferior — necesario para el gancho de recomprobación TCP.
            let peer_kad_version = hello_res.version;
            let peer_udp_port = if hello_res.udp_port != 0 {
                hello_res.udp_port
            } else {
                peer_addr.port()
            };
            // in_flight debe liberarse antes de que learn_contact espere a
            // routing_table — retenerlo crea un orden in_flight → routing_table
            // que provocaría interbloqueo con get_status (routing_table →
            // get_snapshot → in_flight).
            let (matched_id, matched_sender) = {
                let mut inf = in_flight.lock().await;
                let mut matched_tx = None;
                for (id, tx) in inf.iter() {
                    if (tx.target == peer_addr || tx.target.ip() == peer_addr.ip())
                        && tx.expected_response == KadExpectedResponse::HelloRes
                    {
                        matched_tx = Some(*id);
                        break;
                    }
                }
                (matched_tx, matched_tx.and_then(|id| inf.remove(&id)))
            };
            tracing::debug!(
                "Kad UDP: received HelloRes from {peer} (kad_v{}, tcp={}, udp_tag={}, matched_tx={matched_id:?})",
                hello_res.version,
                hello_res.tcp_port,
                peer_udp_port
            , peer = log_peer_addr(peer_addr));
            if let Some(tx) = matched_sender {
                // Un HELLO_RES solicitado demuestra que el remoto posee su IP:puerto
                // anunciados — el contacto queda verificado (IsIpVerified de eMule).
                if let IpAddr::V4(ip) = peer_addr.ip() {
                    learn_contact(
                        &routing_table,
                        hello_res.kad_id,
                        ip,
                        peer_udp_port,
                        if hello_res.tcp_port != 0 {
                            Some(hello_res.tcp_port)
                        } else {
                            None
                        },
                        hello_res.version,
                        true,
                    )
                    .await;
                }
                let _ = tx.sender.send(Ok(KadMessage::HelloRes(hello_res)));
            }
            if wants_ack {
                let ack = KadCodec::encode_hello_res_ack(local_kad_id);
                let _ = socket.send_to(&pack_kad_datagram(&ack), peer_addr).await;
            }
            // eMule tail of Process_KADEMLIA2_HELLO_RES — same recheck hook.
            maybe_tcp_firewall_check(
                socket,
                peer_addr,
                peer_kad_version,
                firewall,
                buddy_tcp_port,
                buddy_connect_options,
                our_user_hash,
            )
            .await;
        }
        KadMessage::HelloResAck(ack) => {
            // Completa el saludo de tres vías (Process_KADEMLIA2_HELLO_RES_ACK de
            // eMule): el par confirmó nuestro HELLO_RES, demostrando que posee su
            // IP anunciada — promueve el contacto almacenado a verificado, pero solo
            // si el origen del paquete coincide con la IP del contacto (un ACK para el
            // KadID de otro no debe verificarlo).
            if let (Some(table), IpAddr::V4(ip)) = (&routing_table, peer_addr.ip()) {
                let mut t = table.lock().await;
                let id = KadId(ack.kad_id);
                if t.get(&id).map(|c| c.ip == ip).unwrap_or(false) {
                    t.mark_seen(&id, now_unix_ms());
                }
            }
            tracing::debug!(
                "Kad UDP: received HelloResAck from {peer} (kad_id={})",
                hex::encode(ack.kad_id),
                peer = log_peer_addr(peer_addr)
            );
        }
        KadMessage::FirewalledReq(req) => {
            handle_firewalled_check(socket, peer_addr, req.tcp_port, false).await;
        }
        KadMessage::FirewalledReq2(req) => {
            handle_firewalled_check(socket, peer_addr, req.tcp_port, true).await;
        }
        KadMessage::FirewalledRes(res) => {
            // Respuesta a un FIREWALLED_REQ enviado por nosotros — contiene nuestra IP
            // pública según la observó el respondedor. `Process_KADEMLIA_FIREWALLED_RES` de eMule
            // descarta respuestas no solicitadas (`IsKadFirewallCheckIP`), registra la IP
            // observada (`SetIPAddress`) y avanza la ronda (`IncRecheckIP`).
            let ip = std::net::Ipv4Addr::from(res.ip.to_le_bytes());
            let mut fw = firewall.lock().await;
            if !fw.is_tcp_check_ip(peer_addr.ip()) {
                tracing::debug!(
                    "Kad UDP: unrequested FirewalledRes from {peer} — ignored",
                    peer = log_peer_addr(peer_addr)
                );
            } else {
                fw.note_firewalled_res(ip);
                tracing::debug!(
                    "Kad UDP: received FirewalledRes from {peer} (observed ip={ip}, round {} done)",
                    fw.tcp_answers,
                    peer = log_peer_addr(peer_addr)
                );
            }
        }
        KadMessage::FirewalledAckRes => {
            // Veredicto de devolución de conexión heredado (Kad < 7) por UDP — eMule
            // `Process_KADEMLIA_FIREWALLED_ACK_RES` → `IncFirewalled`.
            firewall.lock().await.inc_tcp_acks();
            tracing::debug!(
                "Kad UDP: received FirewalledAckRes from {peer} (TCP port reachable)",
                peer = log_peer_addr(peer_addr)
            );
        }
        KadMessage::BootstrapReq(req) => {
            // Process_KADEMLIA2_BOOTSTRAP_REQ de eMule: responde con nuestro KadID +
            // puerto eD2K TCP + versión de Kad y hasta 20 contactos verificados
            // distribuidos a lo largo del espacio de claves (GetBootstrapContacts).
            let contacts: Vec<KadContactWire> = match &routing_table {
                Some(table) => table
                    .lock()
                    .await
                    .spread_verified(20)
                    .iter()
                    .map(contact_to_wire)
                    .collect(),
                None => Vec::new(),
            };
            tracing::debug!(
                "Kad UDP: received BootstrapReq from {peer} (requester_id={}), replying with {} contacts",
                hex::encode(req.kad_id),
                contacts.len()
            , peer = log_peer_addr(peer_addr));
            let res = KadCodec::encode_bootstrap_res(
                local_kad_id,
                buddy_tcp_port,
                OUR_KAD_VERSION,
                &contacts,
            );
            let _ = socket.send_to(&pack_kad_datagram(&res), peer_addr).await;
        }
        KadMessage::Req(req) => {
            // Process_KADEMLIA2_REQ de eMule: [type][target][uCheck] — uCheck es
            // el KadID que el solicitante cree que poseemos; una discrepancia indica
            // una entrada de contacto obsoleta y eMule descarta la solicitud en silencio.
            let count = (req.req_type & 0x1F) as usize;
            if count == 0 || req.sender_id != *local_kad_id {
                tracing::debug!(
                    "Kad UDP: dropping Req from {peer} (type=0x{:02X}, id check: {})",
                    req.req_type,
                    if req.sender_id == *local_kad_id {
                        "ok"
                    } else {
                        "stale kad_id"
                    },
                    peer = log_peer_addr(peer_addr)
                );
                return;
            }
            // Only IP-verified contacts may be returned (eMule GetClosestTo
            // type 2 → bOnlyVerified) — propagating unverified contacts would
            // relay potentially spoofed entries.
            let contacts: Vec<KadContactWire> = match &routing_table {
                Some(table) => table
                    .lock()
                    .await
                    .find_verified_closest(&KadId(req.target_id), count.min(32))
                    .iter()
                    .map(contact_to_wire)
                    .collect(),
                None => Vec::new(),
            };
            tracing::debug!(
                "Kad UDP: received Req (0x21) from {peer} for target {}, replying with {} verified contacts",
                hex::encode(req.target_id),
                contacts.len()
            , peer = log_peer_addr(peer_addr));
            let res = KadCodec::encode_find_node_res(&req.target_id, &contacts);
            let _ = socket.send_to(&pack_kad_datagram(&res), peer_addr).await;
        }
        KadMessage::BootstrapRes(boot_res) => {
            // Liberar in_flight antes de que learn_contact espere routing_table —
            // same in_flight → routing_table ordering hazard as HelloRes.
            let (matched_id, matched_sender) = {
                let mut inf = in_flight.lock().await;
                let mut matched_tx = None;
                for (id, tx) in inf.iter() {
                    if (tx.target == peer_addr || tx.target.ip() == peer_addr.ip())
                        && (tx.expected_response == KadExpectedResponse::BootstrapRes
                            || tx.expected_response == KadExpectedResponse::FindNodeRes)
                    {
                        matched_tx = Some(*id);
                        break;
                    }
                }
                (matched_tx, matched_tx.and_then(|id| inf.remove(&id)))
            };
            tracing::debug!(
                "Kad UDP: received BootstrapRes/FindNodeRes from {peer} with {} contacts (matched_tx={matched_id:?})",
                boot_res.contacts.len()
            , peer = log_peer_addr(peer_addr));
            if matched_id.is_some() {
                // Respuesta solicitada → el respondedor queda verificado. El campo de puerto
                // de su encabezado es su puerto TCP (disposición BOOTSTRAP_RES de eMule).
                if let IpAddr::V4(ip) = peer_addr.ip() {
                    learn_contact(
                        &routing_table,
                        boot_res.kad_id,
                        ip,
                        peer_addr.port(),
                        if boot_res.udp_port != 0 {
                            Some(boot_res.udp_port)
                        } else {
                            None
                        },
                        boot_res.version,
                        true,
                    )
                    .await;
                }
            }
            if let Some(tx) = matched_sender {
                let _ = tx.sender.send(Ok(KadMessage::BootstrapRes(boot_res)));
            }
        }
        KadMessage::SearchSourceReq(req) => {
            let sources = {
                let mut store = storage.lock().await;
                store.get_sources(&req.target_file_hash, req.start_pos)
            };
            // `SendValidSourceResult` de eMule: los resultados se dividen en tantos
            // datagramas como sea necesario para mantenerse bajo `UDP_KAD_MAXFRAGMENT`.
            let packets =
                KadCodec::encode_search_source_res_fragmented(&req.target_file_hash, &sources);
            tracing::debug!(
                "Kad UDP: received SearchSourceReq from {peer} for file {}, replying with {} stored sources in {} packet(s)",
                hex::encode(req.target_file_hash),
                sources.len(),
                packets.len()
            , peer = log_peer_addr(peer_addr));
            for res in packets {
                let _ = socket.send_to(&pack_kad_datagram(&res), peer_addr).await;
            }
        }
        KadMessage::SearchSourceRes(search_res) => {
            let mut inf = in_flight.lock().await;
            let mut matched_tx = None;
            for (id, tx) in inf.iter() {
                let ip_match = tx.target == peer_addr || tx.target.ip() == peer_addr.ip();
                let hash_match = tx
                    .expected_target_hash
                    .is_none_or(|h| h == search_res.target_file_hash);
                let resp_match = tx.expected_response == KadExpectedResponse::SearchSourceRes;
                if ip_match && hash_match && resp_match {
                    matched_tx = Some(*id);
                    break;
                }
            }
            tracing::debug!(
                "Kad UDP: received SearchSourceRes from {peer} with {} sources for file {} (matched_tx={matched_tx:?})",
                search_res.sources.len(),
                hex::encode(search_res.target_file_hash)
            , peer = log_peer_addr(peer_addr));
            if let Some(id) = matched_tx {
                // Respuesta multipaquete: mantener registrada la transacción para que
                // el siguiente lote de este par se siga enrutando al remitente;
                // eliminarla únicamente cuando el recolector haya finalizado.
                if let Some(tx) = inf.get(&id) {
                    if tx
                        .sender
                        .send(Ok(KadMessage::SearchSourceRes(search_res)))
                        .is_err()
                    {
                        inf.remove(&id);
                    }
                }
            }
        }
        KadMessage::PublishSourceReq(req) => {
            // ProcessPublishSourceRequest de eMule: los destinos fuera de tolerancia
            // se descartan en silencio — no somos indexador para esta clave.
            if !within_publish_tolerance(local_kad_id, &req.target_file_hash) {
                tracing::debug!(
                    "Kad UDP: dropping out-of-tolerance PublishSourceReq from {peer} for file {}",
                    hex::encode(req.target_file_hash),
                    peer = log_peer_addr(peer_addr)
                );
                return;
            }
            let peer_ip_bytes = match peer_addr.ip() {
                std::net::IpAddr::V4(v4) => v4.octets(),
                std::net::IpAddr::V6(_) => [127, 0, 0, 1],
            };
            // CKademliaUDPListener::ProcessPublishSourceRequest de eMule: el
            // contacto anunciado debe coincidir con la IP del remitente y portar un puerto TCP.
            let valid_contact = req.contact.ip == peer_ip_bytes && req.contact.tcp_port > 0;
            if valid_contact {
                // El contacto autoanunciado del publicador está vinculado a su IP y
                // validado — aprenderlo (no verificado) como en AddContact de eMule.
                learn_contact(
                    &routing_table,
                    req.contact.kad_id,
                    Ipv4Addr::from(req.contact.ip),
                    req.contact.udp_port,
                    Some(req.contact.tcp_port),
                    req.contact.version,
                    false,
                )
                .await;
            }
            let status = if valid_contact {
                let stored = {
                    let mut store = storage.lock().await;
                    store.store_source(
                        req.target_file_hash,
                        KadSourceWire {
                            kad_id: req.contact.kad_id,
                            ip: req.contact.ip,
                            udp_port: req.contact.udp_port,
                            tcp_port: req.contact.tcp_port,
                            encryption: 0,
                            buddy_id: None,
                        },
                    )
                };
                if stored {
                    persist_indexed_source(downloads, &req).await;
                    0
                } else {
                    1
                }
            } else {
                1
            };
            tracing::debug!(
                "Kad UDP: received PublishSourceReq from {peer} for file {} (status={})",
                hex::encode(req.target_file_hash),
                status,
                peer = log_peer_addr(peer_addr)
            );
            let res = KadCodec::encode_publish_source_res(&req.target_file_hash, status);
            let _ = socket.send_to(&pack_kad_datagram(&res), peer_addr).await;
        }
        KadMessage::PublishSourceRes(pub_res) => {
            // `Process_KADEMLIA2_PUBLISH_RES` de eMule («para uso futuro»): un
            // byte de opciones final con el bit 0 solicita un datagrama
            // `KADEMLIA2_PUBLISH_RES_ACK` (0x4C) simple de vuelta.
            if pub_res.request_ack {
                let ack = [PR_KADEMLIA, KADEMLIA2_PUBLISH_RES_ACK];
                let _ = socket.send_to(&pack_kad_datagram(&ack), peer_addr).await;
            }
            // El opcode 0x4B es la respuesta de publicación unificada tanto para fuentes como para palabras clave.
            // Match in-flight transactions expecting either PublishSourceRes or PublishKeywordRes.
            let mut inf = in_flight.lock().await;
            let mut matched_tx = None;
            for (id, tx) in inf.iter() {
                let ip_match = tx.target == peer_addr || tx.target.ip() == peer_addr.ip();
                let hash_match = tx
                    .expected_target_hash
                    .is_none_or(|h| h == pub_res.target_file_hash);
                let resp_match = tx.expected_response == KadExpectedResponse::PublishSourceRes
                    || tx.expected_response == KadExpectedResponse::PublishKeywordRes
                    || tx.expected_response == KadExpectedResponse::PublishNotesRes;
                if ip_match && hash_match && resp_match {
                    matched_tx = Some(*id);
                    break;
                }
            }
            tracing::debug!(
                "Kad UDP: received PublishRes (0x4B) from {peer} with status {} (matched_tx={matched_tx:?})",
                pub_res.status
            , peer = log_peer_addr(peer_addr));
            if let Some(id) = matched_tx {
                if let Some(tx) = inf.remove(&id) {
                    if tx.expected_response == KadExpectedResponse::PublishNotesRes {
                        let _ =
                            tx.sender
                                .send(Ok(KadMessage::PublishNotesRes(KadPublishNotesRes {
                                    target_file_hash: pub_res.target_file_hash,
                                    load: pub_res.status,
                                })));
                    } else if tx.expected_response == KadExpectedResponse::PublishKeywordRes {
                        let _ = tx.sender.send(Ok(KadMessage::PublishKeywordRes(
                            KadPublishKeywordRes {
                                keyword_hash: pub_res.target_file_hash,
                                status: pub_res.status,
                            },
                        )));
                    } else {
                        let _ = tx.sender.send(Ok(KadMessage::PublishSourceRes(pub_res)));
                    }
                }
            }
        }
        KadMessage::SearchKeywordReq(req) => {
            // `Process_KADEMLIA2_SEARCH_KEY_REQ` de eMule: cuando `restrictive`
            // está activado, los bytes finales forman un árbol de expresiones de búsqueda
            // (`CreateSearchExpressionTree`). Un árbol malformado genera error y el
            // datagrama entero se descarta.
            let expr = if req.restrictive {
                match req.search_expression.as_deref() {
                    Some(bytes) => {
                        match KadSearchExpression::parse(
                            PR_KADEMLIA,
                            KADEMLIA2_SEARCH_KEY_REQ,
                            bytes,
                        ) {
                            Ok(e) => Some(e),
                            Err(err) => {
                                tracing::debug!(
                                    "Kad UDP: dropping SearchKeywordReq from {peer}: \
                                     malformed restrictive expression ({err})",
                                    peer = log_peer_addr(peer_addr)
                                );
                                return;
                            }
                        }
                    }
                    // Indicador restrictivo sin expresión: eMule sirve
                    // unfiltered results (pSearchTerms == NULL).
                    None => None,
                }
            } else {
                None
            };
            let entries = {
                let mut store = storage.lock().await;
                store.get_keyword_entries_filtered(&req.keyword_hash, req.start_pos, |entry| {
                    expr.as_ref()
                        .is_none_or(|e| e.matches(&KadKeywordSearchView::new(entry)))
                })
            };
            let packets =
                KadCodec::encode_search_keyword_res_fragmented(&req.keyword_hash, &entries);
            tracing::debug!(
                "Kad UDP: received SearchKeywordReq from {peer} for keyword {} (restrictive={}), replying with {} stored entries in {} packet(s)",
                hex::encode(req.keyword_hash),
                req.restrictive,
                entries.len(),
                packets.len()
            , peer = log_peer_addr(peer_addr));
            for res in packets {
                let _ = socket.send_to(&pack_kad_datagram(&res), peer_addr).await;
            }
        }
        KadMessage::SearchKeywordRes(search_res) => {
            let mut inf = in_flight.lock().await;
            let mut matched_tx = None;
            for (id, tx) in inf.iter() {
                let ip_match = tx.target == peer_addr || tx.target.ip() == peer_addr.ip();
                let hash_match = tx
                    .expected_target_hash
                    .is_none_or(|h| h == search_res.keyword_hash);
                let resp_match = tx.expected_response == KadExpectedResponse::SearchKeywordRes;
                if ip_match && hash_match && resp_match {
                    matched_tx = Some(*id);
                    break;
                }
            }
            tracing::debug!(
                "Kad UDP: received SearchKeywordRes from {peer} with {} entries for keyword {} (matched_tx={matched_tx:?})",
                search_res.entries.len(),
                hex::encode(search_res.keyword_hash)
            , peer = log_peer_addr(peer_addr));
            if let Some(id) = matched_tx {
                // Respuesta multipaquete: mantener la transacción registrada para que
                // el siguiente lote de este par se siga enrutando al remitente.
                if let Some(tx) = inf.get(&id) {
                    if tx
                        .sender
                        .send(Ok(KadMessage::SearchKeywordRes(search_res)))
                        .is_err()
                    {
                        inf.remove(&id);
                    }
                }
            }
        }
        KadMessage::PublishKeywordReq(req) => {
            // ProcessPublishKeywordRequest de eMule: los hashes de palabras clave
            // fuera de tolerancia se descartan en silencio.
            if !within_publish_tolerance(local_kad_id, &req.keyword_hash) {
                tracing::debug!(
                    "Kad UDP: dropping out-of-tolerance PublishKeywordReq from {peer} for keyword {}",
                    hex::encode(req.keyword_hash)
                , peer = log_peer_addr(peer_addr));
                return;
            }
            // Rechazar entradas malformadas: hash de archivo cero o nombre vacío/excesivo.
            let valid_entries: Vec<_> = req
                .entries
                .into_iter()
                .filter(|e| {
                    e.file_hash != [0u8; 16] && !e.file_name.is_empty() && e.file_name.len() <= 255
                })
                .collect();
            let keyword_hash = req.keyword_hash;
            let to_persist = valid_entries.clone();
            let publisher_ip = match peer_addr.ip() {
                IpAddr::V4(v4) => v4.octets(),
                IpAddr::V6(_) => [0u8; 4],
            };
            let status = {
                let mut store = storage.lock().await;
                if store.store_keyword_entries(keyword_hash, valid_entries, publisher_ip) {
                    0
                } else {
                    1
                }
            };
            if status == 0 {
                persist_keyword_entries(downloads, &keyword_hash, &to_persist).await;
            }
            tracing::debug!(
                "Kad UDP: received PublishKeywordReq from {peer} for keyword {} (status={})",
                hex::encode(req.keyword_hash),
                status,
                peer = log_peer_addr(peer_addr)
            );
            let res = KadCodec::encode_publish_keyword_res(&req.keyword_hash, status);
            let _ = socket.send_to(&pack_kad_datagram(&res), peer_addr).await;
        }
        KadMessage::PublishKeywordRes(pub_res) => {
            let mut inf = in_flight.lock().await;
            let mut matched_tx = None;
            for (id, tx) in inf.iter() {
                if (tx.target == peer_addr || tx.target.ip() == peer_addr.ip())
                    && tx.expected_response == KadExpectedResponse::PublishKeywordRes
                {
                    matched_tx = Some(*id);
                    break;
                }
            }
            tracing::debug!(
                "Kad UDP: received PublishKeywordRes from {peer} with status {} (matched_tx={matched_tx:?})",
                pub_res.status
            , peer = log_peer_addr(peer_addr));
            if let Some(id) = matched_tx {
                if let Some(tx) = inf.remove(&id) {
                    let _ = tx.sender.send(Ok(KadMessage::PublishKeywordRes(pub_res)));
                }
            }
        }
        KadMessage::FirewallUdp(fw) => {
            // Process_KADEMLIA2_FIREWALLUDP de eMule → SetUDPFWCheckResult de
            // CUDPFirewallTester: solo los paquetes de respuesta procedentes de IPs candidatas
            // que realmente hayamos consultado se enrutan a una prueba pendiente.
            let delivered = {
                let mut fw_track = firewall.lock().await;
                fw_track.notify_packet(
                    peer_addr.ip(),
                    FirewallUdpResult {
                        error_code: fw.error_code,
                        port_tested: fw.udp_port,
                    },
                )
            };
            if delivered {
                tracing::debug!(
                    "Kad UDP: received FIREWALLUDP from {peer} (error={}, tested_port={})",
                    fw.error_code,
                    fw.udp_port,
                    peer = log_peer_addr(peer_addr)
                );
            } else {
                tracing::warn!(
                    "Kad UDP: ignoring unsolicited FIREWALLUDP from {peer} (tested_port={})",
                    fw.udp_port,
                    peer = log_peer_addr(peer_addr)
                );
            }
        }
        // --- Rol de repetidor de amigos (mecanismo GetBuddy de CClientList de eMule) ---
        KadMessage::FindBuddyReq(req) => {
            // Process_KADEMLIA_FINDBUDDY_REQ de eMule: solo un nodo verificado,
            // no tras cortafuegos y sin amigo actual puede aceptar.
            let eligible = {
                let fw = firewall.lock().await;
                fw.verified && fw.status == UdpFirewallStatus::Open
            };
            let mut reg = buddy.lock().await;
            if !eligible || buddy_tcp_port == 0 || reg.active.is_some() || req.tcp_port == 0 {
                tracing::debug!(
                    "Kad UDP: rejecting FINDBUDDY_REQ from {peer} (eligible={eligible}, has_buddy={})",
                    reg.active.is_some()
                , peer = log_peer_addr(peer_addr));
                return;
            }
            reg.register_pending(peer_addr.ip(), req.buddy_id, req.user_id, req.tcp_port);
            drop(reg);
            // eMule `Process_KADEMLIA_FINDBUDDY_REQ` responde con `GetClientHash()`,
            // es decir, el hash de usuario eD2K en formato `CUInt128`.
            let client_hash = if *our_user_hash != [0u8; 16] {
                KadId::from_hash_be(our_user_hash).0
            } else {
                *local_kad_id
            };
            let res = KadCodec::encode_findbuddy_res(
                &req.buddy_id,
                &client_hash,
                buddy_tcp_port,
                buddy_connect_options,
            );
            tracing::info!(
                "Kad UDP: accepted FINDBUDDY_REQ from {peer}, awaiting buddy TCP link on port {} (buddy_id={})",
                buddy_tcp_port,
                hex::encode(req.buddy_id)
            , peer = log_peer_addr(peer_addr));
            let _ = socket.send_to(&pack_kad_datagram(&res), peer_addr).await;
        }
        KadMessage::FindBuddyRes(res) => {
            // Nunca enviamos FINDBUDDY_REQ (el cliente buddy para uno mismo en LowID es
            // not implemented); log unsolicited responses.
            tracing::debug!(
                "Kad UDP: ignoring unsolicited FINDBUDDY_RES from {peer} (tcp_port={})",
                res.tcp_port,
                peer = log_peer_addr(peer_addr)
            );
        }
        KadMessage::CallbackReq(req) => {
            // eMule Process_KADEMLIA_CALLBACK_REQ: forward to our buddy's
            // TCP link as OP_CALLBACK
            // [check:16][file:16][requester_ip:4][requester_tcp:2].
            let relayed = {
                let reg = buddy.lock().await;
                if reg.active.is_some() {
                    let mut payload = Vec::with_capacity(38);
                    payload.extend_from_slice(&req.buddy_id);
                    payload.extend_from_slice(&req.file_id);
                    match peer_addr.ip() {
                        IpAddr::V4(v4) => payload.extend_from_slice(&v4.octets()),
                        IpAddr::V6(v6) => payload.extend_from_slice(
                            &v6.to_ipv4_mapped()
                                .map(|v4| v4.octets())
                                .unwrap_or([0, 0, 0, 0]),
                        ),
                    }
                    payload.extend_from_slice(&req.tcp_port.to_le_bytes());
                    let frame = frame_emule_packet(OP_CALLBACK, &payload);
                    reg.send_to_buddy(frame)
                } else {
                    false
                }
            };
            if relayed {
                tracing::info!(
                    "Kad UDP: relayed CALLBACK_REQ from {peer} to buddy link (file={}, cb_port={})",
                    hex::encode(req.file_id),
                    req.tcp_port,
                    peer = log_peer_addr(peer_addr)
                );
            } else {
                tracing::debug!(
                    "Kad UDP: dropping CALLBACK_REQ from {peer} — no active buddy link",
                    peer = log_peer_addr(peer_addr)
                );
            }
        }
        // --- Notas Kad (comentarios / valoraciones de archivos) ---
        KadMessage::SearchNotesReq(req) => {
            let notes = {
                let mut store = storage.lock().await;
                store.get_notes(
                    &req.target_file_hash,
                    req.file_size,
                    MAX_NOTE_SEARCH_RESULTS,
                )
            };
            if notes.is_empty() {
                tracing::debug!(
                    "Kad UDP: SearchNotesReq from {peer} for file {} — no stored notes",
                    hex::encode(req.target_file_hash),
                    peer = log_peer_addr(peer_addr)
                );
                return;
            }
            tracing::debug!(
                "Kad UDP: received SearchNotesReq from {peer} for file {}, replying with {} notes",
                hex::encode(req.target_file_hash),
                notes.len(),
                peer = log_peer_addr(peer_addr)
            );
            if let Some(res) =
                KadCodec::encode_search_notes_res(local_kad_id, &req.target_file_hash, &notes)
            {
                let _ = socket.send_to(&pack_kad_datagram(&res), peer_addr).await;
            }
        }
        KadMessage::SearchNotesRes(res) => {
            let mut inf = in_flight.lock().await;
            let mut matched_tx = None;
            for (id, tx) in inf.iter() {
                let ip_match = tx.target == peer_addr || tx.target.ip() == peer_addr.ip();
                let hash_match = tx
                    .expected_target_hash
                    .is_none_or(|h| h == res.target_file_hash);
                let resp_match = tx.expected_response == KadExpectedResponse::SearchNotesRes;
                if ip_match && hash_match && resp_match {
                    matched_tx = Some(*id);
                    break;
                }
            }
            tracing::debug!(
                "Kad UDP: received SearchNotesRes from {peer} with {} notes for file {} (matched_tx={matched_tx:?})",
                res.notes.len(),
                hex::encode(res.target_file_hash)
            , peer = log_peer_addr(peer_addr));
            if let Some(id) = matched_tx {
                // Respuesta multipaquete: mantener la transacción registrada para que
                // el siguiente lote de este par se siga enrutando al remitente.
                if let Some(tx) = inf.get(&id) {
                    if tx.sender.send(Ok(KadMessage::SearchNotesRes(res))).is_err() {
                        inf.remove(&id);
                    }
                }
            }
        }
        KadMessage::PublishNotesReq(req) => {
            // Process_KADEMLIA2_PUBLISH_NOTES_REQ de eMule: fuera de tolerancia
            // los objetivos se descartan silenciosamente antes de cualquier trabajo de indexación.
            if !within_publish_tolerance(local_kad_id, &req.target_file_hash) {
                tracing::debug!(
                    "Kad UDP: dropping out-of-tolerance PublishNotesReq from {peer} for file {}",
                    hex::encode(req.target_file_hash),
                    peer = log_peer_addr(peer_addr)
                );
                return;
            }
            // eMule CKademliaUDPListener::Process_KADEMLIA2_PUBLISH_NOTES_REQ:
            // a UDP-firewalled node refuses indexing work.
            let firewalled = {
                let fw = firewall.lock().await;
                fw.verified && fw.status == UdpFirewallStatus::Firewalled
            };
            if firewalled {
                tracing::debug!(
                    "Kad UDP: rejecting PublishNotesReq from {peer} — we are UDP-firewalled",
                    peer = log_peer_addr(peer_addr)
                );
                return;
            }
            let publisher_ip = match peer_addr.ip() {
                IpAddr::V4(v4) => v4.octets(),
                IpAddr::V6(_) => [127, 0, 0, 1],
            };
            let note = KadNoteWire {
                source_id: req.source_id,
                tags: req.tags,
                unknown_tags: req.unknown_tags,
            };
            let accepted = {
                let mut store = storage.lock().await;
                store.store_note(
                    req.target_file_hash,
                    note.clone(),
                    publisher_ip,
                    peer_addr.port(),
                )
            };
            if accepted {
                persist_indexed_note(
                    downloads,
                    &req.target_file_hash,
                    &note,
                    publisher_ip,
                    peer_addr.port(),
                )
                .await;
            }
            let load = {
                let store = storage.lock().await;
                store.note_load(&req.target_file_hash)
            };
            tracing::debug!(
                "Kad UDP: received PublishNotesReq from {peer} for file {} (accepted={}, load={})",
                hex::encode(req.target_file_hash),
                accepted,
                load,
                peer = log_peer_addr(peer_addr)
            );
            let res = KadCodec::encode_publish_notes_res(&req.target_file_hash, load);
            let _ = socket.send_to(&pack_kad_datagram(&res), peer_addr).await;
        }
        KadMessage::PublishNotesRes(_) => {
            // Comparte el opcode 0x4B con PublishSourceRes/PublishKeywordRes; la
            // rama PublishSourceRes unificada superior ya lo enruta según el
            // expected_response en curso. Inalcanzable en la práctica.
        }
    }
}

/// Gestiona un `KADEMLIA_FIREWALLED_REQ` (0x50) / `KADEMLIA_FIREWALLED2_REQ`
/// (0x53): `Process_KADEMLIA_FIREWALLED{2}_REQ` de eMule. Devolvemos de inmediato en eco
/// la IPv4 observada del remitente en `KADEMLIA_FIREWALLED_RES`, y luego nos conectamos por TCP
/// a su puerto anunciado. En caso de éxito reconocemos mediante
/// `OP_KAD_FWTCPCHECK_ACK` en el flujo TCP (Kad >= 7) o un paquete nulo UDP
/// `KADEMLIA_FIREWALLED_ACK_RES` (legado).
async fn handle_firewalled_check(
    socket: &Arc<UdpSocket>,
    peer_addr: SocketAddr,
    tcp_port: u16,
    kad7_plus: bool,
) {
    let IpAddr::V4(v4) = peer_addr.ip() else {
        tracing::debug!(
            "Kad UDP: ignoring firewalled check from non-IPv4 {peer}",
            peer = log_peer_addr(peer_addr)
        );
        return;
    };
    if tcp_port == 0 {
        tracing::debug!(
            "Kad UDP: ignoring firewalled check from {peer} with tcp_port=0",
            peer = log_peer_addr(peer_addr)
        );
        return;
    }

    // FIREWALLED_RES devuelve en eco la IP observada del solicitante (eMule lo envía una
    // vez aceptado RequestTCP — antes de conocer el resultado del sondeo).
    let res = KadCodec::encode_firewalled_res(u32::from_le_bytes(v4.octets()));
    let _ = socket.send_to(&pack_kad_datagram(&res), peer_addr).await;

    let socket2 = Arc::clone(socket);
    tokio::spawn(async move {
        let target = SocketAddr::new(IpAddr::V4(v4), tcp_port);
        let probe = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio::net::TcpStream::connect(target),
        )
        .await;
        match probe {
            Ok(Ok(mut stream)) => {
                if kad7_plus {
                    // Kad >= 7: OP_KAD_FWTCPCHECK_ACK (PR_EMULE, carga útil vacía)
                    // en la propia conexión de sondeo.
                    let frame = frame_emule_packet(mule_protocol::ed2k::OP_KAD_FWTCPCHECK_ACK, &[]);
                    use tokio::io::AsyncWriteExt;
                    match stream.write_all(&frame).await {
                        Ok(()) => tracing::info!(
                            "Kad UDP: TCP firewall probe to {target} succeeded, sent OP_KAD_FWTCPCHECK_ACK"
                        ),
                        Err(e) => tracing::debug!(
                            "Kad UDP: probe to {target} connected but ack send failed: {e}"
                        ),
                    }
                } else {
                    // Legacy (<7): empty UDP KADEMLIA_FIREWALLED_ACK_RES.
                    let ack = [PR_KADEMLIA, KADEMLIA_FIREWALLED_ACK_RES];
                    let _ = socket2.send_to(&pack_kad_datagram(&ack), peer_addr).await;
                    tracing::info!(
                        "Kad UDP: TCP firewall probe to {target} succeeded, sent FIREWALLED_ACK_RES"
                    );
                }
            }
            Ok(Err(e)) => {
                tracing::info!("Kad UDP: TCP firewall probe to {target} refused: {e}");
            }
            Err(_) => {
                tracing::info!("Kad UDP: TCP firewall probe to {target} timed out");
            }
        }
    });
}

/// Entrama un paquete TCP eD2K/eMule: `[proto][len:u32][opcode][payload]`
/// (opcode incluido en la longitud, como en `Packet`/`PacketCodec`).
fn frame_emule_packet(opcode: u8, payload: &[u8]) -> Vec<u8> {
    use mule_protocol::ed2k::PR_EMULE;
    let mut out = Vec::with_capacity(6 + payload.len());
    out.push(PR_EMULE);
    out.extend_from_slice(&(payload.len() as u32 + 1).to_le_bytes());
    out.push(opcode);
    out.extend_from_slice(payload);
    out
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Convierte un contacto de la tabla de enrutamiento a su forma en cable para `KADEMLIA2_RES` /
/// `BOOTSTRAP_RES` answers.
fn contact_to_wire(c: &KadContact) -> KadContactWire {
    KadContactWire {
        kad_id: c.kad_id.0,
        ip: c.ip.octets(),
        udp_port: c.udp_port,
        tcp_port: c.tcp_port.unwrap_or(0),
        version: c.version,
    }
}

/// Registra un contacto aprendido de un paquete entrante en la tabla de enrutamiento
/// (`AddContact_KADEMLIA2` / `CRoutingZone::Add` de eMule — cada paquete alimenta
/// la zona de enrutamiento).
///
/// La semántica de `verified` refleja la de eMule: los contactos se aprenden *no verificados*;
/// `verified` se obtiene únicamente mediante una respuesta solicitada (`mark_seen = true`,
/// el par respondió a una de nuestras solicitudes — demostrando la posesión de la IP) o
/// mediante el saludo de tres vías `HELLO_RES_ACK`.
async fn learn_contact(
    routing_table: &Option<Arc<tokio::sync::Mutex<KBucketTable>>>,
    kad_id_bytes: [u8; 16],
    ip: Ipv4Addr,
    udp_port: u16,
    tcp_port: Option<u16>,
    version: u8,
    mark_seen: bool,
) {
    let Some(table) = routing_table else {
        return;
    };
    if kad_id_bytes == [0u8; 16] {
        return;
    }
    let now_ms = now_unix_ms();
    let mut contact = KadContact::new(KadId(kad_id_bytes), ip, udp_port, tcp_port, version, now_ms);
    if mark_seen {
        contact.mark_seen(now_ms);
    }
    table.lock().await.insert_or_update(contact);
}

/// Compuerta `SEARCHTOLERANCE` compartida por los manejadores de publicación: eMule descarta
/// una solicitud de publicación cuya clave de destino quede fuera de la tolerancia de nuestro KadID
/// (`uDistance.Get32BitChunk(0) > SEARCHTOLERANCE → return` en
/// `KademliaUDPListener.cpp`). Sin esto, este nodo se convertiría en un sumidero
/// universal de indexación para claves arbitrarias.
fn within_publish_tolerance(local_kad_id: &[u8; 16], target: &[u8; 16]) -> bool {
    KadId(*local_kad_id).is_within_search_tolerance(&KadId(*target))
}

/// Refleja un `PublishSourceReq` aceptado en `kad_sources_index` para que el rol
/// de nodo de almacenamiento sobreviva a reinicios del daemon (TTL = `KAD_SOURCE_TTL`).
async fn persist_indexed_source(downloads: &DownloadsSlot, req: &KadPublishSourceReq) {
    let Some(dl) = downloads.read().await.as_ref().cloned() else {
        return;
    };
    let now_ms = now_unix_ms();
    let row = mule_core::kad::KadIndexedSource {
        file_hash: req.target_file_hash,
        source_kad_id: mule_core::kad::KadId(req.contact.kad_id),
        ip: std::net::Ipv4Addr::from(req.contact.ip),
        udp_port: req.contact.udp_port,
        tcp_port: req.contact.tcp_port,
        file_size: req.file_size,
        encryption: 0,
        expires_at_unix_ms: now_ms + KAD_SOURCE_TTL.as_millis() as u64,
        updated_at_unix_ms: now_ms,
    };
    let mut dl_guard = dl.lock().await;
    if let Err(e) = dl_guard.db_mut().insert_kad_indexed_sources(&[row]) {
        tracing::warn!("Kad storage: failed to persist indexed source: {e}");
    }
}

/// Refleja un `PublishNotesReq` aceptado en `kad_notes_index` para que el rol
/// de nodo de almacenamiento sobreviva a reinicios del daemon. La lista de etiquetas se
/// persiste como su blob codificado en red (`encode_note_tags_blob`) — las etiquetas
/// arbitrarias/desconocidas realizan el viaje de ida y vuelta con exactitud.
async fn persist_indexed_note(
    downloads: &DownloadsSlot,
    target_hash: &KadIdBytes,
    note: &KadNoteWire,
    publisher_ip: [u8; 4],
    publisher_udp_port: u16,
) {
    let Some(dl) = downloads.read().await.as_ref().cloned() else {
        return;
    };
    let Some(blob) =
        mule_protocol::kad::encoder::encode_note_tags_blob(&note.tags, &note.unknown_tags)
    else {
        return;
    };
    let now_ms = now_unix_ms();
    let row = mule_core::kad::KadIndexedNote {
        file_hash: hex::encode(target_hash),
        source_id: hex::encode(note.source_id),
        publisher_ip: std::net::Ipv4Addr::from(publisher_ip).to_string(),
        publisher_udp_port,
        tags_blob: blob,
        discovered_at_unix_ms: now_ms as i64,
        expires_at_unix_ms: (now_ms + KAD_NOTE_TTL.as_millis() as u64) as i64,
    };
    let mut dl_guard = dl.lock().await;
    if let Err(e) = dl_guard.db_mut().insert_kad_notes(&[row]) {
        tracing::warn!("Kad storage: failed to persist indexed note: {e}");
    }
}

/// Refleja las entradas de palabras clave aceptadas en `kad_keywords_index`.
async fn persist_keyword_entries(
    downloads: &DownloadsSlot,
    keyword_hash: &[u8; 16],
    entries: &[KadKeywordEntryWire],
) {
    if entries.is_empty() {
        return;
    }
    let Some(dl) = downloads.read().await.as_ref().cloned() else {
        return;
    };
    let now_ms = now_unix_ms();
    let kw_hex = hex::encode(keyword_hash);
    let rows: Vec<mule_core::kad::KadKeywordEntry> = entries
        .iter()
        .map(|e| mule_core::kad::KadKeywordEntry {
            keyword_hash: kw_hex.clone(),
            file_hash: hex::encode(e.file_hash),
            file_name: e.file_name.clone(),
            file_size: e.file_size,
            source_count: e.source_count.max(1),
            discovered_at_unix_ms: now_ms as i64,
            expires_at_unix_ms: (now_ms + KAD_KEYWORD_TTL.as_millis() as u64) as i64,
        })
        .collect();
    let mut dl_guard = dl.lock().await;
    if let Err(e) = dl_guard.db_mut().insert_kad_keyword_entries(&rows) {
        tracing::warn!("Kad storage: failed to persist keyword entries: {e}");
    }
}

/// `CKademliaUDPListener::FirewalledCheck` de eMule (KademliaUDPListener.cpp:159):
/// mientras una ronda de recomprobación TCP esté activa, cada HELLO_REQ/HELLO_RES entrante
/// desencadena un `KADEMLIA_FIREWALLED{2}_REQ` de vuelta al remitente, solicitándole conectar
/// por TCP a nuestro puerto eD2K anunciado. La versión del contacto determina el formato
/// del paquete — `FIREWALLED2_REQ` (≥ `KADEMLIA_VERSION7_49A`) transporta nuestro hash
/// de usuario + opciones de conexión para que el respondedor pueda completar un saludo
/// Kad completo en la conexión de retorno.
#[allow(clippy::too_many_arguments)]
async fn maybe_tcp_firewall_check(
    socket: &Arc<UdpSocket>,
    peer_addr: SocketAddr,
    peer_kad_version: u8,
    firewall: &Arc<tokio::sync::Mutex<FirewallTestTracker>>,
    our_tcp_port: u16,
    connect_options: u8,
    our_user_hash: &[u8; 16],
) {
    {
        let mut fw = firewall.lock().await;
        if !fw.tcp_recheck_active() || fw.is_tcp_check_ip(peer_addr.ip()) {
            return;
        }
        // Registrado antes del envío (`AddKadFirewallRequest`) para que una rápida
        // FIREWALLED_RES or TCP connect-back is never dropped as unrequested.
        fw.add_tcp_check_ip(peer_addr.ip());
    }
    if our_tcp_port == 0 {
        tracing::debug!("Kad UDP: TCP firewall check skipped — eD2K TCP port unknown");
        return;
    }
    let pkt = if peer_kad_version >= KADEMLIA_VERSION7_49A {
        // eMule lee este campo con `ReadUInt128` + `ToByteArray`, invirtiendo cada DWORD de 4 bytes.
        let wire_hash = if *our_user_hash != [0u8; 16] {
            KadId::from_hash_be(our_user_hash).0
        } else {
            *our_user_hash
        };
        KadCodec::encode_firewalled2_req(our_tcp_port, &wire_hash, connect_options)
    } else {
        KadCodec::encode_firewalled_req(our_tcp_port)
    };
    if let Err(e) = socket.send_to(&pack_kad_datagram(&pkt), peer_addr).await {
        tracing::debug!(
            "Kad UDP: failed to send FIREWALLED{suffix}_REQ to {peer}: {e}",
            suffix = if peer_kad_version >= KADEMLIA_VERSION7_49A {
                "2"
            } else {
                ""
            },
            peer = log_peer_addr(peer_addr)
        );
        return;
    }
    tracing::debug!(
        "Kad UDP: sent FIREWALLED{suffix}_REQ to {peer} (tcp_port={our_tcp_port}, kad_v{peer_kad_version})",
        suffix = if peer_kad_version >= KADEMLIA_VERSION7_49A { "2" } else { "" },
        peer = log_peer_addr(peer_addr)
    );
}

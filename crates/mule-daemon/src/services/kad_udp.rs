// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

pub mod handlers;
pub mod routing_client;
pub mod search_client;
pub mod storage;
pub mod types;

use crate::services::peer_session::log_peer_addr;
use crate::services::peer_session::log_peer_ip;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use mule_core::kad::{KBucketTable, KadId};
use mule_network_policy::{
    classify_ip, ConnectionPurpose, IpCategory, ListenPermit, NetworkPolicyManager, NetworkProfile,
};
use mule_protocol::kad::*;

use crate::services::client_udp::ClientUdpService;
use mule_protocol::ed2k::packed::pack_kad_datagram;

pub use types::{
    ActiveBuddy, BuddyRegistry, FirewallTestTracker, FirewallUdpResult, InFlightTransaction,
    IpRateLimiter, KadExpectedResponse, KadUdpSnapshot, PendingBuddy, UdpFirewallStatus,
    BUDDY_PENDING_TTL, KAD_TRANSACTION_TIMEOUT_SECS, MAX_IN_FLIGHT_TRANSACTIONS,
    MAX_KAD_REQUESTS_PER_SEC_PER_IP,
};

/// Estado interno del servicio Kad UDP.
pub struct KadUdpInner {
    pub socket: Option<Arc<UdpSocket>>,
    pub listen_permit: Option<ListenPermit>,
    pub bind_addr: Option<SocketAddr>,
    pub listen_task: Option<JoinHandle<()>>,
    pub in_flight: Arc<Mutex<HashMap<u32, InFlightTransaction>>>,
    /// Almacén de entradas indexadas (rol de nodo de almacenamiento DHT).
    pub storage: Arc<Mutex<storage::KadStorage>>,
}

impl KadUdpInner {
    /// Registra una transacción y devuelve una guarda que la elimina al destruirse.
    ///
    /// Las entradas abandonadas por futuros `send_*` cancelados (tiempo de espera externo/select
    /// que descarta el futuro antes de que se ejecute su propia rama de tiempo de espera) mueren
    /// una vez que superan `KAD_TRANSACTION_TIMEOUT_SECS` — el `select!` interno se habría
    /// activado de lo contrario. Se barren aquí para que las fugas no puedan saturar el
    /// mapa ni robar respuestas de transacciones activas dirigidas al mismo par.
    pub async fn insert_in_flight(
        &self,
        tx_id: u32,
        entry: InFlightTransaction,
    ) -> Result<types::InFlightTxGuard, String> {
        let mut inf = self.in_flight.lock().await;
        let now = Instant::now();
        let max_age = Duration::from_secs(KAD_TRANSACTION_TIMEOUT_SECS + 2);
        inf.retain(|_, tx| now.duration_since(tx.created_at) < max_age);
        if inf.len() >= MAX_IN_FLIGHT_TRANSACTIONS {
            return Err(format!(
                "Too many in-flight Kad transactions ({MAX_IN_FLIGHT_TRANSACTIONS} max)"
            ));
        }
        inf.insert(tx_id, entry);
        Ok(types::InFlightTxGuard::new(
            Arc::clone(&self.in_flight),
            tx_id,
        ))
    }
}

/// Servicio Kad UDP que gestiona el ciclo de vida del transporte y las transacciones.
pub struct KadUdpService {
    /// Identidad Kad local — compartida con `KadService` (rotación en caliente).
    pub(crate) local_id: Arc<std::sync::RwLock<KadId>>,
    pub(crate) policy_mgr: NetworkPolicyManager,
    pub(crate) inner: Arc<RwLock<KadUdpInner>>,
    pub(crate) next_tx_id: AtomicU32,
    pub(crate) packets_sent: AtomicU64,
    pub(crate) packets_received: AtomicU64,
    /// Servicio UDP cliente a cliente que comparte este socket (eMule multiplexa
    /// eD2K UDP y Kademlia en el mismo puerto UDP).
    client_udp: Arc<RwLock<Option<ClientUdpService>>>,
    /// Manejador del servicio de descargas — persiste entradas indexadas (rol de nodo de
    /// almacenamiento) y rehidrata el almacén en memoria en el inicio.
    downloads: Arc<RwLock<Option<Arc<Mutex<crate::services::downloads::DownloadService>>>>>,
    /// Estado de prueba de cortafuegos/accesibilidad UDP (KADEMLIA2_FIREWALLUDP).
    firewall: Arc<Mutex<FirewallTestTracker>>,
    /// Estado de repetidor de amigos (aceptaciones de FINDBUDDY_REQ + enlace de amigo activo).
    buddy: Arc<Mutex<BuddyRegistry>>,
    /// Nuestro hash de usuario eD2K — enviado como `clientHash` en `KADEMLIA_FIREWALLED2_REQ`
    /// (`GetPrefs()->GetClientHash()` de eMule = `CUInt128(thePrefs.GetUserHash())`,
    /// distinto del ID de nodo Kad aleatorio `local_id`).
    fw_user_hash: Arc<std::sync::RwLock<[u8; 16]>>,
    /// Puerto eD2K TCP local anunciado en `KADEMLIA_FINDBUDDY_RES` — el cliente
    /// amigo se conecta a él para su enlace de retransmisión.
    buddy_tcp_port: Arc<std::sync::atomic::AtomicU16>,
    /// Flags de opciones de conexión enviados en `KADEMLIA_FINDBUDDY_RES`
    /// (byte `GetMyConnectOptions(true, false)` de eMule).
    buddy_connect_options: Arc<std::sync::atomic::AtomicU8>,
    /// Tabla de enrutamiento Kad compartida, inyectada por `KadService` tras la construcción
    /// (`set_routing_table`). El escuchador UDP la necesita para responder a
    /// `KADEMLIA2_REQ`/`BOOTSTRAP_REQ` con contactos reales y para registrar
    /// contactos entrantes — eMule alimenta su zona de enrutamiento desde cada paquete.
    routing_table: Arc<std::sync::RwLock<Option<Arc<Mutex<KBucketTable>>>>>,
}

impl KadUdpService {
    /// Crea una nueva instancia del servicio Kad UDP.
    pub fn new(local_id: Arc<std::sync::RwLock<KadId>>, policy_mgr: NetworkPolicyManager) -> Self {
        Self {
            local_id,
            policy_mgr,
            inner: Arc::new(RwLock::new(KadUdpInner {
                socket: None,
                listen_permit: None,
                bind_addr: None,
                listen_task: None,
                in_flight: Arc::new(Mutex::new(HashMap::new())),
                storage: Arc::new(Mutex::new(storage::KadStorage::default())),
            })),
            next_tx_id: AtomicU32::new(1),
            packets_sent: AtomicU64::new(0),
            packets_received: AtomicU64::new(0),
            client_udp: Arc::new(RwLock::new(None)),
            downloads: Arc::new(RwLock::new(None)),
            firewall: Arc::new(Mutex::new(FirewallTestTracker::default())),
            buddy: Arc::new(Mutex::new(BuddyRegistry::default())),
            fw_user_hash: Arc::new(std::sync::RwLock::new([0u8; 16])),
            buddy_tcp_port: Arc::new(std::sync::atomic::AtomicU16::new(0)),
            buddy_connect_options: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            routing_table: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// Inyecta la tabla de enrutamiento Kad compartida (`KadService` es su propietaria). Es seguro
    /// llamarlo antes o después de `start()` — el escuchador la resuelve por paquete.
    pub fn set_routing_table(&self, table: Arc<Mutex<KBucketTable>>) {
        if let Ok(mut slot) = self.routing_table.write() {
            *slot = Some(table);
        }
    }

    /// Registro compartido de repetidor de amigos (aceptaciones pendientes + enlace activo).
    pub fn buddy_registry(&self) -> Arc<Mutex<BuddyRegistry>> {
        Arc::clone(&self.buddy)
    }

    /// Establece el puerto eD2K TCP + flags de opciones de conexión anunciados en
    /// las respuestas `KADEMLIA_FINDBUDDY_RES`.
    pub fn set_buddy_tcp_info(&self, tcp_port: u16, connect_options: u8) {
        self.buddy_tcp_port.store(tcp_port, Ordering::Relaxed);
        self.buddy_connect_options
            .store(connect_options, Ordering::Relaxed);
    }

    /// Establece el hash de usuario eD2K enviado en `KADEMLIA_FIREWALLED2_REQ` (llamado en el
    /// inicio y en la rotación de identidad, junto con `set_fw_check_identity`).
    pub fn set_fw_user_hash(&self, user_hash: [u8; 16]) {
        if let Ok(mut h) = self.fw_user_hash.write() {
            *h = user_hash;
        }
    }

    /// Bits de opciones de conexión de capa criptográfica (0-2 de `GetMyConnectOptions`) según
    /// se configuraron en el inicio mediante `set_buddy_tcp_info`.
    pub fn buddy_connect_options(&self) -> u8 {
        self.buddy_connect_options.load(Ordering::Relaxed)
    }

    /// Envía un `KADEMLIA_CALLBACK_REQ` (0x52) a un nodo amigo, solicitándole que
    /// retransmita una devolución de llamada a su cliente amigo LowID para que dicho cliente
    /// se conecte de vuelta a nuestro `tcp_port` (eMule `TryToConnect` → `CCS_KADCALLBACK`).
    /// Se envía sin cifrar, coincidiendo con el comentario FIXME de eMule.
    pub async fn send_callback_req(
        &self,
        buddy_addr: SocketAddr,
        buddy_id: &[u8; 16],
        file_id: &[u8; 16],
        tcp_port: u16,
    ) -> Result<(), String> {
        let socket = { self.inner.read().await.socket.clone() };
        let socket = socket.ok_or_else(|| "Kad UDP socket not bound".to_string())?;
        let pkt = KadCodec::encode_callback_req(buddy_id, file_id, tcp_port);
        socket
            .send_to(&pack_kad_datagram(&pkt), buddy_addr)
            .await
            .map_err(|e| {
                format!(
                    "Failed to send CALLBACK_REQ to {}: {e}",
                    log_peer_addr(buddy_addr)
                )
            })?;
        self.packets_sent.fetch_add(1, Ordering::Relaxed);
        tracing::info!(
            "Kad UDP: CALLBACK_REQ sent to buddy {} (buddy_id={}, file={})",
            log_peer_addr(buddy_addr),
            hex::encode(buddy_id),
            hex::encode(file_id)
        );
        Ok(())
    }

    /// Rastreador compartido de pruebas de cortafuegos (candidatos pendientes + último estado conocido).
    pub fn firewall_tracker(&self) -> Arc<Mutex<FirewallTestTracker>> {
        Arc::clone(&self.firewall)
    }

    /// Envía un paquete de sondeo KADEMLIA2_FIREWALLUDP hacia `target` (lado servidor
    /// de la prueba, respondiendo a un `OP_FWCHECKUDPREQ` por TCP).
    pub async fn send_firewall_udp(
        &self,
        target: SocketAddr,
        error_code: u8,
        tested_port: u16,
    ) -> Result<(), String> {
        let socket = { self.inner.read().await.socket.clone() };
        let socket = socket.ok_or_else(|| "Kad UDP socket not bound".to_string())?;
        let pkt = KadCodec::encode_firewall_udp(error_code, tested_port);
        socket
            .send_to(&pack_kad_datagram(&pkt), target)
            .await
            .map_err(|e| format!("Failed to send FIREWALLUDP to {target}: {e}"))?;
        self.packets_sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Puerto UDP vinculado local, si el socket está activo.
    pub async fn local_udp_port(&self) -> Option<u16> {
        self.inner.read().await.bind_addr.map(|a| a.port())
    }

    /// Dirección de socket vinculada local, si el socket está activo.
    pub async fn bind_addr(&self) -> Option<SocketAddr> {
        self.inner.read().await.bind_addr
    }

    /// Inyecta el servicio de descargas para la persistencia del índice.
    pub async fn set_downloads(
        &self,
        downloads: Arc<Mutex<crate::services::downloads::DownloadService>>,
    ) {
        *self.downloads.write().await = Some(downloads);
    }

    /// Inyecta el servicio UDP cliente compartido; también le entrega el socket
    /// vinculado para que pueda emitir keep-alives `OP_REASKFILEPING`.
    pub async fn set_client_udp(&self, svc: ClientUdpService) {
        let socket = { self.inner.read().await.socket.clone() };
        if let Some(socket) = socket {
            svc.set_socket(socket).await;
        }
        *self.client_udp.write().await = Some(svc);
    }

    /// Inicia el escuchador UDP en la dirección de vinculación especificada conforme a la política de red.
    pub async fn start(&self, bind_addr: SocketAddr) -> Result<SocketAddr, String> {
        let mut inner = self.inner.write().await;
        if inner.socket.is_some() {
            if let Some(addr) = inner.bind_addr {
                return Ok(addr);
            }
        }

        let permit = self
            .policy_mgr
            .request_tcp_listen(bind_addr, ConnectionPurpose::KadNode)
            .map_err(|e| format!("Policy denied Kad UDP bind on {bind_addr}: {e}"))?;

        let socket = UdpSocket::bind(&bind_addr)
            .await
            .map_err(|e| format!("Failed to bind Kad UDP socket to {bind_addr}: {e}"))?;

        let local_addr = socket
            .local_addr()
            .map_err(|e| format!("Failed to get local addr: {e}"))?;

        let socket_arc = Arc::new(socket);
        inner.socket = Some(socket_arc.clone());
        inner.bind_addr = Some(local_addr);
        inner.listen_permit = Some(permit);

        let socket_for_task = socket_arc.clone();
        let in_flight_arc = inner.in_flight.clone();
        let routing_arc = Arc::clone(&self.routing_table);
        // Arc compartido — releído en cada datagrama para seguir la rotación
        // de identidad (`identityRotationHours`).
        let local_id_arc = Arc::clone(&self.local_id);
        let policy_mgr_clone = self.policy_mgr.clone();
        let rate_limiter_arc = Arc::new(Mutex::new(IpRateLimiter::new()));
        let storage_arc = inner.storage.clone();
        let client_udp_arc = Arc::clone(&self.client_udp);
        let downloads_arc = Arc::clone(&self.downloads);
        let firewall_arc = Arc::clone(&self.firewall);
        let buddy_arc = Arc::clone(&self.buddy);
        let user_hash_arc = Arc::clone(&self.fw_user_hash);
        let buddy_tcp_port_arc = Arc::clone(&self.buddy_tcp_port);
        let buddy_connect_options_arc = Arc::clone(&self.buddy_connect_options);
        if let Some(svc) = self.client_udp.read().await.as_ref() {
            svc.set_socket(socket_arc.clone()).await;
        }

        // Rehidratar el almacén de entradas indexadas desde SQLite (el rol de nodo de almacenamiento
        // sobrevive a los reinicios); las filas expiradas se purgan primero.
        if let Some(dl) = downloads_arc.read().await.as_ref() {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let (sources, keywords, notes) = {
                let mut dl_guard = dl.lock().await;
                let _ = dl_guard.db_mut().purge_expired_kad_sources(now_ms);
                let _ = dl_guard.db_mut().purge_expired_kad_keywords(now_ms);
                let _ = dl_guard.db_mut().purge_expired_kad_notes(now_ms);
                (
                    dl_guard
                        .db_mut()
                        .list_all_kad_indexed_sources(now_ms, storage::MAX_TOTAL_ENTRIES)
                        .unwrap_or_default(),
                    dl_guard
                        .db_mut()
                        .list_all_kad_keyword_entries(now_ms, storage::MAX_TOTAL_ENTRIES)
                        .unwrap_or_default(),
                    dl_guard
                        .db_mut()
                        .list_all_kad_notes(now_ms, storage::MAX_TOTAL_ENTRIES)
                        .unwrap_or_default(),
                )
            };
            if !sources.is_empty() || !keywords.is_empty() || !notes.is_empty() {
                let n_s = sources.len();
                let n_k = keywords.len();
                let n_n = notes.len();
                let hydrated_notes: Vec<storage::HydratedNote> = notes
                    .into_iter()
                    .filter_map(|n| {
                        let target =
                            <[u8; 16]>::try_from(hex::decode(&n.file_hash).ok()?.as_slice())
                                .ok()?;
                        let source_id =
                            <[u8; 16]>::try_from(hex::decode(&n.source_id).ok()?.as_slice())
                                .ok()?;
                        let (tags, unknown_tags) =
                            mule_protocol::kad::decoder::decode_note_tags_blob(&n.tags_blob)?;
                        let ip: std::net::Ipv4Addr = n.publisher_ip.parse().ok()?;
                        Some(storage::HydratedNote {
                            target_hash: target,
                            note: KadNoteWire {
                                source_id,
                                tags,
                                unknown_tags,
                            },
                            publisher_ip: ip.octets(),
                            publisher_udp_port: n.publisher_udp_port,
                            remaining_ms: (n.expires_at_unix_ms.max(0) as u64)
                                .saturating_sub(now_ms),
                        })
                    })
                    .collect();
                let mut store = storage_arc.lock().await;
                store.hydrate(sources, keywords, now_ms);
                store.hydrate_notes(hydrated_notes);
                tracing::info!(
                    "Kad storage rehydrated: {n_s} sources, {n_k} keyword entries, {n_n} notes"
                );
            }
        }

        let task = tokio::spawn(async move {
            // El eMule oficial asigna un búfer de recepción de 64 KB para garantizar que los datagramas UDP entrantes
            // de hasta la carga útil IP máxima (65507 bytes) nunca fallen con WSAEMSGSIZE (error del SO 10040).
            let mut buf = vec![0u8; 65536];
            loop {
                tokio::select! {
                    recv_res = socket_for_task.recv_from(&mut buf) => {
                        match recv_res {
                            Ok((len, peer_addr)) => {
                                if len < 2 {
                                    continue;
                                }
                                // 0. Filtrado IP global (ipfilter.dat + Auto-Ban dinámico anti-DDoS)
                                if policy_mgr_clone.ip_filter().check_ip(peer_addr.ip()).is_some() {
                                    continue;
                                }
                                let proto_byte = buf[0];
                                let opcode_byte = buf[1];

                                // 1. Comprobar el identificador de protocolo — los datagramas
                                //    que no son de Kad (PR_EMULE/PR_ED2K) pertenecen al
                                //    canal UDP de cliente (eMule comparte un único
                                //    socket UDP para ambos).
                                if !is_valid_kad_protocol(proto_byte) {
                                    if ClientUdpService::is_client_udp_protocol(proto_byte) {
                                        // `ClientUDPSocket.cpp:76` de eMule:
                                        // descarte antes del procesamiento si el puerto es
                                        // nulo o la IP está filtrada (ipfilter).
                                        // (La comprobación de baneo vive en
                                        // `handle_datagram`, que posee
                                        // el DownloadService.)
                                        if peer_addr.port() == 0 {
                                            continue;
                                        }
                                        if let Some(rule) = policy_mgr_clone
                                            .ip_filter()
                                            .check_ip(peer_addr.ip())
                                        {
                                            // `GetLogFilteredIPs` de eMule
                                            // (detallado, DESACTIVADO por defecto) —
                                            // por datagrama: debug.
                                            tracing::debug!(
                                                "Client UDP: dropped datagram from {} (ipfilter '{}')",
                                                log_peer_addr(peer_addr),
                                                rule.description
                                            );
                                            continue;
                                        }
                                        let cat = classify_ip(peer_addr.ip());
                                        let policy_allowed = match policy_mgr_clone.profile() {
                                            NetworkProfile::LocalLab => cat == IpCategory::Loopback,
                                            NetworkProfile::PublicEd2kPreview | NetworkProfile::PublicEd2kKadPreview => {
                                                cat == IpCategory::PublicRoutable
                                            }
                                        };
                                        if policy_allowed {
                                            if let Some(svc) = client_udp_arc.read().await.as_ref() {
                                                svc.handle_datagram(peer_addr, &buf[..len]).await;
                                            }
                                        }
                                    } else {
                                        // Posiblemente un datagrama Kad ofuscado —
                                        // eMule cifra las respuestas KADEMLIA2_FIREWALLUDP
                                        // con la clave de verificación UDP que
                                        // enviamos en OP_FWCHECKUDPREQ.
                                        let keys = {
                                            let fw = firewall_arc.lock().await;
                                            fw.pending_verify_keys(&peer_addr.ip())
                                        };
                                        let mut decrypted = false;
                                        for key in keys {
                                            if let Some(dec) = decrypt_kad_udp_packet(&buf[..len], key) {
                                                let cat = classify_ip(peer_addr.ip());
                                                let policy_allowed = match policy_mgr_clone.profile() {
                                                    NetworkProfile::LocalLab => cat == IpCategory::Loopback,
                                                    NetworkProfile::PublicEd2kPreview | NetworkProfile::PublicEd2kKadPreview => {
                                                        cat == IpCategory::PublicRoutable
                                                    }
                                                };
                                                if !policy_allowed {
                                                    break;
                                                }
                                                {
                                                    let mut rl = rate_limiter_arc.lock().await;
                                                    if !rl.check_and_record_opcode(peer_addr.ip(), 0xE3) {
                                                        break;
                                                    }
                                                }
                                                match KadCodec::decode(&dec.payload) {
                                                    Ok(msg) => {
                                                        tracing::debug!(
                                                            "Kad UDP: decrypted obfuscated packet from {} (recvKey=0x{:08X})",
                                                            log_peer_addr(peer_addr), dec.receiver_key
                                                        );
                                                        let local_kad_id_bytes =
                                                            local_id_arc.read().unwrap_or_else(|p| p.into_inner()).0;
                                                        let user_hash_bytes =
                                                            *user_hash_arc.read().unwrap_or_else(|p| p.into_inner());
                                                        let routing_table = routing_arc
                                                            .read()
                                                            .ok()
                                                            .and_then(|g| g.clone());
                                                        handlers::handle_incoming_message(
                                                            &socket_for_task,
                                                            peer_addr,
                                                            msg,
                                                            &in_flight_arc,
                                                            &local_kad_id_bytes,
                                                            &user_hash_bytes,
                                                            local_addr.port(),
                                                            &storage_arc,
                                                            &downloads_arc,
                                                            &firewall_arc,
                                                            &buddy_arc,
                                                            buddy_tcp_port_arc.load(Ordering::Relaxed),
                                                            buddy_connect_options_arc.load(Ordering::Relaxed),
                                                            routing_table,
                                                        ).await;
                                                        decrypted = true;
                                                    }
                                                    Err(e) => {
                                                        tracing::debug!(
                                                            "Kad UDP: decrypted datagram from {} failed decode: {}",
                                                            log_peer_addr(peer_addr), e
                                                        );
                                                    }
                                                }
                                                break;
                                            }
                                        }
                                        if !decrypted {
                                            // Ruido UDP de internet (escaneos,
                                            // otros protocolos P2P) — un warn
                                            // por datagrama saturaría el registro;
                                            // solo debug.
                                            tracing::debug!(
                                                "Kad UDP: dropped invalid protocol byte 0x{:02X} from {}",
                                                proto_byte,
                                                log_peer_addr(peer_addr)
                                            );
                                        }
                                    }
                                    continue;
                                }

                                // 2. Clasificación y conformidad de IP según directiva
                                let cat = classify_ip(peer_addr.ip());
                                let policy_allowed = match policy_mgr_clone.profile() {
                                    NetworkProfile::LocalLab => cat == IpCategory::Loopback,
                                    NetworkProfile::PublicEd2kPreview | NetworkProfile::PublicEd2kKadPreview => {
                                        cat == IpCategory::PublicRoutable
                                    }
                                };
                                if !policy_allowed {
                                    // Directiva interna por datagrama —
                                    // no es una señal de seguridad (debug).
                                    tracing::debug!("Kad UDP: dropped packet from non-conforming IP {} ({:?})", log_peer_ip(peer_addr.ip()), cat);
                                    continue;
                                }

                                // 3. Anti-amplificación / límite de tasa por IP y por opcode de origen
                                {
                                    let mut rl = rate_limiter_arc.lock().await;
                                    if !rl.check_and_record_opcode(peer_addr.ip(), opcode_byte) {
                                        let _ = policy_mgr_clone
                                            .ip_filter()
                                            .record_protocol_strike(peer_addr.ip(), "Kad UDP flood");
                                        tracing::debug!(
                                            "Kad UDP rate limit exceeded for {} (opcode=0x{:02X})",
                                            log_peer_ip(peer_addr.ip()),
                                            opcode_byte
                                        );
                                        continue;
                                    }
                                }

                                let packet_slice = &buf[..len];
                                tracing::debug!(
                                    "Kad UDP recv {} bytes from {} (proto=0x{:02X}, opcode=0x{:02X})",
                                    len,
                                    log_peer_addr(peer_addr),
                                    proto_byte,
                                    opcode_byte
                                );

                                // Una respuesta de búsqueda unificada 0x3B es ambigua:
                                // las respuestas de notas (SendValidNoteResult) transportan
                                // listas de etiquetas arbitrarias que el decodificador genérico
                                // aplana a entradas de palabras clave. Cuando una transacción
                                // pendiente de este par espera SearchNotesRes, se redecodifica
                                // la carga útil sin procesar en formato de notas para preservar
                                // las listas de etiquetas completas.
                                let decoded = match KadCodec::decode(packet_slice) {
                                    Ok(msg) => {
                                        if opcode_byte == KADEMLIA2_SEARCH_RES {
                                            let expects_notes = {
                                                let inf = in_flight_arc.lock().await;
                                                types::find_matching_tx_id(&inf, peer_addr, |tx| {
                                                    tx.expected_response
                                                        == KadExpectedResponse::SearchNotesRes
                                                })
                                                .is_some()
                                            };
                                            if expects_notes {
                                                match KadCodec::decode_search_notes_res(
                                                    proto_byte,
                                                    opcode_byte,
                                                    &packet_slice[2..],
                                                ) {
                                                    Ok(m) => Ok(m),
                                                    Err(e) => {
                                                        tracing::warn!(
                                                            "Kad UDP: notes-form redecode of 0x3B from {peer} failed: {e} — using generic form"
                                                        , peer = log_peer_addr(peer_addr));
                                                        Ok(msg)
                                                    }
                                                }
                                            } else {
                                                Ok(msg)
                                            }
                                        } else {
                                            Ok(msg)
                                        }
                                    }
                                    Err(e) => Err(e),
                                };

                                match decoded {
                                    Ok(msg) => {
                                        let local_kad_id_bytes =
                                            local_id_arc.read().unwrap_or_else(|p| p.into_inner()).0;
                                        let user_hash_bytes =
                                            *user_hash_arc.read().unwrap_or_else(|p| p.into_inner());
                                        let routing_table = routing_arc
                                            .read()
                                            .ok()
                                            .and_then(|g| g.clone());
                                        handlers::handle_incoming_message(
                                            &socket_for_task,
                                            peer_addr,
                                            msg,
                                            &in_flight_arc,
                                            &local_kad_id_bytes,
                                            &user_hash_bytes,
                                            local_addr.port(),
                                            &storage_arc,
                                            &downloads_arc,
                                            &firewall_arc,
                                            &buddy_arc,
                                            buddy_tcp_port_arc.load(Ordering::Relaxed),
                                            buddy_connect_options_arc.load(Ordering::Relaxed),
                                            routing_table,
                                        ).await;
                                    }
                                    Err(err) => {
                                        tracing::warn!(
                                            "Kad UDP: decode error from {} (proto=0x{:02X}, opcode=0x{:02X}, len={}): {} (prefix: {:02X?})",
                                            log_peer_addr(peer_addr),
                                            proto_byte,
                                            opcode_byte,
                                            len,
                                            err,
                                            &packet_slice[..packet_slice.len().min(16)]
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Kad UDP: recv_from error: {e}");
                                tokio::time::sleep(Duration::from_millis(10)).await;
                            }
                        }
                    }
                }
            }
        });

        inner.listen_task = Some(task);
        Ok(local_addr)
    }

    /// Detiene el socket UDP, libera permisos y cancela transacciones pendientes.
    pub async fn stop(&self) -> Result<(), String> {
        if let Some(svc) = self.client_udp.read().await.as_ref() {
            svc.clear_socket().await;
        }
        let mut inner = self.inner.write().await;
        if let Some(task) = inner.listen_task.take() {
            task.abort();
        }
        inner.socket = None;
        inner.bind_addr = None;
        inner.listen_permit = None;

        // Aborta todas las transacciones en curso
        let mut in_flight = inner.in_flight.lock().await;
        for (_, tx) in in_flight.drain() {
            let _ = tx.sender.send(Err("Kad UDP socket stopped".to_string()));
        }
        Ok(())
    }

    /// Devuelve una instantánea del estado del servicio Kad UDP.
    pub async fn get_snapshot(&self) -> KadUdpSnapshot {
        let inner = self.inner.read().await;
        let in_flight_count = {
            let inf = inner.in_flight.lock().await;
            inf.len()
        };

        KadUdpSnapshot {
            is_active: inner.socket.is_some(),
            bind_address: inner.bind_addr.map(|a| a.to_string()),
            packets_sent: self.packets_sent.load(Ordering::Relaxed),
            packets_received: self.packets_received.load(Ordering::Relaxed),
            active_transactions: in_flight_count,
        }
    }
}

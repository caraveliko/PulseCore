// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

pub mod firewall;
pub mod lookup;
pub mod nodes_dat;
pub mod search_keywords;
pub mod search_sources;
pub mod types;

use crate::services::peer_session::log_peer_addr;
use crate::services::peer_session::log_peer_ip;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::sync::Mutex;

use mule_core::kad::{KBucketTable, KadContact, KadId};
use mule_core::model::PeerSource;
use mule_filter::IpFilter;
use mule_network_policy::{
    classify_ip, CancellationToken, IpCategory, NetworkPolicyManager, NetworkProfile,
};

use crate::services::downloads::DownloadService;
use crate::services::kad_udp::KadUdpService;

pub use types::{KadContactEntryDto, KadStatusSnapshot, NodesDatInspectReport};

pub struct KadService {
    /// Identidad Kad local — mutable en caliente: `identityRotationHours`
    /// regenera el KadID sin reiniciar el servicio (compartido con
    /// `KadUdpService`).
    local_id: Arc<std::sync::RwLock<KadId>>,
    /// Compartido con `KadUdpService` (los paquetes entrantes alimentan la tabla y sirven
    /// respuestas REQ/BOOTSTRAP) — `Arc` porque el listener UDP es una tarea.
    routing_table: Arc<Mutex<KBucketTable>>,
    /// Nuestra IPv4 pública aprendida del HighID del servidor eD2K
    /// (`set_public_ipv4`) — anunciada en contactos `PUBLISH_SOURCE`. La
    /// dirección de enlace del socket puede ser `0.0.0.0` u ocultarse tras NAT.
    public_ipv4: std::sync::RwLock<Option<Ipv4Addr>>,
    downloads: Arc<Mutex<DownloadService>>,
    policy_mgr: NetworkPolicyManager,
    ip_filter: Arc<IpFilter>,
    udp: Arc<KadUdpService>,
    in_flight_searches: Arc<Mutex<std::collections::HashSet<String>>>,
    /// Puerto TCP eD2K local anunciado al publicarnos como fuente.
    ed2k_tcp_port: std::sync::atomic::AtomicU16,
    /// Hash de usuario eD2K local usado en los hellos de conexión para la prueba de cortafuegos.
    fw_user_hash: std::sync::RwLock<[u8; 16]>,
    /// Marca de tiempo del último `BOOTSTRAP_REQ` emitido — compartida entre los
    /// envíos inmediatos (handshakes eD2K) y el drenaje de la cola de
    /// candidatos, como `CKademlia::m_tBootstrap` de eMule.
    last_bootstrap: std::sync::Mutex<Option<std::time::Instant>>,
    /// Cola de candidatos bootstrap — equivalente de
    /// `CKademlia::s_liBootstrapList` (eMule 0.72a, Kademlia.cpp:287):
    /// los pares eD2K que anuncian un puerto UDP Kad se registran aquí, y luego el
    /// bucle de drenaje los prueba uno a uno mientras la tabla de enrutamiento
    /// continúe siendo escasa (~2 s entre intentos cuando está vacía, 15 s si no).
    /// Cada candidato solo se intenta una vez (`RemoveHead`); nuevos
    /// handshakes reabastecen la cola. Límite de 50 como en eMule.
    bootstrap_candidates: Mutex<std::collections::VecDeque<(SocketAddr, u8)>>,
    /// Canal de eventos broadcast + contador de secuencia compartido —
    /// inyectado por `set_event_channel` para emitir `kad.statusChanged`
    /// cuando la tabla de enrutamiento evoluciona fuera de las acciones RPC (paquetes
    /// entrantes, búsquedas, bootstrap, mantenimiento). No-op mientras el
    /// canal no esté conectado.
    event_channel: std::sync::RwLock<
        Option<(
            tokio::sync::broadcast::Sender<String>,
            Arc<std::sync::atomic::AtomicU64>,
        )>,
    >,
}

/// Intervalo entre rondas de republicación de **fuentes** Kad —
/// `KADEMLIAREPUBLISHTIMES` de eMule (`Opcodes.h:78`, 5 h).
const KAD_SOURCE_REPUBLISH_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(5 * 60 * 60);
/// Intervalo entre rondas de republicación de **palabras clave** Kad —
/// `KADEMLIAREPUBLISHTIMEK` de eMule (`Opcodes.h:80`, 24 h). eMule ejecuta la republicación de
/// fuentes y de palabras clave en temporizadores independientes (`m_tNextSrcPublishTime` /
/// `m_tNextKeyPublishTime`, `Kademlia.cpp`).
const KAD_KEYWORD_REPUBLISH_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(24 * 60 * 60);

impl KadService {
    pub fn new(
        local_id: KadId,
        downloads: Arc<Mutex<DownloadService>>,
        policy_mgr: NetworkPolicyManager,
        ip_filter: Arc<IpFilter>,
    ) -> Self {
        let local_id = Arc::new(std::sync::RwLock::new(local_id));
        let routing_table = Arc::new(Mutex::new(KBucketTable::new(*local_id.read().unwrap())));
        let udp = Arc::new(KadUdpService::new(
            Arc::clone(&local_id),
            policy_mgr.clone(),
        ));
        udp.set_routing_table(Arc::clone(&routing_table));
        Self {
            local_id,
            routing_table,
            public_ipv4: std::sync::RwLock::new(None),
            downloads,
            policy_mgr,
            ip_filter,
            udp,
            in_flight_searches: Arc::new(Mutex::new(std::collections::HashSet::new())),
            ed2k_tcp_port: std::sync::atomic::AtomicU16::new(0),
            fw_user_hash: std::sync::RwLock::new([0u8; 16]),
            last_bootstrap: std::sync::Mutex::new(None),
            bootstrap_candidates: Mutex::new(std::collections::VecDeque::new()),
            event_channel: std::sync::RwLock::new(None),
        }
    }

    /// Establece el puerto TCP eD2K local utilizado en los contactos de fuentes publicados.
    pub fn set_ed2k_tcp_port(&self, port: u16) {
        self.ed2k_tcp_port
            .store(port, std::sync::atomic::Ordering::Relaxed);
    }

    /// Inyecta el canal de eventos del daemon — activa la emisión de
    /// `kad.statusChanged` durante mutaciones autónomas de la tabla de
    /// enrutamiento (contactos aprendidos en segundo plano, expulsiones, rotación).
    pub fn set_event_channel(
        &self,
        event_tx: tokio::sync::broadcast::Sender<String>,
        sequence: Arc<std::sync::atomic::AtomicU64>,
    ) {
        *self.event_channel.write().unwrap() = Some((event_tx, sequence));
    }

    /// Emite un aviso `kad.statusChanged` si el canal está conectado —
    /// no-op en caso contrario (pruebas unitarias, fase de arranque antes de inyección).
    fn emit_status_changed(&self) {
        if let Some((tx, seq)) = self.event_channel.read().unwrap().as_ref() {
            crate::services::DaemonEvent::KadStatusChanged {
                sequence: seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            }
            .send(tx);
        }
    }

    /// Registra nuestra IPv4 pública (aprendida del HighID asignado por el servidor eD2K,
    /// o de un eco `FIREWALLED_RES` de Kad). `None` si es LowID/desconocida — la publicación
    /// de fuentes recurre entonces a la dirección de enlace del socket.
    pub fn set_public_ipv4(&self, ip: Option<Ipv4Addr>) {
        *self.public_ipv4.write().unwrap() = ip;
    }

    pub fn local_id(&self) -> KadId {
        *self.local_id.read().unwrap()
    }

    /// Regenera la identidad Kad local (`identityRotationHours`): nuevo
    /// KadID aleatorio propagado al servicio UDP, y tabla de enrutamiento reconstruida
    /// (las distancias XOR dependen del ID local — las entradas antiguas
    /// quedarían mal ordenadas).
    pub async fn rotate_local_id(&self) -> KadId {
        let new_id = KadId::random();
        *self.local_id.write().unwrap() = new_id;
        *self.routing_table.lock().await = KBucketTable::new(new_id);
        self.emit_status_changed();
        new_id
    }

    pub fn udp(&self) -> &Arc<KadUdpService> {
        &self.udp
    }

    /// Devuelve el contacto de la tabla de enrutamiento para `kad_id`, si se conoce.
    /// Utilizado para resolver un ID de buddy publicado a un endpoint UDP.
    pub async fn find_contact(
        &self,
        kad_id: &mule_core::kad::KadId,
    ) -> Option<mule_core::kad::KadContact> {
        self.routing_table.lock().await.get(kad_id).cloned()
    }

    pub async fn init_from_db(&self) {
        let dl = self.downloads.lock().await;
        if let Ok(contacts) = dl.db_ref().list_kad_contacts(1000) {
            let count = contacts.len();
            let mut table = self.routing_table.lock().await;
            for c in contacts {
                table.insert_or_update(c);
            }
            tracing::info!(
                "Kad routing table initialized with {} contacts from database",
                count
            );
        }
    }

    pub async fn get_status(&self) -> KadStatusSnapshot {
        // El guard de routing_table NO debe retenerse a través de los otros awaits
        // inferiores: get_snapshot bloquea in_flight y los manejadores recv UDP bloquean
        // in_flight → routing_table (learn_contact) — retener ambos aquí
        // crea un ciclo de orden de bloqueo que congelaba toda la canalización de Kad.
        let (total_contacts, total_replacements, active_buckets) = {
            let table = self.routing_table.lock().await;
            (
                table.total_contacts(),
                table.total_replacements(),
                table.bucket_distribution().len(),
            )
        };
        let db_count = {
            let dl = self.downloads.lock().await;
            dl.db_ref().count_kad_contacts().unwrap_or(0)
        };
        let udp_snap = self.udp.get_snapshot().await;
        let (fw_status, fw_verified, fw_extern, fw_effective) = {
            let tracker = self.udp.firewall_tracker();
            let fw = tracker.lock().await;
            let intern = udp_snap
                .bind_address
                .as_deref()
                .and_then(|a| a.rsplit(':').next())
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(0);
            (
                fw.status.as_str().to_string(),
                fw.verified,
                fw.extern_port,
                fw.effective_port(intern),
            )
        };

        KadStatusSnapshot {
            local_id: self.local_id().to_hex(),
            total_contacts,
            total_replacements,
            db_contacts_count: db_count,
            active_buckets,
            network_profile: self.policy_mgr.profile().to_string(),
            is_udp_active: udp_snap.is_active,
            udp_bind_address: udp_snap.bind_address,
            packets_sent: udp_snap.packets_sent,
            packets_received: udp_snap.packets_received,
            active_transactions: udp_snap.active_transactions,
            udp_firewall_status: fw_status,
            udp_firewall_verified: fw_verified,
            extern_udp_port: fw_extern,
            effective_udp_port: fw_effective,
        }
    }

    pub async fn ping_contact(
        &self,
        ip_str: &str,
        udp_port: u16,
        target_kad_id_hex: Option<&str>,
    ) -> Result<u64, String> {
        let ip: IpAddr = ip_str
            .parse()
            .map_err(|e| format!("Invalid IP address: {e}"))?;
        let addr = SocketAddr::new(ip, udp_port);

        let (_pong, latency) = self.udp.ping(addr).await?;

        // Tras un pong exitoso, promocionar / verificar contacto en KBucketTable y SQLite
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        if let IpAddr::V4(v4) = ip {
            let is_verified_id = target_kad_id_hex.is_some();
            let kad_id = target_kad_id_hex
                .and_then(KadId::from_hex)
                .unwrap_or_else(KadId::random);

            let contact = KadContact {
                kad_id,
                ip: v4,
                udp_port,
                tcp_port: None,
                version: 2,
                verified: is_verified_id,
                fail_count: 0,
                first_seen_unix_ms: now_ms,
                last_seen_unix_ms: now_ms,
            };

            // Actualizar tabla de enrutamiento en memoria
            {
                let mut table = self.routing_table.lock().await;
                table.insert_or_update(contact.clone());
            }

            // Actualizar SQLite
            {
                let mut dl = self.downloads.lock().await;
                let _ = dl.db_mut().batch_upsert_kad_contacts(&[contact]);
            }
        }

        Ok(latency.as_millis() as u64)
    }

    /// Realiza el bootstrap de Kad DHT desde un nodo destino (IP y puerto UDP).
    pub async fn bootstrap_from_node(&self, ip_str: &str, udp_port: u16) -> Result<usize, String> {
        let ip: IpAddr = ip_str
            .parse()
            .map_err(|e| format!("Invalid IP address: {e}"))?;
        let addr = SocketAddr::new(ip, udp_port);

        let received_contacts = self.udp.send_bootstrap(addr).await?;
        let count = received_contacts.len();
        let profile = self.policy_mgr.profile();
        let mut valid_contacts = Vec::new();

        for c in received_contacts {
            let cat = classify_ip(IpAddr::V4(c.ip));
            let policy_allowed = match profile {
                NetworkProfile::LocalLab => cat == IpCategory::Loopback,
                NetworkProfile::PublicEd2kPreview | NetworkProfile::PublicEd2kKadPreview => {
                    cat == IpCategory::PublicRoutable
                }
            };
            if policy_allowed && self.ip_filter.check_ip(IpAddr::V4(c.ip)).is_none() {
                valid_contacts.push(c);
            }
        }

        // `Process_KADEMLIA2_BOOTSTRAP_RES` de eMule:
        //   bool bAssumeVerified = GetRoutingZone()->GetNumContacts() == 0;
        // Los contactos de bootstrap se presumen verificados ÚNICAMENTE cuando la tabla está
        // vacía; después deben demostrar la posesión de la IP respondiendo a nuestras
        // peticiones o completando HELLO_RES_ACK. (Calculado antes de la inserción.)
        let assume_verified = {
            let table = self.routing_table.lock().await;
            table.total_contacts() == 0
        };
        for c in &mut valid_contacts {
            c.verified = assume_verified;
        }

        tracing::info!(
            "Kad Bootstrap from {}: received {} contacts, {} valid accepted into routing table (assume_verified={})",
            log_peer_addr(addr),
            count,
            valid_contacts.len(),
            assume_verified
        );

        // Insertar en la tabla de enrutamiento
        {
            let mut table = self.routing_table.lock().await;
            for c in &valid_contacts {
                table.insert_or_update(c.clone());
            }
        }

        // Persistir en SQLite
        if !valid_contacts.is_empty() {
            let mut dl = self.downloads.lock().await;
            let _ = dl.db_mut().batch_upsert_kad_contacts(&valid_contacts);
            // La tabla acaba de enriquecerse fuera de RPC — aviso inmediato para
            // que la insignia Kad de la interfaz refleje la conexión sin esperar
            // al siguiente tick de mantenimiento (30 s).
            self.emit_status_changed();
        }

        Ok(valid_contacts.len())
    }

    /// Número de candidatos bootstrap en espera en la cola de drenaje
    /// (diagnósticos y pruebas — `s_liBootstrapList.GetCount()` de eMule).
    pub async fn pending_bootstrap_candidates(&self) -> usize {
        self.bootstrap_candidates.lock().await.len()
    }

    /// Añade un candidato bootstrap a la cola de drenaje — deduplicado por
    /// dirección, acotado a 50 entradas como `s_liBootstrapList` (el más
    /// antiguo sale cuando la cola está llena).
    pub async fn add_bootstrap_candidate(&self, addr: SocketAddr, kad_version: u8) {
        let mut pool = self.bootstrap_candidates.lock().await;
        if pool.iter().any(|(a, _)| *a == addr) {
            return;
        }
        if pool.len() >= 50 {
            pool.pop_front();
        }
        pool.push_back((addr, kad_version));
    }

    /// Carga `<data_dir>/nodes.dat` en candidatos bootstrap — equivalente
    /// a eMule que lee el archivo de arranque en `s_liBootstrapList` al
    /// iniciar. Los contactos demuestran su vivacidad respondiendo al
    /// `BOOTSTRAP_REQ` del drenaje: ningún zombi entra en la tabla.
    /// No-op si el archivo está ausente o no es válido.
    pub async fn load_nodes_dat_candidates(&self, path: &std::path::Path) {
        let Ok(bytes) = tokio::fs::read(path).await else {
            return;
        };
        match nodes_dat::parse_valid_nodes_contacts(&bytes, &self.policy_mgr, &self.ip_filter) {
            Ok(contacts) => {
                let count = contacts.len();
                for c in contacts {
                    self.add_bootstrap_candidate(
                        SocketAddr::new(IpAddr::V4(c.ip), c.udp_port),
                        c.version,
                    )
                    .await;
                }
                if count > 0 {
                    tracing::info!(
                        "Kad: {count} bootstrap candidates loaded from {}",
                        path.display()
                    );
                }
            }
            Err(e) => {
                tracing::warn!("Kad: ignored nodes.dat at {}: {e}", path.display());
            }
        }
    }

    /// Registra un par eD2K que anuncia un puerto UDP Kad como candidato
    /// bootstrap y luego intenta un envío inmediato —
    /// `CKademlia::Bootstrap(ip, port)` de eMule: compuerta `!IsConnected()` + limitador
    /// global de 10 s. Si el envío falla, el candidato permanece en
    /// `bootstrap_candidates` y el bucle de drenaje lo reintenta (antes, un
    /// simple tiempo de espera dejaba Kad inactivo con una tabla vacía).
    /// `kad_version < 2` (`KADEMLIA_VERSION2_47a`) se ignora: estos pares
    /// no hablan paquetes KADEMLIA2.
    pub async fn maybe_bootstrap_from_peer(
        &self,
        ip: Ipv4Addr,
        kad_udp_port: u16,
        low_id: bool,
        kad_version: u8,
    ) {
        const PEER_BOOTSTRAP_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
        // Un par con LowID está tras cortafuegos: su puerto UDP Kad anunciado no es
        // accesible, por lo que una petición bootstrap solo expiraría.
        if low_id || kad_version < 2 {
            return;
        }
        let addr = SocketAddr::new(IpAddr::V4(ip), kad_udp_port);
        self.add_bootstrap_candidate(addr, kad_version).await;
        let contacts = {
            let table = self.routing_table.lock().await;
            table.total_contacts()
        };
        if contacts >= 50 {
            return;
        }
        {
            let mut last = self.last_bootstrap.lock().unwrap();
            if last
                .map(|t| t.elapsed() < PEER_BOOTSTRAP_MIN_INTERVAL)
                .unwrap_or(false)
            {
                return;
            }
            *last = Some(std::time::Instant::now());
        }
        tracing::info!(
            "Kad: bootstrapping from connected peer {peer}:{kad_udp_port} (routing table: {contacts} contacts)"
        , peer = log_peer_ip(ip));
        let _ = self
            .bootstrap_from_node(&ip.to_string(), kad_udp_port)
            .await;
    }

    /// Realiza el bootstrap de contactos activos enviando peticiones bootstrap a contactos semilla en paralelo.
    pub async fn bootstrap_active_contacts(&self, sample_size: usize) -> usize {
        let sample = {
            let table = self.routing_table.lock().await;
            table.find_closest(&self.local_id(), sample_size)
        };

        if sample.is_empty() {
            return 0;
        }

        tracing::info!(
            "Kad initiating background bootstrap across {} seed contacts...",
            sample.len()
        );

        let mut futs = Vec::with_capacity(sample.len());
        for c in sample {
            let addr = SocketAddr::new(IpAddr::V4(c.ip), c.udp_port);
            let udp = Arc::clone(&self.udp);
            futs.push(async move { (addr, udp.send_bootstrap(addr).await) });
        }

        let results = futures_util::future::join_all(futs).await;
        let profile = self.policy_mgr.profile();
        let mut all_valid = Vec::new();

        for (addr, res) in results {
            match res {
                Ok(contacts) => {
                    tracing::info!(
                        "Kad Bootstrap response from {}: received {} live contacts",
                        log_peer_addr(addr),
                        contacts.len()
                    );
                    for c in contacts {
                        let cat = classify_ip(IpAddr::V4(c.ip));
                        let policy_allowed = match profile {
                            NetworkProfile::LocalLab => cat == IpCategory::Loopback,
                            NetworkProfile::PublicEd2kPreview
                            | NetworkProfile::PublicEd2kKadPreview => {
                                cat == IpCategory::PublicRoutable
                            }
                        };
                        if policy_allowed && self.ip_filter.check_ip(IpAddr::V4(c.ip)).is_none() {
                            // Los contactos aprendidos de una respuesta bootstrap permanecen
                            // no verificados hasta que responden a una de nuestras peticiones
                            // (eMule solo los asume verificados en una tabla vacía,
                            // lo cual no puede ocurrir aquí — muestreamos semillas).
                            all_valid.push(c);
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        "Kad bootstrap attempt to {} failed/timed out: {}",
                        log_peer_addr(addr),
                        e
                    );
                }
            }
        }

        let total_received = all_valid.len();
        if !all_valid.is_empty() {
            // Actualizar tabla de enrutamiento
            {
                let mut table = self.routing_table.lock().await;
                for c in &all_valid {
                    table.insert_or_update(c.clone());
                }
            }
            // Persistir en SQLite
            {
                let mut dl = self.downloads.lock().await;
                let _ = dl.db_mut().batch_upsert_kad_contacts(&all_valid);
            }
            self.emit_status_changed();
        }

        tracing::info!(
            "Kad bootstrap cycle finished: {} new contacts added to routing table and database",
            total_received
        );
        total_received
    }

    /// Genera un bucle de mantenimiento en segundo plano para Kad DHT.
    /// Expande periódicamente la tabla de enrutamiento si el recuento de contactos verificados es inferior a 400
    /// mediante bootstrap a través de contactos semilla y descubriendo nodos en cubos distantes.
    pub fn start_background_maintenance(
        self: &Arc<Self>,
        cancel_token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        // Drenaje de la cola de candidatos bootstrap —
        // `CKademlia::Process` de eMule (Kademlia.cpp:287) vacía `s_liBootstrapList`
        // a razón de un `BOOTSTRAP_REQ` cada 2 s mientras la tabla
        // esté vacía, cada 15 s cuando contenga contactos pero siga
        // siendo escasa. Comparte `last_bootstrap` con los envíos inmediatos para
        // no enviar dos peticiones en ráfaga. Sin este bucle, un
        // tiempo de espera en el primer candidato dejaba a Kad inactivo con 0 contactos.
        let drain_kad = Arc::clone(self);
        let drain_cancel = cancel_token.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
            loop {
                tokio::select! {
                    _ = drain_cancel.cancelled() => break,
                    _ = tick.tick() => {
                        if !drain_kad.udp.get_snapshot().await.is_active {
                            continue;
                        }
                        // Compuerta `!IsConnected()` de eMule: `HasHadContact` —
                        // se drena mientras ningún contacto haya respondido
                        // jamás (verificado), incluso si la tabla contiene
                        // contactos no verificados (zombis importados).
                        let (contacts, has_verified) = {
                            let table = drain_kad.routing_table.lock().await;
                            let has_v = !table
                                .find_verified_closest(&drain_kad.local_id(), 1)
                                .is_empty();
                            (table.total_contacts(), has_v)
                        };
                        if has_verified {
                            continue;
                        }
                        let min_interval = if contacts == 0 {
                            std::time::Duration::from_secs(2)
                        } else {
                            std::time::Duration::from_secs(15)
                        };
                        {
                            let last = drain_kad.last_bootstrap.lock().unwrap();
                            if last
                                .map(|t| t.elapsed() < min_interval)
                                .unwrap_or(false)
                            {
                                continue;
                            }
                        }
                        let (candidate, remaining) = {
                            let mut pool = drain_kad.bootstrap_candidates.lock().await;
                            let c = pool.pop_front();
                            (c, pool.len())
                        };
                        let Some((addr, kad_version)) = candidate else {
                            continue;
                        };
                        *drain_kad.last_bootstrap.lock().unwrap() =
                            Some(std::time::Instant::now());
                        tracing::info!(
                            "Kad: bootstrap drain — trying {peer} (kad v{kad_version}, table {contacts} contacts, {remaining} candidates left)"
                        , peer = log_peer_addr(addr));
                        let _ = drain_kad
                            .bootstrap_from_node(&addr.ip().to_string(), addr.port())
                            .await;
                    }
                }
            }
        });
        let kad = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            // El primer tick se completa de inmediato; omitirlo para dejar finalizar el bootstrap de inicio
            interval.tick().await;
            let mut last_source_publish: Option<std::time::Instant> = None;
            let mut last_keyword_publish: Option<std::time::Instant> = None;
            // Contador de contactos ya notificado — solo se emite
            // `kad.statusChanged` cuando el valor observable cambia.
            let mut last_contacts: Option<usize> = None;

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        tracing::info!("Kad background maintenance task cancelled");
                        break;
                    }
                    _ = interval.tick() => {
                        let udp_snap = kad.udp.get_snapshot().await;
                        if !udp_snap.is_active {
                            continue;
                        }

                        let total_contacts = {
                            let table = kad.routing_table.lock().await;
                            table.total_contacts()
                        };

                        tracing::info!(
                            "[KAD_ROUTING] Kad routing table status: {} verified contacts in memory",
                            total_contacts
                        );

                        // La tabla también evoluciona fuera de RPC (learn_contact,
                        // búsquedas, bootstrap) — avisamos a la interfaz en cada
                        // cambio para que la insignia de la barra lateral se mantenga actualizada.
                        // Sin emisión en la primera observación: el
                        // cliente ya vuelve a solicitar el estado al conectarse, y
                        // un evento intempestivo ~30 s tras el inicio
                        // se intercalaría en las respuestas RPC en vuelo.
                        if let Some(prev) = last_contacts {
                            if prev != total_contacts {
                                kad.emit_status_changed();
                            }
                        }
                        last_contacts = Some(total_contacts);

                        if total_contacts > 0 && total_contacts < 400 {
                            tracing::info!(
                                "Kad background maintenance: routing table has {}/400 contacts, performing expansion...",
                                total_contacts
                            );
                            let added = kad.bootstrap_active_contacts(15).await;
                            tracing::debug!("Kad background maintenance bootstrap cycle added {} contacts", added);

                            // Realizar una búsqueda find_node aleatoria para explorar y poblar cubos distantes
                            let random_target = KadId::random();
                            let _ = kad.find_node(&random_target).await;
                        }

                        // Comprobación de accesibilidad del cortafuegos UDP (eMule la ejecuta cuando
                        // Kad pasa a conectado y vuelve a comprobar mientras no esté verificado).
                        // Se lanza en una tarea independiente para no bloquear el bucle de mantenimiento
                        // con el presupuesto de prueba de hasta 6 minutos.
                        let needs_fw_check = {
                            let tracker = kad.udp.firewall_tracker();
                            let fw = tracker.lock().await;
                            !fw.verified
                                && fw.status != crate::services::kad_udp::UdpFirewallStatus::Testing
                        };
                        if needs_fw_check && total_contacts > 0 {
                            let kad_fw = Arc::clone(&kad);
                            tokio::spawn(async move {
                                kad_fw.run_udp_firewall_check().await;
                            });
                        }

                        // `CKademlia::Process` → `RecheckFirewalled` de eMule
                        // (`m_tNextFirewallCheck`, cada hora): rearma la ronda
                        // TCP (`SetRecheckIP` — los HELLO entrantes vuelven
                        // a emitir FIREWALLED{2}_REQ) y relanza también la
                        // prueba UDP (`ReCheckFirewallUDP`).
                        let tcp_recheck_due = {
                            let tracker = kad.udp.firewall_tracker();
                            let mut fw = tracker.lock().await;
                            if std::time::Instant::now() >= fw.next_tcp_recheck {
                                fw.start_tcp_recheck();
                                true
                            } else {
                                false
                            }
                        };
                        if tcp_recheck_due && total_contacts > 0 {
                            tracing::info!("Kad: hourly firewall recheck — re-arming TCP + UDP rounds");
                            let kad_fw = Arc::clone(&kad);
                            tokio::spawn(async move {
                                kad_fw.run_udp_firewall_check().await;
                            });
                        }

                        // Republicación periódica de archivos compartidos — eMule ejecuta
                        // temporizadores independientes para fuentes (5 h) y palabras clave (24 h)
                        // (`Kademlia.cpp` `m_tNextSrcPublishTime` /
                        // `m_tNextKeyPublishTime`). Una primera publicación de cada
                        // tipo también se ejecuta en cuanto la tabla contiene contactos.
                        let source_due = last_source_publish
                            .map(|t| t.elapsed() >= KAD_SOURCE_REPUBLISH_INTERVAL)
                            .unwrap_or(true);
                        let keyword_due = last_keyword_publish
                            .map(|t| t.elapsed() >= KAD_KEYWORD_REPUBLISH_INTERVAL)
                            .unwrap_or(true);
                        if (source_due || keyword_due) && total_contacts > 0 {
                            let tcp_port = kad
                                .ed2k_tcp_port
                                .load(std::sync::atomic::Ordering::Relaxed);
                            let shared = {
                                let dl = kad.downloads.lock().await;
                                dl.list_all_shared_files().unwrap_or_default()
                            };
                            if tcp_port > 0 {
                                let mut published = 0usize;
                                for f in shared.iter().filter(|f| f.enabled).take(200) {
                                    let hash = f.file_hash.trim().to_string();
                                    // `CKnownFile::PublishSrc` de eMule: los archivos de partes
                                    // también se publican como fuentes — los pares pueden
                                    // descargar las partes que ya hemos completado.
                                    if source_due {
                                        if let Ok(n) = kad
                                            .publish_source(&hash, f.file_size, tcp_port)
                                            .await
                                        {
                                            published += n;
                                        }
                                    }
                                    // La publicación de palabras clave de eMule omite los archivos de partes
                                    // ("alguien más debería tener el archivo completo
                                    // para publicar estas palabras clave").
                                    if keyword_due && !f.is_partial {
                                        let _ = kad
                                            .publish_keyword(
                                                &hash,
                                                &f.file_name,
                                                f.file_size,
                                                f.aich_root,
                                            )
                                            .await;
                                    }
                                }
                                tracing::info!(
                                    "[KAD_PUBLISH] Republication round: {} file(s), {} successful source publishes (keywords={})",
                                    shared.iter().filter(|f| f.enabled).count(),
                                    published,
                                    keyword_due
                                );
                                let now = std::time::Instant::now();
                                if source_due {
                                    last_source_publish = Some(now);
                                }
                                if keyword_due {
                                    last_keyword_publish = Some(now);
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    /// Realiza una búsqueda iterativa de nodos (Find Node) para un ID Kad objetivo.
    ///
    /// Comparte `lookup::converge_closest_contacts` (consultas ALPHA en paralelo,
    /// tiempos de espera por petición, parada temprana por `SEARCHTOLERANCE`) — equivalente
    /// a la máquina de estados de búsqueda de eMule.
    pub async fn find_node(
        &self,
        target_kad_id: &KadId,
    ) -> Result<Vec<KadContactEntryDto>, String> {
        let contacts = lookup::converge_closest_contacts(
            target_kad_id,
            self.routing_table.as_ref(),
            &self.policy_mgr,
            &self.ip_filter,
            &self.udp,
            6,
        )
        .await;

        Ok(contacts
            .iter()
            .take(20)
            .map(KadContactEntryDto::from)
            .collect())
    }

    pub fn inspect_nodes_dat(&self, raw_bytes: &[u8]) -> Result<NodesDatInspectReport, String> {
        nodes_dat::inspect_nodes_dat(raw_bytes, &self.policy_mgr, &self.ip_filter)
    }

    pub async fn import_nodes_dat(&self, raw_bytes: &[u8]) -> Result<usize, String> {
        let imported_contacts =
            nodes_dat::parse_valid_nodes_contacts(raw_bytes, &self.policy_mgr, &self.ip_filter)?;

        // Persistir en SQLite
        {
            let mut dl = self.downloads.lock().await;
            let _ = dl.db_mut().batch_upsert_kad_contacts(&imported_contacts);
        }

        // Insertar en la tabla de enrutamiento en memoria
        let mut table = self.routing_table.lock().await;
        for c in &imported_contacts {
            table.insert_or_update(c.clone());
        }

        tracing::info!(
            "Kad imported {} valid contacts from nodes.dat into routing table and database",
            imported_contacts.len()
        );

        Ok(imported_contacts.len())
    }

    pub async fn list_contacts(&self, limit: usize) -> Vec<KadContactEntryDto> {
        let dl = self.downloads.lock().await;
        if let Ok(contacts) = dl.db_ref().list_kad_contacts(limit) {
            return contacts.iter().map(KadContactEntryDto::from).collect();
        }
        Vec::new()
    }

    pub async fn clear_contacts(&self) -> Result<usize, String> {
        let mut table = self.routing_table.lock().await;
        *table = KBucketTable::new(self.local_id());

        let mut dl = self.downloads.lock().await;
        dl.db_mut()
            .clear_kad_contacts()
            .map_err(|e| format!("Database error clearing kad contacts: {e}"))
    }

    /// Número de búsquedas de fuentes actualmente en vuelo — equivalente de
    /// `CKademlia::GetTotalFile()` (límite eMule `KADEMLIATOTALFILE`).
    pub async fn in_flight_file_searches(&self) -> usize {
        self.in_flight_searches.lock().await.len()
    }

    /// Búsqueda iterativa de fuentes de un hash de archivo objetivo en Kad DHT.
    /// Las fuentes descubiertas se filtran, se convierten a PeerSource (origen Kad) y se inyectan automáticamente en DownloadService si está presente.
    pub async fn search_sources(
        &self,
        file_hash_hex: &str,
        file_size: u64,
    ) -> Result<Vec<PeerSource>, String> {
        search_sources::search_sources_iterative(
            file_hash_hex,
            file_size,
            self.routing_table.as_ref(),
            &self.downloads,
            &self.policy_mgr,
            &self.ip_filter,
            &self.udp,
            &self.in_flight_searches,
        )
        .await
    }

    /// Publica este nodo como fuente para un hash de archivo determinado en los contactos Kad más cercanos.
    ///
    /// Los objetivos se convergen primero con una búsqueda iterativa `find_node`:
    /// los nodos eMule descartan silenciosamente `PUBLISH_SOURCE_REQ` cuando el hash del archivo queda
    /// fuera de `SEARCHTOLERANCE` de su KadID, y solo responden
    /// `PUBLISH_RES` cuando la entrada fue realmente almacenada — publicar a ciegas
    /// desde la tabla local casi siempre agota el tiempo de espera.
    pub async fn publish_source(
        &self,
        file_hash_hex: &str,
        file_size: u64,
        tcp_port: u16,
    ) -> Result<usize, String> {
        let target_kad_id = KadId::from_hash_hex(file_hash_hex)
            .ok_or_else(|| format!("Invalid target file hash hex: {file_hash_hex}"))?;

        let candidates = lookup::converge_closest_contacts(
            &target_kad_id,
            self.routing_table.as_ref(),
            &self.policy_mgr,
            &self.ip_filter,
            &self.udp,
            4,
        )
        .await;

        let mut targets: Vec<_> = candidates
            .iter()
            .filter(|c| c.kad_id.is_within_search_tolerance(&target_kad_id))
            .take(10)
            .cloned()
            .collect();
        if targets.is_empty() {
            targets = candidates.into_iter().take(5).collect();
        }

        if targets.is_empty() {
            return Ok(0);
        }

        // Anunciar nuestra IPv4 *pública* (HighID eD2K / eco FIREWALLED_RES) —
        // los indexadores rechazan contactos de fuentes cuya IP difiera de la del remitente
        // (véase nuestra propia comprobación en PublishSourceReq), por lo que la dirección
        // de enlace del socket (0.0.0.0 o tras NAT) es inutilizable. Lo mismo para el puerto UDP:
        // anunciar el puerto externo una vez que la prueba de cortafuegos demuestre su accesibilidad.
        let intern_udp = self.udp.local_udp_port().await.unwrap_or(0);
        let (fw_observed_ip, advertised_udp_port, udp_verified_open) = {
            let tracker = self.udp.firewall_tracker();
            let fw = tracker.lock().await;
            (
                fw.observed_ip,
                fw.effective_port(intern_udp),
                fw.verified && fw.status == crate::services::kad_udp::UdpFirewallStatus::Open,
            )
        };
        let bind_ip = self.udp.bind_addr().await.and_then(|a| match a.ip() {
            IpAddr::V4(v4) if !v4.is_unspecified() => Some(v4),
            _ => None,
        });
        let has_high_id = self.public_ipv4.read().unwrap().is_some();
        let Some(advertised_ip) = self
            .public_ipv4
            .read()
            .unwrap()
            .or(fw_observed_ip)
            .or(bind_ip)
        else {
            tracing::warn!(
                "Kad publish_source for {} skipped: public IPv4 unknown (no HighID, no FIREWALLED_RES, unspecified bind)",
                file_hash_hex
            );
            return Ok(0);
        };

        // `GetMyConnectOptions(true, true)` (OtherFunctions.cpp:4037,
        // Search.cpp:692): bits 0-2 = capa criptográfica (byte ya calculado al
        // inicio para `FINDBUDDY_RES`), bit 3 = `DirectUDPCallback`
        // anunciado cuando TCP es inaccesible (sin HighID eD2K) pero
        // el UDP Kad se ha probado abierto — el par puede entonces enviarnos
        // `OP_DIRECTCALLBACKREQ` para que conectemos hacia él.
        let connect_options: u8 = (self.udp.buddy_connect_options() & 0x07)
            | (((!has_high_id) && udp_verified_open) as u8) << 3;

        let total_queried = targets.len();
        let mut futs = Vec::with_capacity(total_queried);
        for contact in targets {
            let addr = SocketAddr::new(IpAddr::V4(contact.ip), contact.udp_port);
            let udp = Arc::clone(&self.udp);
            let target = target_kad_id.0;
            futs.push(async move {
                udp.send_publish_source(
                    addr,
                    &target,
                    file_size,
                    tcp_port,
                    advertised_ip,
                    advertised_udp_port,
                    connect_options,
                )
                .await
            });
        }

        let mut success_count = 0;
        for res in futures_util::future::join_all(futs).await {
            if let Ok(0) = res {
                success_count += 1;
            }
        }

        tracing::info!(
            "[KAD_PUBLISH] Published source for file {} ({} bytes, port {}) to {}/{} closest Kad nodes",
            file_hash_hex,
            file_size,
            tcp_port,
            success_count,
            total_queried
        );

        Ok(success_count)
    }

    /// Busca en Kad DHT entradas de palabras clave que coincidan con un término de búsqueda.
    pub async fn search_keywords(
        &self,
        term: &str,
        min_size: Option<u64>,
        max_size: Option<u64>,
    ) -> Result<Vec<serde_json::Value>, String> {
        search_keywords::search_keywords_iterative(
            term,
            min_size,
            max_size,
            self.routing_table.as_ref(),
            &self.policy_mgr,
            &self.ip_filter,
            &self.udp,
        )
        .await
    }

    /// Publica entradas de palabras clave para un archivo en los contactos Kad más cercanos.
    /// `aich_root` se adjunta como `TAG_KADAICHHASHPUB` solo a objetivos v9+
    /// (`PreparePacketForTags` de eMule).
    pub async fn publish_keyword(
        &self,
        file_hash_hex: &str,
        file_name: &str,
        file_size: u64,
        aich_root: Option<[u8; 20]>,
    ) -> Result<usize, String> {
        search_keywords::publish_keyword(
            file_hash_hex,
            file_name,
            file_size,
            aich_root,
            self.routing_table.as_ref(),
            &self.policy_mgr,
            &self.ip_filter,
            &self.udp,
        )
        .await
    }

    /// Busca en Kad DHT notas de archivos (comentarios/valoraciones): envía
    /// `KADEMLIA2_SEARCH_NOTES_REQ` (0x35) a los contactos más cercanos al
    /// hash del archivo y agrega las listas de etiquetas `CEntry` devueltas, deduplicadas por
    /// el `source_id` del publicador. Devuelto como JSON seguro para visualización.
    pub async fn search_notes(
        &self,
        file_hash_hex: &str,
        file_size: u64,
    ) -> Result<Vec<serde_json::Value>, String> {
        let target_kad_id = KadId::from_hash_hex(file_hash_hex)
            .ok_or_else(|| format!("Invalid target file hash hex: {file_hash_hex}"))?;

        // Converger primero (como en search_sources/publish): los indexadores eMule solo
        // responden dentro de SEARCHTOLERANCE de la clave — la tabla local sola
        // golpearía principalmente nodos fuera de tolerancia que descartan la petición.
        let candidates = lookup::converge_closest_contacts(
            &target_kad_id,
            self.routing_table.as_ref(),
            &self.policy_mgr,
            &self.ip_filter,
            &self.udp,
            4,
        )
        .await;
        let mut closest: Vec<_> = candidates
            .iter()
            .filter(|c| c.kad_id.is_within_search_tolerance(&target_kad_id))
            .take(10)
            .cloned()
            .collect();
        if closest.is_empty() {
            closest = candidates.into_iter().take(5).collect();
        }
        if closest.is_empty() {
            return Ok(Vec::new());
        }

        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for contact in closest {
            let addr = SocketAddr::new(IpAddr::V4(contact.ip), contact.udp_port);
            let Ok(notes) = self
                .udp
                .send_search_notes(addr, &target_kad_id.0, file_size)
                .await
            else {
                continue;
            };
            for n in notes {
                if !seen.insert(n.source_id) {
                    continue;
                }
                out.push(serde_json::json!({
                    "sourceId": hex::encode(n.source_id),
                    "fileName": n.file_name(),
                    "fileSize": n.file_size(),
                    "comment": n.comment(),
                    "rating": n.rating(),
                }));
            }
        }
        tracing::info!(
            "Kad search_notes for {}: {} unique notes collected",
            file_hash_hex,
            out.len()
        );
        Ok(out)
    }

    /// Publica una nota de archivo local (comentario + valoración opcional) en los contactos Kad
    /// más cercanos al hash del archivo (`KADEMLIA2_PUBLISH_NOTES_REQ`,
    /// 0x45). Devuelve el número de indexadores que aceptaron.
    pub async fn publish_note(
        &self,
        file_hash_hex: &str,
        file_name: &str,
        file_size: u64,
        comment: &str,
        rating: Option<u8>,
    ) -> Result<usize, String> {
        use mule_protocol::ed2k::tags::{Ed2kTag, TagName, TagValue};
        use mule_protocol::kad::{
            TAG_NOTE_DESCRIPTION, TAG_NOTE_FILENAME, TAG_NOTE_FILERATING, TAG_NOTE_FILESIZE,
        };

        let target_kad_id = KadId::from_hash_hex(file_hash_hex)
            .ok_or_else(|| format!("Invalid target file hash hex: {file_hash_hex}"))?;
        if comment.len() > 512 || file_name.len() > 255 {
            return Err("comment exceeds 512 bytes or file_name exceeds 255 bytes".to_string());
        }

        let mut tags = vec![
            Ed2kTag {
                name: TagName::Id(TAG_NOTE_FILENAME),
                value: TagValue::String(file_name.to_string()),
            },
            Ed2kTag {
                name: TagName::Id(TAG_NOTE_FILESIZE),
                value: TagValue::UInt64(file_size),
            },
            Ed2kTag {
                name: TagName::Id(TAG_NOTE_DESCRIPTION),
                value: TagValue::String(comment.to_string()),
            },
        ];
        if let Some(r) = rating {
            tags.push(Ed2kTag {
                name: TagName::Id(TAG_NOTE_FILERATING),
                value: TagValue::UInt8(r.min(5)),
            });
        }

        // Converger primero — los indexadores descartan silenciosamente las publicaciones para claves fuera
        // de SEARCHTOLERANCE de su KadID.
        let candidates = lookup::converge_closest_contacts(
            &target_kad_id,
            self.routing_table.as_ref(),
            &self.policy_mgr,
            &self.ip_filter,
            &self.udp,
            4,
        )
        .await;
        let mut closest: Vec<_> = candidates
            .iter()
            .filter(|c| c.kad_id.is_within_search_tolerance(&target_kad_id))
            .take(10)
            .cloned()
            .collect();
        if closest.is_empty() {
            closest = candidates.into_iter().take(5).collect();
        }
        let total_queried = closest.len();
        if total_queried == 0 {
            return Ok(0);
        }

        let mut success = 0;
        for contact in closest {
            let addr = SocketAddr::new(IpAddr::V4(contact.ip), contact.udp_port);
            if self
                .udp
                .send_publish_notes(addr, &target_kad_id.0, &self.local_id().0, &tags)
                .await
                .is_ok()
            {
                success += 1;
            }
        }
        tracing::info!(
            "Kad publish_note for {}: accepted by {}/{} closest nodes",
            file_hash_hex,
            success,
            total_queried
        );
        Ok(success)
    }
}

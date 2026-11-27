// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Sabueso DHT Mainline (BEP 5 + BEP 51 `sample_infohashes` + BEP 9 `ut_metadata` + Rendezvous eD2K).
//!
//! Descubre torrents de vídeo vivos en BitTorrent sin ningún indexador externo, respetando el
//! interruptor de corte, los perfiles de red (`LocalLab` / público), el filtrado IP y los límites de tasa.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mule_network_policy::validator::validate_connect_address;
use mule_network_policy::{NetworkPolicyManager, NetworkState};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex, Semaphore};

use super::content_filter::compute_language_affinity_score;
use super::ut_metadata::{fetch_video_metadata_from_peer, DiscoveredVideoTorrent};
use super::wire::{
    decode_krpc_packet, derive_ed2k_rendezvous_id, encode_announce_peer, encode_find_node,
    encode_find_node_response, encode_get_peers, encode_get_peers_response,
    encode_sample_infohashes, encode_simple_response, KrpcMessage,
};

/// Nodos públicos de arranque estándar de Mainline DHT (usados solo en perfiles con red pública).
pub const MAINLINE_DHT_BOOTSTRAP_HOSTS: &[(&str, u16)] = &[
    ("router.bittorrent.com", 6881),
    ("dht.transmissionbt.com", 6881),
    ("router.utorrent.com", 6881),
];
/// Intervalo de sondeo del Sabueso DHT (450 ms — balance óptimo entre velocidad de cosecha y CPU en RPi).
const HARVEST_TICK_INTERVAL: Duration = Duration::from_millis(450);
/// Número máximo de paquetes UDP de sondeo emitidos por tick del Sabueso DHT.
const MAX_PROBES_PER_TICK: usize = 10;
/// Tamaño máximo de la frontera de nodos DHT conocidos en memoria.
const MAX_ROUTING_NODES: usize = 2_048;
/// Tamaño máximo de la cola VIP de nodos que han demostrado devolver pares reales (`get_peers`).
const MAX_BEP51_VIP_NODES: usize = 512;
/// Mínimo de nodos VIP acumulados antes de priorizarlos (evita martillear los 2–3 primeros nodos).
const MIN_VIP_NODES_BEFORE_PREFER: usize = 32;
/// Tamaño máximo de la cola de afinidad (vecindades de idioma español/Filmoteca, pares `ut_pex` y eD2K).
const MAX_AFFINITY_NODES: usize = 1_024;
/// Mínimo de nodos de afinidad acumulados antes de dedicar sondeos preferentes a esa vecindad.
const MIN_AFFINITY_NODES_BEFORE_PREFER: usize = 4;
/// Puertos BitTorrent/DHT habituales sondeados en vecindad cuando se promocionan IPs de pares eD2K.
const DEFAULT_ED2K_NEIGHBOR_BT_PORTS: &[u16] = &[6881, 6882];
/// Número de vecinos reales devueltos en respuestas `find_node` / `get_peers` (estándar Kademlia K=8).
const DHT_REPLY_NEIGHBOR_COUNT: usize = 8;
/// Tamaño máximo de la caché LRU de infohashes ya inspeccionados.
const MAX_SEEN_INFOHASHES: usize = 16_384;
/// Concurrencia máxima de descargas BEP 9 `ut_metadata` simultáneas.
const MAX_CONCURRENT_METADATA_FETCHES: usize = 16;
/// Número máximo de pares probados por cada `info_hash` durante la descarga `ut_metadata`.
const MAX_PEERS_PER_METADATA_FETCH: usize = 4;
/// Capacidad del canal de torrents de vídeo descubiertos hacia el servicio híbrido.
const DISCOVERY_CHANNEL_CAPACITY: usize = 256;
/// Tamaño del búfer de lectura UDP para datagramas KRPC.
const UDP_RECV_BUFFER_BYTES: usize = 4_096;
/// Tiempo de espera para consultas puntuales de `get_peers` (Rendezvous o resolución bajo demanda).
const ON_DEMAND_QUERY_TIMEOUT: Duration = Duration::from_secs(3);
/// Máximo de nodos consultados por ronda en una búsqueda puntual `get_peers` o Rendezvous.
const MAX_ON_DEMAND_FANOUT: usize = 16;
/// Número máximo de rondas iterativas Kademlia (distancia XOR) por búsqueda bajo demanda.
const MAX_ON_DEMAND_ROUNDS: usize = 3;
/// Número máximo total de nodos consultados en una búsqueda iterativa bajo demanda.
const MAX_ON_DEMAND_QUERIED_NODES: usize = 48;
/// Tiempo de espera por ronda iterativa de `get_peers` bajo demanda.
const ON_DEMAND_ROUND_TIMEOUT: Duration = Duration::from_millis(1_100);
/// Número de pares a partir del cual una búsqueda bajo demanda se da por satisfecha.
const ON_DEMAND_SUFFICIENT_PEERS: usize = 8;
/// Límite de pares guardados por `info_hash` en el almacén efímero local del nodo DHT.
const MAX_LOCAL_PEERS_PER_HASH: usize = 32;
/// Límite de `info_hash` guardados en el almacén efímero local de anuncios entrantes.
const MAX_LOCAL_ANNOUNCE_HASHES: usize = 512;
/// Token local devuelto a pares que consultan `get_peers` contra nuestro nodo DHT.
const LOCAL_DHT_TOKEN: &[u8] = b"pc";
/// Enfriamiento mínimo antes de volver a enviar `sample_infohashes` al mismo nodo (BEP 51 cachea muestras varios minutos).
const BEP51_NODE_SAMPLE_COOLDOWN: Duration = Duration::from_secs(300);
/// Tamaño máximo del mapa de enfriamiento de nodos muestreados BEP 51.
const MAX_SAMPLED_NODE_COOLDOWNS: usize = 8_192;
/// Máximo de candidatos inspeccionados por cola en cada tick al buscar un nodo fuera de enfriamiento.
const MAX_COOLDOWN_SCAN_PER_PROBE: usize = 32;

/// Instantánea observable de actividad del Sabueso DHT Mainline.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DhtHarvesterStats {
    pub running: bool,
    pub listen_addr: Option<String>,
    pub routing_nodes: usize,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub infohashes_sampled: u64,
    pub passive_announces: u64,
    pub metadata_attempts: u64,
    pub video_torrents_discovered: u64,
    pub non_video_or_failed: u64,
    pub rendezvous_queries: u64,
    pub pex_peers_discovered: u64,
    pub native_ed2k_tags_found: u64,
    pub affinity_nodes: usize,
}

#[derive(Default)]
struct HarvesterCounters {
    running: AtomicBool,
    packets_sent: AtomicU64,
    packets_received: AtomicU64,
    infohashes_sampled: AtomicU64,
    passive_announces: AtomicU64,
    metadata_attempts: AtomicU64,
    video_torrents_discovered: AtomicU64,
    non_video_or_failed: AtomicU64,
    rendezvous_queries: AtomicU64,
    pex_peers_discovered: AtomicU64,
    native_ed2k_tags_found: AtomicU64,
}

/// Respuesta interna de una consulta puntual `get_peers` (`(pares, token_opcional, nodo_emisor, vecinos)`).
type OnDemandPeersReply = (
    Vec<SocketAddr>,
    Option<Vec<u8>>,
    SocketAddr,
    Vec<([u8; 20], SocketAddr)>,
);

/// Calcula la distancia XOR Kademlia de 160 bits entre dos identificadores DHT.
fn xor_distance(a: &[u8; 20], b: &[u8; 20]) -> [u8; 20] {
    let mut out = [0u8; 20];
    for (idx, byte) in out.iter_mut().enumerate() {
        *byte = a[idx] ^ b[idx];
    }
    out
}

struct PendingGetPeers {
    info_hash: [u8; 20],
    source: &'static str,
    created_at: Instant,
    responder: Option<mpsc::Sender<OnDemandPeersReply>>,
}

struct HarvesterState {
    routing_queue: VecDeque<SocketAddr>,
    routing_set: HashSet<SocketAddr>,
    /// Identificadores DHT de 20 bytes conocidos por dirección para responder `find_node`/`get_peers` con vecinos reales.
    node_ids: HashMap<SocketAddr, [u8; 20]>,
    /// Cola VIP de nodos que han respondido con muestras BEP 51 reales.
    bep51_vip_queue: VecDeque<SocketAddr>,
    bep51_vip_set: HashSet<SocketAddr>,
    /// Cola de afinidad de máxima prioridad (vecindades en español/Filmoteca + pares `ut_pex`).
    affinity_queue: VecDeque<SocketAddr>,
    affinity_set: HashSet<SocketAddr>,
    seen_infohashes: HashSet<[u8; 20]>,
    seen_order: VecDeque<[u8; 20]>,
    /// Registro temporal de nodos recientemente sondeados con BEP 51 para evitar re-muestrear cachés repetidas.
    sampled_nodes: HashMap<SocketAddr, Instant>,
    pending_get_peers: HashMap<Vec<u8>, PendingGetPeers>,
    local_peers: HashMap<[u8; 20], Vec<SocketAddr>>,
    tx_counter: u32,
    probe_counter: u64,
}

impl HarvesterState {
    fn new() -> Self {
        Self {
            routing_queue: VecDeque::new(),
            routing_set: HashSet::new(),
            node_ids: HashMap::new(),
            bep51_vip_queue: VecDeque::new(),
            bep51_vip_set: HashSet::new(),
            affinity_queue: VecDeque::new(),
            affinity_set: HashSet::new(),
            seen_infohashes: HashSet::new(),
            seen_order: VecDeque::new(),
            sampled_nodes: HashMap::new(),
            pending_get_peers: HashMap::new(),
            local_peers: HashMap::new(),
            tx_counter: 1,
            probe_counter: 0,
        }
    }

    fn next_tx_id(&mut self) -> Vec<u8> {
        self.tx_counter = self.tx_counter.wrapping_add(1);
        self.tx_counter.to_be_bytes().to_vec()
    }

    fn insert_node(&mut self, addr: SocketAddr) {
        self.insert_node_with_id(addr, None);
    }

    fn insert_node_with_id(&mut self, addr: SocketAddr, id: Option<[u8; 20]>) {
        if addr.port() == 0 {
            return;
        }
        if let Some(node_id) = id {
            self.node_ids.insert(addr, node_id);
        }
        if self.routing_set.contains(&addr) {
            return;
        }
        if self.routing_queue.len() >= MAX_ROUTING_NODES {
            if let Some(old) = self.routing_queue.pop_back() {
                self.routing_set.remove(&old);
                self.node_ids.remove(&old);
            }
        }
        self.routing_set.insert(addr);
        self.routing_queue.push_front(addr);
    }

    fn promote_bep51_vip(&mut self, addr: SocketAddr) {
        if addr.port() == 0 || self.bep51_vip_set.contains(&addr) {
            return;
        }
        if self.bep51_vip_queue.len() >= MAX_BEP51_VIP_NODES {
            if let Some(old) = self.bep51_vip_queue.pop_front() {
                self.bep51_vip_set.remove(&old);
            }
        }
        self.bep51_vip_set.insert(addr);
        self.bep51_vip_queue.push_back(addr);
    }

    fn promote_affinity_node(&mut self, addr: SocketAddr) {
        if addr.port() == 0 || self.affinity_set.contains(&addr) {
            return;
        }
        if self.affinity_queue.len() >= MAX_AFFINITY_NODES {
            if let Some(old) = self.affinity_queue.pop_front() {
                self.affinity_set.remove(&old);
            }
        }
        self.affinity_set.insert(addr);
        self.affinity_queue.push_back(addr);
    }

    fn record_node_sampled(&mut self, addr: SocketAddr) {
        if self.sampled_nodes.len() >= MAX_SAMPLED_NODE_COOLDOWNS
            && !self.sampled_nodes.contains_key(&addr)
        {
            self.sampled_nodes
                .retain(|_, last| last.elapsed() < BEP51_NODE_SAMPLE_COOLDOWN);
            if self.sampled_nodes.len() >= MAX_SAMPLED_NODE_COOLDOWNS {
                if let Some(oldest_key) = self
                    .sampled_nodes
                    .iter()
                    .min_by_key(|(_, t)| **t)
                    .map(|(k, _)| *k)
                {
                    self.sampled_nodes.remove(&oldest_key);
                }
            }
        }
        self.sampled_nodes.insert(addr, Instant::now());
    }

    fn pop_ready_from_queue(
        queue: &mut VecDeque<SocketAddr>,
        sampled_nodes: &HashMap<SocketAddr, Instant>,
    ) -> Option<SocketAddr> {
        let attempts = queue.len().min(MAX_COOLDOWN_SCAN_PER_PROBE);
        for _ in 0..attempts {
            let addr = queue.pop_front()?;
            queue.push_back(addr);
            let in_cooldown = sampled_nodes
                .get(&addr)
                .is_some_and(|last| last.elapsed() < BEP51_NODE_SAMPLE_COOLDOWN);
            if !in_cooldown {
                return Some(addr);
            }
        }
        None
    }

    fn pop_next_probe_node(
        &mut self,
        prefer_vip: bool,
        prefer_affinity: bool,
        for_sample: bool,
    ) -> Option<SocketAddr> {
        if !for_sample {
            let addr = self.routing_queue.pop_front()?;
            self.routing_queue.push_back(addr);
            return Some(addr);
        }
        if prefer_affinity && self.affinity_queue.len() >= MIN_AFFINITY_NODES_BEFORE_PREFER {
            if let Some(aff) =
                Self::pop_ready_from_queue(&mut self.affinity_queue, &self.sampled_nodes)
            {
                return Some(aff);
            }
        }
        if prefer_vip && self.bep51_vip_queue.len() >= MIN_VIP_NODES_BEFORE_PREFER {
            if let Some(vip) =
                Self::pop_ready_from_queue(&mut self.bep51_vip_queue, &self.sampled_nodes)
            {
                return Some(vip);
            }
        }
        Self::pop_ready_from_queue(&mut self.routing_queue, &self.sampled_nodes)
    }

    /// Devuelve hasta `DHT_REPLY_NEIGHBOR_COUNT` vecinos reales con ID conocido para responder `find_node`/`get_peers`.
    fn sample_reply_neighbors(&self, exclude: SocketAddr) -> Vec<([u8; 20], SocketAddr)> {
        let mut out = Vec::with_capacity(DHT_REPLY_NEIGHBOR_COUNT);
        for (&addr, &id) in &self.node_ids {
            if addr != exclude && addr.is_ipv4() && addr.port() > 0 {
                out.push((id, addr));
                if out.len() >= DHT_REPLY_NEIGHBOR_COUNT {
                    break;
                }
            }
        }
        out
    }

    fn mark_infohash_seen(&mut self, hash: [u8; 20]) -> bool {
        if self.seen_infohashes.contains(&hash) {
            return false;
        }
        if self.seen_order.len() >= MAX_SEEN_INFOHASHES {
            if let Some(old) = self.seen_order.pop_front() {
                self.seen_infohashes.remove(&old);
            }
        }
        self.seen_infohashes.insert(hash);
        self.seen_order.push_back(hash);
        true
    }

    fn record_local_peer(&mut self, hash: [u8; 20], peer: SocketAddr) {
        if !self.local_peers.contains_key(&hash) && self.local_peers.len() >= MAX_LOCAL_ANNOUNCE_HASHES
        {
            if let Some(first_key) = self.local_peers.keys().next().copied() {
                self.local_peers.remove(&first_key);
            }
        }
        let list = self.local_peers.entry(hash).or_default();
        if !list.contains(&peer) && list.len() < MAX_LOCAL_PEERS_PER_HASH {
            list.push(peer);
        }
    }

    fn prune_stale_pending(&mut self) {
        let cutoff = Duration::from_secs(12);
        self.pending_get_peers
            .retain(|_, p| p.created_at.elapsed() < cutoff);
        self.sampled_nodes
            .retain(|_, last| last.elapsed() < BEP51_NODE_SAMPLE_COOLDOWN);
    }
}

/// Instancia manejable del Sabueso DHT Mainline (BEP 5/51 + BEP 9 + Rendezvous).
#[derive(Clone)]
pub struct DhtHarvester {
    node_id: [u8; 20],
    socket: Arc<UdpSocket>,
    listen_addr: SocketAddr,
    policy: Arc<NetworkPolicyManager>,
    state: Arc<Mutex<HarvesterState>>,
    counters: Arc<HarvesterCounters>,
    metadata_sem: Arc<Semaphore>,
    discovery_tx: mpsc::Sender<DiscoveredVideoTorrent>,
}

impl DhtHarvester {
    /// Arranca el Sabueso DHT enlazado a la interfaz permitida por el perfil de red actual
    /// y devuelve `(handle, receptor_torrents_video)`.
    pub async fn start(
        policy: Arc<NetworkPolicyManager>,
        shutdown: Arc<tokio::sync::Notify>,
    ) -> Result<(Self, mpsc::Receiver<DiscoveredVideoTorrent>), std::io::Error> {
        let bind_ip = if policy.profile().is_loopback_only() {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        };
        Self::start_on_addr(SocketAddr::new(bind_ip, 0), policy, shutdown).await
    }

    /// Arranca el Sabueso DHT en una dirección de escucha concreta (útil en pruebas loopback).
    pub async fn start_on_addr(
        bind_addr: SocketAddr,
        policy: Arc<NetworkPolicyManager>,
        shutdown: Arc<tokio::sync::Notify>,
    ) -> Result<(Self, mpsc::Receiver<DiscoveredVideoTorrent>), std::io::Error> {
        let socket = Arc::new(UdpSocket::bind(bind_addr).await?);
        let listen_addr = socket.local_addr()?;

        let mut node_id = [0u8; 20];
        rand::rng().fill_bytes(&mut node_id);

        let (discovery_tx, discovery_rx) = mpsc::channel(DISCOVERY_CHANNEL_CAPACITY);
        let harvester = Self {
            node_id,
            socket,
            listen_addr,
            policy,
            state: Arc::new(Mutex::new(HarvesterState::new())),
            counters: Arc::new(HarvesterCounters::default()),
            metadata_sem: Arc::new(Semaphore::new(MAX_CONCURRENT_METADATA_FETCHES)),
            discovery_tx,
        };
        harvester.counters.running.store(true, Ordering::Relaxed);

        let recv_worker = harvester.clone();
        let recv_shutdown = Arc::clone(&shutdown);
        tokio::spawn(async move {
            recv_worker.run_recv_loop(recv_shutdown).await;
        });

        let probe_worker = harvester.clone();
        let probe_shutdown = Arc::clone(&shutdown);
        tokio::spawn(async move {
            probe_worker.run_probe_loop(probe_shutdown).await;
        });

        Ok((harvester, discovery_rx))
    }

    /// Dirección UDP efectiva en la que escucha este nodo DHT.
    pub fn listen_addr(&self) -> SocketAddr {
        self.listen_addr
    }

    /// Devuelve el identificador de 20 bytes del nodo DHT local.
    pub fn node_id(&self) -> [u8; 20] {
        self.node_id
    }

    /// Inyecta nodos semilla válidos en la tabla de enrutamiento del Sabueso DHT.
    pub async fn seed_nodes(&self, nodes: Vec<SocketAddr>) {
        let mut guard = self.state.lock().await;
        for addr in nodes {
            if self.is_target_allowed(addr) {
                guard.insert_node(addr);
            }
        }
    }

    /// Resuelve e inyecta los nodos públicos estándar de arranque de Mainline DHT
    /// (únicamente cuando el perfil de red permite tráfico público y la red está activa).
    pub async fn bootstrap_public_routers(&self) {
        if self.policy.profile().is_loopback_only()
            || matches!(
                self.policy.state(),
                NetworkState::Disabled | NetworkState::ShuttingDown
            )
        {
            return;
        }
        let mut resolved = Vec::new();
        for &(host, port) in MAINLINE_DHT_BOOTSTRAP_HOSTS {
            if let Ok(addrs) = tokio::net::lookup_host((host, port)).await {
                for addr in addrs {
                    if addr.is_ipv4() && self.is_target_allowed(addr) {
                        resolved.push(addr);
                    }
                }
            }
        }
        if !resolved.is_empty() {
            self.seed_nodes(resolved).await;
        }
    }

    /// Promociona direcciones de pares/nodos a la cola de afinidad de alta prioridad (idioma/Filmoteca).
    pub async fn promote_affinity_nodes(&self, nodes: Vec<SocketAddr>) {
        let mut guard = self.state.lock().await;
        for addr in nodes {
            if self.is_target_allowed(addr) {
                guard.insert_node(addr);
                guard.promote_affinity_node(addr);
            }
        }
    }

    /// Promociona direcciones IP de pares eD2K activos hacia la cola de afinidad DHT usando
    /// los puertos BitTorrent/DHT habituales para sesgar el rastreo hacia la vecindad comunitaria.
    pub async fn promote_ed2k_peer_ips(&self, ips: Vec<IpAddr>) {
        let mut targets = Vec::with_capacity(ips.len() * DEFAULT_ED2K_NEIGHBOR_BT_PORTS.len());
        for ip in ips {
            if !ip.is_ipv4() {
                continue;
            }
            for &port in DEFAULT_ED2K_NEIGHBOR_BT_PORTS {
                targets.push(SocketAddr::new(ip, port));
            }
        }
        if !targets.is_empty() {
            self.promote_affinity_nodes(targets).await;
        }
    }

    /// Obtiene la instantánea de métricas en tiempo real del Sabueso DHT.
    pub async fn stats(&self) -> DhtHarvesterStats {
        let (routing_nodes, affinity_nodes) = {
            let guard = self.state.lock().await;
            (guard.routing_set.len(), guard.affinity_set.len())
        };
        DhtHarvesterStats {
            running: self.counters.running.load(Ordering::Relaxed),
            listen_addr: Some(self.listen_addr.to_string()),
            routing_nodes,
            packets_sent: self.counters.packets_sent.load(Ordering::Relaxed),
            packets_received: self.counters.packets_received.load(Ordering::Relaxed),
            infohashes_sampled: self.counters.infohashes_sampled.load(Ordering::Relaxed),
            passive_announces: self.counters.passive_announces.load(Ordering::Relaxed),
            metadata_attempts: self.counters.metadata_attempts.load(Ordering::Relaxed),
            video_torrents_discovered: self
                .counters
                .video_torrents_discovered
                .load(Ordering::Relaxed),
            non_video_or_failed: self.counters.non_video_or_failed.load(Ordering::Relaxed),
            rendezvous_queries: self.counters.rendezvous_queries.load(Ordering::Relaxed),
            pex_peers_discovered: self.counters.pex_peers_discovered.load(Ordering::Relaxed),
            native_ed2k_tags_found: self.counters.native_ed2k_tags_found.load(Ordering::Relaxed),
            affinity_nodes,
        }
    }

    /// Consulta y anuncia en el punto de encuentro determinista `SHA1("pulse-ed2k-v1:" || ed2k_hash)`
    /// dentro de la DHT de BitTorrent para descubrir otros nodos híbridos que posean el archivo eD2K.
    pub async fn announce_and_find_rendezvous(
        &self,
        ed2k_hash: [u8; 16],
        announce_port: Option<u16>,
    ) -> Vec<SocketAddr> {
        self.counters
            .rendezvous_queries
            .fetch_add(1, Ordering::Relaxed);
        let rendezvous_id = derive_ed2k_rendezvous_id(&ed2k_hash);
        let (peers, tokens) = self.query_peers_on_demand(rendezvous_id).await;

        if let Some(port) = announce_port.filter(|&p| p > 0) {
            for (node_addr, token) in tokens {
                let tx_id = self.state.lock().await.next_tx_id();
                let pkt = encode_announce_peer(&tx_id, &self.node_id, &rendezvous_id, port, &token);
                self.send_packet(node_addr, &pkt).await;
            }
        }
        peers
    }

    /// Consulta pares activos en Mainline DHT para un `info_hash` concreto (20 bytes)
    /// con el fin de inyectarlos como `initial_peers` al lanzar un torrent híbrido en `librqbit`.
    pub async fn find_peers_for_infohash(&self, info_hash: [u8; 20]) -> Vec<SocketAddr> {
        let (peers, _) = self.query_peers_on_demand(info_hash).await;
        peers
    }

    /// Resuelve los metadatos de un `info_hash` concreto consultando la DHT (`get_peers`) y
    /// descargando su diccionario `info` mediante BEP 9 (`ut_metadata`) con política «Solo Vídeo».
    pub async fn resolve_infohash(
        &self,
        info_hash: [u8; 20],
        extra_peers: Vec<SocketAddr>,
        source: &str,
    ) -> Option<DiscoveredVideoTorrent> {
        let mut candidates = extra_peers;
        let (dht_peers, _) = self.query_peers_on_demand(info_hash).await;
        for peer in dht_peers {
            if !candidates.contains(&peer) {
                candidates.push(peer);
            }
        }
        for peer in candidates.into_iter().take(6) {
            self.counters
                .metadata_attempts
                .fetch_add(1, Ordering::Relaxed);
            if let Some(found) =
                fetch_video_metadata_from_peer(&self.policy, peer, info_hash, source).await
            {
                self.counters
                    .video_torrents_discovered
                    .fetch_add(1, Ordering::Relaxed);
                return Some(found);
            }
            self.counters
                .non_video_or_failed
                .fetch_add(1, Ordering::Relaxed);
        }
        None
    }

    async fn query_peers_on_demand(
        &self,
        info_hash: [u8; 20],
    ) -> (Vec<SocketAddr>, Vec<(SocketAddr, Vec<u8>)>) {
        if !self.is_network_active() {
            return (Vec::new(), Vec::new());
        }

        // Inicializar la frontera con los nodos conocidos ordenados por distancia XOR al objetivo
        // y respaldar con los nodos de `routing_queue` cuyo ID aún no se conoce.
        let mut frontier: Vec<(Option<[u8; 20]>, SocketAddr)> = {
            let guard = self.state.lock().await;
            let mut list: Vec<(Option<[u8; 20]>, SocketAddr)> = guard
                .routing_queue
                .iter()
                .copied()
                .map(|addr| (guard.node_ids.get(&addr).copied(), addr))
                .collect();
            list.sort_by(|a, b| match (a.0, b.0) {
                (Some(ida), Some(idb)) => xor_distance(&ida, &info_hash).cmp(&xor_distance(&idb, &info_hash)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            });
            list
        };
        if frontier.is_empty() {
            return (Vec::new(), Vec::new());
        }

        let mut peers = Vec::new();
        let mut tokens = Vec::new();
        let mut queried: HashSet<SocketAddr> = HashSet::new();
        let overall_deadline = Instant::now() + ON_DEMAND_QUERY_TIMEOUT;

        for _round in 0..MAX_ON_DEMAND_ROUNDS {
            if peers.len() >= ON_DEMAND_SUFFICIENT_PEERS || Instant::now() >= overall_deadline {
                break;
            }

            // Reordenar frontera por cercanía XOR antes de cada ronda.
            frontier.sort_by(|a, b| match (a.0, b.0) {
                (Some(ida), Some(idb)) => xor_distance(&ida, &info_hash).cmp(&xor_distance(&idb, &info_hash)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            });

            let mut batch: Vec<SocketAddr> = Vec::with_capacity(MAX_ON_DEMAND_FANOUT);
            for &(_, addr) in &frontier {
                if queried.len() >= MAX_ON_DEMAND_QUERIED_NODES
                    || batch.len() >= MAX_ON_DEMAND_FANOUT
                {
                    break;
                }
                if self.is_target_allowed(addr) && queried.insert(addr) {
                    batch.push(addr);
                }
            }
            if batch.is_empty() {
                break;
            }

            let (tx, mut rx) = mpsc::channel(64);
            for target in batch {
                let tx_id = {
                    let mut guard = self.state.lock().await;
                    let id = guard.next_tx_id();
                    guard.pending_get_peers.insert(
                        id.clone(),
                        PendingGetPeers {
                            info_hash,
                            source: "on_demand",
                            created_at: Instant::now(),
                            responder: Some(tx.clone()),
                        },
                    );
                    id
                };
                let pkt = encode_get_peers(&tx_id, &self.node_id, &info_hash);
                self.send_packet(target, &pkt).await;
            }
            drop(tx);

            let round_deadline = (Instant::now() + ON_DEMAND_ROUND_TIMEOUT).min(overall_deadline);
            while let Ok(Some((batch_peers, token, from_addr, closer_nodes))) =
                tokio::time::timeout_at(round_deadline.into(), rx.recv()).await
            {
                for p in batch_peers {
                    if self.is_target_allowed(p) && !peers.contains(&p) {
                        peers.push(p);
                    }
                }
                if let Some(tok) = token {
                    if !tokens.iter().any(|(a, _)| *a == from_addr) {
                        tokens.push((from_addr, tok));
                    }
                }
                for (nid, naddr) in closer_nodes {
                    if self.is_target_allowed(naddr) && !queried.contains(&naddr) {
                        if let Some(existing) = frontier.iter_mut().find(|(_, a)| *a == naddr) {
                            if existing.0.is_none() {
                                existing.0 = Some(nid);
                            }
                        } else {
                            frontier.push((Some(nid), naddr));
                        }
                    }
                }
            }
        }
        (peers, tokens)
    }

    fn is_network_active(&self) -> bool {
        !matches!(
            self.policy.state(),
            NetworkState::Disabled | NetworkState::ShuttingDown
        )
    }

    fn is_target_allowed(&self, addr: SocketAddr) -> bool {
        if addr == self.listen_addr || addr.port() == 0 {
            return false;
        }
        validate_connect_address(&addr, self.policy.profile()).is_ok()
            && self.policy.ip_filter().check_ip(addr.ip()).is_none()
    }

    async fn send_packet(&self, target: SocketAddr, payload: &[u8]) {
        if !self.is_network_active() || !self.is_target_allowed(target) {
            return;
        }
        if self.socket.send_to(payload, target).await.is_ok() {
            self.counters.packets_sent.fetch_add(1, Ordering::Relaxed);
        }
    }

    async fn run_probe_loop(self, shutdown: Arc<tokio::sync::Notify>) {
        let mut interval = tokio::time::interval(HARVEST_TICK_INTERVAL);
        loop {
            tokio::select! {
                _ = shutdown.notified() => {
                    self.counters.running.store(false, Ordering::Relaxed);
                    break;
                }
                _ = interval.tick() => {
                    if !self.is_network_active() {
                        continue;
                    }
                    let mut random_target = [0u8; 20];
                    rand::rng().fill_bytes(&mut random_target);

                    let nodes_to_probe: Vec<(SocketAddr, Vec<u8>, Vec<u8>)> = {
                        let mut guard = self.state.lock().await;
                        guard.prune_stale_pending();
                        let count = guard.routing_queue.len().min(MAX_PROBES_PER_TICK);
                        let mut batch = Vec::with_capacity(count);
                        for _ in 0..count {
                            let seq = guard.probe_counter;
                            guard.probe_counter = guard.probe_counter.wrapping_add(1);
                            // Reservar 75 % del muestreo BEP 51 para nodos frescos de la frontera global y
                            // respetar el enfriamiento BEP 51 para no re-consultar cachés repetidas.
                            let want_sample = seq % 4 != 3;
                            let prefer_affinity = seq % 2 == 0;
                            let prefer_vip = seq % 8 == 5;
                            if want_sample {
                                if let Some(node_addr) =
                                    guard.pop_next_probe_node(prefer_vip, prefer_affinity, true)
                                {
                                    guard.record_node_sampled(node_addr);
                                    let tx_id = guard.next_tx_id();
                                    let pkt = encode_sample_infohashes(
                                        &tx_id,
                                        &self.node_id,
                                        &random_target,
                                    );
                                    batch.push((node_addr, tx_id, pkt));
                                    continue;
                                }
                            }
                            if let Some(node_addr) = guard.pop_next_probe_node(false, false, false)
                            {
                                let tx_id = guard.next_tx_id();
                                let pkt = encode_find_node(&tx_id, &self.node_id, &random_target);
                                batch.push((node_addr, tx_id, pkt));
                            }
                        }
                        batch
                    };

                    for (node_addr, _tx_id, pkt) in nodes_to_probe {
                        self.send_packet(node_addr, &pkt).await;
                    }
                }
            }
        }
    }

    async fn run_recv_loop(self, shutdown: Arc<tokio::sync::Notify>) {
        let mut buf = vec![0u8; UDP_RECV_BUFFER_BYTES];
        loop {
            tokio::select! {
                _ = shutdown.notified() => {
                    self.counters.running.store(false, Ordering::Relaxed);
                    break;
                }
                recv_res = self.socket.recv_from(&mut buf) => {
                    let Ok((n, from_addr)) = recv_res else {
                        continue;
                    };
                    if !self.is_network_active() || !self.is_target_allowed(from_addr) {
                        continue;
                    }
                    self.counters.packets_received.fetch_add(1, Ordering::Relaxed);
                    if let Some(msg) = decode_krpc_packet(&buf[..n]) {
                        self.handle_krpc_message(from_addr, msg).await;
                    }
                }
            }
        }
    }

    async fn handle_krpc_message(&self, from_addr: SocketAddr, msg: KrpcMessage) {
        match msg {
            KrpcMessage::Query {
                tx_id,
                query_name,
                sender_id,
                info_hash,
                port,
                implied_port,
                ..
            } => {
                self.state
                    .lock()
                    .await
                    .insert_node_with_id(from_addr, Some(sender_id));
                match query_name.as_str() {
                    "ping" => {
                        let reply = encode_simple_response(&tx_id, &self.node_id);
                        self.send_packet(from_addr, &reply).await;
                    }
                    "find_node" => {
                        let neighbors = self.state.lock().await.sample_reply_neighbors(from_addr);
                        let reply = encode_find_node_response(&tx_id, &self.node_id, &neighbors);
                        self.send_packet(from_addr, &reply).await;
                    }
                    "get_peers" => {
                        let (peers, neighbors) = {
                            let guard = self.state.lock().await;
                            let p = info_hash
                                .and_then(|ih| guard.local_peers.get(&ih).cloned())
                                .unwrap_or_default();
                            let n = guard.sample_reply_neighbors(from_addr);
                            (p, n)
                        };
                        let reply = encode_get_peers_response(
                            &tx_id,
                            &self.node_id,
                            LOCAL_DHT_TOKEN,
                            &peers,
                            &neighbors,
                        );
                        self.send_packet(from_addr, &reply).await;
                    }
                    "announce_peer" => {
                        let reply = encode_simple_response(&tx_id, &self.node_id);
                        self.send_packet(from_addr, &reply).await;
                        if let Some(ih) = info_hash {
                            let peer_port = if implied_port {
                                from_addr.port()
                            } else {
                                port.unwrap_or(from_addr.port())
                            };
                            if peer_port > 0 {
                                let peer_addr = SocketAddr::new(from_addr.ip(), peer_port);
                                let is_new = {
                                    let mut guard = self.state.lock().await;
                                    guard.record_local_peer(ih, peer_addr);
                                    guard.mark_infohash_seen(ih)
                                };
                                self.counters
                                    .passive_announces
                                    .fetch_add(1, Ordering::Relaxed);
                                if is_new {
                                    self.spawn_metadata_fetch(ih, vec![peer_addr], "dht_passive");
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            KrpcMessage::Response {
                tx_id,
                sender_id,
                nodes,
                peers,
                samples,
                token,
            } => {
                let mut follow_up_get_peers: Vec<(SocketAddr, Vec<u8>)> = Vec::new();
                let pending = {
                    let mut guard = self.state.lock().await;
                    guard.insert_node_with_id(from_addr, sender_id);
                    if !peers.is_empty() {
                        guard.promote_bep51_vip(from_addr);
                    }
                    for (node_id, node_addr) in &nodes {
                        if self.is_target_allowed(*node_addr) {
                            guard.insert_node_with_id(*node_addr, Some(*node_id));
                        }
                    }
                    for sample_hash in samples {
                        self.counters
                            .infohashes_sampled
                            .fetch_add(1, Ordering::Relaxed);
                        if guard.mark_infohash_seen(sample_hash) {
                            let gp_tx = guard.next_tx_id();
                            guard.pending_get_peers.insert(
                                gp_tx.clone(),
                                PendingGetPeers {
                                    info_hash: sample_hash,
                                    source: "dht_bep51",
                                    created_at: Instant::now(),
                                    responder: None,
                                },
                            );
                            let pkt = encode_get_peers(&gp_tx, &self.node_id, &sample_hash);
                            follow_up_get_peers.push((from_addr, pkt));
                        }
                    }
                    guard.pending_get_peers.remove(&tx_id)
                };

                for (target, pkt) in follow_up_get_peers {
                    self.send_packet(target, &pkt).await;
                }

                if let Some(pending_req) = pending {
                    if let Some(responder) = pending_req.responder {
                        let _ = responder.try_send((peers.clone(), token, from_addr, nodes));
                    } else if !peers.is_empty() {
                        self.spawn_metadata_fetch(
                            pending_req.info_hash,
                            peers,
                            pending_req.source,
                        );
                    }
                }
            }
        }
    }

    fn spawn_metadata_fetch(
        &self,
        info_hash: [u8; 20],
        peers: Vec<SocketAddr>,
        source: &'static str,
    ) {
        let Ok(permit) = Arc::clone(&self.metadata_sem).try_acquire_owned() else {
            return;
        };
        let policy = Arc::clone(&self.policy);
        let counters = Arc::clone(&self.counters);
        let state = Arc::clone(&self.state);
        let tx = self.discovery_tx.clone();
        let listen_addr = self.listen_addr;
        tokio::spawn(async move {
            let _permit = permit;
            for peer in peers.into_iter().take(MAX_PEERS_PER_METADATA_FETCH) {
                counters.metadata_attempts.fetch_add(1, Ordering::Relaxed);
                if let Some(found) =
                    fetch_video_metadata_from_peer(&policy, peer, info_hash, source).await
                {
                    counters
                        .video_torrents_discovered
                        .fetch_add(1, Ordering::Relaxed);
                    let ed2k_tags = found
                        .video_files
                        .iter()
                        .filter(|f| f.ed2k_hash.is_some())
                        .count() as u64;
                    if ed2k_tags > 0 {
                        counters
                            .native_ed2k_tags_found
                            .fetch_add(ed2k_tags, Ordering::Relaxed);
                    }
                    if !found.pex_peers.is_empty() {
                        counters
                            .pex_peers_discovered
                            .fetch_add(found.pex_peers.len() as u64, Ordering::Relaxed);
                    }
                    let affinity =
                        compute_language_affinity_score(&found.name, &found.primary_video.path);
                    if affinity > 0 || !found.pex_peers.is_empty() {
                        let mut guard = state.lock().await;
                        for addr in found.pex_peers.iter().copied() {
                            if addr != listen_addr && addr.port() > 0 {
                                guard.insert_node(addr);
                                if affinity > 0 {
                                    guard.promote_affinity_node(addr);
                                }
                            }
                        }
                        if affinity > 0 && peer != listen_addr && peer.port() > 0 {
                            guard.promote_affinity_node(peer);
                        }
                    }
                    let _ = tx.try_send(found);
                    return;
                }
                counters.non_video_or_failed.fetch_add(1, Ordering::Relaxed);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dht_crawler::bencode::{decode, BValue};
    use crate::dht_crawler::wire::encode_sample_infohashes_response;
    use mule_network_policy::{NetworkProfile, ResourceBudgets};
    use sha1::{Digest, Sha1};
    use std::collections::BTreeMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn end_to_end_loopback_bep51_discovery_and_rendezvous() {
        // 1. Preparar metadatos de vídeo servidos por un par BEP 10/BEP 9 en loopback.
        let mut info_dict = BTreeMap::new();
        info_dict.insert(b"length".to_vec(), BValue::Int(1_234_567));
        info_dict.insert(
            b"name".to_vec(),
            BValue::str_bytes("Dune.Part.Two.2024.1080p.mkv"),
        );
        info_dict.insert(b"piece length".to_vec(), BValue::Int(262_144));
        info_dict.insert(b"pieces".to_vec(), BValue::Bytes(vec![0x33u8; 20]));
        let raw_info = BValue::Dict(info_dict).encode();
        let info_hash: [u8; 20] = Sha1::digest(&raw_info).into();

        let peer_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let peer_addr = peer_listener.local_addr().unwrap();
        let raw_info_srv = raw_info.clone();
        tokio::spawn(async move {
            let (mut sock, _) = peer_listener.accept().await.unwrap();
            let mut hs = [0u8; 68];
            sock.read_exact(&mut hs).await.unwrap();
            let mut reply = [0u8; 68];
            reply[..20].copy_from_slice(b"\x13BitTorrent protocol");
            reply[25] = 0x10;
            reply[28..48].copy_from_slice(&info_hash);
            reply[48..68].copy_from_slice(b"-TEST01-123456789012");
            sock.write_all(&reply).await.unwrap();

            // Leer handshake extendido
            let mut len_buf = [0u8; 4];
            sock.read_exact(&mut len_buf).await.unwrap();
            let mut body = vec![0u8; u32::from_be_bytes(len_buf) as usize];
            sock.read_exact(&mut body).await.unwrap();

            // Enviar handshake extendido
            let mut m = BTreeMap::new();
            m.insert(b"ut_metadata".to_vec(), BValue::Int(2));
            let mut ext = BTreeMap::new();
            ext.insert(b"m".to_vec(), BValue::Dict(m));
            ext.insert(
                b"metadata_size".to_vec(),
                BValue::Int(raw_info_srv.len() as i64),
            );
            let ext_bytes = BValue::Dict(ext).encode();
            let mut frame = Vec::new();
            frame.extend_from_slice(&((2 + ext_bytes.len()) as u32).to_be_bytes());
            frame.push(20);
            frame.push(0);
            frame.extend_from_slice(&ext_bytes);
            sock.write_all(&frame).await.unwrap();

            // Leer petición pieza 0 y responder
            sock.read_exact(&mut len_buf).await.unwrap();
            let mut req_body = vec![0u8; u32::from_be_bytes(len_buf) as usize];
            sock.read_exact(&mut req_body).await.unwrap();
            let _ = decode(&req_body[2..]).unwrap();

            let mut hdr = BTreeMap::new();
            hdr.insert(b"msg_type".to_vec(), BValue::Int(1));
            hdr.insert(b"piece".to_vec(), BValue::Int(0));
            hdr.insert(
                b"total_size".to_vec(),
                BValue::Int(raw_info_srv.len() as i64),
            );
            let mut payload = BValue::Dict(hdr).encode();
            payload.extend_from_slice(&raw_info_srv);
            let mut data_frame = Vec::new();
            data_frame.extend_from_slice(&((2 + payload.len()) as u32).to_be_bytes());
            data_frame.push(20);
            data_frame.push(1);
            data_frame.extend_from_slice(&payload);
            sock.write_all(&data_frame).await.unwrap();
        });

        // 2. Nodo DHT simulado en loopback que responde a `sample_infohashes` y `get_peers`.
        let mock_dht = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let mock_dht_addr = mock_dht.local_addr().unwrap();
        let mock_id = [0x99u8; 20];
        let mock_dht_task = Arc::clone(&mock_dht);
        let rendezvous_ed2k = [0x42u8; 16];
        let rendezvous_id = derive_ed2k_rendezvous_id(&rendezvous_ed2k);
        let rendezvous_peer: SocketAddr = "127.0.0.1:4662".parse().unwrap();

        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            for _ in 0..32 {
                let Ok((n, from)) = mock_dht_task.recv_from(&mut buf).await else {
                    break;
                };
                if let Some(KrpcMessage::Query {
                    tx_id,
                    query_name,
                    info_hash: q_ih,
                    ..
                }) = decode_krpc_packet(&buf[..n])
                {
                    let reply = match query_name.as_str() {
                        "sample_infohashes" => {
                            encode_sample_infohashes_response(&tx_id, &mock_id, &[info_hash], &[])
                        }
                        "get_peers" => {
                            let list = if q_ih == Some(rendezvous_id) {
                                vec![rendezvous_peer]
                            } else {
                                vec![peer_addr]
                            };
                            encode_get_peers_response(&tx_id, &mock_id, b"tk", &list, &[])
                        }
                        _ => encode_simple_response(&tx_id, &mock_id),
                    };
                    let _ = mock_dht_task.send_to(&reply, from).await;
                }
            }
        });

        // 3. Arrancar el Sabueso DHT en perfil LocalLab y verificar descubrimiento autónomo.
        let policy = Arc::new(NetworkPolicyManager::new(
            NetworkProfile::LocalLab,
            ResourceBudgets::default(),
        ));
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let (harvester, mut rx) = DhtHarvester::start(policy, Arc::clone(&shutdown))
            .await
            .unwrap();
        harvester.seed_nodes(vec![mock_dht_addr]).await;

        let discovered = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("el Sabueso DHT debía descubrir el vídeo en loopback")
            .expect("canal activo");
        assert_eq!(discovered.info_hash, hex::encode(info_hash));
        assert_eq!(discovered.primary_video.size_bytes, 1_234_567);

        // 4. Verificar también el punto de encuentro Rendezvous eD2K → BitTorrent DHT.
        let rdv_peers = harvester
            .announce_and_find_rendezvous(rendezvous_ed2k, Some(4662))
            .await;
        assert!(rdv_peers.contains(&rendezvous_peer));

        shutdown.notify_waiters();
    }

    #[test]
    fn bep51_cooldown_sweeps_fresh_frontier_instead_of_looping_on_vip() {
        let mut st = HarvesterState::new();
        for i in 0..40u16 {
            let vip: SocketAddr = format!("198.51.100.{}:{}", (i % 200) + 1, 6881 + i)
                .parse()
                .unwrap();
            st.insert_node(vip);
            st.promote_bep51_vip(vip);
            st.record_node_sampled(vip);
        }
        let fresh: SocketAddr = "203.0.113.77:6881".parse().unwrap();
        st.insert_node(fresh);

        // Con los 40 VIP en enfriamiento, la selección BEP 51 debe saltar a los nodos frescos de la frontera.
        let picked = st.pop_next_probe_node(true, false, true);
        assert_eq!(picked, Some(fresh));

        // Tras registrar el muestreo del nodo fresco, no se vuelve a re-muestrear inmediatamente.
        st.record_node_sampled(fresh);
        assert_eq!(st.pop_next_probe_node(true, false, true), None);
        // Pero `find_node` sigue disponiendo de nodos para expandir la frontera.
        assert!(st.pop_next_probe_node(false, false, false).is_some());
    }

    #[tokio::test]
    async fn iterative_xor_lookup_follows_closer_nodes_in_loopback() {
        let target_hash = [0xABu8; 20];
        let target_peer: SocketAddr = "127.0.0.1:51413".parse().unwrap();

        // Nodo B (más cercano al objetivo): devuelve el par real.
        let node_b = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let node_b_addr = node_b.local_addr().unwrap();
        let node_b_id = [0xAAu8; 20];
        let node_b_task = Arc::clone(&node_b);
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            for _ in 0..8 {
                let Ok((n, from)) = node_b_task.recv_from(&mut buf).await else {
                    break;
                };
                if let Some(KrpcMessage::Query {
                    tx_id, query_name, ..
                }) = decode_krpc_packet(&buf[..n])
                {
                    let reply = if query_name == "get_peers" {
                        encode_get_peers_response(&tx_id, &node_b_id, b"tkb", &[target_peer], &[])
                    } else {
                        encode_simple_response(&tx_id, &node_b_id)
                    };
                    let _ = node_b_task.send_to(&reply, from).await;
                }
            }
        });

        // Nodo A (semilla): no tiene pares pero refiere al nodo B más cercano.
        let node_a = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let node_a_addr = node_a.local_addr().unwrap();
        let node_a_id = [0x11u8; 20];
        let node_a_task = Arc::clone(&node_a);
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            for _ in 0..8 {
                let Ok((n, from)) = node_a_task.recv_from(&mut buf).await else {
                    break;
                };
                if let Some(KrpcMessage::Query {
                    tx_id, query_name, ..
                }) = decode_krpc_packet(&buf[..n])
                {
                    let reply = if query_name == "get_peers" {
                        encode_get_peers_response(
                            &tx_id,
                            &node_a_id,
                            b"tka",
                            &[],
                            &[(node_b_id, node_b_addr)],
                        )
                    } else {
                        encode_simple_response(&tx_id, &node_a_id)
                    };
                    let _ = node_a_task.send_to(&reply, from).await;
                }
            }
        });

        let policy = Arc::new(NetworkPolicyManager::new(
            NetworkProfile::LocalLab,
            ResourceBudgets::default(),
        ));
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let (harvester, _rx) = DhtHarvester::start(policy, Arc::clone(&shutdown))
            .await
            .unwrap();
        harvester.seed_nodes(vec![node_a_addr]).await;

        let peers = harvester.find_peers_for_infohash(target_hash).await;
        assert!(
            peers.contains(&target_peer),
            "el lookup iterativo XOR debía seguir `nodes` hacia el nodo B y encontrar {target_peer}"
        );

        shutdown.notify_waiters();
    }
}

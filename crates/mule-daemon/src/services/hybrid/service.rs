// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Servicio autónomo del motor híbrido eD2K ⇄ Kad ⇄ BitTorrent sin indexadores (`HybridSwarmService`).
//!
//! Orquesta en segundo plano:
//! 1. Sabueso Mainline DHT (BEP 5/51 + BEP 9) que alimenta el catálogo local `dht_torrent_catalog`.
//! 2. Descubrimiento de puentes para descargas eD2K activas (tamaño exacto en catálogo DHT local,
//!    notas Kad `pc1:btih:...` y Rendezvous determinista en Mainline DHT).
//! 3. Verificación criptográfica MD4 anti-fakes y fusión real de bytes BitTorrent → eD2K.
//! 4. Cosecha inversa: los vídeos completados en BitTorrent calculan su hash eD2K y siembran Kad.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mule_core::model::{PeerSource, PeerSourceOrigin, PeerSourceStatus};
use mule_core::persistence::sqlite::hybrid_index::{
    normalize_search_text, MAX_DHT_CATALOG_ENTRIES,
};
use mule_core::{
    DhtTorrentCatalogEntry, DhtTorrentFileEntry, HybridBridgeRecord, HybridBridgeStatus,
    HybridCatalogStats,
};
use mule_network_policy::NetworkState;
use mule_torrent::{
    build_minimal_torrent_bytes, is_blocked_abusive_or_adult, is_clean_catalog_video,
    DhtHarvester, DhtHarvesterStats, DiscoveredVideoTorrent,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::services::downloads::now_ms;
use crate::state::AppState;

use super::active_discovery::{
    discover_bridges_actively, ActiveDiscoveryOutcome, EquivalentTorrentHint,
};
use super::fusion::{
    pump_verified_ranges_into_ed2k, verify_candidate_bt_file_against_ed2k,
    CandidateVerificationOutcome,
};
use super::kad_bridge::{discover_bridges_via_kad_notes, publish_verified_bridge_to_kad};

/// Intervalo principal de orquestación del motor híbrido (5 s).
const HYBRID_TICK_INTERVAL: Duration = Duration::from_secs(5);
/// Número máximo de descargas eD2K aceleradas simultáneamente mediante torrents auxiliares.
const MAX_CONCURRENT_HYBRID_FUSIONS: usize = 3;
/// Enfriamiento mínimo entre consultas de notas Kad por cada descarga eD2K (5 minutos).
const KAD_NOTE_DISCOVERY_COOLDOWN: Duration = Duration::from_secs(5 * 60);
/// Enfriamiento mínimo entre consultas Rendezvous DHT por cada descarga eD2K (3 minutos).
const RENDEZVOUS_DISCOVERY_COOLDOWN: Duration = Duration::from_secs(3 * 60);
/// Enfriamiento mínimo entre búsquedas activas (Torznab + catálogo DHT por tokens) por descarga eD2K (10 minutos).
const ACTIVE_DISCOVERY_COOLDOWN: Duration = Duration::from_secs(10 * 60);
/// Intervalo mínimo entre reintentos de bootstrap público del Sabueso DHT si la tabla queda vacía (60 s).
const DHT_BOOTSTRAP_RETRY_INTERVAL: Duration = Duration::from_secs(60);
/// Intervalo entre sincronizaciones de IPs de pares eD2K hacia la cola de afinidad del Sabueso DHT (120 s).
const ED2K_NEIGHBORHOOD_SYNC_INTERVAL: Duration = Duration::from_secs(120);
/// Máximo de direcciones IP de pares eD2K promocionadas por ciclo hacia el Sabueso DHT.
const MAX_ED2K_PEER_IPS_PER_SYNC: usize = 32;
/// Intervalo mínimo entre republicaciones de un mismo puente verificado en notas Kad (24 h).
const KAD_NOTE_REPUBLISH_INTERVAL_MS: i64 = 24 * 60 * 60 * 1_000;
/// Intervalo entre escaneos de torrents completados para cosecha inversa BT → eD2K (30 s).
const COMPLETED_BT_SCAN_INTERVAL: Duration = Duration::from_secs(30);
/// Intervalo de poda LRU del catálogo DHT local (10 minutos).
const CATALOG_PRUNE_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// Umbral mínimo de nodos en el Sabueso DHT por debajo del cual se re-siembra desde `librqbit`/bootstrap.
const DHT_MIN_ROUTING_NODES: usize = 16;
/// Número de nodos importados desde la tabla de enrutamiento de `librqbit` por re-siembra.
const DHT_SEED_BATCH_SIZE: usize = 64;
/// Subcarpeta dentro de `temp` donde se alojan los torrents temporales de fusión híbrida.
const HYBRID_BT_TEMP_SUBDIR: &str = "hybrid_bt";
/// Nivel de compresión `zstd` para almacenar diccionarios `.torrent` en SQLite.
const TORRENT_META_ZSTD_LEVEL: i32 = 3;
/// Máximo de consultas de red lentas (Kad Notes / Rendezvous) iniciadas por tick para no bloquear puentes locales.
const MAX_SLOW_NETWORK_DISCOVERIES_PER_TICK: usize = 1;

/// Estado de una fusión activa BitTorrent → eD2K en memoria.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveHybridFusion {
    pub download_id: String,
    pub ed2k_hash: String,
    pub bt_info_hash: String,
    pub torrent_id: usize,
    pub bt_file_index: usize,
    pub file_size: u64,
    pub verified: bool,
    pub bytes_fused: u64,
    pub peer_count: usize,
    pub bt_download_rate_bps: u64,
    #[serde(skip)]
    pub temp_dir: PathBuf,
}

/// Telemetría de la última ronda de descubrimiento híbrido por cada descarga eD2K activa.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadHybridTelemetry {
    pub download_id: String,
    pub ed2k_hash: String,
    pub last_checked_ms: i64,
    pub enabled_indexers: usize,
    pub torznab_candidates_checked: usize,
    #[serde(default)]
    pub public_candidates_checked: usize,
    pub dht_candidates_checked: usize,
    pub kad_notes_found: usize,
    pub rendezvous_peers_found: usize,
    pub exact_matches_found: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_equivalent: Option<EquivalentTorrentHint>,
    pub status_summary: String,
}

/// Instantánea pública de estado y telemetría del motor híbrido (expuesta por `hybrid.status`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridStatusSnapshot {
    pub dht_harvester: DhtHarvesterStats,
    pub catalog: HybridCatalogStats,
    pub active_fusions: Vec<ActiveHybridFusion>,
    #[serde(default)]
    pub download_telemetry: Vec<DownloadHybridTelemetry>,
    pub bytes_fused_from_bt: u64,
    pub bridges_verified_session: u64,
    pub bridges_rejected_session: u64,
    pub kad_notes_published: u64,
    pub rendezvous_peers_found: u64,
    pub completed_bt_hashed: u64,
}

/// Informe devuelto tras lanzar una ronda manual de descubrimiento híbrido para una descarga eD2K.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridDiscoveryReport {
    pub download_id: String,
    pub ed2k_hash: String,
    pub file_size: u64,
    pub dht_exact_matches: usize,
    pub kad_note_candidates: usize,
    pub rendezvous_peers_found: usize,
    pub enabled_indexers: usize,
    pub torznab_candidates_checked: usize,
    #[serde(default)]
    pub public_candidates_checked: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_equivalent: Option<EquivalentTorrentHint>,
    pub active_fusion_started: bool,
    pub bridges: Vec<HybridBridgeRecord>,
}

#[derive(Default)]
struct HybridCounters {
    bytes_fused_from_bt: AtomicU64,
    bridges_verified_session: AtomicU64,
    bridges_rejected_session: AtomicU64,
    kad_notes_published: AtomicU64,
    rendezvous_peers_found: AtomicU64,
    completed_bt_hashed: AtomicU64,
}

/// Coordinador híbrido autónomo eD2K ⇄ Kad ⇄ BitTorrent sin indexadores.
#[derive(Clone)]
pub struct HybridSwarmService {
    harvester: Arc<RwLock<Option<DhtHarvester>>>,
    active_fusions: Arc<Mutex<HashMap<Uuid, ActiveHybridFusion>>>,
    counters: Arc<HybridCounters>,
    last_kad_query: Arc<Mutex<HashMap<Uuid, Instant>>>,
    last_rendezvous_query: Arc<Mutex<HashMap<Uuid, Instant>>>,
    last_active_discovery: Arc<Mutex<HashMap<Uuid, Instant>>>,
    last_dht_bootstrap: Arc<Mutex<Option<Instant>>>,
    last_neighborhood_sync: Arc<Mutex<Option<Instant>>>,
    download_telemetry: Arc<Mutex<HashMap<Uuid, DownloadHybridTelemetry>>>,
    hashed_completed_bt: Arc<Mutex<HashSet<String>>>,
}

impl Default for HybridSwarmService {
    fn default() -> Self {
        Self::new()
    }
}

impl HybridSwarmService {
    /// Crea una nueva instancia del servicio híbrido lista para insertarse en `AppState`.
    pub fn new() -> Self {
        Self {
            harvester: Arc::new(RwLock::new(None)),
            active_fusions: Arc::new(Mutex::new(HashMap::new())),
            counters: Arc::new(HybridCounters::default()),
            last_kad_query: Arc::new(Mutex::new(HashMap::new())),
            last_rendezvous_query: Arc::new(Mutex::new(HashMap::new())),
            last_active_discovery: Arc::new(Mutex::new(HashMap::new())),
            last_dht_bootstrap: Arc::new(Mutex::new(None)),
            last_neighborhood_sync: Arc::new(Mutex::new(None)),
            download_telemetry: Arc::new(Mutex::new(HashMap::new())),
            hashed_completed_bt: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Arranca el Sabueso DHT y el bucle autónomo de fusión y cosecha inversa en segundo plano.
    pub fn start(state: AppState) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let service = state.hybrid.clone();
            let shutdown = state.shutdown_notify.clone();

            // 1. Arrancar Sabueso DHT Mainline gobernado por NetworkPolicyManager.
            let policy = Arc::new(state.network_policy.clone());
            match DhtHarvester::start(policy, Arc::clone(&shutdown)).await {
                Ok((harvester, mut discovery_rx)) => {
                    tracing::info!(
                        listen = %harvester.listen_addr(),
                        "Sabueso DHT Mainline (BEP 5/51 + BEP 9) iniciado sin indexadores"
                    );
                    *service.harvester.write().await = Some(harvester.clone());

                    // Consumidor asíncrono que persiste en SQLite los torrents de vídeo descubiertos.
                    let state_ingest = state.clone();
                    let shutdown_ingest = Arc::clone(&shutdown);
                    tokio::spawn(async move {
                        let mut last_prune = Instant::now();
                        loop {
                            tokio::select! {
                                _ = shutdown_ingest.notified() => break,
                                maybe_torrent = discovery_rx.recv() => {
                                    let Some(discovered) = maybe_torrent else { break };
                                    Self::persist_discovered_torrent(&state_ingest, &discovered).await;
                                    if last_prune.elapsed() >= CATALOG_PRUNE_INTERVAL {
                                        last_prune = Instant::now();
                                        let mut dl = state_ingest.downloads.lock().await;
                                        let _ = dl.db_mut().prune_dht_catalog(MAX_DHT_CATALOG_ENTRIES);
                                    }
                                }
                            }
                        }
                    });

                    // Arranque público diferido (solo si el perfil de red lo permite).
                    let boot_harvester = harvester;
                    tokio::spawn(async move {
                        boot_harvester.bootstrap_public_routers().await;
                    });
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "No se pudo iniciar el socket UDP del Sabueso DHT; el resto del puente híbrido seguirá activo"
                    );
                }
            }

            // 2. Bucle principal de coordinación híbrida.
            let mut interval = tokio::time::interval(HYBRID_TICK_INTERVAL);
            let mut last_completed_scan = Instant::now() - COMPLETED_BT_SCAN_INTERVAL;

            loop {
                tokio::select! {
                    _ = shutdown.notified() => {
                        tracing::info!("HybridSwarmService detenido por señal de apagado");
                        break;
                    }
                    _ = interval.tick() => {
                        if matches!(
                            state.network_policy.state(),
                            NetworkState::Disabled | NetworkState::ShuttingDown
                        ) {
                            continue;
                        }
                        service.tick(&state, &mut last_completed_scan).await;
                    }
                }
            }
        })
    }

    pub(super) async fn persist_discovered_torrent(
        state: &AppState,
        discovered: &DiscoveredVideoTorrent,
    ) {
        // Defensa en profundidad: rechazar de forma incondicional cualquier contenido abusivo o adulto.
        if is_blocked_abusive_or_adult(&discovered.name)
            || discovered
                .video_files
                .iter()
                .any(|f| is_blocked_abusive_or_adult(&f.path))
        {
            return;
        }

        // Comprobar si algún archivo coincide exactamente con nuestra cola eD2K o con títulos de `media_library`,
        // o si supera el filtro de cine/series/anime limpios.
        let (active_ed2k_sizes, watched_titles): (HashSet<u64>, Vec<String>) = {
            let dl = state.downloads.lock().await;
            let sizes = dl
                .list_downloads()
                .into_iter()
                .filter(|d| d.total_bytes > 0 && d.status != "completed")
                .map(|d| d.total_bytes)
                .collect();
            let titles = dl
                .db()
                .list_media_items(None, None)
                .unwrap_or_default()
                .into_iter()
                .filter(|m| m.monitored && m.status != "completed")
                .map(|m| normalize_search_text(&m.title, ""))
                .filter(|t| t.len() >= 4)
                .collect();
            (sizes, titles)
        };

        let normalized_torrent = normalize_search_text(&discovered.name, &discovered.primary_video.path);
        let matches_library = watched_titles
            .iter()
            .any(|title| normalized_torrent.contains(title.as_str()));

        // Si el torrent coincide con nuestra Filmoteca, priorizar su vecindad y pares `ut_pex` en el Sabueso DHT.
        if matches_library {
            if let Some(harvester) = state.hybrid.harvester.read().await.clone() {
                let mut affinity_targets = discovered.peers.clone();
                affinity_targets.extend(discovered.pex_peers.iter().copied());
                harvester.promote_affinity_nodes(affinity_targets).await;
            }
        }

        let clean_files: Vec<DhtTorrentFileEntry> = discovered
            .video_files
            .iter()
            .filter(|f| {
                f.ed2k_hash.is_some()
                    || active_ed2k_sizes.contains(&f.size_bytes)
                    || matches_library
                    || is_clean_catalog_video(&discovered.name, &f.path, f.size_bytes)
            })
            .map(|f| DhtTorrentFileEntry {
                info_hash: discovered.info_hash.clone(),
                file_index: f.file_index,
                file_path: f.path.clone(),
                file_size: f.size_bytes,
                file_offset: f.offset_in_torrent,
            })
            .collect();

        if clean_files.is_empty() {
            return;
        }

        let now = now_ms();
        let files_json =
            serde_json::to_string(&discovered.video_files).unwrap_or_else(|_| "[]".to_string());
        let primary = clean_files
            .iter()
            .max_by_key(|f| f.file_size)
            .cloned()
            .unwrap_or(DhtTorrentFileEntry {
                info_hash: discovered.info_hash.clone(),
                file_index: discovered.primary_video.file_index,
                file_path: discovered.primary_video.path.clone(),
                file_size: discovered.primary_video.size_bytes,
                file_offset: discovered.primary_video.offset_in_torrent,
            });
        let entry = DhtTorrentCatalogEntry {
            info_hash: discovered.info_hash.clone(),
            name: discovered.name.clone(),
            total_bytes: discovered.total_bytes,
            video_file_name: primary.file_path,
            video_file_size: primary.file_size,
            video_file_index: primary.file_index,
            video_file_offset: primary.file_offset,
            files_json,
            estimated_peers: discovered.peers.len().max(1) as u32,
            source: discovered.source.clone(),
            first_seen_ms: now,
            last_seen_ms: now,
        };
        let mut dl = state.downloads.lock().await;
        if let Err(err) = dl.db_mut().upsert_dht_catalog_entry(&entry) {
            tracing::debug!(
                info_hash = %entry.info_hash,
                error = %err,
                "Error persistiendo torrent descubierto por DHT en el catálogo local"
            );
            return;
        }
        if let Err(err) =
            dl.db_mut()
                .upsert_dht_catalog_files(&entry.info_hash, &entry.name, &clean_files)
        {
            tracing::debug!(
                info_hash = %entry.info_hash,
                error = %err,
                "Error indexando archivos multifichero del torrent DHT"
            );
        }

        // Persistir el `.torrent` mínimo comprimido con `zstd` y los pares vistos para arranque instantáneo.
        if !discovered.raw_info_bytes.is_empty() {
            let minimal_torrent = build_minimal_torrent_bytes(&discovered.raw_info_bytes);
            if let Ok(compressed) =
                zstd::stream::encode_all(minimal_torrent.as_slice(), TORRENT_META_ZSTD_LEVEL)
            {
                let mut all_peers: Vec<String> = discovered
                    .peers
                    .iter()
                    .chain(discovered.pex_peers.iter())
                    .map(ToString::to_string)
                    .collect();
                all_peers.sort();
                all_peers.dedup();
                let peers_json =
                    serde_json::to_string(&all_peers).unwrap_or_else(|_| "[]".to_string());
                let _ = dl
                    .db_mut()
                    .store_dht_torrent_meta(&entry.info_hash, &compressed, &peers_json);
            }
        }

        // Registrar automáticamente puentes directos cuando el diccionario `.torrent` incluya la etiqueta nativa `"ed2k"`.
        for vf in &discovered.video_files {
            if let Some(ref ed2k_hex) = vf.ed2k_hash {
                let bridge = HybridBridgeRecord {
                    ed2k_hash: ed2k_hex.clone(),
                    bt_info_hash: discovered.info_hash.clone(),
                    file_size: vf.size_bytes,
                    bt_file_index: vf.file_index,
                    bt_file_offset: vf.offset_in_torrent,
                    status: HybridBridgeStatus::Candidate,
                    origin: "torrent_ed2k_tag".to_string(),
                    verified_chunks: 0,
                    published_kad_at_ms: None,
                    created_at_ms: now,
                    updated_at_ms: now,
                };
                let _ = dl.db_mut().upsert_hybrid_bridge(&bridge);
            }
        }
    }

    async fn tick(&self, state: &AppState, last_completed_scan: &mut Instant) {
        // A. Mantener viva la tabla de enrutamiento del Sabueso DHT usando `librqbit` o re-bootstrap público.
        if let Some(harvester) = self.harvester.read().await.clone() {
            let stats = harvester.stats().await;
            if stats.routing_nodes < DHT_MIN_ROUTING_NODES {
                if let Some(engine) = state.torrent.get() {
                    let seed = engine.dht_routing_nodes(DHT_SEED_BATCH_SIZE);
                    if !seed.is_empty() {
                        harvester.seed_nodes(seed).await;
                    }
                }
                let should_bootstrap = {
                    let mut guard = self.last_dht_bootstrap.lock().await;
                    let due = guard.is_none_or(|t| t.elapsed() >= DHT_BOOTSTRAP_RETRY_INTERVAL);
                    if due {
                        *guard = Some(Instant::now());
                    }
                    due
                };
                if should_bootstrap {
                    let boot_harvester = harvester.clone();
                    tokio::spawn(async move {
                        boot_harvester.bootstrap_public_routers().await;
                    });
                }
            }
            self.sync_ed2k_neighborhood_to_dht(state, &harvester).await;
        }

        // B. Bombear y verificar criptográficamente todas las fusiones activas BT → eD2K.
        self.pump_active_fusions(state).await;

        // C. Buscar aceleración híbrida para descargas eD2K activas (sin superar el límite concurrente).
        self.discover_and_attach_for_active_downloads(state).await;

        // D. Cosecha inversa periódica: calcular hash eD2K de vídeos recién completados en BitTorrent.
        if last_completed_scan.elapsed() >= COMPLETED_BT_SCAN_INTERVAL {
            *last_completed_scan = Instant::now();
            self.harvest_completed_torrents(state).await;
        }
    }

    async fn sync_ed2k_neighborhood_to_dht(&self, state: &AppState, harvester: &DhtHarvester) {
        let due = {
            let mut guard = self.last_neighborhood_sync.lock().await;
            let ready = guard.is_none_or(|t| t.elapsed() >= ED2K_NEIGHBORHOOD_SYNC_INTERVAL);
            if ready {
                *guard = Some(Instant::now());
            }
            ready
        };
        if !due {
            return;
        }
        let ips: Vec<IpAddr> = {
            let dl_guard = state.downloads.lock().await;
            let mut seen = HashSet::new();
            let mut out = Vec::new();
            'outer: for d in dl_guard.list_downloads() {
                if d.status == "completed" {
                    continue;
                }
                if let Ok(sources) = dl_guard.db().list_peer_sources(&d.ed2k_hash) {
                    for s in sources {
                        if let Ok(addr) = s.address.parse::<SocketAddr>() {
                            if addr.ip().is_ipv4() && seen.insert(addr.ip()) {
                                out.push(addr.ip());
                                if out.len() >= MAX_ED2K_PEER_IPS_PER_SYNC {
                                    break 'outer;
                                }
                            }
                        }
                    }
                }
            }
            out
        };
        if !ips.is_empty() {
            harvester.promote_ed2k_peer_ips(ips).await;
        }
    }

    async fn pump_active_fusions(&self, state: &AppState) {
        let Some(engine) = state.torrent.get() else {
            return;
        };

        let snapshot_fusions: Vec<(Uuid, ActiveHybridFusion)> = {
            let guard = self.active_fusions.lock().await;
            guard.iter().map(|(k, v)| (*k, v.clone())).collect()
        };

        for (dl_id, mut fusion) in snapshot_fusions {
            let dl_info = {
                let dl_guard = state.downloads.lock().await;
                dl_guard.get_download(&dl_id).map(|s| {
                    let part_hashes = dl_guard
                        .db()
                        .get_part_hashes(&s.ed2k_hash)
                        .unwrap_or_default();
                    (s, part_hashes)
                })
            };

            let Some((dl_snap, part_hashes)) = dl_info else {
                self.cleanup_fusion(state, dl_id, fusion.torrent_id, &fusion.temp_dir)
                    .await;
                continue;
            };

            if matches!(
                dl_snap.status.as_str(),
                "completed" | "verifying" | "completing" | "removed"
            ) {
                self.cleanup_fusion(state, dl_id, fusion.torrent_id, &fusion.temp_dir)
                    .await;
                continue;
            }

            let Some(bt_state) = engine.verified_file_state(fusion.torrent_id, fusion.bt_file_index)
            else {
                self.active_fusions.lock().await.remove(&dl_id);
                continue;
            };

            fusion.peer_count = bt_state.peer_count;
            fusion.bt_download_rate_bps = bt_state.download_rate_bps;

            // Si aún es candidato, exigir validación criptográfica MD4/raíz eD2K antes de inyectar bytes.
            if !fusion.verified {
                let Some(ed2k_bytes) = parse_ed2k_hash_bytes(&dl_snap.ed2k_hash) else {
                    continue;
                };
                let path_clone = bt_state.file_path.clone();
                let display_clone = dl_snap.file_name.clone();
                let verified_ranges = bt_state.verified_ranges.clone();
                let bt_finished = bt_state.finished;
                let total_size = dl_snap.total_bytes;

                let outcome = tokio::task::spawn_blocking(move || {
                    verify_candidate_bt_file_against_ed2k(
                        &path_clone,
                        &display_clone,
                        total_size,
                        &ed2k_bytes,
                        &part_hashes,
                        &verified_ranges,
                        bt_finished,
                    )
                })
                .await
                .unwrap_or(CandidateVerificationOutcome::Pending);

                match outcome {
                    CandidateVerificationOutcome::Pending => {
                        self.active_fusions.lock().await.insert(dl_id, fusion);
                        continue;
                    }
                    CandidateVerificationOutcome::Rejected { reason } => {
                        tracing::warn!(
                            download_id = %dl_id,
                            ed2k_hash = %fusion.ed2k_hash,
                            bt_info_hash = %fusion.bt_info_hash,
                            reason = %reason,
                            "Puente híbrido rechazado criptográficamente; descartando torrent auxiliar"
                        );
                        {
                            let mut dl_guard = state.downloads.lock().await;
                            let _ = dl_guard.db_mut().update_hybrid_bridge_status(
                                &fusion.ed2k_hash,
                                &fusion.bt_info_hash,
                                HybridBridgeStatus::Rejected,
                                0,
                                now_ms(),
                            );
                        }
                        self.counters
                            .bridges_rejected_session
                            .fetch_add(1, Ordering::Relaxed);
                        self.cleanup_fusion(state, dl_id, fusion.torrent_id, &fusion.temp_dir)
                            .await;
                        continue;
                    }
                    CandidateVerificationOutcome::Verified { verified_chunks } => {
                        tracing::info!(
                            download_id = %dl_id,
                            ed2k_hash = %fusion.ed2k_hash,
                            bt_info_hash = %fusion.bt_info_hash,
                            verified_chunks,
                            "Puente híbrido verificado criptográficamente por MD4; iniciando fusión real BT → eD2K"
                        );
                        fusion.verified = true;
                        {
                            let mut dl_guard = state.downloads.lock().await;
                            let _ = dl_guard.db_mut().update_hybrid_bridge_status(
                                &fusion.ed2k_hash,
                                &fusion.bt_info_hash,
                                HybridBridgeStatus::Verified,
                                verified_chunks,
                                now_ms(),
                            );
                        }
                        self.counters
                            .bridges_verified_session
                            .fetch_add(1, Ordering::Relaxed);
                        self.maybe_publish_bridge_to_kad(
                            state,
                            &fusion.ed2k_hash,
                            &dl_snap.file_name,
                            dl_snap.total_bytes,
                            &fusion.bt_info_hash,
                        )
                        .await;
                    }
                }
            }

            // Puente ya verificado: bombear todos los rangos verificados por SHA-1 que falten en eD2K.
            if fusion.verified && !bt_state.verified_ranges.is_empty() {
                match pump_verified_ranges_into_ed2k(
                    &state.downloads,
                    dl_id,
                    &fusion.bt_info_hash,
                    &bt_state.file_path,
                    &bt_state.verified_ranges,
                )
                .await
                {
                    Ok(pump) => {
                        if pump.bytes_injected > 0 {
                            fusion.bytes_fused =
                                fusion.bytes_fused.saturating_add(pump.bytes_injected);
                            self.counters
                                .bytes_fused_from_bt
                                .fetch_add(pump.bytes_injected, Ordering::Relaxed);
                            if let Some(snap) = pump.latest_snapshot {
                                state.broadcast_download_update(snap);
                            }
                        }
                        if pump.ed2k_completed {
                            self.cleanup_fusion(state, dl_id, fusion.torrent_id, &fusion.temp_dir)
                                .await;
                            continue;
                        }
                    }
                    Err(err) => {
                        tracing::debug!(
                            download_id = %dl_id,
                            error = %err,
                            "Error transitorio bombeando bloques híbridos BT → eD2K"
                        );
                    }
                }
            }

            self.active_fusions.lock().await.insert(dl_id, fusion);
        }
    }

    async fn cleanup_fusion(
        &self,
        state: &AppState,
        dl_id: Uuid,
        torrent_id: usize,
        temp_dir: &std::path::Path,
    ) {
        self.active_fusions.lock().await.remove(&dl_id);
        if let Some(engine) = state.torrent.get() {
            let _ = engine.delete(torrent_id, true).await;
        }
        if temp_dir.exists() {
            let _ = tokio::fs::remove_dir_all(temp_dir).await;
        }
    }

    async fn discover_and_attach_for_active_downloads(&self, state: &AppState) {
        let active_count = self.active_fusions.lock().await.len();
        if active_count >= MAX_CONCURRENT_HYBRID_FUSIONS {
            return;
        }

        let candidates: Vec<(Uuid, String, String, u64)> = {
            let dl_guard = state.downloads.lock().await;
            dl_guard
                .list_downloads()
                .into_iter()
                .filter(|d| {
                    d.total_bytes > 0
                        && matches!(
                            d.status.as_str(),
                            "downloading" | "queued" | "waiting" | "waiting_for_sources"
                        )
                })
                .map(|d| (d.id, d.ed2k_hash, d.file_name, d.total_bytes))
                .collect()
        };

        // Limpiar telemetría de descargas que ya finalizaron o fueron eliminadas.
        {
            let active_ids: HashSet<Uuid> = candidates.iter().map(|(id, _, _, _)| *id).collect();
            self.download_telemetry
                .lock()
                .await
                .retain(|id, _| active_ids.contains(id));
        }

        // Pasada 1 (0 ms de red): comprobar TODAS las descargas contra puentes y catálogo DHT ya presentes en SQLite.
        let mut pending_network_discovery = Vec::new();
        for (dl_id, ed2k_hash, file_name, file_size) in candidates {
            if self.active_fusions.lock().await.len() >= MAX_CONCURRENT_HYBRID_FUSIONS {
                return;
            }
            if self.active_fusions.lock().await.contains_key(&dl_id) {
                continue;
            }
            let local_best = {
                let mut dl_guard = state.downloads.lock().await;
                let now = now_ms();
                if let Ok(exact_hits) = dl_guard
                    .db()
                    .find_dht_catalog_by_exact_video_size(file_size, 5)
                {
                    for hit in exact_hits {
                        let rec = HybridBridgeRecord {
                            ed2k_hash: ed2k_hash.clone(),
                            bt_info_hash: hit.info_hash,
                            file_size,
                            bt_file_index: hit.video_file_index,
                            bt_file_offset: hit.video_file_offset,
                            status: HybridBridgeStatus::Candidate,
                            origin: "dht_exact_size".to_string(),
                            verified_chunks: 0,
                            published_kad_at_ms: None,
                            created_at_ms: now,
                            updated_at_ms: now,
                        };
                        let _ = dl_guard.db_mut().upsert_hybrid_bridge(&rec);
                    }
                }
                dl_guard
                    .db()
                    .list_hybrid_bridges_for_ed2k(&ed2k_hash)
                    .ok()
                    .and_then(|list| {
                        list.into_iter()
                            .find(|b| b.status != HybridBridgeStatus::Rejected)
                    })
            };

            if let Some(best_bridge) = local_best {
                if self
                    .launch_hybrid_fusion(state, dl_id, &best_bridge, Vec::new())
                    .await
                {
                    continue;
                }
            }
            pending_network_discovery.push((dl_id, ed2k_hash, file_name, file_size));
        }

        // Pasada 2 (acotada a 1 descarga por tick): consultar búsqueda activa (Torznab/DHT), Kad Notes y Rendezvous.
        let mut slow_queries_done = 0usize;
        for (dl_id, ed2k_hash, file_name, file_size) in pending_network_discovery {
            if slow_queries_done >= MAX_SLOW_NETWORK_DISCOVERIES_PER_TICK
                || self.active_fusions.lock().await.len() >= MAX_CONCURRENT_HYBRID_FUSIONS
            {
                break;
            }
            let needs_active = self
                .last_active_discovery
                .lock()
                .await
                .get(&dl_id)
                .is_none_or(|t| t.elapsed() >= ACTIVE_DISCOVERY_COOLDOWN);
            let needs_kad = self
                .last_kad_query
                .lock()
                .await
                .get(&dl_id)
                .is_none_or(|t| t.elapsed() >= KAD_NOTE_DISCOVERY_COOLDOWN);
            let needs_rdv = self
                .last_rendezvous_query
                .lock()
                .await
                .get(&dl_id)
                .is_none_or(|t| t.elapsed() >= RENDEZVOUS_DISCOVERY_COOLDOWN);
            if !needs_active && !needs_kad && !needs_rdv {
                continue;
            }
            let _ = self
                .run_discovery_for_single_download(
                    state,
                    dl_id,
                    &ed2k_hash,
                    &file_name,
                    file_size,
                    false,
                )
                .await;
            slow_queries_done += 1;
        }
    }

    async fn run_discovery_for_single_download(
        &self,
        state: &AppState,
        dl_id: Uuid,
        ed2k_hash: &str,
        file_name: &str,
        file_size: u64,
        force: bool,
    ) -> HybridDiscoveryReport {
        let now = now_ms();
        let mut dht_exact_matches = 0usize;
        let mut kad_note_candidates = 0usize;
        let mut rendezvous_peers: Vec<SocketAddr> = Vec::new();
        let mut active_outcome = ActiveDiscoveryOutcome::default();

        // 1. Cruce instantáneo O(log N) por tamaño exacto al byte en el catálogo DHT local.
        {
            let mut dl_guard = state.downloads.lock().await;
            if let Ok(exact_hits) = dl_guard
                .db()
                .find_dht_catalog_by_exact_video_size(file_size, 5)
            {
                dht_exact_matches = exact_hits.len();
                for hit in exact_hits {
                    let rec = HybridBridgeRecord {
                        ed2k_hash: ed2k_hash.to_string(),
                        bt_info_hash: hit.info_hash,
                        file_size,
                        bt_file_index: hit.video_file_index,
                        bt_file_offset: hit.video_file_offset,
                        status: HybridBridgeStatus::Candidate,
                        origin: "dht_exact_size".to_string(),
                        verified_chunks: 0,
                        published_kad_at_ms: None,
                        created_at_ms: now,
                        updated_at_ms: now,
                    };
                    let _ = dl_guard.db_mut().upsert_hybrid_bridge(&rec);
                }
            }
        }

        // 1b. Búsqueda activa dirigida en catálogo DHT por tokens e indexadores Torznab/Prowlarr.
        let should_run_active = if force {
            self.last_active_discovery
                .lock()
                .await
                .insert(dl_id, Instant::now());
            true
        } else {
            let mut guard = self.last_active_discovery.lock().await;
            let due = guard
                .get(&dl_id)
                .is_none_or(|t| t.elapsed() >= ACTIVE_DISCOVERY_COOLDOWN);
            if due {
                guard.insert(dl_id, Instant::now());
            }
            due
        };
        if should_run_active {
            let harvester_opt = self.harvester.read().await.clone();
            active_outcome = discover_bridges_actively(
                state,
                harvester_opt.as_ref(),
                ed2k_hash,
                file_name,
                file_size,
            )
            .await;
            if dht_exact_matches == 0 {
                dht_exact_matches = active_outcome.exact_matches_found;
            }
        }

        // 2. Descubrimiento de puentes publicados por otros nodos mediante notas Kad.
        let should_query_kad = if force {
            true
        } else {
            let mut guard = self.last_kad_query.lock().await;
            let due = guard
                .get(&dl_id)
                .is_none_or(|t| t.elapsed() >= KAD_NOTE_DISCOVERY_COOLDOWN);
            if due {
                guard.insert(dl_id, Instant::now());
            }
            due
        };
        if should_query_kad {
            let kad_btihs = discover_bridges_via_kad_notes(state, ed2k_hash, file_size).await;
            kad_note_candidates = kad_btihs.len();
            if !kad_btihs.is_empty() {
                let mut dl_guard = state.downloads.lock().await;
                for btih in kad_btihs {
                    let rec = HybridBridgeRecord {
                        ed2k_hash: ed2k_hash.to_string(),
                        bt_info_hash: btih,
                        file_size,
                        bt_file_index: 0,
                        bt_file_offset: 0,
                        status: HybridBridgeStatus::Candidate,
                        origin: "kad_note".to_string(),
                        verified_chunks: 0,
                        published_kad_at_ms: None,
                        created_at_ms: now,
                        updated_at_ms: now,
                    };
                    let _ = dl_guard.db_mut().upsert_hybrid_bridge(&rec);
                }
            }
        }

        // 3. Punto de encuentro determinista (Rendezvous) en la DHT de BitTorrent.
        let should_query_rdv = if force {
            true
        } else {
            let mut guard = self.last_rendezvous_query.lock().await;
            let due = guard
                .get(&dl_id)
                .is_none_or(|t| t.elapsed() >= RENDEZVOUS_DISCOVERY_COOLDOWN);
            if due {
                guard.insert(dl_id, Instant::now());
            }
            due
        };
        if should_query_rdv {
            if let (Some(hash_bytes), Some(harvester)) = (
                parse_ed2k_hash_bytes(ed2k_hash),
                self.harvester.read().await.clone(),
            ) {
                let ed2k_port = Some(state.ed2k_port());
                rendezvous_peers = harvester
                    .announce_and_find_rendezvous(hash_bytes, ed2k_port)
                    .await;
                if !rendezvous_peers.is_empty() {
                    self.counters
                        .rendezvous_peers_found
                        .fetch_add(rendezvous_peers.len() as u64, Ordering::Relaxed);
                    self.inject_rendezvous_ed2k_sources(state, ed2k_hash, &rendezvous_peers, now)
                        .await;
                }
            }
        }

        // 4. Seleccionar el mejor puente no rechazado e iniciar la fusión si aún no está activa.
        let bridges = {
            let dl_guard = state.downloads.lock().await;
            dl_guard
                .db()
                .list_hybrid_bridges_for_ed2k(ed2k_hash)
                .unwrap_or_default()
        };

        let mut active_fusion_started = false;
        if !self.active_fusions.lock().await.contains_key(&dl_id) {
            if let Some(best) = bridges
                .iter()
                .find(|b| b.status != HybridBridgeStatus::Rejected)
            {
                active_fusion_started = self
                    .launch_hybrid_fusion(state, dl_id, best, Vec::new())
                    .await;
            }
        }

        let summary = build_telemetry_summary(
            &active_outcome,
            dht_exact_matches,
            kad_note_candidates,
            &bridges,
            active_fusion_started,
        );
        self.download_telemetry.lock().await.insert(
            dl_id,
            DownloadHybridTelemetry {
                download_id: dl_id.to_string(),
                ed2k_hash: ed2k_hash.to_string(),
                last_checked_ms: now_ms(),
                enabled_indexers: active_outcome.enabled_indexers,
                torznab_candidates_checked: active_outcome.torznab_candidates_checked,
                public_candidates_checked: active_outcome.public_candidates_checked,
                dht_candidates_checked: active_outcome.dht_token_candidates_checked,
                kad_notes_found: kad_note_candidates,
                rendezvous_peers_found: rendezvous_peers.len(),
                exact_matches_found: dht_exact_matches,
                best_equivalent: active_outcome.best_equivalent.clone(),
                status_summary: summary,
            },
        );

        HybridDiscoveryReport {
            download_id: dl_id.to_string(),
            ed2k_hash: ed2k_hash.to_string(),
            file_size,
            dht_exact_matches,
            kad_note_candidates,
            rendezvous_peers_found: rendezvous_peers.len(),
            enabled_indexers: active_outcome.enabled_indexers,
            torznab_candidates_checked: active_outcome.torznab_candidates_checked,
            public_candidates_checked: active_outcome.public_candidates_checked,
            best_equivalent: active_outcome.best_equivalent,
            active_fusion_started,
            bridges,
        }
    }

    async fn inject_rendezvous_ed2k_sources(
        &self,
        state: &AppState,
        ed2k_hash: &str,
        peers: &[SocketAddr],
        now: i64,
    ) {
        let filter = state.network_policy.ip_filter();
        let profile = state.network_policy.profile();
        let mut sources = Vec::new();
        for &addr in peers {
            let SocketAddr::V4(v4) = addr else {
                continue;
            };
            if addr.port() == 0
                || filter.check_ip(addr.ip()).is_some()
                || (profile.is_loopback_only() && !addr.ip().is_loopback())
            {
                continue;
            }
            sources.push(PeerSource {
                file_hash: ed2k_hash.to_string(),
                address: addr.to_string(),
                ip: u32::from_be_bytes(v4.ip().octets()),
                port: addr.port(),
                origin: PeerSourceOrigin::Kad,
                discovered_at_unix_ms: now,
                last_seen_unix_ms: now,
                status: PeerSourceStatus::Allowed,
                fail_count: 0,
                last_error: None,
                cooldown_until_unix_ms: None,
                user_hash: None,
                crypt_options: 0,
                buddy_id: None,
                software: None,
                availability: None,
                server_ip: 0,
                server_port: 0,
                udp_port: 0,
            });
        }
        if !sources.is_empty() {
            let _ = state
                .downloads
                .lock()
                .await
                .upsert_peer_sources(ed2k_hash, &sources);
        }
    }

    async fn launch_hybrid_fusion(
        &self,
        state: &AppState,
        dl_id: Uuid,
        bridge: &HybridBridgeRecord,
        mut extra_peers: Vec<SocketAddr>,
    ) -> bool {
        let Ok(engine) = state.torrent_engine().await else {
            tracing::warn!(
                download_id = %dl_id,
                bt_info_hash = %bridge.bt_info_hash,
                "Motor BitTorrent no disponible para lanzar fusión híbrida"
            );
            return false;
        };

        // Cargar metadatos `.torrent` cacheados (`zstd`) y pares conocidos para arranque instantáneo.
        let cached_torrent_bytes: Option<Vec<u8>> = {
            let dl_guard = state.downloads.lock().await;
            match dl_guard.db().load_dht_torrent_meta(&bridge.bt_info_hash) {
                Ok(Some((meta_opt, peers_json))) => {
                    if let Ok(peer_strs) = serde_json::from_str::<Vec<String>>(&peers_json) {
                        for ps in peer_strs {
                            if let Ok(addr) = ps.parse::<SocketAddr>() {
                                if !extra_peers.contains(&addr) {
                                    extra_peers.push(addr);
                                }
                            }
                        }
                    }
                    meta_opt.and_then(|comp| zstd::stream::decode_all(comp.as_slice()).ok())
                }
                _ => None,
            }
        };

        // Consultar también pares vivos en el Sabueso DHT como refuerzo inmediato.
        if let (Some(harvester), Ok(raw_ih)) = (
            self.harvester.read().await.clone(),
            hex::decode(&bridge.bt_info_hash),
        ) {
            if raw_ih.len() == 20 {
                let mut ih = [0u8; 20];
                ih.copy_from_slice(&raw_ih);
                for peer in harvester.find_peers_for_infohash(ih).await {
                    if !extra_peers.contains(&peer) {
                        extra_peers.push(peer);
                    }
                }
            }
        }

        let temp_dir = state
            .config
            .data_dir
            .join("temp")
            .join(HYBRID_BT_TEMP_SUBDIR)
            .join(&bridge.ed2k_hash);
        if tokio::fs::create_dir_all(&temp_dir).await.is_err() {
            return false;
        }

        let launch_res = if let Some(torrent_bytes) = cached_torrent_bytes {
            engine
                .add_hybrid_torrent_bytes_for_exact_size(
                    torrent_bytes,
                    bridge.file_size,
                    temp_dir.clone(),
                    extra_peers,
                )
                .await
        } else {
            engine
                .add_hybrid_torrent_for_exact_size(
                    &bridge.bt_info_hash,
                    bridge.file_size,
                    temp_dir.clone(),
                    extra_peers,
                )
                .await
        };

        match launch_res {
            Ok(launched) => {
                tracing::info!(
                    download_id = %dl_id,
                    ed2k_hash = %bridge.ed2k_hash,
                    bt_info_hash = %launched.info_hash,
                    file_index = launched.file_index,
                    file_size = launched.file_size,
                    "Torrent auxiliar híbrido iniciado para acelerar descarga eD2K"
                );
                let fusion = ActiveHybridFusion {
                    download_id: dl_id.to_string(),
                    ed2k_hash: bridge.ed2k_hash.clone(),
                    bt_info_hash: launched.info_hash,
                    torrent_id: launched.torrent_id,
                    bt_file_index: launched.file_index,
                    file_size: launched.file_size,
                    verified: bridge.status == HybridBridgeStatus::Verified,
                    bytes_fused: 0,
                    peer_count: 0,
                    bt_download_rate_bps: 0,
                    temp_dir,
                };
                self.active_fusions.lock().await.insert(dl_id, fusion);
                true
            }
            Err(err) => {
                tracing::warn!(
                    download_id = %dl_id,
                    bt_info_hash = %bridge.bt_info_hash,
                    error = %err,
                    "No se pudo lanzar el torrent auxiliar híbrido"
                );
                false
            }
        }
    }

    async fn harvest_completed_torrents(&self, state: &AppState) {
        let Some(engine) = state.torrent.get() else {
            return;
        };
        let completed = engine.list_completed_video_files();
        for video in completed {
            let cache_key = format!("{}:{}", video.info_hash, video.file_index);
            {
                let mut seen = self.hashed_completed_bt.lock().await;
                if seen.contains(&cache_key) {
                    continue;
                }
                seen.insert(cache_key);
            }

            let path = video.file_path.clone();
            let hash_res = tokio::task::spawn_blocking(
                move || -> Result<mule_format::ed2k::Ed2kHashResult, String> {
                    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                    let mut reader = mule_core::EvictingFileReader::new(file);
                    mule_format::ed2k::hash_reader(&mut reader).map_err(|e| e.to_string())
                },
            )
            .await;

            let Ok(Ok(computed)) = hash_res else {
                continue;
            };
            let ed2k_hex = computed.root_hash.to_hex();
            let now = now_ms();

            {
                let mut dl_guard = state.downloads.lock().await;
                let catalog_entry = DhtTorrentCatalogEntry {
                    info_hash: video.info_hash.clone(),
                    name: video.torrent_name.clone(),
                    total_bytes: video.total_torrent_bytes,
                    video_file_name: video.file_name.clone(),
                    video_file_size: video.file_size,
                    video_file_index: video.file_index,
                    video_file_offset: video.file_offset,
                    files_json: "[]".to_string(),
                    estimated_peers: 1,
                    source: "local_completed".to_string(),
                    first_seen_ms: now,
                    last_seen_ms: now,
                };
                let _ = dl_guard.db_mut().upsert_dht_catalog_entry(&catalog_entry);

                let bridge = HybridBridgeRecord {
                    ed2k_hash: ed2k_hex.clone(),
                    bt_info_hash: video.info_hash.clone(),
                    file_size: video.file_size,
                    bt_file_index: video.file_index,
                    bt_file_offset: video.file_offset,
                    status: HybridBridgeStatus::Verified,
                    origin: "local_hash".to_string(),
                    verified_chunks: computed.part_hashes.len().max(1) as u32,
                    published_kad_at_ms: None,
                    created_at_ms: now,
                    updated_at_ms: now,
                };
                let _ = dl_guard.db_mut().upsert_hybrid_bridge(&bridge);
            }

            self.counters
                .completed_bt_hashed
                .fetch_add(1, Ordering::Relaxed);
            tracing::info!(
                ed2k_hash = %ed2k_hex,
                bt_info_hash = %video.info_hash,
                file = %video.file_name,
                "Cosecha inversa BT → eD2K completada; puente verificado registrado"
            );

            self.maybe_publish_bridge_to_kad(
                state,
                &ed2k_hex,
                &video.file_name,
                video.file_size,
                &video.info_hash,
            )
            .await;

            if let Some(harvester) = self.harvester.read().await.clone() {
                let _ = harvester
                    .announce_and_find_rendezvous(computed.root_hash.0, Some(state.ed2k_port()))
                    .await;
            }
        }
    }

    async fn maybe_publish_bridge_to_kad(
        &self,
        state: &AppState,
        ed2k_hash: &str,
        file_name: &str,
        file_size: u64,
        bt_info_hash: &str,
    ) {
        let now = now_ms();
        let should_publish = {
            let dl_guard = state.downloads.lock().await;
            dl_guard
                .db()
                .list_hybrid_bridges_for_ed2k(ed2k_hash)
                .ok()
                .and_then(|list| {
                    list.into_iter()
                        .find(|b| b.bt_info_hash.eq_ignore_ascii_case(bt_info_hash))
                })
                .is_none_or(|b| {
                    b.published_kad_at_ms
                        .is_none_or(|ts| now.saturating_sub(ts) >= KAD_NOTE_REPUBLISH_INTERVAL_MS)
                })
        };
        if !should_publish {
            return;
        }
        if let Ok(accepted) =
            publish_verified_bridge_to_kad(state, ed2k_hash, file_name, file_size, bt_info_hash)
                .await
        {
            if accepted > 0 {
                let mut dl_guard = state.downloads.lock().await;
                let _ = dl_guard.db_mut().mark_hybrid_bridge_kad_published(
                    ed2k_hash,
                    bt_info_hash,
                    now,
                );
                self.counters
                    .kad_notes_published
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Devuelve la instantánea completa de estado y telemetría del motor híbrido.
    pub async fn status_snapshot(&self, state: &AppState) -> HybridStatusSnapshot {
        let dht_harvester = match self.harvester.read().await.clone() {
            Some(h) => h.stats().await,
            None => DhtHarvesterStats::default(),
        };
        let catalog = {
            let dl_guard = state.downloads.lock().await;
            dl_guard.db().get_hybrid_catalog_stats().unwrap_or_default()
        };
        let active_fusions: Vec<ActiveHybridFusion> = self
            .active_fusions
            .lock()
            .await
            .values()
            .cloned()
            .collect();
        let download_telemetry: Vec<DownloadHybridTelemetry> = self
            .download_telemetry
            .lock()
            .await
            .values()
            .cloned()
            .collect();

        HybridStatusSnapshot {
            dht_harvester,
            catalog,
            active_fusions,
            download_telemetry,
            bytes_fused_from_bt: self.counters.bytes_fused_from_bt.load(Ordering::Relaxed),
            bridges_verified_session: self
                .counters
                .bridges_verified_session
                .load(Ordering::Relaxed),
            bridges_rejected_session: self
                .counters
                .bridges_rejected_session
                .load(Ordering::Relaxed),
            kad_notes_published: self.counters.kad_notes_published.load(Ordering::Relaxed),
            rendezvous_peers_found: self.counters.rendezvous_peers_found.load(Ordering::Relaxed),
            completed_bt_hashed: self.counters.completed_bt_hashed.load(Ordering::Relaxed),
        }
    }

    /// Fuerza una ronda inmediata de descubrimiento híbrido automático (catálogo DHT + Torznab + Kad Notes + Rendezvous)
    /// para la descarga eD2K indicada.
    pub async fn discover_for_download(
        &self,
        state: &AppState,
        download_id: Uuid,
    ) -> Result<HybridDiscoveryReport, String> {
        let dl_snap = {
            let dl_guard = state.downloads.lock().await;
            dl_guard
                .get_download(&download_id)
                .ok_or_else(|| format!("descarga eD2K '{download_id}' no encontrada"))?
        };
        Ok(self
            .run_discovery_for_single_download(
                state,
                download_id,
                &dl_snap.ed2k_hash,
                &dl_snap.file_name,
                dl_snap.total_bytes,
                true,
            )
            .await)
    }
}

fn build_telemetry_summary(
    active: &ActiveDiscoveryOutcome,
    dht_exact_matches: usize,
    kad_note_candidates: usize,
    bridges: &[HybridBridgeRecord],
    active_fusion_started: bool,
) -> String {
    if active_fusion_started {
        return "Puente BT encontrado y fusión híbrida iniciada".to_string();
    }
    if bridges
        .iter()
        .any(|b| b.status != HybridBridgeStatus::Rejected)
    {
        return "Puente candidato encontrado en espera de turno de fusión".to_string();
    }
    let rejected_count = bridges
        .iter()
        .filter(|b| b.status == HybridBridgeStatus::Rejected)
        .count();
    if rejected_count > 0 {
        return format!(
            "Sin copia bit a bit válida ({rejected_count} candidato(s) rechazado(s) por MD4)"
        );
    }
    let total_checked = active
        .torznab_candidates_checked
        .saturating_add(active.public_candidates_checked)
        .saturating_add(active.dht_token_candidates_checked);
    if dht_exact_matches > 0 {
        return format!(
            "{dht_exact_matches} coincidencia(s) exacta(s) encontradas; preparando validación MD4"
        );
    }
    if let Some(ref eq) = active.best_equivalent {
        return format!(
            "Sin copia bit a bit en BT ({total_checked} torrents revisados) · Equivalente: {} ({} semillas)",
            eq.name, eq.seeders
        );
    }
    if active.enabled_indexers == 0 {
        format!(
            "Sin copia bit a bit ({total_checked} candidatos en BT público/DHT local y {kad_note_candidates} notas Kad)"
        )
    } else {
        format!(
            "Sin copia bit a bit en Torrent ({} candidatos revisados en {} indexador(es), BT público y DHT local)",
            total_checked, active.enabled_indexers
        )
    }
}

fn parse_ed2k_hash_bytes(hex_str: &str) -> Option<[u8; 16]> {
    let raw = hex::decode(hex_str.trim()).ok()?;
    if raw.len() != 16 {
        return None;
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(&raw);
    Some(out)
}

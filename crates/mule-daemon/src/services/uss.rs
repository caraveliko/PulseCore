// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! USS (Upload Speed Sense) — limitador dinámico de subida basado en la
//! latencia de red medida, adaptado de `LastCommonRouteFinder` de eMule.
//!
//! Al igual que en eMule, el objetivo de sondeo es el «último host común»: se
//! trazan rutas ICMP hacia una muestra de pares remotos (servidor, clientes de
//! subida, fuentes de descarga) y se hace ping al salto más profundo
//! compartido por todas las rutas — normalmente el router de agregación del
//! ISP. Esto mide específicamente la congestión en el enlace de subida *local*.
//! En Windows utiliza `IcmpSendEcho` (sin privilegios); sin un host común o en
//! otras plataformas recurre al RTT de conexión TCP frente al servidor eD2K
//! conectado o a un `host:port` configurado. La ley de control en sí es la
//! misma:
//!
//! ```text
//! soll_ping = ms_mode ? tolerance_ms : initial_ping * ((tolerance-100)/100)
//! normalized_ping = median(last N pings) - initial_ping
//! hping = soll_ping - normalized_ping
//! hping < 0 -> upload += hping * 1024 * 10 / (goingDownDivider * initial_ping)
//! hping > 0 -> upload += hping * 1024 * 10 / (goingUpDivider * initial_ping)
//! clamp [min_upload, max_upload]
//! ```
//!
//! Cuando la latencia supera la tolerancia, `accept_new_client` pasa a false —
//! la cola de subida mantiene entonces a los nuevos clientes en espera
//! (`AcceptNewClient` de eMule).

use crate::services::peer_session::log_peer_ip;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::task::AbortHandle;

use mule_network_policy::cancellation::CancellationToken;
use mule_network_policy::PolicyRateLimiters;

use crate::services::downloads::DownloadService;
use crate::services::server_session::{ServerConnectionStatus, ServerSessionManager};
use crate::services::upload_budget::{compute_effective_ceiling, split_upload_budget, CeilingInputs};
use crate::services::upload_queue::UploadManager;
use crate::services::uss_icmp;

const USS_SETTINGS_KEY: &str = "uss_config";
/// eMule `USSEnabled` default: off.
const DEFAULT_ENABLED: bool = false;
/// eMule `USSPingTolerance` default: 500 → `(500-100)/100 = 4.0` × initial ping.
const DEFAULT_TOLERANCE_PERCENT: u32 = 500;
/// eMule `USSPingToleranceMilliseconds` default: 200 ms.
const DEFAULT_TOLERANCE_MS: u32 = 200;
/// eMule `USSGoingUpDivider` / `USSGoingDownDivider` defaults.
const DEFAULT_DIVIDER: u32 = 1000;
/// Valor predeterminado de `USSNumberOfPings` en eMule: 1.
const DEFAULT_NUM_PINGS: usize = 1;
/// Techo de subida predeterminado en reposo cuando no hay descargas activas (0 = desactivado).
const DEFAULT_IDLE_UPLOAD_CAP_BPS: u64 = 0;
/// Umbral de bajada agregada por debajo del cual se considera la línea en reposo (1 KiB/s).
const IDLE_DOWNLOAD_THRESHOLD_BPS: u64 = 1024;
/// `uLowestInitialPingAllowed` codificado en `UploadQueue.cpp` de eMule (20 ms).
const LOWEST_INITIAL_PING_MS: u32 = 20;
/// Sondeos utilizados para establecer el RTT de línea base (`initialPingCounter = 10` de eMule).
const BASELINE_PROBES: usize = 10;
/// Tiempo de espera de sondeo.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
/// Tiempo de espera de sondeo ICMP (ping al último host común).
const ICMP_PROBE_TIMEOUT_MS: u32 = 2000;
/// Abandonar el bucle de línea base / control tras este número de fallos
/// consecutivos de sondeo (eMule reinicia tras 60).
const MAX_CONSECUTIVE_FAILURES: u32 = 60;
/// El destino de ping resuelto se almacena en caché este tiempo — el descubrimiento de rutas se ejecuta
/// como máximo una vez por TTL (eMule reevalúa su conjunto de hosts periódicamente).
const TARGET_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);
/// Destinos máximos de traceroute muestreados (eMule mantiene un conjunto pequeño de hosts).
const MAX_TRACE_DESTINATIONS: usize = 8;

/// Destino de sondeo — `Icmp` es el último host común descubierto (LCRF de eMule),
/// `Tcp` es el respaldo de RTT de conexión (`pingHost` explícito o servidor
/// eD2K conectado).
#[derive(Debug, Clone, PartialEq, Eq)]
enum PingTarget {
    Icmp(std::net::Ipv4Addr),
    Tcp(SocketAddr),
}

impl std::fmt::Display for PingTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PingTarget::Icmp(ip) => write!(f, "icmp:{ip}"),
            PingTarget::Tcp(addr) => write!(f, "tcp:{addr}"),
        }
    }
}
/// La velocidad de subida nunca baja de este suelo cuando USS la controla (1 KiB/s,
/// límite inferior `m_minUpload` de eMule).
const MIN_UPLOAD_BPS: u64 = 1024;

/// Configuración de USS, persistida como JSON en la tabla de ajustes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UssConfig {
    pub enabled: bool,
    /// Destino de sondeo `host:port` opcional; recurre al servidor conectado.
    pub ping_host: Option<String>,
    /// Límite inferior del límite dinámico (bytes/s).
    pub min_upload_bps: u64,
    /// Límite superior del límite dinámico (bytes/s); 0 = sin límite
    /// (`UNLIMITED` de eMule — el límite fluctúa libremente).
    pub max_upload_bps: u64,
    /// true → `soll_ping = ping_tolerance_ms`; false → modo porcentaje.
    pub use_ms_tolerance: bool,
    /// Valor sin procesar de `USSPingTolerance` de eMule; factor = (v-100)/100.
    pub ping_tolerance_percent: u32,
    /// Techo de ping objetivo en ms (modo de tolerancia en ms).
    pub ping_tolerance_ms: u32,
    pub going_up_divider: u32,
    pub going_down_divider: u32,
    /// Tamaño de ventana de la mediana (`USSNumberOfPings`).
    pub number_of_pings: usize,
    /// Techo de subida en reposo cuando no hay descargas activas (bytes/s; 0 = desactivado).
    #[serde(default)]
    pub idle_upload_cap_bps: u64,
    /// Techo de subida durante reproducciones activas en Jellyfin/Plex (bytes/s; 0 = desactivado).
    #[serde(default)]
    pub streaming_upload_cap_bps: u64,
}

impl Default for UssConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_ENABLED,
            ping_host: None,
            min_upload_bps: 10 * 1024,
            max_upload_bps: 0,
            use_ms_tolerance: false,
            ping_tolerance_percent: DEFAULT_TOLERANCE_PERCENT,
            ping_tolerance_ms: DEFAULT_TOLERANCE_MS,
            going_up_divider: DEFAULT_DIVIDER,
            going_down_divider: DEFAULT_DIVIDER,
            number_of_pings: DEFAULT_NUM_PINGS,
            idle_upload_cap_bps: DEFAULT_IDLE_UPLOAD_CAP_BPS,
            streaming_upload_cap_bps: 0,
        }
    }
}

/// Instantánea de tiempo de ejecución expuesta mediante RPC.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UssSnapshot {
    pub enabled: bool,
    /// `disabled` | `waiting` (sin destino de sondeo) | `preparing` (línea base) |
    /// `active` | `error`.
    pub state: &'static str,
    /// Límite de subida dinámico actual en bytes/s (0 cuando USS está apagado).
    pub current_limit_bps: u64,
    /// Latencia mediana de los últimos sondeos (ms).
    pub latency_ms: u32,
    /// Latencia de línea base (más baja observada) (ms).
    pub lowest_ping_ms: u32,
    /// Retardo de cola estimado respecto a la línea base (ms).
    pub queuing_delay_ms: i64,
    /// Motivo actual de regulación (`disabled`, `waiting`, `baseline`, `normal`, `latency`, `idle_cap`, `streaming`, `quota_eco`).
    pub reason: &'static str,
    /// `true` cuando está aplicado el techo en reposo por ausencia de descargas activas.
    pub idle_cap_active: bool,
    /// `true` cuando el techo temporal por streaming (Jellyfin/Plex) está activo.
    pub streaming_cap_active: bool,
    /// Número de reproducciones activas detectadas en Jellyfin/Plex.
    pub active_streams: usize,
    /// `true` cuando está activo el modo eco por cuota diaria agotada.
    pub quota_eco_active: bool,
    /// Presupuesto efectivo asignado a eD2K (bytes/s; 0 = ilimitado).
    pub ed2k_upload_limit_bps: u64,
    /// Presupuesto efectivo asignado a BitTorrent (bytes/s; 0 = ilimitado).
    pub torrent_upload_limit_bps: u64,
    /// Si actualmente se aceptan nuevos clientes de subida.
    pub accept_new_client: bool,
    /// Destino de sondeo efectivamente en uso.
    pub ping_target: Option<String>,
}

/// Edad máxima aceptable para utilizar muestras pasivas LEDBAT++ como respaldo (ms).
const LEDBAT_FRESH_WINDOW_MS: i64 = 30_000;

struct UssInner {
    config: UssConfig,
    state: &'static str,
    current_limit_bps: u64,
    latency_ms: u32,
    lowest_ping_ms: u32,
    queuing_delay_ms: i64,
    reason: &'static str,
    idle_cap_active: bool,
    streaming_active: bool,
    active_streams: usize,
    streaming_cap_active: bool,
    quota_eco_cap_bps: u64,
    quota_eco_active: bool,
    ed2k_upload_limit_bps: u64,
    torrent_upload_limit_bps: u64,
    ping_target: Option<String>,
    /// Límite de subida configurado manualmente mediante `limits.set` — restaurado cuando
    /// USS está deshabilitado.
    manual_limit_bps: u64,
    /// Límite de descarga *configurado* (`rate_limit_download_bps`) — el cubo de
    /// descarga se vuelve a ajustar a `emule_ud_ratio(dl_cfg, upload)` en cada
    /// ajuste dinámico (`GetMaxDownloadInBytesPerSec` dinámico de eMule).
    download_limit_bps: u64,
    /// Estimador pasivo LEDBAT++ para respaldo cuando ICMP/TCP no están disponibles.
    ledbat: crate::services::uss_ledbat::LedbatEstimator,
}

/// Referencia diferida al motor BitTorrent compartido con `AppState`.
type TorrentCell = Arc<tokio::sync::OnceCell<Arc<mule_torrent::TorrentEngine>>>;

/// Controlador USS. Controla `upload_bucket` cuando está habilitado.
#[derive(Clone)]
pub struct UssService {
    inner: Arc<RwLock<UssInner>>,
    accept_new_client: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    task: Arc<Mutex<Option<AbortHandle>>>,
    server_session: Arc<RwLock<Option<ServerSessionManager>>>,
    upload_manager: Arc<RwLock<Option<UploadManager>>>,
    downloads: Arc<RwLock<Option<Arc<tokio::sync::Mutex<DownloadService>>>>>,
    torrent: Arc<RwLock<Option<TorrentCell>>>,
}

impl Default for UssService {
    fn default() -> Self {
        Self::new()
    }
}

impl UssService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(UssInner {
                config: UssConfig::default(),
                state: "disabled",
                current_limit_bps: 0,
                latency_ms: 0,
                lowest_ping_ms: 0,
                queuing_delay_ms: 0,
                reason: "disabled",
                idle_cap_active: false,
                streaming_active: false,
                active_streams: 0,
                streaming_cap_active: false,
                quota_eco_cap_bps: 0,
                quota_eco_active: false,
                ed2k_upload_limit_bps: 0,
                torrent_upload_limit_bps: 0,
                ping_target: None,
                manual_limit_bps: 0,
                download_limit_bps: 0,
                ledbat: crate::services::uss_ledbat::LedbatEstimator::new(),
            })),
            accept_new_client: Arc::new(AtomicBool::new(true)),
            running: Arc::new(AtomicBool::new(false)),
            task: Arc::new(Mutex::new(None)),
            server_session: Arc::new(RwLock::new(None)),
            upload_manager: Arc::new(RwLock::new(None)),
            downloads: Arc::new(RwLock::new(None)),
            torrent: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_torrent_cell(
        &self,
        cell: Arc<tokio::sync::OnceCell<Arc<mule_torrent::TorrentEngine>>>,
    ) {
        *self.torrent.write().unwrap_or_else(|e| e.into_inner()) = Some(cell);
    }

    /// Actualiza el estado de reproducciones activas detectadas en Jellyfin/Plex.
    pub fn set_streaming_active(&self, active: bool, streams: usize) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.streaming_active = active;
        inner.active_streams = streams;
    }

    /// Establece o limpia el techo de subida en modo eco al alcanzar la cuota diaria.
    pub fn set_quota_eco_cap_bps(&self, cap_bps: Option<u64>) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.quota_eco_cap_bps = cap_bps.unwrap_or(0);
    }

    fn current_torrent_rates(&self) -> (u64, u64) {
        let cell_opt = self.torrent.read().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(cell) = cell_opt {
            if let Some(engine) = cell.get() {
                return engine.total_rates_bps();
            }
        }
        (0, 0)
    }

    /// Reparte un presupuesto global de subida entre eD2K y BitTorrent según demanda real.
    pub async fn apply_split_upload_limit(&self, total_bps: u64, limiters: &PolicyRateLimiters) {
        let ed2k_active = self.current_upload_rate().await;
        let (_, bt_active) = self.current_torrent_rates();
        let split = split_upload_budget(total_bps, ed2k_active, bt_active);
        limiters
            .upload_bucket
            .set_rate(split.ed2k_bps, split.ed2k_bps.max(65536));
        let cell_opt = self.torrent.read().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(cell) = cell_opt {
            if let Some(engine) = cell.get() {
                engine.set_upload_limit_bps(split.torrent_bps);
            }
        }
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.ed2k_upload_limit_bps = split.ed2k_bps;
        inner.torrent_upload_limit_bps = split.torrent_bps;
    }

    pub fn set_server_session(&self, mgr: ServerSessionManager) {
        *self.server_session.write().unwrap_or_else(|e| e.into_inner()) = Some(mgr);
    }

    pub fn set_upload_manager(&self, mgr: UploadManager) {
        *self.upload_manager.write().unwrap_or_else(|e| e.into_inner()) = Some(mgr);
    }

    pub fn set_downloads(&self, dl: Arc<tokio::sync::Mutex<DownloadService>>) {
        *self.downloads.write().unwrap_or_else(|e| e.into_inner()) = Some(dl);
        // La configuración persistida se carga de forma diferida en la primera
        // iteración del bucle de control (evita bloqueos en `AppState::new`).
    }

    /// Alimenta el estimador pasivo LEDBAT++ con una muestra reciente de RTT (ms)
    /// observada en las sesiones de transferencia o saludo.
    pub fn record_passive_rtt_sample(&self, rtt_ms: u32) {
        if rtt_ms > 0 {
            self.inner
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .ledbat
                .record_rtt_sample(crate::services::downloads::types::now_ms(), rtt_ms);
        }
    }

    async fn apply_ledbat_step(
        &self,
        upload_bps: &mut u64,
        cfg: &UssConfig,
        download_bps: u64,
        limiters: &PolicyRateLimiters,
        activate_state: bool,
    ) -> bool {
        let now_ms = crate::services::downloads::types::now_ms();
        let is_idle = download_bps < IDLE_DOWNLOAD_THRESHOLD_BPS;
        let (streaming_active, quota_eco_cap_bps) = {
            let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
            (inner.streaming_active, inner.quota_eco_cap_bps)
        };
        let decision = compute_effective_ceiling(CeilingInputs {
            max_upload_bps: cfg.max_upload_bps,
            idle_upload_cap_bps: cfg.idle_upload_cap_bps,
            is_idle,
            streaming_upload_cap_bps: cfg.streaming_upload_cap_bps,
            streaming_active,
            quota_eco_cap_bps,
        });
        let (next_bps, cur_rtt, base_rtt, qdelay, congested) = {
            let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
            if !inner.ledbat.has_fresh_samples(now_ms, LEDBAT_FRESH_WINDOW_MS) {
                return false;
            }
            let min_bps = cfg.min_upload_bps.max(MIN_UPLOAD_BPS);
            let seed_bps = (*upload_bps).max(min_bps);
            let Some(next) = inner
                .ledbat
                .compute_rate_adjustment(seed_bps, min_bps, decision.cap_bps)
            else {
                return false;
            };
            let rtt = inner.ledbat.current_delay_ms().unwrap_or(0);
            let base = inner.ledbat.base_delay_ms().unwrap_or(0);
            let qd = inner.ledbat.queuing_delay_ms().unwrap_or(0);
            let cong = qd > crate::services::uss_ledbat::TARGET_QUEUING_DELAY_MS;
            (next, rtt, base, qd, cong)
        };
        *upload_bps = next_bps;
        self.accept_new_client.store(!congested, Ordering::Relaxed);
        self.apply_split_upload_limit(next_bps, limiters).await;
        let eff_dl = mule_network_policy::emule_ud_ratio(
            self.inner.read().unwrap_or_else(|e| e.into_inner()).download_limit_bps,
            next_bps,
        );
        limiters.download_bucket.set_rate(eff_dl, eff_dl.max(65536));
        if activate_state {
            self.set_state("active");
        }
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.current_limit_bps = next_bps;
        inner.latency_ms = cur_rtt;
        if base_rtt > 0 {
            inner.lowest_ping_ms = base_rtt;
        }
        inner.queuing_delay_ms = qdelay.max(0);
        inner.idle_cap_active = decision.idle_cap_active;
        inner.streaming_cap_active = decision.streaming_cap_active;
        inner.quota_eco_active = decision.quota_eco_active;
        inner.reason = if congested {
            "latency"
        } else if decision.cap_bps > 0 && next_bps >= decision.cap_bps {
            decision.reason
        } else {
            "normal"
        };
        true
    }

    /// Indicador compartido consultado por `UploadManager` antes de conceder ranuras
    /// (`LastCommonRouteFinder::AcceptNewClient` de eMule).
    pub fn accept_flag(&self) -> Arc<AtomicBool> {
        self.accept_new_client.clone()
    }

    pub fn config(&self) -> UssConfig {
        self.inner.read().unwrap_or_else(|e| e.into_inner()).config.clone()
    }

    pub fn snapshot(&self) -> UssSnapshot {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        UssSnapshot {
            enabled: inner.config.enabled,
            state: inner.state,
            current_limit_bps: inner.current_limit_bps,
            latency_ms: inner.latency_ms,
            lowest_ping_ms: inner.lowest_ping_ms,
            queuing_delay_ms: inner.queuing_delay_ms,
            reason: inner.reason,
            idle_cap_active: inner.idle_cap_active,
            streaming_cap_active: inner.streaming_cap_active,
            active_streams: inner.active_streams,
            quota_eco_active: inner.quota_eco_active,
            ed2k_upload_limit_bps: inner.ed2k_upload_limit_bps,
            torrent_upload_limit_bps: inner.torrent_upload_limit_bps,
            accept_new_client: self.accept_new_client.load(Ordering::Relaxed),
            ping_target: inner.ping_target.clone(),
        }
    }

    /// Aplica una nueva configuración (actualización parcial), la persiste y notifica al
    /// bucle de control reiniciando la tarea.
    pub async fn update_config<F: FnOnce(&mut UssConfig)>(&self, apply: F) {
        let (json, dl) = {
            let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
            apply(&mut inner.config);
            if !inner.config.enabled {
                inner.state = "disabled";
                inner.current_limit_bps = 0;
                inner.queuing_delay_ms = 0;
                inner.reason = "disabled";
                inner.idle_cap_active = false;
                self.accept_new_client.store(true, Ordering::Relaxed);
            }
            let dl = self.downloads.read().unwrap_or_else(|e| e.into_inner()).clone();
            (serde_json::to_string(&inner.config).ok(), dl)
        };
        if let (Some(json), Some(dl)) = (json, dl) {
            let _ = dl.lock().await.set_setting(USS_SETTINGS_KEY, &json);
        }
    }

    /// Registra el límite de subida manual configurado mediante `limits.set` para que USS
    /// pueda restaurarlo cuando se deshabilite.
    pub fn note_manual_limit(&self, bps: u64) {
        self.inner.write().unwrap_or_else(|e| e.into_inner()).manual_limit_bps = bps;
    }

    /// Registra el límite de descarga *configurado* (`limits.set`, límite de capacidad,
    /// arranque) para que el bucle de control pueda volver a ajustar el cubo de descarga
    /// con la proporción de subida/descarga de eMule en cada actualización dinámica.
    pub fn note_manual_download_limit(&self, bps: u64) {
        self.inner.write().unwrap_or_else(|e| e.into_inner()).download_limit_bps = bps;
    }

    /// Inicia el bucle de control en segundo plano. Idempotente.
    pub fn start(&self, limiters: Arc<PolicyRateLimiters>, cancel: CancellationToken) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let svc = self.clone();
        let running = Arc::clone(&self.running);
        let handle = tokio::spawn(async move {
            svc.run(limiters, cancel).await;
            // El interruptor de corte cancela el bucle mediante el token; se libera la
            // bandera para que `network.enable` pueda reiniciar el servicio.
            running.store(false, Ordering::SeqCst);
        });
        if let Ok(mut t) = self.task.try_lock() {
            *t = Some(handle.abort_handle());
        }
    }

    async fn run(&self, limiters: Arc<PolicyRateLimiters>, cancel: CancellationToken) {
        let upload_bucket = &limiters.upload_bucket;
        let download_bucket = &limiters.download_bucket;
        let mut delays: VecDeque<u32> = VecDeque::new();
        let mut initial_ping_ms: u32 = 0;
        let mut upload_bps: u64 = 0;
        let mut consecutive_failures: u32 = 0;
        let mut baseline_done = false;
        let loop_start = Instant::now();
        // Destino de ping resuelto + marca de tiempo — el descubrimiento del último
        // router común (traceroutes ICMP) es costoso: se vuelve a ejecutar a lo
        // sumo cada TARGET_CACHE_TTL, o tras recalcular la línea base.
        let mut cached_target: Option<(PingTarget, Instant)> = None;

        // Cargar configuración persistida una sola vez.
        let dl_opt = { self.downloads.read().unwrap_or_else(|e| e.into_inner()).clone() };
        if let Some(dl) = dl_opt {
            if let Ok(Some(json)) = dl.lock().await.get_setting(USS_SETTINGS_KEY) {
                if let Ok(cfg) = serde_json::from_str::<UssConfig>(&json) {
                    self.inner.write().unwrap_or_else(|e| e.into_inner()).config = cfg;
                }
            }
            // Límite de descarga configurado persistido (ratio dinámico de eMule).
            if let Ok(Some(s)) = dl.lock().await.get_setting("rate_limit_download_bps") {
                if let Ok(v) = s.trim().parse::<u64>() {
                    self.inner.write().unwrap_or_else(|e| e.into_inner()).download_limit_bps = v;
                }
            }
        }

        /// Vuelve a ajustar el límite del cubo de descarga a la proporción de eMule
        /// (`GetMaxDownloadInBytesPerSec`) para el límite de subida actual.
        macro_rules! apply_ud_ratio {
            ($ul_eff:expr) => {{
                let dl_cfg = self.inner.read().unwrap_or_else(|e| e.into_inner()).download_limit_bps;
                let eff = mule_network_policy::emule_ud_ratio(dl_cfg, $ul_eff);
                download_bucket.set_rate(eff, eff.max(65536));
            }};
        }

        loop {
            if cancel.is_cancelled() {
                return;
            }
            let cfg = self.config();
            if !cfg.enabled {
                let (manual, streaming_active, quota_eco_cap_bps) = {
                    let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
                    (
                        inner.manual_limit_bps,
                        inner.streaming_active,
                        inner.quota_eco_cap_bps,
                    )
                };
                let ctx_decision = compute_effective_ceiling(CeilingInputs {
                    max_upload_bps: manual,
                    idle_upload_cap_bps: 0,
                    is_idle: false,
                    streaming_upload_cap_bps: cfg.streaming_upload_cap_bps,
                    streaming_active,
                    quota_eco_cap_bps,
                });
                if ctx_decision.streaming_cap_active || ctx_decision.quota_eco_active {
                    let eff = ctx_decision.cap_bps;
                    self.apply_split_upload_limit(eff, &limiters).await;
                    apply_ud_ratio!(eff);
                    let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
                    inner.state = "disabled";
                    inner.current_limit_bps = eff;
                    inner.queuing_delay_ms = 0;
                    inner.reason = ctx_decision.reason;
                    inner.idle_cap_active = false;
                    inner.streaming_cap_active = ctx_decision.streaming_cap_active;
                    inner.quota_eco_active = ctx_decision.quota_eco_active;
                } else {
                    // Restaurar límite manual una vez por transición.
                    let needs_restore = {
                        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
                        inner.state != "disabled"
                            || inner.current_limit_bps != 0
                            || inner.streaming_cap_active
                            || inner.quota_eco_active
                    };
                    if needs_restore {
                        self.apply_split_upload_limit(manual, &limiters).await;
                        apply_ud_ratio!(manual);
                        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
                        inner.state = "disabled";
                        inner.current_limit_bps = 0;
                        inner.queuing_delay_ms = 0;
                        inner.reason = "disabled";
                        inner.idle_cap_active = false;
                        inner.streaming_cap_active = false;
                        inner.quota_eco_active = false;
                    }
                }
                baseline_done = false;
                delays.clear();
                initial_ping_ms = 0;
                self.accept_new_client.store(true, Ordering::Relaxed);
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                }
                continue;
            }

            // Resolver destino de sondeo (en caché — el descubrimiento de rutas es costoso).
            let stale = cached_target
                .as_ref()
                .is_none_or(|(_, at)| at.elapsed() >= TARGET_CACHE_TTL);
            if stale {
                cached_target = self.resolve_target(&cfg).await.map(|t| (t, Instant::now()));
            }
            let Some((target, _)) = cached_target.clone() else {
                let cur_download = self.current_download_rate().await;
                if self
                    .apply_ledbat_step(&mut upload_bps, &cfg, cur_download, &limiters, true)
                    .await
                {
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                    }
                    continue;
                }
                self.set_state_and_reason("waiting", "waiting");
                baseline_done = false;
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
                }
                continue;
            };
            self.inner.write().unwrap_or_else(|e| e.into_inner()).ping_target = Some(target.to_string());

            // --- Línea base: el menor de hasta BASELINE_PROBES sondeos ---
            if !baseline_done {
                self.set_state_and_reason("preparing", "baseline");
                let mut best = u32::MAX;
                let mut successes = 0usize;
                for _ in 0..BASELINE_PROBES {
                    if cancel.is_cancelled() || !self.config().enabled {
                        break;
                    }
                    if let Some(ms) = probe_target(&target).await {
                        successes += 1;
                        if ms > 0 && ms < best {
                            best = ms.max(LOWEST_INITIAL_PING_MS);
                        }
                    }
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {}
                    }
                }
                if !self.config().enabled {
                    continue;
                }
                if successes == 0 {
                    let cur_download = self.current_download_rate().await;
                    if self
                        .apply_ledbat_step(&mut upload_bps, &cfg, cur_download, &limiters, true)
                        .await
                    {
                        tokio::select! {
                            _ = cancel.cancelled() => return,
                            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                        }
                        continue;
                    }
                    // Destino inalcanzable — esperar antes de reintentar.
                    self.set_state_and_reason("waiting", "waiting");
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {}
                    }
                    continue;
                }
                initial_ping_ms = if best == u32::MAX {
                    LOWEST_INITIAL_PING_MS
                } else {
                    best
                };
                // eMule: subida = max(curUpload, minUpload), acotado al máximo.
                let cur_upload = self.current_upload_rate().await;
                let max = cfg.max_upload_bps;
                upload_bps = cur_upload.max(cfg.min_upload_bps.max(MIN_UPLOAD_BPS));
                if max > 0 {
                    upload_bps = upload_bps.min(max);
                }
                upload_bucket.set_rate(upload_bps, upload_bps.max(65536));
                apply_ud_ratio!(upload_bps);
                delays.clear();
                consecutive_failures = 0;
                baseline_done = true;
                self.set_state("active");
                self.inner.write().unwrap_or_else(|e| e.into_inner()).lowest_ping_ms = initial_ping_ms;
            }

            // --- Bucle de control ---
            // eMule: ticksBetweenPings = 100*64/upload*1000 ms, acotado a
            // [125ms, 1s] — los sondeos utilizan ~1% del ancho de banda de subida.
            let interval_ms = 6_400_000u64
                .checked_div(upload_bps)
                .map_or(1000, |v| v.clamp(125, 1000));
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(std::time::Duration::from_millis(interval_ms)) => {}
            }
            if cancel.is_cancelled() {
                return;
            }
            let cfg = self.config();
            if !cfg.enabled {
                continue;
            }

            // soll_ping: el modo ms usa la tolerancia fija; el modo porcentaje
            // usa initial_ping * (tolerance-100)/100 (UploadQueue.cpp:921).
            let soll_ping: i64 = if cfg.use_ms_tolerance {
                cfg.ping_tolerance_ms as i64
            } else {
                let factor = (cfg.ping_tolerance_percent.max(101) - 100) as f64 / 100.0;
                (initial_ping_ms as f64 * factor) as i64
            };

            let raw_ping: i64 = match probe_target(&target).await {
                Some(ms) => {
                    consecutive_failures = 0;
                    let ms = ms.max(1);
                    // Registrar nuevo ping más bajo (eMule reduce initial_ping).
                    if ms < initial_ping_ms && initial_ping_ms > LOWEST_INITIAL_PING_MS {
                        initial_ping_ms = ms.max(LOWEST_INITIAL_PING_MS);
                        self.inner.write().unwrap_or_else(|e| e.into_inner()).lowest_ping_ms = initial_ping_ms;
                    }
                    ms as i64
                }
                None => {
                    let cur_download = self.current_download_rate().await;
                    if self
                        .apply_ledbat_step(&mut upload_bps, &cfg, cur_download, &limiters, false)
                        .await
                    {
                        consecutive_failures = 0;
                        continue;
                    }
                    consecutive_failures += 1;
                    // eMule: un ping fallido cuenta como 3*(soll+initial) → fuerza
                    // una disminución.
                    (soll_ping + initial_ping_ms as i64) * 3
                }
            };

            if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                // Recalcular línea base: la topología/destino puede haber cambiado — forzar
                // también la resolución del destino (descubrimiento de ruta).
                baseline_done = false;
                cached_target = None;
                delays.clear();
                continue;
            }

            delays.push_back(raw_ping.max(0) as u32);
            while delays.len() > cfg.number_of_pings.max(1) {
                delays.pop_front();
            }
            let ping_average = median(&delays);

            // eMule incrementa/reduce divisores durante los primeros ~60 s tras el inicio.
            let elapsed = loop_start.elapsed().as_secs();
            let (mut up_div, mut down_div) =
                (cfg.going_up_divider.max(1), cfg.going_down_divider.max(1));
            if elapsed < 20 {
                up_div = 1;
                down_div = 1;
            } else {
                let mul = if elapsed < 30 {
                    1
                } else if elapsed < 40 {
                    2
                } else if elapsed < 60 {
                    3
                } else {
                    4
                };
                down_div = (down_div * mul / 4).max(1);
                up_div = (up_div * mul / 4).max(1);
            }

            let normalized_ping = ping_average as i64 - initial_ping_ms as i64;
            let hping = soll_ping - normalized_ping;

            let cur_upload = self.current_upload_rate().await;
            if hping < 0 {
                // Latencia superior a la tolerancia → reducir la tasa, rechazar nuevos clientes.
                self.accept_new_client.store(false, Ordering::Relaxed);
                let diff = hping * 1024 * 10 / down_div as i64 / initial_ping_ms.max(1) as i64;
                upload_bps = if upload_bps as i64 + diff > 0 {
                    (upload_bps as i64 + diff) as u64
                } else {
                    0
                };
            } else if hping > 0 {
                self.accept_new_client.store(true, Ordering::Relaxed);
                // eMule solo aumenta mientras el rendimiento real sigue el límite
                // (curUpload + 30KB > upload).
                if cur_upload + 30 * 1024 > upload_bps {
                    let diff = hping * 1024 * 10 / (up_div as i64 * initial_ping_ms.max(1) as i64);
                    upload_bps = upload_bps.saturating_add(diff.max(0) as u64);
                }
            }

            let cur_download = self.current_download_rate().await;
            let is_idle = cur_download < IDLE_DOWNLOAD_THRESHOLD_BPS;
            let (streaming_active, quota_eco_cap_bps) = {
                let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
                (inner.streaming_active, inner.quota_eco_cap_bps)
            };
            let decision = compute_effective_ceiling(CeilingInputs {
                max_upload_bps: cfg.max_upload_bps,
                idle_upload_cap_bps: cfg.idle_upload_cap_bps,
                is_idle,
                streaming_upload_cap_bps: cfg.streaming_upload_cap_bps,
                streaming_active,
                quota_eco_cap_bps,
            });
            upload_bps = upload_bps.max(cfg.min_upload_bps.max(MIN_UPLOAD_BPS));
            if decision.cap_bps > 0 {
                upload_bps = upload_bps.min(decision.cap_bps);
            }
            self.apply_split_upload_limit(upload_bps, &limiters).await;
            apply_ud_ratio!(upload_bps);

            {
                let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
                inner.current_limit_bps = upload_bps;
                inner.latency_ms = ping_average;
                inner.queuing_delay_ms = normalized_ping.max(0);
                inner.idle_cap_active = decision.idle_cap_active;
                inner.streaming_cap_active = decision.streaming_cap_active;
                inner.quota_eco_active = decision.quota_eco_active;
                inner.reason = if hping < 0 {
                    "latency"
                } else if decision.cap_bps > 0 && upload_bps >= decision.cap_bps {
                    decision.reason
                } else {
                    "normal"
                };
            }
        }
    }

    /// Caudal agregado de descarga actual (eD2K + BitTorrent en bytes/s) para detectar modo reposo.
    async fn current_download_rate(&self) -> u64 {
        let ed2k_dl: u64 = {
            let dl_opt = self
                .downloads
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            match dl_opt {
                Some(dl) => dl
                    .lock()
                    .await
                    .list_downloads()
                    .iter()
                    .map(|d| d.download_rate_bps)
                    .sum(),
                None => 0,
            }
        };
        let (bt_dl, _) = self.current_torrent_rates();
        ed2k_dl.saturating_add(bt_dl)
    }

    /// Rendimiento de subida alcanzado actual (suma de tasas de ranuras activas) —
    /// eMule utiliza `uploadqueue->GetDatarate()` como `curUpload`.
    async fn current_upload_rate(&self) -> u64 {
        let mgr = self.upload_manager.read().unwrap_or_else(|e| e.into_inner()).clone();
        match mgr {
            Some(mgr) => mgr
                .active_snapshots()
                .await
                .iter()
                .map(|s| s.upload_rate_bps)
                .sum(),
            None => 0,
        }
    }

    async fn resolve_target(&self, cfg: &UssConfig) -> Option<PingTarget> {
        // Prevalece la anulación explícita (`uss.pingHost` en `limits.set`).
        if let Some(host) = &cfg.ping_host {
            if let Ok(addr) = host.parse::<SocketAddr>() {
                return Some(PingTarget::Tcp(addr));
            }
        }
        // `LastCommonRouteFinder` de eMule: traza rutas ICMP hacia una muestra
        // de hosts remotos y hace ping al salto compartido más profundo — el router
        // del lado del ISP donde realmente se manifiesta la congestión de subida local.
        if let Some(ip) = self.discover_last_common_host().await {
            tracing::info!(
                "USS: last common host discovered, pinging {peer}",
                peer = log_peer_ip(ip)
            );
            return Some(PingTarget::Icmp(ip));
        }
        // Respaldo: RTT de conexión TCP al servidor eD2K conectado.
        let mgr = self.server_session.read().unwrap_or_else(|e| e.into_inner()).clone()?;
        let snap = mgr.snapshot().await;
        if snap.status != ServerConnectionStatus::Connected {
            return None;
        }
        snap.server_address
            .and_then(|s| s.parse().ok())
            .map(PingTarget::Tcp)
    }

    /// Traza rutas ICMP hacia pares remotos conocidos (servidor conectado,
    /// clientes de subida, fuentes de descarga) y devuelve el salto más profundo
    /// común a todas las rutas. `None` si ICMP no está disponible o se conocen menos
    /// de dos destinos distintos (una sola ruta no tiene ningún salto "común" significativo).
    async fn discover_last_common_host(&self) -> Option<std::net::Ipv4Addr> {
        if !uss_icmp::icmp_available() {
            return None;
        }
        let dests = self.collect_destinations().await;
        if dests.len() < 2 {
            return None;
        }
        let mut routes = Vec::new();
        for dest in dests {
            match uss_icmp::trace_route(dest).await {
                Some(hops) => routes.push(hops),
                None => tracing::debug!("USS: no ICMP route to {peer}", peer = log_peer_ip(dest)),
            }
        }
        let host = uss_icmp::last_common_host(&routes);
        if host.is_none() && !routes.is_empty() {
            tracing::debug!("USS: {} routes traced but no common hop", routes.len());
        }
        host
    }

    /// Destinos IPv4 públicos distintos para el descubrimiento de rutas: servidor eD2K
    /// conectado, clientes de subida activos y fuentes de descarga (eMule
    /// traza rutas hacia pares con los que está conectado).
    async fn collect_destinations(&self) -> Vec<std::net::Ipv4Addr> {
        let mut ips: Vec<std::net::Ipv4Addr> = Vec::new();

        let server = { self.server_session.read().unwrap_or_else(|e| e.into_inner()).clone() };
        if let Some(mgr) = server {
            let snap = mgr.snapshot().await;
            if snap.status == ServerConnectionStatus::Connected {
                if let Some(Ok(SocketAddr::V4(a))) =
                    snap.server_address.map(|s| s.parse::<SocketAddr>())
                {
                    ips.push(*a.ip());
                }
            }
        }
        let uploader = { self.upload_manager.read().unwrap_or_else(|e| e.into_inner()).clone() };
        if let Some(mgr) = uploader {
            for s in mgr.active_snapshots().await {
                if let Ok(SocketAddr::V4(a)) = s.client_address.parse::<SocketAddr>() {
                    ips.push(*a.ip());
                }
            }
        }
        let downloads = { self.downloads.read().unwrap_or_else(|e| e.into_inner()).clone() };
        if let Some(dl) = downloads {
            let dl = dl.lock().await;
            for d in dl.list_downloads() {
                if let Some(sources) = dl.get_sources(&d.id) {
                    for addr in sources.keys() {
                        if let SocketAddr::V4(a) = addr {
                            ips.push(*a.ip());
                        }
                    }
                }
            }
        }

        // Mantener solo IPs públicas enrutables — saltos LAN/loopback contaminarían
        // la detección del salto común (la pasarela doméstica se comparte de forma trivial).
        ips.retain(|ip| {
            mule_network_policy::classify_ip(std::net::IpAddr::V4(*ip))
                == mule_network_policy::IpCategory::PublicRoutable
        });
        ips.sort_unstable();
        ips.dedup();
        ips.truncate(MAX_TRACE_DESTINATIONS);
        ips
    }

    fn set_state(&self, state: &'static str) {
        self.inner.write().unwrap_or_else(|e| e.into_inner()).state = state;
    }

    fn set_state_and_reason(&self, state: &'static str, reason: &'static str) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.state = state;
        inner.reason = reason;
    }
}

/// Mide el RTT hacia `target` en milisegundos — eco ICMP para el
/// último host común descubierto (comportamiento de eMule), conexión TCP para los
/// destinos de respaldo.
async fn probe_target(target: &PingTarget) -> Option<u32> {
    match target {
        PingTarget::Icmp(ip) => uss_icmp::ping(*ip, ICMP_PROBE_TIMEOUT_MS).await,
        PingTarget::Tcp(addr) => probe_tcp_rtt(*addr).await,
    }
}

/// Mide el RTT de conexión TCP a `addr` en milisegundos.
async fn probe_tcp_rtt(addr: SocketAddr) -> Option<u32> {
    let start = Instant::now();
    match tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect(addr)).await {
        Ok(Ok(_stream)) => Some(start.elapsed().as_millis() as u32),
        _ => None,
    }
}

/// Mediana de la ventana de retardo (`LastCommonRouteFinder::Median`).
fn median(delays: &VecDeque<u32>) -> u32 {
    if delays.is_empty() {
        return 0;
    }
    let mut v: Vec<u32> = delays.iter().copied().collect();
    v.sort_unstable();
    v[v.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_of_window() {
        let mut d = VecDeque::new();
        assert_eq!(median(&d), 0);
        d.extend([10, 30, 20]);
        assert_eq!(median(&d), 20);
        let d2: VecDeque<u32> = [10, 20, 30, 40].into();
        assert_eq!(median(&d2), 30);
    }

    #[test]
    fn default_config_matches_emule() {
        let c = UssConfig::default();
        assert!(!c.enabled);
        assert_eq!(c.ping_tolerance_percent, 500);
        assert_eq!(c.ping_tolerance_ms, 200);
        assert_eq!(c.going_up_divider, 1000);
        assert_eq!(c.going_down_divider, 1000);
        assert_eq!(c.number_of_pings, 1);
    }

    #[tokio::test]
    async fn snapshot_disabled_by_default() {
        let svc = UssService::new();
        let snap = svc.snapshot();
        assert_eq!(snap.state, "disabled");
        assert_eq!(snap.reason, "disabled");
        assert!(!snap.idle_cap_active);
        assert!(snap.accept_new_client);
        assert!(svc.accept_flag().load(Ordering::Relaxed));
    }

    #[test]
    fn legacy_uss_config_deserializes_without_idle_or_streaming_cap() {
        let legacy = r#"{"enabled":true,"pingHost":null,"minUploadBps":10240,"maxUploadBps":0,"useMsTolerance":false,"pingTolerancePercent":500,"pingToleranceMs":200,"goingUpDivider":1000,"goingDownDivider":1000,"numberOfPings":1}"#;
        let parsed: UssConfig = serde_json::from_str(legacy).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.idle_upload_cap_bps, 0);
        assert_eq!(parsed.streaming_upload_cap_bps, 0);
    }

    #[test]
    fn streaming_and_quota_eco_flags_update_inner_state() {
        let svc = UssService::new();
        svc.set_streaming_active(true, 2);
        svc.set_quota_eco_cap_bps(Some(128 * 1024));
        assert_eq!(svc.snapshot().active_streams, 2);
        svc.set_streaming_active(false, 0);
        svc.set_quota_eco_cap_bps(None);
        assert_eq!(svc.snapshot().active_streams, 0);
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

use mule_network_policy::{NetworkPolicyManager, ResourceBudgets};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use crate::config::DaemonConfig;
use crate::services::downloads::types::DownloadSnapshot;
use crate::services::downloads::DownloadService;
use crate::services::peer_session::PeerSessionManager;
use crate::services::server_session::ServerSessionManager;
use crate::services::DaemonEvent;

/// Cadencia de refresco de la instantánea de `sharedfiles` utilizada por
/// el UploadManager para purgar las solicitudes huérfanas — en pulsos (ticks) de 1 s
/// (eMule lee la lista en directo en cada promoción; aquí una instantánea de
/// ~15 s es suficiente, los archivos compartidos rara vez cambian).
const SHARED_HASH_REFRESH_TICKS: u64 = 15;
/// Clave de ajuste persistido para el hash de la contraseña opcional de la WebUI.
pub const SETTING_WEBUI_PASSWORD_HASH: &str = "webui_password_hash";
/// Prefijo de sal estático para derivar el hash robusto de la contraseña de la WebUI mediante SHA-256.
const WEBUI_PASSWORD_SALT_PREFIX: &str = "pulse-core-webui-v2:";

/// Control en caliente del `EnvFilter` global (clave `logLevel` de
/// `settings.set`). El binario instala el `reload::Handle` en el arranque;
/// los manejadores RPC solo tienen que llamar después a [`LogControl::set`].
///
/// El nivel deliberadamente **no se persiste**: un nivel `trace` persistido
/// tras un reinicio saturaría los archivos de registro (datos sensibles y
/// gran volumen). `RUST_LOG` sigue siendo la vía para un nivel permanente.
#[derive(Clone)]
pub struct LogControl {
    handle: Arc<
        std::sync::Mutex<
            Option<
                tracing_subscriber::reload::Handle<
                    tracing_subscriber::EnvFilter,
                    tracing_subscriber::Registry,
                >,
            >,
        >,
    >,
    current: Arc<std::sync::Mutex<String>>,
}

impl Default for LogControl {
    fn default() -> Self {
        Self::new("info")
    }
}

impl LogControl {
    pub fn new(initial_spec: impl Into<String>) -> Self {
        Self {
            handle: Arc::new(std::sync::Mutex::new(None)),
            current: Arc::new(std::sync::Mutex::new(initial_spec.into())),
        }
    }

    /// Invocado una vez por el punto de entrada: el `reload::Handle` reside en
    /// `main` (asociado al suscriptor real) y no puede construirse aquí.
    pub fn install(
        &self,
        handle: tracing_subscriber::reload::Handle<
            tracing_subscriber::EnvFilter,
            tracing_subscriber::Registry,
        >,
    ) {
        if let Ok(mut slot) = self.handle.lock() {
            *slot = Some(handle);
        }
    }

    /// Aplica una nueva especificación de `EnvFilter` («debug», «info»,
    /// «info,mule_daemon::kad=trace»…).
    pub fn set(&self, spec: &str) -> Result<(), String> {
        let filter = spec
            .parse::<tracing_subscriber::EnvFilter>()
            .map_err(|e| format!("spec de filtre invalide: {e}"))?;
        let guard = self
            .handle
            .lock()
            .map_err(|_| "verrou log empoisonné".to_string())?;
        let handle = guard
            .as_ref()
            .ok_or_else(|| "handle de reload non installé".to_string())?;
        handle.reload(filter).map_err(|e| e.to_string())?;
        drop(guard);
        if let Ok(mut cur) = self.current.lock() {
            *cur = spec.to_string();
        }
        Ok(())
    }

    /// Especificación de filtro actualmente vigente (o el valor inicial si
    /// el manejador todavía no está instalado).
    pub fn current(&self) -> String {
        self.current
            .lock()
            .map(|c| c.clone())
            .unwrap_or_else(|_| "info".to_string())
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: DaemonConfig,
    pub downloads: Arc<Mutex<DownloadService>>,
    pub network_policy: NetworkPolicyManager,
    pub server_session: ServerSessionManager,
    pub peer_session: PeerSessionManager,
    /// Source Exchange compartido: directiva de red, identidad local y registros
    /// de limitación (throttle) de eMule (`SetLastSrcReqTime` / `GetLastAskedForSources` /
    /// `m_ClientSrcAnswered`) — inyectado en `peer_session` e
    /// `inbound_listener`.
    pub sx_env: crate::services::peer_session::source_exchange::SxEnv,
    pub inbound_listener: crate::services::inbound_listener::InboundListenerService,
    pub identity: Arc<Mutex<crate::services::IdentityService>>,
    /// Contadores compartidos de anomalías SUI (objeto `sui` de identity.status).
    pub sui_stats: Arc<crate::services::identity::SuiStats>,
    pub kad: Arc<crate::services::KadService>,
    /// Servicio UDP de cliente a cliente que comparte el socket UDP de Kad
    /// (`OP_REASKFILEPING`/`OP_REASKACK`/`OP_QUEUEFULL`/`OP_FILENOTFOUND`).
    pub client_udp: crate::services::ClientUdpService,
    /// Servicio UDP de servidor (`OP_GLOBGETSOURCES2`/`OP_GLOBFOUNDSOURCES`),
    /// socket separado equivalente al `UDPSocket` de eMule.
    pub server_udp: crate::services::server_udp::ServerUdpService,
    /// USS (Upload Speed Sense) — limitador dinámico de subida guiado por
    /// la latencia medida hacia el servidor conectado o un host configurado.
    pub uss: crate::services::UssService,
    pub upnp: Arc<crate::services::UpnpService>,
    pub sequence: Arc<AtomicU64>,
    pub event_tx: broadcast::Sender<String>,
    pub instance_id: Uuid,
    pub session_id: Uuid,
    pub abort_handles: Arc<Mutex<Vec<tokio::task::AbortHandle>>>,
    pub shutdown_notify: Arc<tokio::sync::Notify>,
    /// Puertos efectivos (los ajustes persistidos sobrescriben a `DaemonConfig`):
    /// oyente TCP de eD2K y Kad UDP. Refleja Preferencias de eMule → Conexión.
    ed2k_port_eff: Arc<std::sync::atomic::AtomicU16>,
    kad_port_eff: Arc<std::sync::atomic::AtomicU16>,
    /// Indica si UDP (Kad + UDP de servidor + UDP de cliente) está habilitado —
    /// casilla «UDP desactivado» de eMule.
    udp_enabled: Arc<AtomicBool>,
    /// Compuerta de la cuota diaria de subida (`dailyUploadQuotaGb`, eMuleRust):
    /// `false` bloquea las nuevas solicitudes de ranura. Controlado por
    /// `spawn_quota_watcher`.
    upload_quota_gate: Arc<AtomicBool>,
    /// Totales acumulados desde la instalación (persistidos en los ajustes de BD,
    /// incrementados con el delta de sesión en cada `statsTick`, restablecidos a cero
    /// mediante `stats.resetLifetimeTotals`).
    pub lifetime_downloaded_bytes: Arc<AtomicU64>,
    pub lifetime_uploaded_bytes: Arc<AtomicU64>,
    #[cfg(feature = "lab-tools")]
    pub active_lab_peer: Arc<Mutex<Option<(std::net::SocketAddr, tokio::task::AbortHandle)>>>,
    /// Control del nivel de registro en caliente (clave `logLevel` de `settings.set`).
    /// `None` en pruebas/harness que no instalan un suscriptor.
    pub log_control: Option<LogControl>,
    pub torrent: Arc<tokio::sync::OnceCell<Arc<mule_torrent::TorrentEngine>>>,
    /// Seudonimizador de sesión (`mule-filter`): las direcciones de pares
    /// externos registradas mediante [`AppState::log_peer_addr`] pasan a ser
    /// `peer-NN:port` — nunca una IP pública en bruto en los archivos
    /// persistentes (loopback en claro, inofensivo para el laboratorio).
    pub pseudonymizer: Arc<mule_filter::SessionPseudonymizer>,
    /// Limitador FIFO estricto de copia hacia el NAS/destino (1 transferencia simultánea).
    /// Evita la contienda de E/S y la saturación de CPU del Synology DS212j.
    pub nas_transfer_semaphore: Arc<tokio::sync::Semaphore>,
    /// Identificador de la descarga que posee actualmente la ranura de copia al NAS.
    pub active_nas_transfer_id: Arc<tokio::sync::Mutex<Option<Uuid>>>,
    /// Cliente de metadatos para el gestor multimedia unificado (reemplazo de Radarr/Sonarr/TMDB).
    pub media_metadata: Arc<crate::services::MediaMetadataService>,
    /// Hash en memoria de la contraseña opcional de la WebUI para peticiones en red local.
    pub webui_password_hash: Arc<std::sync::RwLock<Option<String>>>,
    /// Servicio de comprobación, descarga y aplicación de actualizaciones en caliente.
    pub updater: Arc<crate::services::updater::UpdaterService>,
    /// Transporte QUIC P2P autenticado por Ed25519 (gestionado con el ciclo de vida UDP).
    pub quic_transport:
        Arc<tokio::sync::RwLock<Option<Arc<crate::services::quic_transport::QuicPeerTransport>>>>,
}

impl AppState {
    pub fn new(config: DaemonConfig, mut downloads: DownloadService) -> Self {
        let (event_tx, _) = broadcast::channel(1024);
        let (max_per_dl, max_global) = {
            let max_g = downloads
                .get_setting("max_global_download_peers")
                .ok()
                .flatten()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(config.max_global_download_peers);
            let max_f = downloads
                .get_setting("max_peers_per_download")
                .ok()
                .flatten()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(config.max_peers_per_download);
            (max_f, max_g)
        };

        let mut budgets = ResourceBudgets::default();
        if budgets.max_active_peer_connections < max_global {
            budgets.max_active_peer_connections =
                max_global.min(mule_network_policy::MAX_PEER_CONNECTIONS_CAP);
        }
        // Sobrescrituras persistidas de Conexión (Preferencias de eMule) aplicadas en el arranque.
        let get_usize = |key: &str| {
            downloads
                .get_setting(key)
                .ok()
                .flatten()
                .and_then(|s| s.parse::<usize>().ok())
        };
        if let Some(v) = get_usize("max_connections") {
            budgets.max_active_peer_connections =
                v.clamp(1, mule_network_policy::MAX_PEER_CONNECTIONS_CAP);
        }
        if let Some(v) = get_usize("max_conn_per_min") {
            budgets.rate_limit_per_min = (v as u32).clamp(1, 1200);
        }
        let bool_setting = |key: &str| -> Option<bool> {
            downloads
                .get_setting(key)
                .ok()
                .flatten()
                .map(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "on"))
        };
        let obf_enabled = bool_setting("obfuscation_enabled").unwrap_or(config.obfuscation_enabled);
        let obf_required =
            bool_setting("obfuscation_required").unwrap_or(config.obfuscation_required);
        let ed2k_port_eff = get_usize("ed2k_port")
            .map(|v| v.clamp(1, 65535) as u16)
            .unwrap_or(config.ed2k_port);
        let kad_port_eff = get_usize("kad_port")
            .map(|v| v.clamp(1, 65535) as u16)
            .unwrap_or(config.kad_port);
        let udp_enabled = bool_setting("udp_enabled").unwrap_or(true);
        // Modo paranoico (eMuleRust): ranuras de subida reservadas a pares
        // ofuscados con firma SUI verificada.
        let paranoid_mode = bool_setting("paranoid_mode").unwrap_or(false);
        let max_upload_queue = get_usize("max_upload_queue");
        let min_upload_kbs = get_usize("min_upload_kbs");
        let upnp_enabled_boot = bool_setting("upnp_enabled").unwrap_or(config.upnp_enabled);

        let network_policy = NetworkPolicyManager::new(config.network_profile, budgets);
        // Valores acotados para reescribir en el almacén — diferidos porque los
        // cierres anteriores toman prestado `downloads` de forma inmutable.
        let mut clamped_limits: Vec<(&'static str, u64)> = Vec::new();
        // Límites de tasa persistidos (MaxUpload/MaxDownload de eMule, guardados mediante
        // `limits.set`) aplicados en el arranque.
        {
            let get_u64 = |key: &str| {
                downloads
                    .get_setting(key)
                    .ok()
                    .flatten()
                    .and_then(|s| s.parse::<u64>().ok())
            };
            let mut rl = mule_filter::RateLimitsConfig::default();
            let mut has_override = false;
            if let Some(v) = get_u64("rate_limit_download_bps") {
                rl.download_bytes_per_sec = v;
                has_override = true;
            }
            if let Some(v) = get_u64("rate_limit_upload_bps") {
                rl.upload_bytes_per_sec = v;
                has_override = true;
            }
            if let Some(v) = get_u64("rate_limit_conn_per_min") {
                rl.connections_per_minute = v as u32;
                has_override = true;
            }
            if let Some(v) = get_u64("rate_limit_server_req_per_sec") {
                rl.server_requests_per_sec = v as u32;
                has_override = true;
            }
            if let Some(v) = get_u64("rate_limit_kad_pkt_per_sec") {
                rl.kad_packets_per_sec = v as u32;
                has_override = true;
            }
            // LoadPreferences de eMule: un límite persistido no puede
            // superar la capacidad de línea declarada — download ≤ capacidad
            // × 9/10, upload ≤ capacidad × 4/5 (0 = ilimitado, nunca
            // restringido). Misma regla que en `handle_limits_set`.
            let cap_dl = get_u64("capacity_download_kbs").unwrap_or(0);
            let cap_ul = get_u64("capacity_upload_kbs").unwrap_or(0);
            if rl.download_bytes_per_sec > 0 && cap_dl > 0 {
                let cap = cap_dl.saturating_mul(1024).saturating_mul(9) / 10;
                let clamped = rl.download_bytes_per_sec.min(cap);
                if clamped != rl.download_bytes_per_sec {
                    clamped_limits.push(("rate_limit_download_bps", clamped));
                    rl.download_bytes_per_sec = clamped;
                }
            }
            if rl.upload_bytes_per_sec > 0 && cap_ul > 0 {
                let cap = cap_ul.saturating_mul(1024).saturating_mul(4) / 5;
                let clamped = rl.upload_bytes_per_sec.min(cap);
                if clamped != rl.upload_bytes_per_sec {
                    clamped_limits.push(("rate_limit_upload_bps", clamped));
                    rl.upload_bytes_per_sec = clamped;
                }
            }
            // `GetMaxDownloadInBytesPerSec` de eMule: proporción upload/download
            // — el depósito recibe el valor efectivo; el ajuste persistido mantiene el
            // valor *configurado* (la proporción nunca se reescribe).
            let eff_dl = mule_network_policy::emule_ud_ratio(
                rl.download_bytes_per_sec,
                rl.upload_bytes_per_sec,
            );
            if eff_dl != rl.download_bytes_per_sec {
                rl.download_bytes_per_sec = eff_dl;
                has_override = true;
            }
            if has_override {
                network_policy.rate_limiters().apply_config(&rl);
            }
        }
        // Filtro IP persistido (eMule mantiene ipfilter.dat cargado entre
        // reinicios): recarga el archivo y restaura la bandera de activación.
        {
            if let Some(path) = downloads
                .get_setting("ipfilter_path")
                .ok()
                .flatten()
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
            {
                match network_policy
                    .ip_filter()
                    .load_from_file(std::path::Path::new(&path))
                {
                    Ok(report) => {
                        tracing::info!("[IPFILTER] Rechargé depuis {path} ({report:?})")
                    }
                    Err(e) => {
                        tracing::warn!("[IPFILTER] Rechargement impossible ({path}): {e}")
                    }
                }
            }
            // Restaura la bandera en ambos sentidos: sin la rama `false`,
            // un filtro desactivado antes de apagar pasaba silenciosamente a
            // `true` (valor predeterminado de `IpFilter::new`) en el arranque.
            network_policy
                .ip_filter()
                .set_enabled(bool_setting("ipfilter_enabled").unwrap_or(true));
        }
        // Reescritura de los límites restringidos en el arranque: la persistencia refleja
        // el valor efectivamente aplicado (coherencia con `limits.set`,
        // que persiste el valor restringido).
        for (key, v) in clamped_limits {
            if let Err(e) = downloads.set_setting(key, &v.to_string()) {
                tracing::warn!("[LIMITS] Échec persistance {key} clampé: {e}");
            }
        }
        let sequence = Arc::new(AtomicU64::new(1));
        let server_session = ServerSessionManager::new(event_tx.clone(), Arc::clone(&sequence));
        let peer_session = PeerSessionManager::new(event_tx.clone(), Arc::clone(&sequence))
            .with_limits(max_per_dl, max_global);
        // Totales «desde la instalación» persistidos en los ajustes — leídos una
        // vez en el arranque y luego incrementados con los deltas de sesión por el
        // bucle statsTick (restablecidos a cero mediante `stats.resetLifetimeTotals`).
        let u64_setting = |key: &str| {
            downloads
                .get_setting(key)
                .ok()
                .flatten()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        };
        let lifetime_downloaded_bytes =
            Arc::new(AtomicU64::new(u64_setting("lifetime_downloaded_bytes")));
        let lifetime_uploaded_bytes =
            Arc::new(AtomicU64::new(u64_setting("lifetime_uploaded_bytes")));
        let initial_webui_pass_hash = downloads
            .get_setting(SETTING_WEBUI_PASSWORD_HASH)
            .ok()
            .flatten()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let kad_id = downloads
            .get_setting("kad_local_id")
            .ok()
            .flatten()
            .and_then(|s| mule_core::kad::KadId::from_hex(s.trim()))
            .unwrap_or_else(|| {
                let id = mule_core::kad::KadId::random();
                if let Err(e) = downloads.set_setting("kad_local_id", &id.to_hex()) {
                    tracing::warn!("No se pudo persistir kad_local_id: {e}");
                }
                id
            });
        downloads.set_event_channel(event_tx.clone(), Arc::clone(&sequence));
        let dl_arc = Arc::new(Mutex::new(downloads));
        server_session.set_downloads(Arc::clone(&dl_arc));
        let identity_service = crate::services::IdentityService::load_or_generate(&config.data_dir);
        let user_hash = identity_service.user_hash();
        // Entorno de Source Exchange compartido entre todas las sesiones
        // (eMule mantiene estos contadores en CUpDownClient/CPartFile).
        let sx_env = crate::services::peer_session::source_exchange::SxEnv::new(
            network_policy.clone(),
            user_hash,
        );
        let sui_pubkey_der = if identity_service.has_key() {
            Some(identity_service.public_key_bytes().to_vec())
        } else {
            None
        };
        let identity = Arc::new(Mutex::new(identity_service));
        let sui_stats = Arc::new(crate::services::identity::SuiStats::default());
        let mut inbound_listener = crate::services::inbound_listener::InboundListenerService::new(
            Arc::new(network_policy.clone()),
            Arc::clone(&dl_arc),
            user_hash,
            sui_pubkey_der,
            obf_enabled,
            obf_required,
            config.obfuscation_padding_max,
            Arc::clone(&sui_stats),
        );
        inbound_listener.set_identity(Arc::clone(&identity));
        inbound_listener.set_sx_env(sx_env.clone());
        inbound_listener.set_paranoid_mode(paranoid_mode);
        let upload_quota_gate = Arc::new(AtomicBool::new(true));
        inbound_listener
            .upload_manager()
            .set_quota_gate(Arc::clone(&upload_quota_gate));
        inbound_listener.set_peer_session(peer_session.clone());
        inbound_listener.set_server_session(server_session.clone());
        // Sentido inverso: OP_CALLBACKREQUESTED retransmitido por el servidor (un
        // par HighID quiere conectarse a nosotros mientras somos LowID)
        // desencadena una conexión entrante de retorno atendida por el oyente entrante.
        server_session.set_inbound_listener(inbound_listener.clone());
        // IsServerCryptLayerConnection de eMule: intento de ofuscación del
        // enlace inicial (handshake) del servidor según las preferencias efectivas (persistidas).
        server_session.set_obfuscation_preference(
            obf_enabled || obf_required,
            config.obfuscation_padding_max,
        );
        let kad = Arc::new(crate::services::KadService::new(
            kad_id,
            Arc::clone(&dl_arc),
            network_policy.clone(),
            network_policy.ip_filter(),
        ));
        kad.set_ed2k_tcp_port(ed2k_port_eff);
        kad.set_fw_check_identity(user_hash);
        // KADEMLIA_FIREWALLED2_REQ incluye el hash de usuario eD2K (`GetClientHash`
        // de eMule), distinto del KadID aleatorio.
        kad.udp().set_fw_user_hash(user_hash);
        // Avisa `kad.statusChanged` cuando la tabla de enrutamiento evoluciona en
        // segundo plano — el distintivo de la interfaz se actualiza sin abrir la sección.
        kad.set_event_channel(event_tx.clone(), Arc::clone(&sequence));
        // Rol de retransmisión buddy: puerto TCP al que se conecta el cliente amigo, y el
        // byte de opciones al estilo GetMyConnectOptions enviado en KADEMLIA_FINDBUDDY_RES
        // (bit0/1 = ofuscación soportada+solicitada, bit2 = requerida).
        let connect_opts: u8 =
            if obf_enabled { 0x03 } else { 0 } | if obf_required { 0x04 } else { 0 };
        kad.udp().set_buddy_tcp_info(ed2k_port_eff, connect_opts);
        // El HighID asignado por el servidor eD2K proporciona a Kad nuestra IPv4 pública
        // (dirección anunciada en PUBLISH_SOURCE).
        server_session.set_kad(Arc::clone(&kad));
        inbound_listener.set_kad_udp(Arc::clone(kad.udp()));
        inbound_listener.set_kad(Arc::clone(&kad));
        inbound_listener.set_kad_udp_port(kad_port_eff);
        let client_udp = crate::services::ClientUdpService::new();
        client_udp.set_downloads(Arc::clone(&dl_arc));
        client_udp.set_upload_manager(inbound_listener.upload_manager().clone());
        client_udp.set_peer_session(peer_session.clone());
        client_udp.set_buddy_registry(kad.udp().buddy_registry());
        // OP_DIRECTCALLBACKREQ: conexión de retorno mediante el oyente, condicionada
        // al estado del cortafuegos TCP (`CKademlia::IsRunning() &&
        // IsFirewalled()` de eMule — proxy `!is_high_id`).
        client_udp.set_inbound_listener(inbound_listener.clone());
        client_udp.set_server_session(server_session.clone());
        inbound_listener.set_client_udp(client_udp.clone());
        let server_udp = crate::services::server_udp::ServerUdpService::new(network_policy.clone());
        server_udp.set_downloads(Arc::clone(&dl_arc));
        server_udp.set_event_channel(event_tx.clone(), sequence.clone());
        let uss = crate::services::UssService::new();
        uss.set_server_session(server_session.clone());
        uss.set_upload_manager(inbound_listener.upload_manager().clone());
        uss.set_downloads(Arc::clone(&dl_arc));
        peer_session.set_uss(uss.clone());
        inbound_listener
            .upload_manager()
            .set_uss_gate(uss.accept_flag());
        if let Some(q) = max_upload_queue {
            inbound_listener.upload_manager().set_max_queue(q);
        }
        if let Some(min_kbs) = min_upload_kbs {
            inbound_listener
                .upload_manager()
                .set_min_upload_rate((min_kbs as u64).saturating_mul(1024));
        }
        inbound_listener
            .upload_manager()
            .set_event_channel(event_tx.clone(), Arc::clone(&sequence));
        // `MaxSpeed` de eMule para `AcceptNewClient` — lee la tasa efectiva del
        // depósito de subida: límite manual (`limits.set`) o valor medido
        // gobernado por USS cuando está activo.
        {
            let rl = network_policy.rate_limiters();
            inbound_listener
                .upload_manager()
                .set_upload_speed_probe(move || rl.upload_bucket.rate_per_sec());
        }
        // Pulso de la cola de subida — `CUploadQueue::Process()` de eMule: purga
        // de entradas muertas, rotación de sesiones llenas, promoción de los
        // mejores clientes. Se ejecuta con independencia de los abonados WS (la cola
        // debe funcionar en modo desatendido / headless).
        {
            let upload_mgr = inbound_listener.upload_manager().clone();
            let dl_arc = Arc::clone(&dl_arc);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut tick_count = 0u64;
                loop {
                    interval.tick().await;
                    upload_mgr.process_tick().await;
                    // Instantánea de `sharedfiles` para la purga de «archivo ya no
                    // compartido» de `FindBestClientInQueue` — actualizada con
                    // cadencia reducida (los archivos compartidos rara vez cambian).
                    tick_count += 1;
                    if tick_count.is_multiple_of(SHARED_HASH_REFRESH_TICKS) {
                        let hashes = dl_arc.lock().await.list_shared_hashes();
                        upload_mgr.set_shared_hashes(hashes);
                    }
                }
            });
        }
        // Ranuras de amigos: carga los amigos persistidos en el gestor de subidas
        // (friends.met de eMule → ranuras dedicadas para claves `uh:<userhash>`).
        {
            if let Ok(dl) = dl_arc.try_lock() {
                let friends = dl.list_friends().unwrap_or_default();
                inbound_listener.upload_manager().set_friends(
                    friends
                        .iter()
                        .map(|f| format!("uh:{}", f.user_hash))
                        .collect(),
                );
                // Totales subidos por archivo persistidos (`share_upload_stats`)
                // — el UploadManager los incrementa a continuación en memoria.
                let totals = dl.list_share_uploaded_bytes().unwrap_or_default();
                inbound_listener
                    .upload_manager()
                    .set_file_upload_totals(totals);
            }
        }
        let kad_udp = Arc::clone(kad.udp());
        let client_udp_for_kad = client_udp.clone();
        let kad_udp_dl = Arc::clone(&dl_arc);
        tokio::spawn(async move {
            kad_udp.set_client_udp(client_udp_for_kad).await;
            kad_udp.set_downloads(kad_udp_dl).await;
        });

        // Pulso periódico de estadísticas agregadas (event.statsTick, tema `stats`).
        // Se omite por completo mientras no haya clientes WebSocket suscritos.
        {
            let stats_downloads = Arc::clone(&dl_arc);
            let stats_uploads = inbound_listener.upload_manager().clone();
            let stats_peers = peer_session.clone();
            let stats_server = server_session.clone();
            let stats_kad = Arc::clone(&kad);
            let stats_tx = event_tx.clone();
            let stats_seq = Arc::clone(&sequence);
            let stats_lifetime_dl = Arc::clone(&lifetime_downloaded_bytes);
            let stats_lifetime_ul = Arc::clone(&lifetime_uploaded_bytes);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut last_session_dl = 0u64;
                let mut last_session_ul = 0u64;
                // Totales por archivo ya persistidos — el delta escrito en
                // cada pulso solo cubre los bytes enviados desde entonces.
                let mut last_file_totals = stats_uploads.per_file_uploaded_bytes();
                loop {
                    interval.tick().await;
                    if stats_tx.receiver_count() == 0 {
                        continue;
                    }
                    let session_uploaded_bytes = stats_uploads.total_uploaded_bytes();
                    let (downloads, session_downloaded_bytes) = {
                        let mut dl = stats_downloads.lock().await;
                        // Acumulado «desde la instalación»: sumamos el delta
                        // de sesión transcurrido desde el último pulso (solo cuentan
                        // los deltas positivos — el descarte de partes
                        // corruptas no resta nada al total) y luego se
                        // persiste; de lo contrario, una caída perdería el delta actual.
                        let session_dl = dl.session_downloaded_bytes();
                        let d_dl = session_dl.saturating_sub(last_session_dl);
                        let d_ul = session_uploaded_bytes.saturating_sub(last_session_ul);
                        if d_dl > 0 || d_ul > 0 {
                            let lt_dl = stats_lifetime_dl.fetch_add(d_dl, Ordering::SeqCst) + d_dl;
                            let lt_ul = stats_lifetime_ul.fetch_add(d_ul, Ordering::SeqCst) + d_ul;
                            let _ = dl.set_setting("lifetime_downloaded_bytes", &lt_dl.to_string());
                            let _ = dl.set_setting("lifetime_uploaded_bytes", &lt_ul.to_string());
                        }
                        last_session_dl = session_dl;
                        last_session_ul = session_uploaded_bytes;
                        // Persiste los deltas de subida por archivo en
                        // `share_upload_stats` (misma cadencia que los acumulados
                        // históricos — una caída solo pierde un pulso).
                        let file_totals = stats_uploads.per_file_uploaded_bytes();
                        for (hash, total) in &file_totals {
                            let prev = last_file_totals.get(hash.as_str()).copied().unwrap_or(0);
                            let d = total.saturating_sub(prev);
                            if d > 0 {
                                let _ = dl.add_share_uploaded_bytes(hash, d);
                            }
                        }
                        last_file_totals = file_totals;
                        (dl.list_downloads(), session_dl)
                    };
                    let download_bps: u64 = downloads.iter().map(|d| d.download_rate_bps).sum();
                    let downloads_active = downloads
                        .iter()
                        .filter(|d| d.status == "downloading")
                        .count();
                    let upload_bps: u64 = stats_uploads
                        .active_snapshots()
                        .await
                        .iter()
                        .map(|u| u.upload_rate_bps)
                        .sum();
                    let peers_connected = stats_peers.swarm_snapshot(None).await.active_peer_count;
                    let server_connected = matches!(
                        stats_server.snapshot().await.status,
                        crate::services::server_session::ServerConnectionStatus::Connected
                    );
                    let kad_status = stats_kad.get_status().await;
                    DaemonEvent::StatsTick {
                        sequence: stats_seq.fetch_add(1, Ordering::SeqCst),
                        stats: crate::services::events::StatsTickSnapshot {
                            download_bps,
                            upload_bps,
                            downloads_active,
                            peers_connected,
                            server_connected,
                            kad_contacts: kad_status.total_contacts,
                            kad_udp_bound: kad_status.udp_bind_address.is_some(),
                            session_downloaded_bytes,
                            session_uploaded_bytes,
                            lifetime_downloaded_bytes: stats_lifetime_dl.load(Ordering::SeqCst),
                            lifetime_uploaded_bytes: stats_lifetime_ul.load(Ordering::SeqCst),
                        },
                    }
                    .send(&stats_tx);
                    // Envío periódico de instantáneas de subida con la misma
                    // cadencia: refresca las tasas medidas por ventana y
                    // las hace decrecer hacia 0 cuando un cliente deja de
                    // pedir bloques (de lo contrario, la interfaz quedaría fija en el
                    // último valor entre dos llamadas a `update_progress`).
                    if stats_uploads.active_len() > 0 || stats_uploads.waiting_len().await > 0 {
                        stats_uploads.emit_uploads_updated().await;
                    }
                }
            });
        }
        peer_session.set_kad(Arc::clone(&kad));
        peer_session.set_rate_limiters(network_policy.rate_limiters());
        peer_session.set_identity(Arc::clone(&identity));
        peer_session.set_sui_stats(Arc::clone(&sui_stats));
        let upnp = Arc::new(crate::services::UpnpService::new(
            upnp_enabled_boot,
            ed2k_port_eff,
            kad_port_eff,
        ));
        let updater = Arc::new(crate::services::updater::UpdaterService::new(
            config.update_manifest_url.clone(),
            crate::rpc::methods::DAEMON_VERSION.to_string(),
        ));
        Self {
            config,
            downloads: dl_arc,
            network_policy,
            server_session,
            peer_session,
            sx_env,
            inbound_listener,
            identity,
            sui_stats,
            kad,
            client_udp,
            server_udp,
            uss,
            upnp,
            sequence,
            event_tx,
            instance_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            abort_handles: Arc::new(Mutex::new(Vec::new())),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            ed2k_port_eff: Arc::new(std::sync::atomic::AtomicU16::new(ed2k_port_eff)),
            kad_port_eff: Arc::new(std::sync::atomic::AtomicU16::new(kad_port_eff)),
            udp_enabled: Arc::new(AtomicBool::new(udp_enabled)),
            upload_quota_gate,
            lifetime_downloaded_bytes,
            lifetime_uploaded_bytes,
            #[cfg(feature = "lab-tools")]
            active_lab_peer: Arc::new(Mutex::new(None)),
            log_control: None,
            torrent: Arc::new(tokio::sync::OnceCell::new()),
            pseudonymizer: Arc::new(mule_filter::SessionPseudonymizer::new()),
            nas_transfer_semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
            active_nas_transfer_id: Arc::new(Mutex::new(None)),
            media_metadata: Arc::new(crate::services::MediaMetadataService::new()),
            webui_password_hash: Arc::new(std::sync::RwLock::new(initial_webui_pass_hash)),
            updater,
            quic_transport: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Enlaza e inicia el transporte QUIC P2P autenticado por Ed25519 sobre UDP si la
    /// política de red y `udp_enabled` lo permiten.
    pub async fn start_quic_transport(&self) -> Option<SocketAddr> {
        if let Some(existing) = self.quic_transport.read().await.as_ref() {
            return Some(existing.local_addr());
        }
        if !self.udp_enabled()
            || self.network_policy.state() == mule_network_policy::NetworkState::Disabled
        {
            return None;
        }
        let modern_key = self.identity.lock().await.modern_key().cloned()?;
        let policy_arc = Arc::new(self.network_policy.clone());
        let candidates: &[SocketAddr] = if self.network_policy.profile().is_loopback_only() {
            &[SocketAddr::from(([127, 0, 0, 1], 0))]
        } else {
            &[
                SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, 0)),
                SocketAddr::from(([0, 0, 0, 0], 0)),
            ]
        };
        for &bind_addr in candidates {
            match crate::services::quic_transport::QuicPeerTransport::bind(
                bind_addr,
                &modern_key,
                Arc::clone(&policy_arc),
            ) {
                Ok(qt) => {
                    let mut slot = self.quic_transport.write().await;
                    if let Some(existing) = slot.as_ref() {
                        qt.close();
                        return Some(existing.local_addr());
                    }
                    let addr = qt.local_addr();
                    self.inbound_listener.set_quic_port(addr.port());
                    *slot = Some(Arc::new(qt));
                    tracing::info!("[QUIC] Transporte P2P Ed25519 enlazado en {addr}");
                    return Some(addr);
                }
                Err(e) => {
                    tracing::debug!("[QUIC] No se pudo enlazar QUIC en {bind_addr}: {e}");
                }
            }
        }
        tracing::warn!("[QUIC] No se pudo enlazar el transporte QUIC P2P en ningún candidato");
        None
    }

    /// Detiene el transporte QUIC P2P y libera su permiso de escucha y el puerto anunciado.
    pub async fn stop_quic_transport(&self) {
        let prev = self.quic_transport.write().await.take();
        if let Some(qt) = prev {
            qt.close();
        }
        self.inbound_listener.set_quic_port(0);
    }

    /// Identificador de la descarga que ostenta activamente la ranura de copia al NAS.
    pub async fn current_nas_transfer(&self) -> Option<Uuid> {
        *self.active_nas_transfer_id.lock().await
    }

    /// Registra un manejador de tarea en segundo plano para que pueda abortarse limpiamente al apagar.
    /// Directiva efectiva de ofuscación (cifrado del protocolo): los ajustes persistidos
    /// `obfuscation_enabled`/`obfuscation_required` sobrescriben los valores
    /// iniciales de `DaemonConfig` (Preferencias de eMule → Seguridad).
    pub async fn effective_obfuscation(&self) -> (bool, bool) {
        let dl = self.downloads.lock().await;
        let get = |key: &str, default: bool| {
            dl.get_setting(key)
                .ok()
                .flatten()
                .map(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "on"))
                .unwrap_or(default)
        };
        let enabled = get("obfuscation_enabled", self.config.obfuscation_enabled);
        let required = get("obfuscation_required", self.config.obfuscation_required);
        (enabled, required)
    }

    /// Puerto TCP efectivo de eD2K (`ed2k_port` persistido o configuración de inicio).
    pub fn ed2k_port(&self) -> u16 {
        self.ed2k_port_eff.load(Ordering::Relaxed)
    }

    /// Directorio efectivo de descargas (ajuste persistido `default_download_directory` o valor de inicio).
    pub async fn effective_download_dir(&self) -> PathBuf {
        let dl = self.downloads.lock().await;
        dl.get_setting("default_download_directory")
            .ok()
            .flatten()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.config.default_download_dir.clone())
    }

    /// Devuelve `true` si la ruta coincide con un directorio local predeterminado (`.../media/<subdir>`).
    pub fn is_default_local_media_subdir(&self, candidate: &Path, default_dl_dir: &Path, default_subdir: &str) -> bool {
        let local_1 = default_dl_dir.join("media").join(default_subdir);
        let local_2 = self
            .config
            .default_download_dir
            .join("media")
            .join(default_subdir);
        let legacy = PathBuf::from("nas").join(default_subdir);
        candidate == local_1 || candidate == local_2 || candidate == legacy
    }

    /// Devuelve la raíz efectiva del NAS/almacenamiento multimedia (`nas_root_dir` persistido,
    /// variable de entorno `EMULERUST_NAS_PATH` o subcarpeta local `downloads/media`).
    pub async fn effective_nas_root_dir(&self) -> PathBuf {
        let (nas_setting, dl_dir) = {
            let dl = self.downloads.lock().await;
            let nas = dl
                .get_setting("nas_root_dir")
                .ok()
                .flatten()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let dldir = dl
                .get_setting("default_download_directory")
                .ok()
                .flatten()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            (nas, dldir)
        };
        if let Some(nas) = nas_setting {
            return PathBuf::from(nas);
        }
        let default_local_media = self.config.default_download_dir.join("media");
        if self.config.nas_root_dir != default_local_media
            && !self.config.nas_root_dir.as_os_str().is_empty()
        {
            return self.config.nas_root_dir.clone();
        }
        dl_dir
            .unwrap_or_else(|| self.config.default_download_dir.clone())
            .join("media")
    }

    /// Indica si está activada la protección estricta de montaje NFS/red (`nas_require_nfs`).
    pub async fn nas_require_network_mount(&self) -> bool {
        self.setting_bool("nas_require_nfs", false).await
    }

    async fn resolve_effective_media_dir(
        &self,
        setting_key: &str,
        default_subdir: &str,
        config_dir: &Path,
        is_movies: bool,
    ) -> PathBuf {
        let (explicit_dir, explicit_nas, dl_dir) = {
            let dl = self.downloads.lock().await;
            let dir = dl
                .get_setting(setting_key)
                .ok()
                .flatten()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            let nas = dl
                .get_setting("nas_root_dir")
                .ok()
                .flatten()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            let dldir = dl
                .get_setting("default_download_directory")
                .ok()
                .flatten()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| self.config.default_download_dir.clone());
            (dir, nas, dldir)
        };

        let default_local_media = self.config.default_download_dir.join("media");
        let nas_root = explicit_nas.or_else(|| {
            (self.config.nas_root_dir != default_local_media
                && !self.config.nas_root_dir.as_os_str().is_empty())
            .then(|| self.config.nas_root_dir.clone())
        });

        if let Some(dir) = explicit_dir {
            let shadowed_by_nas =
                nas_root.is_some() && self.is_default_local_media_subdir(&dir, &dl_dir, default_subdir);
            if !shadowed_by_nas {
                return dir;
            }
        }

        if let Some(nas) = nas_root {
            let nas_clone = nas.clone();
            let resolved = tokio::task::spawn_blocking(move || {
                let status = crate::services::nfs_mount::inspect_directory_status(&nas_clone, true);
                if is_movies {
                    status.resolved_movies_dir
                } else {
                    status.resolved_series_dir
                }
            })
            .await
            .ok()
            .map(PathBuf::from);
            return resolved.unwrap_or_else(|| nas.join(default_subdir));
        }

        if !config_dir.as_os_str().is_empty()
            && !self.is_default_local_media_subdir(config_dir, &dl_dir, default_subdir)
        {
            return config_dir.to_path_buf();
        }

        dl_dir.join("media").join(default_subdir)
    }

    /// Directorio efectivo de películas (ajuste persistido `movies_dir`, autodetección bajo `nas_root_dir` o valor inicial).
    pub async fn effective_movies_dir(&self) -> PathBuf {
        self.resolve_effective_media_dir("movies_dir", "Movies", &self.config.movies_dir, true)
            .await
    }

    /// Directorio efectivo de series (ajuste persistido `series_dir`, autodetección bajo `nas_root_dir` o valor inicial).
    pub async fn effective_series_dir(&self) -> PathBuf {
        self.resolve_effective_media_dir("series_dir", "Series", &self.config.series_dir, false)
            .await
    }

    /// Puerto UDP efectivo de Kad (`kad_port` persistido o configuración de inicio).
    pub fn kad_port(&self) -> u16 {
        self.kad_port_eff.load(Ordering::Relaxed)
    }

    /// Devuelve el motor BitTorrent inicializado de forma diferida (lazy).
    pub async fn torrent_engine(&self) -> Result<Arc<mule_torrent::TorrentEngine>, String> {
        let state = self.network_policy.state();
        if matches!(
            state,
            mule_network_policy::NetworkState::Disabled
                | mule_network_policy::NetworkState::ShuttingDown
        ) {
            return Err("BitTorrent engine disabled by network policy kill switch".to_string());
        }
        let default_dir = self.effective_download_dir().await;
        let data_dir = self.config.data_dir.clone();
        let enable_upnp = self.upnp.snapshot().await.enabled;
        let policy = Arc::new(self.network_policy.clone());
        self.torrent
            .get_or_try_init(|| async move {
                let mut cfg = mule_torrent::TorrentPolicyConfig::new(default_dir, data_dir);
                cfg.enable_upnp = enable_upnp;
                mule_torrent::TorrentEngine::new_with_policy(policy, cfg)
                    .await
                    .map(Arc::new)
                    .map_err(|e| e.to_string())
            })
            .await
            .cloned()
    }

    /// Indica si los servicios UDP (Kad UDP, UDP de servidor, UDP de cliente) están habilitados.
    pub fn udp_enabled(&self) -> bool {
        self.udp_enabled.load(Ordering::Relaxed)
    }

    /// Actualiza los puertos efectivos o la bandera de UDP (invocado por `settings.set`).
    pub fn set_effective_ports(&self, ed2k: u16, kad: u16) {
        self.ed2k_port_eff.store(ed2k, Ordering::Relaxed);
        self.kad_port_eff.store(kad, Ordering::Relaxed);
    }

    pub fn set_udp_enabled(&self, enabled: bool) {
        self.udp_enabled.store(enabled, Ordering::Relaxed);
    }

    /// Calcula el hash determinista con sal de la contraseña de la WebUI mediante SHA-256.
    pub fn hash_webui_password(password: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(WEBUI_PASSWORD_SALT_PREFIX.as_bytes());
        hasher.update(password.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Indica si el usuario ha configurado una contraseña de acceso para la WebUI.
    pub fn has_webui_password(&self) -> bool {
        self.webui_password_hash
            .read()
            .map(|g| g.is_some())
            .unwrap_or(false)
    }

    /// Verifica si `candidate` coincide con la contraseña configurada (o devuelve `true` si no hay contraseña).
    pub fn verify_webui_password(&self, candidate: &str) -> bool {
        let Ok(guard) = self.webui_password_hash.read() else {
            return false;
        };
        match guard.as_deref() {
            Some(expected) => Self::hash_webui_password(candidate.trim()) == expected,
            None => true,
        }
    }

    /// Actualiza en caliente el hash de la contraseña de la WebUI en memoria.
    pub fn set_webui_password_hash(&self, hash: Option<String>) {
        if let Ok(mut guard) = self.webui_password_hash.write() {
            *guard = hash.filter(|s| !s.trim().is_empty());
        }
    }

    /// Alias (nickname) efectivo anunciado en los hellos de eD2K o en el inicio de sesión del servidor
    /// (preferencia `nick` de eMule). Predeterminado: "eMuleRust".
    pub async fn current_nickname(&self) -> String {
        self.downloads
            .lock()
            .await
            .get_setting("nickname")
            .ok()
            .flatten()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "eMuleRust".to_string())
    }

    pub async fn register_task(&self, handle: tokio::task::AbortHandle) {
        let mut handles = self.abort_handles.lock().await;
        handles.push(handle);
    }

    /// Cancela todas las tareas activas en segundo plano asociadas a este estado del daemon.
    pub async fn abort_all_tasks(&self) {
        let mut handles = self.abort_handles.lock().await;
        for handle in handles.drain(..) {
            handle.abort();
        }
        #[cfg(feature = "lab-tools")]
        {
            let mut peer = self.active_lab_peer.lock().await;
            if let Some((_, h)) = peer.take() {
                h.abort();
            }
        }
    }

    /// Emite un evento secuenciado de actualización de descarga a todos los suscriptores activos de WebSocket.
    pub fn broadcast_download_update(&self, mut snapshot: DownloadSnapshot) {
        // Adjunta los rangos de bloques en vuelo del enjambre (segmentos amarillos "pendientes"
        // de eMule) — el servicio de descargas no puede ver las sesiones de pares por sí mismo.
        snapshot.pending_ranges = self.peer_session.try_pending_ranges(&snapshot.ed2k_hash);
        if snapshot.status == "completing" {
            if let Ok(guard) = self.active_nas_transfer_id.try_lock() {
                snapshot.nas_copy_active = Some(*guard == Some(snapshot.id));
            }
        }
        self.emit_event(|sequence| DaemonEvent::DownloadUpdated {
            sequence,
            download: snapshot,
        });
    }

    /// Emite cualquier evento secuenciado del daemon. Punto de entrada único para
    /// manejadores o servicios que no son propietarios del contador de secuencia.
    pub fn emit_event(&self, make: impl FnOnce(u64) -> DaemonEvent) {
        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);
        make(seq).send(&self.event_tx);
    }

    /// Dirección «segura» para los registros: loopback en claro (inofensivo,
    /// práctico en laboratorio), direcciones externas seudonimizadas
    /// (`peer-NN:port`). Los archivos de registro persistentes nunca deben
    /// contener IPs públicas en bruto.
    pub fn log_peer_addr(&self, addr: SocketAddr) -> String {
        if addr.ip().is_loopback() {
            addr.to_string()
        } else {
            self.pseudonymizer.pseudonymize_peer(addr)
        }
    }

    /// Especificación `EnvFilter` actualmente vigente (`settings.get` →
    /// `logLevel`). Predeterminada `info` si el control no está instalado.
    pub fn current_log_level(&self) -> String {
        self.log_control
            .as_ref()
            .map(LogControl::current)
            .unwrap_or_else(|| "info".to_string())
    }

    /// Lee un ajuste booleano persistido (recurre a `default` cuando está ausente).
    pub async fn setting_bool(&self, key: &str, default: bool) -> bool {
        self.downloads
            .lock()
            .await
            .get_setting(key)
            .ok()
            .flatten()
            .map(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "on"))
            .unwrap_or(default)
    }

    /// Observador de autoconexión / reconexión automática (`Autoconnect`,
    /// `Reconnect`, `AutoDeadServer` de eMule). Cada 15 s: si no hay ninguna sesión de servidor
    /// activa y se dispone de un objetivo, se reconecta a través de la ruta
    /// habitual de RPC (se aplican la directiva de red, el interruptor de corte y las comprobaciones de servidores deshabilitados).
    /// `server_autoremove_dead` elimina un objetivo tras `DeadServerRetry`
    /// fallos consecutivos (valor predeterminado de eMule 0.72a = 1). Invocado una vez tras
    /// la construcción.
    pub fn spawn_server_watcher(&self) {
        // Los campos de `AppState` son todos `Arc` compartidos — una copia sigue apuntando a
        // los mismos servicios/gestores de sesión que el estado RPC.
        let state = Arc::new(self.clone());
        let handle = tokio::spawn(async move {
            let mut failures: u32 = 0;
            let mut autoconnect_done = false;
            let mut serverlist_updated = false;
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(15));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                // `AutoUpdateServerList` de eMule: actualiza la lista de servidores una
                // vez por sesión desde un archivo local `.met` configurado
                // (ajuste `server_autoupdate_path` — sin descarga remota; el
                // daemon deliberadamente no dispone de cliente HTTP saliente).
                if !serverlist_updated {
                    serverlist_updated = true;
                    if state.setting_bool("server_autoupdate_list", false).await {
                        let path = {
                            let dl = state.downloads.lock().await;
                            dl.get_setting("server_autoupdate_path")
                                .ok()
                                .flatten()
                                .unwrap_or_default()
                        };
                        if !path.trim().is_empty() {
                            let params = serde_json::json!({ "filePath": path.trim() });
                            match crate::rpc::handlers::servers::handle_servers_import_met(
                                &state,
                                Some(params),
                            )
                            .await
                            {
                                Ok(v) => tracing::info!("[SERVERLIST] auto-update: {v}"),
                                Err(e) => {
                                    tracing::warn!("[SERVERLIST] auto-update failed: {e}")
                                }
                            }
                        }
                    }
                }
                if state.server_session.is_active().await {
                    failures = 0;
                    continue;
                }
                // Interruptor de corte (kill switch): permanecer en silencio hasta que la red se vuelva a habilitar.
                if state.network_policy.state() == mule_network_policy::NetworkState::Disabled {
                    continue;
                }
                let reconnect = state.setting_bool("server_reconnect", true).await;
                let target = if reconnect {
                    state.server_session.last_target()
                } else {
                    None
                };
                let target = match target {
                    Some(t) => Some(t),
                    None if !autoconnect_done
                        && state.setting_bool("server_autoconnect", false).await =>
                    {
                        autoconnect_done = true;
                        let servers = {
                            let dl = state.downloads.lock().await;
                            dl.list_servers().unwrap_or_default()
                        };
                        servers
                            .iter()
                            .filter(|s| {
                                s.filter_status != mule_core::model::ServerFilterStatus::Disabled
                            })
                            .max_by_key(|s| s.preference)
                            .or_else(|| servers.first())
                            .and_then(|s| s.address.parse::<SocketAddr>().ok())
                            .map(|a| crate::services::server_session::ServerTarget {
                                addr: a,
                                name: None,
                            })
                    }
                    None => continue,
                };
                let Some(t) = target else { continue };
                let addr = t.addr;
                let params = serde_json::json!({ "address": addr.to_string() });
                match crate::rpc::handlers::servers::handle_servers_connect(&state, Some(params))
                    .await
                {
                    Ok(_) => {
                        failures = 0;
                    }
                    Err(e) => {
                        failures += 1;
                        tracing::debug!("[SERVER_RECONNECT] attempt {failures} failed: {e}");
                        // Valor predeterminado de eMule `DeadServerRetry` = 1.
                        if failures >= 1 && state.setting_bool("server_autoremove_dead", true).await
                        {
                            let removed = {
                                let mut dl = state.downloads.lock().await;
                                dl.delete_server(&addr.to_string()).unwrap_or(false)
                            };
                            if removed {
                                tracing::info!(
                                    "[SERVER_RECONNECT] auto-removed dead server {addr} after {failures} failures"
                                );
                            }
                            state.server_session.clear_last_target();
                            failures = 0;
                        }
                    }
                }
            }
        });
        let abort = handle.abort_handle();
        self.abort_handles
            .try_lock()
            .map(|mut h| h.push(abort))
            .ok();
    }

    /// Regenera la identidad criptográfica local (clave RSA SUI, user_hash
    /// eD2K, KadID) y la propaga en caliente a todos los consumidores:
    /// oyente entrante (hellos + desafíos SUI), identidad de cortafuegos Kad,
    /// tabla de enrutamiento Kad. Las sesiones ya establecidas conservan su
    /// identidad de origen; las nuevas emplean la nueva.
    /// Invocada por `identity.rotate` y por la rotación periódica
    /// `identityRotationHours`. Los créditos asociados a la clave anterior dejan de
    /// aplicarse (comportamiento intencionado — nueva identidad).
    pub async fn rotate_identity(
        &self,
    ) -> Result<crate::services::identity::IdentityStatus, String> {
        let new_id = crate::services::IdentityService::rotate_key(&self.config.data_dir)
            .map_err(|e| format!("Failed to rotate identity key: {e}"))?;
        self.propagate_identity(new_id, "Rotation").await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        if let Err(e) = self
            .downloads
            .lock()
            .await
            .set_setting("identity_last_rotation_unix", &now.to_string())
        {
            tracing::warn!("[IDENTITY] Échec persistance last_rotation: {e}");
        }
        Ok(self.identity.lock().await.status())
    }

    /// Instala `new_id` y la propaga en caliente a los consumidores
    /// de identidad: oyente entrante (hellos + desafíos SUI), identidad
    /// de cortafuegos Kad, tabla de enrutamiento Kad (KadID derivado del user_hash).
    /// Las sesiones ya establecidas conservan su identidad de origen.
    pub(crate) async fn propagate_identity(&self, new_id: crate::services::IdentityService, reason: &str) {
        let user_hash = new_id.user_hash();
        let pubkey = new_id.has_key().then(|| new_id.public_key_bytes().to_vec());
        {
            let mut guard = self.identity.lock().await;
            *guard = new_id;
        }
        self.inbound_listener
            .set_identity_material(user_hash, pubkey);
        self.kad.set_fw_check_identity(user_hash);
        self.kad.udp().set_fw_user_hash(user_hash);
        let new_kad_id = self.kad.rotate_local_id().await;
        if let Err(e) = self
            .downloads
            .lock()
            .await
            .set_setting("kad_local_id", &new_kad_id.to_hex())
        {
            tracing::warn!("[IDENTITY] No se pudo persistir el KadID rotado: {e}");
        }
        tracing::info!(
            "[IDENTITY] {reason} effectuée — nouveau user_hash {} / KadID {}",
            hex::encode(user_hash),
            new_kad_id
        );
    }

    /// Importa una identidad de eMule (`cryptkey.dat` + `preferences.dat`)
    /// y luego la propaga en caliente como `rotate_identity` — los créditos
    /// asociados a la identidad de eMule pasan a ser válidos inmediatamente.
    pub async fn import_emule_identity(
        &self,
        cryptkey_src: Option<&std::path::Path>,
        preferences_src: Option<&std::path::Path>,
        replace: bool,
    ) -> Result<crate::services::identity::IdentityImportReport, String> {
        let mut report = crate::services::IdentityService::import_emule_identity(
            &self.config.data_dir,
            cryptkey_src,
            preferences_src,
            replace,
        )
        .map_err(|e| e.to_string())?;
        let new_id = crate::services::IdentityService::load_or_generate(&self.config.data_dir);
        self.propagate_identity(new_id, "Import").await;
        // El hash activo tras la recarga (importado o derivado de la clave).
        report.userhash = Some(hex::encode(self.identity.lock().await.user_hash()));
        Ok(report)
    }

    /// Rotación periódica de la identidad (`identityRotationHours`, predeterminado 0 =
    /// desactivado). Comprueba cada 60 s si ha transcurrido el intervalo desde
    /// `identity_last_rotation_unix` (persistido — sobrevive a reinicios).
    /// Invocar una vez tras la construcción.
    pub fn spawn_identity_rotator(&self) {
        let state = Arc::new(self.clone());
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let (hours, last) = {
                    let dl = state.downloads.lock().await;
                    let h = dl
                        .get_setting("identity_rotation_hours")
                        .ok()
                        .flatten()
                        .and_then(|s| s.trim().parse::<u64>().ok())
                        .unwrap_or(0);
                    let l = dl
                        .get_setting("identity_last_rotation_unix")
                        .ok()
                        .flatten()
                        .and_then(|s| s.trim().parse::<i64>().ok())
                        .unwrap_or(0);
                    (h, l)
                };
                if hours == 0 {
                    continue;
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                if last <= 0 {
                    // Primera activación: el arranque cuenta como punto
                    // de referencia (evita una rotación inmediata en el inicio).
                    let _ = state
                        .downloads
                        .lock()
                        .await
                        .set_setting("identity_last_rotation_unix", &now.to_string());
                    continue;
                }
                if now.saturating_sub(last) < (hours * 3600) as i64 {
                    continue;
                }
                match state.rotate_identity().await {
                    Ok(_) => tracing::info!(
                        "[IDENTITY] Rotation périodique effectuée (intervalle {} h)",
                        hours
                    ),
                    Err(e) => tracing::warn!("[IDENTITY] Rotation périodique échouée: {e}"),
                }
            }
        });
        let abort = handle.abort_handle();
        self.abort_handles
            .try_lock()
            .map(|mut h| h.push(abort))
            .ok();
    }

    /// Cuota diaria de subida (`dailyUploadQuotaGb`, predeterminado 0 = ilimitado).
    /// Cada 30 s: acumula el contador de `UploadManager` en el contador
    /// persistido `upload_quota_used_bytes`, restablecido a cero en cada nuevo día
    /// UTC (`upload_quota_day` = número de día Unix). Cuota alcanzada → la compuerta
    /// corta las nuevas solicitudes de ranura; cuota ampliada/restablecida → compuerta reabierta.
    /// Invocar una vez tras la construcción.
    pub fn spawn_quota_watcher(&self) {
        let state = Arc::new(self.clone());
        let gate = Arc::clone(&self.upload_quota_gate);
        let uploads = self.inbound_listener.upload_manager().clone();
        let handle = tokio::spawn(async move {
            let mut last_total = uploads.total_uploaded_bytes();
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let total = uploads.total_uploaded_bytes();
                let delta = total.saturating_sub(last_total);
                last_total = total;
                let now_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                let today = now_secs / 86_400;
                let mut dl = state.downloads.lock().await;
                let get_u64 = |key: &str| {
                    dl.get_setting(key)
                        .ok()
                        .flatten()
                        .and_then(|s| s.trim().parse::<u64>().ok())
                };
                let quota_gb = get_u64("daily_upload_quota_gb").unwrap_or(0);
                let stored_day = get_u64("upload_quota_day").unwrap_or(0) as i64;
                let mut used = if stored_day == today {
                    get_u64("upload_quota_used_bytes").unwrap_or(0)
                } else {
                    0
                };
                if delta > 0 || stored_day != today {
                    used = used.saturating_add(delta);
                    if let Err(e) = dl.set_setting("upload_quota_day", &today.to_string()) {
                        tracing::warn!("[QUOTA] Échec persistance upload_quota_day: {e}");
                    }
                    if let Err(e) = dl.set_setting("upload_quota_used_bytes", &used.to_string()) {
                        tracing::warn!("[QUOTA] Échec persistance upload_quota_used_bytes: {e}");
                    }
                }
                let exhausted = quota_gb > 0 && used >= quota_gb.saturating_mul(1 << 30);
                let prev = gate.swap(!exhausted, Ordering::SeqCst);
                if prev && exhausted {
                    tracing::warn!(
                        "[QUOTA] Quota journalier d'upload atteint ({} Gio) — nouveaux slots refusés",
                        quota_gb
                    );
                } else if !prev && !exhausted {
                    tracing::info!(
                        "[QUOTA] Quota d'upload réouvert (compteur remis à zéro ou quota relevé)"
                    );
                }
            }
        });
        let abort = handle.abort_handle();
        self.abort_handles
            .try_lock()
            .map(|mut h| h.push(abort))
            .ok();
    }
}

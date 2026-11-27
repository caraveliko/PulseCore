// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

//! Motor BitTorrent integrado acotado, gobernado por política de red, `ipfilter`, uTP y Dual-Stack.

pub mod blocklist;
pub mod config;
pub mod dht_crawler;
pub mod hybrid_io;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use librqbit::api::TorrentIdOrHash;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerMode, ListenerOptions, Session,
    SessionOptions, SessionPersistenceConfig,
};
use mule_network_policy::content_policy::{
    HEADER_INSPECT_BYTES, HEADER_INSPECT_MIN_BYTES, select_torrent_video_files,
    validate_video_header,
};
use mule_network_policy::validator::validate_connect_address;
use mule_network_policy::{NetworkPolicyManager, NetworkState};
use serde::{Deserialize, Serialize};

pub use config::{
    TorrentCapabilitiesSnapshot, TorrentError, TorrentPolicyConfig, DEFAULT_TORRENT_CLIENT_NAME,
    DEFAULT_TORRENT_PEER_LIMIT,
};
pub use dht_crawler::{
    build_minimal_torrent_bytes, derive_ed2k_rendezvous_id, has_media_release_signal,
    is_blocked_abusive_or_adult, is_clean_catalog_video, DhtHarvester, DhtHarvesterStats,
    DhtVideoFile, DiscoveredVideoTorrent, MIN_DHT_MEDIA_VIDEO_BYTES,
};
pub use hybrid_io::{
    compute_verified_file_ranges, CompletedTorrentVideo, HybridTorrentLaunchResult,
    VerifiedTorrentFileState,
};

/// Alias compatible con versiones previas del límite por defecto de pares por torrent.
pub const TORRENT_PEER_LIMIT: usize = DEFAULT_TORRENT_PEER_LIMIT;

/// Carpeta de estado `fastresume` bajo `blocklist_dir`.
const FASTRESUME_SUBDIR: &str = "bt_session";

/// Intervalo de muestreo del vigilante de cabeceras de vídeo para torrents activos.
const VIDEO_HEADER_WATCHDOG_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Número máximo de ciclos de comprobación antes de detener el vigilante de un torrent inactivo.
const VIDEO_HEADER_WATCHDOG_MAX_CHECKS: usize = 1800;

/// Prefijo máximo comprobado para distinguir un archivo disperso preasignado (todo ceros) de datos reales.
const SPARSE_ZERO_PREFIX_BYTES: usize = 64;
/// Tiempo máximo de espera al resolver metadatos `list_only` de un torrent híbrido auxiliar (20 s).
const HYBRID_METADATA_RESOLVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
/// Tiempo máximo de espera al resolver metadatos `list_only` antes de añadir un torrent (25 s).
const TORRENT_METADATA_RESOLVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);

/// Objetivo de vigilancia de cabecera para un archivo de vídeo dentro de un torrent.
#[derive(Clone)]
struct WatchVideoTarget {
    idx: usize,
    path: PathBuf,
    display_name: String,
    expected_len: u64,
}

/// Lee hasta `HEADER_INSPECT_BYTES` bytes del principio de un archivo de torrent sin bloquear el ejecutor.
async fn read_torrent_header_prefix(
    path: &std::path::Path,
) -> std::io::Result<Option<(Vec<u8>, usize)>> {
    use tokio::io::AsyncReadExt;

    let mut file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let mut buf = vec![0u8; HEADER_INSPECT_BYTES];
    let mut read_total = 0usize;
    while read_total < buf.len() {
        let n = file.read(&mut buf[read_total..]).await?;
        if n == 0 {
            break;
        }
        read_total += n;
    }
    Ok(Some((buf, read_total)))
}

/// Supervisa en segundo plano que las cabeceras binarias de los archivos de vídeo en curso sean legítimas.
///
/// Si un archivo declarado como vídeo contiene una cabecera no válida (por ejemplo, un ejecutable o
/// archivo comprimido renombrado a `.mkv`), elimina de inmediato el torrent y sus archivos en disco.
fn spawn_video_header_watchdog(
    session: std::sync::Arc<Session>,
    id: usize,
    handle: std::sync::Arc<librqbit::ManagedTorrent>,
    video_indices: Vec<usize>,
) {
    let cancel = session.cancellation_token().clone();
    tokio::spawn(async move {
        let out_dir = handle.output_folder().to_path_buf();
        let mut pending: Vec<WatchVideoTarget> = match handle.with_metadata(|meta| {
            video_indices
                .iter()
                .filter_map(|&idx| {
                    meta.file_infos.get(idx).map(|fi| WatchVideoTarget {
                        idx,
                        path: out_dir.join(&fi.relative_filename),
                        display_name: fi.relative_filename.to_string_lossy().into_owned(),
                        expected_len: fi.len,
                    })
                })
                .collect()
        }) {
            Ok(targets) => targets,
            Err(_) => return,
        };

        for _ in 0..VIDEO_HEADER_WATCHDOG_MAX_CHECKS {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(VIDEO_HEADER_WATCHDOG_POLL_INTERVAL) => {}
            }

            if session.get(TorrentIdOrHash::Id(id)).is_none() || pending.is_empty() {
                return;
            }

            let stats = handle.stats();
            let mut next_pending = Vec::with_capacity(pending.len());

            for target in pending {
                let file_progress = stats.file_progress.get(target.idx).copied().unwrap_or(0);
                let file_finished = stats.finished
                    || (target.expected_len > 0 && file_progress >= target.expected_len);

                let (buf, read_len) = match read_torrent_header_prefix(&target.path).await {
                    Ok(Some(res)) => res,
                    Ok(None) => {
                        if file_finished {
                            tracing::warn!(
                                torrent_id = id,
                                file = %target.display_name,
                                "Archivo de vídeo ausente al finalizar el torrent; eliminando por política"
                            );
                            let _ = session.delete(TorrentIdOrHash::Id(id), true).await;
                            return;
                        }
                        next_pending.push(target);
                        continue;
                    }
                    Err(err) => {
                        tracing::debug!(
                            torrent_id = id,
                            file = %target.display_name,
                            error = %err,
                            "Error transitorio leyendo cabecera de torrent; se reintentará"
                        );
                        next_pending.push(target);
                        continue;
                    }
                };

                if read_len == 0 && !file_finished {
                    next_pending.push(target);
                    continue;
                }

                let probe = &buf[..read_len];
                if !file_finished && probe.len() < HEADER_INSPECT_MIN_BYTES {
                    next_pending.push(target);
                    continue;
                }

                let sparse_zeros_end = probe.len().min(SPARSE_ZERO_PREFIX_BYTES);
                if !file_finished
                    && sparse_zeros_end > 0
                    && probe[..sparse_zeros_end].iter().all(|&b| b == 0)
                {
                    // Archivo disperso preasignado mientras la pieza 0 aún no ha llegado.
                    next_pending.push(target);
                    continue;
                }

                if let Err(err) = validate_video_header(probe, &target.display_name) {
                    tracing::warn!(
                        torrent_id = id,
                        file = %target.display_name,
                        error = %err,
                        "Cabecera binaria no válida detectada en torrent activo; eliminando torrent y datos"
                    );
                    let _ = session.delete(TorrentIdOrHash::Id(id), true).await;
                    return;
                }
            }

            pending = next_pending;
            if pending.is_empty() {
                return;
            }
        }
    });
}

/// Instantánea pública de estado de una descarga torrent gestionada.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentSnapshot {
    pub id: usize,
    pub name: String,
    pub info_hash: String,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub progress: f64,
    pub status: String,
    pub download_rate_bps: u64,
    pub upload_rate_bps: u64,
    pub peer_count: usize,
    pub output_dir: String,
    pub category: Option<String>,
}

/// Motor BitTorrent moderno integrado con aplicación de política de red y bloqueo IP.
pub struct TorrentEngine {
    session: Arc<Session>,
    default_output_dir: PathBuf,
    categories: RwLock<HashMap<String, String>>,
    policy: Option<Arc<NetworkPolicyManager>>,
    peer_limit: usize,
    capabilities: TorrentCapabilitiesSnapshot,
}

impl TorrentEngine {
    /// Inicia el motor con configuración estándar predeterminada (sin políticas externas).
    pub async fn new(
        default_output_dir: PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let opts = SessionOptions::default();
        let session = Session::new_with_opts(default_output_dir.clone(), opts).await?;
        Ok(Self {
            session,
            default_output_dir,
            categories: RwLock::new(HashMap::new()),
            policy: None,
            peer_limit: DEFAULT_TORRENT_PEER_LIMIT,
            capabilities: TorrentCapabilitiesSnapshot {
                utp_enabled: false,
                ipv6_dual_stack: true,
                fastresume_enabled: false,
                ip_filter_entries: 0,
                upnp_coordinated: false,
                loopback_only: false,
                mse_pe_supported: false,
                bt_v2_supported: false,
                holepunch_supported: false,
            },
        })
    }

    /// Inicia un motor aislado completamente sin red saliente para pruebas en laboratorio.
    pub async fn new_offline(
        default_output_dir: PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let opts = SessionOptions {
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            fastresume: false,
            persistence: None,
            listen: None,
            ..Default::default()
        };
        let session = Session::new_with_opts(default_output_dir.clone(), opts).await?;
        Ok(Self {
            session,
            default_output_dir,
            categories: RwLock::new(HashMap::new()),
            policy: None,
            peer_limit: DEFAULT_TORRENT_PEER_LIMIT,
            capabilities: TorrentCapabilitiesSnapshot {
                utp_enabled: false,
                ipv6_dual_stack: false,
                fastresume_enabled: false,
                ip_filter_entries: 0,
                upnp_coordinated: false,
                loopback_only: true,
                mse_pe_supported: false,
                bt_v2_supported: false,
                holepunch_supported: false,
            },
        })
    }

    /// Inicia el motor BitTorrent integrando `NetworkPolicyManager`, `IpFilter`, uTP,
    /// Dual-Stack IPv6, `fastresume`, UPnP coordinado e interruptor de corte atómico.
    pub async fn new_with_policy(
        policy: Arc<NetworkPolicyManager>,
        cfg: TorrentPolicyConfig,
    ) -> Result<Self, TorrentError> {
        let state = policy.state();
        if matches!(state, NetworkState::Disabled | NetworkState::ShuttingDown) {
            return Err(TorrentError::PolicyDenied(
                "la red está deshabilitada por el interruptor de corte".to_string(),
            ));
        }

        if cfg
            .default_output_dir
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(TorrentError::InvalidPath(
                "no se permite '..' en la carpeta de descargas torrent".to_string(),
            ));
        }

        let filter_for_export = policy.ip_filter();
        let blocklist_dir_for_export = cfg.blocklist_dir.clone();
        let exported = tokio::task::spawn_blocking(move || {
            blocklist::export_ipfilter_to_librqbit_file(
                &filter_for_export,
                &blocklist_dir_for_export,
            )
        })
        .await
        .map_err(|e| TorrentError::Session(format!("error en tarea blocklist: {e}")))?
        .map_err(|e| {
            TorrentError::Session(format!("error exportando lista de bloqueo ipfilter: {e}"))
        })?;
        let (blocklist_url, ip_filter_entries) = match exported {
            Some((path, count)) => {
                let url = url::Url::from_file_path(&path).map_err(|_| {
                    TorrentError::InvalidPath(format!(
                        "no se pudo convertir ruta de blocklist a URI file://: {:?}",
                        path
                    ))
                })?;
                (Some(url.to_string()), count)
            }
            None => (None, 0),
        };

        let policy_cancel = policy.cancellation_token();
        let rqbit_cancel = tokio_util::sync::CancellationToken::new();
        let rqbit_cancel_bridge = rqbit_cancel.clone();
        tokio::spawn(async move {
            policy_cancel.cancelled().await;
            rqbit_cancel_bridge.cancel();
        });

        let loopback_only = policy.profile().is_loopback_only();
        let peer_limit = cfg.peer_limit.max(1);

        let (session, effective_ipv6, effective_fastresume, effective_utp, effective_upnp) =
            if loopback_only {
                let opts = SessionOptions {
                    dht: None,
                    disable_trackers: true,
                    disable_local_service_discovery: true,
                    fastresume: false,
                    persistence: None,
                    listen: None,
                    blocklist_url,
                    peer_limit: Some(peer_limit),
                    cancellation_token: Some(rqbit_cancel),
                    client_name_and_version: cfg.client_name_and_version.clone(),
                    ..Default::default()
                };
                let s = Session::new_with_opts(cfg.default_output_dir.clone(), opts)
                    .await
                    .map_err(|e| TorrentError::Session(e.to_string()))?;
                (s, false, false, false, false)
            } else {
                let persistence_folder = if cfg.fastresume {
                    let folder = cfg.blocklist_dir.join(FASTRESUME_SUBDIR);
                    std::fs::create_dir_all(&folder).map_err(|e| {
                        TorrentError::InvalidPath(format!(
                            "no se pudo crear directorio fastresume {:?}: {e}",
                            folder
                        ))
                    })?;
                    Some(folder)
                } else {
                    None
                };

                let build_opts = |use_ipv6: bool,
                                  bl_url: Option<String>,
                                  pers_dir: Option<&std::path::Path>,
                                  tok: tokio_util::sync::CancellationToken|
                 -> SessionOptions {
                    let listen_ip = if use_ipv6 {
                        IpAddr::V6(Ipv6Addr::UNSPECIFIED)
                    } else {
                        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
                    };
                    let listen_addr = SocketAddr::new(listen_ip, cfg.listen_port.unwrap_or(0));
                    let mode = if cfg.enable_utp {
                        ListenerMode::TcpAndUtp
                    } else {
                        ListenerMode::TcpOnly
                    };
                    let persistence = pers_dir.map(|p| SessionPersistenceConfig::Json {
                        folder: Some(p.to_path_buf()),
                    });
                    SessionOptions {
                        fastresume: persistence.is_some(),
                        persistence,
                        listen: Some(ListenerOptions {
                            mode,
                            listen_addr,
                            enable_upnp_port_forwarding: cfg.enable_upnp,
                            ipv4_only: !use_ipv6,
                            ..Default::default()
                        }),
                        ipv4_only: !use_ipv6,
                        blocklist_url: bl_url,
                        peer_limit: Some(peer_limit),
                        cancellation_token: Some(tok),
                        client_name_and_version: cfg.client_name_and_version.clone(),
                        ..Default::default()
                    }
                };

                let first_opts = build_opts(
                    cfg.ipv6_dual_stack,
                    blocklist_url.clone(),
                    persistence_folder.as_deref(),
                    rqbit_cancel.clone(),
                );
                match Session::new_with_opts(cfg.default_output_dir.clone(), first_opts).await {
                    Ok(s) => (
                        s,
                        cfg.ipv6_dual_stack,
                        cfg.fastresume,
                        cfg.enable_utp,
                        cfg.enable_upnp,
                    ),
                    Err(first_err) if cfg.ipv6_dual_stack => {
                        tracing::warn!(
                            "[BT] Error iniciando escucha dual-stack IPv6 ({first_err}); reintentando en IPv4"
                        );
                        let fallback_opts = build_opts(
                            false,
                            blocklist_url,
                            persistence_folder.as_deref(),
                            rqbit_cancel,
                        );
                        let s = Session::new_with_opts(cfg.default_output_dir.clone(), fallback_opts)
                            .await
                            .map_err(|e| TorrentError::Session(e.to_string()))?;
                        (s, false, cfg.fastresume, cfg.enable_utp, cfg.enable_upnp)
                    }
                    Err(e) => return Err(TorrentError::Session(e.to_string())),
                }
            };

        // Reactivar la vigilancia de cabeceras «Solo Vídeo» en torrents restaurados por `fastresume`.
        let resumed_watchdogs: Vec<_> = session.with_torrents(|torrents| {
                torrents
                    .filter_map(|(id, handle)| {
                        if handle.stats().finished {
                            return None;
                        }
                        let video_indices: Vec<usize> = handle
                            .with_metadata(|meta| {
                                meta.file_infos
                                    .iter()
                                    .enumerate()
                                    .filter_map(|(idx, fi)| {
                                        let name = fi.relative_filename.to_string_lossy();
                                        if mule_network_policy::content_policy::is_allowed_video_filename(&name) {
                                            Some(idx)
                                        } else {
                                            None
                                        }
                                    })
                                    .collect()
                            })
                            .ok()?;
                        if video_indices.is_empty() {
                            None
                        } else {
                            Some((id, handle.clone(), video_indices))
                        }
                    })
                    .collect()
            });
        for (id, handle, video_indices) in resumed_watchdogs {
            spawn_video_header_watchdog(
                Arc::clone(&session),
                id,
                handle,
                video_indices,
            );
        }

        Ok(Self {
            session,
            default_output_dir: cfg.default_output_dir,
            categories: RwLock::new(HashMap::new()),
            policy: Some(policy),
            peer_limit,
            capabilities: TorrentCapabilitiesSnapshot {
                utp_enabled: effective_utp,
                ipv6_dual_stack: effective_ipv6,
                fastresume_enabled: effective_fastresume,
                ip_filter_entries,
                upnp_coordinated: effective_upnp,
                loopback_only,
                mse_pe_supported: false,
                bt_v2_supported: false,
                holepunch_supported: false,
            },
        })
    }

    /// Devuelve la instantánea verificable de capacidades reales del motor BitTorrent.
    pub fn capabilities(&self) -> TorrentCapabilitiesSnapshot {
        self.capabilities.clone()
    }

    fn ensure_policy_active(&self) -> Result<(), TorrentError> {
        if let Some(policy) = &self.policy {
            let st = policy.state();
            if matches!(st, NetworkState::Disabled | NetworkState::ShuttingDown) {
                return Err(TorrentError::PolicyDenied(
                    "operación torrent denegada por interruptor de corte".to_string(),
                ));
            }
        }
        Ok(())
    }

    async fn validate_torrent_url(&self, raw_url: &str) -> Result<(), TorrentError> {
        self.ensure_policy_active()?;
        let Some(policy) = &self.policy else {
            return Ok(());
        };
        let parsed = url::Url::parse(raw_url)
            .map_err(|e| TorrentError::SsrfRejected(format!("URI inválida '{raw_url}': {e}")))?;

        match parsed.scheme() {
            "magnet" => Ok(()),
            "http" | "https" => Self::validate_http_url_against_policy(&parsed, policy).await,
            other => Err(TorrentError::SsrfRejected(format!(
                "esquema URI no permitido '{other}' (solo magnet/http/https)"
            ))),
        }
    }

    async fn validate_http_url_against_policy(
        parsed: &url::Url,
        policy: &NetworkPolicyManager,
    ) -> Result<(), TorrentError> {
        let host = parsed
            .host_str()
            .ok_or_else(|| TorrentError::SsrfRejected("la URL carece de host".to_string()))?;
        let trimmed = host.trim_start_matches('[').trim_end_matches(']');
        let port = parsed.port_or_known_default().unwrap_or(80);
        let profile = policy.profile();

        let addrs: Vec<SocketAddr> = if let Ok(ip) = trimmed.parse::<IpAddr>() {
            vec![SocketAddr::new(ip, port)]
        } else {
            if profile.is_loopback_only() && !trimmed.eq_ignore_ascii_case("localhost") {
                return Err(TorrentError::SsrfRejected(format!(
                    "host remoto '{host}' prohibido en perfil aislado loopback"
                )));
            }
            let resolved: Vec<SocketAddr> = tokio::net::lookup_host((trimmed, port))
                .await
                .map_err(|e| {
                    TorrentError::SsrfRejected(format!("error resolviendo DNS para '{host}': {e}"))
                })?
                .collect();
            if resolved.is_empty() {
                return Err(TorrentError::SsrfRejected(format!(
                    "resolución DNS vacía para '{host}'"
                )));
            }
            resolved
        };

        for addr in addrs {
            validate_connect_address(&addr, profile)
                .map_err(|v| TorrentError::SsrfRejected(v.to_string()))?;
            if let Some(rule) = policy.ip_filter().check_ip(addr.ip()) {
                return Err(TorrentError::SsrfRejected(format!(
                    "IP {addr} bloqueada por ipfilter ({})",
                    rule.description
                )));
            }
        }
        Ok(())
    }

    /// Añade un torrent por URL (`magnet:`, `http://` o `https://`), validando antes la política anti-SSRF.
    pub async fn add_url(
        &self,
        url: &str,
        output_folder: Option<PathBuf>,
        category: Option<String>,
    ) -> Result<TorrentSnapshot, Box<dyn std::error::Error + Send + Sync>> {
        self.validate_torrent_url(url).await?;
        let add_torrent = AddTorrent::from_url(url.to_string());
        self.add_torrent_internal(add_torrent, output_folder, category)
            .await
    }

    /// Añade un torrent desde los bytes de un archivo `.torrent`.
    pub async fn add_bytes(
        &self,
        bytes: Vec<u8>,
        output_folder: Option<PathBuf>,
        category: Option<String>,
    ) -> Result<TorrentSnapshot, Box<dyn std::error::Error + Send + Sync>> {
        self.ensure_policy_active()?;
        let add_torrent = AddTorrent::from_bytes(bytes);
        self.add_torrent_internal(add_torrent, output_folder, category)
            .await
    }

    async fn add_torrent_internal(
        &self,
        add_torrent: AddTorrent<'static>,
        output_folder: Option<PathBuf>,
        category: Option<String>,
    ) -> Result<TorrentSnapshot, Box<dyn std::error::Error + Send + Sync>> {
        let out_dir = output_folder.unwrap_or_else(|| self.default_output_dir.clone());
        if out_dir
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err("Path traversal ('..') denied in torrent output folder".into());
        }

        // Paso 1: resolver metadatos sin asignar archivos en disco (`list_only`) y filtrar por política «Solo Vídeo».
        let list_opts = AddTorrentOptions {
            list_only: true,
            output_folder: Some(out_dir.to_string_lossy().to_string()),
            peer_limit: Some(self.peer_limit),
            ..Default::default()
        };
        let list_only = match tokio::time::timeout(
            TORRENT_METADATA_RESOLVE_TIMEOUT,
            self.session.add_torrent(add_torrent, Some(list_opts)),
        )
        .await
        .map_err(|_| {
            Box::new(TorrentError::Session(format!(
                "tiempo de espera agotado ({} s) resolviendo metadatos del torrent",
                TORRENT_METADATA_RESOLVE_TIMEOUT.as_secs()
            )))
        })?? {
            AddTorrentResponse::ListOnly(resp) => resp,
            _ => {
                return Err(Box::new(TorrentError::Session(
                    "se esperaba respuesta ListOnly al inspeccionar el torrent".to_string(),
                )));
            }
        };

        let file_entries: Vec<(usize, String)> = list_only
            .info
            .iter_file_details()
            .enumerate()
            .map(|(idx, fd)| (idx, fd.filename.to_string()))
            .collect();
        let selection = select_torrent_video_files(
            file_entries.iter().map(|(idx, path)| (*idx, path.as_str())),
        )
        .map_err(|err| Box::new(TorrentError::ContentRejected(err.to_string())))?;

        let resolved_add = AddTorrent::from_bytes(list_only.torrent_bytes);
        let opts = AddTorrentOptions {
            output_folder: Some(out_dir.to_string_lossy().to_string()),
            overwrite: true,
            only_files: Some(selection.selected_indices),
            initial_peers: if list_only.seen_peers.is_empty() {
                None
            } else {
                Some(list_only.seen_peers)
            },
            peer_limit: Some(self.peer_limit),
            ..Default::default()
        };

        let res = self.session.add_torrent(resolved_add, Some(opts)).await?;
        let (id, handle) = match res {
            AddTorrentResponse::Added(id, handle) => {
                if !selection.video_indices.is_empty() {
                    spawn_video_header_watchdog(
                        std::sync::Arc::clone(&self.session),
                        id,
                        handle.clone(),
                        selection.video_indices,
                    );
                }
                (id, handle)
            }
            AddTorrentResponse::AlreadyManaged(id, handle) => (id, handle),
            AddTorrentResponse::ListOnly(_) => return Err("Torrent is list only".into()),
        };

        let hash_hex = handle.info_hash().as_string();
        if let Some(cat) = category.clone() {
            if let Ok(mut guard) = self.categories.write() {
                guard.insert(hash_hex.clone(), cat);
            }
        }

        let name = handle.name().unwrap_or_else(|| "Torrent".to_string());
        let stats = handle.stats();

        Ok(TorrentSnapshot {
            id,
            name,
            info_hash: hash_hex,
            total_bytes: stats.total_bytes,
            downloaded_bytes: stats.progress_bytes,
            progress: if stats.total_bytes > 0 {
                stats.progress_bytes as f64 / stats.total_bytes as f64
            } else {
                0.0
            },
            status: if stats.finished {
                "completed".to_string()
            } else {
                "downloading".to_string()
            },
            download_rate_bps: stats
                .live
                .as_ref()
                .map(|l| (l.download_speed.mbps * 125_000.0) as u64)
                .unwrap_or(0),
            upload_rate_bps: stats
                .live
                .as_ref()
                .map(|l| (l.upload_speed.mbps * 125_000.0) as u64)
                .unwrap_or(0),
            peer_count: stats
                .live
                .as_ref()
                .map(|l| l.snapshot.peer_stats.live as usize)
                .unwrap_or(0),
            output_dir: out_dir.to_string_lossy().to_string(),
            category,
        })
    }

    /// Lista el estado instantáneo de todos los torrents gestionados.
    pub fn list(&self) -> Vec<TorrentSnapshot> {
        let cats = self.categories.read().ok();
        self.session.with_torrents(|torrents| {
            torrents
                .map(|(id, handle)| {
                    let hash_hex = handle.info_hash().as_string();
                    let name = handle.name().unwrap_or_else(|| "Torrent".to_string());
                    let stats = handle.stats();
                    let category = cats.as_ref().and_then(|c| c.get(&hash_hex).cloned());
                    TorrentSnapshot {
                        id,
                        name,
                        info_hash: hash_hex,
                        total_bytes: stats.total_bytes,
                        downloaded_bytes: stats.progress_bytes,
                        progress: if stats.total_bytes > 0 {
                            stats.progress_bytes as f64 / stats.total_bytes as f64
                        } else {
                            0.0
                        },
                        status: if stats.finished {
                            "completed".to_string()
                        } else {
                            "downloading".to_string()
                        },
                        download_rate_bps: stats
                            .live
                            .as_ref()
                            .map(|l| (l.download_speed.mbps * 125_000.0) as u64)
                            .unwrap_or(0),
                        upload_rate_bps: stats
                            .live
                            .as_ref()
                            .map(|l| (l.upload_speed.mbps * 125_000.0) as u64)
                            .unwrap_or(0),
                        peer_count: stats
                            .live
                            .as_ref()
                            .map(|l| l.snapshot.peer_stats.live as usize)
                            .unwrap_or(0),
                        output_dir: handle.output_folder().to_string_lossy().to_string(),
                        category,
                    }
                })
                .collect()
        })
    }

    /// Pausa la transferencia del torrent identificado por `id`.
    pub async fn pause(&self, id: usize) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(handle) = self.session.get(TorrentIdOrHash::Id(id)) {
            self.session.pause(&handle).await?;
        }
        Ok(())
    }

    /// Reanuda un torrent previamente pausado.
    pub async fn resume(&self, id: usize) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.ensure_policy_active()?;
        if let Some(handle) = self.session.get(TorrentIdOrHash::Id(id)) {
            self.session.unpause(&handle).await?;
        }
        Ok(())
    }

    /// Elimina un torrent de la sesión (opcionalmente borrando archivos del disco).
    pub async fn delete(
        &self,
        id: usize,
        delete_files: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.session
            .delete(TorrentIdOrHash::Id(id), delete_files)
            .await?;
        Ok(())
    }

    /// Ajusta en caliente el límite global de subida BitTorrent (bytes/s; 0 = sin límite).
    pub fn set_upload_limit_bps(&self, bps: u64) {
        let limit = if bps == 0 {
            None
        } else {
            let clamped = bps.min(u64::from(u32::MAX)) as u32;
            std::num::NonZeroU32::new(clamped.max(1))
        };
        self.session.ratelimits.set_upload_bps(limit);
    }

    /// Ajusta en caliente el límite global de bajada BitTorrent (bytes/s; 0 = sin límite).
    pub fn set_download_limit_bps(&self, bps: u64) {
        let limit = if bps == 0 {
            None
        } else {
            let clamped = bps.min(u64::from(u32::MAX)) as u32;
            std::num::NonZeroU32::new(clamped.max(1))
        };
        self.session.ratelimits.set_download_bps(limit);
    }

    /// Devuelve el límite global de subida BitTorrent activo (bytes/s; 0 = sin límite).
    pub fn upload_limit_bps(&self) -> u64 {
        self.session
            .ratelimits
            .get_upload_bps()
            .map_or(0, |v| u64::from(v.get()))
    }

    /// Devuelve las tasas agregadas actuales `(descarga_bps, subida_bps)` de los torrents activos.
    pub fn total_rates_bps(&self) -> (u64, u64) {
        self.session.with_torrents(|torrents| {
            torrents.fold((0u64, 0u64), |(dl, ul), (_id, handle)| {
                let stats = handle.stats();
                let (d, u) = stats
                    .live
                    .as_ref()
                    .map(|l| {
                        (
                            (l.download_speed.mbps * 125_000.0) as u64,
                            (l.upload_speed.mbps * 125_000.0) as u64,
                        )
                    })
                    .unwrap_or((0, 0));
                (dl.saturating_add(d), ul.saturating_add(u))
            })
        })
    }

    /// Extrae hasta `max_nodes` direcciones activas de la tabla de enrutamiento DHT de `librqbit`
    /// para sembrar instantáneamente el Sabueso DHT sin tráfico de bootstrap adicional.
    pub fn dht_routing_nodes(&self, max_nodes: usize) -> Vec<SocketAddr> {
        let Some(dht) = self.session.get_dht() else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(max_nodes);
        dht.with_routing_tables(|v4, _v6| {
            for node in v4.iter() {
                if out.len() >= max_nodes {
                    break;
                }
                let addr = node.addr();
                if addr.port() > 0 {
                    out.push(addr);
                }
            }
        });
        out
    }

    /// Añade un torrent auxiliar por `info_hash` (40 hex) seleccionando exclusivamente el archivo
    /// de vídeo que coincida exactamente al byte con `expected_file_size`.
    pub async fn add_hybrid_torrent_for_exact_size(
        &self,
        info_hash_hex: &str,
        expected_file_size: u64,
        output_folder: PathBuf,
        initial_peers: Vec<SocketAddr>,
    ) -> Result<HybridTorrentLaunchResult, Box<dyn std::error::Error + Send + Sync>> {
        self.ensure_policy_active()?;
        let normalized = info_hash_hex.trim().to_ascii_lowercase();
        if normalized.len() != 40 || !normalized.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Box::new(TorrentError::Session(format!(
                "info_hash inválido para descarga híbrida: '{info_hash_hex}'"
            ))));
        }
        if self.capabilities.loopback_only && initial_peers.is_empty() {
            return Err(Box::new(TorrentError::PolicyDenied(
                "no es posible resolver un magnet sin pares iniciales en perfil aislado loopback"
                    .to_string(),
            )));
        }
        let magnet = format!("magnet:?xt=urn:btih:{normalized}");
        self.add_hybrid_torrent_internal(
            AddTorrent::from_url(magnet),
            expected_file_size,
            output_folder,
            initial_peers,
        )
        .await
    }

    /// Añade un torrent auxiliar desde los bytes de un `.torrent` seleccionando exclusivamente
    /// el archivo de vídeo con tamaño exacto `expected_file_size` (arranque instantáneo sin re-descargar metadatos).
    pub async fn add_hybrid_torrent_bytes_for_exact_size(
        &self,
        torrent_bytes: Vec<u8>,
        expected_file_size: u64,
        output_folder: PathBuf,
        initial_peers: Vec<SocketAddr>,
    ) -> Result<HybridTorrentLaunchResult, Box<dyn std::error::Error + Send + Sync>> {
        self.ensure_policy_active()?;
        self.add_hybrid_torrent_internal(
            AddTorrent::from_bytes(torrent_bytes),
            expected_file_size,
            output_folder,
            initial_peers,
        )
        .await
    }

    async fn add_hybrid_torrent_internal(
        &self,
        add_torrent: AddTorrent<'static>,
        expected_file_size: u64,
        output_folder: PathBuf,
        initial_peers: Vec<SocketAddr>,
    ) -> Result<HybridTorrentLaunchResult, Box<dyn std::error::Error + Send + Sync>> {
        if output_folder
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err("Path traversal ('..') denied in hybrid torrent output folder".into());
        }

        let list_opts = AddTorrentOptions {
            list_only: true,
            output_folder: Some(output_folder.to_string_lossy().to_string()),
            initial_peers: if initial_peers.is_empty() {
                None
            } else {
                Some(initial_peers.clone())
            },
            peer_limit: Some(self.peer_limit),
            ..Default::default()
        };
        let list_only = match tokio::time::timeout(
            HYBRID_METADATA_RESOLVE_TIMEOUT,
            self.session.add_torrent(add_torrent, Some(list_opts)),
        )
        .await
        .map_err(|_| {
            Box::new(TorrentError::Session(format!(
                "tiempo de espera agotado ({} s) resolviendo metadatos del torrent híbrido",
                HYBRID_METADATA_RESOLVE_TIMEOUT.as_secs()
            )))
        })?? {
            AddTorrentResponse::ListOnly(resp) => resp,
            _ => {
                return Err(Box::new(TorrentError::Session(
                    "se esperaba respuesta ListOnly al inspeccionar el torrent híbrido".to_string(),
                )));
            }
        };

        let mut offset = 0u64;
        let mut matched: Option<(usize, u64, String, u64)> = None;
        for (idx, fd) in list_only.info.iter_file_details().enumerate() {
            let name = fd.filename.to_string();
            let len = fd.len;
            if len == expected_file_size
                && mule_network_policy::content_policy::is_allowed_video_filename(&name)
            {
                matched = Some((idx, offset, name, len));
                break;
            }
            offset = offset.saturating_add(len);
        }

        let (file_index, file_offset, file_name, file_size) = matched.ok_or_else(|| {
            Box::new(TorrentError::ContentRejected(format!(
                "ningún archivo de vídeo del torrent coincide con el tamaño exacto de {expected_file_size} bytes"
            )))
        })?;

        let mut merged_peers = list_only.seen_peers;
        for p in initial_peers {
            if !merged_peers.contains(&p) {
                merged_peers.push(p);
            }
        }

        let resolved_add = AddTorrent::from_bytes(list_only.torrent_bytes);
        let opts = AddTorrentOptions {
            output_folder: Some(output_folder.to_string_lossy().to_string()),
            overwrite: true,
            only_files: Some(vec![file_index]),
            initial_peers: if merged_peers.is_empty() {
                None
            } else {
                Some(merged_peers)
            },
            peer_limit: Some(self.peer_limit),
            ..Default::default()
        };

        let res = self.session.add_torrent(resolved_add, Some(opts)).await?;
        let (id, handle) = match res {
            AddTorrentResponse::Added(id, handle) => {
                spawn_video_header_watchdog(
                    Arc::clone(&self.session),
                    id,
                    handle.clone(),
                    vec![file_index],
                );
                (id, handle)
            }
            AddTorrentResponse::AlreadyManaged(id, handle) => (id, handle),
            AddTorrentResponse::ListOnly(_) => return Err("Hybrid torrent is list only".into()),
        };

        let info_hash = handle.info_hash().as_string();
        if let Ok(mut guard) = self.categories.write() {
            guard.insert(info_hash.clone(), "__hybrid__".to_string());
        }

        Ok(HybridTorrentLaunchResult {
            torrent_id: id,
            info_hash,
            file_index,
            file_offset,
            file_name,
            file_size,
        })
    }

    /// Devuelve los rangos de bytes verificados por SHA-1 del archivo `file_index` de un torrent activo.
    pub fn verified_file_state(
        &self,
        torrent_id: usize,
        file_index: usize,
    ) -> Option<VerifiedTorrentFileState> {
        let handle = self.session.get(TorrentIdOrHash::Id(torrent_id))?;
        let info_hash = handle.info_hash().as_string();
        let out_dir = handle.output_folder().to_path_buf();
        let stats = handle.stats();

        let (file_path, file_size, file_offset, piece_range, piece_length) = handle
            .with_metadata(|meta| {
                let fi = meta.file_infos.get(file_index)?;
                Some((
                    out_dir.join(&fi.relative_filename),
                    fi.len,
                    fi.offset_in_torrent,
                    fi.piece_range.clone(),
                    u64::from(meta.lengths().default_piece_length()),
                ))
            })
            .ok()??;

        let verified_ranges = if stats.finished {
            vec![(0, file_size)]
        } else {
            let api = librqbit::Api::new(Arc::clone(&self.session), None);
            api.api_dump_haves(TorrentIdOrHash::Id(torrent_id))
                .map(|(have, _total_pieces)| {
                    compute_verified_file_ranges(
                        file_offset,
                        file_size,
                        piece_length,
                        piece_range,
                        |piece_idx| have.get(piece_idx as usize).is_some_and(|b| *b),
                    )
                })
                .unwrap_or_default()
        };

        Some(VerifiedTorrentFileState {
            torrent_id,
            info_hash,
            file_index,
            file_path,
            file_size,
            finished: stats.finished
                || (file_size > 0
                    && stats.file_progress.get(file_index).copied().unwrap_or(0) >= file_size),
            peer_count: stats
                .live
                .as_ref()
                .map(|l| l.snapshot.peer_stats.live as usize)
                .unwrap_or(0),
            download_rate_bps: stats
                .live
                .as_ref()
                .map(|l| (l.download_speed.mbps * 125_000.0) as u64)
                .unwrap_or(0),
            verified_ranges,
        })
    }

    /// Enumera los archivos de vídeo completamente descargados por BitTorrent (excluyendo torrents
    /// auxiliares `__hybrid__`) para calcular su hash eD2K y sembrar el puente en Kad.
    pub fn list_completed_video_files(&self) -> Vec<CompletedTorrentVideo> {
        let cats = self.categories.read().ok();
        self.session.with_torrents(|torrents| {
            let mut out = Vec::new();
            for (id, handle) in torrents {
                let info_hash = handle.info_hash().as_string();
                if cats
                    .as_ref()
                    .and_then(|c| c.get(&info_hash))
                    .is_some_and(|cat| cat == "__hybrid__")
                {
                    continue;
                }
                let stats = handle.stats();
                if !stats.finished {
                    continue;
                }
                let torrent_name = handle.name().unwrap_or_else(|| "Torrent".to_string());
                let out_dir = handle.output_folder().to_path_buf();
                if let Ok(files) = handle.with_metadata(|meta| {
                    meta.file_infos
                        .iter()
                        .enumerate()
                        .filter_map(|(idx, fi)| {
                            let rel = fi.relative_filename.to_string_lossy().into_owned();
                            if fi.len > 0
                                && mule_network_policy::content_policy::is_allowed_video_filename(
                                    &rel,
                                )
                            {
                                let path = out_dir.join(&fi.relative_filename);
                                if path.is_file() {
                                    return Some(CompletedTorrentVideo {
                                        torrent_id: id,
                                        info_hash: info_hash.clone(),
                                        torrent_name: torrent_name.clone(),
                                        file_index: idx,
                                        file_offset: fi.offset_in_torrent,
                                        file_name: rel,
                                        file_path: path,
                                        file_size: fi.len,
                                        total_torrent_bytes: stats.total_bytes,
                                    });
                                }
                            }
                            None
                        })
                        .collect::<Vec<_>>()
                }) {
                    out.extend(files);
                }
            }
            out
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mule_network_policy::{NetworkProfile, ResourceBudgets};

    /// Construye un archivo `.torrent` válido mínimo de un único fichero en memoria para pruebas sin conexión.
    fn single_file_torrent_bytes(file_name: &str, length: usize) -> Vec<u8> {
        let mut buf = Vec::new();
        let header = format!(
            "d8:announce25:http://127.0.0.1/announce4:infod6:lengthi{length}e4:name{}:{file_name}12:piece lengthi16384e6:pieces20:",
            file_name.len()
        );
        buf.extend_from_slice(header.as_bytes());
        buf.extend_from_slice(&[0x41u8; 20]);
        buf.extend_from_slice(b"ee");
        buf
    }

    /// Construye un archivo `.torrent` multifichero válido en memoria para pruebas sin conexión.
    fn multi_file_torrent_bytes(dir_name: &str, files: &[(&str, usize)]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"d8:announce25:http://127.0.0.1/announce4:infod5:filesl");
        for (name, len) in files {
            let entry = format!(
                "d6:lengthi{len}e4:pathl{}:{name}ee",
                name.len()
            );
            buf.extend_from_slice(entry.as_bytes());
        }
        let trailer = format!(
            "e4:name{}:{dir_name}12:piece lengthi16384e6:pieces20:",
            dir_name.len()
        );
        buf.extend_from_slice(trailer.as_bytes());
        buf.extend_from_slice(&[0x41u8; 20]);
        buf.extend_from_slice(b"ee");
        buf
    }

    /// Construye el torrent de vídeo de ejemplo por defecto para pruebas sin conexión.
    fn sample_torrent_bytes() -> Vec<u8> {
        single_file_torrent_bytes("sample.mkv", 1024)
    }

    #[tokio::test]
    async fn test_rejects_non_video_single_and_multifile_torrent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let engine = TorrentEngine::new_offline(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        let err = engine
            .add_bytes(single_file_torrent_bytes("malware.exe", 2048), None, None)
            .await
            .expect_err("el torrent con único archivo .exe debe rechazarse");
        assert!(
            err.to_string().contains("vídeo"),
            "error inesperado: {err}"
        );

        let err_multi = engine
            .add_bytes(
                multi_file_torrent_bytes(
                    "ReleasePack",
                    &[("readme.txt", 128), ("setup.msi", 4096)],
                ),
                None,
                None,
            )
            .await
            .expect_err("el torrent multifichero sin vídeo debe rechazarse");
        assert!(
            err_multi.to_string().contains("vídeo"),
            "error inesperado: {err_multi}"
        );
        assert!(engine.list().is_empty());
    }

    #[tokio::test]
    async fn test_selects_only_video_and_subtitles_in_multifile_torrent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let engine = TorrentEngine::new_offline(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        let torrent = multi_file_torrent_bytes(
            "MoviePack",
            &[
                ("readme.nfo", 64),
                ("movie.mkv", 2048),
                ("subs.es.srt", 128),
                ("keygen.exe", 512),
            ],
        );
        let snap = engine
            .add_bytes(torrent, None, Some("movies".to_string()))
            .await
            .expect("un torrent con vídeo y subtítulo debe admitirse");

        let handle = engine
            .session
            .get(TorrentIdOrHash::Id(snap.id))
            .expect("el torrent debe estar registrado");
        assert_eq!(handle.only_files(), Some(vec![1, 2]));
    }

    #[tokio::test]
    async fn test_watchdog_deletes_torrent_with_invalid_binary_header() {
        let temp_dir = tempfile::tempdir().unwrap();
        let engine = TorrentEngine::new_offline(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        let snap = engine
            .add_bytes(
                single_file_torrent_bytes("spoofed_movie.mkv", 1024),
                None,
                None,
            )
            .await
            .expect("la admisión nominal por .mkv debe permitirse inicialmente");

        let handle = engine
            .session
            .get(TorrentIdOrHash::Id(snap.id))
            .expect("handle activo");
        let file_path = handle.output_folder().join("spoofed_movie.mkv");
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut fake_payload = vec![0x90u8; 1024];
        fake_payload[..2].copy_from_slice(b"MZ");
        std::fs::write(&file_path, &fake_payload).unwrap();

        for _ in 0..60 {
            if engine.list().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        assert!(
            engine.list().is_empty(),
            "el vigilante debe eliminar el torrent cuya cabecera binaria sea PE/MZ"
        );
        assert!(
            !file_path.exists(),
            "los datos del torrent rechazado también deben borrarse del disco"
        );
    }

    #[tokio::test]
    async fn test_offline_engine_add_list_category_and_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let engine = TorrentEngine::new_offline(dir.path().to_path_buf())
            .await
            .expect("offline torrent engine should initialize");

        assert!(engine.list().is_empty());

        let snap = engine
            .add_bytes(sample_torrent_bytes(), None, Some("movies".to_string()))
            .await
            .expect("valid bencoded torrent should be added");

        assert_eq!(snap.name, "sample.mkv");
        assert_eq!(snap.total_bytes, 1024);
        assert_eq!(snap.category.as_deref(), Some("movies"));

        engine.pause(snap.id).await.unwrap();

        let listed = engine.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].info_hash, snap.info_hash);
        assert_eq!(listed[0].category.as_deref(), Some("movies"));

        engine.delete(snap.id, false).await.unwrap();
        assert!(engine.list().is_empty());
    }

    #[tokio::test]
    async fn test_invalid_torrent_bytes_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let engine = TorrentEngine::new_offline(dir.path().to_path_buf())
            .await
            .unwrap();
        let err = engine.add_bytes(b"not-valid-bencode".to_vec(), None, None).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_new_with_policy_loads_ipfilter_blocklist_and_respects_kill_switch() {
        let dir = tempfile::tempdir().unwrap();
        let dl_dir = dir.path().join("downloads");
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&dl_dir).unwrap();

        let policy = Arc::new(NetworkPolicyManager::new(
            NetworkProfile::LocalLab,
            ResourceBudgets::default(),
        ));
        policy
            .ip_filter()
            .load_from_str("203.0.113.10 - 203.0.113.20 , 0 , TestBlock\n")
            .unwrap();

        let cfg = TorrentPolicyConfig::new(dl_dir, data_dir.clone());
        let engine = TorrentEngine::new_with_policy(Arc::clone(&policy), cfg)
            .await
            .expect("engine with ipfilter blocklist should start");

        let caps = engine.capabilities();
        assert_eq!(caps.ip_filter_entries, 1);
        assert!(caps.loopback_only);
        assert!(!caps.mse_pe_supported);
        assert!(data_dir.join("librqbit_ipfilter.p2p").exists());

        // Activar interruptor de corte bloquea nuevas operaciones.
        policy.disconnect_all();
        let err = engine.add_bytes(sample_torrent_bytes(), None, None).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_add_url_enforces_ssrf_and_network_policy() {
        let dir = tempfile::tempdir().unwrap();
        let dl_dir = dir.path().join("dl");
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&dl_dir).unwrap();

        // Bajo perfil LocalLab se bloquean URLs HTTP no-loopback.
        let lab_policy = Arc::new(NetworkPolicyManager::new(
            NetworkProfile::LocalLab,
            ResourceBudgets::default(),
        ));
        let engine = TorrentEngine::new_with_policy(
            lab_policy,
            TorrentPolicyConfig::new(dl_dir.clone(), data_dir.clone()),
        )
        .await
        .unwrap();

        for bad in [
            "http://169.254.169.254/latest.torrent",
            "http://192.168.1.1/file.torrent",
            "http://example.com/linux.torrent",
        ] {
            let err = engine.add_url(bad, None, None).await;
            assert!(err.is_err(), "debe rechazarse {bad} en LocalLab");
        }

        // Validación anti-SSRF bajo perfil público sin arrancar tráfico DHT saliente.
        let pub_policy = NetworkPolicyManager::new(
            NetworkProfile::PublicEd2kKadPreview,
            ResourceBudgets::default(),
        );
        pub_policy
            .ip_filter()
            .load_from_str("8.8.4.4 - 8.8.4.4 , 0 , BlockedPublic\n")
            .unwrap();
        for bad_ssrf in [
            "http://169.254.169.254/a.torrent",
            "http://10.0.0.1/a.torrent",
            "http://127.0.0.1/a.torrent",
            "http://8.8.4.4/a.torrent",
        ] {
            let parsed = url::Url::parse(bad_ssrf).unwrap();
            assert!(
                TorrentEngine::validate_http_url_against_policy(&parsed, &pub_policy)
                    .await
                    .is_err(),
                "SSRF o ipfilter en perfil público debía rechazar {bad_ssrf}"
            );
        }
        let ok_public = url::Url::parse("https://8.8.8.8/ubuntu.torrent").unwrap();
        assert!(
            TorrentEngine::validate_http_url_against_policy(&ok_public, &pub_policy)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_hot_rate_limit_update_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let engine = TorrentEngine::new_offline(dir.path().to_path_buf())
            .await
            .unwrap();
        assert_eq!(engine.upload_limit_bps(), 0);
        engine.set_upload_limit_bps(512 * 1024);
        assert_eq!(engine.upload_limit_bps(), 512 * 1024);
        engine.set_upload_limit_bps(0);
        assert_eq!(engine.upload_limit_bps(), 0);
        assert_eq!(engine.total_rates_bps(), (0, 0));
    }
}

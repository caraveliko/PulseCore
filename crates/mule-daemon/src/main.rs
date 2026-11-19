// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Punto de entrada principal de `mule-daemon` (servidor P2P eD2K/Kad/BitTorrent,
//! WebUI embebida e icono opcional en la bandeja del sistema en Windows).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use mule_daemon::{
    create_router, AppState, DaemonConfig, DaemonSession, DirectoryLock, DownloadService,
};
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[cfg(all(target_os = "linux", not(target_env = "musl")))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

// Configuración de purga agresiva de jemalloc para minimizar el RSS en Linux:
// Libera páginas sucias y "muzzy" inmediatamente al kernel evitando la retención de memoria inactiva.
#[cfg(all(target_os = "linux", not(target_env = "musl")))]
#[allow(non_upper_case_globals)]
#[export_name = "malloc_conf"]
pub static malloc_conf: &[u8] = b"dirty_decay_ms:0,muzzy_decay_ms:0,background_thread:true\0";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Limpieza silenciosa de binarios temporales huérfanos (.update), conservando .old para rollback
    if let Ok(exe_path) = std::env::current_exe() {
        mule_daemon::services::updater::cleanup_stale_staging_if_present(&exe_path);
    }

    let args: Vec<String> = std::env::args().collect();
    let mut data_dir_arg: Option<PathBuf> = None;
    let mut downloads_dir_arg: Option<PathBuf> = None;
    let mut mode_override: Option<mule_daemon::WorkDirMode> = None;
    let mut bind_addr_arg: Option<std::net::SocketAddr> = None;
    let mut auth_token_arg: Option<String> = None;
    let mut token_file_arg: Option<PathBuf> = None;
    let mut parent_pid_arg: Option<u32> = None;
    let mut profile_arg: Option<mule_network_policy::NetworkProfile> = None;
    let mut ed2k_port_arg: Option<u16> = None;
    let mut kad_port_arg: Option<u16> = None;
    let mut upnp_arg: Option<bool> = None;
    let mut web_dir_arg: Option<PathBuf> = None;
    let mut no_tray_arg: bool = std::env::var("MULE_NO_TRAY")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--workdir" if i + 1 < args.len() => {
                data_dir_arg = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--downloads-dir" | "--download-dir" if i + 1 < args.len() => {
                downloads_dir_arg = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--mode" | "--workdir-mode" if i + 1 < args.len() => {
                mode_override = match args[i + 1].to_lowercase().as_str() {
                    "portable" => Some(mule_daemon::WorkDirMode::Portable),
                    "installeddefault" | "installed" => {
                        Some(mule_daemon::WorkDirMode::InstalledDefault)
                    }
                    "environment" | "env" => Some(mule_daemon::WorkDirMode::Environment),
                    "explicitcli" | "cli" => Some(mule_daemon::WorkDirMode::ExplicitCli),
                    _ => None,
                };
                i += 2;
            }
            "--profile" | "--network-profile" if i + 1 < args.len() => {
                if let Ok(p) = args[i + 1].parse() {
                    profile_arg = Some(p);
                }
                i += 2;
            }
            "--ed2k-port" if i + 1 < args.len() => {
                if let Ok(port) = args[i + 1].parse::<u16>() {
                    ed2k_port_arg = Some(port);
                }
                i += 2;
            }
            "--kad-port" if i + 1 < args.len() => {
                if let Ok(port) = args[i + 1].parse::<u16>() {
                    kad_port_arg = Some(port);
                }
                i += 2;
            }
            "--upnp" if i + 1 < args.len() => {
                upnp_arg = match args[i + 1].to_lowercase().as_str() {
                    "true" | "1" | "yes" | "enable" => Some(true),
                    "false" | "0" | "no" | "disable" => Some(false),
                    _ => None,
                };
                i += 2;
            }
            "--bind" if i + 1 < args.len() => {
                if let Ok(addr) = args[i + 1].parse() {
                    bind_addr_arg = Some(addr);
                }
                i += 2;
            }
            "--web-dir" | "--webdir" if i + 1 < args.len() => {
                web_dir_arg = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--token" | "--auth-token" if i + 1 < args.len() => {
                auth_token_arg = Some(args[i + 1].clone());
                i += 2;
            }
            "--token-file" if i + 1 < args.len() => {
                token_file_arg = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--parent-pid" if i + 1 < args.len() => {
                if let Ok(pid) = args[i + 1].parse::<u32>() {
                    parent_pid_arg = Some(pid);
                }
                i += 2;
            }
            "--no-tray" => {
                no_tray_arg = true;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    let mut resolved_workdir =
        mule_daemon::WorkDirectoryResolver::resolve(data_dir_arg, downloads_dir_arg);
    if let Some(mode) = mode_override {
        resolved_workdir.mode = mode;
    } else if resolved_workdir.mode == mule_daemon::WorkDirMode::ExplicitCli {
        // Si se pasó una ruta de workdir explícita, comprobar si reside en la raíz de un paquete portable
        // (es decir, existe <bundle_root>/portable.flag)
        if let Some(parent) = resolved_workdir.workdir.parent() {
            if parent.join(mule_daemon::PORTABLE_FLAG_FILENAME).exists() {
                resolved_workdir.mode = mule_daemon::WorkDirMode::Portable;
            }
        }
    }
    let data_dir = resolved_workdir.workdir;
    let default_download_dir = resolved_workdir.default_download_dir;
    std::fs::create_dir_all(&data_dir)?;
    std::fs::create_dir_all(&default_download_dir)?;

    // === Registro (logging): consola legible + archivo JSONL rotativo ===
    // Predeterminado 'info', sobrescritura mediante `RUST_LOG` o en caliente mediante la clave
    // `logLevel` de `settings.set` (solo en tiempo de ejecución, nunca persistido).
    // Archivos: <workdir>/logs/daemon.<fecha>.log — JSONL, rotación
    // diaria, retención de 7 días, escritura no bloqueante (el enjambre
    // nunca debe verse ralentizado por un fsync). El antiguo `daemon.log` plano
    // (truncado en cada arranque) se elimina.
    let logs_dir = data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)?;
    let legacy_log = data_dir.join("daemon.log");
    if legacy_log.is_file() {
        let _ = std::fs::remove_file(&legacy_log);
    }

    let timer = tracing_subscriber::fmt::time::OffsetTime::local_rfc_3339().unwrap_or_else(|_| {
        tracing_subscriber::fmt::time::OffsetTime::new(
            time::UtcOffset::UTC,
            time::format_description::well_known::Rfc3339,
        )
    });

    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("daemon")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&logs_dir)
        .map_err(|e| format!("Cannot init log rotation in {}: {e}", logs_dir.display()))?;
    let (file_writer, log_guard) = tracing_appender::non_blocking(file_appender);

    let env_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let log_control = mule_daemon::LogControl::new(env_filter.to_string());
    let (reloadable_filter, filter_reload) = tracing_subscriber::reload::Layer::new(env_filter);

    tracing_subscriber::registry()
        .with(reloadable_filter)
        .with(tracing_subscriber::fmt::layer().with_timer(timer.clone()))
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_ansi(false)
                .with_timer(timer)
                .with_current_span(true)
                .with_writer(file_writer),
        )
        .init();
    log_control.install(filter_reload);
    // La guarda vive hasta el final de `main` — vaciado (flush) del escritor al apagar.
    let _log_guard = log_guard;

    // === Captura de pánicos: crash.log síncrono + rastreo de eventos ===
    // El escritor no bloqueante puede haber muerto cuando ocurra el pánico;
    // la escritura síncrona garantiza que exista una traza utilizable.
    {
        let crash_log = logs_dir.join("crash.log");
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            default_hook(info);
            let stamp = time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| "<time>".to_string());
            let line = format!("{stamp} PANIC: {info}\n");
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&crash_log)
            {
                use std::io::Write;
                let _ = f.write_all(line.as_bytes());
            }
            tracing::error!("{line}");
        }));
    }

    tracing::info!(
        "==> Workdir mode: {:?} ({})",
        resolved_workdir.mode,
        data_dir.display()
    );
    tracing::info!(
        "==> Default download dir: {}",
        default_download_dir.display()
    );

    // 1. Adquirir bloqueo exclusivo del directorio
    let dir_lock = DirectoryLock::acquire(&data_dir).map_err(|e| {
        tracing::error!("==> ERROR: {e}");
        e
    })?;

    let db_path = data_dir.join("mule.db");
    let temp_dir = data_dir.join("temp");
    std::fs::create_dir_all(&temp_dir)?;

    // 2. Resolver token de autenticación
    let auth_token = if let Some(tok) = auth_token_arg {
        DaemonSession::write_token(&data_dir, &tok)?;
        tok
    } else if let Some(token_file) = token_file_arg {
        let tok = std::fs::read_to_string(token_file)?.trim().to_string();
        DaemonSession::write_token(&data_dir, &tok)?;
        tok
    } else if let Ok(tok) = std::env::var("MULE_AUTH_TOKEN") {
        DaemonSession::write_token(&data_dir, &tok)?;
        tok
    } else if let Some(existing) = DaemonSession::read_token(&data_dir) {
        existing
    } else {
        let tok = DaemonSession::generate_token();
        DaemonSession::write_token(&data_dir, &tok)?;
        tracing::info!("==> Generated session auth token stored in auth.token (redacted)");
        tok
    };

    // 3. Resolver dirección de enlace (bind) del socket con alternativa efímera automática si el puerto está ocupado
    let default_bind: std::net::SocketAddr = "127.0.0.1:4660".parse().unwrap();
    let requested_bind = bind_addr_arg
        .or_else(|| {
            std::env::var("MULE_BIND_ADDR")
                .ok()
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(default_bind);

    let listener = match tokio::net::TcpListener::bind(requested_bind).await {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && bind_addr_arg.is_none() => {
            tracing::warn!(
                "==> Port {requested_bind} is in use. Falling back to ephemeral port..."
            );
            let fallback_addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
            tokio::net::TcpListener::bind(fallback_addr).await?
        }
        Err(e) => return Err(e.into()),
    };

    let actual_addr = listener.local_addr()?;
    let endpoint = format!("ws://{actual_addr}/rpc");

    let mut downloads = DownloadService::new(&db_path, temp_dir)?;
    let initial_download_dir = downloads
        .get_setting("default_download_directory")
        .ok()
        .flatten()
        .map(PathBuf::from)
        .unwrap_or(default_download_dir);

    // eMule carga cada .part.met encontrado en el directorio temporal al inicio:
    // los huérfanos (estado de BD perdido o archivos copiados desde eMule) se recuperan
    // como descargas pausadas mediante la ruta de importación aislada.
    let recovered = downloads.recover_orphan_part_mets(&initial_download_dir);
    if recovered > 0 {
        tracing::info!(
            "==> Recovered {recovered} orphaned .part.met download(s) from temp directory"
        );
    }

    let ed2k_port = ed2k_port_arg.unwrap_or(39735);
    let kad_port = kad_port_arg.unwrap_or(39737);
    let upnp_enabled = upnp_arg.unwrap_or(true);
    let network_profile =
        profile_arg.unwrap_or(mule_network_policy::NetworkProfile::PublicEd2kKadPreview);

    // Interfaz de usuario web remota embebida: prevalece `--web-dir`/`MULE_WEB_DIR`, de lo contrario
    // sondea la estructura empaquetada (bin/mule-daemon.exe → <bundle>/web) o
    // recurre a los activos compilados de `web/index.html`.
    let web_dir = resolve_web_dir(web_dir_arg);
    if let Some(dir) = &web_dir {
        tracing::info!("==> Web UI: http://{actual_addr}/ (dir: {})", dir.display());
    } else {
        tracing::info!("==> Web UI embebida: http://{actual_addr}/");
    }

    let config = DaemonConfig::new(data_dir.clone(), auth_token)
        .with_bind_addr(actual_addr)
        .with_workdir_mode(resolved_workdir.mode)
        .with_default_download_dir(initial_download_dir)
        .with_network_profile(network_profile)
        .with_upnp_enabled(upnp_enabled)
        .with_ports(ed2k_port, kad_port)
        .with_web_dir(web_dir);
    let mut state = AppState::new(config, downloads);
    state.log_control = Some(log_control);
    // eMule `Autoconnect`/`Reconnect`/`AutoDeadServer`: observador en segundo plano
    // que se reconecta al último servidor objetivo (o al mejor servidor conocido) cuando
    // los ajustes `server_reconnect`/`server_autoconnect` están habilitados.
    state.spawn_server_watcher();
    // eMuleRust: rotación periódica de identidad (`identityRotationHours`)
    // y cuota diaria de subida (`dailyUploadQuotaGb`).
    state.spawn_identity_rotator();
    state.spawn_quota_watcher();

    // Compartición de la carpeta "Incoming" de eMule: calcula el hash de cada archivo en el
    // directorio de descargas y lo registra como compartido (tarea en segundo plano —
    // el cálculo de hash puede ser lento en archivos grandes). Opción de exclusión mediante el
    // ajuste `share_download_dir`.
    {
        let dl = std::sync::Arc::clone(&state.downloads);
        let dir = state.config.default_download_dir.clone();
        tokio::spawn(async move {
            mule_daemon::services::incoming_share::scan_download_dir_shares(&dl, dir).await;
        });
    }

    // 4. Escribir atómicamente el archivo de sesión
    let session = DaemonSession::new(1, state.instance_id, &endpoint);
    session.write_atomic(&data_dir)?;

    tracing::info!("==> mule-daemon listening on http://{actual_addr}");
    tracing::info!("==> WebSocket RPC endpoint: {endpoint}");
    tracing::info!("==> Network profile: {network_profile}");
    tracing::info!("==> eD2K port: {ed2k_port}, Kad port: {kad_port}, UPnP: {upnp_enabled}");
    tracing::info!(
        "==> Session file: {}",
        data_dir.join("session.json").display()
    );

    // Si UPnP está habilitado y en un perfil de vista previa, intentar el mapeo de puertos en segundo plano con reintentos progresivos y actualización periódica
    let eff_ed2k_port = state.ed2k_port();
    let eff_kad_port = state.kad_port();
    let eff_udp = state.udp_enabled();
    let eff_upnp = state.upnp.snapshot().await.enabled;
    tracing::info!(
        "==> Effective ports: TCP {eff_ed2k_port}, UDP {eff_kad_port} (udp_enabled={eff_udp}, upnp={eff_upnp})"
    );

    if eff_upnp && network_profile.is_preview() {
        let upnp_svc = state.upnp.clone();
        let cancel_token = state.network_policy.cancellation_token();
        let cancel_token_retry = cancel_token.clone();
        tokio::spawn(async move {
            let retry_delays = [
                tokio::time::Duration::from_secs(30),
                tokio::time::Duration::from_secs(120),
                tokio::time::Duration::from_secs(300),
            ];
            let mut result = upnp_svc.map_ports(eff_ed2k_port, eff_kad_port).await;
            if result.is_err() {
                for delay in retry_delays {
                    tokio::select! {
                        _ = cancel_token_retry.cancelled() => break,
                        _ = tokio::time::sleep(delay) => {}
                    }
                    tracing::info!("Retrying UPnP port mapping after initial failure...");
                    result = upnp_svc.map_ports(eff_ed2k_port, eff_kad_port).await;
                    if result.is_ok() {
                        tracing::info!("UPnP port mapping succeeded on background retry!");
                        break;
                    }
                }
            }
        });
        let refresh_handle = state
            .upnp
            .clone()
            .start_periodic_refresh(tokio::time::Duration::from_secs(45 * 60), cancel_token);
        state.register_task(refresh_handle.abort_handle()).await;
    }

    // Auto-inicializar tabla de enrutamiento Kad y socket UDP si se está en un perfil de vista previa
    if network_profile.allows_udp_kad() && eff_udp {
        let kad_svc = state.kad.clone();
        let kad_cancel = state.network_policy.cancellation_token();
        let kad_bind_ip = if network_profile.is_loopback_only() {
            "127.0.0.1"
        } else {
            "0.0.0.0"
        };
        let kad_bind_addr: std::net::SocketAddr =
            format!("{kad_bind_ip}:{eff_kad_port}").parse().unwrap();
        let nodes_dat_path = state.config.data_dir.join("nodes.dat");
        tokio::spawn(async move {
            kad_svc.init_from_db().await;
            if let Err(e) = kad_svc.udp().start(kad_bind_addr).await {
                tracing::warn!("Failed to auto-start Kad UDP on {kad_bind_addr}: {e}");
            } else {
                tracing::info!("Kad UDP socket auto-started on {kad_bind_addr}");
                // nodes.dat en la raíz de la carpeta data: arranque (bootstrap)
                // opcional — eMule lo lee en `s_liBootstrapList` en
                // cada inicio; los contactos demuestran su disponibilidad
                // respondiendo al BOOTSTRAP_REQ del vaciado.
                kad_svc.load_nodes_dat_candidates(&nodes_dat_path).await;
                kad_svc.bootstrap_active_contacts(15).await;
                kad_svc.start_background_maintenance(kad_cancel);
            }
        });

        // Socket UDP de servidor (OP_GLOBGETSOURCES2 / OP_GLOBFOUNDSOURCES) —
        // socket efímero separado como el `UDPSocket` de eMule.
        let server_udp_svc = state.server_udp.clone();
        let server_udp_bind_ip: std::net::IpAddr = kad_bind_ip.parse().unwrap();
        tokio::spawn(async move {
            match server_udp_svc.start(server_udp_bind_ip).await {
                Ok(bound) => tracing::info!("Server UDP socket auto-started on {bound}"),
                Err(e) => tracing::warn!("Failed to auto-start server UDP: {e}"),
            }
        });

        // Transporte QUIC P2P autenticado por Ed25519 sobre UDP.
        let quic_state = state.clone();
        tokio::spawn(async move {
            let _ = quic_state.start_quic_transport().await;
        });
    }

    // Auto-iniciar oyente TCP entrante en el puerto eD2K si se está en un perfil de vista previa o habilitado
    if network_profile.is_preview() || network_profile.is_loopback_only() {
        let listener_svc = state.inbound_listener.clone();
        let inbound_bind_ip = if network_profile.is_loopback_only() {
            "127.0.0.1"
        } else {
            "0.0.0.0"
        };
        let inbound_bind_addr: std::net::SocketAddr = format!("{inbound_bind_ip}:{eff_ed2k_port}")
            .parse()
            .unwrap();
        tokio::spawn(async move {
            match listener_svc.start(inbound_bind_addr).await {
                Ok(bound) => {
                    tracing::info!("==> Inbound eD2K TCP listener auto-started on {bound}");
                }
                Err(e) => {
                    tracing::warn!(
                        "==> Failed to auto-start Inbound TCP listener on {inbound_bind_addr}: {e}"
                    );
                }
            }
        });
    }

    // USS (Upload Speed Sense) — limitador dinámico de subida guiado por sondas de
    // latencia al servidor conectado (inactivo mientras esté deshabilitado).
    state.uss.start(
        state.network_policy.rate_limiters(),
        state.network_policy.cancellation_token(),
    );

    // Auto-iniciar el coordinador autónomo de enjambre para el descubrimiento de fuentes y conexiones de pares en segundo plano
    let coordinator_handle = mule_daemon::services::SwarmCoordinator::start(state.clone());
    state.register_task(coordinator_handle.abort_handle()).await;

    // Auto-iniciar automatización multimedia autónoma (organizador + subtítulos Bazarr + auto-descarga)
    let media_automation_handle = mule_daemon::services::MediaAutomation::start(state.clone());
    state.register_task(media_automation_handle.abort_handle()).await;

    let shutdown_notify = state.shutdown_notify.clone();
    let upnp_cleanup = state.upnp.clone();
    let listener_cleanup = state.inbound_listener.clone();
    let downloads_cleanup = state.downloads.clone();
    let updater_cleanup = state.updater.clone();
    let tray_downloads_dir = state.config.default_download_dir.clone();
    let tray_event_rx = if !no_tray_arg {
        Some(state.event_tx.subscribe())
    } else {
        None
    };
    let app = create_router(state);

    if let Some(parent_pid) = parent_pid_arg {
        if parent_pid > 0 {
            spawn_parent_watchdog(parent_pid, shutdown_notify.clone());
        }
    }

    let tray_handle = if !no_tray_arg {
        let tray_host = if actual_addr.ip().is_unspecified() {
            "127.0.0.1".to_string()
        } else {
            actual_addr.ip().to_string()
        };
        Some(mule_daemon::tray::spawn(mule_daemon::tray::TrayConfig {
            web_url: format!("http://{tray_host}:{}/", actual_addr.port()),
            downloads_dir: tray_downloads_dir,
            data_dir: data_dir.clone(),
            shutdown_notify: shutdown_notify.clone(),
            event_rx: tray_event_rx,
        }))
    } else {
        None
    };

    // 5. Ejecutar con apagado ordenado (graceful shutdown)
    let data_dir_cleanup = data_dir.clone();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = async {
                    if tokio::signal::ctrl_c().await.is_err() {
                        std::future::pending::<()>().await;
                    }
                } => {
                    tracing::info!("==> Shutting down mule-daemon via Ctrl+C...");
                }
                _ = shutdown_notify.notified() => {
                    tracing::info!("==> Shutting down mule-daemon via shutdown signal/watchdog...");
                }
            }
            DaemonSession::remove(&data_dir_cleanup);
            listener_cleanup.stop().await;
            // eMule guarda cada .part.met al salir para que el estado de reanudación
            // nunca quede obsoleto (CPartFile::~CPartFile → SavePartFile).
            {
                let mut dl = downloads_cleanup.lock().await;
                dl.save_active_part_mets();
            }
            // Mejor esfuerzo acotado a 500 ms: las llamadas SOAP hacia la puerta de enlace
            // se ejecutan en un hilo independiente — un router lento o ausente nunca
            // debe retrasar la detención del proceso (Tokio esperaría a spawn_blocking
            // al salir).
            upnp_cleanup.unmap_ports_for_shutdown().await;
        })
        .await?;

    if let Some(handle) = tray_handle {
        handle.stop();
    }

    DaemonSession::remove(&data_dir);
    drop(dir_lock);
    tracing::info!("==> mule-daemon exited cleanly.");

    // Si se aplicó una actualización con reinicio solicitado, coordinar relanzamiento según entorno
    if updater_cleanup.is_restart_pending() {
        if mule_daemon::services::updater::is_managed_by_systemd() {
            tracing::info!(
                "==> Proceso gestionado por systemd: saliendo de forma ordenada para que systemd relance el binario actualizado..."
            );
        } else {
            tracing::info!("==> Relanzando proceso eMuleRust con la nueva versión actualizada...");
            if let Err(e) = mule_daemon::services::updater::respawn_current_process() {
                tracing::error!("Error al relanzar el proceso actualizado: {e}");
            }
        }
    }

    Ok(())
}

#[cfg(windows)]
fn spawn_parent_watchdog(parent_pid: u32, shutdown_notify: std::sync::Arc<tokio::sync::Notify>) {
    tokio::task::spawn_blocking(move || {
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenProcess(
                dwDesiredAccess: u32,
                bInheritHandle: i32,
                dwProcessId: u32,
            ) -> *mut std::ffi::c_void;
            fn WaitForSingleObject(hHandle: *mut std::ffi::c_void, dwMilliseconds: u32) -> u32;
            fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
        }

        const SYNCHRONIZE: u32 = 0x00100000;
        let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, parent_pid) };
        if handle.is_null() {
            tracing::warn!(
                "==> Parent process PID {parent_pid} not running or inaccessible; initiating exit."
            );
            shutdown_notify.notify_waiters();
            return;
        }
        tracing::info!("==> Watching parent process PID {parent_pid} for auto-teardown...");
        unsafe {
            WaitForSingleObject(handle, 0xFFFFFFFF);
            CloseHandle(handle);
        }
        tracing::info!("==> Parent process PID {parent_pid} terminated. Initiating graceful daemon shutdown...");
        shutdown_notify.notify_waiters();
    });
}

#[cfg(not(windows))]
fn spawn_parent_watchdog(parent_pid: u32, shutdown_notify: std::sync::Arc<tokio::sync::Notify>) {
    tokio::spawn(async move {
        tracing::info!("==> Watching parent process PID {parent_pid} for auto-teardown...");
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            let exists = std::path::Path::new(&format!("/proc/{parent_pid}")).exists();
            if !exists {
                tracing::info!("==> Parent process PID {parent_pid} terminated. Initiating graceful daemon shutdown...");
                shutdown_notify.notify_waiters();
                break;
            }
        }
    });
}

/// Resuelve el directorio de la interfaz de usuario web estática. Un `--web-dir` /
/// `MULE_WEB_DIR` explícito debe contener `index.html` (advierte en caso contrario); a falta de ello,
/// sondea ubicaciones relativas al ejecutable: `<exe>/../web` (paquete
/// empaquetado `bin/` → `web/`), `<exe>/web` y `../../web`
/// (compilaciones de desarrollo con cargo en `target/{debug,release}`).
fn resolve_web_dir(explicit: Option<PathBuf>) -> Option<PathBuf> {
    let candidate = explicit.or_else(|| std::env::var("MULE_WEB_DIR").ok().map(PathBuf::from));
    if let Some(dir) = candidate {
        if dir.join("index.html").is_file() {
            return Some(dir);
        }
        tracing::warn!(
            "==> Web dir '{}' has no index.html — web UI disabled",
            dir.display()
        );
        return None;
    }
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    for rel in ["../web", "web", "../../web"] {
        let dir = exe_dir.join(rel);
        if dir.join("index.html").is_file() {
            return Some(dir);
        }
    }
    None
}

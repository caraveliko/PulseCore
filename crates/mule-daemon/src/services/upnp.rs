// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Servicio UPnP IGD (Internet Gateway Device).
//!
//! Descubre automáticamente los routers/puertas de enlace con UPnP habilitado
//! en la red local, resuelve la IP WAN externa y gestiona las concesiones de
//! mapeo de puertos para eD2K TCP y Kad UDP.

pub mod discovery;
pub mod ipv6_pinhole;
pub mod mapping;
pub mod pcp_natpmp;
pub mod types;

use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use igd_next::{Gateway, PortMappingProtocol};
use tokio::sync::RwLock;
use tracing::{info, warn};

use discovery::discover_gateway_with_fallback;
use mapping::{add_port_safe, check_existing_mapping};
pub use types::{
    current_unix_ms, detect_local_ipv4_for_gateway, detect_primary_local_ipv4,
    inspect_mapping_entry, is_cgnat_ip, is_valid_gateway_addr, is_valid_wan_ip, MappingOwnership,
    UpnpStatusSnapshot,
};

/// Servicio que gestiona el ciclo de vida de UPnP IGD.
#[derive(Debug, Clone)]
pub struct UpnpService {
    snapshot: Arc<RwLock<UpnpStatusSnapshot>>,
    cached_gateway: Arc<RwLock<Option<Gateway>>>,
}

impl UpnpService {
    /// Crea un nuevo servicio UPnP.
    pub fn new(enabled: bool, tcp_port: u16, udp_port: u16) -> Self {
        let snap = UpnpStatusSnapshot {
            enabled,
            tcp_port,
            udp_port,
            ..Default::default()
        };
        Self {
            snapshot: Arc::new(RwLock::new(snap)),
            cached_gateway: Arc::new(RwLock::new(None)),
        }
    }

    /// Devuelve una instantánea del estado actual.
    pub async fn snapshot(&self) -> UpnpStatusSnapshot {
        self.snapshot.read().await.clone()
    }

    /// Actualiza los puertos TCP/UDP mapeados (cuando los puertos del cliente eMule
    /// cambian en tiempo de ejecución) para que las renovaciones y remapeos utilicen los valores actuales.
    pub async fn set_ports(&self, tcp_port: u16, udp_port: u16) {
        let mut snap = self.snapshot.write().await;
        snap.tcp_port = tcp_port;
        snap.udp_port = udp_port;
    }

    /// Habilita o deshabilita UPnP.
    pub async fn set_enabled(&self, enabled: bool) {
        let mut snap = self.snapshot.write().await;
        snap.enabled = enabled;
        snap.updated_at_unix_ms = current_unix_ms();
    }

    /// Genera una tarea en segundo plano que renueva periódicamente los mapeos de puertos UPnP antes de que expiren las concesiones.
    pub fn start_periodic_refresh(
        self: Arc<Self>,
        interval: Duration,
        cancel_token: mule_network_policy::CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await;
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        info!("UPnP periodic refresh loop cancelled");
                        break;
                    }
                    _ = ticker.tick() => {
                        let (enabled, tcp_port, udp_port) = {
                            let snap = self.snapshot.read().await;
                            (snap.enabled, snap.tcp_port, snap.udp_port)
                        };
                        if enabled {
                            info!("Renewing UPnP port mapping lease (TCP {tcp_port}, UDP {udp_port})");
                            let _ = self.map_ports(tcp_port, udp_port).await;
                        }
                    }
                }
            }
        })
    }

    /// Descubre la puerta de enlace y mapea los puertos.
    pub async fn map_ports(
        &self,
        tcp_port: u16,
        udp_port: u16,
    ) -> Result<UpnpStatusSnapshot, String> {
        {
            let mut snap = self.snapshot.write().await;
            snap.tcp_port = tcp_port;
            snap.udp_port = udp_port;
            snap.last_error = None;
            snap.updated_at_unix_ms = current_unix_ms();
        }

        const MAX_ATTEMPTS: u32 = 3;
        let mut last_err = String::new();

        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                let delay_secs = 2u64 << (attempt - 1);
                warn!(
                    "UPnP: attempt {}/{MAX_ATTEMPTS} failed, retrying in {delay_secs}s (cause: {last_err})",
                    attempt
                );
                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
            }

            let cached_gw = self.cached_gateway.read().await.clone();

            let join_handle = tokio::task::spawn_blocking(
                move || -> Result<(Gateway, IpAddr, Ipv4Addr, bool, bool, bool), String> {
                    let gateway = match cached_gw {
                        Some(gw) => gw,
                        None => discover_gateway_with_fallback()?,
                    };

                    if !is_valid_gateway_addr(gateway.addr) {
                        return Err(format!(
                            "Discovered UPnP gateway has invalid non-LAN address: {}",
                            gateway.addr
                        ));
                    }

                    let ext_ip = gateway
                        .get_external_ip()
                        .map_err(|e| format!("Failed to retrieve external WAN IP: {e}"))?;

                    let cgnat = is_cgnat_ip(ext_ip);

                    if !is_valid_wan_ip(ext_ip) && !cgnat {
                        return Err(format!(
                            "Gateway reported invalid non-public WAN IP: {ext_ip}"
                        ));
                    }

                    if cgnat {
                        warn!(
                            "UPnP: CGNAT detected — WAN IP {ext_ip} is in the RFC 6598 shared address space. eD2K/Kad internet connectivity may be limited."
                        );
                    }

                    let local_ip =
                        detect_local_ipv4_for_gateway(gateway.addr).ok_or_else(|| {
                            "Failed to detect local IPv4 address for gateway".to_string()
                        })?;

                    let local_tcp_addr = SocketAddrV4::new(local_ip, tcp_port);
                    let local_udp_addr = SocketAddrV4::new(local_ip, udp_port);

                    let tcp_ok = add_port_safe(
                        &gateway,
                        PortMappingProtocol::TCP,
                        tcp_port,
                        local_tcp_addr,
                        3600,
                        "eMuleRust eD2K TCP",
                    );

                    let udp_ok = add_port_safe(
                        &gateway,
                        PortMappingProtocol::UDP,
                        udp_port,
                        local_udp_addr,
                        3600,
                        "eMuleRust Kad UDP",
                    );

                    if !tcp_ok && !udp_ok {
                        return Err(format!(
                            "Failed to map both TCP {tcp_port} and UDP {udp_port} ports on gateway"
                        ));
                    }

                    Ok((gateway, ext_ip, local_ip, tcp_ok, udp_ok, cgnat))
                },
            );

            let res = match tokio::time::timeout(Duration::from_secs(15), join_handle).await {
                Ok(Ok(inner_res)) => inner_res,
                Ok(Err(join_err)) => Err(format!("UPnP task join error: {join_err}")),
                Err(_) => Err("UPnP gateway discovery timed out after 15 seconds".to_string()),
            };

            match res {
                Ok((gw, ext_ip, _local_ip, tcp_ok, udp_ok, cgnat)) => {
                    let mut pinhole_id: Option<String> = None;
                    if gw.control_schema.contains_key("AddPinhole") {
                        if let Some(local_v6) = ipv6_pinhole::detect_local_global_ipv6() {
                            let ctl_url = format!("http://{}{}", gw.addr, gw.control_url);
                            if let Ok(id) = ipv6_pinhole::add_ipv6_pinhole(
                                &ipv6_pinhole::Ipv6PinholeRequest {
                                    gateway_control_url: ctl_url,
                                    internal_client: local_v6,
                                    internal_port: tcp_port,
                                    protocol: 6,
                                    lease_secs: ipv6_pinhole::DEFAULT_PINHOLE_LEASE_SECS,
                                },
                            )
                            .await
                            {
                                pinhole_id = Some(id);
                            }
                        }
                    }

                    let mut snap = self.snapshot.write().await;
                    snap.available = true;
                    snap.gateway_addr = Some(gw.addr.to_string());
                    snap.external_ip = Some(ext_ip.to_string());
                    snap.tcp_mapped = tcp_ok;
                    snap.udp_mapped = udp_ok;
                    snap.cgnat_detected = cgnat;
                    snap.mapping_protocol = Some("upnp-igd".to_string());
                    snap.ipv6_pinhole_mapped = pinhole_id.is_some();
                    snap.ipv6_pinhole_id = pinhole_id;
                    snap.last_error = if cgnat {
                        Some(
                            "CGNAT detectado: su operador utiliza un NAT compartido (100.64.x.x). Los puertos están mapeados localmente, pero la conectividad eD2K/Kad desde internet puede estar limitada."
                                .to_string(),
                        )
                    } else if !tcp_ok || !udp_ok {
                        Some("Mapeo UPnP parcial: algunos puertos no pudieron mapearse".to_string())
                    } else {
                        None
                    };
                    snap.updated_at_unix_ms = current_unix_ms();
                    *self.cached_gateway.write().await = Some(gw);
                    info!(
                        "UPnP setup succeeded on attempt {}/{MAX_ATTEMPTS}: Gateway {}, External IP {}, TCP: {}, UDP: {}, CGNAT: {}",
                        attempt + 1,
                        snap.gateway_addr.as_deref().unwrap_or(""),
                        ext_ip,
                        tcp_ok,
                        udp_ok,
                        cgnat,
                    );
                    return Ok(snap.clone());
                }
                Err(e) => {
                    last_err = e;
                    *self.cached_gateway.write().await = None;
                }
            }
        }

        // Respaldo cascada PCP (RFC 6887) -> NAT-PMP (RFC 6886) si el router no ofrece UPnP IGD.
        if let Some(gw_v4) = pcp_natpmp::detect_default_gateway_ipv4() {
            let gw_sock = std::net::SocketAddr::V4(SocketAddrV4::new(
                gw_v4,
                pcp_natpmp::PCP_NATPMP_PORT,
            ));
            if is_valid_gateway_addr(gw_sock) {
                if let Some(local_v4) = detect_local_ipv4_for_gateway(gw_sock) {
                    if let Ok(cascade) = pcp_natpmp::map_ports_cascade_at(
                        gw_sock,
                        IpAddr::V4(local_v4),
                        tcp_port,
                        udp_port,
                    )
                    .await
                    {
                        let mut snap = self.snapshot.write().await;
                        snap.available = true;
                        snap.gateway_addr = Some(gw_sock.to_string());
                        snap.external_ip = cascade.external_ip.map(|ip| ip.to_string());
                        snap.tcp_mapped = cascade.tcp_mapped;
                        snap.udp_mapped = cascade.udp_mapped;
                        snap.cgnat_detected = cascade.external_ip.is_some_and(is_cgnat_ip);
                        snap.mapping_protocol = Some(cascade.protocol.to_string());
                        snap.ipv6_pinhole_mapped = false;
                        snap.ipv6_pinhole_id = None;
                        snap.last_error = if snap.cgnat_detected {
                            Some("CGNAT detectado en puerta de enlace PCP/NAT-PMP".to_string())
                        } else {
                            None
                        };
                        snap.updated_at_unix_ms = current_unix_ms();
                        info!(
                            "Port mapping succeeded via {} fallback on {gw_sock} (TCP: {}, UDP: {})",
                            cascade.protocol, cascade.tcp_mapped, cascade.udp_mapped
                        );
                        return Ok(snap.clone());
                    }
                }
            }
        }

        let mut snap = self.snapshot.write().await;
        snap.available = false;
        snap.cgnat_detected = false;
        snap.tcp_mapped = false;
        snap.udp_mapped = false;
        snap.mapping_protocol = None;
        snap.ipv6_pinhole_mapped = false;
        snap.ipv6_pinhole_id = None;
        snap.last_error = Some(format!("{last_err} (tras {MAX_ATTEMPTS} intentos)"));
        snap.updated_at_unix_ms = current_unix_ms();
        warn!("UPnP setup failed after {MAX_ATTEMPTS} attempts: {last_err}");
        Err(last_err)
    }

    /// Elimina de la puerta de enlace los puertos mapeados previamente si se verifica que todavía pertenecen a eMuleRust.
    pub async fn unmap_ports(&self) -> Result<(), String> {
        let (gw_opt, tcp_port, udp_port, tcp_mapped, udp_mapped) = {
            let snap = self.snapshot.read().await;
            let gw = self.cached_gateway.read().await.clone();
            (
                gw,
                snap.tcp_port,
                snap.udp_port,
                snap.tcp_mapped,
                snap.udp_mapped,
            )
        };

        if !tcp_mapped && !udp_mapped {
            return Ok(());
        }

        if let Some(gateway) = gw_opt {
            tokio::task::spawn_blocking(move || {
                unmap_gateway_blocking(&gateway, tcp_port, udp_port, tcp_mapped, udp_mapped)
            })
            .await
            .map_err(|e| format!("UPnP unmap task join error: {e}"))?;
        }

        let mut snap = self.snapshot.write().await;
        snap.tcp_mapped = false;
        snap.udp_mapped = false;
        snap.updated_at_unix_ms = current_unix_ms();
        Ok(())
    }

    /// Desmapeo de mejor esfuerzo para el cierre del proceso, acotado estrictamente a 500 ms.
    ///
    /// Las llamadas SOAP a la puerta de enlace se ejecutan en un `std::thread` desacoplado,
    /// nunca en `spawn_blocking`: el runtime de Tokio espera a todos los hilos del grupo
    /// bloqueante durante el apagado, por lo que un `spawn_blocking` que siga ejecutando una
    /// llamada lenta a la puerta de enlace mantendría el proceso activo más allá de la salida
    /// (haciendo ineficaz el antiguo `timeout()` externo). Un hilo desacoplado se descarta
    /// con el proceso, y el canal oneshot nos permite esperar su finalización sin ocupar
    /// una ranura del grupo bloqueante.
    pub async fn unmap_ports_for_shutdown(&self) {
        const SHUTDOWN_UNMAP_BUDGET: Duration = Duration::from_millis(500);

        let (gw_opt, tcp_port, udp_port, tcp_mapped, udp_mapped) = {
            let snap = self.snapshot.read().await;
            let gw = self.cached_gateway.read().await.clone();
            (
                gw,
                snap.tcp_port,
                snap.udp_port,
                snap.tcp_mapped,
                snap.udp_mapped,
            )
        };

        if !tcp_mapped && !udp_mapped {
            return;
        }

        if let Some(gateway) = gw_opt {
            let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
            std::thread::spawn(move || {
                unmap_gateway_blocking(&gateway, tcp_port, udp_port, tcp_mapped, udp_mapped);
                let _ = done_tx.send(());
            });
            let _ = tokio::time::timeout(SHUTDOWN_UNMAP_BUDGET, done_rx).await;
        }
    }
}

/// Comprobación de pertenencia SOAP bloqueante + eliminación para ambas rutas de desmapeo.
/// Se ejecuta en un hilo bloqueante (`spawn_blocking` o `std::thread` desacoplado) —
/// nunca en un hilo ejecutor asíncrono.
fn unmap_gateway_blocking(
    gateway: &Gateway,
    tcp_port: u16,
    udp_port: u16,
    tcp_mapped: bool,
    udp_mapped: bool,
) {
    let local_ip_opt = detect_local_ipv4_for_gateway(gateway.addr);

    for (proto, port, mapped) in [
        (PortMappingProtocol::TCP, tcp_port, tcp_mapped),
        (PortMappingProtocol::UDP, udp_port, udp_mapped),
    ] {
        if !mapped {
            continue;
        }

        let should_remove = match local_ip_opt {
            Some(lip) => match check_existing_mapping(
                gateway,
                proto,
                port,
                SocketAddrV4::new(lip, port),
                "eMuleRust",
            ) {
                MappingOwnership::ExactMatch => true,
                MappingOwnership::StaleLease => {
                    warn!("Skipped unmapping {proto:?} port {port}: entry is an old/stale lease from another IP or port; letting it expire naturally");
                    false
                }
                MappingOwnership::ForeignConflict => {
                    warn!("Skipped unmapping {proto:?} port {port}: entry belongs to another application/host");
                    false
                }
                MappingOwnership::NotFound => false,
            },
            None => {
                warn!("Skipped unmapping {proto:?} port {port}: unable to determine local endpoint for ownership verification");
                false
            }
        };

        if should_remove {
            let _ = gateway.remove_port(proto, port);
            info!("UPnP unmapped {proto:?} port {port}");
        }
    }
}

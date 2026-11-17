// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use thiserror::Error;

use crate::audit::{AuditDecision, AuditEventType, NetworkAuditEntry, NetworkAuditLog};
use crate::budgets::ResourceBudgets;
use crate::cancellation::CancellationToken;
use crate::permit::{ConnectionPurpose, ListenPermit, PermitTracker, SocketPermit};
use crate::profile::NetworkProfile;
use crate::state::NetworkState;
use crate::validator::{validate_connect_address, validate_listen_address, PolicyViolation};

use mule_filter::{IpFilter, RateLimitsConfig, SessionPseudonymizer, TokenBucket};

/// Errores encontrados al solicitar permisos de red o actualizar la política.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PolicyError {
    #[error("Network is disabled (Kill Switch active)")]
    NetworkDisabled,

    #[error("Address policy violation: {0}")]
    AddressViolation(#[from] PolicyViolation),

    #[error("IP address {0} is blocked by IP filter (level {1}: {2})")]
    IpFiltered(String, u32, String),

    #[error("Connection rate limit exceeded")]
    ConnectionRateLimited,

    #[error("Resource budget exhausted: {0}")]
    BudgetExhausted(&'static str),

    #[error("Invalid budget configuration: {0}")]
    InvalidBudgets(&'static str),

    #[error("Kademlia UDP operations not permitted in profile {0}")]
    KadNotAllowedInProfile(NetworkProfile),

    #[error("Network subsystem in shutting down state")]
    ShuttingDown,
}

#[derive(Debug)]
struct PolicyManagerInner {
    profile: NetworkProfile,
    state: NetworkState,
    budgets: ResourceBudgets,
    audit_log: NetworkAuditLog,
    next_permit_id: u64,
    cancel_token: CancellationToken,
}

/// Limitadores de tasa administrados por la política.
pub struct PolicyRateLimiters {
    pub download_bucket: TokenBucket,
    pub upload_bucket: TokenBucket,
    pub conn_bucket: TokenBucket,
    pub server_bucket: TokenBucket,
    pub kad_bucket: TokenBucket,
}

impl PolicyRateLimiters {
    pub fn new(cfg: &RateLimitsConfig) -> Self {
        // connections_per_minute -> tokens por segundo: cfg.connections_per_minute / 60
        let conn_per_sec = (cfg.connections_per_minute as u64 / 60).max(1);
        let conn_burst = (cfg.connections_per_minute as u64 / 10).max(5);

        Self {
            download_bucket: TokenBucket::new(
                cfg.download_bytes_per_sec,
                cfg.download_bytes_per_sec.max(65536),
            ),
            upload_bucket: TokenBucket::new(
                cfg.upload_bytes_per_sec,
                cfg.upload_bytes_per_sec.max(65536),
            ),
            conn_bucket: TokenBucket::new(conn_per_sec, conn_burst),
            server_bucket: TokenBucket::new(
                cfg.server_requests_per_sec as u64,
                (cfg.server_requests_per_sec * 2).max(10) as u64,
            ),
            kad_bucket: TokenBucket::new(
                cfg.kad_packets_per_sec as u64,
                (cfg.kad_packets_per_sec * 2).max(50) as u64,
            ),
        }
    }

    pub fn apply_config(&self, cfg: &RateLimitsConfig) {
        self.download_bucket.set_rate(
            cfg.download_bytes_per_sec,
            cfg.download_bytes_per_sec.max(65536),
        );
        self.upload_bucket.set_rate(
            cfg.upload_bytes_per_sec,
            cfg.upload_bytes_per_sec.max(65536),
        );
        let conn_per_sec = (cfg.connections_per_minute as u64 / 60).max(1);
        let conn_burst = (cfg.connections_per_minute as u64 / 10).max(5);
        self.conn_bucket.set_rate(conn_per_sec, conn_burst);
        self.server_bucket.set_rate(
            cfg.server_requests_per_sec as u64,
            (cfg.server_requests_per_sec * 2).max(10) as u64,
        );
        self.kad_bucket.set_rate(
            cfg.kad_packets_per_sec as u64,
            (cfg.kad_packets_per_sec * 2).max(50) as u64,
        );
    }
}

/// Coordinador central de políticas de red para eMuleRust.
/// Punto de estrangulamiento único obligatorio para validar direcciones, emitir permisos RAII,
/// hacer cumplir los presupuestos de recursos y activar la desconexión global atómica.
#[derive(Clone)]
pub struct NetworkPolicyManager {
    inner: Arc<RwLock<PolicyManagerInner>>,
    tracker: Arc<PermitTracker>,
    filter: Arc<IpFilter>,
    rate_limiters: Arc<PolicyRateLimiters>,
    pseudonymizer: Arc<SessionPseudonymizer>,
}

impl NetworkPolicyManager {
    /// Crea un nuevo administrador con el perfil inicial y los presupuestos de recursos especificados.
    pub fn new(profile: NetworkProfile, budgets: ResourceBudgets) -> Self {
        let rate_limits_cfg = RateLimitsConfig::default();
        Self {
            inner: Arc::new(RwLock::new(PolicyManagerInner {
                profile,
                state: NetworkState::PreviewActive,
                budgets,
                audit_log: NetworkAuditLog::new(500),
                next_permit_id: 1,
                cancel_token: CancellationToken::new(),
            })),
            tracker: Arc::new(PermitTracker::default()),
            filter: Arc::new(IpFilter::new()),
            rate_limiters: Arc::new(PolicyRateLimiters::new(&rate_limits_cfg)),
            pseudonymizer: Arc::new(SessionPseudonymizer::new()),
        }
    }

    /// Accede al filtro IP subyacente.
    pub fn ip_filter(&self) -> Arc<IpFilter> {
        self.filter.clone()
    }

    /// Accede a los limitadores de tasa subyacentes.
    pub fn rate_limiters(&self) -> Arc<PolicyRateLimiters> {
        self.rate_limiters.clone()
    }

    /// Accede al seudonimizador de sesión subyacente.
    pub fn pseudonymizer(&self) -> Arc<SessionPseudonymizer> {
        self.pseudonymizer.clone()
    }

    /// Devuelve el perfil de red actualmente activo.
    pub fn profile(&self) -> NetworkProfile {
        self.inner.read().unwrap().profile
    }

    /// Actualiza el perfil de red, registrando la transición en el registro de auditoría.
    /// Invariante: si se regresa a `local-lab` con sockets externos activos,
    /// dichos sockets se cancelan atómicamente a través del token de cancelación.
    pub fn set_profile(&self, new_profile: NetworkProfile) {
        let mut inner = self.inner.write().unwrap();
        let old = inner.profile;
        if old != new_profile {
            inner.profile = new_profile;
            inner.audit_log.record(
                AuditEventType::ProfileChanged,
                None,
                None,
                new_profile,
                AuditDecision::Allowed,
                Some(format!("Switched profile from {old} to {new_profile}")),
            );
            tracing::info!(from = %old, to = %new_profile, "Network profile updated");

            // Invariante: si se cambia a local-lab, cancelar los permisos activos permanentemente e instanciar un nuevo token
            if new_profile == NetworkProfile::LocalLab {
                inner.cancel_token.cancel();
                inner.cancel_token = CancellationToken::new();
            }
        }
    }

    /// Devuelve el estado de red dinámico actual.
    pub fn state(&self) -> NetworkState {
        let inner = self.inner.read().unwrap();
        match inner.state {
            NetworkState::Disabled => NetworkState::Disabled,
            NetworkState::ShuttingDown => NetworkState::ShuttingDown,
            NetworkState::Error => NetworkState::Error,
            NetworkState::PreviewActive | NetworkState::Connected => {
                let total_active = self.tracker.active_peer_conns.load(Ordering::SeqCst)
                    + self.tracker.active_server_conns.load(Ordering::SeqCst);
                if total_active > 0 {
                    NetworkState::Connected
                } else {
                    NetworkState::PreviewActive
                }
            }
        }
    }

    /// Devuelve los presupuestos de recursos configurados.
    pub fn budgets(&self) -> ResourceBudgets {
        self.inner.read().unwrap().budgets.clone()
    }

    /// Actualiza los presupuestos de recursos tras validar los límites de los parámetros.
    pub fn set_budgets(&self, budgets: ResourceBudgets) -> Result<(), PolicyError> {
        budgets.validate().map_err(PolicyError::InvalidBudgets)?;
        let mut inner = self.inner.write().unwrap();
        inner.budgets = budgets;
        Ok(())
    }

    /// Solicita un permiso de socket TCP saliente para una dirección de destino y propósito.
    /// Falla si la red está deshabilitada, la dirección infringe las restricciones del perfil,
    /// la IP está bloqueada por el filtro IP, se superan los límites de tasa de conexión,
    /// o se agotan los presupuestos de conexiones activas.
    pub fn request_tcp_connect(
        &self,
        addr: SocketAddr,
        purpose: ConnectionPurpose,
    ) -> Result<SocketPermit, PolicyError> {
        let mut inner = self.inner.write().unwrap();
        let profile = inner.profile;

        let pseudo = match purpose {
            ConnectionPurpose::Ed2kServer => self.pseudonymizer.pseudonymize_server(addr),
            ConnectionPurpose::Ed2kPeer | ConnectionPurpose::KadNode => {
                self.pseudonymizer.pseudonymize_peer(addr)
            }
            ConnectionPurpose::Listener => format!("listener:{}", addr.port()),
        };

        // 1. Comprobar el estado del ciclo de vida
        if inner.state == NetworkState::Disabled {
            inner.audit_log.record(
                AuditEventType::ConnectAttempt,
                Some(&addr),
                Some(pseudo),
                profile,
                AuditDecision::Denied,
                Some("Network disabled by Kill Switch".to_string()),
            );
            return Err(PolicyError::NetworkDisabled);
        }
        if inner.state == NetworkState::ShuttingDown {
            return Err(PolicyError::ShuttingDown);
        }

        // 2. Comprobar capacidad protocolaria (Kademlia UDP en vista previa)
        if purpose == ConnectionPurpose::KadNode && !profile.allows_udp_kad() {
            let reason = format!("Kademlia UDP not allowed in profile {profile}");
            inner.audit_log.record(
                AuditEventType::ConnectAttempt,
                Some(&addr),
                Some(pseudo),
                profile,
                AuditDecision::Denied,
                Some(reason),
            );
            return Err(PolicyError::KadNotAllowedInProfile(profile));
        }

        // 3. Comprobar filtro IP (búsqueda en memoria de alto rendimiento)
        if let Some(range) = self.filter.check_ip(addr.ip()) {
            let reason = format!(
                "Blocked by IP filter (level {}: '{}' - [{} - {}])",
                range.level,
                range.description,
                range.start_ip(),
                range.end_ip()
            );
            inner.audit_log.record(
                AuditEventType::FilterBlocked,
                Some(&addr),
                Some(pseudo),
                profile,
                AuditDecision::Denied,
                Some(reason.clone()),
            );
            tracing::warn!(target = %addr, level = range.level, desc = %range.description, "Connection blocked by IP filter");
            return Err(PolicyError::IpFiltered(
                addr.ip().to_string(),
                range.level,
                range.description.to_string(),
            ));
        }

        // 4. Comprobar limitador de tasa de conexión (Token Bucket)
        let rate_allowed = match purpose {
            ConnectionPurpose::KadNode => self.rate_limiters.kad_bucket.try_acquire(1),
            _ => self.rate_limiters.conn_bucket.try_acquire(1),
        };
        if !rate_allowed {
            let reason = match purpose {
                ConnectionPurpose::KadNode => "Kad UDP packet rate limit exceeded".to_string(),
                _ => "Connection creation rate limit exceeded".to_string(),
            };
            inner.audit_log.record(
                AuditEventType::BudgetExhausted,
                Some(&addr),
                Some(pseudo),
                profile,
                AuditDecision::Denied,
                Some(reason),
            );
            return Err(PolicyError::ConnectionRateLimited);
        }

        // 5. Validar dirección de destino
        if let Err(violation) = validate_connect_address(&addr, profile) {
            let reason = violation.to_string();
            inner.audit_log.record(
                AuditEventType::ConnectAttempt,
                Some(&addr),
                Some(pseudo),
                profile,
                AuditDecision::Denied,
                Some(reason),
            );
            return Err(PolicyError::AddressViolation(violation));
        }

        // 6. Validar presupuestos de recursos
        match purpose {
            ConnectionPurpose::Ed2kPeer => {
                let active = self.tracker.active_peer_conns.load(Ordering::SeqCst);
                if active >= inner.budgets.max_active_peer_connections {
                    let reason = format!(
                        "Active peer connections limit reached ({active}/{})",
                        inner.budgets.max_active_peer_connections
                    );
                    inner.audit_log.record(
                        AuditEventType::BudgetExhausted,
                        Some(&addr),
                        Some(pseudo),
                        profile,
                        AuditDecision::Denied,
                        Some(reason),
                    );
                    return Err(PolicyError::BudgetExhausted("max_active_peer_connections"));
                }
            }
            ConnectionPurpose::KadNode => {
                // KadNode son operaciones de datagramas UDP sin conexión; rastreado por kad_bucket,
                // no limitado por la capacidad de conexiones TCP de pares persistentes.
            }
            ConnectionPurpose::Ed2kServer => {
                let active = self.tracker.active_server_conns.load(Ordering::SeqCst);
                if active >= inner.budgets.max_active_server_connections {
                    let reason = format!(
                        "Active server connections limit reached ({active}/{})",
                        inner.budgets.max_active_server_connections
                    );
                    inner.audit_log.record(
                        AuditEventType::BudgetExhausted,
                        Some(&addr),
                        Some(pseudo),
                        profile,
                        AuditDecision::Denied,
                        Some(reason),
                    );
                    return Err(PolicyError::BudgetExhausted(
                        "max_active_server_connections",
                    ));
                }
            }
            ConnectionPurpose::Listener => {
                let active = self.tracker.active_listeners.load(Ordering::SeqCst);
                if active >= inner.budgets.max_active_listeners {
                    return Err(PolicyError::BudgetExhausted("max_active_listeners"));
                }
            }
        }

        // 7. Emitir permiso
        let permit_id = inner.next_permit_id;
        inner.next_permit_id = inner.next_permit_id.wrapping_add(1);

        inner.audit_log.record(
            AuditEventType::ConnectAttempt,
            Some(&addr),
            Some(pseudo),
            profile,
            AuditDecision::Allowed,
            Some(format!("Authorized permit #{permit_id} for {purpose:?}")),
        );

        let cancel_token = inner.cancel_token.clone();
        let permit =
            SocketPermit::new(permit_id, addr, purpose, cancel_token, self.tracker.clone());

        Ok(permit)
    }

    /// Solicita un permiso de socket de escucha para vincularse a una dirección.
    pub fn request_tcp_listen(
        &self,
        addr: SocketAddr,
        purpose: ConnectionPurpose,
    ) -> Result<ListenPermit, PolicyError> {
        let mut inner = self.inner.write().unwrap();
        let profile = inner.profile;

        let pseudo = format!("listener:{}", addr.port());

        if inner.state == NetworkState::Disabled {
            return Err(PolicyError::NetworkDisabled);
        }
        if inner.state == NetworkState::ShuttingDown {
            return Err(PolicyError::ShuttingDown);
        }

        if let Err(violation) = validate_listen_address(&addr, profile) {
            let reason = violation.to_string();
            inner.audit_log.record(
                AuditEventType::ListenAttempt,
                Some(&addr),
                Some(pseudo),
                profile,
                AuditDecision::Denied,
                Some(reason),
            );
            return Err(PolicyError::AddressViolation(violation));
        }

        let active = self.tracker.active_listeners.load(Ordering::SeqCst);
        if active >= inner.budgets.max_active_listeners {
            return Err(PolicyError::BudgetExhausted("max_active_listeners"));
        }

        let permit_id = inner.next_permit_id;
        inner.next_permit_id = inner.next_permit_id.wrapping_add(1);

        inner.audit_log.record(
            AuditEventType::ListenAttempt,
            Some(&addr),
            Some(pseudo),
            profile,
            AuditDecision::Allowed,
            Some(format!(
                "Authorized listener permit #{permit_id} for {purpose:?}"
            )),
        );

        let cancel_token = inner.cancel_token.clone();
        let permit = ListenPermit::new(permit_id, addr, cancel_token, self.tracker.clone());

        Ok(permit)
    }

    /// Interruptor de corte global ("Desconectar todo").
    /// Marca atómicamente la red como Deshabilitada (Disabled), activa la cancelación de todas las
    /// tareas de socket pendientes y escribe un evento de auditoría.
    pub fn disconnect_all(&self) {
        let (profile, token_to_cancel) = {
            let mut inner = self.inner.write().unwrap();
            inner.state = NetworkState::ShuttingDown;
            let profile = inner.profile;
            inner.audit_log.record(
                AuditEventType::KillSwitchTriggered,
                None,
                None,
                profile,
                AuditDecision::Allowed,
                Some("Kill switch activated: tearing down all connections".to_string()),
            );
            tracing::warn!("Kill switch activated: cancelling all network operations");
            (profile, inner.cancel_token.clone())
        };

        // Activar la cancelación para todos los permisos activos
        token_to_cancel.cancel();

        // Finalizar el estado a Disabled
        let mut inner = self.inner.write().unwrap();
        inner.state = NetworkState::Disabled;
        let _ = profile;
    }

    /// Vuelve a habilitar la red desde el estado Deshabilitada (Disabled).
    pub fn enable(&self) {
        let mut inner = self.inner.write().unwrap();
        if inner.state == NetworkState::Disabled || inner.state == NetworkState::ShuttingDown {
            inner.cancel_token = CancellationToken::new();
            inner.state = NetworkState::PreviewActive;
            let profile = inner.profile;
            inner.audit_log.record(
                AuditEventType::NetworkEnabled,
                None,
                None,
                profile,
                AuditDecision::Allowed,
                Some("Network policy re-enabled".to_string()),
            );
            tracing::info!("Network policy re-enabled");
        }
    }

    /// Recupera las entradas más recientes del registro de auditoría de red.
    pub fn audit_entries(&self, limit: usize) -> Vec<NetworkAuditEntry> {
        let inner = self.inner.read().unwrap();
        inner.audit_log.recent(limit)
    }

    /// Devuelve el token de cancelación asociado con el administrador de red.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.inner.read().unwrap().cancel_token.clone()
    }

    /// Devuelve el número de conexiones de pares actualmente activas.
    pub fn active_peer_connections_count(&self) -> usize {
        self.tracker.active_peer_conns.load(Ordering::SeqCst)
    }

    /// Devuelve el número de conexiones de servidor actualmente activas.
    pub fn active_server_connections_count(&self) -> usize {
        self.tracker.active_server_conns.load(Ordering::SeqCst)
    }

    /// Devuelve el número de escuchas actualmente activas.
    pub fn active_listeners_count(&self) -> usize {
        self.tracker.active_listeners.load(Ordering::SeqCst)
    }
}

/// `Preferences::GetMaxDownloadInBytesPerSec` de eMule (Preferences.cpp:694,
/// « don't be a Lam3r :) ») — ratio anti-leech subida/descarga: cuando el
/// límite de subida desciende por debajo de 20 KB/s, la descarga efectiva se limita
/// a `coef × upload` (3× si <4 KB/s, 4× si <10, 5× si <20). Un límite de
/// descarga a 0 (ilimitado) también se limita a `coef × upload` —
/// eMule calcula `min(coef × maxup, maxdown)` con maxdown = ∞.
/// Subida 0 (ilimitada) o ≥ 20 KB/s → sin límite de ratio.
///
/// El valor *configurado* permanece inalterado en los ajustes: como en eMule,
/// el ratio es un límite recalculado en cada aplicación, nunca
/// rescrito en las preferencias.
pub fn emule_ud_ratio(download_bps: u64, upload_bps: u64) -> u64 {
    // don't be a Lam3r :)
    if upload_bps == 0 || upload_bps >= 20 * 1024 {
        return download_bps;
    }
    let coef: u64 = if upload_bps < 4 * 1024 {
        3
    } else if upload_bps < 10 * 1024 {
        4
    } else {
        5
    };
    let capped = coef.saturating_mul(upload_bps);
    if download_bps == 0 {
        capped
    } else {
        download_bps.min(capped)
    }
}

#[cfg(test)]
mod tests {
    use super::emule_ud_ratio;

    #[test]
    fn ratio_tiers_match_emule() {
        // < 4 KB/s → coef 3
        assert_eq!(emule_ud_ratio(100 * 1024, 3 * 1024), 9 * 1024);
        // 4..10 KB/s → coef 4
        assert_eq!(emule_ud_ratio(100 * 1024, 5 * 1024), 20 * 1024);
        // 10..20 KB/s → coef 5
        assert_eq!(emule_ud_ratio(100 * 1024, 15 * 1024), 75 * 1024);
        // ≥ 20 KB/s → sin ratio
        assert_eq!(emule_ud_ratio(100 * 1024, 20 * 1024), 100 * 1024);
        assert_eq!(emule_ud_ratio(100 * 1024, 500 * 1024), 100 * 1024);
    }

    #[test]
    fn ratio_keeps_configured_when_lower() {
        // Descarga configurada por debajo del límite → sin cambios.
        assert_eq!(emule_ud_ratio(10 * 1024, 5 * 1024), 10 * 1024);
    }

    #[test]
    fn ratio_caps_unlimited_download() {
        // Descarga ilimitada (0) + subida 5 KB/s → efectivo 4× = 20 KB/s
        // (eMule: min(coef × up, ∞)).
        assert_eq!(emule_ud_ratio(0, 5 * 1024), 20 * 1024);
        // Subida ilimitada (0) → descarga intacta.
        assert_eq!(emule_ud_ratio(0, 0), 0);
        assert_eq!(emule_ud_ratio(100 * 1024, 0), 100 * 1024);
    }
}

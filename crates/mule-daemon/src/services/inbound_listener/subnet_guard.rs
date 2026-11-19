// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Cuotas por subred y tiempos de espera pre-handshake para endurecimiento entrante.
//!
//! Protege al receptor TCP frente a ataques Slowloris y Sybil desde un mismo
//! prefijo `/24` (IPv4) o `/48` (IPv6), eximiendo loopback para pruebas locales.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Límite de conexiones entrantes simultáneas por subred `/24` en IPv4.
pub const MAX_INBOUND_PER_SUBNET_V4: usize = 16;
/// Límite de conexiones entrantes simultáneas por subred `/48` en IPv6.
pub const MAX_INBOUND_PER_SUBNET_V6: usize = 16;
/// Tiempo máximo para recibir el primer byte del protocolo tras aceptar el socket.
pub const PRE_HANDSHAKE_FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(3);
/// Tiempo máximo para completar el saludo de ofuscación RC4 o `PC26-AEAD`.
pub const PRE_HANDSHAKE_OBFU_TIMEOUT: Duration = Duration::from_secs(5);

/// Clave de subred normalizada (`/24` para IPv4 o IPv4-mapeada; `/48` para IPv6 nativa).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubnetKey {
    /// Prefijo IPv4 `/24`.
    V4([u8; 3]),
    /// Prefijo IPv6 `/48`.
    V6([u8; 6]),
}

impl SubnetKey {
    /// Calcula la subred correspondiente a `ip`, devolviendo `None` para direcciones
    /// loopback (`127.0.0.0/8`, `::1`).
    pub fn from_ip(ip: IpAddr) -> Option<Self> {
        let canonical = match ip {
            IpAddr::V4(v4) => IpAddr::V4(v4),
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => IpAddr::V4(v4),
                None => IpAddr::V6(v6),
            },
        };
        if canonical.is_loopback() {
            return None;
        }
        match canonical {
            IpAddr::V4(v4) => {
                let o = v4.octets();
                Some(Self::V4([o[0], o[1], o[2]]))
            }
            IpAddr::V6(v6) => {
                let o = v6.octets();
                Some(Self::V6([o[0], o[1], o[2], o[3], o[4], o[5]]))
            }
        }
    }

    fn limit(self) -> usize {
        match self {
            Self::V4(_) => MAX_INBOUND_PER_SUBNET_V4,
            Self::V6(_) => MAX_INBOUND_PER_SUBNET_V6,
        }
    }
}

/// Error devuelto cuando una subred alcanza el límite de conexiones simultáneas pre-handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubnetQuotaExceeded;

/// Controlador concurrente de cuotas por subred con liberación automática por RAII.
#[derive(Debug, Clone, Default)]
pub struct SubnetQuotaGuard {
    counts: Arc<Mutex<HashMap<SubnetKey, usize>>>,
}

impl SubnetQuotaGuard {
    /// Crea un nuevo controlador de cuotas de subred vacío.
    pub fn new() -> Self {
        Self::default()
    }

    /// Intenta adquirir una ranura para la subred de `ip`.
    /// Devuelve `Err(SubnetQuotaExceeded)` si se alcanzó el tope de la subred.
    pub fn try_acquire(&self, ip: IpAddr) -> Result<Option<SubnetPermit>, SubnetQuotaExceeded> {
        let Some(key) = SubnetKey::from_ip(ip) else {
            return Ok(None);
        };
        let mut guard = self
            .counts
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let entry = guard.entry(key).or_insert(0);
        if *entry >= key.limit() {
            return Err(SubnetQuotaExceeded);
        }
        *entry += 1;
        Ok(Some(SubnetPermit {
            counts: Arc::clone(&self.counts),
            key,
        }))
    }

    /// Limpia el estado interno de cuotas al detener el receptor.
    pub fn clear(&self) {
        if let Ok(mut guard) = self.counts.lock() {
            guard.clear();
        }
    }
}

/// Permiso RAII que decrementa el contador de subred al finalizar o cancelarse la tarea.
#[derive(Debug)]
pub struct SubnetPermit {
    counts: Arc<Mutex<HashMap<SubnetKey, usize>>>,
    key: SubnetKey,
}

impl Drop for SubnetPermit {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.counts.lock() {
            if let std::collections::hash_map::Entry::Occupied(mut e) = guard.entry(self.key) {
                if *e.get() <= 1 {
                    e.remove();
                } else {
                    *e.get_mut() -= 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn subnet_quota_enforces_v4_and_exempts_loopback() {
        let guard = SubnetQuotaGuard::new();
        let mut permits = Vec::new();
        for i in 1..=(MAX_INBOUND_PER_SUBNET_V4 as u8) {
            let ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, i));
            permits.push(guard.try_acquire(ip).unwrap().unwrap());
        }
        // El intento 17 en el mismo /24 debe rechazarse.
        let overflow_ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 200));
        assert!(guard.try_acquire(overflow_ip).is_err());

        // Al liberar uno, vuelve a haber espacio.
        permits.pop();
        assert!(guard.try_acquire(overflow_ip).is_ok());

        // Loopback queda exento.
        for _ in 0..32 {
            let lo = IpAddr::V4(Ipv4Addr::LOCALHOST);
            assert!(guard.try_acquire(lo).unwrap().is_none());
            let lo6 = IpAddr::V6(Ipv6Addr::LOCALHOST);
            assert!(guard.try_acquire(lo6).unwrap().is_none());
        }
    }

    #[test]
    fn subnet_quota_enforces_v6_slash48() {
        let guard = SubnetQuotaGuard::new();
        let mut permits = Vec::new();
        for i in 1..=(MAX_INBOUND_PER_SUBNET_V6 as u16) {
            let ip: IpAddr = format!("2001:db8:abcd::{i:x}").parse().unwrap();
            permits.push(guard.try_acquire(ip).unwrap().unwrap());
        }
        let overflow: IpAddr = "2001:db8:abcd:9999::1".parse().unwrap();
        assert!(guard.try_acquire(overflow).is_err());

        // Otro /48 distinto debe admitirse.
        let other: IpAddr = "2001:db8:abce::1".parse().unwrap();
        assert!(guard.try_acquire(other).is_ok());
    }
}

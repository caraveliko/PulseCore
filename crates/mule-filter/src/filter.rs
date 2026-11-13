// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Cursor};
use std::net::IpAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use crate::interval::FilterRange;
use crate::parser::{parse_ipfilter, ParseError, ParseReport, MAX_FILE_SIZE};

/// Fenêtre glissante (en secondes) pour la détection de flood de connexions par IP.
pub const FLOOD_WINDOW_SECS: u64 = 10;
/// Nombre maximal de nouvelles connexions entrantes tolérées par IP dans `FLOOD_WINDOW_SECS`.
pub const MAX_CONNS_PER_FLOOD_WINDOW: u32 = 24;
/// Nombre maximal d'erreurs protocolaires / paquets corrompus avant bannissement automatique.
pub const MAX_PROTOCOL_STRIKES: u32 = 6;
/// Durée (en secondes) d'un bannissement automatique dynamique (30 minutes).
pub const AUTO_BAN_DURATION_SECS: u64 = 1800;
/// Capacité maximale de la table de suivi anti-flood en mémoire (évite tout épuisement RAM).
pub const MAX_TRACKED_DYNAMIC_IPS: usize = 8192;

#[derive(Debug, Clone)]
struct DynamicIpState {
    window_start: Instant,
    conn_attempts: u32,
    protocol_strikes: u32,
    banned_until: Option<Instant>,
    ban_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterStats {
    pub enabled: bool,
    pub range_count: usize,
    pub hits_count: u64,
    pub last_import_report: Option<ParseReport>,
}

/// Thread-safe in-memory IP filter based on sorted disjoint intervals
/// plus a bounded dynamic auto-ban table for anti-DDoS / flood / malformed packet protection.
/// Lookup is \(O(\log N)\) using binary search.
pub struct IpFilter {
    enabled: AtomicBool,
    ranges: RwLock<Vec<FilterRange>>,
    dynamic_bans: RwLock<HashMap<IpAddr, DynamicIpState>>,
    hits: std::sync::atomic::AtomicU64,
    last_report: RwLock<Option<ParseReport>>,
}

impl Default for IpFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl IpFilter {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            ranges: RwLock::new(Vec::new()),
            dynamic_bans: RwLock::new(HashMap::new()),
            hits: std::sync::atomic::AtomicU64::new(0),
            last_report: RwLock::new(None),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn len(&self) -> usize {
        self.ranges.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        let mut w = self.ranges.write().unwrap();
        w.clear();
        self.dynamic_bans.write().unwrap().clear();
        *self.last_report.write().unwrap() = None;
    }

    /// Vérifie si une adresse IP est exemptée d'auto-ban dynamique (loopback ou LAN privée RFC1918).
    fn is_exempt_from_dynamic_ban(ip: IpAddr) -> bool {
        let canonical = match ip {
            IpAddr::V4(v4) => IpAddr::V4(v4),
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => IpAddr::V4(v4),
                None => IpAddr::V6(v6),
            },
        };
        match canonical {
            IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
            IpAddr::V6(v6) => v6.is_loopback(),
        }
    }

    /// Enregistre une tentative de connexion entrante et déclenche un bannissement
    /// automatique si l'IP dépasse `MAX_CONNS_PER_FLOOD_WINDOW` dans `FLOOD_WINDOW_SECS`.
    /// Retourne `true` si l'IP vient d'être bannie ou est déjà bannie.
    pub fn record_connection_attempt(&self, ip: IpAddr) -> bool {
        if !self.is_enabled() || Self::is_exempt_from_dynamic_ban(ip) {
            return false;
        }
        let now = Instant::now();
        let mut map = self.dynamic_bans.write().unwrap();
        if map.len() >= MAX_TRACKED_DYNAMIC_IPS && !map.contains_key(&ip) {
            map.retain(|_, st| {
                st.banned_until.is_some_and(|until| until > now)
                    || now.duration_since(st.window_start)
                        < Duration::from_secs(FLOOD_WINDOW_SECS * 2)
            });
            if map.len() >= MAX_TRACKED_DYNAMIC_IPS {
                return false;
            }
        }

        let entry = map.entry(ip).or_insert_with(|| DynamicIpState {
            window_start: now,
            conn_attempts: 0,
            protocol_strikes: 0,
            banned_until: None,
            ban_reason: String::new(),
        });

        if let Some(until) = entry.banned_until {
            if now < until {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return true;
            }
            entry.banned_until = None;
            entry.conn_attempts = 0;
            entry.protocol_strikes = 0;
            entry.window_start = now;
        }

        if now.duration_since(entry.window_start) > Duration::from_secs(FLOOD_WINDOW_SECS) {
            entry.window_start = now;
            entry.conn_attempts = 1;
        } else {
            entry.conn_attempts += 1;
            if entry.conn_attempts > MAX_CONNS_PER_FLOOD_WINDOW {
                entry.banned_until = Some(now + Duration::from_secs(AUTO_BAN_DURATION_SECS));
                entry.ban_reason = format!(
                    "Auto-Ban DDoS/Flood ({} conns/{}s)",
                    entry.conn_attempts, FLOOD_WINDOW_SECS
                );
                self.hits.fetch_add(1, Ordering::Relaxed);
                return true;
            }
        }
        false
    }

    /// Enregistre une anomalie protocolaire (paquet malformé, échec d'obfuscation répété, flood UDP).
    /// Retourne `true` si l'IP atteint `MAX_PROTOCOL_STRIKES` et est bannie automatiquement.
    pub fn record_protocol_strike(&self, ip: IpAddr, reason: &str) -> bool {
        if !self.is_enabled() || Self::is_exempt_from_dynamic_ban(ip) {
            return false;
        }
        let now = Instant::now();
        let mut map = self.dynamic_bans.write().unwrap();
        if map.len() >= MAX_TRACKED_DYNAMIC_IPS && !map.contains_key(&ip) {
            return false;
        }

        let entry = map.entry(ip).or_insert_with(|| DynamicIpState {
            window_start: now,
            conn_attempts: 0,
            protocol_strikes: 0,
            banned_until: None,
            ban_reason: String::new(),
        });

        if let Some(until) = entry.banned_until {
            if now < until {
                return true;
            }
            entry.banned_until = None;
            entry.protocol_strikes = 0;
        }

        entry.protocol_strikes += 1;
        if entry.protocol_strikes >= MAX_PROTOCOL_STRIKES {
            entry.banned_until = Some(now + Duration::from_secs(AUTO_BAN_DURATION_SECS));
            entry.ban_reason = format!("Auto-Ban Protocol Strike ({reason})");
            self.hits.fetch_add(1, Ordering::Relaxed);
            return true;
        }
        false
    }

    /// Load and parse filter ranges from a local file path.
    pub fn load_from_file<P: AsRef<Path>>(&self, path: P) -> Result<ParseReport, ParseError> {
        let path_ref = path.as_ref();
        let metadata = std::fs::metadata(path_ref)?;
        if metadata.len() > MAX_FILE_SIZE {
            return Err(ParseError::FileTooLarge(metadata.len()));
        }

        let file = File::open(path_ref)?;
        let reader = BufReader::new(file);
        let (ranges, report) = parse_ipfilter(reader)?;

        {
            let mut w = self.ranges.write().unwrap();
            *w = ranges;
        }
        {
            let mut lr = self.last_report.write().unwrap();
            *lr = Some(report.clone());
        }

        Ok(report)
    }

    /// Load and parse filter ranges from an in-memory string or byte slice.
    pub fn load_from_str(&self, content: &str) -> Result<ParseReport, ParseError> {
        if content.len() as u64 > MAX_FILE_SIZE {
            return Err(ParseError::FileTooLarge(content.len() as u64));
        }
        let reader = BufReader::new(Cursor::new(content.as_bytes()));
        let (ranges, report) = parse_ipfilter(reader)?;

        {
            let mut w = self.ranges.write().unwrap();
            *w = ranges;
        }
        {
            let mut lr = self.last_report.write().unwrap();
            *lr = Some(report.clone());
        }

        Ok(report)
    }

    /// Checks whether an IP address is blocked (by dynamic Auto-Ban or static `ipfilter.dat`).
    /// Supports native IPv4 as well as IPv4-mapped IPv6 (`::ffff:a.b.c.d`).
    pub fn check_ip(&self, ip: IpAddr) -> Option<FilterRange> {
        if !self.is_enabled() {
            return None;
        }

        // 1. Vérification de la table dynamique d'Auto-Ban (anti-DDoS / Flood / Strikes)
        if let Some(st) = self.dynamic_bans.read().unwrap().get(&ip) {
            if let Some(until) = st.banned_until {
                if Instant::now() < until {
                    self.hits.fetch_add(1, Ordering::Relaxed);
                    let v4 = match ip {
                        IpAddr::V4(v4) => v4,
                        IpAddr::V6(v6) => v6
                            .to_ipv4_mapped()
                            .unwrap_or(std::net::Ipv4Addr::UNSPECIFIED),
                    };
                    return Some(FilterRange::new(v4, v4, 0, st.ban_reason.clone()));
                }
            }
        }

        let v4_opt = match ip {
            IpAddr::V4(v4) => Some(v4),
            IpAddr::V6(v6) => v6.to_ipv4_mapped(),
        };

        let v4 = v4_opt?;
        let ip_u32 = u32::from(v4);

        let guard = self.ranges.read().unwrap();
        let ranges = guard.as_slice();

        // Binary search on ranges. Since ranges are sorted by start ascending and merged (disjoint):
        // We binary search for the first range whose start <= ip_u32
        let idx = match ranges.binary_search_by(|r| {
            if r.start <= ip_u32 && ip_u32 <= r.end {
                std::cmp::Ordering::Equal
            } else if r.start > ip_u32 {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            }
        }) {
            Ok(found_idx) => found_idx,
            Err(_) => return None,
        };

        self.hits.fetch_add(1, Ordering::Relaxed);
        Some(ranges[idx].clone())
    }

    pub fn stats(&self) -> FilterStats {
        FilterStats {
            enabled: self.is_enabled(),
            range_count: self.len(),
            hits_count: self.hits.load(Ordering::Relaxed),
            last_import_report: self.last_report.read().unwrap().clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_dynamic_auto_ban_on_connection_flood() {
        let filter = IpFilter::new();
        let attacker_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 42));

        for _ in 0..MAX_CONNS_PER_FLOOD_WINDOW {
            assert!(!filter.record_connection_attempt(attacker_ip));
        }
        // Le dépassement déclenche le bannissement automatique
        assert!(filter.record_connection_attempt(attacker_ip));
        let rule = filter
            .check_ip(attacker_ip)
            .expect("attacker IP should be dynamically banned");
        assert!(rule.description.contains("Auto-Ban DDoS/Flood"));
    }

    #[test]
    fn test_dynamic_auto_ban_exempts_loopback_and_lan() {
        let filter = IpFilter::new();
        let lan_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50));
        for _ in 0..(MAX_CONNS_PER_FLOOD_WINDOW * 2) {
            assert!(!filter.record_connection_attempt(lan_ip));
        }
        assert!(filter.check_ip(lan_ip).is_none());
    }
}

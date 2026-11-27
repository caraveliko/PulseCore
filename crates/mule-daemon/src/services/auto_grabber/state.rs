// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::services::scorer::normalize_accents_and_symbols;
use super::TITLE_BLACKLIST_PREFIX;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BlacklistedEntry {
    pub hash: String,
    pub title: String,
    pub reason: String,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveGrabTracker {
    pub download_id: String,
    pub target_id: String,
    pub media_id: String,
    pub is_episode: bool,
    pub hash: String,
    pub release_title: String,
    pub protocol: String,
    pub quality: String,
    pub language: String,
    pub score: i32,
    pub started_at_ms: i64,
    pub last_progress_bytes: u64,
    pub last_progress_at_ms: i64,
    pub is_upgrade: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GrabDiagnostic {
    pub last_search_ms: i64,
    pub consecutive_empty_searches: u32,
    pub next_eligible_search_ms: i64,
    pub total_candidates: usize,
    pub valid_candidates: usize,
    pub last_status: String,
    pub summary_message: String,
    pub top_rejection_reasons: Vec<String>,
    pub grabbed_title: Option<String>,
    pub grabbed_quality: Option<String>,
    pub grabbed_language: Option<String>,
    pub grabbed_score: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AutoGrabberState {
    #[serde(default)]
    pub blacklisted_hashes: HashMap<String, Vec<BlacklistedEntry>>,
    #[serde(default)]
    pub active_grabs: HashMap<String, ActiveGrabTracker>,
    #[serde(default)]
    pub diagnostics: HashMap<String, GrabDiagnostic>,
}

impl AutoGrabberState {
    fn normalized_title_key(title: &str) -> String {
        normalize_accents_and_symbols(title)
    }

    pub fn is_hash_blacklisted(&self, target_id: &str, hash: &str) -> bool {
        if hash.is_empty() {
            return false;
        }
        let h_lower = hash.to_lowercase();
        self.blacklisted_hashes
            .get(target_id)
            .map(|list| list.iter().any(|e| e.hash.eq_ignore_ascii_case(&h_lower)))
            .unwrap_or(false)
    }

    /// Comprueba si una release está en lista negra por hash o por título normalizado equivalente.
    pub fn is_release_blacklisted(&self, target_id: &str, hash: &str, title: &str) -> bool {
        if self.is_hash_blacklisted(target_id, hash) {
            return true;
        }
        let title_key = Self::normalized_title_key(title);
        if title_key.is_empty() {
            return false;
        }
        self.blacklisted_hashes
            .get(target_id)
            .map(|list| {
                list.iter()
                    .any(|e| !e.title.is_empty() && Self::normalized_title_key(&e.title) == title_key)
            })
            .unwrap_or(false)
    }

    pub fn blacklist_hash(&mut self, target_id: &str, hash: &str, title: &str, reason: &str, now: i64) {
        if hash.is_empty() {
            return;
        }
        let entry = self.blacklisted_hashes.entry(target_id.to_string()).or_default();
        if !entry.iter().any(|e| e.hash.eq_ignore_ascii_case(hash)) {
            entry.push(BlacklistedEntry {
                hash: hash.to_lowercase(),
                title: title.to_string(),
                reason: reason.to_string(),
                timestamp_ms: now,
            });
        }
    }

    /// Registra una release en lista negra usando su hash o, si no existe, una clave sintética de título.
    pub fn blacklist_release(
        &mut self,
        target_id: &str,
        hash: Option<&str>,
        title: &str,
        reason: &str,
        now: i64,
    ) {
        if let Some(h) = hash.filter(|s| !s.trim().is_empty()) {
            self.blacklist_hash(target_id, h.trim(), title, reason, now);
            return;
        }
        let title_key = Self::normalized_title_key(title);
        if title_key.is_empty() {
            return;
        }
        let synthetic_hash = format!("{TITLE_BLACKLIST_PREFIX}{title_key}");
        self.blacklist_hash(target_id, &synthetic_hash, title, reason, now);
    }

    /// Indica si un motivo de lista negra debe ser permanente (idioma erróneo o archivo inválido)
    /// frente a motivos transitorios de disponibilidad o estancamiento en red P2P.
    pub fn is_permanent_blacklist_reason(reason: &str) -> bool {
        let lower = reason.to_lowercase();
        lower == super::WRONG_LANGUAGE_REASON
            || lower.contains("idioma")
            || lower.contains("wrong_language")
            || lower.contains("corrupt")
            || lower.contains("fake")
    }

    /// Purga las entradas temporales de lista negra (estancamiento / sin fuentes) cuyo plazo `ttl_ms`
    /// haya caducado, manteniendo intactas las exclusiones permanentes por idioma erróneo o archivo inválido.
    pub fn prune_expired_blacklists(&mut self, now: i64, ttl_ms: i64) -> usize {
        let mut pruned = 0usize;
        self.blacklisted_hashes.retain(|_, entries| {
            let before = entries.len();
            entries.retain(|entry| {
                Self::is_permanent_blacklist_reason(&entry.reason)
                    || now.saturating_sub(entry.timestamp_ms) < ttl_ms
            });
            pruned += before.saturating_sub(entries.len());
            !entries.is_empty()
        });
        pruned
    }
}


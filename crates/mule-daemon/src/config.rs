// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use std::net::SocketAddr;
use std::path::PathBuf;

use crate::workdir::WorkDirMode;

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub bind_addr: SocketAddr,
    pub data_dir: PathBuf,
    pub default_download_dir: PathBuf,
    pub auth_token: String,
    pub workdir_mode: WorkDirMode,
    pub network_profile: mule_network_policy::NetworkProfile,
    pub upnp_enabled: bool,
    pub ed2k_port: u16,
    pub kad_port: u16,
    pub obfuscation_enabled: bool,
    pub obfuscation_required: bool,
    pub obfuscation_padding_max: u8,
    pub max_global_download_peers: usize,
    pub max_peers_per_download: usize,
    pub min_peers_per_download: usize,
    /// Static web UI directory served over HTTP (bundle `web/` folder).
    /// `None` disables the embedded web remote controller.
    pub web_dir: Option<PathBuf>,
    /// Root directory for media library / NAS storage.
    pub nas_root_dir: PathBuf,
    /// Directory for organized movies.
    pub movies_dir: PathBuf,
    /// Directory for organized TV series.
    pub series_dir: PathBuf,
    /// Additional incoming directories scanned by the media organizer.
    pub extra_incoming_dirs: Vec<PathBuf>,
    /// Sonarr base URL (e.g. `http://127.0.0.1:8989`).
    pub sonarr_url: String,
    /// Optional Sonarr API key (`EMULERUST_SONARR_API_KEY`).
    pub sonarr_api_key: Option<String>,
    /// Radarr base URL (e.g. `http://127.0.0.1:7878`).
    pub radarr_url: String,
    /// Optional Radarr API key (`EMULERUST_RADARR_API_KEY`).
    pub radarr_api_key: Option<String>,
    /// Jellyfin base URL (e.g. `http://127.0.0.1:8096`).
    pub jellyfin_url: String,
    /// Optional Jellyfin API token (`EMULERUST_JELLYFIN_API_KEY`).
    pub jellyfin_api_key: Option<String>,
    /// Optional Plex refresh URL (`EMULERUST_PLEX_URL`).
    pub plex_url: Option<String>,
    /// Optional SSH target (`user@host`) for NAS hardware telemetry (`EMULERUST_NAS_SSH_TARGET`).
    pub nas_ssh_target: Option<String>,
    /// SSH port for NAS hardware telemetry (`EMULERUST_NAS_SSH_PORT`).
    pub nas_ssh_port: u16,
}

impl DaemonConfig {
    pub fn new(data_dir: PathBuf, auth_token: impl Into<String>) -> Self {
        let default_download_dir = data_dir.join("downloads");
        let nas_root_dir = std::env::var("EMULERUST_NAS_PATH")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| default_download_dir.join("media"));
        let movies_dir = std::env::var("EMULERUST_MOVIES_DIR")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| nas_root_dir.join("Movies"));
        let series_dir = std::env::var("EMULERUST_SERIES_DIR")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| nas_root_dir.join("Series"));
        let extra_incoming_dirs = std::env::var("EMULERUST_EXTRA_INCOMING_DIRS")
            .ok()
            .map(|s| {
                s.split(':')
                    .filter(|p| !p.trim().is_empty())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default();
        let sonarr_url = std::env::var("EMULERUST_SONARR_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "http://127.0.0.1:8989".to_string());
        let sonarr_api_key = std::env::var("EMULERUST_SONARR_API_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let radarr_url = std::env::var("EMULERUST_RADARR_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "http://127.0.0.1:7878".to_string());
        let radarr_api_key = std::env::var("EMULERUST_RADARR_API_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let jellyfin_url = std::env::var("EMULERUST_JELLYFIN_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "http://127.0.0.1:8096".to_string());
        let jellyfin_api_key = std::env::var("EMULERUST_JELLYFIN_API_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let plex_url = std::env::var("EMULERUST_PLEX_URL")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let nas_ssh_target = std::env::var("EMULERUST_NAS_SSH_TARGET")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let nas_ssh_port = std::env::var("EMULERUST_NAS_SSH_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(2222);

        Self {
            bind_addr: "127.0.0.1:4660".parse().unwrap(),
            data_dir,
            default_download_dir,
            auth_token: auth_token.into(),
            workdir_mode: WorkDirMode::InstalledDefault,
            network_profile: mule_network_policy::NetworkProfile::PublicEd2kKadPreview,
            // eMule 0.72a `EnableUPnP` default = false (opt-in).
            upnp_enabled: false,
            ed2k_port: 39735,
            kad_port: 39737,
            obfuscation_enabled: true,
            obfuscation_required: false,
            obfuscation_padding_max: 16,
            max_global_download_peers: 500,
            max_peers_per_download: 400,
            min_peers_per_download: 5,
            web_dir: None,
            nas_root_dir,
            movies_dir,
            series_dir,
            extra_incoming_dirs,
            sonarr_url,
            sonarr_api_key,
            radarr_url,
            radarr_api_key,
            jellyfin_url,
            jellyfin_api_key,
            plex_url,
            nas_ssh_target,
            nas_ssh_port,
        }
    }

    pub fn with_bind_addr(mut self, addr: SocketAddr) -> Self {
        self.bind_addr = addr;
        self
    }

    pub fn with_workdir_mode(mut self, mode: WorkDirMode) -> Self {
        self.workdir_mode = mode;
        self
    }

    pub fn with_default_download_dir(mut self, dir: PathBuf) -> Self {
        self.default_download_dir = dir;
        self
    }

    pub fn with_network_profile(mut self, profile: mule_network_policy::NetworkProfile) -> Self {
        self.network_profile = profile;
        self
    }

    pub fn with_upnp_enabled(mut self, enabled: bool) -> Self {
        self.upnp_enabled = enabled;
        self
    }

    pub fn with_ports(mut self, ed2k_port: u16, kad_port: u16) -> Self {
        self.ed2k_port = ed2k_port;
        self.kad_port = kad_port;
        self
    }

    pub fn with_obfuscation(mut self, enabled: bool, required: bool) -> Self {
        self.obfuscation_enabled = enabled;
        self.obfuscation_required = required;
        self
    }

    pub fn with_web_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.web_dir = dir;
        self
    }
}

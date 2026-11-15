// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

//! Motor BitTorrent integrado acotado y optimizado para bajo consumo de memoria.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use librqbit::api::TorrentIdOrHash;
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions};
use serde::{Deserialize, Serialize};

/// Límite estricto de pares concurrentes por torrent para evitar sobrecarga de descriptores y memoria de búfer.
pub const TORRENT_PEER_LIMIT: usize = 32;

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

pub struct TorrentEngine {
    session: Arc<Session>,
    default_output_dir: PathBuf,
    categories: RwLock<HashMap<String, String>>,
}

impl TorrentEngine {
    pub async fn new(default_output_dir: PathBuf) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let opts = SessionOptions::default();
        let session = Session::new_with_opts(default_output_dir.clone(), opts).await?;
        Ok(Self {
            session,
            default_output_dir,
            categories: RwLock::new(HashMap::new()),
        })
    }

    pub async fn new_offline(
        default_output_dir: PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let opts = SessionOptions {
            dht: None,
            disable_trackers: true,
            persistence: None,
            listen: None,
            ..Default::default()
        };
        let session = Session::new_with_opts(default_output_dir.clone(), opts).await?;
        Ok(Self {
            session,
            default_output_dir,
            categories: RwLock::new(HashMap::new()),
        })
    }

    pub async fn add_url(
        &self,
        url: &str,
        output_folder: Option<PathBuf>,
        category: Option<String>,
    ) -> Result<TorrentSnapshot, Box<dyn std::error::Error + Send + Sync>> {
        let add_torrent = AddTorrent::from_url(url.to_string());
        self.add_torrent_internal(add_torrent, output_folder, category).await
    }

    pub async fn add_bytes(
        &self,
        bytes: Vec<u8>,
        output_folder: Option<PathBuf>,
        category: Option<String>,
    ) -> Result<TorrentSnapshot, Box<dyn std::error::Error + Send + Sync>> {
        let add_torrent = AddTorrent::from_bytes(bytes);
        self.add_torrent_internal(add_torrent, output_folder, category).await
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
        let opts = AddTorrentOptions {
            output_folder: Some(out_dir.to_string_lossy().to_string()),
            overwrite: true,
            peer_limit: Some(TORRENT_PEER_LIMIT),
            ..Default::default()
        };

        let res = self.session.add_torrent(add_torrent, Some(opts)).await?;
        let (id, handle) = match res {
            AddTorrentResponse::Added(id, handle) => (id, handle),
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

    pub fn list(&self) -> Vec<TorrentSnapshot> {
        let cats = self.categories.read().ok();
        self.session.with_torrents(|torrents| {
            torrents
                .map(|(id, handle)| {
                    let hash_hex = handle.info_hash().as_string();
                    let name = handle.name().unwrap_or_else(|| "Torrent".to_string());
                    let stats = handle.stats();
                    let category = cats
                        .as_ref()
                        .and_then(|c| c.get(&hash_hex).cloned());
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
                        output_dir: self.default_output_dir.to_string_lossy().to_string(),
                        category,
                    }
                })
                .collect()
        })
    }

    pub async fn pause(&self, id: usize) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(handle) = self.session.get(TorrentIdOrHash::Id(id)) {
            self.session.pause(&handle).await?;
        }
        Ok(())
    }

    pub async fn resume(&self, id: usize) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(handle) = self.session.get(TorrentIdOrHash::Id(id)) {
            self.session.unpause(&handle).await?;
        }
        Ok(())
    }

    pub async fn delete(&self, id: usize, delete_files: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.session.delete(TorrentIdOrHash::Id(id), delete_files).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid bencoded single-file `.torrent` buffer for offline testing.
    fn sample_torrent_bytes() -> Vec<u8> {
        // d8:announce25:http://127.0.0.1/announce4:infod6:lengthi1024e4:name8:test.bin12:piece lengthi16384e6:pieces20:AAAAAAAAAAAAAAAAAAAAee
        let mut buf = Vec::new();
        buf.extend_from_slice(
            b"d8:announce25:http://127.0.0.1/announce4:infod6:lengthi1024e4:name8:test.bin12:piece lengthi16384e6:pieces20:",
        );
        buf.extend_from_slice(&[0x41u8; 20]);
        buf.extend_from_slice(b"ee");
        buf
    }

    #[tokio::test]
    async fn test_offline_engine_add_list_category_and_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let engine = TorrentEngine::new_offline(dir.path().to_path_buf())
            .await
            .expect("offline torrent engine should initialize");

        assert!(engine.list().is_empty());

        let snap = engine
            .add_bytes(
                sample_torrent_bytes(),
                None,
                Some("movies".to_string()),
            )
            .await
            .expect("valid bencoded torrent should be added");

        assert_eq!(snap.name, "test.bin");
        assert_eq!(snap.total_bytes, 1024);
        assert_eq!(snap.category.as_deref(), Some("movies"));

        // Pause immediately so no tracker activity continues
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
}

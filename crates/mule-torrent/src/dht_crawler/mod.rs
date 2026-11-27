// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Descubrimiento descentralizado de torrents de vídeo en Mainline DHT sin indexadores externos.

pub mod bencode;
pub mod content_filter;
pub mod crawler;
pub mod ut_metadata;
pub mod wire;

pub use content_filter::{
    has_media_release_signal, is_blocked_abusive_or_adult, is_clean_catalog_video,
    MIN_DHT_MEDIA_VIDEO_BYTES,
};

pub use crawler::{DhtHarvester, DhtHarvesterStats, MAINLINE_DHT_BOOTSTRAP_HOSTS};
pub use ut_metadata::{
    build_minimal_torrent_bytes, fetch_video_metadata_from_peer, parse_verified_video_info,
    DhtVideoFile, DiscoveredVideoTorrent,
};
pub use wire::derive_ed2k_rendezvous_id;

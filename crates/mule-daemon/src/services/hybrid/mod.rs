// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Motor híbrido eD2K ⇄ Kad ⇄ BitTorrent sin indexadores externos.

pub mod active_discovery;
pub mod fusion;
pub mod kad_bridge;
pub mod public_search;
pub mod service;

pub use active_discovery::EquivalentTorrentHint;
pub use service::{
    ActiveHybridFusion, DownloadHybridTelemetry, HybridDiscoveryReport, HybridStatusSnapshot,
    HybridSwarmService,
};

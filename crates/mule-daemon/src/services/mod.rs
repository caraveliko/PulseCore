// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

pub mod auto_grabber;
pub mod cleaner;
pub mod compression_policy;
pub mod client_udp;
pub mod disk_scanner;
pub mod downloads;
pub mod events;
pub(crate) mod hello_caps;
pub mod hole_punch;
pub mod identity;
pub mod inbound_listener;
pub mod incoming_share;
pub mod indexer_aggregator;
pub mod kad;
pub mod kad_udp;
pub mod known_met_import;
pub mod media_automation;
pub mod media_metadata;
pub mod media_probe;
pub mod nas_transfer;
pub mod nfs_mount;
pub mod obfuscation_stream;
pub mod organizer;
pub mod pc26_stream;
pub mod peer_session;
pub mod quic_transport;
pub mod scorer;
pub mod server_session;
pub mod server_udp;
pub mod subtitles;
pub mod swarm_coordinator;
pub mod upload_queue;
pub mod upnp;
pub mod updater;
pub mod uss;
mod uss_icmp;
pub mod uss_ledbat;

pub use client_udp::{ClientUdpService, UdpReaskResult, UdpReaskTarget};
pub use events::DaemonEvent;
pub use identity::{IdentityService, IdentityStatus};
pub use kad::{KadContactEntryDto, KadService, KadStatusSnapshot, NodesDatInspectReport};
pub use kad_udp::{KadUdpService, KadUdpSnapshot, UdpFirewallStatus};
pub use obfuscation_stream::ObfuscatedStream;
pub use upnp::{UpnpService, UpnpStatusSnapshot};
pub use updater::{UpdateStatusSnapshot, UpdaterService};

pub use auto_grabber::AutoGrabber;
pub use cleaner::MediaCleaner;
pub use disk_scanner::DiskScanner;
pub use downloads::{
    DownloadDetail, DownloadService, DownloadSnapshot, ImportPartMetResult, PartMetInspection,
    PartProgressSummary, SourceAvailabilitySummary,
};
pub use inbound_listener::{InboundListenerService, InboundListenerSnapshot};
pub use indexer_aggregator::IndexerAggregator;
pub use media_automation::MediaAutomation;
pub use media_metadata::MediaMetadataService;
pub use organizer::MediaOrganizer;
pub use peer_session::{PeerConnectionStatus, PeerSessionManager, PeerSessionSnapshot};
pub use server_session::{ServerConnectionStatus, ServerSessionManager, ServerSessionSnapshot};
pub use subtitles::SubtitleService;
pub use swarm_coordinator::SwarmCoordinator;
pub use upload_queue::{
    UploadManager, UploadSessionSnapshot, UploadSlotState, MAX_ACTIVE_UPLOADS, MAX_UPLOAD_QUEUE,
};
pub use uss::{UssConfig, UssService, UssSnapshot};

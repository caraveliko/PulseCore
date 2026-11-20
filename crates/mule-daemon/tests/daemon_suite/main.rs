// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Arnés unificado de pruebas de integración estándar de `mule-daemon`.

mod aich_tests;
mod category_rpc_tests;
mod content_policy_tests;
mod corruption_blackbox_tests;
mod download_details_and_destination_tests;
mod dual_portable_bundles;
mod ed2k_obfuscation_tests;
mod filter_rpc_tests;
mod hashset_tests;
mod history_rpc_tests;
mod kad_bootstrap_tests;
mod kad_keywords_tests;
mod kad_rpc_tests;
mod kad_sources_tests;
mod kad_udp_tests;
mod known_met_import_tests;
mod lifecycle_and_locking;
mod migration_import_tests;
mod network_policy_rpc_tests;
mod notifications_tests;
mod orphan_recovery_tests;
mod part_met_rpc_tests;
mod part_met_write_tests;
mod peer_adversarial_tests;
mod peer_connection_tests;
mod rpc_contract;
mod secure_ident_tests;
mod server_connection_tests;
mod servers_rpc_tests;
mod swarm_dual_peer_tests;
mod transmission_rpc_tests;
mod updater_tests;
mod upload_inbound_tests;
mod upnp_and_wan_tests;
mod webui_auth_tests;
mod workdir_tests;

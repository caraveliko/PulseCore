// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

/// Capacidades anunciadas por el daemon en `system.hello` y `system.status`.
///
/// Fuente única de verdad: ambos manejadores RPC deben notificar el mismo conjunto.
pub fn daemon_capabilities() -> Vec<&'static str> {
    vec![
        "downloads",
        "download-events",
        "history",
        "categories",
        "import_part_met",
        "import_known_met",
        "import_emfriends_met",
        "import_cancelled_met",
        "import_clients_met",
        "import_emule_identity",
        "network-policy",
        "ip-filter",
        "rate-limits",
        "servers",
        "server-connection",
        "server-search",
        "server-sources",
        "peer-sources",
        "peer-connection",
        "peer-robustness",
        "swarm-dual-source",
        "shared-files",
        "upload-single-slot",
        "secure-ident",
        "peer-credits",
        "kad-contacts",
        "notifications",
        "hybrid-swarm",
        #[cfg(feature = "lab-tools")]
        "lab-tools",
    ]
}

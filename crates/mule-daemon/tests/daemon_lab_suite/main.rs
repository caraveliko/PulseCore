// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Arnés unificado de pruebas de laboratorio loopback (`lab-loopback` / `lab-tools`) de `mule-daemon`.

mod daemon_adversarial;
mod inter_daemon_transfer;
#[cfg(feature = "lab-tools")]
mod lab_rpc_tests;

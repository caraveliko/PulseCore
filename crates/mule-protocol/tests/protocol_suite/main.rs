// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Arnés unificado de pruebas estándar de `mule-protocol`.

#[path = "../common/mod.rs"]
pub mod common;

mod ed2k_aich_tests;
mod ed2k_corpus_tests;
mod ed2k_emule_compat_tests;
mod ed2k_extended_tests;
mod ed2k_hashset_tests;
mod ed2k_isolation_guard;
mod ed2k_opcode_disambiguation_tests;
mod ed2k_roundtrip_tests;
mod ed2k_sequencer_tests;
mod kad_aich_tests;
mod kad_codec_tests;
mod malformed_packets;
mod packet_codec;
mod protocol_limits;
mod server_codec_tests;

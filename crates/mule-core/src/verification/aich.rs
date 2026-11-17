// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_format::{verify_aich_proof, AichProof};

use crate::error::CoreError;

/// Verifica un subbloque de 180 KiB utilizando una prueba de Merkle AICH.
pub fn verify_aich_block(
    expected_root: [u8; 20],
    block_bytes: &[u8],
    proof: &AichProof,
) -> Result<(), CoreError> {
    verify_aich_proof(expected_root, block_bytes, proof).map_err(|e| {
        CoreError::CorruptedPart {
            part_index: proof.leaf_index / 53, // estimación aproximada basada en partes
            reason: e.to_string(),
        }
    })
}

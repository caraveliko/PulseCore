// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use md4::{Digest, Md4};

/// Verifica que una parte eD2K completa o parcial coincida con su hash MD4 esperado.
pub fn verify_part_hash(part_bytes: &[u8], expected_hash: &[u8; 16]) -> bool {
    let mut hasher = Md4::new();
    hasher.update(part_bytes);
    let digest: [u8; 16] = hasher.finalize().into();
    &digest == expected_hash
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_format::{
    calculate_aich_root, verify_aich_proof, AichHash, AichNode, AichProofError, AichSibling,
    SiblingPosition, AICH_BLOCK_SIZE, BLOCKS_PER_PART, ED2K_PART_SIZE, LAST_BLOCK_SIZE_FULL_PART,
};
use sha1::{Digest, Sha1};

fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let mut h = Sha1::new();
    h.update(data);
    h.finalize().into()
}

#[test]
fn test_aich_single_block() {
    let data = b"Hello, AICH!";
    let expected = sha1_digest(data);

    let root = calculate_aich_root(data).unwrap();
    assert_eq!(root.as_bytes(), &expected);
}

#[test]
fn test_aich_exact_block_size() {
    let data = vec![0x33; AICH_BLOCK_SIZE];
    let expected = sha1_digest(&data);

    let root = calculate_aich_root(&data).unwrap();
    assert_eq!(root.as_bytes(), &expected);
}

#[test]
fn test_aich_full_part_53_blocks_structure_and_proof() {
    // 52 bloques de 184 320 + 1 bloque de 143 360 = 9 728 000
    assert_eq!(
        52 * AICH_BLOCK_SIZE + LAST_BLOCK_SIZE_FULL_PART,
        ED2K_PART_SIZE
    );
    assert_eq!(BLOCKS_PER_PART, 53);

    let data = vec![0x7a; ED2K_PART_SIZE];
    let root = calculate_aich_root(&data).unwrap();

    let mut tree = AichNode::new_tree(ED2K_PART_SIZE as u64).unwrap();
    let mut offset = 0;
    while offset < data.len() {
        let chunk_len = (data.len() - offset).min(AICH_BLOCK_SIZE);
        let block_bytes = &data[offset..offset + chunk_len];
        let digest = sha1_digest(block_bytes);
        tree.set_leaf_hash(offset as u64, chunk_len as u64, AichHash(digest));
        offset += chunk_len;
    }
    tree.recalculate_hash().unwrap();

    // 1. Verificar prueba válida para hoja #0
    let proof0 = tree.generate_leaf_proof(0, BLOCKS_PER_PART as u32).unwrap();
    assert_eq!(proof0.leaf_index, 0);
    assert_eq!(proof0.leaf_count, 53);
    assert!(verify_aich_proof(root.0, &data[..AICH_BLOCK_SIZE], &proof0).is_ok());

    // 2. Verificar prueba válida para la última hoja #52 (143 360 bytes)
    let last_offset = 52 * AICH_BLOCK_SIZE;
    let proof52 = tree
        .generate_leaf_proof(52, BLOCKS_PER_PART as u32)
        .unwrap();
    assert_eq!(proof52.leaf_index, 52);
    assert!(verify_aich_proof(root.0, &data[last_offset..], &proof52).is_ok());

    // 3. Prueba estructural: índice de hoja fuera de límites
    let mut invalid_proof = proof0.clone();
    invalid_proof.leaf_index = 53;
    assert_eq!(
        verify_aich_proof(root.0, &data[..AICH_BLOCK_SIZE], &invalid_proof).unwrap_err(),
        AichProofError::LeafIndexOutOfBounds {
            index: 53,
            count: 53
        }
    );

    // 4. Prueba estructural: prueba truncada (falta hermano)
    let mut truncated_proof = proof0.clone();
    truncated_proof.siblings.pop();
    assert!(matches!(
        verify_aich_proof(root.0, &data[..AICH_BLOCK_SIZE], &truncated_proof).unwrap_err(),
        AichProofError::InvalidDepth { .. }
    ));

    // 5. Prueba estructural: hermano adicional
    let mut extra_proof = proof0.clone();
    extra_proof.siblings.push(AichSibling {
        hash: [0u8; 20],
        position: SiblingPosition::Left,
    });
    assert!(matches!(
        verify_aich_proof(root.0, &data[..AICH_BLOCK_SIZE], &extra_proof).unwrap_err(),
        AichProofError::InvalidDepth { .. }
    ));

    // 6. Prueba estructural: posición de hermano invertida
    let mut inverted_proof = proof0.clone();
    let old_pos = inverted_proof.siblings[0].position;
    inverted_proof.siblings[0].position = match old_pos {
        SiblingPosition::Left => SiblingPosition::Right,
        SiblingPosition::Right => SiblingPosition::Left,
    };
    assert!(matches!(
        verify_aich_proof(root.0, &data[..AICH_BLOCK_SIZE], &inverted_proof).unwrap_err(),
        AichProofError::InvalidSiblingPosition { .. }
    ));

    // 7. Prueba criptográfica: hash de hermano corrupto
    let mut bad_hash_proof = proof0.clone();
    bad_hash_proof.siblings[0].hash[0] ^= 0xff;
    assert!(matches!(
        verify_aich_proof(root.0, &data[..AICH_BLOCK_SIZE], &bad_hash_proof).unwrap_err(),
        AichProofError::RootMismatch { .. }
    ));

    // 8. Prueba criptográfica: datos de hoja corruptos
    let mut corrupted_data = data[..AICH_BLOCK_SIZE].to_vec();
    corrupted_data[0] ^= 0xff;
    assert!(matches!(
        verify_aich_proof(root.0, &corrupted_data, &proof0).unwrap_err(),
        AichProofError::RootMismatch { .. }
    ));

    // 9. Prueba de tamaño: datos de hoja mayores que 180 KiB
    let oversized = vec![0x11; AICH_BLOCK_SIZE + 1];
    assert!(matches!(
        verify_aich_proof(root.0, &oversized, &proof0).unwrap_err(),
        AichProofError::LeafTooLarge { .. }
    ));
}

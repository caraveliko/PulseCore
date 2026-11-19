// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Árboles de Merkle modernos sobre BLAKE3 y SHA-256 para verificación por bloques.
//!
//! Se aplica separación explícita de dominio entre hojas (`0x00 || datos`) y nodos
//! internos (`0x01 || izq || der`) para prevenir ataques de segundo preimagen.

use std::marker::PhantomData;

use sha2::{Digest, Sha256};

/// Tamaño de bloque alineado con subbloques AICH de eMule (180 KiB).
pub const DEFAULT_MERKLE_BLOCK_SIZE: usize = 184_320;
/// Tamaño de bloque alineado con árboles Merkle de BitTorrent v2 / BEP 52 (16 KiB).
pub const BT_V2_MERKLE_BLOCK_SIZE: usize = 16_384;
/// Longitud en bytes de cada nodo hash del árbol.
pub const MERKLE_HASH_LEN: usize = 32;

const LEAF_DOMAIN_TAG: u8 = 0x00;
const NODE_DOMAIN_TAG: u8 = 0x01;

/// Errores de construcción o verificación de pruebas de inclusión Merkle.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MerkleVerifyError {
    /// El tamaño de bloque debe ser estrictamente positivo.
    #[error("el tamaño de bloque debe ser mayor que 0")]
    ZeroBlockSize,
    /// El índice de bloque solicitado está fuera del rango del archivo.
    #[error("índice de bloque {index} fuera de rango (total {total})")]
    IndexOutOfRange {
        /// Índice solicitado.
        index: usize,
        /// Número total de hojas.
        total: usize,
    },
    /// El número de hermanos en la prueba no coincide con la geometría del árbol.
    #[error("número de hermanos inválido en la prueba Merkle")]
    InvalidProofShape,
    /// La raíz reconstruida no coincide con la raíz esperada.
    #[error("la raíz Merkle calculada no coincide con la esperada")]
    RootMismatch,
}

/// Abstracción interna de algoritmo hash de 32 bytes para evitar código duplicado.
pub trait HashAlgo32 {
    /// Calcula el resumen de una hoja (`0x00 || data`).
    fn hash_leaf(data: &[u8]) -> [u8; MERKLE_HASH_LEN];
    /// Calcula el resumen de un nodo interno (`0x01 || left || right`).
    fn hash_node(left: &[u8; MERKLE_HASH_LEN], right: &[u8; MERKLE_HASH_LEN])
        -> [u8; MERKLE_HASH_LEN];
}

/// Implementación basada en BLAKE3.
#[derive(Debug, Clone, Copy)]
pub struct Blake3Algo;

impl HashAlgo32 for Blake3Algo {
    fn hash_leaf(data: &[u8]) -> [u8; MERKLE_HASH_LEN] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&[LEAF_DOMAIN_TAG]);
        hasher.update(data);
        *hasher.finalize().as_bytes()
    }

    fn hash_node(
        left: &[u8; MERKLE_HASH_LEN],
        right: &[u8; MERKLE_HASH_LEN],
    ) -> [u8; MERKLE_HASH_LEN] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&[NODE_DOMAIN_TAG]);
        hasher.update(left);
        hasher.update(right);
        *hasher.finalize().as_bytes()
    }
}

/// Implementación basada en SHA-256 (compatible conceptualmente con BEP 52).
#[derive(Debug, Clone, Copy)]
pub struct Sha256Algo;

impl HashAlgo32 for Sha256Algo {
    fn hash_leaf(data: &[u8]) -> [u8; MERKLE_HASH_LEN] {
        let mut hasher = Sha256::new();
        hasher.update([LEAF_DOMAIN_TAG]);
        hasher.update(data);
        hasher.finalize().into()
    }

    fn hash_node(
        left: &[u8; MERKLE_HASH_LEN],
        right: &[u8; MERKLE_HASH_LEN],
    ) -> [u8; MERKLE_HASH_LEN] {
        let mut hasher = Sha256::new();
        hasher.update([NODE_DOMAIN_TAG]);
        hasher.update(left);
        hasher.update(right);
        hasher.finalize().into()
    }
}

/// Prueba de inclusión para verificar un único bloque frente a la raíz Merkle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleInclusionProof {
    /// Índice del bloque verificado (base 0).
    pub block_index: usize,
    /// Número total de bloques hoja en el árbol.
    pub total_leaves: usize,
    /// Secuencia de nodos hermanos desde la hoja hacia la raíz.
    pub siblings: Vec<[u8; MERKLE_HASH_LEN]>,
}

/// Árbol binario de Merkle parametrizado por algoritmo hash de 32 bytes.
#[derive(Debug, Clone)]
pub struct MerkleTree32<H: HashAlgo32> {
    levels: Vec<Vec<[u8; MERKLE_HASH_LEN]>>,
    leaf_count: usize,
    _algo: PhantomData<H>,
}

/// Árbol de Merkle sobre BLAKE3.
pub type Blake3MerkleTree = MerkleTree32<Blake3Algo>;
/// Árbol de Merkle sobre SHA-256.
pub type Sha256MerkleTree = MerkleTree32<Sha256Algo>;

impl<H: HashAlgo32> MerkleTree32<H> {
    /// Construye un árbol segmentando `data` en bloques de `block_size` bytes.
    pub fn from_data(data: &[u8], block_size: usize) -> Result<Self, MerkleVerifyError> {
        if block_size == 0 {
            return Err(MerkleVerifyError::ZeroBlockSize);
        }
        let leaves: Vec<[u8; MERKLE_HASH_LEN]> = data.chunks(block_size).map(H::hash_leaf).collect();
        Ok(Self::from_leaf_hashes(leaves))
    }

    /// Construye un árbol a partir de hashes de hojas ya calculados.
    pub fn from_leaf_hashes(leaves: Vec<[u8; MERKLE_HASH_LEN]>) -> Self {
        let leaf_count = leaves.len();
        if leaves.is_empty() {
            let empty_root = H::hash_leaf(&[]);
            return Self {
                levels: vec![vec![empty_root]],
                leaf_count: 0,
                _algo: PhantomData,
            };
        }

        let mut levels = Vec::new();
        levels.push(leaves);
        while let Some(prev) = levels.last() {
            if prev.len() <= 1 {
                break;
            }
            let mut next = Vec::with_capacity(prev.len().div_ceil(2));
            for pair in prev.chunks(2) {
                if pair.len() == 2 {
                    next.push(H::hash_node(&pair[0], &pair[1]));
                } else {
                    // Promoción directa del nodo impar al siguiente nivel.
                    next.push(pair[0]);
                }
            }
            levels.push(next);
        }

        Self {
            levels,
            leaf_count,
            _algo: PhantomData,
        }
    }

    /// Devuelve la raíz de 32 bytes del árbol.
    #[inline]
    pub fn root(&self) -> [u8; MERKLE_HASH_LEN] {
        self.levels
            .last()
            .and_then(|top| top.first().copied())
            .unwrap_or_else(|| H::hash_leaf(&[]))
    }

    /// Devuelve el número de hojas reales del árbol (0 para archivo vacío).
    #[inline]
    pub fn leaf_count(&self) -> usize {
        self.leaf_count
    }

    /// Genera la prueba de inclusión para el bloque `block_index`.
    pub fn prove_block(&self, block_index: usize) -> Result<MerkleInclusionProof, MerkleVerifyError> {
        if block_index >= self.leaf_count {
            return Err(MerkleVerifyError::IndexOutOfRange {
                index: block_index,
                total: self.leaf_count,
            });
        }
        let mut siblings = Vec::new();
        let mut idx = block_index;
        for level in &self.levels[..self.levels.len().saturating_sub(1)] {
            let sib_idx = idx ^ 1;
            if sib_idx < level.len() {
                siblings.push(level[sib_idx]);
            }
            idx /= 2;
        }
        Ok(MerkleInclusionProof {
            block_index,
            total_leaves: self.leaf_count,
            siblings,
        })
    }

    /// Verifica los bytes de un bloque frente a su prueba de inclusión y la raíz esperada.
    pub fn verify_block_data(
        expected_root: [u8; MERKLE_HASH_LEN],
        block_data: &[u8],
        proof: &MerkleInclusionProof,
    ) -> Result<(), MerkleVerifyError> {
        if proof.total_leaves == 0 {
            if proof.block_index == 0 && proof.siblings.is_empty() && block_data.is_empty() {
                return if H::hash_leaf(&[]) == expected_root {
                    Ok(())
                } else {
                    Err(MerkleVerifyError::RootMismatch)
                };
            }
            return Err(MerkleVerifyError::InvalidProofShape);
        }
        if proof.block_index >= proof.total_leaves {
            return Err(MerkleVerifyError::IndexOutOfRange {
                index: proof.block_index,
                total: proof.total_leaves,
            });
        }

        let mut cur = H::hash_leaf(block_data);
        let mut idx = proof.block_index;
        let mut level_len = proof.total_leaves;
        let mut sib_iter = proof.siblings.iter();

        while level_len > 1 {
            let sib_idx = idx ^ 1;
            if sib_idx < level_len {
                let sibling = sib_iter
                    .next()
                    .ok_or(MerkleVerifyError::InvalidProofShape)?;
                cur = if idx.is_multiple_of(2) {
                    H::hash_node(&cur, sibling)
                } else {
                    H::hash_node(sibling, &cur)
                };
            }
            idx /= 2;
            level_len = level_len.div_ceil(2);
        }

        if sib_iter.next().is_some() {
            return Err(MerkleVerifyError::InvalidProofShape);
        }

        if cur == expected_root {
            Ok(())
        } else {
            Err(MerkleVerifyError::RootMismatch)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_tree_empty_single_and_odd_blocks() {
        // Archivo vacío.
        let empty = Blake3MerkleTree::from_data(&[], 64).unwrap();
        assert_eq!(empty.leaf_count(), 0);
        assert!(empty.prove_block(0).is_err());
        let empty_proof = MerkleInclusionProof {
            block_index: 0,
            total_leaves: 0,
            siblings: vec![],
        };
        Blake3MerkleTree::verify_block_data(empty.root(), &[], &empty_proof).unwrap();

        // Un solo bloque.
        let one = Blake3MerkleTree::from_data(b"bloque-unico", 64).unwrap();
        assert_eq!(one.leaf_count(), 1);
        let p0 = one.prove_block(0).unwrap();
        Blake3MerkleTree::verify_block_data(one.root(), b"bloque-unico", &p0).unwrap();

        // Número impar de bloques (5 bloques con block_size=4 sobre 19 bytes).
        let payload = b"0123456789abcdefghi";
        let odd = Blake3MerkleTree::from_data(payload, 4).unwrap();
        assert_eq!(odd.leaf_count(), 5);
        for (i, chunk) in payload.chunks(4).enumerate() {
            let proof = odd.prove_block(i).unwrap();
            Blake3MerkleTree::verify_block_data(odd.root(), chunk, &proof).unwrap();
        }

        // Detección de corrupción de 1 bit.
        let proof2 = odd.prove_block(2).unwrap();
        let mut corrupted = payload[8..12].to_vec();
        corrupted[1] ^= 0x01;
        assert_eq!(
            Blake3MerkleTree::verify_block_data(odd.root(), &corrupted, &proof2),
            Err(MerkleVerifyError::RootMismatch)
        );
    }

    #[test]
    fn sha256_tree_power_of_two() {
        let payload = vec![0x7Au8; 16 * 8];
        let tree = Sha256MerkleTree::from_data(&payload, 16).unwrap();
        assert_eq!(tree.leaf_count(), 8);
        for (i, chunk) in payload.chunks(16).enumerate() {
            let proof = tree.prove_block(i).unwrap();
            Sha256MerkleTree::verify_block_data(tree.root(), chunk, &proof).unwrap();
        }
    }
}

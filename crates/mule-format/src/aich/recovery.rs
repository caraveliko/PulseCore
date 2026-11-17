// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Datos de recuperación de partes AICH (semántica de cable de eMule
//! `CAICHRecoveryHashSet::CreatePartRecoveryData` / `ReadRecoveryData`).
//!
//! Los datos de recuperación consisten en una lista de pares
//! `(hash_ident, SHA-1 hash)` donde el identificador codifica la ruta del nodo
//! dentro del árbol de hashes AICH: el identificador es un `u32`/`u16` cuyo
//! bit activo más significativo actúa como marcador inicial (la contribución
//! `is_left_branch = true` de la raíz); cada bit siguiente selecciona el hijo
//! en ese nivel (`1` = nodo de la rama izquierda, `0` = nodo de la rama derecha).
//!
//! En el cable, la lista se serializa como:
//!   `u16 count16` + `count16 * (u16 ident + hash[20])` +
//!   `u16 count32` + `count32 * (u32 ident + hash[20])`
//! Solo se rellena una de las dos listas (identificadores de 16 bits para
//! ficheros normales, y de 32 bits para ficheros grandes > 4 GiB).

use sha1::{Digest, Sha1};
use std::io::Read;

use crate::ed2k::hash::ED2K_PART_SIZE;
use crate::error::FormatError;

use super::hash_tree::{AichHash, AichNode, AICH_BLOCK_SIZE};

/// Una entrada de datos de recuperación: identificador de ruta en el árbol + hash SHA-1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AichRecoveryEntry {
    /// Identificador de hash (codificación de ruta en el árbol, marcador MSB incluido).
    pub ident: u32,
    /// Hash SHA-1 del nodo direccionado del árbol.
    pub hash: AichHash,
}

/// Construye el árbol de hashes AICH completo sobre `data` (todos los hashes de nodos calculados).
pub fn build_aich_tree(data: &[u8]) -> Result<AichNode, FormatError> {
    if data.is_empty() {
        return Err(FormatError::EmptyInput);
    }
    let mut tree = AichNode::new_tree(data.len() as u64)?;
    let mut ok = true;
    AichNode::for_each_block_range(data.len() as u64, |start, len| {
        let digest: [u8; 20] = Sha1::digest(&data[start as usize..(start + len) as usize]).into();
        if !tree.set_leaf_hash(start, len, AichHash(digest)) {
            ok = false;
        }
    });
    if !ok {
        return Err(FormatError::AichVerificationFailed(
            "failed to set leaf hash".to_string(),
        ));
    }
    tree.recalculate_hash()?;
    Ok(tree)
}

/// Construye el árbol de hashes AICH completo leyendo en flujo `file_size` bytes de `r`.
/// El uso de memoria permanece acotado a la estructura del árbol, no al contenido del archivo.
pub fn build_aich_tree_from_reader<R: Read>(
    r: &mut R,
    file_size: u64,
) -> Result<AichNode, FormatError> {
    if file_size == 0 {
        return Err(FormatError::EmptyInput);
    }
    let mut tree = AichNode::new_tree(file_size)?;
    let mut buf = vec![0u8; AICH_BLOCK_SIZE];
    let mut ok = true;
    let mut err = None;
    AichNode::for_each_block_range(file_size, |start, len| {
        let chunk_len = len as usize;
        if let Err(e) = r.read_exact(&mut buf[..chunk_len]) {
            err = Some(e);
            ok = false;
            return;
        }
        let digest: [u8; 20] = Sha1::digest(&buf[..chunk_len]).into();
        if !tree.set_leaf_hash(start, len, AichHash(digest)) {
            ok = false;
        }
    });
    if let Some(e) = err {
        return Err(e.into());
    }
    if !ok {
        return Err(FormatError::AichVerificationFailed(
            "failed to set leaf hash".to_string(),
        ));
    }
    tree.recalculate_hash()?;
    Ok(tree)
}

/// Profundidad del subárbol de parte bajo la raíz (`FindHash(..., &nLevel)` de eMule).
/// El subárbol de parte es el nodo que cubre `[part_start, part_start + part_size)`.
fn find_part_node(root: &AichNode, part_start: u64, part_size: u64) -> Option<(&AichNode, u8)> {
    fn walk(node: &AichNode, start: u64, size: u64, level: u8) -> Option<(&AichNode, u8)> {
        if start == 0 && size == node.data_size {
            return Some((node, level));
        }
        if node.data_size <= node.base_size {
            return None;
        }
        let n_blocks = node.data_size.div_ceil(node.base_size);
        let left_blocks = if node.is_left_branch {
            n_blocks.div_ceil(2)
        } else {
            n_blocks / 2
        };
        let left_size = left_blocks * node.base_size;
        if start < left_size {
            if start + size > left_size {
                return None;
            }
            walk(node.left.as_ref()?, start, size, level + 1)
        } else {
            walk(node.right.as_ref()?, start - left_size, size, level + 1)
        }
    }
    walk(root, part_start, part_size, 0)
}

/// Serializa los hashes hermanos a lo largo de la ruta hacia el subárbol de parte más todos
/// los hashes del nivel más bajo (bloque) de la parte, en orden de cable de eMule.
///
/// Equivalente a `CAICHRecoveryHashSet::CreatePartRecoveryData`:
/// `nHashesToWrite = (nLevel - 1) + ceil(partSize / EMBLOCKSIZE)`.
pub fn create_part_recovery_data(
    root: &AichNode,
    file_size: u64,
    part_index: u32,
) -> Result<Vec<AichRecoveryEntry>, FormatError> {
    if file_size == 0 || file_size <= AICH_BLOCK_SIZE as u64 {
        // eMule: no hay recuperación AICH para archivos menores que un bloque.
        return Err(FormatError::AichVerificationFailed(
            "file too small for AICH recovery data".to_string(),
        ));
    }
    if root.data_size != file_size {
        return Err(FormatError::AichVerificationFailed(
            "tree root does not cover the requested file size".to_string(),
        ));
    }
    let part_start = part_index as u64 * ED2K_PART_SIZE as u64;
    if part_start >= file_size {
        return Err(FormatError::AichVerificationFailed(
            "part index out of range".to_string(),
        ));
    }
    let part_size = (ED2K_PART_SIZE as u64).min(file_size - part_start);

    let mut out = Vec::new();
    if !create_recovery_inner(root, part_start, part_size, 0, &mut out) {
        return Err(FormatError::AichVerificationFailed(
            "failed to create recovery data".to_string(),
        ));
    }

    let (_, level) = find_part_node(root, part_start, part_size)
        .ok_or_else(|| FormatError::AichVerificationFailed("part node not found".to_string()))?;
    // eMule: nHashesToWrite = (nLevel - 1) + blocks, donde nLevel = depth + 1.
    let expected = level as usize + part_size.div_ceil(AICH_BLOCK_SIZE as u64) as usize;
    if out.len() != expected {
        return Err(FormatError::AichVerificationFailed(format!(
            "recovery data has {} entries, expected {expected}",
            out.len()
        )));
    }
    Ok(out)
}

/// Devuelve los hashes raíz del subárbol AICH por parte en orden de partes — el
/// `m_aAICHPartHashSet` serializado por `CFileIdentifier::WriteAICHHashsetToFile`
/// (`FileIdentifier.cpp` 0.72a) dentro de `OP_HASHSETANSWER2`.
///
/// `root` debe ser un árbol completamente hasheado que cubra exactamente `file_size` bytes.
/// `GetTheoreticalAICHPartHashCount` de eMule produce ningún conjunto de partes AICH para
/// archivos de a lo sumo una parte eD2K, por lo que se devuelve un `Vec` vacío en ese
/// caso (los llamadores omiten entonces el bit de opción `0x02` en el cable).
pub fn aich_part_hashes(
    root: &AichNode,
    file_size: u64,
) -> Result<Vec<AichHash>, FormatError> {
    if file_size <= ED2K_PART_SIZE as u64 {
        return Ok(Vec::new());
    }
    if root.data_size != file_size {
        return Err(FormatError::AichVerificationFailed(
            "tree root does not cover the requested file size".to_string(),
        ));
    }
    let part_count = file_size.div_ceil(ED2K_PART_SIZE as u64);
    let mut out = Vec::with_capacity(part_count as usize);
    for part in 0..part_count {
        let part_start = part * ED2K_PART_SIZE as u64;
        let part_size = (ED2K_PART_SIZE as u64).min(file_size - part_start);
        let (node, _) = find_part_node(root, part_start, part_size).ok_or_else(|| {
            FormatError::AichVerificationFailed("part node not found".to_string())
        })?;
        out.push(node.hash.ok_or_else(|| {
            FormatError::AichVerificationFailed("part node hash missing".to_string())
        })?);
    }
    Ok(out)
}

/// `CAICHHashTree::CreatePartRecoveryData`: escribe los hashes hermanos a lo largo del
/// descenso, y luego los hashes del nivel más bajo de la parte de izquierda a derecha.
fn create_recovery_inner(
    node: &AichNode,
    part_start: u64,
    part_size: u64,
    ident: u32,
    out: &mut Vec<AichRecoveryEntry>,
) -> bool {
    if part_start == 0 && part_size == node.data_size {
        return write_lowest_level_hashes(node, ident, out);
    }
    if node.data_size <= node.base_size {
        return false;
    }
    let ident = (ident << 1) | (node.is_left_branch as u32);
    let n_blocks = node.data_size.div_ceil(node.base_size);
    let left_blocks = if node.is_left_branch {
        n_blocks.div_ceil(2)
    } else {
        n_blocks / 2
    };
    let left_size = left_blocks * node.base_size;
    let (Some(left), Some(right)) = (&node.left, &node.right) else {
        return false;
    };
    if part_start < left_size {
        if part_start + part_size > left_size {
            return false;
        }
        if !write_ident_hash(right, ident, out) {
            return false;
        }
        return create_recovery_inner(left, part_start, part_size, ident, out);
    }
    if !write_ident_hash(left, ident, out) {
        return false;
    }
    create_recovery_inner(right, part_start - left_size, part_size, ident, out)
}

/// `CAICHHashTree::WriteHash`: `(ident << 1) | is_left_branch` seguido del hash.
fn write_ident_hash(node: &AichNode, ident: u32, out: &mut Vec<AichRecoveryEntry>) -> bool {
    let ident = (ident << 1) | (node.is_left_branch as u32);
    match node.hash {
        Some(hash) => {
            out.push(AichRecoveryEntry { ident, hash });
            true
        }
        None => false,
    }
}

/// `CAICHHashTree::WriteLowestLevelHashes` con identificadores.
fn write_lowest_level_hashes(
    node: &AichNode,
    ident: u32,
    out: &mut Vec<AichRecoveryEntry>,
) -> bool {
    let ident = (ident << 1) | (node.is_left_branch as u32);
    if node.left.is_none() && node.right.is_none() {
        return match (node.data_size <= node.base_size, node.hash) {
            (true, Some(hash)) => {
                out.push(AichRecoveryEntry { ident, hash });
                true
            }
            _ => false,
        };
    }
    let (Some(left), Some(right)) = (&node.left, &node.right) else {
        return false;
    };
    write_lowest_level_hashes(left, ident, out) && write_lowest_level_hashes(right, ident, out)
}

/// Establece un hash en el árbol esqueleto direccionado por `ident`
/// (`CAICHHashTree::SetHash`). Devuelve falso si el ident no direcciona un
/// nodo válido o intenta sobrescribir el hash maestro (ident == 1).
pub fn set_hash_by_ident(
    root: &mut AichNode,
    ident: u32,
    hash: AichHash,
    allow_overwrite: bool,
) -> bool {
    if ident == 0 || ident == 1 {
        return false;
    }
    let depth = 31 - ident.leading_zeros();
    let mut node = root;
    for i in (0..depth).rev() {
        if node.data_size <= node.base_size {
            return false;
        }
        let go_left = (ident >> i) & 1 == 1;
        node = if go_left {
            match node.left.as_mut() {
                Some(l) => l,
                None => return false,
            }
        } else {
            match node.right.as_mut() {
                Some(r) => r,
                None => return false,
            }
        };
    }
    if node.hash.is_some() && !allow_overwrite {
        // Hash ya presente: eMule omite la escritura pero acepta la entrada.
        return node.hash == Some(hash);
    }
    node.hash = Some(hash);
    true
}

/// Recalcula los hashes de los padres de abajo hacia arriba para nodos cuyo subárbol está completamente hasheado
/// (semántica de `CAICHHashTree::VerifyHashTree`). Devuelve el hash raíz resultante
/// si es determinable.
fn recalculate_partial(node: &mut AichNode) -> Option<AichHash> {
    if let Some(h) = node.hash {
        return Some(h);
    }
    let (left, right) = (&mut node.left, &mut node.right);
    match (left, right) {
        (Some(l), Some(r)) => {
            let lh = recalculate_partial(l)?;
            let rh = recalculate_partial(r)?;
            let mut hasher = Sha1::new();
            hasher.update(lh.as_bytes());
            hasher.update(rh.as_bytes());
            let digest: [u8; 20] = hasher.finalize().into();
            let h = AichHash(digest);
            node.hash = Some(h);
            Some(h)
        }
        _ => None,
    }
}

/// Lee los datos de recuperación recibidos para `part_index` de un archivo de `file_size` bytes y
/// devuelve el hash SHA-1 esperado de cada bloque de 180 KiB de esa parte, tras
/// verificar el árbol reconstruido frente a `master_hash`
/// (`CAICHRecoveryHashSet::ReadRecoveryData` + `VerifyHashTree`).
pub fn read_recovery_data(
    file_size: u64,
    part_index: u32,
    master_hash: AichHash,
    entries: &[AichRecoveryEntry],
) -> Result<Vec<AichHash>, FormatError> {
    if file_size == 0 {
        return Err(FormatError::EmptyInput);
    }
    if entries.is_empty() {
        return Err(FormatError::AichVerificationFailed(
            "recovery data contains no hashes".to_string(),
        ));
    }
    let part_start = part_index as u64 * ED2K_PART_SIZE as u64;
    if part_start >= file_size {
        return Err(FormatError::AichVerificationFailed(
            "part index out of range".to_string(),
        ));
    }
    let part_size = (ED2K_PART_SIZE as u64).min(file_size - part_start);
    let blocks_in_part = part_size.div_ceil(AICH_BLOCK_SIZE as u64) as usize;

    let mut tree = AichNode::new_tree(file_size)?;
    for entry in entries {
        if !set_hash_by_ident(&mut tree, entry.ident, entry.hash, false) {
            return Err(FormatError::AichVerificationFailed(format!(
                "invalid hash identifier {:#x} in recovery data",
                entry.ident
            )));
        }
    }

    match recalculate_partial(&mut tree) {
        Some(root) if root == master_hash => {}
        Some(_) => {
            return Err(FormatError::AichVerificationFailed(
                "reconstructed AICH root does not match master hash".to_string(),
            ))
        }
        None => {
            return Err(FormatError::AichVerificationFailed(
                "incomplete AICH tree after recovery data".to_string(),
            ))
        }
    }

    // Extraer los hashes de los bloques hoja de la parte en orden de izquierda a derecha.
    let mut block_hashes = Vec::with_capacity(blocks_in_part);
    collect_part_block_hashes(&tree, part_start, part_size, &mut block_hashes);
    if block_hashes.len() != blocks_in_part {
        return Err(FormatError::AichVerificationFailed(format!(
            "recovery data only covers {} of {blocks_in_part} blocks",
            block_hashes.len()
        )));
    }
    Ok(block_hashes)
}

fn collect_part_block_hashes(node: &AichNode, start: u64, size: u64, out: &mut Vec<AichHash>) {
    if node.data_size <= node.base_size {
        if node.data_size == size && start == 0 {
            if let Some(h) = node.hash {
                out.push(h);
            }
        }
        return;
    }
    let n_blocks = node.data_size.div_ceil(node.base_size);
    let left_blocks = if node.is_left_branch {
        n_blocks.div_ceil(2)
    } else {
        n_blocks / 2
    };
    let left_size = left_blocks * node.base_size;
    if start < left_size {
        if let Some(l) = &node.left {
            collect_part_block_hashes(l, start, size.min(left_size - start), out);
        }
        if start + size > left_size {
            if let Some(r) = &node.right {
                collect_part_block_hashes(r, 0, start + size - left_size, out);
            }
        }
    } else if let Some(r) = &node.right {
        collect_part_block_hashes(r, start - left_size, size, out);
    }
}

/// Hash SHA-1 de un único bloque AICH (<= 180 KiB de datos de archivo).
pub fn aich_block_hash(block: &[u8]) -> AichHash {
    AichHash(Sha1::digest(block).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data(blocks: usize) -> Vec<u8> {
        let mut data = Vec::with_capacity(blocks * AICH_BLOCK_SIZE);
        for i in 0..blocks {
            for j in 0..AICH_BLOCK_SIZE {
                data.push((i * 31 + j % 7) as u8);
            }
        }
        data
    }

    #[test]
    fn recovery_roundtrip_full_part() {
        // 3 bloques AICH en una única parte eD2K (parcial).
        let data = sample_data(3);
        let tree = build_aich_tree(&data).unwrap();
        let root = tree.root_hash().unwrap();

        let entries = create_part_recovery_data(&tree, data.len() as u64, 0).unwrap();
        assert_eq!(entries.len(), 3);

        let block_hashes = read_recovery_data(data.len() as u64, 0, root, &entries).unwrap();
        assert_eq!(block_hashes.len(), 3);
        for (i, h) in block_hashes.iter().enumerate() {
            let block = &data[i * AICH_BLOCK_SIZE..(i + 1) * AICH_BLOCK_SIZE];
            assert_eq!(*h, aich_block_hash(block));
        }
    }

    #[test]
    fn recovery_roundtrip_last_partial_block() {
        // 2 bloques completos + un bloque final de 100 bytes.
        let mut data = sample_data(2);
        data.extend_from_slice(&[0x55u8; 100]);
        let tree = build_aich_tree(&data).unwrap();
        let root = tree.root_hash().unwrap();

        let entries = create_part_recovery_data(&tree, data.len() as u64, 0).unwrap();
        let block_hashes = read_recovery_data(data.len() as u64, 0, root, &entries).unwrap();
        assert_eq!(block_hashes.len(), 3);
        assert_eq!(block_hashes[2], aich_block_hash(&[0x55u8; 100]));
    }

    #[test]
    fn recovery_rejects_wrong_master_hash() {
        let data = sample_data(3);
        let tree = build_aich_tree(&data).unwrap();
        let entries = create_part_recovery_data(&tree, data.len() as u64, 0).unwrap();

        let wrong = AichHash([0xEE; 20]);
        assert!(read_recovery_data(data.len() as u64, 0, wrong, &entries).is_err());
    }

    #[test]
    fn recovery_rejects_invalid_ident() {
        let data = sample_data(3);
        let tree = build_aich_tree(&data).unwrap();
        let root = tree.root_hash().unwrap();
        let mut entries = create_part_recovery_data(&tree, data.len() as u64, 0).unwrap();
        // ident 0 / 1 direccionan el hash maestro mismo — prohibido.
        entries[0].ident = 1;
        assert!(read_recovery_data(data.len() as u64, 0, root, &entries).is_err());
        entries[0].ident = 0;
        assert!(read_recovery_data(data.len() as u64, 0, root, &entries).is_err());
        // ident que apunta más allá de la profundidad del árbol.
        entries[0].ident = 0x7FFF_FFFF;
        assert!(read_recovery_data(data.len() as u64, 0, root, &entries).is_err());
    }

    #[test]
    fn recovery_rejects_incomplete_tree() {
        let data = sample_data(4);
        let tree = build_aich_tree(&data).unwrap();
        let root = tree.root_hash().unwrap();
        let mut entries = create_part_recovery_data(&tree, data.len() as u64, 0).unwrap();
        entries.pop();
        assert!(read_recovery_data(data.len() as u64, 0, root, &entries).is_err());
    }

    #[test]
    fn recovery_multi_part_tree() {
        // Un archivo que abarca el equivalente a 2 partes eD2K de bloques AICH es demasiado grande para una
        // prueba; emular un archivo de 60 bloques (manteniendo la semántica de límite de 1 parte completa).
        let data = sample_data(60);
        let tree = build_aich_tree(&data).unwrap();
        let root = tree.root_hash().unwrap();

        let entries = create_part_recovery_data(&tree, data.len() as u64, 0).unwrap();
        // La parte 0 cubre los bloques 0..53; se incluyen los hashes hermanos
        // para los 7 bloques restantes.
        let block_hashes = read_recovery_data(data.len() as u64, 0, root, &entries).unwrap();
        assert_eq!(block_hashes.len(), 53);
        // Los límites de bloque están alineados por partes: el bloque 52 cubre solo 143 360
        // bytes (resto de la parte 0).
        let mut expected = Vec::new();
        AichNode::for_each_block_range(data.len() as u64, |start, len| {
            expected.push(aich_block_hash(
                &data[start as usize..(start + len) as usize],
            ));
        });
        assert_eq!(block_hashes, expected[..53]);
    }

    #[test]
    fn streaming_reader_matches_in_memory() {
        let data = sample_data(5);
        let t1 = build_aich_tree(&data).unwrap();
        let mut cursor = std::io::Cursor::new(&data);
        let t2 = build_aich_tree_from_reader(&mut cursor, data.len() as u64).unwrap();
        assert_eq!(t1.root_hash(), t2.root_hash());
    }

    #[test]
    fn part_hashes_return_part_subtree_roots() {
        // Árbol sintético para un archivo de 2 partes + resto: `aich_part_hashes` debe
        // devolver el hash raíz del subárbol de cada parte en orden de partes (`m_aAICHPartHashSet`,
        // `WriteAICHHashsetToFile` de eMule).
        let file_size = 2 * ED2K_PART_SIZE as u64 + 1234;
        let mut tree = AichNode::new_tree(file_size).unwrap();
        let mut i = 0u8;
        AichNode::for_each_block_range(file_size, |start, len| {
            i = i.wrapping_add(1);
            assert!(tree.set_leaf_hash(start, len, AichHash([i; 20])));
        });
        tree.recalculate_hash().unwrap();

        let parts = aich_part_hashes(&tree, file_size).unwrap();
        assert_eq!(parts.len(), 3, "ceil(size/PARTSIZE) part hashes");
        // Cada entrada debe ser la raíz real del subárbol que cubre esa parte.
        for (idx, h) in parts.iter().enumerate() {
            let start = idx as u64 * ED2K_PART_SIZE as u64;
            let size = (ED2K_PART_SIZE as u64).min(file_size - start);
            let (node, _) = find_part_node(&tree, start, size).unwrap();
            assert_eq!(node.hash.as_ref().unwrap(), h);
        }

        // Archivo de una sola parte: `GetTheoreticalAICHPartHashCount` de eMule == 0.
        let small = sample_data(2);
        let small_tree = build_aich_tree(&small).unwrap();
        assert!(aich_part_hashes(&small_tree, small.len() as u64)
            .unwrap()
            .is_empty());
    }
}

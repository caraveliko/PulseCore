// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use md4::{Digest, Md4};
use std::fmt;
use std::io::Read;

use crate::error::FormatError;

/// Tamaño exacto de una parte eD2K: 9 728 000 bytes = 9 500 KiB.
pub const ED2K_PART_SIZE: usize = 9_728_000;

/// Resumen MD4 de un flujo vacío (0 bytes).
pub const MD4_EMPTY_DIGEST: [u8; 16] = [
    0x31, 0xd6, 0xcf, 0xe0, 0xd1, 0x6a, 0xe9, 0x31, 0xb7, 0x3c, 0x59, 0xd7, 0xe0, 0xc0, 0x89, 0xc0,
];

/// Hash eD2K de 16 bytes (resumen MD4).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Ed2kHash(pub [u8; 16]);

impl Ed2kHash {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, FormatError> {
        let trimmed = s.trim();
        if trimmed.len() != 32 {
            return Err(FormatError::InvalidEd2kHashString(s.to_string()));
        }
        let mut bytes = [0u8; 16];
        hex::decode_to_slice(trimmed, &mut bytes)
            .map_err(|_| FormatError::InvalidEd2kHashString(s.to_string()))?;
        Ok(Self(bytes))
    }
}

impl fmt::Debug for Ed2kHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ed2kHash({})", self.to_hex())
    }
}

impl fmt::Display for Ed2kHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// Resultado de hashear un archivo o flujo con el algoritmo eD2K.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kHashResult {
    /// Hash raíz eD2K final.
    pub root_hash: Ed2kHash,
    /// Lista de hashes individuales de partes (MD4 de cada parte).
    pub part_hashes: Vec<[u8; 16]>,
    /// Total de bytes procesados.
    pub total_bytes: u64,
}

/// Hasheador eD2K en flujo continuo.
pub struct Ed2kHasher {
    current_part_hasher: Md4,
    current_part_bytes: usize,
    part_hashes: Vec<[u8; 16]>,
    total_bytes: u64,
}

impl Default for Ed2kHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Ed2kHasher {
    pub fn new() -> Self {
        Self {
            current_part_hasher: Md4::new(),
            current_part_bytes: 0,
            part_hashes: Vec::new(),
            total_bytes: 0,
        }
    }

    /// Alimenta bloques arbitrarios de datos al hasheador.
    pub fn update(&mut self, mut data: &[u8]) {
        self.total_bytes += data.len() as u64;

        while !data.is_empty() {
            let space_in_part = ED2K_PART_SIZE - self.current_part_bytes;
            if data.len() < space_in_part {
                self.current_part_hasher.update(data);
                self.current_part_bytes += data.len();
                break;
            } else {
                let (head, tail) = data.split_at(space_in_part);
                self.current_part_hasher.update(head);
                let digest: [u8; 16] = self.current_part_hasher.finalize_reset().into();
                self.part_hashes.push(digest);
                self.current_part_bytes = 0;
                data = tail;
            }
        }
    }

    /// Finaliza el cálculo y devuelve el resultado del hash eD2K.
    ///
    /// Los flujos vacíos (0 bytes) se rechazan por defecto como archivos eD2K no válidos.
    pub fn finalize(mut self) -> Result<Ed2kHashResult, FormatError> {
        if self.total_bytes == 0 {
            return Err(FormatError::EmptyInput);
        }

        // Caso 1: El archivo es estrictamente menor que una parte
        if self.total_bytes < ED2K_PART_SIZE as u64 {
            let single_digest: [u8; 16] = self.current_part_hasher.finalize().into();
            return Ok(Ed2kHashResult {
                root_hash: Ed2kHash(single_digest),
                part_hashes: vec![single_digest],
                total_bytes: self.total_bytes,
            });
        }

        // Caso 2: El archivo es >= ED2K_PART_SIZE
        if self.current_part_bytes > 0 {
            // Había una parte parcial final
            let last_digest: [u8; 16] = self.current_part_hasher.finalize().into();
            self.part_hashes.push(last_digest);
        } else if self.total_bytes.is_multiple_of(ED2K_PART_SIZE as u64) {
            // Compatibilidad con eMule: los múltiplos exactos añaden MD4(vacío)
            self.part_hashes.push(MD4_EMPTY_DIGEST);
        }

        // Calcular el MD4 externo sobre la concatenación de todos los resúmenes de partes
        let mut outer_hasher = Md4::new();
        for ph in &self.part_hashes {
            outer_hasher.update(ph);
        }
        let root: [u8; 16] = outer_hasher.finalize().into();

        Ok(Ed2kHashResult {
            root_hash: Ed2kHash(root),
            part_hashes: self.part_hashes,
            total_bytes: self.total_bytes,
        })
    }
}

/// Obtiene la raíz eD2K de un archivo a partir de su conjunto de hashes de partes sin
/// volver a leer el disco — mismas reglas que [`Ed2kHasher::finalize`]:
/// archivo < 1 parte → la parte única es la raíz; de lo contrario MD4 de la
/// concatenación de los hashes de partes, con `MD4_EMPTY_DIGEST` añadido a los
/// múltiplos exactos de [`ED2K_PART_SIZE`] cuando el conjunto de hashes no lo incluya
/// ya. Devuelve `None` cuando el conjunto de hashes no cubra el archivo.
///
/// `part_hashes` puede contener `total_parts` o `total_parts + 1`
/// entradas (la parte vacía de eMule); cualquier otra cardinalidad es rechazada.
pub fn ed2k_root_from_part_hashes(total_bytes: u64, part_hashes: &[[u8; 16]]) -> Option<Ed2kHash> {
    if total_bytes == 0 || part_hashes.is_empty() {
        return None;
    }

    if total_bytes < ED2K_PART_SIZE as u64 {
        return Some(Ed2kHash(*part_hashes.first()?));
    }

    let real_parts = total_bytes.div_ceil(ED2K_PART_SIZE as u64);
    let exact_multiple = total_bytes.is_multiple_of(ED2K_PART_SIZE as u64);
    let len = part_hashes.len() as u64;

    let mut hashes: Vec<[u8; 16]> = part_hashes.to_vec();
    if exact_multiple {
        match len {
            l if l == real_parts => hashes.push(MD4_EMPTY_DIGEST),
            l if l == real_parts + 1 => {}
            _ => return None,
        }
    } else if len != real_parts {
        return None;
    }

    let mut outer_hasher = Md4::new();
    for ph in &hashes {
        outer_hasher.update(ph);
    }
    let root: [u8; 16] = outer_hasher.finalize().into();
    Some(Ed2kHash(root))
}

/// Auxiliar de conveniencia para hashear cualquier lector con un tamaño de búfer especificado.
pub fn hash_reader<R: Read>(reader: &mut R) -> Result<Ed2kHashResult, FormatError> {
    let mut hasher = Ed2kHasher::new();
    let mut buffer = [0u8; 64 * 1024]; // Búfer de lectura de 64 KiB
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_bytes(data: &[u8]) -> Ed2kHashResult {
        hash_reader(&mut std::io::Cursor::new(data)).expect("hash_reader")
    }

    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn root_from_part_hashes_sub_part_file() {
        let data = pattern(1234);
        let res = hash_bytes(&data);
        assert_eq!(
            ed2k_root_from_part_hashes(res.total_bytes, &res.part_hashes),
            Some(res.root_hash)
        );
    }

    #[test]
    fn root_from_part_hashes_multi_part_with_tail() {
        let data = pattern(ED2K_PART_SIZE + 123);
        let res = hash_bytes(&data);
        assert_eq!(res.part_hashes.len(), 2);
        assert_eq!(
            ed2k_root_from_part_hashes(res.total_bytes, &res.part_hashes),
            Some(res.root_hash)
        );
    }

    #[test]
    fn root_from_part_hashes_exact_multiple_accepts_both_cardinalities() {
        let data = pattern(ED2K_PART_SIZE * 2);
        let res = hash_bytes(&data);
        // Hashset al estilo eMule: parte final MD4(vacía) incluida.
        assert_eq!(res.part_hashes.len(), 3);
        assert_eq!(
            ed2k_root_from_part_hashes(res.total_bytes, &res.part_hashes),
            Some(res.root_hash)
        );
        // Hashset sin la parte vacía final → misma raíz derivada.
        let trimmed = &res.part_hashes[..2];
        assert_eq!(
            ed2k_root_from_part_hashes(res.total_bytes, trimmed),
            Some(res.root_hash)
        );
    }

    #[test]
    fn root_from_part_hashes_rejects_incomplete_or_empty_sets() {
        let data = pattern(ED2K_PART_SIZE + 123);
        let res = hash_bytes(&data);
        // Hashset truncado (1 parte en lugar de 2).
        assert_eq!(
            ed2k_root_from_part_hashes(res.total_bytes, &res.part_hashes[..1]),
            None
        );
        assert_eq!(ed2k_root_from_part_hashes(res.total_bytes, &[]), None);
        assert_eq!(ed2k_root_from_part_hashes(0, &res.part_hashes), None);
    }
}

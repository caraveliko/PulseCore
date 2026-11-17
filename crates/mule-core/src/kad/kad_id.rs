// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde::{Deserialize, Serialize};
use std::fmt;

/// Un ID de nodo o ID de clave de 128 bits de Kad.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KadId(pub [u8; 16]);

impl KadId {
    pub const ZERO: Self = Self([0u8; 16]);

    pub fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub fn random() -> Self {
        use rand::RngCore;
        let mut bytes = [0u8; 16];
        rand::rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 32 {
            return None;
        }
        let mut bytes = [0u8; 16];
        for i in 0..16 {
            bytes[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(Self(bytes))
    }

    /// Convierte un hash big-endian eD2K de 16 bytes (hash de archivo eD2K, hash de usuario o resumen MD4 de palabra clave)
    /// al formato de cable de Kad de eMule (donde cada DWORD de 4 bytes tiene los bytes invertidos,
    /// coincidiendo con CUInt128::SetValueBE / GetData de eMule).
    pub fn from_hash_be(hash: &[u8; 16]) -> Self {
        let mut res = [0u8; 16];
        for i in 0..4 {
            res[i * 4] = hash[i * 4 + 3];
            res[i * 4 + 1] = hash[i * 4 + 2];
            res[i * 4 + 2] = hash[i * 4 + 1];
            res[i * 4 + 3] = hash[i * 4];
        }
        KadId(res)
    }

    /// Convierte una cadena hexadecimal de hash eD2K al formato de cable de Kad de eMule.
    pub fn from_hash_hex(hex_str: &str) -> Option<Self> {
        let raw = KadId::from_hex(hex_str)?;
        Some(Self::from_hash_be(&raw.0))
    }

    /// Convierte un ID de cable de Kad de vuelta a un hash big-endian eD2K
    /// (coincidiendo con CUInt128::ToByteArray de eMule).
    pub fn to_hash_be(&self) -> [u8; 16] {
        let mut res = [0u8; 16];
        for i in 0..4 {
            res[i * 4] = self.0[i * 4 + 3];
            res[i * 4 + 1] = self.0[i * 4 + 2];
            res[i * 4 + 2] = self.0[i * 4 + 1];
            res[i * 4 + 3] = self.0[i * 4];
        }
        res
    }

    /// Convierte un ID de cable de Kad de vuelta a una cadena hexadecimal de hash eD2K.
    pub fn to_hash_hex(&self) -> String {
        hex::encode(self.to_hash_be())
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Devuelve el fragmento DWORD de 32 bits en `index` (0..4), donde el fragmento 0 es el DWORD más significativo.
    /// Coincide con `CUInt128::Get32BitChunk(index)` de eMule.
    #[inline]
    pub fn dword(&self, index: usize) -> u32 {
        let start = index * 4;
        u32::from_le_bytes([
            self.0[start],
            self.0[start + 1],
            self.0[start + 2],
            self.0[start + 3],
        ])
    }

    /// Calcula la distancia XOR entre dos IDs de Kad.
    pub fn xor_distance(&self, other: &KadId) -> KadId {
        let mut res = [0u8; 16];
        for (i, byte) in res.iter_mut().enumerate() {
            *byte = self.0[i] ^ other.0[i];
        }
        KadId(res)
    }

    /// Número de bits cero iniciales en este ID (visto como un entero de 128 bits,
    /// coincidiendo con el bit 0 de eMule en `(m_uData[0] >> 31) & 1` hasta el bit 127 en `(m_uData[3] >> 0) & 1`).
    pub fn leading_zeros(&self) -> u32 {
        let mut total = 0;
        for i in 0..4 {
            let dw = self.dword(i);
            let lz = dw.leading_zeros();
            total += lz;
            if lz < 32 {
                break;
            }
        }
        total
    }

    /// Comprueba si la distancia de este contacto a `target` está dentro de SEARCHTOLERANCE (0x01000000) de eMule.
    /// En C++ de eMule, los nodos descartan peticiones de búsqueda/publicación si `distance.Get32BitChunk(0) > SEARCHTOLERANCE`.
    #[inline]
    pub fn is_within_search_tolerance(&self, target: &KadId) -> bool {
        const SEARCH_TOLERANCE: u32 = 0x01000000;
        let dist = self.xor_distance(target);
        dist.dword(0) <= SEARCH_TOLERANCE
    }

    /// Calcula el índice de cubeta [0..127] para otro ID relativo a este ID local.
    /// Cubeta 0: ceros iniciales de distancia == 127 (difieren solo en el último bit).
    /// Cubeta 127: ceros iniciales de distancia == 0 (difieren en el bit más significativo).
    pub fn bucket_index_for(&self, other: &KadId) -> Option<usize> {
        let dist = self.xor_distance(other);
        let lz = dist.leading_zeros();
        if lz >= 128 {
            None // Mismo ID (la distancia es 0)
        } else {
            Some(127 - lz as usize)
        }
    }
}

impl Ord for KadId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        for i in 0..4 {
            let a = self.dword(i);
            let b = other.dword(i);
            match a.cmp(&b) {
                std::cmp::Ordering::Equal => continue,
                ord => return ord,
            }
        }
        std::cmp::Ordering::Equal
    }
}

impl PartialOrd for KadId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Debug for KadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KadId({})", self.to_hex())
    }
}

impl fmt::Display for KadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kad_id_dword_endian_swap_exact_vector() {
        // Hash big-endian MD4 sin procesar de eD2K
        let ed2k_hash = [
            0x00, 0x01, 0x02, 0x03, // DWORD 0
            0x04, 0x05, 0x06, 0x07, // DWORD 1
            0x08, 0x09, 0x0A, 0x0B, // DWORD 2
            0x0C, 0x0D, 0x0E, 0x0F, // DWORD 3
        ];

        // En CUInt128::SetValueBE de eMule en x86/x64 little-endian:
        // cada DWORD de 32 bits se almacena con los bytes invertidos:
        let expected_wire = [
            0x03, 0x02, 0x01, 0x00, 0x07, 0x06, 0x05, 0x04, 0x0B, 0x0A, 0x09, 0x08, 0x0F, 0x0E,
            0x0D, 0x0C,
        ];

        let kad_id = KadId::from_hash_be(&ed2k_hash);
        assert_eq!(kad_id.0, expected_wire);

        // Convirtiendo de vuelta al hash big-endian mediante to_hash_be()
        let roundtrip_hash = kad_id.to_hash_be();
        assert_eq!(roundtrip_hash, ed2k_hash);
    }

    #[test]
    fn test_kad_id_hex_conversions() {
        let hash_hex = "d37c98517e79ddc1688e27d1fe849be5";
        let kad_id = KadId::from_hash_hex(hash_hex).expect("valid hex");
        assert_eq!(kad_id.to_hash_hex(), hash_hex);
    }

    #[test]
    fn test_kad_id_dword_and_comparison() {
        // ID con DWORD 0 = 0x00000001 (distancia pequeña)
        let mut b1 = [0u8; 16];
        b1[0] = 0x01; // LSB little-endian de DWORD 0 -> 0x00000001
        let id_small = KadId(b1);
        assert_eq!(id_small.dword(0), 1);

        // ID con DWORD 0 = 0x01000000 (límite de SEARCHTOLERANCE)
        let mut b2 = [0u8; 16];
        b2[3] = 0x01; // el byte 3 es MSB de DWORD 0 -> 0x01000000
        let id_boundary = KadId(b2);
        assert_eq!(id_boundary.dword(0), 0x01000000);

        // ID con DWORD 0 = 0x02000000 (supera SEARCHTOLERANCE)
        let mut b3 = [0u8; 16];
        b3[3] = 0x02;
        let id_large = KadId(b3);
        assert_eq!(id_large.dword(0), 0x02000000);

        // Ord debe comparar DWORD 0 primero: id_small < id_boundary < id_large
        assert!(id_small < id_boundary);
        assert!(id_boundary < id_large);

        // Comprobación de tolerancia
        let target = KadId::ZERO;
        assert!(id_small.is_within_search_tolerance(&target));
        assert!(id_boundary.is_within_search_tolerance(&target));
        assert!(!id_large.is_within_search_tolerance(&target));
    }
}

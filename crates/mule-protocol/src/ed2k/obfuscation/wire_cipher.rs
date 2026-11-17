// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Implementación de la permutación de flujo de ofuscación conforme a la especificación de eMule (`drop1024`).
//!
//! Este módulo implementa la permutación de flujo simétrica (KSA/PRGA de 256 bytes) requerida
//! estrictamente por el protocolo de ofuscación de cable de eMule 0.47b+ para la evasión de inspección
//! profunda de paquetes (DPI) en redes de pares eD2K y datagramas Kad. No constituye un cifrador
//! de propósito general para la confidencialidad de datos.

/// Cifrador de flujo de ofuscación de cable de eMule con tabla de permutación de 256 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireObfuscationCipher {
    state: [u8; 256],
    x: u8,
    y: u8,
}

impl WireObfuscationCipher {
    /// Inicializa la permutación de ofuscación con la clave suministrada.
    ///
    /// Si `skip_discard` es `false`, descarta los primeros 1024 bytes de flujo de claves (`drop1024`),
    /// según lo exigido por el protocolo de ofuscación de eMule.
    pub fn new(key: &[u8], skip_discard: bool) -> Self {
        assert!(!key.is_empty(), "La clave de ofuscación no puede estar vacía");
        let mut state = [0u8; 256];
        for (i, item) in state.iter_mut().enumerate() {
            *item = i as u8;
        }
        let mut j: u8 = 0;
        let mut key_idx = 0;
        for i in 0..256 {
            j = j.wrapping_add(state[i]).wrapping_add(key[key_idx]);
            state.swap(i, j as usize);
            key_idx = (key_idx + 1) % key.len();
        }
        let mut cipher = Self { state, x: 0, y: 0 };
        if !skip_discard {
            cipher.drop_bytes(1024);
        }
        cipher
    }

    /// Crea un cifrador de ofuscación estándar de eMule con los primeros 1024 bytes descartados.
    pub fn new_drop1024(key: &[u8]) -> Self {
        Self::new(key, false)
    }

    /// Descarta `count` bytes de flujo de permutación sin modificar la carga útil.
    pub fn drop_bytes(&mut self, count: usize) {
        for _ in 0..count {
            self.x = self.x.wrapping_add(1);
            self.y = self.y.wrapping_add(self.state[self.x as usize]);
            self.state.swap(self.x as usize, self.y as usize);
        }
    }

    /// Ofusca o desofusca bytes in situ mediante la permutación de cable.
    pub fn process(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            self.x = self.x.wrapping_add(1);
            self.y = self.y.wrapping_add(self.state[self.x as usize]);
            self.state.swap(self.x as usize, self.y as usize);
            let xor_index = self.state[self.x as usize].wrapping_add(self.state[self.y as usize]);
            let k = self.state[xor_index as usize];
            *byte ^= k;
        }
    }

    /// Devuelve los índices internos actuales `(x, y)` para depuración o inspección de estado.
    #[inline]
    pub fn indices(&self) -> (u8, u8) {
        (self.x, self.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_permutation_rfc6229_vector() {
        // Vector de prueba de permutación estándar RFC 6229: Clave 0102030405
        let key = [0x01, 0x02, 0x03, 0x04, 0x05];
        let mut cipher = WireObfuscationCipher::new(&key, true);
        let mut plaintext = vec![0u8; 16];
        cipher.process(&mut plaintext);
        // Primeros 16 bytes de RFC 6229: B2 39 63 05 F0 3D C0 27 CC C3 52 4A 0A 11 18 A8
        assert_eq!(
            hex::encode(&plaintext).to_uppercase(),
            "B2396305F03DC027CCC3524A0A1118A8"
        );
    }

    #[test]
    fn test_stream_permutation_drop1024_bidirectional() {
        let key = b"emule_test_key_16b";
        let mut enc = WireObfuscationCipher::new_drop1024(key);
        let mut dec = WireObfuscationCipher::new_drop1024(key);

        let original = b"Hello, eMule Obfuscation Protocol World! 0xE3 PR_ED2K";
        let mut buffer = original.to_vec();

        enc.process(&mut buffer);
        assert_ne!(
            &buffer, original,
            "Los datos ofuscados deben diferir del texto en claro"
        );
        assert_ne!(buffer[0], 0xE3, "El primer byte no debe ser 0xE3");

        dec.process(&mut buffer);
        assert_eq!(&buffer, original, "Los datos desofuscados deben coincidir con el original");
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Implementación del cifrado de flujo RC4 conforme a la ofuscación de protocolo de eMule (`RC4-drop1024`).

/// Cifrador de flujo RC4 determinista con estado de 256 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rc4Cipher {
    state: [u8; 256],
    x: u8,
    y: u8,
}

impl Rc4Cipher {
    /// Crea un nuevo `Rc4Cipher` con la configuración de clave KSA estándar.
    ///
    /// Si `skip_discard` es `false`, descarta los primeros 1024 bytes de flujo de claves (`RC4-drop1024`),
    /// según lo especificado en la ofuscación de protocolo de eMule.
    pub fn new(key: &[u8], skip_discard: bool) -> Self {
        assert!(!key.is_empty(), "RC4 key cannot be empty");
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

    /// Descarta `count` bytes de flujo de claves sin modificar ninguna carga útil.
    pub fn drop_bytes(&mut self, count: usize) {
        for _ in 0..count {
            self.x = self.x.wrapping_add(1);
            self.y = self.y.wrapping_add(self.state[self.x as usize]);
            self.state.swap(self.x as usize, self.y as usize);
        }
    }

    /// Cifra o descifra bytes in situ utilizando el flujo de claves PRGA.
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

    /// Devuelve los índices internos actuales `(x, y)` para depuración o inspecciones de estado.
    #[inline]
    pub fn indices(&self) -> (u8, u8) {
        (self.x, self.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rc4_rfc6229_vector() {
        // Vector de prueba RFC 6229: Clave 0102030405
        let key = [0x01, 0x02, 0x03, 0x04, 0x05];
        let mut cipher = Rc4Cipher::new(&key, true); // no drop
        let mut plaintext = vec![0u8; 16];
        cipher.process(&mut plaintext);
        // Primeros 16 bytes de RFC 6229: B2 39 63 05 F0 3D C0 27 CC C3 52 4A 0A 11 18 A8
        assert_eq!(
            hex::encode(&plaintext).to_uppercase(),
            "B2396305F03DC027CCC3524A0A1118A8"
        );
    }

    #[test]
    fn test_rc4_drop1024_bidirectional() {
        let key = b"emule_test_key_16b";
        let mut enc = Rc4Cipher::new_drop1024(key);
        let mut dec = Rc4Cipher::new_drop1024(key);

        let original = b"Hello, eMule Obfuscation Protocol World! 0xE3 PR_ED2K";
        let mut buffer = original.to_vec();

        enc.process(&mut buffer);
        assert_ne!(
            &buffer, original,
            "Encrypted data must differ from plaintext"
        );
        assert_ne!(buffer[0], 0xE3, "First byte must not be 0xE3");

        dec.process(&mut buffer);
        assert_eq!(&buffer, original, "Decrypted data must match original");
    }
}

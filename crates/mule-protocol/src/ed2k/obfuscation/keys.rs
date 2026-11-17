// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Derivación de claves para la ofuscación del protocolo de eMule (MD5 sobre UserHash y secretos DH).

use md5::{Digest, Md5};

use super::rc4::Rc4Cipher;

/// Modificación de la clave de envío del solicitante y recepción del servidor (`34` = `0x22`).
pub const MAGICVALUE_REQUESTER: u8 = 34;

/// Modificación de la clave de envío del servidor y recepción del solicitante (`203` = `0xCB`).
pub const MAGICVALUE_SERVER: u8 = 203;

/// Deriva cifradores RC4-drop1024 para el Cliente A (iniciador de la conexión de par saliente).
///
/// Devuelve `(send_cipher, recv_cipher)`.
pub fn derive_initiator_peer_ciphers(
    target_user_hash: &[u8; 16],
    random_key_part: u32,
) -> (Rc4Cipher, Rc4Cipher) {
    let (send_key, recv_key) = derive_initiator_peer_raw_keys(target_user_hash, random_key_part);
    (
        Rc4Cipher::new_drop1024(&send_key),
        Rc4Cipher::new_drop1024(&recv_key),
    )
}

/// Deriva las claves sin procesar de 16 bytes `(send_key, recv_key)` para el Cliente A (iniciador).
pub fn derive_initiator_peer_raw_keys(
    target_user_hash: &[u8; 16],
    random_key_part: u32,
) -> ([u8; 16], [u8; 16]) {
    // SendKey = MD5(target_user_hash[16] || 34 || random_key_part[4])
    let mut mat_send = [0u8; 21];
    mat_send[..16].copy_from_slice(target_user_hash);
    mat_send[16] = MAGICVALUE_REQUESTER;
    mat_send[17..21].copy_from_slice(&random_key_part.to_le_bytes());
    let send_key: [u8; 16] = Md5::digest(mat_send).into();

    // RecvKey = MD5(target_user_hash[16] || 203 || random_key_part[4])
    let mut mat_recv = [0u8; 21];
    mat_recv[..16].copy_from_slice(target_user_hash);
    mat_recv[16] = MAGICVALUE_SERVER;
    mat_recv[17..21].copy_from_slice(&random_key_part.to_le_bytes());
    let recv_key: [u8; 16] = Md5::digest(mat_recv).into();

    (send_key, recv_key)
}

/// Deriva cifradores RC4-drop1024 para el Cliente B (receptor de la conexión de par entrante).
///
/// Devuelve `(send_cipher, recv_cipher)`.
pub fn derive_responder_peer_ciphers(
    local_user_hash: &[u8; 16],
    random_key_part: u32,
) -> (Rc4Cipher, Rc4Cipher) {
    let (send_key, recv_key) = derive_responder_peer_raw_keys(local_user_hash, random_key_part);
    (
        Rc4Cipher::new_drop1024(&send_key),
        Rc4Cipher::new_drop1024(&recv_key),
    )
}

/// Deriva las claves sin procesar de 16 bytes `(send_key, recv_key)` para el Cliente B (respondedor).
pub fn derive_responder_peer_raw_keys(
    local_user_hash: &[u8; 16],
    random_key_part: u32,
) -> ([u8; 16], [u8; 16]) {
    // Responder SendKey = MD5(local_user_hash[16] || 203 || random_key_part[4])
    let mut mat_send = [0u8; 21];
    mat_send[..16].copy_from_slice(local_user_hash);
    mat_send[16] = MAGICVALUE_SERVER;
    mat_send[17..21].copy_from_slice(&random_key_part.to_le_bytes());
    let send_key: [u8; 16] = Md5::digest(mat_send).into();

    // Responder RecvKey = MD5(local_user_hash[16] || 34 || random_key_part[4])
    let mut mat_recv = [0u8; 21];
    mat_recv[..16].copy_from_slice(local_user_hash);
    mat_recv[16] = MAGICVALUE_REQUESTER;
    mat_recv[17..21].copy_from_slice(&random_key_part.to_le_bytes());
    let recv_key: [u8; 16] = Md5::digest(mat_recv).into();

    (send_key, recv_key)
}

/// Deriva cifradores RC4-drop1024 para una conexión Cliente <-> Servidor a partir del secreto compartido DH.
///
/// Devuelve `(send_cipher, recv_cipher)`.
pub fn derive_server_ciphers(shared_secret: &[u8; 96]) -> (Rc4Cipher, Rc4Cipher) {
    let mut mat_send = [0u8; 97];
    mat_send[..96].copy_from_slice(shared_secret);
    mat_send[96] = MAGICVALUE_REQUESTER;
    let send_key: [u8; 16] = Md5::digest(mat_send).into();

    let mut mat_recv = [0u8; 97];
    mat_recv[..96].copy_from_slice(shared_secret);
    mat_recv[96] = MAGICVALUE_SERVER;
    let recv_key: [u8; 16] = Md5::digest(mat_recv).into();

    (
        Rc4Cipher::new_drop1024(&send_key),
        Rc4Cipher::new_drop1024(&recv_key),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_peer_key_symmetry() {
        let user_hash_b = [0x55u8; 16];
        let random_key_part_a = 0x12345678u32;

        let (a_send_key, a_recv_key) =
            derive_initiator_peer_raw_keys(&user_hash_b, random_key_part_a);
        let (b_send_key, b_recv_key) =
            derive_responder_peer_raw_keys(&user_hash_b, random_key_part_a);

        // Client A's SendKey must match Client B's RecvKey!
        assert_eq!(
            a_send_key, b_recv_key,
            "Client A SendKey must equal Client B RecvKey"
        );
        // Client B's SendKey must match Client A's RecvKey!
        assert_eq!(
            b_send_key, a_recv_key,
            "Client B SendKey must equal Client A RecvKey"
        );
        // Send and Receive keys must differ
        assert_ne!(a_send_key, a_recv_key);
    }
}

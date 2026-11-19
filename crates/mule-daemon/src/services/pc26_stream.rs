// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Entramado asíncrono de saludo y registros AEAD `PC26` (X25519 / híbrido ML-KEM-768).
//!
//! Separa la negociación criptográfica moderna y el búfer de registros cifrados
//! del envoltorio general [`crate::services::obfuscation_stream::ObfuscatedStream`].

use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use mule_crypto::pc26_handshake::{
    Pc26AeadCipher, Pc26Error, Pc26Initiator, Pc26Responder, Pc26Suite, PC26_AEAD_TAG_LEN,
    PC26_MAX_RECORD_PLAINTEXT,
};
use mule_protocol::ed2k::obfuscation::PC26_OPPORTUNISTIC_MAGIC;

/// Tamaño máximo aceptado para mensajes de saludo `ClientHello` / `ServerHello` en cable.
pub const PC26_MAX_HANDSHAKE_MSG_LEN: usize = 4096;

/// Estado de cifrado autenticado activo para una conexión `PC26`.
pub(crate) struct Pc26AeadState {
    pub(crate) send_cipher: Pc26AeadCipher,
    pub(crate) recv_cipher: Pc26AeadCipher,
    pub(crate) suite: Pc26Suite,
    pub(crate) transcript_hash: [u8; 32],
    /// Búfer acumulador de bytes cifrados recibidos antes de completar un registro AEAD.
    pub(crate) recv_cipher_buf: Vec<u8>,
}

impl Pc26AeadState {
    /// Intenta descifrar un registro completo desde `recv_cipher_buf`.
    /// Devuelve `Ok(Some(texto_plano))` si había un registro íntegro, `Ok(None)` si faltan bytes.
    pub(crate) fn try_open_next_record(&mut self) -> io::Result<Option<Vec<u8>>> {
        if self.recv_cipher_buf.len() < 2 {
            return Ok(None);
        }
        let plain_len_u16 =
            u16::from_le_bytes([self.recv_cipher_buf[0], self.recv_cipher_buf[1]]);
        let plain_len = usize::from(plain_len_u16);
        if plain_len == 0 || plain_len > PC26_MAX_RECORD_PLAINTEXT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("longitud de registro PC26 fuera de límites: {plain_len}"),
            ));
        }
        let total_record_len = 2 + plain_len + PC26_AEAD_TAG_LEN;
        if self.recv_cipher_buf.len() < total_record_len {
            return Ok(None);
        }
        let plain = self
            .recv_cipher
            .open_record(&self.recv_cipher_buf[2..total_record_len], plain_len_u16)
            .map_err(pc26_to_io_err)?;
        self.recv_cipher_buf.drain(..total_record_len);
        Ok(Some(plain))
    }
}

fn pc26_to_io_err(e: Pc26Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("PC26: {e}"))
}

/// Ejecuta el saludo oportunista `PC26` saliente: envía `"PC26" || len || ClientHello`
/// y lee `len || ServerHello`, devolviendo el transporte listo para registros AEAD.
pub(crate) async fn connect_pc26_stream_outgoing<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    suite: Pc26Suite,
) -> io::Result<(S, Pc26AeadState)> {
    let (initiator, client_hello) = Pc26Initiator::new(suite).map_err(pc26_to_io_err)?;
    let ch_len = u16::try_from(client_hello.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "ClientHello PC26 excede u16::MAX",
        )
    })?;

    let mut out = Vec::with_capacity(PC26_OPPORTUNISTIC_MAGIC.len() + 2 + client_hello.len());
    out.extend_from_slice(&PC26_OPPORTUNISTIC_MAGIC);
    out.extend_from_slice(&ch_len.to_le_bytes());
    out.extend_from_slice(&client_hello);
    stream.write_all(&out).await?;
    stream.flush().await?;

    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf).await?;
    let sh_len = usize::from(u16::from_le_bytes(len_buf));
    if sh_len == 0 || sh_len > PC26_MAX_HANDSHAKE_MSG_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("ServerHello PC26 de longitud inválida: {sh_len}"),
        ));
    }

    let mut server_hello = vec![0u8; sh_len];
    stream.read_exact(&mut server_hello).await?;

    let keys = initiator.finish(&server_hello).map_err(pc26_to_io_err)?;
    let negotiated = keys.suite;
    let transcript_hash = keys.transcript_hash;
    let (send_cipher, recv_cipher) = keys.into_initiator_ciphers().map_err(pc26_to_io_err)?;

    Ok((
        stream,
        Pc26AeadState {
            send_cipher,
            recv_cipher,
            suite: negotiated,
            transcript_hash,
            recv_cipher_buf: Vec::new(),
        },
    ))
}

/// Completa el saludo `PC26` entrante justo después de verificar el prefijo `"PC26"`.
pub(crate) async fn accept_pc26_after_magic<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    allow_pq: bool,
) -> io::Result<(S, Pc26AeadState)> {
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf).await?;
    let ch_len = usize::from(u16::from_le_bytes(len_buf));
    if ch_len == 0 || ch_len > PC26_MAX_HANDSHAKE_MSG_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("ClientHello PC26 de longitud inválida: {ch_len}"),
        ));
    }

    let mut client_hello = vec![0u8; ch_len];
    stream.read_exact(&mut client_hello).await?;

    let (server_hello, keys) = Pc26Responder::accept(&client_hello, allow_pq).map_err(pc26_to_io_err)?;
    let sh_len = u16::try_from(server_hello.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "ServerHello PC26 excede u16::MAX",
        )
    })?;

    let mut out = Vec::with_capacity(2 + server_hello.len());
    out.extend_from_slice(&sh_len.to_le_bytes());
    out.extend_from_slice(&server_hello);
    stream.write_all(&out).await?;
    stream.flush().await?;

    let negotiated = keys.suite;
    let transcript_hash = keys.transcript_hash;
    let (send_cipher, recv_cipher) = keys.into_responder_ciphers().map_err(pc26_to_io_err)?;

    Ok((
        stream,
        Pc26AeadState {
            send_cipher,
            recv_cipher,
            suite: negotiated,
            transcript_hash,
            recv_cipher_buf: Vec::new(),
        },
    ))
}

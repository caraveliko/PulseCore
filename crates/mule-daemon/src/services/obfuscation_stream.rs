// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use crate::services::pc26_stream::{
    accept_pc26_after_magic, connect_pc26_stream_outgoing, Pc26AeadState,
};
use mule_crypto::pc26_handshake::{Pc26Suite, PC26_MAX_RECORD_PLAINTEXT};
use mule_protocol::ed2k::obfuscation::{
    derive_initiator_peer_ciphers, derive_responder_peer_ciphers, derive_server_ciphers,
    generate_semi_random_marker, Dh768KeyPair, WireObfuscationCipher, ENM_OBFUSCATION,
    MAGICVALUE_SYNC,
};
use rand::Rng;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

/// Tamaño máximo del búfer acumulador de bytes cifrados `PC26` pendientes de completar registro (256 KiB).
const MAX_PC26_CIPHER_BUF_BYTES: usize = 256 * 1024;
/// Límite de fragmento cifrado por escritura en modo RC4 clásico.
const RC4_WRITE_CHUNK_BYTES: usize = 8192;

enum CipherBackend {
    Plaintext,
    LegacyRc4 {
        send: Box<WireObfuscationCipher>,
        recv: Box<WireObfuscationCipher>,
    },
    Pc26Aead(Box<Pc26AeadState>),
}

/// Un envoltorio de flujo bidireccional que gestiona de forma transparente la ofuscación
/// clásica RC4 de eMule, el transporte AEAD moderno `PC26` (X25519 / híbrido ML-KEM-768)
/// o la transmisión en texto sin formato.
pub struct ObfuscatedStream<S> {
    inner: S,
    backend: CipherBackend,
    read_prefix: Vec<u8>,
    read_prefix_offset: usize,
    write_buf: Vec<u8>,
    write_offset: usize,
}

impl<S> ObfuscatedStream<S> {
    /// Crea un nuevo flujo no cifrado (texto sin formato).
    pub fn plaintext(inner: S) -> Self {
        Self {
            inner,
            backend: CipherBackend::Plaintext,
            read_prefix: Vec::new(),
            read_prefix_offset: 0,
            write_buf: Vec::new(),
            write_offset: 0,
        }
    }

    /// Crea un flujo no cifrado con un búfer de bytes leído previamente (útil para conexiones entrantes
    /// donde se inspeccionó el primer byte para comprobar si se solicitó ofuscación).
    pub fn plaintext_with_prefix(inner: S, prefix: Vec<u8>) -> Self {
        Self {
            inner,
            backend: CipherBackend::Plaintext,
            read_prefix: prefix,
            read_prefix_offset: 0,
            write_buf: Vec::new(),
            write_offset: 0,
        }
    }

    /// Devuelve si este flujo está actualmente cifrado (RC4 clásico o AEAD `PC26`).
    pub fn is_obfuscated(&self) -> bool {
        !matches!(self.backend, CipherBackend::Plaintext)
    }

    /// Devuelve `true` si la sesión utiliza transporte autenticado moderno `PC26-AEAD`.
    pub fn is_modern_aead(&self) -> bool {
        matches!(self.backend, CipherBackend::Pc26Aead(_))
    }

    /// Devuelve la suite criptográfica `PC26` negociada, si el flujo usa AEAD moderno.
    pub fn pc26_suite(&self) -> Option<Pc26Suite> {
        match &self.backend {
            CipherBackend::Pc26Aead(st) => Some(st.suite),
            _ => None,
        }
    }

    /// Devuelve la huella SHA-256 de la transcripción del saludo `PC26` para vinculación de canal.
    pub fn transcript_hash(&self) -> Option<[u8; 32]> {
        match &self.backend {
            CipherBackend::Pc26Aead(st) => Some(st.transcript_hash),
            _ => None,
        }
    }

    /// Devuelve una referencia al flujo de transporte subyacente.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// Consume el envoltorio y devuelve el flujo subyacente.
    pub fn into_inner(self) -> S {
        self.inner
    }

    /// Reinyecta bytes ya leídos (y ya descifrados si el flujo está ofuscado) al inicio
    /// del búfer de prefijo para que la siguiente fase de lectura (`poll_read`) los consuma
    /// sin perder paquetes concatenados en el mismo segmento TCP.
    pub fn put_back_unread(&mut self, leftover: &[u8]) {
        if leftover.is_empty() {
            return;
        }
        if self.read_prefix_offset >= self.read_prefix.len() {
            self.read_prefix.clear();
            self.read_prefix_offset = 0;
            self.read_prefix.extend_from_slice(leftover);
        } else {
            let remaining = &self.read_prefix[self.read_prefix_offset..];
            let mut combined = Vec::with_capacity(leftover.len() + remaining.len());
            combined.extend_from_slice(leftover);
            combined.extend_from_slice(remaining);
            self.read_prefix = combined;
            self.read_prefix_offset = 0;
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> ObfuscatedStream<S> {
    /// Realiza un saludo de ofuscación cliente a cliente eD2K saliente según las especificaciones de eMule.
    ///
    /// El iniciador genera una parte de clave aleatoria de 32 bits y deriva:
    /// - SendKey = MD5(TargetUserHash || 34 || RandomKeyPart)
    /// - RecvKey = MD5(TargetUserHash || 203 || RandomKeyPart)
    ///
    /// Envía:
    /// - `[SemiRandomMarker (1B), RandomKeyPart (4B)]` (no cifrado)
    /// - `[MAGICVALUE_SYNC (4B), ENM_OBFUSCATION (1B), ENM_OBFUSCATION (1B), PadLen (1B), RandomPadding]` (cifrado)
    ///
    /// Y aguarda la respuesta cifrada del respondedor:
    /// - `[MAGICVALUE_SYNC (4B), ENM_OBFUSCATION (1B), RespPadLen (1B), RespRandomPadding]`
    pub async fn connect_peer_outgoing(
        mut stream: S,
        target_user_hash: &[u8; 16],
        max_padding: u8,
    ) -> Result<Self, io::Error> {
        let (random_key_part, semi_random_marker, padding) = {
            let mut rng = rand::rng();
            let random_key_part: u32 = rng.random();
            let semi_random_marker = generate_semi_random_marker(&mut rng);
            let pad_len = if max_padding > 0 {
                rng.random_range(0..=max_padding)
            } else {
                0
            };
            let mut padding = Vec::with_capacity(pad_len as usize);
            for _ in 0..pad_len {
                padding.push(rng.random());
            }
            (random_key_part, semi_random_marker, padding)
        };

        // Derivar cifradores (RC4 con 1024 bytes descartados)
        let (mut send_cipher, mut recv_cipher) =
            derive_initiator_peer_ciphers(target_user_hash, random_key_part);

        // Preparar cabecera no cifrada de 5 bytes: [semi_random_marker, random_key_part (little-endian)]
        let mut header = [0u8; 5];
        header[0] = semi_random_marker;
        header[1..5].copy_from_slice(&random_key_part.to_le_bytes());

        // Preparar carga útil del iniciador cifrada
        let mut payload = Vec::with_capacity(7 + padding.len());
        payload.extend_from_slice(&MAGICVALUE_SYNC.to_le_bytes());
        payload.push(ENM_OBFUSCATION); // Supported encryption method
        payload.push(ENM_OBFUSCATION); // Preferred encryption method
        payload.push(padding.len() as u8);
        payload.extend_from_slice(&padding);

        // Cifrar carga útil con send_cipher
        send_cipher.process(&mut payload);

        // Escribir cabecera y carga útil cifrada en el flujo
        stream.write_all(&header).await?;
        stream.write_all(&payload).await?;

        // Leer y verificar la respuesta del respondedor
        let mut resp_sync = [0u8; 4];
        stream.read_exact(&mut resp_sync).await?;
        recv_cipher.process(&mut resp_sync);
        let sync_val = u32::from_le_bytes(resp_sync);
        if sync_val != MAGICVALUE_SYNC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid obfuscation magic sync from peer: 0x{sync_val:08X}"),
            ));
        }

        let mut resp_meta = [0u8; 2]; // selected_method, pad_len
        stream.read_exact(&mut resp_meta).await?;
        recv_cipher.process(&mut resp_meta);

        let selected_method = resp_meta[0];
        if selected_method != ENM_OBFUSCATION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unsupported obfuscation method selected by peer: {selected_method}"),
            ));
        }

        let resp_pad_len = resp_meta[1] as usize;
        if resp_pad_len > 0 {
            let mut pad_buf = vec![0u8; resp_pad_len];
            stream.read_exact(&mut pad_buf).await?;
            recv_cipher.process(&mut pad_buf);
        }

        Ok(Self {
            inner: stream,
            backend: CipherBackend::LegacyRc4 {
                send: Box::new(send_cipher),
                recv: Box::new(recv_cipher),
            },
            read_prefix: Vec::new(),
            read_prefix_offset: 0,
            write_buf: Vec::new(),
            write_offset: 0,
        })
    }

    /// Realiza un saludo de ofuscación cliente a servidor eD2K saliente
    /// (eMule `ECS_PENDING_SERVER` / `ONS_BASIC_SERVER_*`):
    ///
    /// - Cliente → `[SemiRandomMarker (1B), G^A (96B), PadLen (1B), RandomPadding]` (no cifrado)
    /// - Servidor → `[G^B (96B)]` (no cifrado) luego `[MAGICVALUE_SYNC (4B), MethodsSupported (1B), MethodPreferred (1B), PadLen (1B), Padding]` (cifrado)
    /// - Cliente → `[MAGICVALUE_SYNC (4B), MethodSelected (1B), PadLen (1B), Padding]` (cifrado)
    ///
    /// Claves: `SendKey = MD5(S[96] ‖ 34)`, `RecvKey = MD5(S[96] ‖ 203)` donde
    /// `S` es el secreto compartido DH768; RC4-drop1024 en ambos sentidos.
    pub async fn connect_server_outgoing(
        mut stream: S,
        max_padding: u8,
    ) -> Result<Self, io::Error> {
        let (key_pair, marker, pad_len, padding) = {
            let mut rng = rand::rng();
            let kp = Dh768KeyPair::generate(&mut rng);
            let marker = generate_semi_random_marker(&mut rng);
            // eMule añade relleno de 0-15 bytes en el saludo con el servidor.
            let pad_len = if max_padding > 0 {
                rng.random_range(0..=max_padding.min(15))
            } else {
                0
            };
            let mut padding = Vec::with_capacity(pad_len as usize);
            for _ in 0..pad_len {
                padding.push(rng.random());
            }
            (kp, marker, pad_len, padding)
        };

        // No cifrado: marcador + nuestra clave pública DH de 96 bytes + relleno.
        let mut out = Vec::with_capacity(98 + pad_len as usize);
        out.push(marker);
        out.extend_from_slice(&key_pair.public_key);
        out.push(pad_len);
        out.extend_from_slice(&padding);
        stream.write_all(&out).await?;

        // No cifrado: clave pública DH de 96 bytes del servidor.
        let mut remote_pub = [0u8; 96];
        stream.read_exact(&mut remote_pub).await?;
        let shared = key_pair.compute_shared_secret(&remote_pub);
        let (mut send_cipher, mut recv_cipher) = derive_server_ciphers(&shared);

        // Cifrado: [MAGICVALUE_SYNC][supported][preferred][padlen][padding]
        let mut resp_head = [0u8; 7];
        stream.read_exact(&mut resp_head).await?;
        recv_cipher.process(&mut resp_head);
        let magic = u32::from_le_bytes([resp_head[0], resp_head[1], resp_head[2], resp_head[3]]);
        if magic != MAGICVALUE_SYNC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid obfuscation magic sync from server: 0x{magic:08X}"),
            ));
        }
        if resp_head[5] != ENM_OBFUSCATION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Unsupported obfuscation method from server: {}",
                    resp_head[5]
                ),
            ));
        }
        let resp_pad_len = resp_head[6] as usize;
        if resp_pad_len > 0 {
            let mut pad_buf = vec![0u8; resp_pad_len];
            stream.read_exact(&mut pad_buf).await?;
            recv_cipher.process(&mut pad_buf);
        }

        // Respuesta cifrada: [MAGICVALUE_SYNC][método seleccionado][padlen][padding]
        // (eMule la retrasa hasta la primera carga útil; enviarla de inmediato es
        // equivalente en el cable).
        let mut ans = Vec::with_capacity(7 + padding.len());
        ans.extend_from_slice(&MAGICVALUE_SYNC.to_le_bytes());
        ans.push(ENM_OBFUSCATION);
        ans.push(pad_len);
        ans.extend_from_slice(&padding);
        send_cipher.process(&mut ans);
        stream.write_all(&ans).await?;

        Ok(Self {
            inner: stream,
            backend: CipherBackend::LegacyRc4 {
                send: Box::new(send_cipher),
                recv: Box::new(recv_cipher),
            },
            read_prefix: Vec::new(),
            read_prefix_offset: 0,
            write_buf: Vec::new(),
            write_offset: 0,
        })
    }

    /// Inicia un saludo cifrado moderno `PC26-AEAD` (oportunista sin requerir `user_hash`).
    pub async fn connect_peer_pc26_outgoing(
        stream: S,
        suite: Pc26Suite,
    ) -> Result<Self, io::Error> {
        let (inner, state) = connect_pc26_stream_outgoing(stream, suite).await?;
        Ok(Self {
            inner,
            backend: CipherBackend::Pc26Aead(Box::new(state)),
            read_prefix: Vec::new(),
            read_prefix_offset: 0,
            write_buf: Vec::new(),
            write_offset: 0,
        })
    }

    /// Acepta una conexión entrante `PC26-AEAD` inmediatamente tras validar el prefijo `"PC26"`.
    pub async fn accept_peer_pc26_after_magic(
        stream: S,
        allow_pq: bool,
    ) -> Result<Self, io::Error> {
        let (inner, state) = accept_pc26_after_magic(stream, allow_pq).await?;
        Ok(Self {
            inner,
            backend: CipherBackend::Pc26Aead(Box::new(state)),
            read_prefix: Vec::new(),
            read_prefix_offset: 0,
            write_buf: Vec::new(),
            write_offset: 0,
        })
    }

    /// Realiza un saludo de ofuscación entrante cliente a cliente eD2K cuando una conexión entrante
    /// comienza con un byte de protocolo no estándar.
    pub async fn accept_peer_incoming(
        stream: S,
        first_byte: u8,
        local_user_hash: &[u8; 16],
        max_padding: u8,
    ) -> Result<Self, io::Error> {
        Self::accept_peer_incoming_with_fallbacks(
            stream,
            first_byte,
            local_user_hash,
            &[],
            max_padding,
        )
        .await
    }

    /// Variante de [`Self::accept_peer_incoming`] que intenta descifrar el marcador
    /// `MAGICVALUE_SYNC` probando una lista ordenada de hashes candidatos tras `local_user_hash`.
    pub async fn accept_peer_incoming_with_fallbacks(
        stream: S,
        first_byte: u8,
        local_user_hash: &[u8; 16],
        fallback_hashes: &[[u8; 16]],
        max_padding: u8,
    ) -> Result<Self, io::Error> {
        Self::accept_peer_incoming_with_prefetched_and_fallbacks(
            stream,
            first_byte,
            None,
            local_user_hash,
            fallback_hashes,
            max_padding,
        )
        .await
    }

    /// Variante que admite 3 bytes ya leídos de `random_key_part` cuando el primer byte
    /// coincidió fortuitamente con el byte inicial de `PC26_OPPORTUNISTIC_MAGIC`.
    pub async fn accept_peer_incoming_with_prefetched_and_fallbacks(
        mut stream: S,
        _first_byte: u8,
        prefetched_key_prefix: Option<[u8; 3]>,
        local_user_hash: &[u8; 16],
        fallback_hashes: &[[u8; 16]],
        max_padding: u8,
    ) -> Result<Self, io::Error> {
        // 1. Leer los 4 bytes de RandomKeyPart y los 4 bytes de sincronización.
        let mut random_key_part_bytes = [0u8; 4];
        if let Some(prefix) = prefetched_key_prefix {
            random_key_part_bytes[..3].copy_from_slice(&prefix);
            let mut last = [0u8; 1];
            stream.read_exact(&mut last).await?;
            random_key_part_bytes[3] = last[0];
        } else {
            stream.read_exact(&mut random_key_part_bytes).await?;
        }
        let random_key_part = u32::from_le_bytes(random_key_part_bytes);

        let mut magic_bytes = [0u8; 4];
        stream.read_exact(&mut magic_bytes).await?;

        // 2. Derivar cifradores probando primero `local_user_hash` y después los respaldos.
        let mut first_magic = 0u32;
        let mut matched = None;
        for (idx, candidate_hash) in std::iter::once(local_user_hash)
            .chain(fallback_hashes.iter())
            .enumerate()
        {
            let (send_candidate, mut recv_candidate) =
                derive_responder_peer_ciphers(candidate_hash, random_key_part);
            let mut probe = magic_bytes;
            recv_candidate.process(&mut probe);
            let magic = u32::from_le_bytes(probe);
            if idx == 0 {
                first_magic = magic;
            }
            if magic == MAGICVALUE_SYNC {
                matched = Some((send_candidate, recv_candidate));
                break;
            }
        }

        let (mut send_cipher, mut recv_cipher) = matched.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid obfuscation magic sync from incoming peer: 0x{first_magic:08X}"),
            )
        })?;

        let mut meta = [0u8; 3]; // supported, preferred, pad_len
        stream.read_exact(&mut meta).await?;
        recv_cipher.process(&mut meta);

        let req_pad_len = meta[2] as usize;
        if req_pad_len > 0 {
            let mut pad_buf = vec![0u8; req_pad_len];
            stream.read_exact(&mut pad_buf).await?;
            recv_cipher.process(&mut pad_buf);
        }

        // 4. Enviar respuesta cifrada del respondedor
        let (pad_len, padding) = {
            let mut rng = rand::rng();
            let pad_len = if max_padding > 0 {
                rng.random_range(0..=max_padding)
            } else {
                0
            };
            let mut padding = Vec::with_capacity(pad_len as usize);
            for _ in 0..pad_len {
                padding.push(rng.random());
            }
            (pad_len, padding)
        };
        let mut resp = Vec::with_capacity(6 + pad_len as usize);
        resp.extend_from_slice(&MAGICVALUE_SYNC.to_le_bytes());
        resp.push(ENM_OBFUSCATION);
        resp.push(pad_len);
        resp.extend_from_slice(&padding);

        send_cipher.process(&mut resp);
        stream.write_all(&resp).await?;

        Ok(Self {
            inner: stream,
            backend: CipherBackend::LegacyRc4 {
                send: Box::new(send_cipher),
                recv: Box::new(recv_cipher),
            },
            read_prefix: Vec::new(),
            read_prefix_offset: 0,
            write_buf: Vec::new(),
            write_offset: 0,
        })
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for ObfuscatedStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }

        // 1. Drenar primero los bytes de prefijo leídos previamente (ya descifrados)
        if self.read_prefix_offset < self.read_prefix.len() {
            let available = self.read_prefix.len() - self.read_prefix_offset;
            let to_copy = available.min(buf.remaining());
            buf.put_slice(
                &self.read_prefix[self.read_prefix_offset..self.read_prefix_offset + to_copy],
            );
            self.read_prefix_offset += to_copy;
            return Poll::Ready(Ok(()));
        }

        // 2. Leer del flujo interno según el backend activo
        let this = &mut *self;
        match &mut this.backend {
            CipherBackend::Plaintext => Pin::new(&mut this.inner).poll_read(cx, buf),
            CipherBackend::LegacyRc4 { recv, .. } => {
                let before_len = buf.filled().len();
                match Pin::new(&mut this.inner).poll_read(cx, buf) {
                    Poll::Ready(Ok(())) => {
                        let after_len = buf.filled().len();
                        if after_len > before_len {
                            let filled = buf.filled_mut();
                            recv.process(&mut filled[before_len..after_len]);
                        }
                        Poll::Ready(Ok(()))
                    }
                    other => other,
                }
            }
            CipherBackend::Pc26Aead(state) => loop {
                match state.try_open_next_record() {
                    Ok(Some(plain)) => {
                        let to_copy = plain.len().min(buf.remaining());
                        buf.put_slice(&plain[..to_copy]);
                        if to_copy < plain.len() {
                            this.read_prefix = plain[to_copy..].to_vec();
                            this.read_prefix_offset = 0;
                        }
                        return Poll::Ready(Ok(()));
                    }
                    Ok(None) => {}
                    Err(e) => return Poll::Ready(Err(e)),
                }

                let mut tmp = [0u8; 4096];
                let mut tmp_rb = ReadBuf::new(&mut tmp);
                match Pin::new(&mut this.inner).poll_read(cx, &mut tmp_rb) {
                    Poll::Ready(Ok(())) => {
                        let newly_read = tmp_rb.filled().len();
                        if newly_read == 0 {
                            if state.recv_cipher_buf.is_empty() {
                                return Poll::Ready(Ok(()));
                            }
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "registro AEAD PC26 truncado antes de EOF",
                            )));
                        }
                        if state.recv_cipher_buf.len() + newly_read > MAX_PC26_CIPHER_BUF_BYTES {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "búfer de entrada cifrado PC26 excede el límite",
                            )));
                        }
                        state.recv_cipher_buf.extend_from_slice(tmp_rb.filled());
                    }
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                }
            },
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for ObfuscatedStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        // 1. Vaciar cualquier byte cifrado pendiente de una escritura parcial anterior
        while this.write_offset < this.write_buf.len() {
            let pending_slice = &this.write_buf[this.write_offset..];
            match Pin::new(&mut this.inner).poll_write(cx, pending_slice) {
                Poll::Ready(Ok(n)) => {
                    if n == 0 {
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "failed to write buffered data to inner stream",
                        )));
                    }
                    this.write_offset += n;
                }
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            }
        }
        this.write_buf.clear();
        this.write_offset = 0;

        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        let (chunk_size, chunk) = match &mut this.backend {
            CipherBackend::Plaintext => {
                return Pin::new(&mut this.inner).poll_write(cx, buf);
            }
            CipherBackend::LegacyRc4 { send, .. } => {
                let chunk_size = buf.len().min(RC4_WRITE_CHUNK_BYTES);
                let mut chunk = buf[..chunk_size].to_vec();
                send.process(&mut chunk);
                (chunk_size, chunk)
            }
            CipherBackend::Pc26Aead(state) => {
                let chunk_size = buf.len().min(PC26_MAX_RECORD_PLAINTEXT);
                let sealed = state
                    .send_cipher
                    .seal_record(&buf[..chunk_size])
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("PC26: {e}")))?;
                (chunk_size, sealed)
            }
        };

        match Pin::new(&mut this.inner).poll_write(cx, &chunk) {
            Poll::Ready(Ok(n)) => {
                if n < chunk.len() {
                    this.write_buf = chunk;
                    this.write_offset = n;
                }
                Poll::Ready(Ok(chunk_size))
            }
            Poll::Pending => {
                this.write_buf = chunk;
                this.write_offset = 0;
                Poll::Ready(Ok(chunk_size))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        while this.write_offset < this.write_buf.len() {
            let pending_slice = &this.write_buf[this.write_offset..];
            match Pin::new(&mut this.inner).poll_write(cx, pending_slice) {
                Poll::Ready(Ok(n)) => {
                    if n == 0 {
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "failed to flush buffered data",
                        )));
                    }
                    this.write_offset += n;
                }
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            }
        }
        this.write_buf.clear();
        this.write_offset = 0;
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        while this.write_offset < this.write_buf.len() {
            let pending_slice = &this.write_buf[this.write_offset..];
            match Pin::new(&mut this.inner).poll_write(cx, pending_slice) {
                Poll::Ready(Ok(n)) => {
                    if n == 0 {
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "failed to flush buffered data during shutdown",
                        )));
                    }
                    this.write_offset += n;
                }
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            }
        }
        this.write_buf.clear();
        this.write_offset = 0;
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_obfuscated_stream_bidirectional_handshake_and_crypto() {
        let (client_io, server_io) = tokio::io::duplex(4096);
        let server_user_hash = [0x55u8; 16];

        let client_task = tokio::spawn(async move {
            let mut stream =
                ObfuscatedStream::connect_peer_outgoing(client_io, &server_user_hash, 8)
                    .await
                    .expect("client connect_peer_outgoing failed");
            assert!(stream.is_obfuscated());

            // Escribir mensaje de prueba al servidor
            stream.write_all(b"PING_FROM_CLIENT").await.unwrap();
            stream.flush().await.unwrap();

            // Leer respuesta del servidor
            let mut buf = vec![0u8; 16];
            stream.read_exact(&mut buf).await.unwrap();
            assert_eq!(&buf, b"PONG_FROM_SERVER");
        });

        let server_task = tokio::spawn(async move {
            let mut server_io = server_io;
            let mut first_byte = [0u8; 1];
            server_io.read_exact(&mut first_byte).await.unwrap();

            let mut stream = ObfuscatedStream::accept_peer_incoming(
                server_io,
                first_byte[0],
                &server_user_hash,
                8,
            )
            .await
            .expect("server accept_peer_incoming failed");
            assert!(stream.is_obfuscated());

            // Leer mensaje del cliente
            let mut buf = vec![0u8; 16];
            stream.read_exact(&mut buf).await.unwrap();
            assert_eq!(&buf, b"PING_FROM_CLIENT");

            // Escribir respuesta de vuelta al cliente
            stream.write_all(b"PONG_FROM_SERVER").await.unwrap();
            stream.flush().await.unwrap();
        });

        let (c_res, s_res) = tokio::join!(client_task, server_task);
        c_res.unwrap();
        s_res.unwrap();
    }

    #[tokio::test]
    async fn test_plaintext_stream_with_prefix() {
        let (mut client_io, server_io) = tokio::io::duplex(1024);

        tokio::spawn(async move {
            client_io.write_all(b"WORLD").await.unwrap();
        });

        let mut stream = ObfuscatedStream::plaintext_with_prefix(server_io, b"HELLO ".to_vec());
        assert!(!stream.is_obfuscated());

        let mut buf = vec![0u8; 11];
        stream.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"HELLO WORLD");
    }

    /// Prueba de loopback del saludo cliente↔servidor DH768+RC4 de eD2K
    /// (`connect_server_outgoing`) con un servidor simulado realizando el
    /// lado respondedor exactamente como lo hace eMule `ONS_BASIC_SERVER_*`.
    #[tokio::test]
    async fn test_server_obfuscation_handshake_and_crypto() {
        let (client_io, mut server_io) = tokio::io::duplex(8192);

        let client_task = tokio::spawn(async move {
            let mut stream = ObfuscatedStream::connect_server_outgoing(client_io, 8)
                .await
                .expect("client server handshake failed");
            assert!(stream.is_obfuscated());
            stream.write_all(b"PING_TO_SERVER").await.unwrap();
            stream.flush().await.unwrap();
            let mut buf = vec![0u8; 15];
            stream.read_exact(&mut buf).await.unwrap();
            assert_eq!(&buf, b"PONG_FROM_SRV!!");
        });

        let server_task = tokio::spawn(async move {
            // Hello de cliente no cifrado: [marker(1)][G^A(96)][padlen(1)][pad]
            let mut marker = [0u8; 1];
            server_io.read_exact(&mut marker).await.unwrap();
            let mut client_pub = [0u8; 96];
            server_io.read_exact(&mut client_pub).await.unwrap();
            let mut pad_len = [0u8; 1];
            server_io.read_exact(&mut pad_len).await.unwrap();
            let mut pad = vec![0u8; pad_len[0] as usize];
            server_io.read_exact(&mut pad).await.unwrap();

            let server_kp = Dh768KeyPair::generate(&mut rand::rng());
            let shared = server_kp.compute_shared_secret(&client_pub);
            server_io.write_all(&server_kp.public_key).await.unwrap();

            // Las claves del servidor son el reflejo de las del cliente
            // (cliente send = MD5(S‖34), cliente recv = MD5(S‖203)).
            let (client_send, client_recv) = derive_server_ciphers(&shared);
            let mut send_cipher = client_recv;
            let mut recv_cipher = client_send;

            // Respuesta cifrada del servidor: [magic(4)][supported][requested][padlen][pad]
            let mut ans = Vec::new();
            ans.extend_from_slice(&MAGICVALUE_SYNC.to_le_bytes());
            ans.push(ENM_OBFUSCATION);
            ans.push(ENM_OBFUSCATION);
            ans.push(0u8);
            send_cipher.process(&mut ans);
            server_io.write_all(&ans).await.unwrap();

            // Confirmación cifrada del cliente: [magic(4)][method][padlen][pad]
            let mut conf = [0u8; 6];
            server_io.read_exact(&mut conf).await.unwrap();
            recv_cipher.process(&mut conf);
            assert_eq!(
                u32::from_le_bytes(conf[..4].try_into().unwrap()),
                MAGICVALUE_SYNC
            );
            assert_eq!(conf[4], ENM_OBFUSCATION);
            let conf_pad = conf[5] as usize;
            if conf_pad > 0 {
                let mut p = vec![0u8; conf_pad];
                server_io.read_exact(&mut p).await.unwrap();
                recv_cipher.process(&mut p);
            }

            // Intercambio de datos cifrados.
            let mut buf = vec![0u8; 14];
            server_io.read_exact(&mut buf).await.unwrap();
            recv_cipher.process(&mut buf);
            assert_eq!(&buf, b"PING_TO_SERVER");
            let mut pong = b"PONG_FROM_SRV!!".to_vec();
            send_cipher.process(&mut pong);
            server_io.write_all(&pong).await.unwrap();
        });

        let (c_res, s_res) = tokio::join!(client_task, server_task);
        c_res.unwrap();
        s_res.unwrap();
    }

    /// Un par sin ofuscación que responda al sondeo DH con basura en texto plano
    /// debe producir un fallo limpio de saludo (el servidor recurre a texto plano).
    #[tokio::test]
    async fn test_server_obfuscation_rejects_plaintext_answer() {
        let (client_io, mut server_io) = tokio::io::duplex(8192);
        let client_task =
            tokio::spawn(
                async move { ObfuscatedStream::connect_server_outgoing(client_io, 0).await },
            );
        let server_task = tokio::spawn(async move {
            // Leer el hello del cliente, responder con una clave pública DH
            // sintácticamente válida pero seguirla con una respuesta en TEXTO PLANO — la comprobación
            // mágica descifrada del cliente debe fallar limpiamente.
            let mut hello = [0u8; 98];
            server_io.read_exact(&mut hello).await.unwrap();
            let mut client_pub = [0u8; 96];
            client_pub.copy_from_slice(&hello[1..97]);
            let server_kp = Dh768KeyPair::generate(&mut rand::rng());
            server_io.write_all(&server_kp.public_key).await.unwrap();
            // Basura sin cifrar donde se espera el valor mágico cifrado.
            server_io.write_all(&[0xDE; 7]).await.unwrap();
        });
        let (c_res, s_res) = tokio::join!(client_task, server_task);
        s_res.unwrap();
        assert!(c_res.unwrap().is_err());
    }

    #[tokio::test]
    async fn test_put_back_unread_preserves_bytes_without_double_decrypt() {
        // Caso 1: flujo en texto plano con lecturas concatenadas y prefijo parcial previo
        {
            let (mut tx_io, rx_io) = tokio::io::duplex(1024);
            tokio::spawn(async move {
                tx_io.write_all(b"CCC").await.unwrap();
            });
            let mut stream = ObfuscatedStream::plaintext_with_prefix(rx_io, b"AB".to_vec());
            let mut one = [0u8; 1];
            stream.read_exact(&mut one).await.unwrap();
            assert_eq!(&one, b"A");
            // Queda "B" en read_prefix; reinyectamos "XX" delante de "B"
            stream.put_back_unread(b"XX");
            let mut out = [0u8; 6];
            stream.read_exact(&mut out).await.unwrap();
            assert_eq!(&out, b"XXBCCC");
        }

        // Caso 2: flujo cifrado RC4 (verificar que los bytes reinyectados no se descifran dos veces)
        let (client_io, server_io) = tokio::io::duplex(4096);
        let server_user_hash = [0x42u8; 16];

        let client_task = tokio::spawn(async move {
            let mut stream =
                ObfuscatedStream::connect_peer_outgoing(client_io, &server_user_hash, 4)
                    .await
                    .unwrap();
            // Simular Fase 1: leer un búfer grande donde llegan dos mensajes juntos
            let mut buf = [0u8; 32];
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n >= 5);
            // Si llegó parte del segundo mensaje en el mismo segmento, reinyectarlo antes de Fase 2
            let first = buf[..5].to_vec();
            assert_eq!(&first, b"FASE1");
            if n > 5 {
                stream.put_back_unread(&buf[5..n]);
            }
            // Fase 2: leer exactamente el segundo mensaje ("FASE2") + posible tercer bloque ("FASE3")
            let mut rest = [0u8; 10];
            stream.read_exact(&mut rest).await.unwrap();
            assert_eq!(&rest, b"FASE2FASE3");
        });

        let server_task = tokio::spawn(async move {
            let mut server_io = server_io;
            let mut first_byte = [0u8; 1];
            server_io.read_exact(&mut first_byte).await.unwrap();
            let mut stream = ObfuscatedStream::accept_peer_incoming(
                server_io,
                first_byte[0],
                &server_user_hash,
                4,
            )
            .await
            .unwrap();
            stream.write_all(b"FASE1FASE2FASE3").await.unwrap();
            stream.flush().await.unwrap();
        });

        let (c_res, s_res) = tokio::join!(client_task, server_task);
        c_res.unwrap();
        s_res.unwrap();
    }

    #[tokio::test]
    async fn test_pc26_aead_x25519_and_put_back_unread() {
        let (client_io, server_io) = tokio::io::duplex(8192);

        let client_task = tokio::spawn(async move {
            let mut stream = ObfuscatedStream::connect_peer_pc26_outgoing(
                client_io,
                Pc26Suite::X25519ChaCha20Poly1305,
            )
            .await
            .unwrap();
            assert!(stream.is_obfuscated());
            assert!(stream.is_modern_aead());
            let th = stream.transcript_hash().unwrap();

            stream.write_all(b"HOLA_PC26_X25519").await.unwrap();
            stream.flush().await.unwrap();

            let mut buf = [0u8; 32];
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n >= 5);
            assert_eq!(&buf[..5], b"PART1");
            if n > 5 {
                stream.put_back_unread(&buf[5..n]);
            }
            let mut rest = [0u8; 5];
            stream.read_exact(&mut rest).await.unwrap();
            assert_eq!(&rest, b"PART2");
            th
        });

        let server_task = tokio::spawn(async move {
            let mut server_io = server_io;
            let mut magic = [0u8; 4];
            server_io.read_exact(&mut magic).await.unwrap();
            assert_eq!(magic, mule_protocol::ed2k::obfuscation::PC26_OPPORTUNISTIC_MAGIC);

            let mut stream = ObfuscatedStream::accept_peer_pc26_after_magic(server_io, true)
                .await
                .unwrap();
            assert!(stream.is_modern_aead());
            let th = stream.transcript_hash().unwrap();

            let mut msg = [0u8; 16];
            stream.read_exact(&mut msg).await.unwrap();
            assert_eq!(&msg, b"HOLA_PC26_X25519");

            stream.write_all(b"PART1PART2").await.unwrap();
            stream.flush().await.unwrap();
            th
        });

        let (c_res, s_res) = tokio::join!(client_task, server_task);
        assert_eq!(c_res.unwrap(), s_res.unwrap());
    }

    #[tokio::test]
    async fn test_pc26_aead_hybrid_mlkem768_duplex() {
        let (client_io, server_io) = tokio::io::duplex(16384);

        let client_task = tokio::spawn(async move {
            let mut stream = ObfuscatedStream::connect_peer_pc26_outgoing(
                client_io,
                Pc26Suite::HybridMlKem768ChaCha20Poly1305,
            )
            .await
            .unwrap();
            assert_eq!(
                stream.pc26_suite(),
                Some(Pc26Suite::HybridMlKem768ChaCha20Poly1305)
            );
            stream.write_all(b"PQ_SECURE_CHANNEL").await.unwrap();
            stream.flush().await.unwrap();
        });

        let server_task = tokio::spawn(async move {
            let mut server_io = server_io;
            let mut magic = [0u8; 4];
            server_io.read_exact(&mut magic).await.unwrap();
            assert_eq!(magic, mule_protocol::ed2k::obfuscation::PC26_OPPORTUNISTIC_MAGIC);

            let mut stream = ObfuscatedStream::accept_peer_pc26_after_magic(server_io, true)
                .await
                .unwrap();
            let mut msg = [0u8; 17];
            stream.read_exact(&mut msg).await.unwrap();
            assert_eq!(&msg, b"PQ_SECURE_CHANNEL");
        });

        let (c_res, s_res) = tokio::join!(client_task, server_task);
        c_res.unwrap();
        s_res.unwrap();
    }

    #[tokio::test]
    async fn test_rc4_prefetched_prefix_when_first_byte_is_0x50() {
        let (mut client_io, server_io) = tokio::io::duplex(4096);
        let server_user_hash = [0x99u8; 16];

        let client_task = tokio::spawn(async move {
            let random_key_part: u32 = 0x1234_5678;
            let (mut send_cipher, mut recv_cipher) =
                derive_initiator_peer_ciphers(&server_user_hash, random_key_part);
            // Simular un cliente heredado cuyo marcador aleatorio es 0x50 ('P')
            // pero cuya continuación NO es "C26".
            let mut hdr = [0u8; 5];
            hdr[0] = b'P';
            hdr[1..5].copy_from_slice(&random_key_part.to_le_bytes());

            let mut payload = Vec::new();
            payload.extend_from_slice(&MAGICVALUE_SYNC.to_le_bytes());
            payload.push(ENM_OBFUSCATION);
            payload.push(ENM_OBFUSCATION);
            payload.push(0);
            send_cipher.process(&mut payload);

            client_io.write_all(&hdr).await.unwrap();
            client_io.write_all(&payload).await.unwrap();
            client_io.flush().await.unwrap();

            let mut resp = [0u8; 6];
            client_io.read_exact(&mut resp).await.unwrap();
            recv_cipher.process(&mut resp);
            assert_eq!(
                u32::from_le_bytes(resp[..4].try_into().unwrap()),
                MAGICVALUE_SYNC
            );
        });

        let server_task = tokio::spawn(async move {
            let mut server_io = server_io;
            let mut b0 = [0u8; 1];
            server_io.read_exact(&mut b0).await.unwrap();
            assert_eq!(b0[0], b'P');
            let mut rest3 = [0u8; 3];
            server_io.read_exact(&mut rest3).await.unwrap();
            assert_ne!(&rest3, b"C26");

            let stream = ObfuscatedStream::accept_peer_incoming_with_prefetched_and_fallbacks(
                server_io,
                b0[0],
                Some(rest3),
                &server_user_hash,
                &[],
                0,
            )
            .await
            .unwrap();
            assert!(stream.is_obfuscated());
            assert!(!stream.is_modern_aead());
        });

        let (c_res, s_res) = tokio::join!(client_task, server_task);
        c_res.unwrap();
        s_res.unwrap();
    }
}

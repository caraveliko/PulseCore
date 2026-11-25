// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use super::*;

impl InboundListenerService {
    /// Traduce las sanciones `BAD_CLIENT_BAN` de la cola de subida a
    /// baneos de IP (`DownloadService::ban_source`, 2 h — `Ban()`/`AddBannedClient`
    /// de eMule). Devuelve `true` cuando el par actual
    /// acaba de ser baneado — quien llama debe hacer `break` para cortar el socket
    /// (`Disconnected` en `Ban` de eMule).
    pub(super) async fn drain_upload_bans(
        &self,
        peer_ip: Option<std::net::IpAddr>,
        remote_str: &str,
    ) -> bool {
        let bans = self.upload_manager.drain_pending_bans().await;
        if bans.is_empty() {
            return false;
        }
        let mut drop_peer = false;
        {
            let mut dl = self.downloads.lock().await;
            for ip in &bans {
                dl.ban_source(ip);
                tracing::warn!(
                    "[BAN] Client IP {} banned (upload request flood — eMule BADCLIENTBAN)",
                    ip
                );
                if peer_ip.is_some_and(|p| &p.to_string() == ip) {
                    drop_peer = true;
                }
            }
        }
        if drop_peer {
            tracing::warn!(
                "[BAN] Inbound peer {} dropped — request flood ban",
                log_addr_str(remote_str)
            );
        }
        drop_peer
    }

    /// Despacha las subpeticiones de un `OP_MULTIPACKET*` entrante — `ListenSocket.cpp:970-1011`
    /// de eMule. El hash de archivo del envoltorio se aplica a cada
    /// subpetición; las subpeticiones SX van al mismo manejador que los
    /// opcodes independientes, y las respuestas de nombre/file-status/AICH se agrupan
    /// en un único `OP_MULTIPACKETANSWER` como hace eMule.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn handle_inbound_multipacket(
        &self,
        mp: &mule_protocol::ed2k::Ed2kMultiPacketRequest,
        remote_str: &str,
        peer_masks: &mut std::collections::HashMap<[u8; 16], Vec<u8>>,
        sx_env: &sx::SxEnv,
        sx_caps: &sx::PeerSxCaps,
        encoder: &Ed2kEncoder,
        stream: &mut ObfuscatedStream<tokio::net::TcpStream>,
    ) {
        let file_hash = mp.file.file_hash();
        if file_hash == [0u8; 16] {
            // `FileIdentifier` sin MD4 (contexto Kad) — ninguna respuesta
            // de archivo posible.
            return;
        }
        let target_addr = remote_str
            .parse::<SocketAddr>()
            .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        let hash_hex = hex::encode(file_hash);

        let mut want_filename = false;
        let mut want_aich = false;
        for sub in &mp.sub_requests {
            match sub {
                Ed2kMultiPacketSubRequest::FileName { ext_info } => {
                    want_filename = true;
                    // Ext-info: estado de partes del solicitante → `reqstatus` SX.
                    if let Some(ext) = ext_info {
                        peer_masks.insert(file_hash, ext.part_status.to_vec());
                    }
                }
                Ed2kMultiPacketSubRequest::SetReqFileId => {}
                Ed2kMultiPacketSubRequest::RequestSources => {
                    let req = Ed2kRequestSources::new_v1(file_hash);
                    let mask = peer_masks.get(&file_hash).map(Vec::as_slice);
                    sx::respond_to_sx_request(
                        &req,
                        false,
                        None,
                        target_addr,
                        encoder,
                        &self.downloads,
                        stream,
                        sx_env,
                        sx_caps,
                        mask,
                    )
                    .await;
                }
                Ed2kMultiPacketSubRequest::RequestSources2 { version, options } => {
                    let req = Ed2kRequestSources {
                        file_hash,
                        version: *version,
                        options: *options,
                    };
                    let mask = peer_masks.get(&file_hash).map(Vec::as_slice);
                    sx::respond_to_sx_request(
                        &req,
                        true,
                        None,
                        target_addr,
                        encoder,
                        &self.downloads,
                        stream,
                        sx_env,
                        sx_caps,
                        mask,
                    )
                    .await;
                }
                Ed2kMultiPacketSubRequest::AichFileHash => want_aich = true,
            }
        }
        if !want_filename && !want_aich {
            return;
        }

        // Respuesta agrupada de eMule (`OP_MULTIPACKETANSWER`): nombre del archivo +
        // file-status (+ hash AICH) para el archivo del envoltorio.
        let shared_file = {
            let dl_svc = self.downloads.lock().await;
            dl_svc.get_shared_file(&hash_hex).unwrap_or(None)
        };
        let mut sub_answers: Vec<Ed2kMultiPacketSubAnswer> = Vec::new();
        if want_filename {
            if let Some(sf) = shared_file.as_ref().filter(|s| s.enabled) {
                sub_answers.push(Ed2kMultiPacketSubAnswer::FileName(sf.file_name.clone()));
                let part_count = if sf.file_size == 0 {
                    1
                } else {
                    sf.file_size.div_ceil(9728000).max(1) as u16
                };
                let avail: Vec<u16> = if sf.is_partial {
                    sf.completed_parts.clone()
                } else {
                    (0..part_count).collect()
                };
                let mut mask = vec![0u8; mule_protocol::ed2k::expected_mask_bytes(part_count)];
                for p in avail {
                    mask[(p / 8) as usize] |= 1 << (p % 8);
                }
                sub_answers.push(Ed2kMultiPacketSubAnswer::FileStatus {
                    part_count,
                    part_status: Bytes::from(mask),
                });
            }
        }
        if want_aich {
            if let Some(sf) = shared_file
                .as_ref()
                .filter(|s| s.enabled && s.file_size > 0)
            {
                if let Some(root) = self.shared_aich_root(sf, &hash_hex).await {
                    sub_answers.push(Ed2kMultiPacketSubAnswer::AichFileHash(root));
                }
            }
        }
        if sub_answers.is_empty() {
            return;
        }
        let ans = Ed2kMultiPacketAnswer {
            file: mp.file.clone(),
            sub_answers,
        };
        match encoder.encode_message(&Ed2kMessage::MultiPacketAnswer(ans)) {
            Ok(ans_pkt) => {
                let mut out = Vec::with_capacity(6 + ans_pkt.payload.len());
                out.push(ans_pkt.protocol);
                out.extend_from_slice(&(ans_pkt.frame_len() as u32).to_le_bytes());
                out.push(ans_pkt.opcode);
                out.extend_from_slice(&ans_pkt.payload);
                let _ = stream.write_all(&out).await;
            }
            Err(e) => {
                warn!(
                    "Failed to encode multipacket answer for {peer}: {e}",
                    peer = log_addr_str(remote_str)
                );
            }
        }
    }

    /// Devuelve la raíz AICH de confianza de un archivo compartido, calculando (y almacenando
    /// en caché) el árbol de hashes desde el disco cuando esté ausente.
    /// Resuelve el conjunto de hashes MD4 por partes para `hash_hex`: primero la caché en BD
    /// (aprendida durante la descarga o ya calculada), si no calculada a partir del
    /// archivo compartido en disco y almacenada en caché (`hash_reader` verifica el hash raíz
    /// contra `expected_hash` antes de servir nada).
    pub(super) async fn resolve_part_hashes(
        &self,
        hash_hex: &str,
        expected_hash: &[u8; 16],
    ) -> Option<Vec<[u8; 16]>> {
        let cached = {
            let dl_svc = self.downloads.lock().await;
            dl_svc.get_part_hashes_by_hash(hash_hex)
        };
        if !cached.is_empty() {
            return Some(cached);
        }
        let shared_file = {
            let dl_svc = self.downloads.lock().await;
            dl_svc.get_shared_file(hash_hex).unwrap_or(None)
        };
        match shared_file {
            Some(sf) if sf.enabled && !sf.is_partial && !sf.file_path.is_empty() => {
                let path = sf.file_path.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let f = std::fs::File::open(&path)?;
                    let mut reader = mule_core::EvictingFileReader::new(f);
                    mule_format::ed2k::hash_reader(&mut reader)
                })
                .await;
                match res {
                    Ok(Ok(hashed)) => {
                        if hashed.root_hash.0 == *expected_hash {
                            let mut dl_svc = self.downloads.lock().await;
                            let _ = dl_svc
                                .store_part_hashes_by_hash(hash_hex, &hashed.part_hashes);
                            Some(hashed.part_hashes)
                        } else {
                            warn!(
                                "[HASHSET] Shared file {} on disk does not match its ed2k hash {}, refusing to serve hashset",
                                sf.file_name, hash_hex
                            );
                            None
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub(super) async fn shared_aich_root(
        &self,
        sf: &mule_core::model::SharedFile,
        hash_hex: &str,
    ) -> Option<[u8; 20]> {
        if sf.is_partial {
            return sf.aich_root;
        }
        self.shared_aich_tree(sf, hash_hex).await.map(|(_, r)| r)
    }

    /// Carga o calcula el árbol de hashes AICH completo de un archivo compartido.
    /// La raíz calculada se persiste en el registro del archivo compartido.
    pub(super) async fn shared_aich_tree(
        &self,
        sf: &mule_core::model::SharedFile,
        hash_hex: &str,
    ) -> Option<(mule_format::AichNode, [u8; 20])> {
        if sf.file_path.is_empty() || sf.file_size == 0 {
            return None;
        }
        // Compartido parcial: el árbol AICH no puede calcularse a partir de un archivo
        // .part incompleto — la raíz de confianza (del .met) sigue siendo servible vía
        // `shared_aich_root`, pero no se puede construir ningún conjunto de recuperación.
        if sf.is_partial {
            return None;
        }
        let path = sf.file_path.clone();
        let size = sf.file_size;
        let tree = tokio::task::spawn_blocking(move || {
            let mut f = std::fs::File::open(&path).ok()?;
            mule_format::build_aich_tree_from_reader(&mut f, size).ok()
        })
        .await
        .ok()??;
        let root = tree.root_hash()?.0;
        if let Some(cached) = sf.aich_root {
            if cached != root {
                warn!(
                    "[AICH] Shared file {} on disk does not match its cached AICH root",
                    sf.file_name
                );
                return None;
            }
        } else {
            let mut dl_svc = self.downloads.lock().await;
            let _ = dl_svc.set_shared_aich_root(hash_hex, &root);
        }
        Some((tree, root))
    }

    /// Responde al desafío `OP_SECIDENTSTATE` de un par entrante (rol de firmante):
    /// envía nuestra `OP_PUBLICKEY` cuando se solicita (state=2), luego `OP_SIGNATURE`
    /// en formato V1 o V2 según `peer_sec_ident`.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn answer_peer_challenge(
        &self,
        state: u8,
        challenge: u32,
        peer_pubkey_der: &[u8],
        peer_ip: Option<std::net::IpAddr>,
        peer_sec_ident: u8,
        remote_str: &str,
        encoder: &Ed2kEncoder,
        stream: &mut ObfuscatedStream<tokio::net::TcpStream>,
    ) {
        let Some(identity) = &self.identity else {
            return;
        };

        if state == 2 {
            let my_pub = self.sui_pubkey_der.read().unwrap_or_else(|p| p.into_inner()).clone();
            if let Some(my_pub) = my_pub {
                if let Ok(pk_pkt) = encoder.encode_public_key(&Ed2kPublicKey {
                    key_bytes: Bytes::copy_from_slice(&my_pub),
                }) {
                    let mut out = Vec::new();
                    out.push(pk_pkt.protocol);
                    out.extend_from_slice(&(pk_pkt.frame_len() as u32).to_le_bytes());
                    out.push(pk_pkt.opcode);
                    out.extend_from_slice(&pk_pkt.payload);
                    if let Err(e) = stream.write_all(&out).await {
                        warn!(
                            "[SUI] Failed to send OP_PUBLICKEY to {}: {}",
                            log_addr_str(remote_str),
                            e
                        );
                        return;
                    }
                }
            }
        }

        let use_v2 = (peer_sec_ident & 0x01) == 0 && (peer_sec_ident & 0x02) != 0;
        let (v2_kind, payload_res) = if use_v2 {
            let (ip_u32, kind_byte) = match peer_ip {
                Some(std::net::IpAddr::V4(v4)) => (u32::from_be_bytes(v4.octets()), 10u8),
                _ => (0u32, 30u8),
            };
            (
                Some(kind_byte),
                build_sui_signing_payload(
                    SecureIdentVersion::V2,
                    peer_pubkey_der,
                    challenge,
                    Some(ip_u32),
                    Some(kind_byte),
                ),
            )
        } else {
            (
                None,
                build_sui_signing_payload(
                    SecureIdentVersion::V1,
                    peer_pubkey_der,
                    challenge,
                    None,
                    None,
                ),
            )
        };

        let payload = match payload_res {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    "[SUI] Failed to build signing payload for {}: {}",
                    log_addr_str(remote_str),
                    e
                );
                return;
            }
        };
        let sig = {
            let id = identity.lock().await;
            id.sign_sui_payload(&payload)
        };
        match sig {
            Ok(sig_bytes) => {
                if let Ok(sig_pkt) = encoder.encode_signature(&Ed2kSignature {
                    signature_bytes: Bytes::from(sig_bytes),
                }) {
                    let extra = usize::from(v2_kind.is_some());
                    let mut out = Vec::with_capacity(6 + sig_pkt.payload.len() + extra);
                    out.push(sig_pkt.protocol);
                    out.extend_from_slice(&((sig_pkt.frame_len() + extra) as u32).to_le_bytes());
                    out.push(sig_pkt.opcode);
                    out.extend_from_slice(&sig_pkt.payload);
                    if let Some(kind) = v2_kind {
                        out.push(kind);
                    }
                    match stream.write_all(&out).await {
                        Ok(()) => {
                            self.sui_stats
                                .signatures_sent
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            tracing::info!(
                                "[SUI] Answered challenge from inbound peer {} (state={}, {} signature)",
                                log_addr_str(remote_str),
                                state,
                                if v2_kind.is_some() { "V2" } else { "V1" }
                            );
                        }
                        Err(e) => {
                            warn!(
                                "[SUI] Failed to send OP_SIGNATURE to {}: {}",
                                log_addr_str(remote_str),
                                e
                            )
                        }
                    }
                }
            }
            Err(e) => warn!(
                "[SUI] Failed to sign challenge from {}: {}",
                log_addr_str(remote_str),
                e
            ),
        }
    }
}

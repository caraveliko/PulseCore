// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! `OP_CHANGE_CLIENT_ID` de eMule (`ListenSocket.cpp:590-616`) — un par que
//! cambió de servidor eD2K anuncia su nuevo ID híbrido y la IP de su nuevo servidor:
//! `<u32 newUserID><u32 newServerIP>` (se toleran bytes finales adicionales).
//!
//! eMule aplica `SetUserIDHybrid` + `SetServerIP`/`SetServerPort` en el objeto
//! cliente cuando la carga útil tiene sentido:
//! - `newUserID` es LowID y el servidor anunciado existe en la lista local de
//!   servidores → adopta el LowID + las coordenadas del servidor;
//! - `newUserID` es igual a la IP real del par (un HighID) → lo adopta y
//!   actualiza las coordenadas del servidor cuando este es conocido;
//! - cualquier otro caso → se ignora ("unknown contents").
//!
//! Nuestro modelo por conexión no tiene campo de ID híbrido: la parte observable
//! son las coordenadas del servidor de la fuente (`PeerSource.server_ip` /
//! `server_port`), que alimentan `IsLocalServer` / `has_server_callback_option`
//! para las decisiones de callback LowID — exactamente para lo que las usa eMule.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::services::downloads::DownloadService;

use super::log_addr_str;

/// `IsLowID` de eMule — un ID de cliente por debajo de este límite es un
/// LowID emitido por un servidor, no una IP enrutable (`opcodes.h` `HIGHEST_LOWID_ED2K + 1`).
const LOWID_BOUND: u32 = 16_777_216;

/// Aplica una carga útil entrante de `OP_CHANGE_CLIENT_ID` para la conexión
/// en `remote_str` ("ip:port"), en nombre de cada hash de archivo con el que el par esté
/// comprometido. Operación no operativa ante cargas malformadas y en los casos que eMule
/// ignora (servidor desconocido, contenido desconocido).
pub(crate) async fn apply_change_client_id(
    payload: &[u8],
    remote_str: &str,
    file_hashes: &[String],
    downloads: &Arc<Mutex<DownloadService>>,
) {
    if payload.len() < 8 {
        return;
    }
    let new_user_id = u32::from_le_bytes(payload[..4].try_into().unwrap_or_default());
    let new_server_ip = u32::from_le_bytes(payload[4..8].try_into().unwrap_or_default());

    let is_lowid = new_user_id != 0 && new_user_id < LOWID_BOUND;
    // Caso HighID: el ID anunciado coincide con la IP real del par en el
    // cable — eMule compara `nNewUserID` (lectura en little-endian) con
    // `client->GetIP()` almacenado en orden de red.
    let peer_ip_wire: Option<u32> =
        remote_str
            .parse::<SocketAddr>()
            .ok()
            .and_then(|a| match a.ip() {
                IpAddr::V4(v4) => Some(u32::from_be_bytes(v4.octets())),
                IpAddr::V6(_) => None,
            });
    let high_id_case = !is_lowid && peer_ip_wire == Some(new_user_id);
    if !is_lowid && !high_id_case {
        tracing::debug!(
            "Peer {} sent OP_CHANGE_CLIENT_ID with unknown contents (newUserID={new_user_id:#010x})",
            log_addr_str(remote_str),
        );
        return;
    }

    // `serverlist->GetServerByIP(nNewServerIP)` de eMule — la actualización solo
    // se produce cuando el servidor anunciado es uno que ya conocemos.
    let server_port = {
        let dl_svc = downloads.lock().await;
        dl_svc
            .list_servers()
            .unwrap_or_default()
            .into_iter()
            .find_map(|s| {
                let ip_u32 =
                    s.ip.parse::<std::net::Ipv4Addr>()
                        .ok()
                        .map(|v4| u32::from_le_bytes(v4.octets()))?;
                (ip_u32 == new_server_ip).then_some(s.port)
            })
    };
    let Some(server_port) = server_port else {
        return;
    };

    // Actualiza las filas de origen a las que esta conexión corresponde: la fila `ip:port`
    // siempre, además de la fila `lowid:<new_id>` cuando el ID anunciado es un
    // LowID (la declaración vincula esa identidad lowid a su nuevo servidor —
    // `IsLocalServer` puede entonces emparejarla para decisiones de callback).
    let mut dl_svc = downloads.lock().await;
    for file_hash in file_hashes {
        let _ = dl_svc.update_peer_source_server(file_hash, remote_str, new_server_ip, server_port);
        if is_lowid {
            let _ = dl_svc.update_peer_source_server(
                file_hash,
                &format!("lowid:{new_user_id}"),
                new_server_ip,
                server_port,
            );
        }
    }
    tracing::debug!(
        "Peer {} changed eD2K identity: newUserID={new_user_id:#010x} on known server {new_server_ip:#010x}:{server_port} ({} file scope(s) updated)",
        log_addr_str(remote_str),
        file_hashes.len(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use mule_core::model::{
        Ed2kServer, PeerSource, PeerSourceOrigin, PeerSourceStatus, ServerFilterStatus,
    };

    fn test_service(dir: &tempfile::TempDir) -> DownloadService {
        DownloadService::new(&dir.path().join("mule.db"), dir.path().join("tmp"))
            .expect("DownloadService")
    }

    fn server(ip: &str, port: u16) -> Ed2kServer {
        Ed2kServer {
            address: format!("{ip}:{port}"),
            ip: ip.to_string(),
            port,
            name: "test".to_string(),
            description: String::new(),
            ping: 0,
            preference: 0,
            failed_count: 0,
            dyn_ip: None,
            version: None,
            filter_status: ServerFilterStatus::Allowed,
            last_seen_unix_ms: None,
            created_at_unix_ms: 0,
            updated_at_unix_ms: 0,
        }
    }

    fn source(file_hash: &str, addr: &str) -> PeerSource {
        PeerSource {
            file_hash: file_hash.to_string(),
            address: addr.to_string(),
            ip: 0,
            port: 0,
            origin: PeerSourceOrigin::SourceExchange,
            discovered_at_unix_ms: 0,
            last_seen_unix_ms: 0,
            status: PeerSourceStatus::Candidate,
            fail_count: 0,
            last_error: None,
            cooldown_until_unix_ms: None,
            user_hash: None,
            crypt_options: 0,
            buddy_id: None,
            software: None,
            availability: None,
            server_ip: 0,
            server_port: 0,
            udp_port: 0,
        }
    }

    fn payload(new_id: u32, server_ip: u32) -> Vec<u8> {
        let mut p = Vec::with_capacity(8);
        p.extend_from_slice(&new_id.to_le_bytes());
        p.extend_from_slice(&server_ip.to_le_bytes());
        p
    }

    fn server_fields(svc: &DownloadService, file_hash: &str, addr: &str) -> (u32, u16) {
        svc.list_peer_sources(file_hash)
            .unwrap()
            .into_iter()
            .find(|s| s.address == addr)
            .map(|s| (s.server_ip, s.server_port))
            .unwrap_or((0, 0))
    }

    /// `ListenSocket.cpp:598-605` de eMule: LowID + servidor conocido → las
    /// coordenadas del nuevo servidor se aplican en la fila `lowid:<id>`
    /// (y en la fila `ip:port` de la conexión).
    #[tokio::test]
    async fn lowid_on_known_server_updates_source_server_fields() {
        let dir = tempfile::tempdir().unwrap();
        let svc = Arc::new(Mutex::new(test_service(&dir)));
        {
            let mut dl = svc.lock().await;
            dl.set_lowid_source_acceptance(true);
            // Servidor conocido: 1.2.3.4:49661 (server_ip wire = octetos LE).
            dl.upsert_servers(&[server("1.2.3.4", 49661)]).unwrap();
            let hash = "aa".repeat(16);
            dl.upsert_peer_sources(
                &hash,
                &[source(&hash, "lowid:42"), source(&hash, "9.9.9.9:49662")],
            )
            .unwrap();
        }
        let hash = "aa".repeat(16);

        // newUserID=42 (LowID), newServerIP=1.2.3.4 en u32 LE wire.
        let p = payload(42, u32::from_le_bytes([1, 2, 3, 4]));
        apply_change_client_id(&p, "9.9.9.9:49662", std::slice::from_ref(&hash), &svc).await;

        let dl = svc.lock().await;
        let wire_ip = u32::from_le_bytes([1, 2, 3, 4]);
        assert_eq!(server_fields(&dl, &hash, "lowid:42"), (wire_ip, 49661));
        assert_eq!(server_fields(&dl, &hash, "9.9.9.9:49662"), (wire_ip, 49661));
    }

    /// LowID anunciado en un servidor DESCONOCIDO de la lista de servidores → eMule no
    /// modifica nada (`pNewServer == NULL` → sin SetUserIDHybrid).
    #[tokio::test]
    async fn lowid_on_unknown_server_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let svc = Arc::new(Mutex::new(test_service(&dir)));
        {
            let mut dl = svc.lock().await;
            dl.set_lowid_source_acceptance(true);
            let hash = "bb".repeat(16);
            dl.upsert_peer_sources(&hash, &[source(&hash, "lowid:42")])
                .unwrap();
        }
        let hash = "bb".repeat(16);
        let p = payload(42, u32::from_le_bytes([8, 8, 8, 8]));
        apply_change_client_id(&p, "9.9.9.9:49662", std::slice::from_ref(&hash), &svc).await;
        let dl = svc.lock().await;
        assert_eq!(server_fields(&dl, &hash, "lowid:42"), (0, 0));
    }

    /// HighID: `nNewUserID` == IP real del par (cable/BE) + servidor
    /// conocido → las coordenadas se aplican en la fila `ip:port`
    /// únicamente (no existe ninguna fila `lowid:` para un HighID).
    #[tokio::test]
    async fn highid_matching_peer_ip_updates_ip_row_only() {
        let dir = tempfile::tempdir().unwrap();
        let svc = Arc::new(Mutex::new(test_service(&dir)));
        {
            let mut dl = svc.lock().await;
            dl.upsert_servers(&[server("1.2.3.4", 49661)]).unwrap();
            let hash = "cc".repeat(16);
            dl.upsert_peer_sources(&hash, &[source(&hash, "9.9.9.9:49662")])
                .unwrap();
        }
        let hash = "cc".repeat(16);
        // newUserID = IP del par 9.9.9.9 en orden de cable (BE).
        let p = payload(
            u32::from_be_bytes([9, 9, 9, 9]),
            u32::from_le_bytes([1, 2, 3, 4]),
        );
        apply_change_client_id(&p, "9.9.9.9:49662", std::slice::from_ref(&hash), &svc).await;
        let dl = svc.lock().await;
        assert_eq!(
            server_fields(&dl, &hash, "9.9.9.9:49662"),
            (u32::from_le_bytes([1, 2, 3, 4]), 49661)
        );
    }

    /// Contenido desconocido (HighID que no corresponde con la IP del par) y
    /// carga útil truncada → ignorados igual que en eMule.
    #[tokio::test]
    async fn unknown_contents_and_short_payload_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let svc = Arc::new(Mutex::new(test_service(&dir)));
        {
            let mut dl = svc.lock().await;
            dl.upsert_servers(&[server("1.2.3.4", 49661)]).unwrap();
            let hash = "dd".repeat(16);
            dl.upsert_peer_sources(&hash, &[source(&hash, "9.9.9.9:49662")])
                .unwrap();
        }
        let hash = "dd".repeat(16);
        // HighID ≠ IP del par → "unknown contents" de eMule.
        let p = payload(
            u32::from_be_bytes([7, 7, 7, 7]),
            u32::from_le_bytes([1, 2, 3, 4]),
        );
        apply_change_client_id(&p, "9.9.9.9:49662", std::slice::from_ref(&hash), &svc).await;
        // Carga útil truncada → ignorada.
        apply_change_client_id(
            &[1, 2, 3],
            "9.9.9.9:49662",
            std::slice::from_ref(&hash),
            &svc,
        )
        .await;
        let dl = svc.lock().await;
        assert_eq!(server_fields(&dl, &hash, "9.9.9.9:49662"), (0, 0));
    }
}

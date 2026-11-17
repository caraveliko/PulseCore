// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Convergencia iterativa `find_node` de Kad compartida por las rutas de búsqueda y publicación.
//!
//! eMule solo atiende una solicitud de publicación/búsqueda cuando la clave objetivo
//! se encuentra dentro de `SEARCHTOLERANCE` del KadID del nodo que responde — y
//! descarta el paquete en silencio en caso contrario. Por ello, publicar directamente
//! desde la tabla de enrutamiento local falla casi siempre cuando la tabla está
//! dispersa u obsoleta: los contactos locales «más cercanos» suelen seguir fuera de
//! tolerancia, y los contactos caídos no pueden responder en absoluto. Converger primero
//! con `find_node` proporciona contactos vivos y realmente cercanos.

use crate::services::peer_session::log_peer_ip;
use std::collections::{BTreeMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use mule_core::kad::{KBucketTable, KadContact, KadId};
use mule_filter::IpFilter;
use mule_network_policy::{classify_ip, IpCategory, NetworkPolicyManager, NetworkProfile};

use crate::services::kad_udp::KadUdpService;

/// eMule `ALPHA_QUERY` (`kademlia/Defines.h:45`) — parallel `find_node`
/// queries per convergence round.
const ALPHA_QUERY: usize = 3;
/// eMule `CSearch::Go` seed pool — `GetClosestTo(3, target, distance, 50,
/// m_mapPossible)` (`Search.cpp:172`).
const SEARCH_SEED_CONTACTS: usize = 50;

/// Consulta iterativamente `find_node` hacia `target` partiendo de los contactos
/// conocidos más cercanos, hasta que no quede ningún contacto sin consultar más cercano
/// o se cumpla el criterio de convergencia temprana (`SEARCHTOLERANCE` de eMule).
///
/// Devuelve el conjunto convergente de candidatos ordenado por distancia XOR a `target`
/// (los más cercanos primero). Los contactos que responden se marcan como vistos/verificados en la
/// tabla de enrutamiento; los que no responden acumulan fallos y pueden ser desalojados.
pub async fn converge_closest_contacts(
    target: &KadId,
    routing_table: &Mutex<KBucketTable>,
    policy_mgr: &NetworkPolicyManager,
    ip_filter: &Arc<IpFilter>,
    udp: &Arc<KadUdpService>,
    max_rounds: usize,
) -> Vec<KadContact> {
    let initial_closest = {
        let table = routing_table.lock().await;
        // `CSearch::Go` de eMule inicializa con `GetClosestTo(3, target, distance,
        // 50, m_mapPossible)` (`Search.cpp:172`).
        table.find_closest_verified(target, SEARCH_SEED_CONTACTS)
    };

    let mut candidates: BTreeMap<KadId, KadContact> = BTreeMap::new();
    for c in initial_closest {
        let dist = c.kad_id.xor_distance(target);
        candidates.insert(dist, c);
    }

    let mut queried_nodes = HashSet::new();
    const ALPHA: usize = ALPHA_QUERY;

    for iter in 0..max_rounds {
        let unqueried: Vec<_> = candidates
            .values()
            .filter(|c| !queried_nodes.contains(&SocketAddr::new(IpAddr::V4(c.ip), c.udp_port)))
            .take(ALPHA)
            .cloned()
            .collect();

        if unqueried.is_empty() {
            break;
        }

        // Early stop once enough close candidates sit inside SEARCHTOLERANCE.
        let within_tolerance_count = candidates
            .values()
            .take(16)
            .filter(|c| c.kad_id.is_within_search_tolerance(target))
            .count();
        if within_tolerance_count >= 12 && iter >= 2 {
            break;
        }

        let mut find_futs = Vec::new();
        for contact in &unqueried {
            let addr = SocketAddr::new(IpAddr::V4(contact.ip), contact.udp_port);
            let receiver_id = contact.kad_id;
            queried_nodes.insert(addr);
            let udp_c = Arc::clone(udp);
            let target_id = *target;
            find_futs.push(async move {
                match tokio::time::timeout(
                    Duration::from_millis(2500),
                    udp_c.send_find_node(addr, &target_id, &receiver_id),
                )
                .await
                {
                    Ok(res) => res,
                    Err(_) => Err("FindNode round timeout (2.5s)".to_string()),
                }
            });
        }

        let find_results = futures_util::future::join_all(find_futs).await;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        {
            let mut table = routing_table.lock().await;
            for (contact, res) in unqueried.iter().zip(find_results.iter()) {
                if res.is_ok() {
                    table.mark_seen(&contact.kad_id, now_ms);
                } else if table.mark_failed(&contact.kad_id) {
                    tracing::debug!(
                        "Kad: contact {}:{} evicted after repeated unresponsive queries",
                        log_peer_ip(contact.ip),
                        contact.udp_port
                    );
                }
            }
        }

        let mut new_contacts = Vec::new();
        let profile = policy_mgr.profile();
        for contacts in find_results.into_iter().flatten() {
            for c in contacts {
                let cat = classify_ip(IpAddr::V4(c.ip));
                let policy_allowed = match profile {
                    NetworkProfile::LocalLab => cat == IpCategory::Loopback,
                    NetworkProfile::PublicEd2kPreview | NetworkProfile::PublicEd2kKadPreview => {
                        cat == IpCategory::PublicRoutable
                    }
                };
                if policy_allowed && ip_filter.check_ip(IpAddr::V4(c.ip)).is_none() {
                    // Semántica de eMule: un contacto aprendido de una respuesta de
                    // enrutamiento *no* está verificado por IP — la verificación requiere
                    // que responda a una de nuestras solicitudes (mark_seen) o el
                    // saludo HELLO_RES_ACK. Mantenerlo no verificado; todavía puede
                    // ser consultado y promovido al responder.
                    new_contacts.push(c);
                }
            }
        }

        if !new_contacts.is_empty() {
            let mut table = routing_table.lock().await;
            for c in &new_contacts {
                table.insert_or_update(c.clone());
                let dist = c.kad_id.xor_distance(target);
                candidates.insert(dist, c.clone());
            }
        }
    }

    candidates.into_values().collect()
}

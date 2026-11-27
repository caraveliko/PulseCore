// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use futures_util::stream::StreamExt as _;
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use mule_core::kad::{extract_kad_keywords, hash_kad_keyword, KBucketTable, KadContact, KadId};
use mule_filter::IpFilter;
use mule_network_policy::{
    classify_ip, CancellationToken, IpCategory, NetworkPolicyManager, NetworkProfile,
};

use crate::services::kad_udp::KadUdpService;

/// Carga máxima devuelta en `KADEMLIA2_PUBLISH_RES` considerada como publicación aceptada (0-100 %).
pub const KAD_PUBLISH_MAX_ACCEPTED_LOAD: u8 = 100;
/// Entradas máximas de palabras clave empaquetadas por datagrama `PUBLISH_KEY_REQ`.
pub const MAX_KEYWORD_ENTRIES_PER_PACKET: usize = 50;
/// Máximo de archivos agrupados por palabra clave en una ronda de publicación por lotes (eMule).
pub const MAX_FILES_PER_KEYWORD_BATCH: usize = 150;
/// Concurrencia máxima de nodos indexadores al publicar un lote de palabra clave.
pub const KAD_KEYWORD_PUBLISH_CONCURRENCY: usize = 3;
/// Pausa de cadencia entre palabras clave durante la republicación en lote (eMule: 2 s).
pub const KAD_KEYWORD_PUBLISH_PACING_MS: u64 = 2000;

/// Elemento de archivo a indexar por palabras clave en Kad DHT.
#[derive(Debug, Clone)]
pub struct KeywordPublishItem {
    pub file_hash_hex: String,
    pub file_name: String,
    pub file_size: u64,
    pub aich_root: Option<[u8; 20]>,
}

/// Lista de etiquetas almacenada estándar publicada con cada entrada de palabra clave —
/// `PreparePacketForTags` de eMule añade `TAG_SOURCES` (y metadatos opcionales)
/// a la lista de etiquetas de publicación; `TAG_FILENAME`/`TAG_FILESIZE` son emitidos
/// por el codificador a partir de los campos de la entrada (`WriteTagListInc`), no almacenados
/// en `m_listTag`.
fn standard_keyword_tags(
    _file_name: &str,
    _file_size: u64,
    source_count: u32,
) -> Vec<mule_protocol::ed2k::tags::Ed2kTag> {
    use mule_protocol::ed2k::tags::{Ed2kTag, TagName, TagValue};
    vec![Ed2kTag {
        name: TagName::Id(0x15), // TAG_SOURCES
        value: TagValue::UInt32(source_count),
    }]
}

pub const STOP_WORDS: &[&str] = &[
    "the", "el", "la", "los", "las", "un", "una", "unos", "unas",
    "de", "del", "y", "and", "en", "in", "on", "of", "to", "for",
    "with", "con", "por", "para", "le", "les", "des", "du", "et",
    "who", "whom", "whose", "which", "what", "where", "when", "why", "how",
    "que", "qui", "donde", "quien", "quienes", "como", "cuando",
    "dans", "sur", "sous", "avec"
];

// Palabras genéricas o títulos honoríficos multilingües que no deben ser preferidos
// frente a un nombre propio más específico pero ligeramente más corto (ej.: "Monsieur Ibrahim" -> "ibrahim",
// o "The Secret of Roan Inish" -> "inish"/"roan").
pub const LOW_SPECIFICITY_WORDS: &[&str] = &[
    "monsieur", "madame", "mademoiselle", "senor", "senora", "senorita",
    "mister", "doctor", "docteur", "professor", "profesor", "captain", "capitan",
    "chronicles", "cronicas", "adventures", "aventuras", "histoire", "historia",
    "stories", "historias", "legend", "leyenda", "episode", "episodio", "temporada",
    "season", "chapter", "capitulo", "pelicula", "movie", "film", "part", "parte",
    "secret", "secrets", "secreto", "secretos", "dark", "night", "blood", "world",
    "dead", "love", "game", "city", "house", "time", "death", "fire", "water",
    "true", "lost", "black", "white",
    "castellano", "spanish", "espanol", "español", "english", "dual", "vose",
    "1080p", "720p", "2160p", "bluray", "webrip", "dvdrip", "hdtv", "x264", "x265", "hevc"
];

/// Selecciona la palabra clave más discriminante para una búsqueda en Kad DHT.
pub fn select_best_kad_keyword(term: &str) -> (String, KadId) {
    let keywords = extract_kad_keywords(term);
    let candidates: Vec<&(String, KadId)> = keywords
        .iter()
        .filter(|(w, _)| !STOP_WORDS.contains(&w.as_str()) && !w.chars().all(|c| c.is_ascii_digit()))
        .collect();

    let best_kw = candidates
        .iter()
        .copied()
        .filter(|(w, _)| !LOW_SPECIFICITY_WORDS.contains(&w.as_str()) && w.len() >= 4)
        .max_by_key(|(w, _)| w.len())
        .or_else(|| {
            candidates
                .iter()
                .copied()
                .filter(|(w, _)| !LOW_SPECIFICITY_WORDS.contains(&w.as_str()))
                .max_by_key(|(w, _)| w.len())
        })
        .or_else(|| candidates.into_iter().max_by_key(|(w, _)| w.len()))
        .or_else(|| keywords.first());

    if let Some((w, id)) = best_kw {
        (w.clone(), *id)
    } else {
        hash_kad_keyword(term)
    }
}

pub async fn search_keywords_iterative(
    term: &str,
    min_size: Option<u64>,
    max_size: Option<u64>,
    routing_table: &Mutex<KBucketTable>,
    policy_mgr: &NetworkPolicyManager,
    ip_filter: &Arc<IpFilter>,
    udp: &Arc<KadUdpService>,
) -> Result<Vec<serde_json::Value>, String> {
    let (norm_word, keyword_kad_id) = select_best_kad_keyword(term);

    {
        let table = routing_table.lock().await;
        if table.find_closest(&keyword_kad_id, 1).is_empty() {
            tracing::warn!(
                "Kad search_keywords for '{}' (norm='{}'): routing table is empty (0 contacts)",
                term,
                norm_word
            );
            return Ok(Vec::new());
        }
    }

    tracing::info!(
        "Kad search_keywords: starting search for '{}' (hash={}): performing iterative find_node convergence...",
        term,
        keyword_kad_id.to_hex()
    );

    const MAX_CONVERGENCE_ROUNDS: usize = 6;
    let candidates = super::lookup::converge_closest_contacts(
        &keyword_kad_id,
        routing_table,
        policy_mgr,
        ip_filter,
        udp,
        MAX_CONVERGENCE_ROUNDS,
    )
    .await;

    let search_targets: Vec<KadContact> = candidates.into_iter().take(16).collect();
    let within_tol = search_targets
        .iter()
        .filter(|c| c.kad_id.is_within_search_tolerance(&keyword_kad_id))
        .count();

    tracing::info!(
        "Kad search_keywords: querying {} converged contacts for keyword '{}' ({} within SEARCHTOLERANCE)...",
        search_targets.len(),
        term,
        within_tol
    );

    let mut discovered_entries: Vec<serde_json::Value> = Vec::new();
    let mut seen_hashes = std::collections::HashSet::new();
    let profile = policy_mgr.profile();

    let mut futs = Vec::with_capacity(search_targets.len());
    for contact in search_targets {
        let addr = SocketAddr::new(IpAddr::V4(contact.ip), contact.udp_port);
        let cat = classify_ip(IpAddr::V4(contact.ip));
        let policy_allowed = match profile {
            NetworkProfile::LocalLab => cat == IpCategory::Loopback,
            NetworkProfile::PublicEd2kPreview | NetworkProfile::PublicEd2kKadPreview => {
                cat == IpCategory::PublicRoutable
            }
        };

        if !policy_allowed || ip_filter.check_ip(IpAddr::V4(contact.ip)).is_some() {
            continue;
        }

        let udp_c = Arc::clone(udp);
        let target_kw = keyword_kad_id.0;
        futs.push(async move {
            let res = udp_c.send_search_keyword(addr, &target_kw, 0).await;
            (addr, res)
        });
    }

    let results = futures_util::future::join_all(futs).await;
    for (addr, res) in results {
        match res {
            Ok(wires) => {
                tracing::info!(
                    "Kad received {} keyword search entries from node {}",
                    wires.len(),
                    addr
                );
                for w in wires {
                    if !mule_network_policy::content_policy::is_allowed_video_filename(&w.file_name)
                    {
                        continue;
                    }
                    let file_hash_hex = KadId(w.file_hash).to_hash_hex();
                    if let Some(min) = min_size {
                        if w.file_size < min {
                            continue;
                        }
                    }
                    if let Some(max) = max_size {
                        if w.file_size > max {
                            continue;
                        }
                    }

                    if seen_hashes.insert(file_hash_hex.clone()) {
                        // `ProcessResultKeyword` de eMule (Search.cpp:1165):
                        // el histograma de popularidad AICH y el u32 de información
                        // de publicación acompañan a cada resultado de palabra clave
                        // en nodos Kad >= v9.
                        discovered_entries.push(serde_json::json!({
                            "fileHash": file_hash_hex,
                            "fileName": w.file_name,
                            "fileSize": w.file_size,
                            "sourceCount": w.source_count,
                            "aichVotes": w
                                .aich_votes
                                .iter()
                                .map(|v| serde_json::json!({
                                    "popularity": v.popularity,
                                    "aichRoot": hex::encode(v.root),
                                }))
                                .collect::<Vec<_>>(),
                            "publishInfo": w.publish_info,
                        }));
                    }
                }
            }
            Err(e) => {
                tracing::debug!(
                    "Kad search_keyword to {} timed out or node does not index keyword: {}",
                    addr,
                    e
                );
            }
        }
    }

    tracing::info!(
        "Kad search_keywords for '{}' finished: found {} unique file results",
        term,
        discovered_entries.len()
    );

    Ok(discovered_entries)
}

/// Entrada de palabra clave pendiente de envío junto a su raíz AICH opcional.
pub(crate) type KeywordEntryWithAich = (mule_protocol::kad::KadKeywordEntryWire, Option<[u8; 20]>);
/// Lote de entradas agrupadas por identificador Kad de palabra clave.
pub(crate) type GroupedKeywordBatch = (KadId, Vec<KeywordEntryWithAich>);

/// Agrupa una colección de archivos por hash de palabra clave único, limitando cada palabra
/// clave a `MAX_FILES_PER_KEYWORD_BATCH` (150) archivos como hace eMule antes de publicar.
pub(crate) fn group_files_by_keyword(items: &[KeywordPublishItem]) -> Vec<GroupedKeywordBatch> {
    let mut grouped: BTreeMap<KadId, Vec<KeywordEntryWithAich>> = BTreeMap::new();

    for item in items {
        let Some(file_hash_kad_id) = KadId::from_hash_hex(&item.file_hash_hex) else {
            continue;
        };
        let keywords = extract_kad_keywords(&item.file_name);
        let mut seen_kw = std::collections::HashSet::with_capacity(keywords.len());
        for (_word, kw_kad_id) in keywords {
            if !seen_kw.insert(kw_kad_id) {
                continue;
            }
            let list = grouped.entry(kw_kad_id).or_default();
            if list.len() >= MAX_FILES_PER_KEYWORD_BATCH {
                continue;
            }
            let entry = mule_protocol::kad::KadKeywordEntryWire {
                file_hash: file_hash_kad_id.0,
                file_name: item.file_name.clone(),
                file_size: item.file_size,
                source_count: 1,
                // Lista de etiquetas publicada estándar (filename/filesize/sources) —
                // los indexadores remotos evalúan expresiones de búsqueda restrictivas
                // contra estas etiquetas (`WriteTagListWithPublishInfo` de eMule).
                tags: standard_keyword_tags(&item.file_name, item.file_size, 1),
                aich_pub: None,
                aich_votes: Vec::new(),
                publish_info: None,
            };
            list.push((entry, item.aich_root));
        }
    }

    grouped.into_iter().collect()
}

/// Publica entradas de palabras clave agrupadas en lotes de hasta `MAX_KEYWORD_ENTRIES_PER_PACKET`
/// entradas por palabra clave única y paquete, respetando cadencia y cancelación.
pub async fn publish_keywords_batch(
    items: &[KeywordPublishItem],
    routing_table: &Mutex<KBucketTable>,
    policy_mgr: &NetworkPolicyManager,
    ip_filter: &Arc<IpFilter>,
    udp: &Arc<KadUdpService>,
    cancel_token: &CancellationToken,
    pace_between_keywords: bool,
) -> Result<usize, String> {
    let groups = group_files_by_keyword(items);
    if groups.is_empty() {
        return Ok(0);
    }

    let groups_len = groups.len();
    let mut total_published = 0usize;

    for (idx, (kw_kad_id, entries_with_aich)) in groups.into_iter().enumerate() {
        if cancel_token.is_cancelled() {
            break;
        }
        let candidates = super::lookup::converge_closest_contacts(
            &kw_kad_id,
            routing_table,
            policy_mgr,
            ip_filter,
            udp,
            4,
        )
        .await;

        // Solo los nodos dentro de SEARCHTOLERANCE indexarán la palabra clave y responderán;
        // recurrir a los contactos convergidos más cercanos si ninguno cumple el criterio.
        let mut targets: Vec<KadContact> = candidates
            .iter()
            .filter(|c| c.kad_id.is_within_search_tolerance(&kw_kad_id))
            .take(10)
            .cloned()
            .collect();
        if targets.is_empty() {
            targets = candidates.into_iter().take(5).collect();
        }
        if targets.is_empty() {
            continue;
        }

        let kw = kw_kad_id.0;
        let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for packet_chunk in entries_with_aich.chunks(MAX_KEYWORD_ENTRIES_PER_PACKET) {
            if cancel_token.is_cancelled() {
                break;
            }
            let accepted_c = Arc::clone(&accepted);
            let chunk_vec = packet_chunk.to_vec();
            futures_util::stream::iter(targets.clone())
                .for_each_concurrent(KAD_KEYWORD_PUBLISH_CONCURRENCY, move |contact| {
                    let udp_c = Arc::clone(udp);
                    let accepted_inner = Arc::clone(&accepted_c);
                    let entries_c: Vec<_> = chunk_vec
                        .iter()
                        .map(|(e, aich_root)| {
                            let mut clone = e.clone();
                            // `PreparePacketForTags` de eMule (`Search.cpp:1390`): `TAG_KADAICHHASHPUB`
                            // solo se adjunta cuando el nodo destino anuncia Kad >= v9.
                            if contact.version >= mule_protocol::kad::KADEMLIA_VERSION_AICH {
                                clone.aich_pub = *aich_root;
                            }
                            clone
                        })
                        .collect();
                    async move {
                        let addr = SocketAddr::new(IpAddr::V4(contact.ip), contact.udp_port);
                        if let Ok(load) = udp_c.send_publish_keyword(addr, &kw, &entries_c).await {
                            if load <= KAD_PUBLISH_MAX_ACCEPTED_LOAD {
                                accepted_inner.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    }
                })
                .await;
        }
        total_published += accepted.load(std::sync::atomic::Ordering::Relaxed);

        if pace_between_keywords && idx + 1 < groups_len {
            tokio::select! {
                _ = cancel_token.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_millis(KAD_KEYWORD_PUBLISH_PACING_MS)) => {}
            }
        }
    }

    Ok(total_published)
}

/// Publica entradas de palabras clave para un archivo en los contactos Kad más cercanos.
///
/// Los nodos eMule descartan silenciosamente un `PUBLISH_KEY_REQ` cuyo hash de palabra clave
/// quede fuera de `SEARCHTOLERANCE` de su KadID, y solo los nodos activos responden —
/// por tanto los destinos de publicación se convergen primero con una búsqueda iterativa
/// `find_node`, restringiéndose luego a los contactos dentro de tolerancia.
pub async fn publish_keyword(
    file_hash_hex: &str,
    file_name: &str,
    file_size: u64,
    aich_root: Option<[u8; 20]>,
    routing_table: &Mutex<KBucketTable>,
    policy_mgr: &NetworkPolicyManager,
    ip_filter: &Arc<IpFilter>,
    udp: &Arc<KadUdpService>,
) -> Result<usize, String> {
    if KadId::from_hash_hex(file_hash_hex).is_none() {
        return Err(format!("Invalid file hash hex: {file_hash_hex}"));
    }
    let item = KeywordPublishItem {
        file_hash_hex: file_hash_hex.to_string(),
        file_name: file_name.to_string(),
        file_size,
        aich_root,
    };
    let cancel_token = CancellationToken::new();
    publish_keywords_batch(
        &[item],
        routing_table,
        policy_mgr,
        ip_filter,
        udp,
        &cancel_token,
        false,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_files_by_keyword_batches_shared_terms_and_caps_at_150() {
        let mut items = Vec::with_capacity(160);
        for i in 0..160u16 {
            let mut hash = [0u8; 16];
            hash[0..2].copy_from_slice(&i.to_le_bytes());
            items.push(KeywordPublishItem {
                file_hash_hex: hex::encode(hash),
                file_name: format!("Documental_Naturaleza_Ep_{i:03}.mkv"),
                file_size: 1_000_000 + u64::from(i),
                aich_root: Some([0x11u8; 20]),
            });
        }
        let groups = group_files_by_keyword(&items);
        let (_, kw_doc) = hash_kad_keyword("documental");
        let doc_entries = groups
            .iter()
            .find(|(id, _)| *id == kw_doc)
            .map(|(_, v)| v)
            .expect("la palabra clave común debe estar presente");
        assert_eq!(doc_entries.len(), MAX_FILES_PER_KEYWORD_BATCH);
    }

    #[test]
    fn test_select_best_kad_keyword_avoids_stop_words_and_low_specificity() {
        // "The Secret of Roan Inish 1994" : "the", "of" son palabras vacías, "secret" es poco específico.
        // Debe elegir "inish" (5 letras) o "roan" (4 letras), no "secret" ni "1994".
        let (kw, _) = select_best_kad_keyword("The Secret of Roan Inish 1994");
        assert_eq!(kw, "inish");

        // "Ali G: Who Iz I? 2026" : "who" es palabra vacía, 2026 es numérico.
        // No debe seleccionar "who" ni "2026".
        let (kw_ali, _) = select_best_kad_keyword("Ali G: Who Iz I? 2026");
        assert!(kw_ali == "ali" || kw_ali == "iz", "Actual kw was: {}", kw_ali);

        // "Le Voyage dans la Lune 1902" : "le", "dans", "la" son palabras vacías, "1902" numérico.
        // "voyage" tiene 6 letras, "lune" 4 letras.
        let (kw_lune, _) = select_best_kad_keyword("Le Voyage dans la Lune 1902");
        assert_eq!(kw_lune, "voyage");

        // "El secreto de la isla de las focas" : "secreto" es poco específico, "isla" 4 letras, "focas" 5 letras.
        let (kw_focas, _) = select_best_kad_keyword("El secreto de la isla de las focas");
        assert_eq!(kw_focas, "focas");
    }
}

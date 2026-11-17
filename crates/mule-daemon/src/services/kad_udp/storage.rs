// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Almacenamiento en memoria de entradas indexadas de Kad (rol de nodo de almacenamiento DHT).
//!
//! Refleja los datos indexados de `CKademlia` en eMule: los nodos remotos
//! publican entradas de fuentes y palabras clave dirigidas a claves cercanas
//! a nuestro Kad ID; las conservamos con un TTL y las servimos de vuelta ante
//! `SearchSourceReq` / `SearchKeywordReq`.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use mule_protocol::kad::{
    AichHashVote, KadIdBytes, KadKeywordEntryWire, KadNoteWire, KadSourceWire,
    AICH_ROOT_HASH_SIZE,
};

/// Tiempo de vida de una entrada de **fuente** almacenada — eMule `KADEMLIAREPUBLISHTIMES`
/// (`Opcodes.h:78`, 5 h); las fuentes se republican en cada ciclo.
pub const KAD_SOURCE_TTL: Duration = Duration::from_secs(5 * 60 * 60);
/// Tiempo de vida de una entrada de **palabra clave** almacenada — eMule `KADEMLIAREPUBLISHTIMEK`
/// (`Opcodes.h:80`, 24 h).
pub const KAD_KEYWORD_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Tiempo de vida de una entrada de **nota** almacenada — eMule `KADEMLIAREPUBLISHTIMEN`
/// (`Opcodes.h:79`, 24 h).
pub const KAD_NOTE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Número máximo de claves distintas indexadas — eMule `KADEMLIAMAXINDEX` (`Opcodes.h:82`).
pub const MAX_STORE_KEYS: usize = 50_000;
/// Entradas máximas de fuentes/palabras clave almacenadas por clave — eMule
/// `KADEMLIAMAXSOURCEPERFILE` (`Opcodes.h:84`).
pub const MAX_ENTRIES_PER_KEY: usize = 1_000;
/// Resultados máximos servidos por página de búsqueda — eMule `iMaxResults`
/// (`Indexed.cpp:619/707`, fragmentados sobre `UDP_KAD_MAXFRAGMENT`).
pub const MAX_SEARCH_RESULT_PAGE: usize = 300;
/// Límite estricto sobre el total de entradas almacenadas — eMule `KADEMLIAMAXENTRIES`
/// (`Opcodes.h:83`), también límite anti-inundación.
pub const MAX_TOTAL_ENTRIES: usize = 60_000;
/// Entradas de notas máximas almacenadas por archivo de destino — eMule
/// `KADEMLIAMAXNOTESPERFILE` (`Opcodes.h:85`).
pub const MAX_NOTES_PER_TARGET: usize = 150;
/// Publicadores máximos rastreados por entrada de palabra clave — eMule conserva como máximo 100
/// registros `structPublishingIP`, desalojando el más antiguo
/// (`Entry.cpp` `MergeIPsAndFilenames`).
const MAX_TRACKED_PUBLISHERS: usize = 100;
/// Nombres de archivo distintos máximos rastreados por entrada de palabra clave — «límite natural»
/// de `m_aFileNames` en eMule (`Entry.cpp:437`).
const MAX_TRACKED_FILENAMES: usize = 255;
/// `PUBLISHPOINTSSPERSUBNET` (`Entry.cpp:477`) — puntos de confianza aportados por un publicador,
/// divididos entre el recuento global de publicaciones de su subred /24.
const PUBLISH_POINTS_PER_SUBNET: f32 = 10.0;
/// Umbral de confianza para la primera pasada de resultados — eMule sirve entradas con
/// `GetTrustValue() >= 1.0` antes que las de menor confianza (`Indexed.cpp:640`).
const TRUSTED_RESULT_THRESHOLD: f32 = 1.0;
/// Ventana para `bFastRefresh` de eMule: una republicación por el *mismo* publicador
/// antes de `KADEMLIAREPUBLISHTIMES - HR2S(1)` (5 h - 1 h) no incrementa la popularidad del nombre.
const FAST_REFRESH_WINDOW: Duration = Duration::from_secs(4 * 60 * 60);
/// TTL del publicador dentro de una entrada — eMule `KADEMLIAREPUBLISHTIMEK`
/// (`CleanUpTrackedPublishers` descarta rastreadores con más de 24 h).
const PUBLISHER_TTL: Duration = KAD_KEYWORD_TTL;

/// `EqualKadTagStr` de eMule — comparación de nombres de archivo insensible a mayúsculas Unicode
/// empleada para desduplicar nombres en `MergeIPsAndFilenames`.
fn kad_names_equal(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// `uIP & ~0xFF` — la clave de subred /24 sobre la cual `AdjustGlobalPublishTracking` de eMule
/// agrega recuentos de publicaciones (u32 en orden de host de los octetos IPv4).
fn publish_subnet_key(ip: [u8; 4]) -> u32 {
    u32::from_be_bytes(ip) & !0xFF
}

#[derive(Debug, Clone)]
struct StoredSource {
    source: KadSourceWire,
    expires_at: Instant,
}

/// Un publicador rastreado de una entrada de palabra clave — eMule `structPublishingIP`
/// (`Entry.h`), ordenado del más antiguo al más reciente dentro de `publishers`.
#[derive(Debug, Clone)]
struct PublisherTrack {
    /// Octetos IPv4 del publicador (remitente de `PUBLISH_KEY_REQ`).
    ip: [u8; 4],
    /// Momento de la última publicación desde este publicador.
    last_publish: Instant,
    /// Índice en `aich_hashes` de la raíz AICH que este publicador reclamó por última
    /// vez — eMule `m_wAICHHashIdx`, `None` = `_UI16_MAX`.
    aich_idx: Option<usize>,
}

/// Un nombre de archivo distinto y su popularidad — eMule
/// `structFileNameEntry` (`Entry.h:38`).
#[derive(Debug, Clone)]
struct FileNameTrack {
    name: String,
    /// Índice de popularidad relativa — no es un recuento exacto de publicadores
    /// (`GetCommonFileName` compara solo valores).
    popularity: u32,
}

#[derive(Debug, Clone)]
struct StoredKeywordEntry {
    entry: KadKeywordEntryWire,
    expires_at: Instant,
    /// Publicadores rastreados, el más antiguo primero — eMule `m_pliPublishingIPs`.
    publishers: VecDeque<PublisherTrack>,
    /// Raíces AICH indexadas para este archivo — eMule `m_aAICHHashes`.
    aich_hashes: Vec<[u8; AICH_ROOT_HASH_SIZE]>,
    /// Recuento de publicadores por hash — eMule `m_anAICHHashPopularity`.
    aich_popularity: Vec<u8>,
    /// Nombres de archivo distintos con su popularidad — eMule `m_aFileNames`.
    filenames: Vec<FileNameTrack>,
}

impl StoredKeywordEntry {
    fn new(entry: KadKeywordEntryWire, expires_at: Instant) -> Self {
        Self {
            filenames: Vec::new(),
            entry,
            expires_at,
            publishers: VecDeque::new(),
            aich_hashes: Vec::new(),
            aich_popularity: Vec::new(),
        }
    }

    /// eMule `CKeyEntry::AddRemoveAICHHash` — `add` incrementa (o añade)
    /// la popularidad de `root` y devuelve su índice; `!add` decrementa
    /// y devuelve `None`. Los hashes con popularidad cero permanecen en el array
    /// para mantener estables los índices `aich_idx` de los publicadores.
    fn add_remove_aich(&mut self, root: &[u8; AICH_ROOT_HASH_SIZE], add: bool) -> Option<usize> {
        let idx = self.aich_hashes.iter().position(|h| h == root);
        if add {
            match idx {
                Some(i) => {
                    self.aich_popularity[i] = self.aich_popularity[i].saturating_add(1);
                    Some(i)
                }
                None => {
                    self.aich_hashes.push(*root);
                    self.aich_popularity.push(1);
                    Some(self.aich_hashes.len() - 1)
                }
            }
        } else {
            idx.inspect(|&i| {
                self.aich_popularity[i] = self.aich_popularity[i].saturating_sub(1);
            })
        }
    }

    /// Fusión de nombres de archivo `CKeyEntry::MergeIPsAndFilenames` de eMule: el nombre
    /// de la nueva publicación se añade con popularidad 1 salvo que coincida
    /// de forma insensible a mayúsculas con un nombre conocido — en tal caso la popularidad
    /// de ese nombre aumenta (excepto en refrescos rápidos del mismo publicador).
    fn merge_filenames(&mut self, new_name: &str, fast_refresh: bool) {
        if new_name.is_empty() {
            return;
        }
        if let Some(f) = self
            .filenames
            .iter_mut()
            .find(|f| kad_names_equal(&f.name, new_name))
        {
            if !fast_refresh {
                f.popularity = f.popularity.saturating_add(1);
            }
            return;
        }
        self.filenames.push(FileNameTrack {
            name: new_name.to_string(),
            popularity: 1,
        });
        if self.filenames.len() > MAX_TRACKED_FILENAMES {
            self.filenames.truncate(MAX_TRACKED_FILENAMES);
        }
    }

    /// eMule `CKeyEntry::GetCommonFileName` — el nombre en el que coinciden más publicadores
    /// (la primera popularidad máxima gana).
    fn common_file_name(&self) -> &str {
        let mut best: Option<&FileNameTrack> = None;
        for f in &self.filenames {
            if best.map_or(0, |b| b.popularity) < f.popularity {
                best = Some(f);
            }
        }
        best.map(|f| f.name.as_str())
            .unwrap_or(&self.entry.file_name)
    }

    /// eMule `CKeyEntry::CleanUpTrackedPublishers` — descarta rastreadores con más
    /// de `PUBLISHER_TTL` desde la cabeza de la lista ordenada. Al igual que en eMule,
    /// la popularidad AICH NO se decrementa para publicadores caducados (solo para
    /// purgas de >100 publicadores y cambios de republicación).
    fn cleanup_publishers(&mut self, now: Instant, global: &mut HashMap<u32, u32>) {
        while let Some(front) = self.publishers.front() {
            if now < front.last_publish + PUBLISHER_TTL {
                break;
            }
            let old = self.publishers.pop_front().expect("front checked");
            adjust_publish_tracking(global, old.ip, false);
        }
    }

    /// eMule `CKeyEntry::GetTrustValue` — suma de
    /// `PUBLISH_POINTS_PER_SUBNET / subnet_count` sobre publicadores activos.
    fn trust_value(&self, global: &HashMap<u32, u32>) -> f32 {
        self.publishers
            .iter()
            .map(|p| {
                let n = global.get(&publish_subnet_key(p.ip)).copied().unwrap_or(0);
                if n > 0 {
                    PUBLISH_POINTS_PER_SUBNET / n as f32
                } else {
                    0.0
                }
            })
            .sum()
    }

    /// Construye la entrada de red servida en los resultados de búsqueda: nombre común,
    /// histograma AICH (origen de `TAG_KADAICHHASHRESULT`) y `TAG_PUBLISHINFO` u32 —
    /// eMule `WriteTagListWithPublishInfo`.
    fn to_wire(&self, global: &HashMap<u32, u32>) -> KadKeywordEntryWire {
        let mut wire = self.entry.clone();
        wire.file_name = self.common_file_name().to_string();
        wire.aich_pub = None;
        wire.aich_votes = self
            .aich_hashes
            .iter()
            .zip(&self.aich_popularity)
            .filter(|(_, p)| **p > 0)
            .map(|(h, p)| AichHashVote {
                popularity: *p,
                root: *h,
            })
            .collect();
        wire.publish_info = (!self.publishers.is_empty()).then(|| {
            let names = self.filenames.len().min(255) as u32;
            let pubs = self.publishers.len().min(255) as u32;
            let trust = (100.0 * self.trust_value(global)) as u32 & 0xFFFF;
            (names << 24) | (pubs << 16) | trust
        });
        wire
    }
}

/// `CKeyEntry::AdjustGlobalPublishTracking` de eMule — ±1 en el recuento
/// de subred /24 del publicador.
fn adjust_publish_tracking(global: &mut HashMap<u32, u32>, ip: [u8; 4], increase: bool) {
    let key = publish_subnet_key(ip);
    let count = global.entry(key).or_insert(0);
    if increase {
        *count += 1;
    } else {
        *count = count.saturating_sub(1);
    }
    if *count == 0 {
        global.remove(&key);
    }
}

#[derive(Debug, Clone)]
struct StoredNote {
    note: KadNoteWire,
    publisher_ip: [u8; 4],
    publisher_udp_port: u16,
    expires_at: Instant,
}

/// Entrada de nota rehidratada desde la persistencia (etiquetas ya decodificadas).
pub struct HydratedNote {
    pub target_hash: KadIdBytes,
    pub note: KadNoteWire,
    pub publisher_ip: [u8; 4],
    pub publisher_udp_port: u16,
    /// Remaining TTL in milliseconds (0 = already expired, skipped).
    pub remaining_ms: u64,
}

/// Bounded in-memory store for Kad published entries.
#[derive(Default)]
pub struct KadStorage {
    sources: HashMap<KadIdBytes, Vec<StoredSource>>,
    keywords: HashMap<KadIdBytes, Vec<StoredKeywordEntry>>,
    /// hash del archivo objetivo → entradas de notas (`CEntry` con `m_bSource = false`),
    /// desduplicadas por `source_id` (el ID Kad del publicador).
    notes: HashMap<KadIdBytes, Vec<StoredNote>>,
    /// subred /24 → recuento de publicadores de palabras clave registrados desde ella —
    /// `CKeyEntry::s_mapGlobalPublishIPs` de eMule, empleado por el valor de confianza.
    publish_ip_counts: HashMap<u32, u32>,
    total_entries: usize,
}

impl KadStorage {
    /// Almacena un contacto de fuente para `file_hash`. Una entrada por publicador
    /// (indexada por `kad_id`, recurriendo a `ip:puerto` cuando el id es nulo).
    /// Devuelve `true` cuando se aceptó la entrada.
    pub fn store_source(&mut self, file_hash: KadIdBytes, source: KadSourceWire) -> bool {
        let now = Instant::now();
        let expires_at = now + KAD_SOURCE_TTL;

        let dedup_key: [u8; 22] = if source.kad_id != [0u8; 16] {
            let mut k = [0u8; 22];
            k[..16].copy_from_slice(&source.kad_id);
            k
        } else {
            let mut k = [0u8; 22];
            k[..4].copy_from_slice(&source.ip);
            k[4..6].copy_from_slice(&source.tcp_port.to_be_bytes());
            k
        };

        let is_new_key = !self.sources.contains_key(&file_hash);
        if is_new_key && self.sources.len() >= MAX_STORE_KEYS {
            return false;
        }

        let entries = self.sources.entry(file_hash).or_default();
        let before = entries.len();
        entries.retain(|e| e.expires_at > now);
        self.total_entries = self.total_entries.saturating_sub(before - entries.len());

        let key_of = |s: &KadSourceWire| -> [u8; 22] {
            if s.kad_id != [0u8; 16] {
                let mut k = [0u8; 22];
                k[..16].copy_from_slice(&s.kad_id);
                k
            } else {
                let mut k = [0u8; 22];
                k[..4].copy_from_slice(&s.ip);
                k[4..6].copy_from_slice(&s.tcp_port.to_be_bytes());
                k
            }
        };

        if let Some(existing) = entries.iter_mut().find(|e| key_of(&e.source) == dedup_key) {
            existing.source = source;
            existing.expires_at = expires_at;
            return true;
        }
        if entries.len() >= MAX_ENTRIES_PER_KEY || self.total_entries >= MAX_TOTAL_ENTRIES {
            return false;
        }
        entries.push(StoredSource { source, expires_at });
        self.total_entries += 1;
        true
    }

    /// Devuelve fuentes activas para `file_hash`, respetando la paginación `start_pos`.
    pub fn get_sources(&mut self, file_hash: &KadIdBytes, start_pos: u16) -> Vec<KadSourceWire> {
        let now = Instant::now();
        let Some(entries) = self.sources.get_mut(file_hash) else {
            return Vec::new();
        };
        let before = entries.len();
        entries.retain(|e| e.expires_at > now);
        self.total_entries = self.total_entries.saturating_sub(before - entries.len());
        if entries.is_empty() {
            self.sources.remove(file_hash);
            return Vec::new();
        }
        entries
            .iter()
            .skip(start_pos as usize)
            .take(MAX_SEARCH_RESULT_PAGE)
            .map(|e| e.source.clone())
            .collect()
    }

    /// Almacena entradas de palabras clave publicadas bajo `keyword_hash`.
    ///
    /// Desduplica por `(file_hash, file_size)` — `AddKeyword` de eMule mantiene
    /// entradas separadas cuando el mismo id de archivo se publica con un
    /// tamaño diferente. `publisher_ip` alimenta el seguimiento de publicaciones de eMule
    /// (`MergeIPsAndFilenames`): la republicación del mismo publicador refresca
    /// su registro (actualizando/eliminando su contribución AICH) en lugar de
    /// añadir un segundo voto.
    ///
    /// Devuelve `true` cuando se aceptó al menos una entrada.
    pub fn store_keyword_entries(
        &mut self,
        keyword_hash: KadIdBytes,
        new_entries: Vec<KadKeywordEntryWire>,
        publisher_ip: [u8; 4],
    ) -> bool {
        if new_entries.is_empty() {
            return false;
        }
        let now = Instant::now();
        let expires_at = now + KAD_KEYWORD_TTL;

        let is_new_key = !self.keywords.contains_key(&keyword_hash);
        if is_new_key && self.keywords.len() >= MAX_STORE_KEYS {
            return false;
        }

        let entries = self.keywords.entry(keyword_hash).or_default();
        let before = entries.len();
        entries.retain(|e| e.expires_at > now);
        self.total_entries = self.total_entries.saturating_sub(before - entries.len());

        let mut accepted = false;
        for mut entry in new_entries {
            // La declaración AICH del publicador es consumida por la fusión;
            // nunca forma parte de la lista de etiquetas almacenada (semántica `AddTag` de eMule).
            let aich_pub = entry.aich_pub.take();
            entry.aich_votes.clear();
            entry.publish_info = None;
            let pos = entries.iter().position(|e| {
                e.entry.file_hash == entry.file_hash && e.entry.file_size == entry.file_size
            });
            if let Some(i) = pos {
                Self::merge_keyword_entry(
                    &mut entries[i],
                    entry,
                    aich_pub,
                    publisher_ip,
                    now,
                    expires_at,
                    &mut self.publish_ip_counts,
                );
                accepted = true;
                continue;
            }
            if entries.len() >= MAX_ENTRIES_PER_KEY || self.total_entries >= MAX_TOTAL_ENTRIES {
                break;
            }
            let mut stored = StoredKeywordEntry::new(entry, expires_at);
            let new_name = stored.entry.file_name.clone();
            stored.merge_filenames(&new_name, false);
            let aich_idx = aich_pub.and_then(|root| stored.add_remove_aich(&root, true));
            stored.publishers.push_back(PublisherTrack {
                ip: publisher_ip,
                last_publish: now,
                aich_idx,
            });
            adjust_publish_tracking(&mut self.publish_ip_counts, publisher_ip, true);
            entries.push(stored);
            self.total_entries += 1;
            accepted = true;
        }
        accepted
    }

    /// `CKeyEntry::MergeIPsAndFilenames` de eMule — fusiona el seguimiento de
    /// publicador/nombre/AICH de la nueva publicación en `existing`, y luego
    /// reemplaza sus datos de red (la asignación `*pOldEntry = *pEntry`).
    fn merge_keyword_entry(
        existing: &mut StoredKeywordEntry,
        mut new_entry: KadKeywordEntryWire,
        aich_pub: Option<[u8; AICH_ROOT_HASH_SIZE]>,
        publisher_ip: [u8; 4],
        now: Instant,
        expires_at: Instant,
        global: &mut HashMap<u32, u32>,
    ) {
        // Seguimiento de publicadores.
        let mut fast_refresh = false;
        let pub_pos = existing.publishers.iter().position(|p| p.ip == publisher_ip);
        if let Some(pos) = pub_pos {
            // `bFastRefresh`: el mismo publicador republicando antes de
            // KADEMLIAREPUBLISHTIMES - 1h no incrementa la popularidad del nombre.
            fast_refresh = existing.publishers[pos].last_publish > now - FAST_REFRESH_WINDOW;
            existing.publishers[pos].last_publish = now;
            // Actualización de la contribución AICH (eMule: un hash modificado decrementa
            // el antiguo e incrementa el nuevo; un hash eliminado decrementa).
            let prev_idx = existing.publishers[pos].aich_idx;
            match (aich_pub, prev_idx) {
                (Some(root), None) => {
                    existing.publishers[pos].aich_idx = existing.add_remove_aich(&root, true);
                }
                (Some(root), Some(idx)) if existing.aich_hashes[idx] != root => {
                    let old_root = existing.aich_hashes[idx];
                    existing.add_remove_aich(&old_root, false);
                    existing.publishers[pos].aich_idx = existing.add_remove_aich(&root, true);
                }
                (None, Some(idx)) => {
                    let old_root = existing.aich_hashes[idx];
                    existing.add_remove_aich(&old_root, false);
                    existing.publishers[pos].aich_idx = None;
                }
                _ => {}
            }
        } else {
            let aich_idx = aich_pub.and_then(|root| existing.add_remove_aich(&root, true));
            existing.publishers.push_back(PublisherTrack {
                ip: publisher_ip,
                last_publish: now,
                aich_idx,
            });
            adjust_publish_tracking(global, publisher_ip, true);
            // eMule keeps at most 100 tracked publishers per entry.
            if existing.publishers.len() > MAX_TRACKED_PUBLISHERS {
                if let Some(old) = existing.publishers.pop_front() {
                    if let Some(idx) = old.aich_idx {
                        let old_root = existing.aich_hashes[idx];
                        existing.add_remove_aich(&old_root, false);
                    }
                    adjust_publish_tracking(global, old.ip, false);
                }
            }
        }

        // Fusión de nombres de archivo (nombre nuevo añadido o popularidad de nombre coincidente).
        let new_name = new_entry.file_name.clone();
        existing.merge_filenames(&new_name, fast_refresh);

        // `*pOldEntry = *pEntry`: ganan los datos de red más recientes; las
        // etiquetas calculadas por el nodo se vuelven a sintetizar en el momento de servirlas.
        new_entry.aich_pub = None;
        existing.entry = new_entry;
        existing.expires_at = expires_at;
    }

    /// Devuelve entradas de palabras clave activas para `keyword_hash`, respetando `start_pos`.
    pub fn get_keyword_entries(
        &mut self,
        keyword_hash: &KadIdBytes,
        start_pos: u16,
    ) -> Vec<KadKeywordEntryWire> {
        self.get_keyword_entries_filtered(keyword_hash, start_pos, |_| true)
    }

    /// Devuelve entradas de palabras clave activas coincidentes con `filter`, respetando
    /// `start_pos` sobre el conjunto *coincidente* — eMule `SendValidKeywordResult` cuenta
    /// la posición inicial tras el filtrado `StartSearchTermsMatch`, no antes (`Indexed.cpp:630`).
    ///
    /// Los resultados se sirven en dos pasadas como en eMule: entradas de confianza
    /// (`trust >= 1.0`) primero, luego el resto — el desplazamiento `start_pos`
    /// omite entradas coincidentes a lo largo de ambas pasadas.
    pub fn get_keyword_entries_filtered(
        &mut self,
        keyword_hash: &KadIdBytes,
        start_pos: u16,
        filter: impl Fn(&KadKeywordEntryWire) -> bool,
    ) -> Vec<KadKeywordEntryWire> {
        let now = Instant::now();
        let Some(entries) = self.keywords.get_mut(keyword_hash) else {
            return Vec::new();
        };
        let before = entries.len();
        entries.retain(|e| e.expires_at > now);
        self.total_entries = self.total_entries.saturating_sub(before - entries.len());
        for e in entries.iter_mut() {
            e.cleanup_publishers(now, &mut self.publish_ip_counts);
        }
        if entries.is_empty() {
            self.keywords.remove(keyword_hash);
            return Vec::new();
        }
        // eMule `SendValidKeywordResult`: entradas de confianza primero, luego el
        // resto — ambas pasadas comparten el mismo contador `start_pos`. El
        // filtro ve el formato de red servido (nombre común — eMule aplica
        // `StartSearchTermsMatch` sobre `GetCommonFileName`).
        let mut out = Vec::with_capacity(MAX_SEARCH_RESULT_PAGE.min(entries.len()));
        let mut skipped = 0usize;
        'passes: for only_trusted in [true, false] {
            for e in entries.iter() {
                let trusted = e.trust_value(&self.publish_ip_counts) >= TRUSTED_RESULT_THRESHOLD;
                if trusted != only_trusted {
                    continue;
                }
                let wire = e.to_wire(&self.publish_ip_counts);
                if !filter(&wire) {
                    continue;
                }
                if skipped < start_pos as usize {
                    skipped += 1;
                    continue;
                }
                if out.len() >= MAX_SEARCH_RESULT_PAGE {
                    break 'passes;
                }
                out.push(wire);
            }
        }
        out
    }

    /// Almacena una entrada de nota publicada para `target_hash`, actualizada por
    /// `source_id` (una nota por publicador y por archivo, como en eMule `CEntry`).
    /// Devuelve `true` cuando se acepta.
    pub fn store_note(
        &mut self,
        target_hash: KadIdBytes,
        note: KadNoteWire,
        publisher_ip: [u8; 4],
        publisher_udp_port: u16,
    ) -> bool {
        if note.source_id == [0u8; 16] {
            return false;
        }
        let now = Instant::now();
        let expires_at = now + KAD_NOTE_TTL;

        let is_new_key = !self.notes.contains_key(&target_hash);
        if is_new_key && self.notes.len() >= MAX_STORE_KEYS {
            return false;
        }

        let entries = self.notes.entry(target_hash).or_default();
        let before = entries.len();
        entries.retain(|e| e.expires_at > now);
        self.total_entries = self.total_entries.saturating_sub(before - entries.len());

        if let Some(existing) = entries
            .iter_mut()
            .find(|e| e.note.source_id == note.source_id)
        {
            existing.note = note;
            existing.publisher_ip = publisher_ip;
            existing.publisher_udp_port = publisher_udp_port;
            existing.expires_at = expires_at;
            return true;
        }
        if entries.len() >= MAX_NOTES_PER_TARGET || self.total_entries >= MAX_TOTAL_ENTRIES {
            return false;
        }
        entries.push(StoredNote {
            note,
            publisher_ip,
            publisher_udp_port,
            expires_at,
        });
        self.total_entries += 1;
        true
    }

    /// Devuelve notas activas para `target_hash`. Refleja `CIndexed::SendValidNoteResult`:
    /// se filtran las entradas cuyo tamaño de archivo declarado no coincide con el solicitado.
    pub fn get_notes(
        &mut self,
        target_hash: &KadIdBytes,
        file_size: u64,
        max_results: usize,
    ) -> Vec<KadNoteWire> {
        let now = Instant::now();
        let Some(entries) = self.notes.get_mut(target_hash) else {
            return Vec::new();
        };
        let before = entries.len();
        entries.retain(|e| e.expires_at > now);
        self.total_entries = self.total_entries.saturating_sub(before - entries.len());
        if entries.is_empty() {
            self.notes.remove(target_hash);
            return Vec::new();
        }
        entries
            .iter()
            .filter(|e| e.note.file_size().is_none_or(|s| s == file_size))
            .take(max_results)
            .map(|e| e.note.clone())
            .collect()
    }

    /// Carga del indexador para `target_hash` expresada en porcentaje (0-100),
    /// notificada en `KADEMLIA2_PUBLISH_RES` (eMule `uLoad`).
    pub fn note_load(&self, target_hash: &KadIdBytes) -> u8 {
        let count = self.notes.get(target_hash).map_or(0, |e| e.len());
        ((count * 100) / MAX_NOTES_PER_TARGET).min(100) as u8
    }

    /// (claves de fuentes almacenadas, claves de palabras clave almacenadas, claves de notas almacenadas, total de entradas activas) — para diagnósticos.
    pub fn stats(&mut self) -> (usize, usize, usize, usize) {
        (
            self.sources.len(),
            self.keywords.len(),
            self.notes.len(),
            self.total_entries,
        )
    }

    /// Rehidrata entradas de notas persistidas, preservando el TTL restante de cada
    /// entrada (el llamador omite entradas caducadas o malformadas).
    pub fn hydrate_notes(&mut self, notes: Vec<HydratedNote>) {
        for n in notes {
            if n.remaining_ms == 0 || self.total_entries >= MAX_TOTAL_ENTRIES {
                continue;
            }
            let entries = self.notes.entry(n.target_hash).or_default();
            if entries.len() >= MAX_NOTES_PER_TARGET {
                continue;
            }
            if let Some(existing) = entries
                .iter_mut()
                .find(|e| e.note.source_id == n.note.source_id)
            {
                existing.note = n.note;
                existing.expires_at = Instant::now() + Duration::from_millis(n.remaining_ms);
                continue;
            }
            entries.push(StoredNote {
                note: n.note,
                publisher_ip: n.publisher_ip,
                publisher_udp_port: n.publisher_udp_port,
                expires_at: Instant::now() + Duration::from_millis(n.remaining_ms),
            });
            self.total_entries += 1;
        }
    }

    /// Rehidrata el almacenamiento desde filas de índice persistidas, preservando el TTL
    /// restante de cada entrada (se omiten las entradas ya caducadas).
    pub fn hydrate(
        &mut self,
        sources: Vec<mule_core::kad::KadIndexedSource>,
        keywords: Vec<mule_core::kad::KadKeywordEntry>,
        now_unix_ms: u64,
    ) {
        for s in sources {
            let remaining = s.expires_at_unix_ms.saturating_sub(now_unix_ms);
            if remaining == 0 || self.total_entries >= MAX_TOTAL_ENTRIES {
                continue;
            }
            let entries = self.sources.entry(s.file_hash).or_default();
            if entries.len() >= MAX_ENTRIES_PER_KEY {
                continue;
            }
            entries.push(StoredSource {
                source: KadSourceWire {
                    kad_id: s.source_kad_id.0,
                    ip: s.ip.octets(),
                    udp_port: s.udp_port,
                    tcp_port: s.tcp_port,
                    encryption: s.encryption,
                    buddy_id: None,
                },
                expires_at: Instant::now() + Duration::from_millis(remaining),
            });
            self.total_entries += 1;
        }

        let mut grouped: HashMap<KadIdBytes, Vec<mule_core::kad::KadKeywordEntry>> = HashMap::new();
        for e in keywords {
            if self.total_entries >= MAX_TOTAL_ENTRIES {
                break;
            }
            let remaining = (e.expires_at_unix_ms.max(0) as u64).saturating_sub(now_unix_ms);
            if remaining == 0 {
                continue;
            }
            let Ok(hash_bytes) =
                <[u8; 16]>::try_from(hex::decode(&e.keyword_hash).unwrap_or_default().as_slice())
            else {
                continue;
            };
            grouped.entry(hash_bytes).or_default().push(e);
        }
        for (kw_hash, group) in grouped {
            let entries = self.keywords.entry(kw_hash).or_default();
            for e in group {
                if entries.len() >= MAX_ENTRIES_PER_KEY || self.total_entries >= MAX_TOTAL_ENTRIES {
                    break;
                }
                let remaining = (e.expires_at_unix_ms.max(0) as u64).saturating_sub(now_unix_ms);
                let Ok(file_hash) =
                    <[u8; 16]>::try_from(hex::decode(&e.file_hash).unwrap_or_default().as_slice())
                else {
                    continue;
                };
                entries.push(StoredKeywordEntry {
                    entry: KadKeywordEntryWire {
                        file_hash,
                        file_name: e.file_name.clone(),
                        file_size: e.file_size,
                        source_count: e.source_count,
                        // Las etiquetas de metadatos publicadas se mantienen solo en memoria
                        // (eMule tampoco persiste su índice), por lo que las entradas
                        // rehidratadas responden únicamente a términos restrictivos de nombre/tamaño.
                        tags: Vec::new(),
                        aich_pub: None,
                        aich_votes: Vec::new(),
                        publish_info: None,
                    },
                    expires_at: Instant::now() + Duration::from_millis(remaining),
                    // El seguimiento de publicadores/AICH no se persiste (eMule sí lo
                    // persiste mediante `WritePublishTrackingDataToFile` — discrepancia
                    // documentada): las entradas rehidratadas se sirven con confianza 0.
                    publishers: VecDeque::new(),
                    aich_hashes: Vec::new(),
                    aich_popularity: Vec::new(),
                    filenames: vec![FileNameTrack {
                        name: e.file_name,
                        popularity: 1,
                    }],
                });
                self.total_entries += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: u16, ip: [u8; 4]) -> KadSourceWire {
        let mut kad_id = [0u8; 16];
        kad_id[..2].copy_from_slice(&id.to_le_bytes());
        KadSourceWire {
            kad_id,
            ip,
            udp_port: 49672,
            tcp_port: 49662,
            encryption: 0,
            buddy_id: None,
        }
    }

    fn kw_entry(hash_id: u16) -> KadKeywordEntryWire {
        let mut file_hash = [0u8; 16];
        file_hash[..2].copy_from_slice(&hash_id.to_le_bytes());
        KadKeywordEntryWire {
            file_hash,
            file_name: "file.bin".to_string(),
            file_size: 1234,
            source_count: 1,
            tags: Vec::new(),
            aich_pub: None,
            aich_votes: Vec::new(),
            publish_info: None,
        }
    }

    const TEST_PUB_IP: [u8; 4] = [10, 0, 0, 1];

    #[test]
    fn source_store_lookup_and_dedup() {
        let mut st = KadStorage::default();
        let key = [7u8; 16];
        assert!(st.store_source(key, source(1, [1, 2, 3, 4])));
        assert!(st.store_source(key, source(2, [5, 6, 7, 8])));
        assert_eq!(st.get_sources(&key, 0).len(), 2);
        // El mismo kad_id de publicador actualiza en lugar de duplicar.
        let mut updated = source(1, [9, 9, 9, 9]);
        updated.tcp_port = 9999;
        assert!(st.store_source(key, updated));
        let got = st.get_sources(&key, 0);
        assert_eq!(got.len(), 2);
        assert_eq!(
            got.iter().find(|s| s.kad_id[0] == 1).unwrap().tcp_port,
            9999
        );
        // Paginación.
        assert_eq!(st.get_sources(&key, 1).len(), 1);
        assert!(st.get_sources(&key, 2).is_empty());
    }

    #[test]
    fn source_per_key_cap_enforced() {
        let mut st = KadStorage::default();
        let key = [3u8; 16];
        for i in 0..MAX_ENTRIES_PER_KEY {
            assert!(st.store_source(key, source(i as u16, [1, 1, 1, 1])));
        }
        assert!(!st.store_source(key, source(2000, [2, 2, 2, 2])));
        // El servicio está paginado a `MAX_SEARCH_RESULT_PAGE` (eMule iMaxResults).
        assert_eq!(st.get_sources(&key, 0).len(), MAX_SEARCH_RESULT_PAGE);
    }

    #[test]
    fn keyword_store_lookup_and_dedup() {
        let mut st = KadStorage::default();
        let kw = [9u8; 16];
        assert!(st.store_keyword_entries(kw, vec![kw_entry(1), kw_entry(2)], TEST_PUB_IP));
        assert_eq!(st.get_keyword_entries(&kw, 0).len(), 2);
        // La republicación del mismo hash de archivo actualiza en lugar de duplicar.
        let mut upd = kw_entry(1);
        upd.file_name = "renamed.bin".to_string();
        assert!(st.store_keyword_entries(kw, vec![upd], TEST_PUB_IP));
        let got = st.get_keyword_entries(&kw, 0);
        assert_eq!(got.len(), 2);
        // eMule `GetCommonFileName` sirve el nombre más popular — un
        // único renombre no supera en votos al nombre acordado por los publicadores
        // (ambos con popularidad 1, gana el más antiguo).
        assert_eq!(
            got.iter().find(|e| e.file_hash[0] == 1).unwrap().file_name,
            "file.bin"
        );
        // Se rechaza una lista de entradas vacía.
        assert!(!st.store_keyword_entries([4u8; 16], Vec::new(), TEST_PUB_IP));
    }

    #[test]
    fn keyword_filtered_query_paginates_after_filter() {
        // eMule `SendValidKeywordResult`: `start_pos` cuenta las entradas
        // que superan `StartSearchTermsMatch`, no el conjunto sin procesar.
        let mut st = KadStorage::default();
        let kw = [6u8; 16];
        let mk = |id: u16, name: &str| {
            let mut e = kw_entry(id);
            e.file_name = name.to_string();
            e
        };
        assert!(st.store_keyword_entries(
            kw,
            vec![
                mk(1, "skip.bin"),
                mk(2, "keep-one.bin"),
                mk(3, "skip.bin"),
                mk(4, "keep-two.bin"),
                mk(5, "keep-three.bin"),
            ],
            TEST_PUB_IP,
        ));
        let got = st.get_keyword_entries_filtered(&kw, 0, |e| e.file_name.starts_with("keep"));
        assert_eq!(got.len(), 3);
        // start_pos=2 debe omitir dos entradas *coincidentes*.
        let got = st.get_keyword_entries_filtered(&kw, 2, |e| e.file_name.starts_with("keep"));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].file_name, "keep-three.bin");
        // Sin coincidencia -> vacío, sin pánico.
        assert!(st
            .get_keyword_entries_filtered(&kw, 0, |e| e.file_name == "nope")
            .is_empty());
    }

    #[test]
    fn keyword_per_key_cap_enforced() {
        let mut st = KadStorage::default();
        let kw = [5u8; 16];
        let batch: Vec<_> = (0..MAX_ENTRIES_PER_KEY)
            .map(|i| kw_entry(i as u16))
            .collect();
        assert!(st.store_keyword_entries(kw, batch, TEST_PUB_IP));
        assert!(!st.store_keyword_entries(kw, vec![kw_entry(2000)], TEST_PUB_IP));
    }

    #[test]
    fn hydrate_restores_entries_and_skips_expired() {
        use mule_core::kad::{KadId, KadIndexedSource, KadKeywordEntry};
        let now_ms: u64 = 1_000_000;
        let mut st = KadStorage::default();
        let src = KadIndexedSource {
            file_hash: [9u8; 16],
            source_kad_id: KadId([5u8; 16]),
            ip: std::net::Ipv4Addr::new(1, 2, 3, 4),
            udp_port: 49672,
            tcp_port: 49662,
            file_size: 42,
            encryption: 0,
            expires_at_unix_ms: now_ms + 60_000,
            updated_at_unix_ms: now_ms,
        };
        let expired = KadIndexedSource {
            expires_at_unix_ms: now_ms - 1,
            ..src.clone()
        };
        let kw = KadKeywordEntry {
            keyword_hash: hex::encode([7u8; 16]),
            file_hash: hex::encode([9u8; 16]),
            file_name: "file.bin".into(),
            file_size: 42,
            source_count: 3,
            discovered_at_unix_ms: now_ms as i64,
            expires_at_unix_ms: (now_ms + 60_000) as i64,
        };
        st.hydrate(vec![src, expired], vec![kw], now_ms);
        assert_eq!(st.get_sources(&[9u8; 16], 0).len(), 1);
        assert_eq!(st.get_keyword_entries(&[7u8; 16], 0).len(), 1);
        let (_, _, _, total) = st.stats();
        assert_eq!(total, 2);
    }

    #[test]
    fn stats_track_entries() {
        let mut st = KadStorage::default();
        st.store_source([1u8; 16], source(1, [1, 1, 1, 1]));
        st.store_keyword_entries([2u8; 16], vec![kw_entry(1), kw_entry(2)], TEST_PUB_IP);
        let (s, k, n, total) = st.stats();
        assert_eq!((s, k, n, total), (1, 1, 0, 3));
    }

    // ---- AICH keyword tracking (eMule `CKeyEntry`) ----

    fn kw_with_aich(hash_id: u16, root: [u8; 20]) -> KadKeywordEntryWire {
        let mut e = kw_entry(hash_id);
        e.aich_pub = Some(root);
        e
    }

    fn votes_of(e: &KadKeywordEntryWire) -> Vec<(u8, [u8; 20])> {
        e.aich_votes
            .iter()
            .map(|v| (v.popularity, v.root))
            .collect()
    }

    #[test]
    fn aich_two_publishers_same_root_popularity_two() {
        let mut st = KadStorage::default();
        let kw = [9u8; 16];
        let root = [0x42u8; 20];
        assert!(st.store_keyword_entries(
            kw,
            vec![kw_with_aich(1, root)],
            [10, 0, 0, 1]
        ));
        // Segundo publicador, IP distinta, mismo archivo+tamaño, misma raíz.
        assert!(st.store_keyword_entries(
            kw,
            vec![kw_with_aich(1, root)],
            [10, 0, 0, 2]
        ));
        let got = st.get_keyword_entries(&kw, 0);
        assert_eq!(got.len(), 1);
        assert_eq!(votes_of(&got[0]), vec![(2, root)]);
        // PUBLISHINFO: 1 nombre, 2 publicadores.
        let info = got[0].publish_info.expect("publish info");
        assert_eq!((info >> 24) & 0xFF, 1);
        assert_eq!((info >> 16) & 0xFF, 2);
    }

    #[test]
    fn aich_distinct_roots_popularity_split() {
        let mut st = KadStorage::default();
        let kw = [9u8; 16];
        assert!(st.store_keyword_entries(
            kw,
            vec![kw_with_aich(1, [0x11; 20])],
            [10, 0, 0, 1]
        ));
        assert!(st.store_keyword_entries(
            kw,
            vec![kw_with_aich(1, [0x22; 20])],
            [10, 0, 0, 2]
        ));
        let got = st.get_keyword_entries(&kw, 0);
        let mut votes = votes_of(&got[0]);
        votes.sort();
        assert_eq!(votes, vec![(1, [0x11; 20]), (1, [0x22; 20])]);
    }

    #[test]
    fn aich_refresh_changes_root_old_vote_released() {
        // eMule `MergeIPsAndFilenames`: el mismo publicador republicando
        // con una raíz AICH diferente decrementa su voto anterior.
        let mut st = KadStorage::default();
        let kw = [9u8; 16];
        let ip = [10, 0, 0, 1];
        assert!(st.store_keyword_entries(kw, vec![kw_with_aich(1, [0x11; 20])], ip));
        assert!(st.store_keyword_entries(kw, vec![kw_with_aich(1, [0x22; 20])], ip));
        let got = st.get_keyword_entries(&kw, 0);
        assert_eq!(votes_of(&got[0]), vec![(1, [0x22; 20])]);
        assert_eq!((got[0].publish_info.unwrap() >> 16) & 0xFF, 1);
    }

    #[test]
    fn aich_refresh_drops_root_vote_removed() {
        // Un publicador que republica sin AICH elimina su contribución.
        let mut st = KadStorage::default();
        let kw = [9u8; 16];
        let ip = [10, 0, 0, 1];
        assert!(st.store_keyword_entries(kw, vec![kw_with_aich(1, [0x11; 20])], ip));
        assert!(st.store_keyword_entries(kw, vec![kw_entry(1)], ip));
        let got = st.get_keyword_entries(&kw, 0);
        assert!(got[0].aich_votes.is_empty());
    }

    #[test]
    fn aich_republish_same_root_no_double_vote() {
        let mut st = KadStorage::default();
        let kw = [9u8; 16];
        let ip = [10, 0, 0, 1];
        assert!(st.store_keyword_entries(kw, vec![kw_with_aich(1, [0x11; 20])], ip));
        assert!(st.store_keyword_entries(kw, vec![kw_with_aich(1, [0x11; 20])], ip));
        let got = st.get_keyword_entries(&kw, 0);
        assert_eq!(votes_of(&got[0]), vec![(1, [0x11; 20])]);
    }

    #[test]
    fn common_filename_prefers_popular_name() {
        // eMule `GetCommonFileName`: tres publicadores que coinciden en "good"
        // superan en votos a un nombre "bad".
        let mut st = KadStorage::default();
        let kw = [9u8; 16];
        let mk = |name: &str| {
            let mut e = kw_entry(1);
            e.file_name = name.to_string();
            e
        };
        assert!(st.store_keyword_entries(kw, vec![mk("bad.bin")], [10, 0, 0, 1]));
        assert!(st.store_keyword_entries(kw, vec![mk("good.bin")], [10, 0, 0, 2]));
        assert!(st.store_keyword_entries(kw, vec![mk("good.bin")], [10, 0, 0, 3]));
        let got = st.get_keyword_entries(&kw, 0);
        assert_eq!(got[0].file_name, "good.bin");
        assert_eq!((got[0].publish_info.unwrap() >> 24) & 0xFF, 2);
    }

    #[test]
    fn trusted_entries_served_first() {
        // Servicio en dos pasadas de `SendValidKeywordResult` en eMule: las entradas cuyo
        // valor de confianza es >= 1.0 preceden a las de menor confianza con independencia
        // del orden de almacenamiento.
        let mut st = KadStorage::default();
        let kw = [9u8; 16];
        // 11 publicadores de la misma subred /24 en archivos distintos: la confianza
        // de cada entrada = 10/11 < 1.
        for i in 0..11u8 {
            assert!(st.store_keyword_entries(
                kw,
                vec![kw_entry(i as u16 + 1)],
                [10, 0, 0, i + 1]
            ));
        }
        // Un publicador de una subred /24 nueva: confianza = 10.
        assert!(st.store_keyword_entries(kw, vec![kw_entry(100)], [11, 0, 0, 1]));
        let got = st.get_keyword_entries(&kw, 0);
        assert_eq!(got.len(), 12);
        assert_eq!(
            got[0].file_hash,
            kw_entry(100).file_hash,
            "fresh-subnet entry first"
        );
    }

    #[test]
    fn publisher_cleanup_keeps_aich_popularity() {
        // eMule `CleanUpTrackedPublishers`: los publicadores caducados dejan el
        // histograma AICH intacto (solo las purgas de >100 publicadores y los
        // cambios de republicación lo decrementan).
        let mut st = KadStorage::default();
        let kw = [9u8; 16];
        let root = [0x33u8; 20];
        assert!(st.store_keyword_entries(kw, vec![kw_with_aich(1, root)], [10, 0, 0, 1]));
        // Forzar la marca temporal del rastreador del publicador al pasado.
        let entries = st.keywords.get_mut(&kw).unwrap();
        entries[0].publishers[0].last_publish =
            Instant::now() - PUBLISHER_TTL - Duration::from_secs(1);
        let got = st.get_keyword_entries(&kw, 0);
        assert_eq!(votes_of(&got[0]), vec![(1, root)], "AICH vote survives expiry");
        // …pero el propio publicador ha desaparecido: sin etiqueta PUBLISHINFO.
        assert!(got[0].publish_info.is_none());
    }

    #[test]
    fn same_filehash_different_size_coexists() {
        // eMule `AddKeyword`: el mismo ID de fuente publicado con un tamaño
        // diferente produce una segunda entrada, no una fusión.
        let mut st = KadStorage::default();
        let kw = [9u8; 16];
        let mut big = kw_entry(1);
        big.file_size = 9999;
        assert!(st.store_keyword_entries(kw, vec![kw_entry(1)], TEST_PUB_IP));
        assert!(st.store_keyword_entries(kw, vec![big], TEST_PUB_IP));
        let got = st.get_keyword_entries(&kw, 0);
        assert_eq!(got.len(), 2);
    }

    fn note(src_byte: u8, size: u64) -> KadNoteWire {
        use mule_protocol::ed2k::tags::{Ed2kTag, TagName, TagValue};
        let mut source_id = [0u8; 16];
        source_id[0] = src_byte;
        KadNoteWire {
            source_id,
            tags: vec![
                Ed2kTag {
                    name: TagName::Id(0x02),
                    value: TagValue::UInt64(size),
                },
                Ed2kTag {
                    name: TagName::Id(0x0B),
                    value: TagValue::String("note text".to_string()),
                },
            ],
            unknown_tags: vec![],
        }
    }

    #[test]
    fn note_store_lookup_upsert_and_size_filter() {
        let mut st = KadStorage::default();
        let target = [8u8; 16];
        assert!(st.store_note(target, note(1, 1000), [1, 2, 3, 4], 49672));
        assert!(st.store_note(target, note(2, 1000), [5, 6, 7, 8], 49672));
        // El mismo publicador republica → actualización (upsert), no duplicado.
        assert!(st.store_note(target, note(1, 1000), [9, 9, 9, 9], 1234));
        assert_eq!(st.get_notes(&target, 1000, 50).len(), 2);
        // Un tamaño de archivo erróneo filtra las entradas descartándolas (SendValidNoteResult).
        assert!(st.get_notes(&target, 999, 50).is_empty());
        // Se rechaza un source_id nulo.
        assert!(!st.store_note(target, note(0, 1000), [1, 1, 1, 1], 1));
        // La carga refleja la ocupación como porcentaje de MAX_NOTES_PER_TARGET.
        assert_eq!(st.note_load(&target), (2 * 100 / MAX_NOTES_PER_TARGET) as u8);
        assert_eq!(st.note_load(&[0xEEu8; 16]), 0);
    }

    #[test]
    fn note_per_target_cap_enforced() {
        let mut st = KadStorage::default();
        let target = [6u8; 16];
        for i in 0..MAX_NOTES_PER_TARGET {
            assert!(st.store_note(target, note(i as u8 + 1, 5), [1, 1, 1, 1], 1));
        }
        assert!(!st.store_note(target, note(250, 5), [1, 1, 1, 1], 1));
        assert_eq!(st.note_load(&target), 100);
    }
}

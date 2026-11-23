// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_core::{Download, DownloadStatus, PartFile, RangeSet};
use mule_protocol::ed2k::{PartAvailabilityMask, ED2K_PART_SIZE};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use uuid::Uuid;

use serde::{Deserialize, Serialize};

pub const MAX_IMPORT_TOTAL_SIZE: u64 = 1024 * 1024 * 1024 * 1024; // 1 TB safety ceiling
pub const MAX_IMPORT_PART_COUNT: u32 = 65_536; // 65,536 parts ceiling

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadSnapshot {
    pub id: Uuid,
    pub file_name: String,
    pub total_bytes: u64,
    pub received_bytes: u64,
    pub verified_bytes: u64,
    pub progress: f64,
    pub status: String,
    pub priority: String,
    pub source_count: u32,
    pub download_rate_bps: u64,
    pub uploaded_bytes: u64,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
    pub category_id: Option<String>,
    /// Segundos restantes a la tasa actual (None cuando está estancada o finalizada).
    #[serde(default)]
    pub eta_seconds: Option<u64>,
    /// Hash de archivo eD2K (hex) — necesario para que los clientes consulten fuentes de pares
    /// sin un viaje de ida y vuelta adicional a `downloads.get`.
    #[serde(default)]
    pub ed2k_hash: String,
    /// Resumen de progreso por parte para la barra de disponibilidad de la interfaz. En las
    /// instantáneas de lista, `is_verified` solo es exacto para descargas completadas (todas
    /// las partes verificadas); `downloads.get` informa de los indicadores fidedignos.
    #[serde(default)]
    pub parts: Vec<PartProgressSummary>,
    /// Rangos de bytes de bloques actualmente solicitados pero aún no recibidos
    /// (`requestedblocks_list` de eMule — los segmentos amarillos «pendientes»).
    /// Enriquecido por la capa RPC/eventos desde el estado del enjambre.
    #[serde(default)]
    pub pending_ranges: Vec<(u64, u64)>,
    /// Límite de fuentes por archivo importado desde FT_MAXSOURCES de `.part.met`
    /// (`CPartFile::GetMaxSources` de eMule); `None` = límite global.
    #[serde(default)]
    pub max_sources: Option<u32>,
    /// Indica si este archivo está activamente en curso de copia hacia el NAS
    /// (`Some(true)`) o en espera en la cola FIFO del NAS (`Some(false)`),
    /// activo únicamente cuando el estado es `Completing`.
    #[serde(default)]
    pub nas_copy_active: Option<bool>,
    /// Subetapa detallada durante la verificación y finalización post-descarga:
    /// `"verifying_hashes"`, `"reconstructing"`, `"queued_copy"`, `"copying"`, `"organizing"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_stage: Option<String>,
    /// Progreso (0.0 a 1.0) de la copia del archivo reconstruido hacia el destino final.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_progress: Option<f64>,
    /// Protocolo: "ed2k" o "torrent"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PartProgressSummary {
    pub part_index: u32,
    pub start_offset: u64,
    pub end_offset: u64,
    pub size: u64,
    pub received_bytes: u64,
    pub is_verified: bool,
    pub state: String, // "verified", "completed", "downloading", "missing"
    /// Número de fuentes conocidas cuya máscara de disponibilidad cubre esta parte
    /// (coloración azul/roja de eMule: 0 = ninguna fuente la tiene → parte atascada).
    pub available_source_count: u32,
    /// Mapa de bits de la parte dividida en [`PART_MASK_CELLS`] celdas iguales; el bit i se
    /// activa cuando la celda está totalmente cubierta por datos recibidos. Permite a la interfaz dibujar
    /// huecos dentro de la parte al estilo de eMule sin enviar la lista de rangos en bruto.
    #[serde(default)]
    pub received_mask: u64,
}

/// Número de celdas en [`PartProgressSummary::received_mask`] (un bit cada una).
/// ≈152 KiB por celda en una parte completa de 9,28 MiB — cercana a la granularidad de bloque
/// eD2K de 180 KiB que la barra de eMule muestra en realidad.
pub const PART_MASK_CELLS: u64 = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadDetail {
    #[serde(flatten)]
    pub snapshot: DownloadSnapshot,
    pub ed2k_hash: String,
    pub aich_root: Option<String>,
    pub ed2k_link: String,
    pub temp_path: String,
    pub target_path: String,
    pub remaining_bytes: u64,
    pub average_rate_bps: u64,
    pub eta_seconds: Option<u64>,
    pub connection_attempts: u32,
    pub last_error: Option<String>,
    pub last_status_change_unix_ms: i64,
    pub parts: Vec<PartProgressSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PartMetInspection {
    pub file_name: String,
    pub total_size: u64,
    pub ed2k_hash: String,
    pub aich_hash: Option<String>,
    pub part_count: u32,
    pub part_met_path: String,
    pub part_path: Option<String>,
    pub part_file_exists: bool,
    pub part_file_size: Option<u64>,
    pub estimated_transferred_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportPartMetResult {
    pub download: DownloadSnapshot,
    pub total_parts: u32,
    pub verified_parts: u32,
    pub corrupted_parts: u32,
    pub transferred_bytes: u64,
    pub total_size: u64,
    #[serde(default)]
    pub report: Option<String>,
}

/// Resumen neutral de disponibilidad expuesto a la interfaz y clientes externos.
/// Nunca filtra la SocketAddr en bruto ni la topología de red.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceAvailabilitySummary {
    /// Identificador opaco de fuente.
    pub source_id: String,
    /// Número de partes disponibles.
    pub available_part_count: u16,
    /// Número total de partes en el archivo.
    pub total_part_count: u16,
    /// Marca temporal de la última observación.
    pub last_seen_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub struct ActiveSource {
    pub source_id: String,
    pub _addr: SocketAddr,
    pub mask: PartAvailabilityMask,
    pub last_seen_unix_ms: i64,
}

pub struct ActiveDownload {
    pub download: Download,
    pub ranges: RangeSet,
    pub part_file: Option<PartFile>,
    pub sources: HashMap<SocketAddr, ActiveSource>,
    pub connection_attempts: u32,
    pub resume_count: u32,
    pub last_error: Option<String>,
    pub last_status_change_unix_ms: i64,
    pub session_start_ms: i64,
    pub session_bytes_start: u64,
    pub current_rate_bps: u64,
    pub last_rate_update: Option<Instant>,
    pub source_count: u32,
    pub verification_in_progress: bool,
    /// Partes que fallaron en la verificación MD4 y esperan datos de recuperación AICH
    /// (solo se llena cuando se conoce una raíz AICH de confianza):
    /// `(part_index, enqueued_at)`.
    pub aich_pending: VecDeque<(u32, Instant)>,
    /// Partes para las cuales ya se envió una solicitud de recuperación AICH:
    /// `part -> (sent_at, attempts)`.
    pub aich_requested: HashMap<u32, (Instant, u32)>,
    /// Votos de hash maestro AICH no confiables: hash raíz -> IPs de fuentes enmascaradas
    /// (`CAICHUntrustedHash` de eMule; IP enmascarada con `0x00F0FFFF`).
    pub aich_votes: HashMap<[u8; 20], HashSet<u32>>,
    /// Procedencia de los bloques recibidos por parte para atribuir la corrupción al
    /// remitente (`m_CorruptionBlackBox` de eMule).
    pub corruption_blackbox: super::corruption_blackbox::CorruptionBlackBox,
    /// Remitentes condenados por `CorruptionBlackBox::evaluate` (>32 % de
    /// datos corruptos) — vaciados por el llamador que desconecta al par y
    /// marca sus fuentes como `Rejected` (`Ban`/`AddBannedClient` de eMule).
    pub pending_corruption_bans: Vec<String>,
    /// Datos recibidos aún no sincronizados con fsync: el `sync_data` del `.part` se
    /// procesa en lotes según [`PART_PERSIST_INTERVAL`] en lugar de pagarse por bloque
    /// de 180 KiB. La contabilidad SQLite de `received_ranges` se mantiene sincronizada
    /// en cada bloque mediante un delta `INSERT` O(1) (`append_received_range`)
    /// para la reanudación tras fallos, sirviendo la reescritura completa como compactación
    /// periódica; el fsync diferido solo arriesga una pérdida en disco duro —
    /// los datos serían entonces inconsistentes pero la parte será
    /// reverificada (hash MD4/AICH) y redescargada si es inválida.
    pub persist_dirty: bool,
    /// Último vaciado a disco real (fsync + compactación de rangos) de los datos
    /// recibidos.
    pub last_persist_flush: Instant,
    /// Búferes RAM de las partes en curso de recepción (`m_tData` de eMule,
    /// preparación en libtorrent → hash → vaciado): la verificación MD4 de la parte
    /// se efectúa desde estos búferes sin relectura de disco cuando están
    /// completos — véase [`super::staged`].
    pub staged_parts: std::collections::HashMap<u32, super::staged::StagedPart>,
    /// Indica si la cabecera binaria de la parte 0 ya superó la validación de la política de vídeo.
    pub content_header_verified: bool,
}

/// Cadencia máxima de vaciado a disco de la ruta de recepción: `sync_data`
/// del `.part` + compactación completa de `received_ranges`. eMule vacía su
/// búfer de parte (`CPartFile::FlushBuffer`) periódicamente en lugar de en
/// cada bloque; el mismo enfoque aquí — los puntos críticos (finalización de
/// parte, fin de archivo, pausa, transición de estado, apagado) fuerzan
/// un vaciado inmediato. La reanudación tras fallo por bloque está garantizada por el delta
/// `append_received_range`, no por la reescritura completa.
pub(crate) const PART_PERSIST_INTERVAL: Duration = Duration::from_secs(30);

impl ActiveDownload {
    pub fn snapshot(&self) -> DownloadSnapshot {
        self.snapshot_impl(true)
    }

    /// Instantánea sin el resumen por parte (`compute_part_summaries` es
    /// O(partes × rangos)) — para la ruta crítica de recepción que solo consulta
    /// `status`/`progress`. `parts` permanece vacío: no emitir nunca
    /// hacia los clientes RPC/WS.
    pub fn snapshot_brief(&self) -> DownloadSnapshot {
        self.snapshot_impl(false)
    }

    fn snapshot_impl(&self, with_parts: bool) -> DownloadSnapshot {
        let effective_rate = if let Some(last_update) = self.last_rate_update {
            if last_update.elapsed() > Duration::from_secs(2) {
                0
            } else {
                self.current_rate_bps
            }
        } else {
            0
        };

        let mut snap = make_snapshot(
            &self.download,
            &self.ranges,
            self.source_count.max(self.sources.len() as u32),
            effective_rate,
        );
        if with_parts {
            // Los indicadores de partes verificadas residen en SQLite; la instantánea de lista solo conoce
            // "todas verificadas" para descargas completadas. `downloads.get` vuelve a leer
            // el conjunto fidedigno para la vista de detalle.
            let verified_parts: HashSet<u32> = if self.download.status == DownloadStatus::Completed {
                let pc = self
                    .download
                    .total_size
                    .div_ceil(ED2K_PART_SIZE as u64)
                    .max(1) as u32;
                (0..pc).collect()
            } else {
                HashSet::new()
            };
            snap.parts = compute_part_summaries(
                self.download.total_size,
                &self.ranges,
                self.sources.values(),
                &verified_parts,
            );
        }
        snap
    }
}

/// Ventana de frescura de las máscaras de disponibilidad (30 min, comparable al
/// ciclo de reconsulta de fuentes de eMule). Más allá, una fuente cuya sesión
/// haya muerto ya no contribuye a `available_source_count` — eMule retira el
/// cliente de `srclist` al desconectarse.
pub(crate) const SOURCE_FRESHNESS_MS: i64 = 30 * 60 * 1000;

/// Calcula el resumen de progreso por parte compartido por las instantáneas
/// de `downloads.list` y los detalles de `downloads.get` (estados de parte de `CPartFile` en eMule).
pub fn compute_part_summaries<'a>(
    total_size: u64,
    ranges: &RangeSet,
    sources: impl Iterator<Item = &'a ActiveSource>,
    verified_parts: &HashSet<u32>,
) -> Vec<PartProgressSummary> {
    let part_size_const = ED2K_PART_SIZE as u64;
    let part_count = if total_size == 0 {
        1
    } else {
        total_size.div_ceil(part_size_const) as u32
    };

    let all_ranges = ranges.ranges();
    let now = now_ms();
    let masks: Vec<&PartAvailabilityMask> = sources
        .filter(|s| now - s.last_seen_unix_ms <= SOURCE_FRESHNESS_MS)
        .map(|s| &s.mask)
        .collect();
    let mut parts = Vec::with_capacity(part_count as usize);

    for i in 0..part_count {
        let start_offset = i as u64 * part_size_const;
        let end_offset = ((i as u64 + 1) * part_size_const).min(total_size);
        let size = end_offset.saturating_sub(start_offset);

        let received_in_part: u64 = all_ranges
            .iter()
            .filter_map(|r| {
                let s = r.start.max(start_offset);
                let e = r.end.min(end_offset);
                if s < e {
                    Some(e - s)
                } else {
                    None
                }
            })
            .sum();

        let is_verified = verified_parts.contains(&i);
        let state = if is_verified {
            "verified"
        } else if received_in_part >= size && size > 0 {
            "completed"
        } else if received_in_part > 0 {
            "downloading"
        } else {
            "missing"
        };

        // Convención eD2K: `part_count == 0` (máscara vacía) = fuente
        // completa que cubre todas las partes — `ProcessFileStatus` de eMule
        // materializa entonces `m_abyPartStatus` todo a 1.
        let available_source_count = masks
            .iter()
            .filter(|m| m.part_count() == 0 || u16::try_from(i).is_ok_and(|p| m.has_part(p)))
            .count() as u32;

        // Mapa de bits recibido dentro de la parte: 64 celdas iguales, bit activado cuando la celda
        // está totalmente cubierta. Omitido para partes vacías/llenas (la máscara es trivial).
        let received_mask = if received_in_part == 0 {
            0
        } else if received_in_part >= size {
            u64::MAX
        } else {
            let cell = size.div_ceil(PART_MASK_CELLS);
            let mut mask = 0u64;
            for bit in 0..PART_MASK_CELLS {
                let cs = start_offset + bit * cell;
                let ce = (cs + cell).min(end_offset);
                if cs < ce && ranges.contains(cs..ce) {
                    mask |= 1 << bit;
                }
            }
            mask
        };

        parts.push(PartProgressSummary {
            part_index: i,
            start_offset,
            end_offset,
            size,
            received_bytes: received_in_part,
            is_verified,
            state: state.to_string(),
            available_source_count,
            received_mask,
        });
    }

    parts
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn make_snapshot(
    dl: &Download,
    ranges: &RangeSet,
    source_count: u32,
    download_rate_bps: u64,
) -> DownloadSnapshot {
    let received_bytes = ranges.covered_len();
    let progress = if dl.total_size > 0 {
        (received_bytes as f64 / dl.total_size as f64).min(1.0)
    } else {
        0.0
    };

    let verified_bytes = if dl.status == DownloadStatus::Completed {
        dl.total_size
    } else {
        0
    };

    let remaining = dl.total_size.saturating_sub(received_bytes);
    let eta_seconds = if download_rate_bps > 0 && remaining > 0 {
        Some(remaining / download_rate_bps)
    } else {
        None
    };

    DownloadSnapshot {
        id: dl.id,
        file_name: mule_format::fix_mojibake(&dl.display_name),
        total_bytes: dl.total_size,
        received_bytes,
        verified_bytes,
        progress,
        status: dl.status.as_str().to_string(),
        priority: dl.priority.clone(),
        source_count,
        download_rate_bps,
        uploaded_bytes: 0,
        created_at_unix_ms: dl.created_at,
        updated_at_unix_ms: dl.updated_at,
        category_id: dl.category_id.clone(),
        eta_seconds,
        ed2k_hash: dl.ed2k_hash.to_string(),
        parts: Vec::new(),
        pending_ranges: Vec::new(),
        max_sources: dl.max_sources,
        nas_copy_active: None,
        completion_stage: None,
        copy_progress: None,
        protocol: Some("ed2k".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(mask: PartAvailabilityMask, age_ms: i64) -> ActiveSource {
        ActiveSource {
            source_id: "src_test".to_string(),
            _addr: "127.0.0.1:49662".parse().unwrap(),
            mask,
            last_seen_unix_ms: now_ms() - age_ms,
        }
    }

    #[test]
    fn part_summaries_count_only_queried_sources() {
        // Archivo de 2 partes, nada recibido aún.
        let total = ED2K_PART_SIZE as u64 * 2;
        let ranges = RangeSet::new();
        let verified = HashSet::new();

        // Sin fuentes en absoluto → cada parte en rojo (0 fuentes).
        let parts = compute_part_summaries(
            total,
            &ranges,
            std::iter::empty::<&ActiveSource>(),
            &verified,
        );
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|p| p.available_source_count == 0));

        // Fuente parcial cubriendo solo la parte 0 → la parte 1 permanece en 0.
        let partial = PartAvailabilityMask::from_parts(&[0], 2);
        let sources = [src(partial, 0)];
        let parts = compute_part_summaries(total, &ranges, sources.iter(), &verified);
        assert_eq!(parts[0].available_source_count, 1);
        assert_eq!(parts[1].available_source_count, 0);
    }

    #[test]
    fn part_summaries_empty_mask_is_complete_source() {
        // Convención eD2K (ProcessFileStatus de eMule): part_count == 0 → el
        // par es una fuente completa que cubre cada parte.
        let total = ED2K_PART_SIZE as u64 * 3;
        let ranges = RangeSet::new();
        let verified = HashSet::new();

        let sources = [src(PartAvailabilityMask::empty(), 0)];
        let parts = compute_part_summaries(total, &ranges, sources.iter(), &verified);
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.available_source_count == 1));
    }

    #[test]
    fn part_summaries_stale_source_is_not_counted() {
        let total = ED2K_PART_SIZE as u64;
        let ranges = RangeSet::new();
        let verified = HashSet::new();

        // Máscara aprendida hace 31 min → fuera de la ventana de frescura.
        let stale = src(
            PartAvailabilityMask::from_parts(&[0], 1),
            SOURCE_FRESHNESS_MS + 60_000,
        );
        let sources = [stale];
        let parts = compute_part_summaries(total, &ranges, sources.iter(), &verified);
        assert_eq!(parts[0].available_source_count, 0);
    }
}

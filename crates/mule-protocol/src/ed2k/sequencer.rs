// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Secuenciación determinista de descargas eD2K y planificación de peticiones de bloques.
//!
//! El nivel 3.3 (Tier 3.3) aísla y formaliza las reglas de planificación previas a la transferencia:
//! - Comprobación determinista de disponibilidad de partes (respetando tanto las máscaras de bits parciales como
//!   la semántica de fichero completo de eMule, donde `part_count == 0` denota una fuente completa).
//! - Selección determinista de partes.
//! - Descomposición de partes de fichero en rangos de bloques eD2K estándar ($\le 184\,320$ bytes).
//! - Alineación estricta de límites de parte (límite de $9\,728\,000$ bytes) evitando bloques entre partes.
//! - Validación acotada frente a desbordamientos, desplazamientos no válidos y límites de fichero.
//! - Generación local de tramas de red para `OP_REQUESTPARTS_EMULE` (`0x47`) sin transmisión.
//! - Rechazo estricto de la planificación de bloques al recibir `OP_FILEREQANSNOFILE` (`0x48`).

pub mod sink;
pub mod types;

use crate::framing::RawPacket;
pub use sink::{Ed2kBlockSink, MAX_MEMORY_SINK_SIZE};
pub use types::{Ed2kSequencerError, InFlightBlock};

use super::encoder::Ed2kEncoder;
use super::packet::{
    Ed2kBlockRange, Ed2kBlockRange64, Ed2kFileStatus, Ed2kRequestParts, Ed2kRequestPartsI64,
    Ed2kSendingPart,
};
use super::session::Ed2kSessionError;

/// Tamaño estándar de una parte eD2K en bytes (9.500 KiB = 9.728.000 bytes).
pub const ED2K_PART_SIZE: usize = crate::limits::ED2K_PART_SIZE;

/// Tamaño estándar de un bloque de transferencia eD2K en bytes (180 KiB = 184.320 bytes).
pub const ED2K_BLOCK_SIZE: usize = crate::limits::ED2K_BLOCK_SIZE;

/// Número máximo de rangos de bloques permitidos por paquete de petición eD2K estándar (3).
pub const MAX_REQUEST_RANGES_PER_PACKET: usize = 3;

/// Umbral de fin de descarga (end-game) de eMule: cuando la descarga restante cabe en un
/// único lote de petición (≈ 3 bloques), un bloque que ya está en vuelo en *otro* par
/// puede solicitarse en paralelo para que el archivo termine a la velocidad de la
/// fuente más rápida (`CPartFile::GetNextRequestedBlock` duplica los últimos
/// fragmentos cerca de la finalización).
const ENDGAME_REMAINING_BYTES: u64 =
    ED2K_BLOCK_SIZE as u64 * MAX_REQUEST_RANGES_PER_PACKET as u64;

/// Número máximo de asignaciones simultáneas en vuelo permitidas para el mismo rango de bloque
/// durante el fin de descarga (la primera copia que llega gana; el duplicado se descarta).
const MAX_INFLIGHT_PER_BLOCK: usize = 2;

/// Secuenciador determinista de descargas y planificador de peticiones de bloques para un par eD2K activo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kDownloadSequencer {
    file_hash: [u8; 16],
    file_size: u64,
    total_parts: u16,
    status: Ed2kFileStatus,
    /// Modo de vista previa de eMule: la primera y la última parte se solicitan antes que todas
    /// las demás para poder previsualizar el archivo durante la descarga
    /// (`CPartFile` les asigna `PR_VERYHIGH` cuando la vista previa está activada).
    preview_priority: bool,
    /// Pista de recuperación del enjambre (extensión de eMuleRust): `part_scarcity[i]` = número
    /// de *fuentes conocidas* que ofrecen la parte `i` en todo el enjambre. Si se establece,
    /// la selección de partes prefiere primero las partes más raras (luego las menos recibidas),
    /// convirtiendo al daemon en fuente de rescate para partes a punto de desaparecer
    /// del enjambre. `None` mantiene la estrategia estándar de eMule por progreso.
    part_scarcity: Option<Vec<u32>>,
    /// Umbral personalizado de bytes restantes para fin de descarga (por defecto ENDGAME_REMAINING_BYTES si es None).
    endgame_threshold: Option<u64>,
}

impl Ed2kDownloadSequencer {
    /// Crea un nuevo `Ed2kDownloadSequencer` a partir de un hash de archivo conocido, el tamaño del archivo y un `Ed2kFileStatus` validado.
    pub fn new(
        file_hash: [u8; 16],
        file_size: u64,
        status: Ed2kFileStatus,
    ) -> Result<Self, Ed2kSequencerError> {
        if file_size == 0 {
            return Err(Ed2kSequencerError::EmptyFile);
        }
        if status.file_hash != file_hash {
            return Err(Ed2kSequencerError::HashMismatch {
                expected: hex::encode(file_hash),
                actual: hex::encode(status.file_hash),
            });
        }

        let total_parts = (file_size.div_ceil(ED2K_PART_SIZE as u64)).max(1) as u16;

        Ok(Self {
            file_hash,
            file_size,
            total_parts,
            status,
            preview_priority: false,
            part_scarcity: None,
            endgame_threshold: None,
        })
    }

    /// Establece un umbral personalizado de bytes restantes para entrar en el modo de fin de descarga.
    pub fn with_endgame_threshold(mut self, bytes: u64) -> Self {
        self.endgame_threshold = Some(bytes);
        self
    }

    /// Activa la prioridad de partes de vista previa de eMule: la primera y la última parte se
    /// planifican antes que todas las demás (usado para archivos multimedia previsualizables).
    pub fn with_preview_priority(mut self, enabled: bool) -> Self {
        self.preview_priority = enabled;
        self
    }

    /// Instala los recuentos de escasez en el enjambre por parte utilizados por la estrategia de
    /// recuperación del enjambre (`part_scarcity[i]` = fuentes que ofrecen la parte `i`). Los vectores
    /// cuya longitud difiera de `total_parts` se ignoran.
    pub fn with_part_scarcity(mut self, counts: Vec<u32>) -> Self {
        if counts.len() == self.total_parts as usize {
            self.part_scarcity = Some(counts);
        }
        self
    }

    /// Crea un secuenciador a partir del resultado de una petición de estado de archivo.
    pub fn from_file_status_result(
        file_hash: [u8; 16],
        file_size: u64,
        result: Result<Ed2kFileStatus, Ed2kSessionError>,
    ) -> Result<Self, Ed2kSequencerError> {
        match result {
            Ok(status) => Self::new(file_hash, file_size, status),
            Err(Ed2kSessionError::FileNotFound(h)) => Err(Ed2kSequencerError::FileNotFound(h)),
            Err(e) => Err(Ed2kSequencerError::FileNotFound(e.to_string())),
        }
    }

    /// Devuelve el hash MD4 de 16 bytes del archivo.
    #[inline]
    pub fn file_hash(&self) -> [u8; 16] {
        self.file_hash
    }

    /// Devuelve el tamaño total del archivo en bytes.
    #[inline]
    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    /// Devuelve el número total de partes de este archivo.
    #[inline]
    pub fn total_parts(&self) -> u16 {
        self.total_parts
    }

    /// Referencia al `Ed2kFileStatus` subyacente.
    #[inline]
    pub fn status(&self) -> &Ed2kFileStatus {
        &self.status
    }

    /// Devuelve true si el par remoto tiene disponible el índice de parte especificado.
    pub fn is_part_available(&self, part_index: u16) -> bool {
        if part_index >= self.total_parts {
            return false;
        }
        if self.status.is_complete() {
            true
        } else {
            self.status.has_part(part_index)
        }
    }

    /// Devuelve una lista de todos los índices de partes disponibles de esta fuente remota.
    pub fn available_parts(&self) -> Vec<u16> {
        (0..self.total_parts)
            .filter(|&p| self.is_part_available(p))
            .collect()
    }

    /// Devuelve el primer índice de parte disponible en orden ascendente, si lo hay.
    pub fn select_first_available_part(&self) -> Option<u16> {
        (0..self.total_parts).find(|&p| self.is_part_available(p))
    }

    /// Selecciona y valida un índice de parte explícito para la descarga.
    pub fn select_part(&self, part_index: u16) -> Result<u16, Ed2kSequencerError> {
        if part_index >= self.total_parts {
            return Err(Ed2kSequencerError::PartIndexOutOfBounds {
                part_index,
                total_parts: self.total_parts,
            });
        }
        if !self.is_part_available(part_index) {
            return Err(Ed2kSequencerError::PartNotAvailable {
                part_index,
                total_parts: self.total_parts,
            });
        }
        Ok(part_index)
    }

    /// Devuelve los límites de bytes semiabiertos `[start, end)` para un índice de parte dado.
    pub fn part_byte_bounds(&self, part_index: u16) -> Result<(u64, u64), Ed2kSequencerError> {
        if part_index >= self.total_parts {
            return Err(Ed2kSequencerError::PartIndexOutOfBounds {
                part_index,
                total_parts: self.total_parts,
            });
        }
        let start = part_index as u64 * ED2K_PART_SIZE as u64;
        let end = std::cmp::min(start + ED2K_PART_SIZE as u64, self.file_size);
        Ok((start, end))
    }

    /// Planifica la secuencia de rangos de bloques ($\le 184\,320$ bytes cada uno) para una parte seleccionada.
    pub fn plan_part_blocks(
        &self,
        part_index: u16,
    ) -> Result<Vec<Ed2kBlockRange>, Ed2kSequencerError> {
        self.select_part(part_index)?;
        let (part_start, part_end) = self.part_byte_bounds(part_index)?;

        let mut blocks = Vec::new();
        let mut curr = part_start;
        while curr < part_end {
            let block_end = std::cmp::min(curr + ED2K_BLOCK_SIZE as u64, part_end);
            let range =
                Ed2kBlockRange::from_wire_bounds(curr as u32, block_end as u32).map_err(|e| {
                    Ed2kSequencerError::InvalidBlockRange {
                        start: curr,
                        end: block_end,
                        reason: e.to_string(),
                    }
                })?;
            blocks.push(range);
            curr = block_end;
        }

        Ok(blocks)
    }

    /// Variante de 64 bits de `plan_part_blocks` para archivos mayores de 4 GiB:
    /// los desplazamientos que superan `u32::MAX` se conservan en lugar de truncarse.
    pub fn plan_part_blocks64(
        &self,
        part_index: u16,
    ) -> Result<Vec<Ed2kBlockRange64>, Ed2kSequencerError> {
        self.select_part(part_index)?;
        let (part_start, part_end) = self.part_byte_bounds(part_index)?;

        let mut blocks = Vec::new();
        let mut curr = part_start;
        while curr < part_end {
            let block_end = std::cmp::min(curr + ED2K_BLOCK_SIZE as u64, part_end);
            blocks.push(Ed2kBlockRange64::new(curr, block_end));
            curr = block_end;
        }

        Ok(blocks)
    }

    /// Valida un rango de bloques arbitrario de 64 bits `[start, end)` frente a todos
    /// los invariantes de protocolo y archivo (mismas reglas que `validate_block_range`).
    pub fn validate_block_range64(
        &self,
        start: u64,
        end: u64,
    ) -> Result<Ed2kBlockRange64, Ed2kSequencerError> {
        if start >= end {
            return Err(Ed2kSequencerError::InvalidBlockRange {
                start,
                end,
                reason: format!("start offset {start} must be strictly less than end offset {end}"),
            });
        }
        if end > self.file_size {
            return Err(Ed2kSequencerError::InvalidBlockRange {
                start,
                end,
                reason: format!("end offset {end} exceeds file size {}", self.file_size),
            });
        }
        let len = end - start;
        if len > ED2K_BLOCK_SIZE as u64 {
            return Err(Ed2kSequencerError::InvalidBlockRange {
                start,
                end,
                reason: format!(
                    "block length {len} exceeds maximum allowed {ED2K_BLOCK_SIZE} bytes"
                ),
            });
        }

        let p_start = (start / ED2K_PART_SIZE as u64) as u16;
        let p_end = ((end - 1) / ED2K_PART_SIZE as u64) as u16;
        if p_start != p_end {
            let part_boundary = (p_start as u64 + 1) * ED2K_PART_SIZE as u64;
            return Err(Ed2kSequencerError::CrossPartBoundary {
                start,
                end,
                part_boundary,
            });
        }

        if !self.is_part_available(p_start) {
            return Err(Ed2kSequencerError::PartNotAvailable {
                part_index: p_start,
                total_parts: self.total_parts,
            });
        }

        Ok(Ed2kBlockRange64::new(start, end))
    }

    /// Construye un mensaje `Ed2kRequestPartsI64` (`OP_REQUESTPARTS_I64`) validado
    /// a partir de 1 a 3 rangos de bloques de 64 bits.
    pub fn build_request_parts_i64(
        &self,
        ranges: &[Ed2kBlockRange64],
    ) -> Result<Ed2kRequestPartsI64, Ed2kSequencerError> {
        if ranges.is_empty() {
            return Err(Ed2kSequencerError::EmptyRanges);
        }
        if ranges.len() > MAX_REQUEST_RANGES_PER_PACKET {
            return Err(Ed2kSequencerError::TooManyRanges {
                count: ranges.len(),
                max: MAX_REQUEST_RANGES_PER_PACKET,
            });
        }
        for range in ranges {
            self.validate_block_range64(range.start, range.end)?;
        }
        Ok(Ed2kRequestPartsI64 {
            file_hash: self.file_hash,
            ranges: ranges.to_vec(),
        })
    }

    /// Variante de 64 bits de `plan_next_block_for_peer`: necesaria para descargas
    /// cuyos bloques sobrepasan el límite de 4 GiB.
    ///
    /// La selección de partes sigue la estrategia de eMule "más rara/progreso primero":
    /// en lugar de agotar siempre el índice de parte más bajo (lo que concentra a todo
    /// el enjambre en los mismos fragmentos), se asigna al par la parte necesaria que
    /// ofrece y que actualmente tiene la *menor* cantidad de bytes completados.
    /// Los empates se resuelven a favor del índice más bajo. Esto distribuye las fuentes
    /// entre diferentes partes tal como hace `CPartFile::GetNextRequestedBlock`.
    pub fn plan_next_block_for_peer64(
        &self,
        peer_id: &str,
        completed_ranges: &[(u64, u64)],
        in_flight_blocks: &[InFlightBlock],
        now_unix_ms: i64,
    ) -> Result<Option<(u16, Ed2kBlockRange64)>, Ed2kSequencerError> {
        // Vista previa de eMule: la primera y la última parte se sirven primero para que el
        // archivo pueda abrirse durante la descarga (`PR_VERYHIGH` en los extremos).
        if self.preview_priority {
            let last = self.total_parts - 1;
            for edge in [0u16, last] {
                if !self.is_part_available(edge) {
                    continue;
                }
                if let Some(block) =
                    self.next_free_block_in_part64(edge, completed_ranges, in_flight_blocks, now_unix_ms)?
                {
                    return Ok(Some((edge, block)));
                }
            }
        }

        // Recuperación del enjambre: cuando se conocen los recuentos de escasez por parte, las partes
        // ofrecidas por menos fuentes se seleccionan primero (prioridad de rescate),
        // luego las menos recibidas. Si no, progreso primero estándar de eMule.
        let scarcity_of = |p: u16| -> u32 {
            self.part_scarcity
                .as_ref()
                .and_then(|s| s.get(p as usize).copied())
                .unwrap_or(u32::MAX)
        };
        let mut best: Option<(u16, Ed2kBlockRange64, u32, u64)> = None;

        for part_idx in 0..self.total_parts {
            if !self.is_part_available(part_idx) {
                continue;
            }

            // Bytes recibidos que ya cubren esta parte (métrica de progreso).
            let (part_start, part_end) = self.part_byte_bounds(part_idx)?;
            let received_in_part: u64 = completed_ranges
                .iter()
                .filter_map(|&(s, e)| {
                    let cs = s.max(part_start);
                    let ce = e.min(part_end);
                    (cs < ce).then(|| ce - cs)
                })
                .sum();
            if received_in_part >= part_end - part_start {
                continue;
            }

            if let Some(block) =
                self.next_free_block_in_part64(part_idx, completed_ranges, in_flight_blocks, now_unix_ms)?
            {
                let scarcity = scarcity_of(part_idx);
                let replace = match &best {
                    None => true,
                    Some((_, _, best_sc, best_received)) => {
                        (scarcity, received_in_part) < (*best_sc, *best_received)
                    }
                };
                if replace {
                    best = Some((part_idx, block, scarcity, received_in_part));
                }
            }
        }

        if let Some((part_idx, block, _, _)) = best {
            return Ok(Some((part_idx, block)));
        }

        self.endgame_block64(peer_id, completed_ranges, in_flight_blocks, now_unix_ms)
    }

    /// Planifica hasta `max_blocks` rangos de bloques disjuntos para un par en una sola pasada: semántica
    /// de `CreateBlockRequests` + `SendBlockRequests` de eMule: un paquete `OP_REQUESTPARTS` transporta
    /// hasta [`MAX_REQUEST_RANGES_PER_PACKET`] rangos para que varios bloques permanezcan en vuelo por
    /// fuente en lugar de realizar ida y vuelta de uno en uno.
    ///
    /// Los rangos devueltos se tratan como ya asignados: se espera que quien llama los registre
    /// en `in_flight_blocks` antes de planificar de nuevo. El orden de selección sigue
    /// `plan_next_block_for_peer64` (extremos de vista previa, escasez/progreso primero); los duplicados de
    /// fin de descarga no se procesan en lote intencionadamente: quedan como respaldo de un solo rango.
    pub fn plan_next_blocks_for_peer64(
        &self,
        peer_id: &str,
        completed_ranges: &[(u64, u64)],
        in_flight_blocks: &[InFlightBlock],
        now_unix_ms: i64,
        max_blocks: usize,
    ) -> Result<Vec<(u16, Ed2kBlockRange64)>, Ed2kSequencerError> {
        let mut planned: Vec<(u16, Ed2kBlockRange64)> = Vec::new();
        // Lista oculta de seguimiento: bloques en vuelo reales más los rangos ya reservados
        // por esta pasada para no planificar ningún rango dos veces.
        let mut shadow: Vec<InFlightBlock> = in_flight_blocks.to_vec();
        while planned.len() < max_blocks {
            match self.plan_next_block_for_peer64(
                peer_id,
                completed_ranges,
                &shadow,
                now_unix_ms,
            )? {
                Some((part_idx, block)) => {
                    shadow.push(InFlightBlock::new(
                        peer_id.to_string(),
                        part_idx,
                        block.start,
                        block.end,
                        now_unix_ms,
                        i64::MAX - now_unix_ms, // nunca expira dentro de una pasada
                    ));
                    planned.push((part_idx, block));
                }
                None => break,
            }
            // Los duplicados de fin de descarga no deben agruparse en una sola petición:
            // `plan_next_block_for_peer64` recurre a `endgame_block64` solo cuando no hay nada más libre;
            // detecta ese caso comprobando si el bloque devuelto ya estaba en vuelo en otro par,
            // y detiene la agrupación en lote allí.
            if let Some((_, block)) = planned.last() {
                if in_flight_blocks.iter().any(|ifb| {
                    !ifb.is_expired(now_unix_ms)
                        && ifb.peer_id != peer_id
                        && ifb.overlaps(block.start, block.end)
                }) {
                    break;
                }
            }
        }
        Ok(planned)
    }

    /// Mecanismo de respaldo de fin de descarga de eMule: cuando quedan menos de `ENDGAME_REMAINING_BYTES`,
    /// permite solicitar un bloque que ya está en vuelo en un par *diferente* (máximo de
    /// `MAX_INFLIGHT_PER_BLOCK` asignaciones simultáneas por bloque). Devuelve `None` fuera del
    /// fin de descarga, cuando todos los bloques faltantes ya están doblemente asignados, o cuando
    /// las únicas copias pendientes pertenecen a este par.
    fn endgame_block64(
        &self,
        peer_id: &str,
        completed_ranges: &[(u64, u64)],
        in_flight_blocks: &[InFlightBlock],
        now_unix_ms: i64,
    ) -> Result<Option<(u16, Ed2kBlockRange64)>, Ed2kSequencerError> {
        let covered: u64 = completed_ranges
            .iter()
            .map(|&(s, e)| e.saturating_sub(s))
            .sum();
        let remaining = self.file_size.saturating_sub(covered);
        let threshold = self.endgame_threshold.unwrap_or(ENDGAME_REMAINING_BYTES);
        if remaining == 0 || remaining > threshold {
            return Ok(None);
        }
        for part_idx in 0..self.total_parts {
            if !self.is_part_available(part_idx) {
                continue;
            }
            for block in self.plan_part_blocks64(part_idx)? {
                if Self::is_range_in_list(block.start, block.end, completed_ranges) {
                    continue;
                }
                let mut active = 0usize;
                let mut same_peer = false;
                for ifb in in_flight_blocks {
                    if ifb.is_expired(now_unix_ms) || !ifb.overlaps(block.start, block.end) {
                        continue;
                    }
                    if ifb.peer_id == peer_id {
                        same_peer = true;
                        break;
                    }
                    active += 1;
                }
                if same_peer || active == 0 || active >= MAX_INFLIGHT_PER_BLOCK {
                    continue;
                }
                return Ok(Some((part_idx, block)));
            }
        }
        Ok(None)
    }

    /// Devuelve el primer bloque faltante que no esté en vuelo dentro de `part_idx`,
    /// o `None` si la parte está completa, totalmente en vuelo o no la ofrece este par.
    fn next_free_block_in_part64(
        &self,
        part_idx: u16,
        completed_ranges: &[(u64, u64)],
        in_flight_blocks: &[InFlightBlock],
        now_unix_ms: i64,
    ) -> Result<Option<Ed2kBlockRange64>, Ed2kSequencerError> {
        let blocks = self.plan_part_blocks64(part_idx)?;
        for mut block in blocks {
            if Self::is_range_in_list(block.start, block.end, completed_ranges) {
                continue;
            }
            // Recorta el bloque hacia adelante y hacia atrás superando los rangos completados que cubran sus límites
            for &(s, e) in completed_ranges {
                if s <= block.start && e > block.start {
                    block.start = e.min(block.end);
                }
                if s < block.end && e >= block.end {
                    block.end = s.max(block.start);
                }
            }
            if block.start >= block.end {
                continue;
            }
            let in_flight = in_flight_blocks.iter().any(|ifb| {
                !ifb.is_expired(now_unix_ms) && ifb.overlaps(block.start, block.end)
            });
            if !in_flight {
                return Ok(Some(block));
            }
        }
        Ok(None)
    }

    /// Planifica la secuencia de rangos de bloques para una parte, filtrando los bloques
    /// que ya estén totalmente cubiertos por los rangos completados dados.
    pub fn plan_missing_blocks(
        &self,
        part_index: u16,
        completed_ranges: &[(u64, u64)],
    ) -> Result<Vec<Ed2kBlockRange>, Ed2kSequencerError> {
        let all_blocks = self.plan_part_blocks(part_index)?;
        let missing = all_blocks
            .into_iter()
            .filter(|block| {
                let b_start = block.start as u64;
                let b_end = block.end as u64;
                !Self::is_range_in_list(b_start, b_end, completed_ranges)
            })
            .collect();
        Ok(missing)
    }

    /// Función auxiliar para comprobar si el rango semiabierto de bytes `[start, end)` está completamente cubierto por una lista de rangos.
    pub fn is_range_in_list(start: u64, end: u64, ranges: &[(u64, u64)]) -> bool {
        if start >= end {
            return true;
        }
        let mut curr = start;
        let mut matching_ranges: Vec<(u64, u64)> = ranges
            .iter()
            .copied()
            .filter(|&(s, e)| e > start && s < end)
            .collect();
        matching_ranges.sort_unstable_by_key(|r| r.0);

        for (s, e) in matching_ranges {
            if s > curr {
                return false;
            }
            if e > curr {
                curr = e;
            }
            if curr >= end {
                return true;
            }
        }
        curr >= end
    }

    /// Valida un rango de bloques arbitrario `[start, end)` frente a todos los invariantes de protocolo y archivo.
    pub fn validate_block_range(
        &self,
        start: u64,
        end: u64,
    ) -> Result<Ed2kBlockRange, Ed2kSequencerError> {
        if start >= end {
            return Err(Ed2kSequencerError::InvalidBlockRange {
                start,
                end,
                reason: format!("start offset {start} must be strictly less than end offset {end}"),
            });
        }
        if end > self.file_size {
            return Err(Ed2kSequencerError::InvalidBlockRange {
                start,
                end,
                reason: format!("end offset {end} exceeds file size {}", self.file_size),
            });
        }
        let len = end - start;
        if len > ED2K_BLOCK_SIZE as u64 {
            return Err(Ed2kSequencerError::InvalidBlockRange {
                start,
                end,
                reason: format!(
                    "block length {len} exceeds maximum allowed {ED2K_BLOCK_SIZE} bytes"
                ),
            });
        }

        let p_start = (start / ED2K_PART_SIZE as u64) as u16;
        let p_end = ((end - 1) / ED2K_PART_SIZE as u64) as u16;
        if p_start != p_end {
            let part_boundary = (p_start as u64 + 1) * ED2K_PART_SIZE as u64;
            return Err(Ed2kSequencerError::CrossPartBoundary {
                start,
                end,
                part_boundary,
            });
        }

        if !self.is_part_available(p_start) {
            return Err(Ed2kSequencerError::PartNotAvailable {
                part_index: p_start,
                total_parts: self.total_parts,
            });
        }

        Ed2kBlockRange::from_wire_bounds(start as u32, end as u32).map_err(|e| {
            Ed2kSequencerError::InvalidBlockRange {
                start,
                end,
                reason: e.to_string(),
            }
        })
    }

    /// Construye un mensaje `Ed2kRequestParts` validado a partir de 1 a 3 rangos de bloques.
    pub fn build_request_parts(
        &self,
        ranges: &[Ed2kBlockRange],
    ) -> Result<Ed2kRequestParts, Ed2kSequencerError> {
        if ranges.is_empty() {
            return Err(Ed2kSequencerError::EmptyRanges);
        }
        if ranges.len() > MAX_REQUEST_RANGES_PER_PACKET {
            return Err(Ed2kSequencerError::TooManyRanges {
                count: ranges.len(),
                max: MAX_REQUEST_RANGES_PER_PACKET,
            });
        }

        for range in ranges {
            self.validate_block_range(range.start as u64, range.end as u64)?;
        }

        Ok(Ed2kRequestParts::multi(self.file_hash, ranges.to_vec()))
    }

    /// Construye y codifica un paquete de cable `OP_REQUESTPARTS_EMULE` (`0x47`) sin transmitirlo.
    pub fn build_request_packet(
        &self,
        ranges: &[Ed2kBlockRange],
        encoder: &Ed2kEncoder,
    ) -> Result<RawPacket, Ed2kSequencerError> {
        let req = self.build_request_parts(ranges)?;
        encoder
            .encode_request_parts(&req)
            .map_err(|e| Ed2kSequencerError::Encode(e.to_string()))
    }

    /// Valida un bloque `Ed2kSendingPart` entrante frente al hash del archivo, longitud de datos,
    /// límite de longitud del bloque y límites de parte.
    pub fn validate_sending_part(
        &self,
        part: &Ed2kSendingPart,
    ) -> Result<Ed2kBlockRange, Ed2kSequencerError> {
        if part.file_hash != self.file_hash {
            return Err(Ed2kSequencerError::HashMismatch {
                expected: hex::encode(self.file_hash),
                actual: hex::encode(part.file_hash),
            });
        }

        let expected_len = part.end_offset.saturating_sub(part.start_offset) as usize;
        if part.data.len() != expected_len {
            return Err(Ed2kSequencerError::BlockDataMismatch {
                expected: expected_len,
                actual: part.data.len(),
            });
        }

        self.validate_block_range(part.start_offset as u64, part.end_offset as u64)
    }

    /// Planifica el siguiente rango de bloques no asignado y no completado para un par específico.
    ///
    /// Misma selección de partes más rara/progreso primero que `plan_next_block_for_peer64`: se asigna
    /// al par la parte necesaria que ofrece con la menor cantidad de bytes completados (empates → índice más bajo).
    pub fn plan_next_block_for_peer(
        &self,
        peer_id: &str,
        completed_ranges: &[(u64, u64)],
        in_flight_blocks: &[InFlightBlock],
        now_unix_ms: i64,
    ) -> Result<Option<(u16, Ed2kBlockRange)>, Ed2kSequencerError> {
        if self.preview_priority {
            let last = self.total_parts - 1;
            for edge in [0u16, last] {
                if !self.is_part_available(edge) {
                    continue;
                }
                if let Some(block) =
                    self.next_free_block_in_part(edge, completed_ranges, in_flight_blocks, now_unix_ms)?
                {
                    return Ok(Some((edge, block)));
                }
            }
        }

        let scarcity_of = |p: u16| -> u32 {
            self.part_scarcity
                .as_ref()
                .and_then(|s| s.get(p as usize).copied())
                .unwrap_or(u32::MAX)
        };
        let mut best: Option<(u16, Ed2kBlockRange, u32, u64)> = None;

        for part_idx in 0..self.total_parts {
            if !self.is_part_available(part_idx) {
                continue;
            }

            let (part_start, part_end) = self.part_byte_bounds(part_idx)?;
            let received_in_part: u64 = completed_ranges
                .iter()
                .filter_map(|&(s, e)| {
                    let cs = s.max(part_start);
                    let ce = e.min(part_end);
                    (cs < ce).then(|| ce - cs)
                })
                .sum();
            if received_in_part >= part_end - part_start {
                continue;
            }

            if let Some(block) =
                self.next_free_block_in_part(part_idx, completed_ranges, in_flight_blocks, now_unix_ms)?
            {
                let scarcity = scarcity_of(part_idx);
                let replace = match &best {
                    None => true,
                    Some((_, _, best_sc, best_received)) => {
                        (scarcity, received_in_part) < (*best_sc, *best_received)
                    }
                };
                if replace {
                    best = Some((part_idx, block, scarcity, received_in_part));
                }
            }
        }

        if let Some((part_idx, block, _, _)) = best {
            return Ok(Some((part_idx, block)));
        }

        self.endgame_block(peer_id, completed_ranges, in_flight_blocks, now_unix_ms)
    }

    /// Variante de 32 bits de `endgame_block64`.
    fn endgame_block(
        &self,
        peer_id: &str,
        completed_ranges: &[(u64, u64)],
        in_flight_blocks: &[InFlightBlock],
        now_unix_ms: i64,
    ) -> Result<Option<(u16, Ed2kBlockRange)>, Ed2kSequencerError> {
        let covered: u64 = completed_ranges
            .iter()
            .map(|&(s, e)| e.saturating_sub(s))
            .sum();
        let remaining = self.file_size.saturating_sub(covered);
        let threshold = self.endgame_threshold.unwrap_or(ENDGAME_REMAINING_BYTES);
        if remaining == 0 || remaining > threshold {
            return Ok(None);
        }
        for part_idx in 0..self.total_parts {
            if !self.is_part_available(part_idx) {
                continue;
            }
            for block in self.plan_part_blocks(part_idx)? {
                let b_start = block.start as u64;
                let b_end = block.end as u64;
                if Self::is_range_in_list(b_start, b_end, completed_ranges) {
                    continue;
                }
                let mut active = 0usize;
                let mut same_peer = false;
                for ifb in in_flight_blocks {
                    if ifb.is_expired(now_unix_ms) || !ifb.overlaps(b_start, b_end) {
                        continue;
                    }
                    if ifb.peer_id == peer_id {
                        same_peer = true;
                        break;
                    }
                    active += 1;
                }
                if same_peer || active == 0 || active >= MAX_INFLIGHT_PER_BLOCK {
                    continue;
                }
                return Ok(Some((part_idx, block)));
            }
        }
        Ok(None)
    }

    /// Variante de 32 bits de `next_free_block_in_part64`.
    fn next_free_block_in_part(
        &self,
        part_idx: u16,
        completed_ranges: &[(u64, u64)],
        in_flight_blocks: &[InFlightBlock],
        now_unix_ms: i64,
    ) -> Result<Option<Ed2kBlockRange>, Ed2kSequencerError> {
        let blocks = self.plan_part_blocks(part_idx)?;
        for mut block in blocks {
            let mut b_start = block.start as u64;
            let mut b_end = block.end as u64;
            if Self::is_range_in_list(b_start, b_end, completed_ranges) {
                continue;
            }
            for &(s, e) in completed_ranges {
                if s <= b_start && e > b_start {
                    b_start = e.min(b_end);
                }
                if s < b_end && e >= b_end {
                    b_end = s.max(b_start);
                }
            }
            if b_start >= b_end {
                continue;
            }
            let in_flight = in_flight_blocks
                .iter()
                .any(|ifb| !ifb.is_expired(now_unix_ms) && ifb.overlaps(b_start, b_end));
            if !in_flight {
                block.start = b_start as u32;
                block.end = b_end as u32;
                return Ok(Some(block));
            }
        }
        Ok(None)
    }
}

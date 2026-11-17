// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Rasgos (traits) asíncronos de alto nivel que desacoplan el motor del protocolo de red
//! del almacenamiento subyacente (`mule-core`) y de la lógica de planificación de piezas.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use crate::error::ProtocolError;
use crate::types::PeerId;

/// Un fragmento contiguo de datos de archivo recibido de un par.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedBlock {
    /// Hash de archivo eD2K de 16 bytes.
    pub file_hash: [u8; 16],
    /// Desplazamiento absoluto en bytes dentro del archivo donde comienza este bloque.
    pub start_offset: u64,
    /// Carga útil de datos binarios para este bloque.
    pub data: Bytes,
}

impl ReceivedBlock {
    /// Crea un nuevo `ReceivedBlock`.
    pub fn new(file_hash: [u8; 16], start_offset: u64, data: Bytes) -> Self {
        Self {
            file_hash,
            start_offset,
            data,
        }
    }

    /// Desplazamiento de byte final (exclusivo: `start_offset + data.len()`).
    #[inline]
    pub fn end_offset(&self) -> u64 {
        self.start_offset + self.data.len() as u64
    }

    /// Longitud en bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Devuelve verdadero si este bloque tiene datos vacíos.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// El resultado devuelto por un `DownloadSink` tras persistir un bloque recibido.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlockWriteOutcome {
    /// Número de bytes confirmados con éxito en el almacenamiento.
    pub bytes_written: usize,
    /// Indica si este bloque completó el límite de una parte eD2K (9,5 MB).
    pub part_completed: bool,
    /// Índice de la parte completada, si corresponde.
    pub completed_part_index: Option<u32>,
    /// Indica si esta escritura completó el archivo de destino por completo.
    pub file_completed: bool,
}

/// Sumidero de almacenamiento abstracto para escribir bloques descargados de pares.
///
/// Los implementadores gestionan las escrituras físicas en disco, las actualizaciones
/// de archivos dispersos `.part` y las comprobaciones de integridad de hash sin acoplarse
/// a la capa del protocolo de red.
pub trait DownloadSink: Send + Sync {
    /// Persiste un bloque descargado en el almacenamiento.
    fn write_block(
        &self,
        block: ReceivedBlock,
    ) -> Pin<Box<dyn Future<Output = Result<BlockWriteOutcome, ProtocolError>> + Send + '_>>;
}

/// Petición de un único bloque de datos a un par.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRequest {
    /// Hash de archivo eD2K de 16 bytes solicitado.
    pub file_hash: [u8; 16],
    /// Desplazamiento absoluto en bytes donde comienza el bloque solicitado.
    pub start_offset: u64,
    /// Longitud del bloque solicitado en bytes (lo estándar son 184.320 bytes = 180 KiB).
    pub length: u32,
}

impl BlockRequest {
    /// Crea un nuevo `BlockRequest`.
    pub fn new(file_hash: [u8; 16], start_offset: u64, length: u32) -> Self {
        Self {
            file_hash,
            start_offset,
            length,
        }
    }

    /// Desplazamiento de byte final (exclusivo: `start_offset + length`).
    #[inline]
    pub fn end_offset(&self) -> u64 {
        self.start_offset + self.length as u64
    }
}

/// Actualización de estado que describe el estado y la disponibilidad de partes de un par remoto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStateUpdate {
    /// Identificador del par.
    pub peer_id: PeerId,
    /// Hash de archivo eD2K de 16 bytes.
    pub file_hash: [u8; 16],
    /// Máscara de bits de disponibilidad de partes (true = el par dispone de esta parte).
    pub available_parts: Vec<bool>,
    /// Posición o rango opcional en la cola de subida remota.
    pub queue_rank: Option<u32>,
}

/// Planificador abstracto responsable de la programación de descargas, la selección de piezas
/// y la gestión de peticiones entre pares activos.
pub trait DownloadPlanner: Send + Sync {
    /// Obtiene la siguiente petición de bloque a enviar a un par específico, si algún bloque es elegible.
    fn next_request(
        &self,
        peer_id: &PeerId,
    ) -> Pin<Box<dyn Future<Output = Option<BlockRequest>> + Send + '_>>;

    /// Notifica al planificador una actualización en el estado del par remoto (máscara de bits de partes, posición en cola, etc.).
    fn on_source_state(
        &self,
        update: SourceStateUpdate,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

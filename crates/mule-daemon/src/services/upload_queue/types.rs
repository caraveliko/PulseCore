// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde::{Deserialize, Serialize};

use super::RATE_WINDOW_MS;

pub(super) type UploadSpeedProbe = Option<std::sync::Arc<dyn Fn() -> u64 + Send + Sync>>;

/// Estado de una ranura de subida o entrada en cola.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadSlotState {
    Waiting,
    Uploading,
    /// Cliente lento mantenido con ancho de banda residual; no cuenta para
    /// `max_active` ("ranura de goteo" / trickle slot de eMule).
    Trickle,
    Completed,
    Rejected,
    Aborted,
}

/// Instantánea de una sesión o ranura de subida.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UploadSessionSnapshot {
    /// Clave de identidad para deduplicación: `uh:<userhash>` si el par anunció
    /// un hash de usuario en OP_HELLO; en caso contrario, su dirección `ip:port`.
    pub client_key: String,
    pub client_address: String,
    pub file_hash: String,
    pub file_name: String,
    pub transferred_bytes: u64,
    pub upload_rate_bps: u64,
    pub state: UploadSlotState,
    pub queue_rank: Option<u32>,
    pub started_at_unix_ms: i64,
    /// Inicio de la ventana de medición actual (ms Unix, 0 = sin medición).
    /// Estado interno de `UploadManager` — no serializado.
    #[serde(skip)]
    pub rate_window_start_ms: i64,
    /// Bytes enviados en la ventana actual. No serializado.
    #[serde(skip)]
    pub rate_window_bytes: u64,
    /// `true` en cuanto se cierra una ventana de medición — requerido para la
    /// degradación a `Trickle` (una velocidad no medida aún no se considera «lenta»).
    /// No serializado.
    #[serde(skip)]
    pub rate_sampled: bool,
    /// Multiplicador de créditos ×100 en el momento de admisión — conservado
    /// para reconstruir la entrada de la cola durante una rotación de sesión
    /// (eMule preserva la puntuación mediante `credits`). No serializado.
    #[serde(skip)]
    pub credit_multiplier_bps: u32,
    /// Peso de prioridad del archivo servido (`GetFilePrioAsNumber`) — mismo
    /// uso que `credit_multiplier_bps`. No serializado.
    #[serde(skip)]
    pub file_prio_num: u32,
    /// Inicio de espera original (eMule `credits->GetSecWaitStartTime`):
    /// `enqueued_at` de la entrada promocionada, o la hora de concesión directa —
    /// preservado para que un cliente devuelto a la cola recupere su puntuación
    /// (rotación de sesión). No serializado.
    #[serde(skip)]
    pub queued_since_ms: i64,
    /// `true` cuando el archivo servido tiene prioridad de subida «automática»
    /// (eMule `m_bAutoUpPriority`). No serializado.
    #[serde(skip)]
    pub auto_prio: bool,
}

impl UploadSessionSnapshot {
    /// Velocidad efectiva en `now_ms`: la última tasa muestreada mientras
    /// la ventana actual permanezca reciente; tras `RATE_WINDOW_MS` sin
    /// nuevos envíos, los bytes residuales de la ventana se ponderan según el
    /// tiempo transcurrido — la tasa decrece hacia 0 cuando el par deja de solicitar
    /// datos (misma convención que `decay_stale_rates` en descargas).
    pub fn effective_rate_bps(&self, now_ms: i64) -> u64 {
        let elapsed = now_ms.saturating_sub(self.rate_window_start_ms);
        if self.rate_window_start_ms == 0 || elapsed < RATE_WINDOW_MS {
            self.upload_rate_bps
        } else {
            self.rate_window_bytes.saturating_mul(1000) / elapsed as u64
        }
    }
}

/// Resultado de la búsqueda de `OP_REASKFILEPING` en la cola de espera —
/// eMule `GetWaitingClientByIP_UDP` + comprobación `GetUploadFileID`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdpRankLookup {
    /// Cliente identificado, hash coincidente: responder `OP_REASKACK` + posición en cola.
    Rank(u32),
    /// Varios clientes en cola comparten esta IP sin coincidencia
    /// de puerto UDP — eMule no responde (fuerza la conexión TCP).
    Ambiguous,
    /// Cliente identificado pero el hash solicitado ≠ su archivo en espera —
    /// eMule registra el error y no responde.
    FileMismatch,
    /// Ningún cliente en cola con esta IP — eMule responde `OP_QUEUEFULL`
    /// únicamente si la cola se acerca a la saturación.
    NoSender,
}

/// Entrada en espera dentro de la cola de subida.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedUpload {
    /// Clave de identidad (`uh:<userhash>` o `ip:port`) usada para deduplicación/promoción.
    pub client_key: String,
    pub client_address: String,
    pub file_hash: String,
    pub file_name: String,
    pub enqueued_at_unix_ms: i64,
    pub credit_multiplier_bps: u32, // Multiplier * 100 (e.g. 100 for 1.0x, 1000 for 10.0x)
    /// `GetFilePrioAsNumber` de eMule para el archivo compartido solicitado:
    /// veryhigh=18, high=9, normal=7, low=6, verylow=2.
    #[serde(default = "default_file_prio_num")]
    pub file_prio_num: u32,
    /// Semántica de `IS_IDBADGUY`/`IsBanned` de eMule: la puntuación es siempre 0, la entrada
    /// espera indefinidamente al final (nunca es promocionada, sigue listada).
    #[serde(default)]
    pub banned: bool,
    /// Marca de tiempo del último re-ask del cliente (`SetLastUpRequest` de eMule) —
    /// sirve para la purga `MAX_PURGEQUEUETIME` (1 h).
    #[serde(default)]
    pub last_request_unix_ms: i64,
    /// Contador de «malas peticiones» de eMule (`Requested_File_Struct::badrequests`)
    /// : re-ask del mismo archivo antes de `MIN_REQUESTTIME` (10 min) → ++ ;
    /// al alcanzar `BAD_CLIENT_BAN` → `banned`.
    #[serde(default)]
    pub bad_requests: u8,
    /// Señal de promoción: el despachador que recibió `OP_QUEUERANKING`
    /// espera en ella para emitir `OP_ACCEPTUPLOADREQ` en cuanto se libera una ranura
    /// (eMule `AddUpNextClient` escribe directamente en el socket en espera
    /// — sin esto, el cliente solo sería atendido en su siguiente
    /// re-ask, ~30 min).
    #[serde(skip)]
    pub promote_notify: std::sync::Arc<tokio::sync::Notify>,
    /// Puerto UDP de eMule anunciado en el HELLO del cliente (`emule_udp_port`)
    /// — eMule `GetUDPPort()`: permite a `GetWaitingClientByIP_UDP` distinguir
    /// dos clientes tras la misma IP (NAT) durante un
    /// `OP_REASKFILEPING`.
    #[serde(default)]
    pub client_udp_port: Option<u16>,
    /// `true` cuando el archivo solicitado tiene `up_priority == "auto"` (eMule
    /// `m_bAutoUpPriority`) — `file_prio_num` se actualiza desde
    /// `auto_up_prio` en cada ordenación de la cola, igual que eMule lee
    /// `GetFilePrioAsNumber()` directamente en `GetScore`.
    #[serde(default)]
    pub auto_prio: bool,
    /// El cliente tiene una conexión TCP activa — eMule no destruye un
    /// cliente en cola cuando se cierra su socket (`bDelete =
    /// m_eUploadState != US_ONUPLOADQUEUE`, `BaseClient.cpp:1169`): su
    /// entrada sobrevive hasta `MAX_PURGEQUEUETIME`. Sin socket no se le puede
    /// promocionar (`GetScore(false)` lo excluye implícitamente — un
    /// `ACCEPTUPLOADREQ` no puede escribirse en ningún sitio), pero su puntuación
    /// sigue aumentando. Una entrada deserializada carece por definición
    /// de socket → `false`.
    #[serde(default)]
    pub connected: bool,
    /// eMule `m_bAddNextConnect` (`UploadQueue.cpp:151`): un cliente no
    /// conectado cuya puntuación supere a la del mejor cliente conectado queda
    /// marcado — en su reconexión/re-ask recibe una ranura **inmediatamente**
    /// (`AddClientToQueue:537` → `AddUpNextClient`), sin esperar una ranura
    /// libre (compensa el tiempo perdido sin socket, especialmente para clientes LowID
    /// cuya reconexión requiere callback del servidor).
    #[serde(default)]
    pub add_next_connect: bool,
}

fn default_file_prio_num() -> u32 {
    7
}

/// Mapea una cadena de prioridad de subida al peso `GetFilePrioAsNumber` de eMule
/// (`UploadClient.cpp`): veryhigh=18, high=9, normal=7, low=6, verylow=2.
/// `PR_POWER` (powershare) no tiene un caso dedicado en eMule → rama
/// `default` = 7, igual que `PR_NORMAL` (el powershare influye en la publicación,
/// no en la puntuación de cola).
/// `"auto"` (eMule `m_bAutoUpPriority`) recae en la prioridad efectiva
/// inicial `PR_HIGH` = 9 — el recálculo dinámico
/// (`UpdateAutoUpPriority`) se gestiona mediante `UploadManager::auto_up_prio`.
pub fn file_prio_num(priority: &str) -> u32 {
    match priority.to_lowercase().as_str() {
        "veryhigh" => 18,
        "high" => 9,
        "low" => 6,
        "verylow" => 2,
        "auto" => 9,
        _ => 7,
    }
}

impl QueuedUpload {
    /// Calcula la puntuación de espera de eMule:
    /// `wait_secs * credit_multiplier * (file_prio / 10)`, 0 si está bloqueado.
    /// Almacenado en centi-unidades (los campos `*_bps` conservan dos decimales).
    pub fn effective_score(&self, now_ms: i64) -> u64 {
        if self.banned {
            return 0;
        }
        let wait_secs = ((now_ms.saturating_sub(self.enqueued_at_unix_ms)) / 1000).max(1) as u64;
        let mult = (self.credit_multiplier_bps as u64).max(100);
        wait_secs * mult * self.file_prio_num as u64 / 1000
    }

    /// eMule `CUpDownClient::GetCombinedFilePrioAndCredit`:
    /// `10 * GetFilePrioAsNumber() * GetScoreRatio()` — utilizado para
    /// la admisión por encima del límite flexible (*soft limit*) de la cola.
    pub fn combined_prio_credit(&self) -> u64 {
        10 * self.file_prio_num as u64 * self.credit_multiplier_bps as u64 / 100
    }
}

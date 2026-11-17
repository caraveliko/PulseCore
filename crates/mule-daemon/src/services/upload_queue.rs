// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use crate::services::peer_session::log_addr_str;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

use crate::services::DaemonEvent;

/// Número máximo de peticiones de subida en cola.
/// Valor predeterminado de eMule 0.72a `QueueSize` (100 × QueueSizePref 50).
pub const MAX_UPLOAD_QUEUE: usize = 5000;
/// Número máximo de subidas activas simultáneas.
/// eMule 0.72a `MAX_UP_CLIENTS_ALLOWED` (Opcodes.h) — tope absoluto de
/// ranuras (slots) de subida, independientemente del resto de ajustes.
pub const MAX_ACTIVE_UPLOADS: usize = 100;
/// eMule `AcceptNewClient`: las `max(MIN_UP_CLIENTS_ALLOWED, 4)` = 4
/// primeras ranuras siempre se conceden — el suelo de tasa media
/// (`min_upload_rate`) no puede congelar la subida cuando hay clientes
/// esperando. A partir de ahí, cada nueva ranura exige que la media por ranura
/// se mantenga por encima del umbral mínimo.
const GUARANTEED_UPLOAD_SLOTS: usize = 4;

/// Umbral de goteo (trickle) de eMule: una ranura activa cuya tasa medida permanezca
/// por debajo de 1 KiB/s tras el periodo de gracia se degrada a ranura de goteo — mantiene
/// una cuota residual de ancho de banda pero deja de ocupar una ranura de subida completa
/// (`CUploadQueue` mantiene vivos a los clientes lentos liberando la ranura).
const TRICKLE_RATE_BPS: u64 = 1024;
/// Periodo de gracia antes de que una ranura lenta pueda ser degradada (60 s).
const TRICKLE_GRACE_MS: i64 = 60_000;

/// eMule `UPLOAD_CLIENT_MAXDATARATE` (`Opcodes.h`): tasa objetivo máxima
/// por cliente de subida — 25 KiB/s.
const UPLOAD_CLIENT_MAXDATARATE_BPS: u64 = 25 * 1024;
/// eMule `MAX_PURGEQUEUETIME` (`Opcodes.h`): una entrada de la cola cuya
/// última repetición de petición (re-ask) date de más de 1 h es purgada
/// (`CUploadQueue::FindBestClientInQueue`).
const MAX_PURGEQUEUE_MS: i64 = 60 * 60 * 1000;
/// eMule `SESSIONMAXTRANS` (`Opcodes.h`) = `PARTSIZE + 20*1024`: carga
/// útil máxima servida por sesión cuando `FullChunkTransfers` está activo
/// (predeterminado en eMule = `true`). Más allá de esto, el cliente es devuelto a la cola si no
/// se puede abrir una nueva ranura (`!ForceNewClient()`).
const SESSION_MAX_TRANS_BYTES: u64 = 9_728_000 + 20 * 1024;
/// eMule `MIN_REQUESTTIME` (`Opcodes.h`): un re-ask del mismo archivo antes
/// de 10 min cuenta como «mala petición»; 4 `BADCLIENTBAN` acumulados → bloqueo.
const MIN_REQUEST_TIME_MS: i64 = 10 * 60 * 1000;
const BAD_CLIENT_BAN: u8 = 4;
/// eMule `MIN_UP_CLIENTS_ALLOWED` (`Opcodes.h`): por debajo de este número de ranuras
/// activas, `ForceNewClient` devuelve verdadero sin ninguna otra condición — no se
/// aplica temporización alguna.
const MIN_UP_CLIENTS_ALLOWED: usize = 2;
/// eMule `ForceNewClient` (`UploadQueue.cpp:420`): entre dos aperturas
/// de ranura, 1 s mínimo mientras la velocidad medida permanezca por debajo de ~100 KB/s
/// (`::GetTickCount() < m_nLastStartUpload + SEC2MS(1) && datarate < 102400`).
/// Sin esta salvaguarda, una cola llena abriría todas las ranuras permitidas en una
/// ráfaga al inicio, antes de que la velocidad medida converja.
const PROMOTION_MIN_INTERVAL_MS: i64 = 1000;
/// Por encima de ~100 KB/s medidos (`datarate < 102400` invertido), las
/// promociones dejan de espaciarse.
const PROMOTION_UNPACED_BPS: u64 = 102_400;
/// eMule `AddClientToQueue` (`UploadQueue.cpp:586-596`): como máximo 3
/// clientes desde la misma IP en cola de espera — cola y ranuras activas
/// combinadas (la lista `TrackedClientsList` de eMule abarca ambos lados).
const MAX_SAME_IP_QUEUED: usize = 3;

/// Ventana de muestreo de la velocidad de subida (~2 s, cadencia de statsTick):
/// `upload_rate_bps` se recalcula en cada ventana vencida — misma estrategia
/// que las tasas de descarga medidas por tramos de 500 ms en
/// `peer_session/transfer.rs`.
const RATE_WINDOW_MS: i64 = 2_000;
/// Primer muestreo rápido (500 ms): una sesión que acaba de abrir su
/// ventana no espera 2 s antes de mostrar una velocidad real.
const RATE_FIRST_SAMPLE_MS: i64 = 500;

/// Sonda de velocidad de subida efectiva (`MaxSpeed` de eMule) — típicamente una
/// clausura que lee `upload_bucket.rate_per_sec()`.
type UploadSpeedProbe = Option<std::sync::Arc<dyn Fn() -> u64 + Send + Sync>>;

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

/// Cola de subida equitativa y acotada con seguimiento de ranuras activas.
/// Soporta hasta `MAX_ACTIVE_UPLOADS` ranuras de subida simultáneas
/// (eMule concede varias ranuras de subida concurrentes).
#[derive(Clone)]
pub struct UploadManager {
    queue: Arc<Mutex<VecDeque<QueuedUpload>>>,
    active_uploads: Arc<Mutex<std::collections::HashMap<String, UploadSessionSnapshot>>>,
    /// Señal hacia la conexión de una sesión **activa** rotada por
    /// `process_tick` (`CheckForTimeOver`) — el despachador emite entonces
    /// `OP_OUTOFPARTREQS` (0x57) para que el descargador vuelva a estar en cola
    /// en lugar de esperar bloques que no llegarán (eMule
    /// `SendOutOfPartReqsAndAddToWaitingQueue`, `UploadQueue.cpp:312`).
    rotation_notifies:
        Arc<Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Notify>>>>,
    active_count: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
    /// Compuerta `AcceptNewClient` de USS — cuando es false, los nuevos clientes permanecen en cola
    /// mientras la latencia supere la tolerancia (`UssService::accept_flag`).
    uss_gate: Arc<std::sync::RwLock<Arc<AtomicBool>>>,
    /// Canal opcional de eventos (establecido por `AppState`) para `upload.uploadsUpdated`.
    event_tx: Arc<std::sync::RwLock<Option<broadcast::Sender<String>>>>,
    sequence: Arc<std::sync::RwLock<Option<Arc<AtomicU64>>>>,
    /// Limitador de frecuencia para emisiones impulsadas por progreso (~500 ms).
    last_progress_emit_ms: Arc<AtomicI64>,
    /// Claves de clientes amigos (`uh:<userhash>`) — se les concede una ranura dedicada incluso
    /// cuando se alcanza `max_active` (ranura de amigo de eMule).
    friends: Arc<std::sync::RwLock<std::collections::HashSet<String>>>,
    /// Longitud máxima de la cola de espera (preferencia `QueueSize` de eMule, 500 por defecto).
    max_queue: Arc<AtomicUsize>,
    /// Ancho de banda de subida mínimo por ranura en bytes/s (eMule `MinUpload`):
    /// solo se abre una nueva ranura si las ranuras existentes continúan alcanzándolo.
    min_upload_rate_bps: Arc<std::sync::atomic::AtomicU64>,
    /// Sonda de velocidad de subida configurada/efectiva (`MaxSpeed` de eMule) — lee
    /// `upload_bucket.rate_per_sec()` directamente: refleja tanto el límite manual
    /// (`rate_limit_upload_bps`) como el valor dinámico guiado por USS
    /// (`IsDynUpEnabled → lastCommonRouteFinder->GetUpload()`). 0 = ilimitado.
    upload_speed_probe: Arc<std::sync::RwLock<UploadSpeedProbe>>,
    /// Cuota diaria de subida (eMuleRust `dailyUploadQuotaGb`) — cuando es
    /// `false`, no se acepta ninguna nueva petición de ranura.
    quota_gate: Arc<std::sync::RwLock<Arc<AtomicBool>>>,
    /// Total acumulado de bytes subidos desde el inicio del daemon
    /// (contador de origen para la cuota diaria).
    total_uploaded_bytes: Arc<AtomicU64>,
    /// Totales subidos por archivo (hash hex en minúsculas) — cargados
    /// desde `share_upload_stats` al arrancar y luego incrementados con cada
    /// bloque enviado; persistidos mediante deltas en el bucle de `statsTick`.
    file_upload_totals: Arc<std::sync::RwLock<std::collections::HashMap<String, u64>>>,
    /// Inicio de espera preservado para un cliente desalojado de su ranura por
    /// rotación de sesión (`CheckForTimeOver`) — aplicado como
    /// `enqueued_at` si vuelve a solicitarla (eMule conserva el inicio de espera en
    /// `credits`; la puntuación acumulada no se pierde).
    rotation_grace: Arc<Mutex<std::collections::HashMap<String, i64>>>,
    /// Instantánea de los hashes actualmente compartidos (en minúsculas) — actualizada
    /// por el bucle de tick mediante [`Self::set_shared_hashes`]. eMule purga
    /// de la cola a los clientes cuyo archivo solicitado ya no esté compartido
    /// (`FindBestClientInQueue` → `!sharedfiles->GetFileByID`). `None` =
    /// sonda no conectada → la purga por compartición está inactiva (pruebas).
    shared_hashes: Arc<std::sync::RwLock<Option<std::collections::HashSet<String>>>>,
    /// Prioridad efectiva recalculada de los archivos con `up_priority = "auto"`
    /// (hash en minúsculas → `GetFilePrioAsNumber`) — registro de
    /// archivos auto *y* caché del último `UpdateAutoUpPriority`.
    /// eMule `CKnownFile::UpdateAutoUpPriority`: más de 20 subidores
    /// → baja (6), más de 1 → normal (7), en otro caso → alta (9).
    auto_up_prio: Arc<std::sync::RwLock<std::collections::HashMap<String, u32>>>,
    /// Marca de tiempo de la última apertura de ranura (eMule
    /// `m_nLastStartUpload`, establecido en `AddUpNextClient`) — alimenta
    /// el intervalo de temporización `PROMOTION_MIN_INTERVAL_MS` de `promotion_paced`.
    last_upload_start_ms: Arc<AtomicI64>,
    /// IPs cuyas `bad_requests` acaban de alcanzar `BAD_CLIENT_BAN` —
    /// eMule `Ban()` → `AddBannedClient` bloquea la IP durante 2 h en vez de
    /// marcar únicamente la entrada. El despachador las vacía tras
    /// `request_upload_slot` y alimenta `DownloadService::ban_source`.
    pending_bans: Arc<Mutex<Vec<String>>>,
}

impl Default for UploadManager {
    fn default() -> Self {
        Self::new()
    }
}

impl UploadManager {
    /// Crea un nuevo `UploadManager`.
    pub fn new() -> Self {
        Self {
            queue: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_UPLOAD_QUEUE))),
            active_uploads: Arc::new(Mutex::new(std::collections::HashMap::new())),
            rotation_notifies: Arc::new(Mutex::new(std::collections::HashMap::new())),
            active_count: Arc::new(AtomicUsize::new(0)),
            max_active: Arc::new(AtomicUsize::new(MAX_ACTIVE_UPLOADS)),
            uss_gate: Arc::new(std::sync::RwLock::new(Arc::new(AtomicBool::new(true)))),
            event_tx: Arc::new(std::sync::RwLock::new(None)),
            sequence: Arc::new(std::sync::RwLock::new(None)),
            last_progress_emit_ms: Arc::new(AtomicI64::new(0)),
            friends: Arc::new(std::sync::RwLock::new(std::collections::HashSet::new())),
            max_queue: Arc::new(AtomicUsize::new(MAX_UPLOAD_QUEUE)),
            min_upload_rate_bps: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            upload_speed_probe: Arc::new(std::sync::RwLock::new(None)),
            quota_gate: Arc::new(std::sync::RwLock::new(Arc::new(AtomicBool::new(true)))),
            total_uploaded_bytes: Arc::new(AtomicU64::new(0)),
            file_upload_totals: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            last_upload_start_ms: Arc::new(AtomicI64::new(0)),
            pending_bans: Arc::new(Mutex::new(Vec::new())),
            rotation_grace: Arc::new(Mutex::new(std::collections::HashMap::new())),
            shared_hashes: Arc::new(std::sync::RwLock::new(None)),
            auto_up_prio: Arc::new(std::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
        }
    }

    /// Sustituye la instantánea de los hashes compartidos — invocado periódicamente por
    /// el bucle de tick (eMule lee `sharedfiles` directamente en
    /// `FindBestClientInQueue`). Pasar `Some(set)` activa la purga de las
    /// entradas de la cola cuyo archivo ya no esté compartido.
    pub fn set_shared_hashes(&self, hashes: std::collections::HashSet<String>) {
        *self.shared_hashes.write().unwrap() = Some(hashes);
    }

    /// `true` cuando el hash no está en la instantánea de compartidos —
    /// `true` también si la sonda no está conectada (sin purga en dicho
    /// caso, siendo `sharedfiles` de eMule la fuente de verdad).
    fn hash_still_shared(&self, file_hash: &str) -> bool {
        self.shared_hashes
            .read()
            .unwrap()
            .as_ref()
            .is_none_or(|set| set.contains(file_hash))
    }

    /// Carga los totales subidos por archivo persistidos en SQLite
    /// (`share_upload_stats`) — llamado una vez al arrancar.
    pub fn set_file_upload_totals(&self, totals: std::collections::HashMap<String, u64>) {
        *self.file_upload_totals.write().unwrap() = totals;
    }

    /// Copia de los totales subidos por archivo (hash en minúsculas).
    pub fn per_file_uploaded_bytes(&self) -> std::collections::HashMap<String, u64> {
        self.file_upload_totals.read().unwrap().clone()
    }

    /// Velocidad efectiva actual acumulada por archivo (suma de ranuras activas
    /// sobre el mismo hash, claves en minúsculas) — eMule muestra esta velocidad en
    /// la columna «Velocidad de subida» de la lista de archivos compartidos.
    pub async fn per_file_rates_bps(&self) -> std::collections::HashMap<String, u64> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let active = self.active_uploads.lock().await;
        let mut rates = std::collections::HashMap::new();
        for s in active.values() {
            *rates.entry(s.file_hash.to_lowercase()).or_insert(0u64) += s.effective_rate_bps(now);
        }
        rates
    }

    /// Establece la velocidad de subida mínima por ranura (eMule `MinUpload`, B/s).
    /// `0` desactiva el umbral mínimo.
    pub fn set_min_upload_rate(&self, bps: u64) {
        self.min_upload_rate_bps
            .store(bps, std::sync::atomic::Ordering::Relaxed);
    }

    /// Instala la sonda de velocidad de subida efectiva (`MaxSpeed` de eMule) — invocada
    /// por `AppState` mediante una clausura que lee
    /// `upload_bucket.rate_per_sec()`.
    pub fn set_upload_speed_probe(&self, probe: impl Fn() -> u64 + Send + Sync + 'static) {
        *self.upload_speed_probe.write().unwrap() = Some(std::sync::Arc::new(probe));
    }

    /// Velocidad de subida configurada/efectiva en bytes/s (0 = ilimitado o sonda
    /// ausente) — eMule `MaxSpeed` en `AcceptNewClient`.
    fn max_upload_speed_bps(&self) -> u64 {
        self.upload_speed_probe
            .read()
            .unwrap()
            .as_ref()
            .map(|p| p())
            .unwrap_or(0)
    }

    /// Establece la longitud máxima de la cola de espera (preferencia `QueueSize` de eMule).
    pub fn set_max_queue(&self, max: usize) {
        self.max_queue
            .store(max.clamp(10, 5000), std::sync::atomic::Ordering::Relaxed);
    }

    /// Instala el conjunto de claves de clientes amigos (entradas `uh:<userhash>`).
    pub fn set_friends(&self, friends: std::collections::HashSet<String>) {
        *self.friends.write().unwrap() = friends;
    }

    /// Añade o elimina una clave de cliente amigo en tiempo de ejecución (mediante RPC).
    pub fn set_friend(&self, client_key: &str, is_friend: bool) {
        let mut f = self.friends.write().unwrap();
        if is_friend {
            f.insert(client_key.to_string());
        } else {
            f.remove(client_key);
        }
    }

    /// Devuelve verdadero si `client_key` corresponde a un amigo configurado.
    fn is_friend(&self, client_key: &str) -> bool {
        self.friends.read().unwrap().contains(client_key)
    }

    /// IPs recién caídas bajo `BAD_CLIENT_BAN` — el despachador las
    /// traduce a `DownloadService::ban_source` (bloqueo de IP 2 h, eMule
    /// `AddBannedClient`) y cierra la conexión del par.
    pub async fn drain_pending_bans(&self) -> Vec<String> {
        std::mem::take(&mut *self.pending_bans.lock().await)
    }

    /// Instala el canal de difusión utilizado para emitir `upload.uploadsUpdated`.
    pub fn set_event_channel(&self, tx: broadcast::Sender<String>, seq: Arc<AtomicU64>) {
        *self.event_tx.write().unwrap() = Some(tx);
        *self.sequence.write().unwrap() = Some(seq);
    }

    /// Emite la instantánea completa de subidas (ranuras activas + cola + tasa
    /// agregada). No realiza ninguna operación si no hay canal de eventos instalado. `pub` para la
    /// emisión periódica desde el bucle `statsTick` (`state.rs`) que
    /// refresca las velocidades medidas entre envíos de bloques.
    pub async fn emit_uploads_updated(&self) {
        let (tx, seq) = {
            let tx = self.event_tx.read().unwrap().clone();
            let seq = self.sequence.read().unwrap().clone();
            (tx, seq)
        };
        let (Some(tx), Some(seq)) = (tx, seq) else {
            return;
        };
        let active = self.active_snapshots().await;
        let queue = self.list_queue().await;
        let rate = active.iter().map(|s| s.upload_rate_bps).sum();
        DaemonEvent::UploadsUpdated {
            sequence: seq.fetch_add(1, Ordering::SeqCst),
            active,
            queue,
            upload_rate_bps: rate,
        }
        .send(&tx);
    }

    /// Emite `uploadsUpdated` desde la ruta de progreso activo, limitado a
    /// como máximo una vez cada 500 ms.
    async fn emit_uploads_updated_throttled(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let last = self.last_progress_emit_ms.load(Ordering::SeqCst);
        if now - last < 500 {
            return;
        }
        if self
            .last_progress_emit_ms
            .compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.emit_uploads_updated().await;
        }
    }

    /// Instala la bandera de aceptación de nuevos clientes de USS
    /// (semántica `LastCommonRouteFinder::AcceptNewClient`).
    pub fn set_uss_gate(&self, flag: Arc<AtomicBool>) {
        *self.uss_gate.write().unwrap() = flag;
    }

    /// Instala la compuerta de cuota diaria (`dailyUploadQuotaGb`) — `false`
    /// bloquea cualquier nueva petición de ranura.
    pub fn set_quota_gate(&self, flag: Arc<AtomicBool>) {
        *self.quota_gate.write().unwrap() = flag;
    }

    /// Bytes subidos acumulados desde el inicio del daemon.
    pub fn total_uploaded_bytes(&self) -> u64 {
        self.total_uploaded_bytes.load(Ordering::SeqCst)
    }

    /// Sobrescribe el número máximo de ranuras de subida activas simultáneas.
    pub fn set_max_active(&self, max: usize) {
        self.max_active.store(max.max(1), Ordering::SeqCst);
    }

    /// Número de ranuras de subida activas actualmente.
    pub fn active_len(&self) -> usize {
        self.active_count.load(Ordering::SeqCst)
    }

    /// Solicita una ranura de subida para un cliente entrante.
    ///
    /// - Si hay una ranura disponible, concede de inmediato `UploadSlotState::Uploading`.
    /// - Si todas las ranuras están ocupadas, encola al cliente en la
    ///   cola ponderada por créditos y devuelve `UploadSlotState::Waiting` + posición.
    /// - Devuelve un error si se supera la capacidad máxima de la cola.
    ///
    /// `client_key` es la identidad usada para deduplicación/liberación — `uh:<userhash>`
    /// si el par anunció un hash de usuario; en caso contrario, `client_address`.
    /// `client_address` se mantiene como la dirección de visualización/búsqueda UDP (`ip:port`).
    pub async fn request_upload_slot(
        &self,
        client_key: &str,
        client_address: &str,
        file_hash: &str,
        file_name: &str,
        credit_multiplier: f64,
        file_prio: &str,
        banned: bool,
        client_udp_port: Option<u16>,
    ) -> Result<(UploadSlotState, Option<u32>), String> {
        let mut active = self.active_uploads.lock().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let auto_prio = file_prio.eq_ignore_ascii_case("auto");

        // El cliente ya tiene una ranura — eMule `IsDownloading()`: un cliente
        // que ya está descargando y pide otro archivo conserva su ranura
        // (`OP_ACCEPTUPLOADREQ` + `SetUploadFileID`), nunca vuelve
        // a la cola.
        if active.contains_key(client_key) {
            let mut switched_from = None;
            if let Some(cur) = active.get_mut(client_key) {
                if cur.file_hash != file_hash {
                    switched_from = Some(cur.file_hash.clone());
                    cur.file_hash = file_hash.to_string();
                    cur.file_name = file_name.to_string();
                    cur.auto_prio = auto_prio;
                }
            }
            if let Some(old_hash) = switched_from {
                let new_num = self.resolve_file_prio_num(&active, file_hash, file_prio);
                if let Some(cur) = active.get_mut(client_key) {
                    cur.file_prio_num = new_num;
                }
                // El archivo anterior pierde un subidor y el nuevo
                // gana uno — `UpdateAutoUpPriority` en ambos lados.
                self.update_auto_up_prio(&active, &old_hash);
                self.update_auto_up_prio(&active, file_hash);
            }
            return Ok((UploadSlotState::Uploading, None));
        }

        // Cuota diaria alcanzada (`dailyUploadQuotaGb`): no se acepta
        // ninguna nueva petición hasta el restablecimiento del contador.
        if !self.quota_gate.read().unwrap().load(Ordering::SeqCst) {
            return Err("Daily upload quota reached".to_string());
        }

        // Concede una ranura de inmediato mientras haya capacidad y USS acepte
        // nuevos clientes (acceptNewClient=false si la latencia es excesiva).
        // Las ranuras de goteo (trickle) mantienen ancho de banda residual pero no ocupan una ranura completa.
        // eMule `AddClientToQueue` → `ForceNewClient(true)`: la salvaguarda
        // de espaciado de 1 s también se aplica a la adición directa.
        if self.can_open_slot(&active, now) && !self.promotion_paced(&active, now) {
            let snap = UploadSessionSnapshot {
                client_key: client_key.to_string(),
                client_address: client_address.to_string(),
                file_hash: file_hash.to_string(),
                file_name: file_name.to_string(),
                transferred_bytes: 0,
                upload_rate_bps: 0,
                state: UploadSlotState::Uploading,
                queue_rank: None,
                started_at_unix_ms: now,
                rate_window_start_ms: 0,
                rate_window_bytes: 0,
                rate_sampled: false,
                credit_multiplier_bps: (credit_multiplier.clamp(1.0, 10.0) * 100.0) as u32,
                file_prio_num: self.resolve_file_prio_num(&active, file_hash, file_prio),
                queued_since_ms: now,
                auto_prio,
            };
            active.insert(client_key.to_string(), snap);
            // `AddUpNextClient` → `m_nLastStartUpload = GetTickCount()`.
            self.last_upload_start_ms.store(now, Ordering::Relaxed);
            // `AddUploadingClient` → `UpdateAutoUpPriority`: el número
            // de subidores del archivo acaba de aumentar.
            self.update_auto_up_prio(&active, file_hash);
            self.active_count
                .store(Self::uploading_count(&active), Ordering::SeqCst);
            tracing::info!(
                "[UPLOAD_SLOT_GRANTED] Client {} granted active upload slot for file '{}' (hash: {}) ({}/{} slots used)",
                log_addr_str(client_address), file_name, file_hash, Self::uploading_count(&active), self.max_active.load(Ordering::SeqCst)
            );
            drop(active);
            self.emit_uploads_updated().await;
            return Ok((UploadSlotState::Uploading, None));
        }

        // Ranura de amigo de eMule: un amigo se salta la cola y recibe
        // una ranura adicional dedicada por encima de `max_active`.
        if self.is_friend(client_key) {
            let snap = UploadSessionSnapshot {
                client_key: client_key.to_string(),
                client_address: client_address.to_string(),
                file_hash: file_hash.to_string(),
                file_name: file_name.to_string(),
                transferred_bytes: 0,
                upload_rate_bps: 0,
                state: UploadSlotState::Uploading,
                queue_rank: None,
                started_at_unix_ms: now,
                rate_window_start_ms: 0,
                rate_window_bytes: 0,
                rate_sampled: false,
                credit_multiplier_bps: (credit_multiplier.clamp(1.0, 10.0) * 100.0) as u32,
                file_prio_num: self.resolve_file_prio_num(&active, file_hash, file_prio),
                queued_since_ms: now,
                auto_prio,
            };
            active.insert(client_key.to_string(), snap);
            self.update_auto_up_prio(&active, file_hash);
            tracing::info!(
                "[UPLOAD_FRIEND_SLOT] Friend {} granted dedicated upload slot for file '{}' ({} full slots)",
                log_addr_str(client_address), file_name, Self::uploading_count(&active)
            );
            drop(active);
            self.emit_uploads_updated().await;
            return Ok((UploadSlotState::Uploading, None));
        }

        // Un cliente desalojado de su ranura por rotación de sesión
        // recupera su inicio de espera original (eMule `credits` lo conserva):
        // su puntuación acumulada no se pierde — regresa cerca de la cabecera.
        let rotation_grace = self.rotation_grace.lock().await.remove(client_key);

        // Todas las ranuras activas ocupadas: encolar si hay espacio disponible
        let mut q = self.queue.lock().await;
        let mult_bps = (credit_multiplier.clamp(1.0, 10.0) * 100.0) as u32;

        // Prioridad cambiada a manual: el archivo sale del registro automático
        // y las entradas que lo solicitan retoman el peso explícito.
        if !auto_prio {
            self.clear_auto_file(&mut q, file_hash, file_prio_num(file_prio));
        }

        // eMule: un único objeto cliente = una única entrada en cola, sin importar
        // el archivo pedido. El re-ask actualiza `last_request`,
        // actualiza el archivo solicitado y devuelve la posición actual —
        // y nutre el contador anti-abuso (`MIN_REQUESTTIME`/`BADCLIENTBAN`).
        if let Some(pos) = q.iter().position(|entry| entry.client_key == client_key) {
            let entry = &mut q[pos];
            // Reconexión de un cliente cuya entrada sobrevivió al cierre del
            // socket (`mark_disconnected`). `AddClientToQueue` de eMule: si
            // `m_bAddNextConnect` estaba fijado (puntuación ≥ mejor conectado
            // mientras estuvo inalcanzable), recibe una ranura inmediata.
            entry.connected = true;
            if entry.add_next_connect {
                entry.add_next_connect = false;
                let next = q.remove(pos).expect("queue entry at pos");
                let snap = UploadSessionSnapshot {
                    client_key: next.client_key.clone(),
                    client_address: next.client_address.clone(),
                    file_hash: next.file_hash.clone(),
                    file_name: next.file_name.clone(),
                    transferred_bytes: 0,
                    upload_rate_bps: 0,
                    state: UploadSlotState::Uploading,
                    queue_rank: None,
                    started_at_unix_ms: now,
                    rate_window_start_ms: 0,
                    rate_window_bytes: 0,
                    rate_sampled: false,
                    credit_multiplier_bps: next.credit_multiplier_bps,
                    file_prio_num: self.resolve_file_prio_num(&active, file_hash, file_prio),
                    queued_since_ms: next.enqueued_at_unix_ms,
                    auto_prio: next.auto_prio,
                };
                active.insert(next.client_key.clone(), snap);
                self.update_auto_up_prio(&active, file_hash);
                self.active_count
                    .store(Self::uploading_count(&active), Ordering::SeqCst);
                tracing::info!(
                    "[UPLOAD_ADD_NEXT_CONNECT] Client {} promoted on reconnect for file '{}' — outscored connected peers while unreachable (eMule m_bAddNextConnect)",
                    log_addr_str(client_address), file_name
                );
                drop(q);
                drop(active);
                self.emit_uploads_updated().await;
                return Ok((UploadSlotState::Uploading, None));
            }
            // `!GetFriendSlot()` en `AddRequestCount` de eMule: los amigos
            // están exentos del contador anti-flood.
            if !self.is_friend(client_key)
                && entry.file_hash == file_hash
                && now.saturating_sub(entry.last_request_unix_ms) < MIN_REQUEST_TIME_MS
            {
                entry.bad_requests = entry.bad_requests.saturating_add(1);
                if entry.bad_requests >= BAD_CLIENT_BAN && !entry.banned {
                    entry.banned = true;
                    // eMule `Ban()` → `AddBannedClient(IP)`: el par queda
                    // bloqueado 2 h a nivel de IP, no solo desprovisto de puntuación.
                    let ip = client_address
                        .parse::<std::net::SocketAddr>()
                        .map(|a| a.ip().to_string())
                        .unwrap_or_else(|_| client_address.to_string());
                    self.pending_bans.lock().await.push(ip);
                }
            } else {
                entry.bad_requests = entry.bad_requests.saturating_sub(1);
            }
            entry.last_request_unix_ms = now;
            entry.file_hash = file_hash.to_string();
            entry.file_name = file_name.to_string();
            entry.file_prio_num = self.resolve_file_prio_num(&active, file_hash, file_prio);
            entry.auto_prio = auto_prio;
            entry.credit_multiplier_bps = mult_bps;
            entry.banned |= banned;
            // La puntuación evoluciona con el tiempo — recalcular el orden antes de
            // devolver la posición (eMule evalúa `GetScore` bajo demanda).
            self.sort_queue_by_score(&mut q, now);
            let rank = q
                .iter()
                .position(|e| e.client_key == client_key)
                .map(|p| (p + 1) as u32)
                .unwrap_or(q.len() as u32);
            drop(q);
            drop(active);
            self.emit_uploads_updated_throttled().await;
            return Ok((UploadSlotState::Waiting, Some(rank)));
        }

        // eMule `AddClientToQueue` — el límite de cola es *flexible*:
        // `hard = soft + max(soft, 800)/4`. Entre el límite suave y el estricto, solo los
        // clientes con ranura de amigo o cuyo `combined(prio × créditos)` supere la
        // media de la cola son admitidos.
        let soft_limit = self.max_queue.load(std::sync::atomic::Ordering::Relaxed);
        let hard_limit = soft_limit + soft_limit.max(800) / 4;
        if q.len() >= hard_limit {
            return Err(format!("Upload queue full ({hard_limit} max)"));
        }
        if q.len() >= soft_limit && !self.is_friend(client_key) {
            let avg: u64 = q.iter().map(|e| e.combined_prio_credit()).sum::<u64>()
                / q.len().max(1) as u64;
            let mine = 10
                * self.resolve_file_prio_num(&active, file_hash, file_prio) as u64
                * mult_bps as u64
                / 100;
            if mine < avg {
                return Err("Upload queue soft limit reached".to_string());
            }
        }

        // eMule `AddClientToQueue`: rechaza la entrada si ya hay 3 clientes de
        // la misma IP en cola o activos (`cSameIP >= 3` +
        // `GetClientsFromIP >= 3`). La IP se extrae de `client_address`
        // mediante `rsplit_once(':')` para admitir direcciones IPv6 entre corchetes.
        let req_ip = client_address.rsplit_once(':').map(|(h, _)| h).unwrap_or("");
        if !req_ip.is_empty() {
            let same_ip = q
                .iter()
                .filter(|e| e.client_address.rsplit_once(':').is_some_and(|(h, _)| h == req_ip))
                .count()
                + active
                    .values()
                    .filter(|s| s.client_address.rsplit_once(':').is_some_and(|(h, _)| h == req_ip))
                    .count();
            if same_ip >= MAX_SAME_IP_QUEUED {
                return Err("Too many clients from the same IP".to_string());
            }
        }

        let enqueued_at = rotation_grace.unwrap_or(now);

        q.push_back(QueuedUpload {
            client_key: client_key.to_string(),
            client_address: client_address.to_string(),
            file_hash: file_hash.to_string(),
            file_name: file_name.to_string(),
            enqueued_at_unix_ms: enqueued_at,
            credit_multiplier_bps: mult_bps,
            file_prio_num: self.resolve_file_prio_num(&active, file_hash, file_prio),
            banned,
            last_request_unix_ms: now,
            bad_requests: 0,
            promote_notify: std::sync::Arc::new(tokio::sync::Notify::new()),
            client_udp_port,
            auto_prio,
            connected: true,
            add_next_connect: false,
        });

        // Ordena la cola por puntuación efectiva descendente (eMule califica a los clientes
        // según `GetScore` en vivo, no por orden de llegada).
        self.sort_queue_by_score(&mut q, now);

        let rank = q
            .iter()
            .position(|entry| entry.client_key == client_key)
            .map(|pos| (pos + 1) as u32)
            .unwrap_or(q.len() as u32);

        tracing::info!(
            "[UPLOAD_QUEUED] Client {} queued for file '{}' (hash: {}) with rank #{} (credits mult: {:.2}x, queue size: {})",
            log_addr_str(client_address), file_name, file_hash, rank, credit_multiplier, q.len()
        );
        drop(q);
        drop(active);
        self.emit_uploads_updated().await;

        Ok((UploadSlotState::Waiting, Some(rank)))
    }

    /// Número de ranuras que ocupan actualmente una ranura de subida completa — las ranuras
    /// de goteo (trickle) se excluyen (eMule solo cuenta subidas a velocidad normal).
    fn uploading_count(active: &std::collections::HashMap<String, UploadSessionSnapshot>) -> usize {
        active
            .values()
            .filter(|s| s.state == UploadSlotState::Uploading)
            .count()
    }

    /// eMule `CKnownFile::UpdateAutoUpPriority`: prioridad efectiva de un
    /// archivo «auto» según la cantidad de clientes que lo suben
    /// (`GetQueuedCount` — sesiones Uploading + Trickle): más de 20
    /// → baja (6), más de 1 → normal (7), en otro caso → alta (9).
    fn compute_auto_up_prio(
        active: &std::collections::HashMap<String, UploadSessionSnapshot>,
        lower_hash: &str,
    ) -> u32 {
        let count = active
            .values()
            .filter(|s| {
                s.file_hash.eq_ignore_ascii_case(lower_hash)
                    && matches!(s.state, UploadSlotState::Uploading | UploadSlotState::Trickle)
            })
            .count();
        if count > 20 {
            6
        } else if count > 1 {
            7
        } else {
            9
        }
    }

    /// Prioridad efectiva de un archivo «auto»: la primera petición
    /// registra el archivo en `auto_up_prio` con el valor calculado
    /// sobre el estado actual (eMule comienza en `PR_HIGH` y recalcula con
    /// cada adición o retirada de subidor).
    fn auto_up_prio_for(
        &self,
        active: &std::collections::HashMap<String, UploadSessionSnapshot>,
        file_hash: &str,
    ) -> u32 {
        let key = file_hash.to_lowercase();
        if let Some(&v) = self.auto_up_prio.read().unwrap().get(&key) {
            return v;
        }
        let v = Self::compute_auto_up_prio(active, &key);
        self.auto_up_prio.write().unwrap().insert(key, v);
        v
    }

    /// Recalcula la prioridad efectiva de un archivo auto tras un cambio
    /// en las sesiones activas — no hace nada si el archivo no está en modo auto
    /// (ausente del registro `auto_up_prio`).
    fn update_auto_up_prio(
        &self,
        active: &std::collections::HashMap<String, UploadSessionSnapshot>,
        file_hash: &str,
    ) {
        let key = file_hash.to_lowercase();
        if !self.auto_up_prio.read().unwrap().contains_key(&key) {
            return;
        }
        let v = Self::compute_auto_up_prio(active, &key);
        self.auto_up_prio.write().unwrap().insert(key, v);
    }

    /// El archivo vuelve a prioridad manual: sale del registro automático y
    /// reescribe `file_prio_num`/`auto_prio` en las entradas de cola correspondientes
    /// (la prioridad del archivo es una propiedad del archivo compartido, común
    /// a todos los clientes que lo solicitan).
    fn clear_auto_file(&self, q: &mut VecDeque<QueuedUpload>, file_hash: &str, manual_num: u32) {
        let key = file_hash.to_lowercase();
        if self.auto_up_prio.write().unwrap().remove(&key).is_none()
            && !q.iter().any(|e| e.auto_prio && e.file_hash.eq_ignore_ascii_case(file_hash))
        {
            return;
        }
        for e in q.iter_mut() {
            if e.auto_prio && e.file_hash.eq_ignore_ascii_case(file_hash) {
                e.auto_prio = false;
                e.file_prio_num = manual_num;
            }
        }
    }

    /// `GetFilePrioAsNumber` efectivo: `"auto"` lee el valor recalculado
    /// (`auto_up_prio`), mientras que el resto de prioridades utilizan el mapeo
    /// estático de eMule.
    fn resolve_file_prio_num(
        &self,
        active: &std::collections::HashMap<String, UploadSessionSnapshot>,
        file_hash: &str,
        prio_str: &str,
    ) -> u32 {
        if prio_str.eq_ignore_ascii_case("auto") {
            self.auto_up_prio_for(active, file_hash)
        } else {
            file_prio_num(prio_str)
        }
    }

    /// Ordena la cola por `effective_score` decreciente — eMule compara las
    /// puntuaciones **bajo demanda** (`GetQueueRank`, `FindBestClientInQueue`);
    /// la ordenación reproduce el mismo orden para `pop_front`/posición mostrada.
    /// Las entradas «auto» actualizan primero `file_prio_num` desde
    /// `auto_up_prio`, al igual que eMule lee `GetFilePrioAsNumber()` directamente.
    fn sort_queue_by_score(&self, q: &mut VecDeque<QueuedUpload>, now_ms: i64) {
        {
            let auto_map = self.auto_up_prio.read().unwrap();
            for e in q.iter_mut() {
                if e.auto_prio {
                    if let Some(&v) = auto_map.get(&e.file_hash.to_lowercase()) {
                        e.file_prio_num = v;
                    }
                }
            }
        }
        let mut entries: Vec<QueuedUpload> = q.drain(..).collect();
        entries.sort_by_key(|a| std::cmp::Reverse(a.effective_score(now_ms)));
        q.extend(entries);
    }

    /// eMule `CUploadQueue::GetTargetClientDataRate`: ≤3 ranuras → 3 KiB/s,
    /// en otro caso `slots × 1 KiB/s` con tope en `UPLOAD_CLIENT_MAXDATARATE`
    /// (25 KiB/s). La variante `min_datarate` devuelve ×3/4.
    fn target_client_data_rate_bps(open_slots: usize, min_datarate: bool) -> u64 {
        let r = if open_slots <= 3 {
            3 * 1024
        } else {
            (open_slots as u64 * 1024).min(UPLOAD_CLIENT_MAXDATARATE_BPS)
        };
        if min_datarate {
            r * 3 / 4
        } else {
            r
        }
    }

    /// eMule `CUploadQueue::AcceptNewClient`: abre una ranura si
    /// - no hay veto de USS (`lastCommonRouteFinder->AcceptNewClient`),
    /// - `uploading < max(MIN_UP_CLIENTS_ALLOWED, 4)` → siempre,
    /// - `uploading >= MAX_UP_CLIENTS_ALLOWED` → nunca,
    /// - `uploading < datarate / minTargetRate`: la velocidad medida aún
    ///   abastece a cada cliente en el umbral mínimo,
    /// - `uploading < MaxSpeed / targetRate`: la capacidad configurada
    ///   (o medida por USS) admite una ranura adicional a la tasa objetivo.
    ///
    /// Compartido por `request_upload` (nuevo cliente), `promote_waiting`
    /// (promoción) y `process_tick` (rotación) para evitar divergencias.
    fn can_open_slot(
        &self,
        active: &std::collections::HashMap<String, UploadSessionSnapshot>,
        now: i64,
    ) -> bool {
        if !self.uss_gate.read().unwrap().load(Ordering::SeqCst) {
            return false;
        }
        let uploading = Self::uploading_count(active);
        // `MAX_UP_CLIENTS_ALLOWED` primero: eMule prueba lo garantizado antes
        // del tope, pero el tope (100) siempre supera lo garantizado
        // (4) — aquí `max_active` es configurable y puede descender de 4.
        if uploading >= self.max_active.load(Ordering::SeqCst) {
            return false;
        }
        if uploading < GUARANTEED_UPLOAD_SLOTS {
            return true;
        }
        let target = Self::target_client_data_rate_bps(uploading, false).max(1);
        let min_target = Self::target_client_data_rate_bps(uploading, true).max(1);
        let total_bps: u64 = active
            .values()
            .filter(|s| s.state == UploadSlotState::Uploading)
            .map(|s| s.effective_rate_bps(now))
            .sum();
        // `curUploadSlots >= datarate / minTarget` → rechazo: cada cliente
        // existente debe mantener al menos la velocidad mínima.
        if uploading as u64 >= total_bps / min_target {
            return false;
        }
        // `curUploadSlots >= MaxSpeed*1024 / target` → rechazo: la capacidad
        // de subida configurada no permite una ranura adicional a la velocidad
        // objetivo. 0 = ilimitado → únicamente el término datarate limita el crecimiento
        // (eMule añade un límite interno «graph rate» no modelado aquí).
        let max_speed = self.max_upload_speed_bps();
        if max_speed > 0 && uploading as u64 >= max_speed / target {
            return false;
        }
        // Umbral mínimo opcional `min_upload_kbs` (eMule `MinUpload` alimenta USS,
        // no `AcceptNewClient` — mantenido como salvaguarda adicional de usuario,
        // inactivo de forma predeterminada).
        let min_rate = self.min_upload_rate_bps.load(Ordering::Relaxed);
        !(min_rate > 0 && total_bps / (uploading.max(1) as u64) < min_rate)
    }

    /// Temporización de `ForceNewClient` de eMule (`UploadQueue.cpp:417-421`): `true`
    /// cuando la apertura de una ranura adicional debe esperar — menos de
    /// `PROMOTION_MIN_INTERVAL_MS` desde la última apertura y velocidad
    /// medida < `PROMOTION_UNPACED_BPS`. Por debajo de `MIN_UP_CLIENTS_ALLOWED`
    /// ranuras activas, nunca hay temporización (`return true` anticipado).
    fn promotion_paced(
        &self,
        active: &std::collections::HashMap<String, UploadSessionSnapshot>,
        now: i64,
    ) -> bool {
        if Self::uploading_count(active) < MIN_UP_CLIENTS_ALLOWED {
            return false;
        }
        let last = self.last_upload_start_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= PROMOTION_MIN_INTERVAL_MS {
            return false;
        }
        let total_bps: u64 = active
            .values()
            .filter(|s| s.state == UploadSlotState::Uploading)
            .map(|s| s.effective_rate_bps(now))
            .sum();
        total_bps < PROMOTION_UNPACED_BPS
    }

    /// Promociona a los clientes en espera mientras queden ranuras disponibles. El invocador debe mantener
    /// el cerrojo de `active_uploads` y pasarlo como argumento.
    async fn promote_waiting(
        &self,
        active: &mut std::collections::HashMap<String, UploadSessionSnapshot>,
    ) {
        let mut q = self.queue.lock().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        // Purga de entradas inactivas — eMule `FindBestClientInQueue` retira
        // a los clientes cuya última repetición de petición (re-ask) date de más de
        // `MAX_PURGEQUEUETIME` (1 h) o cuyo archivo solicitado ya no
        // esté compartido (`!sharedfiles->GetFileByID`).
        q.retain(|e| {
            now.saturating_sub(e.last_request_unix_ms) <= MAX_PURGEQUEUE_MS
                && self.hash_still_shared(&e.file_hash)
        });
        // La puntuación evoluciona con la espera: volver a ordenar antes de elegir al
        // mejor candidato (eMule lee `GetScore` en vivo).
        self.sort_queue_by_score(&mut q, now);
        self.promote_locked(&mut q, active, now);
        self.active_count
            .store(Self::uploading_count(active), Ordering::SeqCst);
    }

    /// Cuerpo de `promote_waiting` sobre una cola ya bloqueada — compartido
    /// con `process_tick` que promociona tras cada desalojo de sesión.
    /// Devuelve `true` si al menos un cliente fue promocionado.
    fn promote_locked(
        &self,
        q: &mut VecDeque<QueuedUpload>,
        active: &mut std::collections::HashMap<String, UploadSessionSnapshot>,
        now: i64,
    ) -> bool {
        // eMule `FindBestClientInQueue` marca al mejor cliente
        // inalcanzable cuando su puntuación supera a la del mejor conectado —
        // independientemente de la disponibilidad de ranura (`m_bAddNextConnect`
        // → ranura inmediata al reconectar). La cola está ordenada por puntuación
        // decreciente: el primer conectado no bloqueado es el mejor
        // candidato, el primer desconectado no bloqueado/no marcado previamente es
        // el mejor inalcanzable.
        let mut best_conn_score = 0u64;
        let mut best_disc: Option<(usize, u64)> = None;
        for (i, e) in q.iter().enumerate() {
            if e.banned {
                continue;
            }
            let s = e.effective_score(now);
            if e.connected {
                if s > best_conn_score {
                    best_conn_score = s;
                }
            } else if !e.add_next_connect && best_disc.is_none_or(|(_, bs)| s > bs) {
                best_disc = Some((i, s));
            }
        }
        if let Some((di, ds)) = best_disc {
            if ds > best_conn_score {
                q[di].add_next_connect = true;
            }
        }

        let mut promoted = false;
        loop {
            // Misma elegibilidad que `AcceptNewClient`: la tasa objetivo por
            // cliente, la capacidad `MaxSpeed` y el veto de USS se aplican también
            // a la promoción.
            if !self.can_open_slot(active, now) {
                break;
            }
            // eMule `ForceNewClient`: 1 s mínimo entre dos aperturas
            // mientras la tasa medida sea < 100 KB/s — la cola llena no
            // se vacía en ráfaga durante el arranque.
            if self.promotion_paced(active, now) {
                break;
            }
            // Primer cliente conectado, no bloqueado y no activo ya — al
            // estar la cola ordenada por puntuación, es el mejor candidato.
            // Los clientes desconectados no pueden recibir `ACCEPTUPLOADREQ`.
            let Some(idx) = q.iter().position(|e| {
                e.connected && !e.banned && !active.contains_key(&e.client_key)
            }) else {
                break;
            };
            let next = q.remove(idx).expect("queue index in bounds");

            let snap = UploadSessionSnapshot {
                client_key: next.client_key.clone(),
                client_address: next.client_address.clone(),
                file_hash: next.file_hash.clone(),
                file_name: next.file_name.clone(),
                transferred_bytes: 0,
                upload_rate_bps: 0,
                state: UploadSlotState::Uploading,
                queue_rank: None,
                started_at_unix_ms: now,
                rate_window_start_ms: 0,
                rate_window_bytes: 0,
                rate_sampled: false,
                credit_multiplier_bps: next.credit_multiplier_bps,
                file_prio_num: next.file_prio_num,
                // El inicio de espera sobrevive a la promoción — eMule lo conserva
                // en `credits`; volverá a usarse si el cliente es devuelto
                // a la cola por rotación de sesión.
                queued_since_ms: next.enqueued_at_unix_ms,
                auto_prio: next.auto_prio,
            };
            let promoted_hash = snap.file_hash.clone();
            tracing::info!(
                "[UPLOAD_PROMOTED] Client {} promoted from queue to active upload slot for file '{}' (hash: {})",
                log_addr_str(&snap.client_address), snap.file_name, snap.file_hash
            );
            active.insert(next.client_key, snap);
            // `AddUpNextClient` → `m_nLastStartUpload = GetTickCount()`.
            self.last_upload_start_ms.store(now, Ordering::Relaxed);
            // `AddUploadingClient` → `UpdateAutoUpPriority`.
            self.update_auto_up_prio(active, &promoted_hash);
            // eMule `AddUpNextClient` envía OP_ACCEPTUPLOADREQ al
            // socket en espera — el despachador despertado lo emite.
            next.promote_notify.notify_one();
            promoted = true;
        }
        promoted
    }

    /// Actualiza el progreso de subida activo y mide la velocidad real de la ranura.
    ///
    /// La velocidad se mide internamente por ventanas (`RATE_WINDOW_MS`,
    /// primera muestra en `RATE_FIRST_SAMPLE_MS`) a partir de los bytes
    /// realmente enviados — el invocador solo aporta `bytes_sent`, nunca
    /// una tasa estimada. `effective_rate_bps` disminuye hacia 0 cuando el par
    /// deja de pedir bloques.
    ///
    /// Focalización de ranuras (semántica de eMule): una ranura activa cuya tasa medida
    /// permanezca bajo `TRICKLE_RATE_BPS` tras `TRICKLE_GRACE_MS` se degrada a
    /// `Trickle` — mantiene ancho de banda residual pero libera su ranura para que el
    /// mejor cliente en cola sea promocionado. Esto concentra el ancho de banda en los clientes
    /// rápidos.
    pub async fn update_progress(&self, client_key: &str, bytes_sent: u64) {
        let mut demoted = false;
        {
            let mut active = self.active_uploads.lock().await;
            if let Some(cur) = active.get_mut(client_key) {
                cur.transferred_bytes = cur.transferred_bytes.saturating_add(bytes_sent);
                self.total_uploaded_bytes
                    .fetch_add(bytes_sent, Ordering::SeqCst);
                *self
                    .file_upload_totals
                    .write()
                    .unwrap()
                    .entry(cur.file_hash.to_lowercase())
                    .or_default() += bytes_sent;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                if cur.rate_window_start_ms == 0 {
                    cur.rate_window_start_ms = now;
                }
                cur.rate_window_bytes = cur.rate_window_bytes.saturating_add(bytes_sent);
                let elapsed = now.saturating_sub(cur.rate_window_start_ms);
                // Cierra la ventana vencida — o tras 500 ms en la primera
                // muestra — y mide la velocidad realmente servida.
                if elapsed >= RATE_WINDOW_MS
                    || (!cur.rate_sampled && elapsed >= RATE_FIRST_SAMPLE_MS)
                {
                    cur.upload_rate_bps =
                        cur.rate_window_bytes.saturating_mul(1000) / elapsed as u64;
                    cur.rate_window_start_ms = now;
                    cur.rate_window_bytes = 0;
                    cur.rate_sampled = true;
                }
                if cur.state == UploadSlotState::Uploading
                    && cur.rate_sampled
                    && cur.upload_rate_bps < TRICKLE_RATE_BPS
                    && now.saturating_sub(cur.started_at_unix_ms) > TRICKLE_GRACE_MS
                {
                    cur.state = UploadSlotState::Trickle;
                    demoted = true;
                    tracing::info!(
                        "[UPLOAD_TRICKLE] Client {} demoted to trickle slot (rate {} B/s < {} B/s) — slot freed for queue",
                        cur.client_address, cur.upload_rate_bps, TRICKLE_RATE_BPS
                    );
                }
            }
            if demoted {
                self.promote_waiting(&mut active).await;
            }
        }
        if demoted {
            self.emit_uploads_updated().await;
        } else {
            self.emit_uploads_updated_throttled().await;
        }
    }

    /// Libera la ranura de subida activa del cliente y promociona clientes en espera
    /// hasta volver a llenar las ranuras. Acepta tanto `client_key`
    /// (`uh:<userhash>` o `ip:port`) como una dirección simple `ip:port` — esta
    /// última se resuelve mediante el `client_address` almacenado para que las llamadas RPC
    /// que solo conocen la dirección sigan funcionando.
    pub async fn release_slot(&self, client_key: &str) {
        self.release_active(client_key, true).await;
        self.promote_after_release().await;
    }

    /// El socket TCP del cliente acaba de cerrarse — `Disconnected` de eMule **no**
    /// destruye a un cliente en cola (`bDelete = m_eUploadState !=
    /// US_ONUPLOADQUEUE`, `BaseClient.cpp:1169`): la entrada en `waitinglist`
    /// sobrevive hasta `MAX_PURGEQUEUETIME` con su `WaitStartTime` intacto,
    /// y `FindBestClientInQueue` la marca con `m_bAddNextConnect` si su
    /// puntuación supera al mejor conectado. Una ranura activa, en cambio,
    /// siempre se libera (`US_UPLOADING → RemoveFromUploadQueue`).
    pub async fn mark_disconnected(&self, client_key: &str) {
        let resolved_key = self.release_active(client_key, false).await;
        {
            let mut q = self.queue.lock().await;
            for entry in q.iter_mut() {
                if entry.client_key == resolved_key || entry.client_address == client_key {
                    entry.connected = false;
                }
            }
        }
        // Una ranura activa liberada puede promocionar a un cliente en espera conectado.
        self.promote_after_release().await;
    }

    /// Libera la posible ranura activa y resuelve la clave canónica
    /// (`uh:<hash>` si el cliente activo está registrado con `ip:port`).
    /// `drop_queue` retira también la entrada de la cola (finalización explícita);
    /// `!drop_queue` la conserva con `connected = false` según lo aplique
    /// el invocador (`mark_disconnected`). Devuelve la clave resuelta.
    async fn release_active(&self, client_key: &str, drop_queue: bool) -> String {
        let mut active = self.active_uploads.lock().await;
        let resolved_key = if active.contains_key(client_key) {
            client_key.to_string()
        } else {
            active
                .iter()
                .find(|(_, s)| s.client_address == client_key)
                .map(|(k, _)| k.clone())
                .unwrap_or_else(|| client_key.to_string())
        };
        if let Some(cur) = active.remove(&resolved_key) {
            tracing::info!(
                "[UPLOAD_RELEASED] Active upload slot released by client {} for file '{}' (transferred: {} bytes)",
                resolved_key, cur.file_name, cur.transferred_bytes
            );
            // `RemoveUploadingClient` → `UpdateAutoUpPriority`: el archivo
            // acaba de perder un subidor.
            self.update_auto_up_prio(&active, &cur.file_hash);
        }
        drop(active);

        if drop_queue {
            let mut q = self.queue.lock().await;
            q.retain(|entry| {
                entry.client_key != resolved_key && entry.client_address != client_key
            });
        }
        // Y el inicio de espera preservado por la rotación — el cliente ya no
        // volverá a reclamarlo.
        self.rotation_grace.lock().await.remove(&resolved_key);
        self.rotation_notifies.lock().await.remove(&resolved_key);
        resolved_key
    }

    /// Promociona desde la cola mientras queden ranuras libres — vuelve a adquirir el cerrojo
    /// `active_uploads` tras `release_active`.
    async fn promote_after_release(&self) {
        let mut active = self.active_uploads.lock().await;
        self.promote_waiting(&mut active).await;
        drop(active);
        self.emit_uploads_updated().await;
    }

    /// `OP_END_OF_DOWNLOAD` (`0x49`) — eMule `ListenSocket.cpp:537`: el
    /// descargador señala que ha terminado el archivo. Si `file_hash`
    /// coincide con el archivo actualmente subido o solicitado por este cliente,
    /// este es retirado de la ranura activa / de la cola (`RemoveFromUploadQueue`).
    /// Devuelve `true` si el cliente fue retirado; `false` si el hash no
    /// coincide con ningún archivo servido para este cliente (el llamador aplica
    /// entonces `CheckFailedFileIdReqs`).
    pub async fn end_of_download(&self, client_key: &str, file_hash: &str) -> bool {
        let hash_matches = {
            let active = self.active_uploads.lock().await;
            let in_active = active.values().any(|s| {
                (s.client_key == client_key || s.client_address == client_key)
                    && s.file_hash.eq_ignore_ascii_case(file_hash)
            });
            let q = self.queue.lock().await;
            let in_queue = q.iter().any(|e| {
                (e.client_key == client_key || e.client_address == client_key)
                    && e.file_hash.eq_ignore_ascii_case(file_hash)
            });
            in_active || in_queue
        };
        if hash_matches {
            self.release_slot(client_key).await;
        }
        hash_matches
    }

    /// Descriptor de promoción para la entrada en cola de `client_key` — el
    /// despachador lo espera para emitir `OP_ACCEPTUPLOADREQ` en cuanto el
    /// cliente sea promocionado (eMule `AddUpNextClient` escribe en el socket).
    pub async fn promotion_notify(
        &self,
        client_key: &str,
    ) -> Option<std::sync::Arc<tokio::sync::Notify>> {
        self.queue
            .lock()
            .await
            .iter()
            .find(|e| e.client_key == client_key)
            .map(|e| std::sync::Arc::clone(&e.promote_notify))
    }

    /// Descriptor de rotación para una sesión **activa** de `client_key` —
    /// `None` si el cliente no está subiendo datos. El despachador
    /// se suscribe para emitir `OP_OUTOFPARTREQS` cuando `process_tick`
    /// libera la ranura (`SendOutOfPartReqsAndAddToWaitingQueue` de eMule).
    pub async fn rotation_notify(
        &self,
        client_key: &str,
    ) -> Option<std::sync::Arc<tokio::sync::Notify>> {
        if !self
            .active_uploads
            .lock()
            .await
            .contains_key(client_key)
        {
            return None;
        }
        Some(
            self.rotation_notifies
                .lock()
                .await
                .entry(client_key.to_string())
                .or_insert_with(|| std::sync::Arc::new(tokio::sync::Notify::new()))
                .clone(),
        )
    }

    /// Tick periódico — equivalente a `CUploadQueue::Process()` invocado por
    /// `UploadTimer` en eMule:
    /// 1. purga de entradas de cola > `MAX_PURGEQUEUETIME` (1 h),
    /// 2. rotación de sesiones — `CheckForTimeOver` con
    ///    `FullChunkTransfers` (por defecto en eMule): un cliente que ha recibido más de
    ///    `SESSIONMAXTRANS` regresa a la cola si no se puede
    ///    abrir una nueva ranura (`!ForceNewClient()`), computando su tiempo de subida +
    ///    bonificación de 15 min como espera (`GetScore(isdownloading)`),
    /// 3. promoción de los mejores clientes en espera mientras las ranuras lo
    ///    permitan (`ForceNewClient`/`AcceptNewClient`).
    ///
    /// Debe ejecutarse permanentemente, incluso sin interfaz de usuario conectada.
    pub async fn process_tick(&self) {
        let mut active = self.active_uploads.lock().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let mut evicted: Vec<(String, i64)> = Vec::new();
        let mut changed = false;
        {
            let mut q = self.queue.lock().await;
            let before = q.len();
            q.retain(|e| {
                now.saturating_sub(e.last_request_unix_ms) <= MAX_PURGEQUEUE_MS
                    && self.hash_still_shared(&e.file_hash)
            });
            changed |= q.len() != before;
            // Rotación: mientras la cola esté disputada (sin nuevas ranuras
            // disponibles), las sesiones que superen SESSIONMAXTRANS ceden su
            // ranura — eMule `CheckForTimeOver`. La ranura liberada se cubre
            // inmediatamente (eMule promociona tras cada desalojo) antes
            // de evaluar la siguiente sesión.
            loop {
                if q.is_empty() || self.can_open_slot(&active, now) {
                    break;
                }
                let victim = active
                    .values()
                    .find(|s| {
                        s.state == UploadSlotState::Uploading
                            && s.transferred_bytes >= SESSION_MAX_TRANS_BYTES
                            && !self.is_friend(&s.client_key)
                    })
                    .map(|s| s.client_key.clone());
                let Some(victim) = victim else {
                    break;
                };
                if let Some(cur) = active.remove(&victim) {
                    tracing::info!(
                        "[UPLOAD_ROTATED] Client {} upload slot released after {} bytes session ('{}') — re-queues on next request",
                        log_addr_str(&cur.client_address), cur.transferred_bytes, cur.file_name
                    );
                    // eMule `SendOutOfPartReqsAndAddToWaitingQueue` — el
                    // despachador suscrito emite OP_OUTOFPARTREQS (0x57) por
                    // el socket aún abierto para que el descargador
                    // vuelva a estar en cola en lugar de seguir esperando bloques.
                    if let Some(n) =
                        self.rotation_notifies.lock().await.remove(&victim)
                    {
                        n.notify_one();
                    }
                    // eMule: el cliente NO se reencola aquí — vuelve
                    // a la cola en su próximo re-ask.
                    self.update_auto_up_prio(&active, &cur.file_hash);
                    evicted.push((cur.client_key, cur.queued_since_ms));
                }
                self.sort_queue_by_score(&mut q, now);
                changed |= self.promote_locked(&mut q, &mut active, now);
            }
            self.sort_queue_by_score(&mut q, now);
            changed |= self.promote_locked(&mut q, &mut active, now);
            drop(q);
        }
        // Inicio de espera original (conservado en `credits` en eMule) mantenido
        // para que la puntuación acumulada sobreviva a la rotación.
        if !evicted.is_empty() {
            let mut grace = self.rotation_grace.lock().await;
            for (key, since) in evicted {
                grace.insert(key, since);
            }
            drop(grace);
            changed = true;
        }
        self.active_count
            .store(Self::uploading_count(&active), Ordering::SeqCst);
        drop(active);
        if changed {
            self.emit_uploads_updated().await;
        }
    }

    /// Limpia la cola y las ranuras activas al activar el interruptor de corte o reiniciar la red.
    pub async fn abort_all(&self) {
        let mut active = self.active_uploads.lock().await;
        active.clear();
        self.active_count.store(0, Ordering::SeqCst);
        let mut q = self.queue.lock().await;
        q.clear();
        drop(q);
        self.rotation_grace.lock().await.clear();
        drop(active);
        self.emit_uploads_updated().await;
    }

    /// Devuelve la instantánea de la primera subida activa (si existe).
    /// `upload_rate_bps` es la tasa efectiva en el momento de la lectura.
    pub async fn active_snapshot(&self) -> Option<UploadSessionSnapshot> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let active = self.active_uploads.lock().await;
        active.values().next().cloned().map(|mut s| {
            s.upload_rate_bps = s.effective_rate_bps(now);
            s
        })
    }

    /// Devuelve instantáneas de todas las ranuras de subida activas.
    /// `upload_rate_bps` representa la velocidad efectiva en el instante de la lectura
    /// (disminuye hacia 0 si un par deja de pedir bloques).
    pub async fn active_snapshots(&self) -> Vec<UploadSessionSnapshot> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let active = self.active_uploads.lock().await;
        active
            .values()
            .cloned()
            .map(|mut s| {
                s.upload_rate_bps = s.effective_rate_bps(now);
                s
            })
            .collect()
    }

    /// Devuelve la posición basada en 1 de un cliente en espera coincidente por
    /// dirección IP (semántica `CUploadQueue::GetWaitingClientByIP_UDP`:
    /// los pings UDP solo transportan la IP emisora).
    pub async fn waiting_rank_by_ip(&self, ip: &std::net::IpAddr) -> Option<u32> {
        let ip_str = ip.to_string();
        let q = self.queue.lock().await;
        q.iter()
            .position(|entry| {
                entry
                    .client_address
                    .rsplit_once(':')
                    .map(|(host, _)| host == ip_str)
                    .unwrap_or(false)
            })
            .map(|pos| (pos + 1) as u32)
    }

    /// eMule `GetWaitingClientByIP_UDP(ip, udp_port, bIgnorePortOnUniqueIP
    /// = true, &bSenderMultipleIpUnknown)` + comprobación `md4equ(reqfilehash,
    /// sender->GetUploadFileID())` de `ClientUDPSocket.cpp`:
    /// - un cliente en cola con la misma IP **y** el mismo puerto UDP anunciado
    ///   se identifica directamente;
    /// - en caso contrario, si solo existe **un único** cliente en cola con esa IP,
    ///   se acepta (tolerancia de puerto, `bIgnorePortOnUniqueIP`);
    /// - varios clientes en la misma IP sin coincidencia de puerto →
    ///   [`UdpRankLookup::Ambiguous`] (eMule no responde: fuerza una
    ///   conexión TCP para despejar la ambigüedad de NAT);
    /// - el hash solicitado debe coincidir con el archivo del cliente encontrado,
    ///   de lo contrario [`UdpRankLookup::FileMismatch`] (eMule no responde).
    pub async fn waiting_rank_for_udp(
        &self,
        ip: &std::net::IpAddr,
        udp_port: u16,
        file_hash: &str,
    ) -> UdpRankLookup {
        let ip_str = ip.to_string();
        let clean_hash = file_hash.trim().to_lowercase();
        let q = self.queue.lock().await;
        let same_ip = |e: &&QueuedUpload| {
            e.client_address
                .rsplit_once(':')
                .map(|(host, _)| host == ip_str)
                .unwrap_or(false)
        };
        // 1. Coincidencia exacta IP + puerto UDP (desambiguación NAT).
        if let Some(pos) = q.iter().position(|e| {
            same_ip(&e) && e.client_udp_port.is_some_and(|p| p == udp_port)
        }) {
            let entry = &q[pos];
            return if entry.file_hash.eq_ignore_ascii_case(&clean_hash) {
                UdpRankLookup::Rank((pos + 1) as u32)
            } else {
                UdpRankLookup::FileMismatch
            };
        }
        // 2. Tolerancia de «IP única»: solo un cliente en cola en esta IP.
        let matches: Vec<usize> = q
            .iter()
            .enumerate()
            .filter(|(_, e)| same_ip(e))
            .map(|(pos, _)| pos)
            .collect();
        match matches.as_slice() {
            [] => UdpRankLookup::NoSender,
            [pos] => {
                let entry = &q[*pos];
                if entry.file_hash.eq_ignore_ascii_case(&clean_hash) {
                    UdpRankLookup::Rank((*pos + 1) as u32)
                } else {
                    UdpRankLookup::FileMismatch
                }
            }
            _ => UdpRankLookup::Ambiguous,
        }
    }

    /// eMule `SetLastUpRequest` — un `OP_REASKFILEPING` aceptado de un
    /// cliente en cola actualiza su marca temporal de re-ask (la purga
    /// `MAX_PURGEQUEUETIME` de 1 h no debe expulsar a un cliente activo que
    /// carece de socket TCP abierto para re-ask).
    pub async fn note_udp_reask(&self, ip: &std::net::IpAddr) {
        let ip_str = ip.to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let mut q = self.queue.lock().await;
        for entry in q.iter_mut() {
            if entry
                .client_address
                .rsplit_once(':')
                .map(|(host, _)| host == ip_str)
                .unwrap_or(false)
            {
                entry.last_request_unix_ms = now;
            }
        }
    }

    /// Cantidad de clientes esperando actualmente en la cola de subida.
    pub async fn waiting_len(&self) -> usize {
        self.queue.lock().await.len()
    }

    /// Verdadero si la cola de espera está a menos de 50 entradas de su capacidad máxima —
    /// eMule responde `OP_QUEUEFULL` a pings UDP de clientes desconocidos en
    /// dicho estado (`GetWaitingUserCount() + 50 > GetQueueSize()`).
    pub async fn is_nearly_full(&self) -> bool {
        self.waiting_len().await + 50 > self.max_queue.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Devuelve la lista de instantáneas de todas las subidas en cola.
    pub async fn list_queue(&self) -> Vec<UploadSessionSnapshot> {
        let q = self.queue.lock().await;
        q.iter()
            .enumerate()
            .map(|(idx, item)| UploadSessionSnapshot {
                client_key: item.client_key.clone(),
                client_address: item.client_address.clone(),
                file_hash: item.file_hash.clone(),
                file_name: item.file_name.clone(),
                transferred_bytes: 0,
                upload_rate_bps: 0,
                state: UploadSlotState::Waiting,
                queue_rank: Some((idx + 1) as u32),
                started_at_unix_ms: item.enqueued_at_unix_ms,
                rate_window_start_ms: 0,
                rate_window_bytes: 0,
                rate_sampled: false,
                credit_multiplier_bps: item.credit_multiplier_bps,
                file_prio_num: item.file_prio_num,
                queued_since_ms: item.enqueued_at_unix_ms,
                auto_prio: item.auto_prio,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rebobina `m_nLastStartUpload` entre dos aperturas de ranura — las
    /// pruebas evalúan las condiciones de admisión sin sufrir la espera de 1 s de
    /// `ForceNewClient` (comportamiento de eMule probado por separado).
    fn unpaced(mgr: &UploadManager) {
        mgr.last_upload_start_ms.store(0, Ordering::Relaxed);
    }

    /// eMule `ForceNewClient` (`UploadQueue.cpp:417-421`): por debajo de
    /// `MIN_UP_CLIENTS_ALLOWED` nunca hay espera; por encima, 1 s mínimo
    /// entre dos aperturas mientras la velocidad medida sea < 100 KB/s;
    /// por encima de esa cifra, sin espaciamiento.
    #[tokio::test]
    async fn force_new_client_paces_slot_opens_below_100kbs() {
        let mgr = UploadManager::new();
        // Las dos primeras ranuras se conceden sin temporización
        // (`curUploadSlots < MIN_UP_CLIENTS_ALLOWED → true`).
        for i in 0..2 {
            let (state, _) = mgr
                .request_upload_slot(&format!("c{i}"), &format!("c{i}"), "h", "f", 1.0, "normal", false, None)
                .await
                .unwrap();
            assert_eq!(state, UploadSlotState::Uploading);
        }
        // 3.ª apertura < 1 s tras la anterior, velocidad 0 < 100 KB/s → cola.
        let (state, _) = mgr
            .request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Waiting);
        // Velocidad medida ≥ 100 KB/s → sin espaciamiento: el cliente en cola
        // es promocionado de inmediato.
        {
            let mut active = mgr.active_uploads.lock().await;
            for s in active.values_mut() {
                s.upload_rate_bps = 60 * 1024;
            }
        }
        let (state, _) = mgr
            .request_upload_slot("c3", "c3", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Uploading);
        // Intervalo transcurrido (> 1 s simulado) → apertura incluso por debajo de 100 KB/s.
        mgr.last_upload_start_ms.store(0, Ordering::Relaxed);
        {
            let mut active = mgr.active_uploads.lock().await;
            for s in active.values_mut() {
                s.upload_rate_bps = 0;
            }
        }
        let (state, _) = mgr
            .request_upload_slot("c4", "c4", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Uploading);
    }

    #[tokio::test]
    async fn multiple_active_slots_up_to_limit() {
        let mgr = UploadManager::new();
        mgr.set_max_active(3);
        for i in 0..3 {
            unpaced(&mgr);
            let (state, _) = mgr
                .request_upload_slot(
                    &format!("c{i}"),
                    &format!("c{i}"),
                    "h",
                    "f",
                    1.0,
                    "normal",
                    false,
                    None,
                )
                .await
                .unwrap();
            assert_eq!(state, UploadSlotState::Uploading);
        }
        // El 4.º cliente entra en cola
        let (state, rank) = mgr
            .request_upload_slot("c3", "c3", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Waiting);
        assert_eq!(rank, Some(1));
        assert_eq!(mgr.active_snapshots().await.len(), 3);
    }

    #[tokio::test]
    async fn release_promotes_until_slots_full() {
        let mgr = UploadManager::new();
        mgr.set_max_active(2);
        mgr.request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.request_upload_slot("b", "b", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.request_upload_slot("c", "c", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.request_upload_slot("d", "d", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();

        mgr.release_slot("a").await;
        let actives = mgr.active_snapshots().await;
        assert_eq!(actives.len(), 2);
        assert!(actives.iter().any(|s| s.client_address == "c"));
        assert_eq!(mgr.list_queue().await.len(), 1);

        mgr.release_slot("b").await;
        assert!(mgr
            .active_snapshots()
            .await
            .iter()
            .any(|s| s.client_address == "d"));
        assert!(mgr.list_queue().await.is_empty());
    }

    #[tokio::test]
    async fn userhash_key_dedups_across_ips_and_release_by_address_works() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        // El mismo userhash reconectando desde una IP diferente representa al mismo cliente.
        mgr.request_upload_slot("uh:AA", "1.1.1.1:49662", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        let (state, _) = mgr
            .request_upload_slot("uh:AA", "2.2.2.2:49662", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Uploading);
        // release_slot mediante ip:port simple se resuelve a través de client_address.
        mgr.release_slot("1.1.1.1:49662").await;
        assert_eq!(mgr.active_len(), 0);
    }

    #[tokio::test]
    async fn slow_slot_demoted_to_trickle_and_promotes_queue() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        // c1 ocupa la única ranura.
        let (state, _) = mgr
            .request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Uploading);
        // c2 espera en cola.
        let (state, _) = mgr
            .request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Waiting);
        // c1 permanece lento tras el periodo de gracia → trickle → c2 promocionado.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        {
            let mut active = mgr.active_uploads.lock().await;
            let s = active.get_mut("c1").unwrap();
            s.started_at_unix_ms -= TRICKLE_GRACE_MS + 1;
            // Ventana de medición iniciada hace 1,5 s: la primera
            // muestra (≥ 500 ms) mide ~341 B/s < TRICKLE_RATE_BPS.
            s.rate_window_start_ms = now_ms - 1_500;
        }
        mgr.update_progress("c1", 512).await;
        let actives = mgr.active_snapshots().await;
        assert_eq!(
            actives.iter().find(|s| s.client_key == "c1").unwrap().state,
            UploadSlotState::Trickle
        );
        assert_eq!(
            actives.iter().find(|s| s.client_key == "c2").unwrap().state,
            UploadSlotState::Uploading
        );
        assert_eq!(mgr.active_len(), 1, "trickle does not occupy a full slot");
    }

    #[tokio::test]
    async fn per_file_stats_aggregate_sessions_and_persist_totals() {
        let mgr = UploadManager::new();
        // Carga al arrancar: el archivo ya tiene 1000 B subidos.
        mgr.set_file_upload_totals(std::collections::HashMap::from([(
            "haa".to_string(),
            1000u64,
        )]));
        // Dos ranuras activas sobre el mismo hash (mayúsculas/minúsculas mixtas) + otro archivo.
        mgr.request_upload_slot("c1", "c1", "HAA", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.request_upload_slot("c2", "c2", "haa", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        unpaced(&mgr);
        mgr.request_upload_slot("c3", "c3", "hbb", "g", 1.0, "normal", false, None)
            .await
            .unwrap();
        // Ventana de medición iniciada hace 1 s → primera muestra en el
        // próximo update_progress (transcurrido ≥ RATE_FIRST_SAMPLE_MS).
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        {
            let mut active = mgr.active_uploads.lock().await;
            for s in active.values_mut() {
                s.rate_window_start_ms = now - 1_000;
            }
        }
        mgr.update_progress("c1", 1500).await;
        mgr.update_progress("c2", 1500).await;
        mgr.update_progress("c3", 1500).await;

        let totals = mgr.per_file_uploaded_bytes();
        assert_eq!(totals["haa"], 4000, "hydraté 1000 + 1500 + 1500");
        assert_eq!(totals["hbb"], 1500);
        assert!(!totals.contains_key("hcc"));

        let rates = mgr.per_file_rates_bps().await;
        // ~1500 B/s por ranura → ~3000 B/s agregados para haa, ~1500 para hbb.
        let haa = rates["haa"];
        let hbb = rates["hbb"];
        assert!(haa > hbb, "deux slots agrégés ({haa}) > un slot ({hbb})");
        assert!(haa > 0 && hbb > 0);
        assert!(!rates.contains_key("hcc"));

        // Ranuras liberadas: la tasa vuelve a descender, manteniéndose el total persistido.
        mgr.release_slot("c1").await;
        mgr.release_slot("c2").await;
        assert_eq!(
            mgr.per_file_rates_bps()
                .await
                .get("haa")
                .copied()
                .unwrap_or(0),
            0
        );
        assert_eq!(mgr.per_file_uploaded_bytes()["haa"], 4000);
    }

    #[tokio::test]
    async fn fast_slot_within_grace_is_not_demoted() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        // Tasa lenta pero dentro del periodo de gracia → permanece en Uploading.
        mgr.update_progress("c1", 512).await;
        let s = &mgr.active_snapshots().await[0];
        assert_eq!(s.state, UploadSlotState::Uploading);
    }

    #[tokio::test]
    async fn uss_gate_keeps_new_clients_queued() {
        let mgr = UploadManager::new();
        let flag = Arc::new(AtomicBool::new(false));
        mgr.set_uss_gate(flag.clone());
        // Compuerta cerrada: existen ranuras libres pero el cliente debe esperar en cola.
        let (state, rank) = mgr
            .request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Waiting);
        assert_eq!(rank, Some(1));
        assert_eq!(mgr.active_len(), 0);
        // Compuerta abierta: la siguiente petición se concede de inmediato.
        flag.store(true, Ordering::SeqCst);
        let (state, _) = mgr
            .request_upload_slot("b", "b", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Uploading);
    }

    #[tokio::test]
    async fn events_emitted_on_grant_queue_release() {
        let mgr = UploadManager::new();
        let (tx, mut rx) = broadcast::channel(16);
        mgr.set_event_channel(tx, Arc::new(AtomicU64::new(1)));

        mgr.set_max_active(1);
        mgr.request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        let ev = rx.recv().await.unwrap();
        assert!(ev.contains("upload.uploadsUpdated"));
        assert!(ev.contains("\"uploadRateBps\""));

        mgr.request_upload_slot("b", "b", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        let ev = rx.recv().await.unwrap();
        assert!(ev.contains("upload.uploadsUpdated"));
        assert!(ev.contains("queue"));

        mgr.release_slot("a").await;
        let ev = rx.recv().await.unwrap();
        assert!(ev.contains("upload.uploadsUpdated"));
    }

    #[tokio::test]
    async fn progress_tracked_per_client() {
        let mgr = UploadManager::new();
        mgr.request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.request_upload_slot("b", "b", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.update_progress("a", 100).await;
        mgr.update_progress("b", 200).await;
        let actives = mgr.active_snapshots().await;
        assert_eq!(
            actives
                .iter()
                .find(|s| s.client_address == "a")
                .unwrap()
                .transferred_bytes,
            100
        );
        assert_eq!(
            actives
                .iter()
                .find(|s| s.client_address == "b")
                .unwrap()
                .transferred_bytes,
            200
        );
    }

    #[tokio::test]
    async fn rate_is_measured_per_window_and_decays_on_stall() {
        let mgr = UploadManager::new();
        mgr.request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        let now_ms = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
        };
        // Ventana iniciada hace 1 s: 200 KiB enviados → primera muestra
        // (≥ 500 ms) ≈ 200 KiB/s — la tasa medida refleja los bytes
        // realmente servidos, no el límite configurado.
        {
            let mut active = mgr.active_uploads.lock().await;
            active.get_mut("a").unwrap().rate_window_start_ms = now_ms() - 1_000;
        }
        mgr.update_progress("a", 200 * 1024).await;
        let snaps = mgr.active_snapshots().await;
        let rate = snaps[0].upload_rate_bps;
        assert!(
            (150 * 1024..=250 * 1024).contains(&rate),
            "measured rate should be ~200 KiB/s, got {rate}"
        );
        // Par inactivo desde hace > 2 s: la velocidad efectiva cae a 0 en vez de
        // quedarse congelada en el último valor.
        assert_eq!(snaps[0].effective_rate_bps(now_ms() + 10_000), 0);
    }

    /// eMule `AcceptNewClient`: `curUploadSlots >= MaxSpeed*1024/targetRate`
    /// → rechazo. La cantidad de ranuras sigue la capacidad de subida configurada,
    /// no un tope fijo de 100.
    #[tokio::test]
    async fn accept_new_client_caps_slots_by_configured_upload_speed() {
        let mgr = UploadManager::new();
        let speed = Arc::new(AtomicU64::new(12 * 1024));
        let speed_probe = Arc::clone(&speed);
        mgr.set_upload_speed_probe(move || speed_probe.load(Ordering::Relaxed));

        // Las 4 ranuras garantizadas se conceden siempre (max(MIN_UP_CLIENTS,4)).
        for i in 0..4 {
            unpaced(&mgr);
            let (state, _) = mgr
                .request_upload_slot(&format!("c{i}"), &format!("c{i}"), "h", "f", 1.0, "normal", false, None)
                .await
                .unwrap();
            assert_eq!(state, UploadSlotState::Uploading);
        }
        // 5.ª: 12 KiB/s / 4 KiB/s objetivo = 3 < 4 ranuras → rechazo → cola.
        let (state, _) = mgr
            .request_upload_slot("c5", "c5", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Waiting);

        // Velocidad medida real (40 KiB/s): el término `datarate/minTarget`
        // pasaría (40/3 ≈ 13 > 4) pero `MaxSpeed/target` sigue denegándolo.
        {
            let mut active = mgr.active_uploads.lock().await;
            for s in active.values_mut() {
                s.upload_rate_bps = 10 * 1024;
            }
        }
        let (state, _) = mgr
            .request_upload_slot("c6", "c6", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Waiting);

        // Límite aumentado a 200 KiB/s → 200/4 = 50 > 4 → ranura concedida.
        speed.store(200 * 1024, Ordering::Relaxed);
        unpaced(&mgr);
        let (state, _) = mgr
            .request_upload_slot("c7", "c7", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Uploading);
    }

    /// eMule `IsDownloading` → `OP_ACCEPTUPLOADREQ` + `SetUploadFileID`:
    /// un cliente que posee una ranura conserva dicha ranura al solicitar
    /// otro archivo — nunca se duplica la entrada en cola.
    #[tokio::test]
    async fn downloading_client_other_file_keeps_slot() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.request_upload_slot("c1", "c1", "h1", "f1", 1.0, "normal", false, None)
            .await
            .unwrap();
        let (state, _) = mgr
            .request_upload_slot("c1", "c1", "h2", "f2", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Uploading);
        let snaps = mgr.active_snapshots().await;
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].file_hash, "h2");
        assert_eq!(mgr.waiting_len().await, 0);
    }

    /// eMule: un objeto cliente = una entrada en cola. El re-ask para
    /// otro archivo actualiza la entrada existente en vez de crear una nueva.
    #[tokio::test]
    async fn queued_client_single_entry_and_updates_on_reask() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.request_upload_slot("c2", "c2", "h1", "f1", 1.0, "normal", false, None)
            .await
            .unwrap();
        let (state, _) = mgr
            .request_upload_slot("c2", "c2", "h2", "f2", 1.0, "high", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Waiting);
        assert_eq!(mgr.waiting_len().await, 1);
        let q = mgr.queue.lock().await;
        assert_eq!(q[0].file_hash, "h2");
        assert_eq!(q[0].file_prio_num, 9);
    }

    /// eMule `CheckForTimeOver` (`FullChunkTransfers`, predeterminado): una sesión
    /// por encima de `SESSIONMAXTRANS` es devuelta a la cola cuando no se puede
    /// abrir ninguna ranura — el inicio de espera (`credits`) sobrevive a la rotación.
    #[tokio::test]
    async fn process_tick_rotates_full_session_and_preserves_wait() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        {
            let mut active = mgr.active_uploads.lock().await;
            let s = active.get_mut("c1").unwrap();
            s.transferred_bytes = SESSION_MAX_TRANS_BYTES;
            s.queued_since_ms = now - 60_000; // wait start d'origine
        }
        mgr.process_tick().await;
        // c1 desalojado → c2 promocionado. c1 solo se reincorpora a la cola en su próximo
        // re-ask (comportamiento de eMule: RemoveFromUploadQueue → US_NONE).
        let snaps = mgr.active_snapshots().await;
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].client_key, "c2");
        assert_eq!(mgr.waiting_len().await, 0);

        // Re-ask de c1: inicio de espera restaurado → enqueued_at anterior a now,
        // conservando la puntuación acumulada.
        let (state, rank) = mgr
            .request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Waiting);
        assert_eq!(rank, Some(1));
        let q = mgr.queue.lock().await;
        assert_eq!(
            q[0].enqueued_at_unix_ms,
            now - 60_000,
            "wait start d'origine restauré après rotation"
        );
    }

    /// `SendOutOfPartReqsAndAddToWaitingQueue` de eMule — tras el desalojo por
    /// `CheckForTimeOver`, se notifica al despachador suscrito mediante `rotation_notify`
    /// para emitir `OP_OUTOFPARTREQS` (0x57) en el socket.
    #[tokio::test]
    async fn rotation_notify_fires_on_timeover_eviction() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        // Sin descriptor para un cliente desconocido o que no esté subiendo.
        assert!(mgr.rotation_notify("c2").await.is_none());
        assert!(mgr.rotation_notify("nobody").await.is_none());
        let notify = mgr.rotation_notify("c1").await.unwrap();
        {
            let mut active = mgr.active_uploads.lock().await;
            active.get_mut("c1").unwrap().transferred_bytes = SESSION_MAX_TRANS_BYTES;
        }
        mgr.process_tick().await;
        tokio::time::timeout(std::time::Duration::from_millis(200), notify.notified())
            .await
            .expect("la rotation doit signaler le dispatcher");
        // Descriptor consumido — cesan las notificaciones para esta sesión.
        assert!(mgr
            .rotation_notifies
            .lock()
            .await
            .get("c1")
            .is_none());
    }

    /// La promoción avisa al despachador (`notify`) que emite
    /// `OP_ACCEPTUPLOADREQ` — el cliente en espera no depende de su
    /// siguiente re-ask.
    #[tokio::test]
    async fn promotion_notify_fires_when_slot_frees() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        let notify = mgr.promotion_notify("c2").await.unwrap();
        mgr.release_slot("c1").await;
        tokio::time::timeout(std::time::Duration::from_millis(200), notify.notified())
            .await
            .expect("la promotion doit signaler le dispatcher");
        assert_eq!(mgr.active_snapshots().await[0].client_key, "c2");
    }

    /// eMule `MAX_PURGEQUEUETIME`: una entrada sin re-ask desde hace más de 1 h
    /// es purgada de la cola.
    #[tokio::test]
    async fn stale_queue_entries_purged_after_one_hour() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        {
            let mut q = mgr.queue.lock().await;
            q[0].last_request_unix_ms -= MAX_PURGEQUEUE_MS + 1;
        }
        mgr.process_tick().await;
        assert_eq!(mgr.waiting_len().await, 0);
    }

    /// Límite suave y estricto de eMule: por encima de `QueueSize`, solo los clientes
    /// cuyo `combined(prio × créditos)` supere la media de la cola (o
    /// los amigos) son admitidos; `hard = soft + max(soft,800)/4` supone
    /// un rechazo tajante.
    #[tokio::test]
    async fn soft_limit_admits_only_above_average_combined_score() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.set_max_queue(10); // soft=10, hard=210
        mgr.request_upload_slot("c0", "c0", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        // Llena el límite suave con clientes de alta puntuación
        // (veryhigh=18 × 10.0 créditos → combinado 1800).
        for i in 0..10 {
            mgr.request_upload_slot(&format!("q{i}"), &format!("q{i}"), "h", "f", 10.0, "veryhigh", false, None)
                .await
                .unwrap();
        }
        assert_eq!(mgr.waiting_len().await, 10);
        // Combinado bajo (normal ×1.0 = 70 < 1800) → rechazo.
        assert!(mgr
            .request_upload_slot("low1", "low1", "h", "f", 1.0, "normal", false, None)
            .await
            .is_err());
        // Combinado igual a la media → admitido.
        let (state, _) = mgr
            .request_upload_slot("hi1", "hi1", "h", "f", 10.0, "veryhigh", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Waiting);
    }

    /// eMule `MIN_REQUESTTIME`/`BADCLIENTBAN`: 4 re-asks del mismo archivo en
    /// menos de 10 min → cliente bloqueado (puntuación 0, nunca promocionado).
    #[tokio::test]
    async fn aggressive_reasks_mark_client_banned() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.request_upload_slot("c1", "c1", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        for _ in 0..3 {
            mgr.request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
                .await
                .unwrap();
        }
        assert!(!mgr.queue.lock().await[0].banned);
        mgr.request_upload_slot("c2", "c2", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        let q = mgr.queue.lock().await;
        assert!(q[0].banned);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert_eq!(q[0].effective_score(now), 0);
    }

    /// Mapeo `GetFilePrioAsNumber` de eMule: veryhigh=18, high=9, normal=7,
    /// low=6, verylow=2; powershare recae en la rama predeterminada (7);
    /// «auto» = prioridad efectiva inicial `PR_HIGH` (9).
    #[test]
    fn file_prio_num_matches_emule_get_file_prio_as_number() {
        assert_eq!(file_prio_num("veryhigh"), 18);
        assert_eq!(file_prio_num("high"), 9);
        assert_eq!(file_prio_num("normal"), 7);
        assert_eq!(file_prio_num("low"), 6);
        assert_eq!(file_prio_num("verylow"), 2);
        assert_eq!(file_prio_num("powershare"), 7);
        assert_eq!(file_prio_num("auto"), 9);
        assert_eq!(file_prio_num("unknown"), 7);
    }

    /// eMule `CKnownFile::UpdateAutoUpPriority`: la prioridad efectiva
    /// de un archivo «auto» sigue el número de clientes que lo suben —
    /// más de 20 → baja (6), más de 1 → normal (7), en otro caso → alta (9).
    #[tokio::test]
    async fn auto_up_priority_tracks_uploader_count() {
        let mgr = UploadManager::new();
        let (state, _) = mgr
            .request_upload_slot("c1", "c1", "haa", "f", 1.0, "auto", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Uploading);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let fake = |key: &str| UploadSessionSnapshot {
            client_key: key.to_string(),
            client_address: key.to_string(),
            file_hash: "haa".to_string(),
            file_name: "f".to_string(),
            transferred_bytes: 0,
            upload_rate_bps: 0,
            state: UploadSlotState::Uploading,
            queue_rank: None,
            started_at_unix_ms: now,
            rate_window_start_ms: 0,
            rate_window_bytes: 0,
            rate_sampled: false,
            credit_multiplier_bps: 100,
            file_prio_num: 9,
            queued_since_ms: now,
            auto_prio: true,
        };

        let mut active = mgr.active_uploads.lock().await;
        // c1 activo → +20 simulados = 21 subidores del mismo archivo.
        for i in 0..20 {
            let k = format!("f{i}");
            active.insert(k.clone(), fake(&k));
        }
        mgr.update_auto_up_prio(&active, "haa");
        assert_eq!(
            mgr.auto_up_prio.read().unwrap()["haa"], 6,
            ">20 uploaders → low"
        );

        active.remove("f0");
        mgr.update_auto_up_prio(&active, "haa");
        assert_eq!(
            mgr.auto_up_prio.read().unwrap()["haa"], 7,
            "20 uploaders → normal"
        );

        for i in 1..19 {
            active.remove(&format!("f{i}"));
        }
        mgr.update_auto_up_prio(&active, "haa");
        assert_eq!(
            mgr.auto_up_prio.read().unwrap()["haa"], 7,
            "2 uploaders → encore normal"
        );

        active.remove("f19");
        mgr.update_auto_up_prio(&active, "haa");
        assert_eq!(
            mgr.auto_up_prio.read().unwrap()["haa"], 9,
            "≤1 uploader → high"
        );
    }

    /// Una entrada en cola «auto» refleja el último
    /// `UpdateAutoUpPriority` en cada re-ask u ordenación; un re-ask con
    /// prioridad manual saca al archivo del modo automático.
    #[tokio::test]
    async fn auto_prio_queue_entry_refresh_and_manual_override() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.request_upload_slot("busy", "busy", "hzz", "z", 1.0, "normal", false, None)
            .await
            .unwrap();
        let (state, _) = mgr
            .request_upload_slot("q1", "q1", "haa", "f", 1.0, "auto", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Waiting);
        {
            let q = mgr.queue.lock().await;
            assert!(q[0].auto_prio);
            assert_eq!(q[0].file_prio_num, 9, "auto initial → high");
        }

        // 21 subidores activos en haa → prioridad baja efectiva en el próximo re-ask.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        {
            let mut active = mgr.active_uploads.lock().await;
            for i in 0..21 {
                let k = format!("f{i}");
                active.insert(
                    k.clone(),
                    UploadSessionSnapshot {
                        client_key: k.clone(),
                        client_address: k,
                        file_hash: "haa".to_string(),
                        file_name: "f".to_string(),
                        transferred_bytes: 0,
                        upload_rate_bps: 0,
                        state: UploadSlotState::Uploading,
                        queue_rank: None,
                        started_at_unix_ms: now,
                        rate_window_start_ms: 0,
                        rate_window_bytes: 0,
                        rate_sampled: false,
                        credit_multiplier_bps: 100,
                        file_prio_num: 9,
                        queued_since_ms: now,
                        auto_prio: true,
                    },
                );
            }
            mgr.update_auto_up_prio(&active, "haa");
        }
        mgr.request_upload_slot("q1", "q1", "haa", "f", 1.0, "auto", false, None)
            .await
            .unwrap();
        {
            let q = mgr.queue.lock().await;
            assert!(q[0].auto_prio);
            assert_eq!(q[0].file_prio_num, 6, "auto recalculée → low");
        }

        // Cambio a manual: salida del registro automático + peso explícito.
        mgr.request_upload_slot("q1", "q1", "haa", "f", 1.0, "high", false, None)
            .await
            .unwrap();
        {
            let q = mgr.queue.lock().await;
            assert!(!q[0].auto_prio);
            assert_eq!(q[0].file_prio_num, 9);
        }
        assert!(!mgr.auto_up_prio.read().unwrap().contains_key("haa"));
    }

    #[tokio::test]
    async fn udp_rank_lookup_matches_emule() {
        use std::net::{IpAddr, Ipv4Addr};
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));

        // Ocupa la única ranura activa para que los siguientes clientes
        // pasen a la cola.
        mgr.request_upload_slot("uh:F", "f", "hf", "f", 1.0, "normal", false, None)
            .await
            .unwrap();

        // Un cliente en cola para "haa" con puerto UDP anunciado 49672.
        mgr.request_upload_slot(
            "uh:AAAA",
            "10.0.0.5:5000",
            "haa",
            "f",
            1.0,
            "normal",
            false,
            Some(49672),
        )
        .await
        .unwrap();

        // Hash coincidente + puerto coincidente → posición 1.
        assert_eq!(
            mgr.waiting_rank_for_udp(&ip, 49672, "haa").await,
            UdpRankLookup::Rank(1)
        );
        // Puerto desconocido pero IP única → tolerancia `bIgnorePortOnUniqueIP`.
        assert_eq!(
            mgr.waiting_rank_for_udp(&ip, 9999, "haa").await,
            UdpRankLookup::Rank(1)
        );
        // Hash diferente → silencio en eMule (`reqfile does not match`).
        assert_eq!(
            mgr.waiting_rank_for_udp(&ip, 49672, "hbb").await,
            UdpRankLookup::FileMismatch
        );
        // IP desconocida → NoSender (→ QUEUEFULL únicamente si la cola está casi llena).
        assert_eq!(
            mgr.waiting_rank_for_udp(&IpAddr::V4(Ipv4Addr::new(10, 0, 0, 6)), 49672, "haa")
                .await,
            UdpRankLookup::NoSender
        );
    }

    #[tokio::test]
    async fn udp_rank_lookup_disambiguates_same_ip_by_port() {
        use std::net::{IpAddr, Ipv4Addr};
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 9));

        mgr.request_upload_slot("uh:F", "f", "hf", "f", 1.0, "normal", false, None)
            .await
            .unwrap();

        // Dos clientes tras la misma IP (NAT), puertos UDP diferentes.
        mgr.request_upload_slot(
            "uh:A",
            "10.0.0.9:5000",
            "haa",
            "f",
            1.0,
            "normal",
            false,
            Some(49672),
        )
        .await
        .unwrap();
        mgr.request_upload_slot(
            "uh:B",
            "10.0.0.9:5001",
            "hbb",
            "g",
            1.0,
            "normal",
            false,
            Some(49673),
        )
        .await
        .unwrap();

        // El puerto UDP desambigua → el cliente correcto y el archivo correcto.
        assert_eq!(
            mgr.waiting_rank_for_udp(&ip, 49673, "hbb").await,
            UdpRankLookup::Rank(2)
        );
        // Sin coincidencia de puerto → Ambiguous (eMule fuerza conexión TCP).
        assert_eq!(
            mgr.waiting_rank_for_udp(&ip, 1111, "haa").await,
            UdpRankLookup::Ambiguous
        );
        // Puerto exacto pero archivo diferente → FileMismatch (silencio).
        assert_eq!(
            mgr.waiting_rank_for_udp(&ip, 49672, "hcc").await,
            UdpRankLookup::FileMismatch
        );
    }
    /// eMule `Disconnected` (`BaseClient.cpp:1169`): `bDelete =
    /// m_eUploadState != US_ONUPLOADQUEUE` — un cliente en cola sobrevive al
    /// cierre de su socket conservando intacta su puntuación; `release_slot` (finalización
    /// explícita) siempre lo retira.
    #[tokio::test]
    async fn disconnect_keeps_queue_entry_and_wait_score() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.request_upload_slot("q", "q", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        let enqueued_at = mgr.queue.lock().await[0].enqueued_at_unix_ms;

        mgr.mark_disconnected("q").await;
        {
            let q = mgr.queue.lock().await;
            assert_eq!(q.len(), 1);
            assert!(!q[0].connected);
            assert_eq!(q[0].enqueued_at_unix_ms, enqueued_at);
        }

        // Finalización explícita (CANCELTRANSFER/fin de archivo): retirada completa.
        mgr.release_slot("q").await;
        assert!(mgr.queue.lock().await.is_empty());
    }

    /// eMule `FindBestClientInQueue`: un cliente en espera sin socket no puede
    /// ser promocionado (`ACCEPTUPLOADREQ` se escribe en la conexión) — el
    /// siguiente cliente conectado obtiene la ranura liberada.
    #[tokio::test]
    async fn disconnected_waiter_is_skipped_for_promotion() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        // Dos clientes en espera: "q1" entra primero (mayor puntuación),
        // luego "q2".
        mgr.request_upload_slot("q1", "q1", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.request_upload_slot("q2", "q2", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.mark_disconnected("q1").await;

        mgr.release_slot("a").await;
        let actives = mgr.active_snapshots().await;
        assert_eq!(actives.len(), 1);
        assert_eq!(actives[0].client_address, "q2");
        // q1 permanece en cola, aún marcado como desconectado.
        let q = mgr.queue.lock().await;
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].client_key, "q1");
        assert!(!q[0].connected);
    }

    /// eMule `m_bAddNextConnect` (`UploadQueue.cpp:134-151, 537-549`): el
    /// mejor cliente en espera inalcanzable que supere al mejor conectado queda
    /// marcado — en su reconexión recibe una ranura inmediatamente, incluso
    /// con la cola llena de clientes conectados por debajo.
    #[tokio::test]
    async fn add_next_connect_promotes_on_reconnect() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.request_upload_slot("a", "a", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        // Cliente antiguo en espera (larga espera -> gran puntuación) y luego uno reciente.
        mgr.request_upload_slot("old", "old", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        {
            let mut q = mgr.queue.lock().await;
            q[0].enqueued_at_unix_ms -= 3_600_000;
            q[0].last_request_unix_ms = i64::MAX / 2; // evite la purge
        }
        mgr.request_upload_slot("new", "new", "h", "f", 1.0, "low", false, None)
            .await
            .unwrap();
        mgr.mark_disconnected("old").await;

        // "old" (desconectado, alta puntuación) supera a "new" (conectado, baja prioridad)
        // -> add_next_connect fijado por promote_locked (disparado por el
        // tick de liberación).
        mgr.release_slot("a").await;
        {
            let q = mgr.queue.lock().await;
            let old = q.iter().find(|e| e.client_key == "old").unwrap();
            assert!(old.add_next_connect);
            // "new" fue promocionado a la ranura liberada.
            assert!(mgr.active_snapshots().await.iter().any(|s| s.client_address == "new"));
        }

        // Reconexión de "old": ranura inmediata a pesar de que la cola esté llena.
        let (state, _) = mgr
            .request_upload_slot("old", "old", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        assert_eq!(state, UploadSlotState::Uploading);
        assert!(mgr
            .active_snapshots()
            .await
            .iter()
            .any(|s| s.client_address == "old"));
    }

    /// eMule `Ban()` → `AddBannedClient`: cuando `bad_requests` alcanza
    /// `BAD_CLIENT_BAN`, la IP se incluye en `pending_bans` para un bloqueo
    /// de IP de 2 h en `DownloadService` — no solo retirada de puntuación.
    #[tokio::test]
    async fn bad_client_ban_pushes_ip_for_two_hour_ban() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.request_upload_slot("a", "1.1.1.1:4662", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        mgr.request_upload_slot("f", "9.9.9.9:4662", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        // 4 re-asks muy seguidos para el mismo archivo por parte del par en cola.
        for _ in 0..4 {
            mgr.request_upload_slot("f", "9.9.9.9:4662", "h", "f", 1.0, "normal", false, None)
                .await
                .unwrap();
        }
        assert!(mgr.queue.lock().await[0].banned);
        let bans = mgr.drain_pending_bans().await;
        assert_eq!(bans, vec!["9.9.9.9".to_string()]);
        assert!(mgr.drain_pending_bans().await.is_empty());
    }

    /// `!GetFriendSlot()` en `AddRequestCount`: un amigo nunca se
    /// contabiliza en el control anti-flood de peticiones.
    #[tokio::test]
    async fn friend_is_exempt_from_bad_request_count() {
        let mgr = UploadManager::new();
        mgr.set_max_active(1);
        mgr.set_friend("f", true);
        mgr.request_upload_slot("a", "1.1.1.1:4662", "h", "f", 1.0, "normal", false, None)
            .await
            .unwrap();
        // El amigo entra en cola y vuelve a solicitar 5 veces consecutivas.
        for _ in 0..6 {
            let r = mgr
                .request_upload_slot("f", "9.9.9.9:4662", "h", "f", 1.0, "normal", false, None)
                .await;
            assert!(r.is_ok());
        }
        let q = mgr.queue.lock().await;
        if let Some(e) = q.iter().find(|e| e.client_key == "f") {
            assert!(!e.banned);
            assert_eq!(e.bad_requests, 0);
        }
        drop(q);
        assert!(mgr.drain_pending_bans().await.is_empty());
    }

}

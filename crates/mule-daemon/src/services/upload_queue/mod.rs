// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use crate::services::peer_session::log_addr_str;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

use crate::services::DaemonEvent;


mod lifecycle;
mod scoring;
mod types;
#[cfg(test)]
mod tests;

pub use types::{
    file_prio_num, QueuedUpload, UdpRankLookup, UploadSessionSnapshot, UploadSlotState,
};
use types::UploadSpeedProbe;

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
/// Cola de subida equitativa y acotada con seguimiento de ranuras activas y tiempos de espera.
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

}

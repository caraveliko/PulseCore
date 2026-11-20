// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_format::{AichHash, Ed2kHash};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    Queued,
    WaitingForSources,
    Downloading,
    Verifying,
    Paused,
    Completing,
    Completed,
    Failed,
    Removed,
}

impl DownloadStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::WaitingForSources => "waiting_for_sources",
            Self::Downloading => "downloading",
            Self::Verifying => "verifying",
            Self::Paused => "paused",
            Self::Completing => "completing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Removed => "removed",
        }
    }
}

impl std::str::FromStr for DownloadStatus {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "waiting_for_sources" => Self::WaitingForSources,
            "downloading" => Self::Downloading,
            "verifying" => Self::Verifying,
            "paused" => Self::Paused,
            "completing" => Self::Completing,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "removed" => Self::Removed,
            _ => Self::Queued,
        })
    }
}

/// Representación en memoria de una tarea de descarga activa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub id: Uuid,
    pub display_name: String,
    pub total_size: u64,
    pub ed2k_hash: Ed2kHash,
    pub aich_root: Option<AichHash>,
    pub target_path: PathBuf,
    pub temp_path: PathBuf,
    pub status: DownloadStatus,
    pub created_at: i64,
    pub updated_at: i64,
    pub category_id: Option<String>,
    /// Prioridad de descarga: "low", "normal", "high" (prioridad de archivo de eMule —
    /// sesga la cuota de ranuras del asignador de enjambre para este archivo).
    pub priority: String,
    /// Límite de fuentes por archivo importado desde `.part.met` `FT_MAXSOURCES`
    /// (eMule `CPartFile::GetMaxSources`). `None` = usar el límite global.
    pub max_sources: Option<u32>,
}

impl Download {
    pub fn new(
        display_name: impl Into<String>,
        total_size: u64,
        ed2k_hash: Ed2kHash,
        aich_root: Option<AichHash>,
        target_path: PathBuf,
        temp_path: PathBuf,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        Self {
            id: Uuid::new_v4(),
            display_name: display_name.into(),
            total_size,
            ed2k_hash,
            aich_root,
            target_path,
            temp_path,
            status: DownloadStatus::Queued,
            created_at: now,
            updated_at: now,
            category_id: None,
            priority: "normal".to_string(),
            max_sources: None,
        }
    }
}

/// Metadatos de categoría para organizar las descargas locales.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadCategory {
    pub id: String,
    pub name: String,
    pub color: String, // Hex "#RRGGBB"
    pub sort_order: i32,
    pub default_destination: Option<String>,
    pub default_priority: Option<String>, // "low", "normal", "high"
    pub created_at: i64,
    pub updated_at: i64,
}

impl DownloadCategory {
    pub fn new(
        name: impl Into<String>,
        color: impl Into<String>,
        sort_order: i32,
        default_destination: Option<String>,
        default_priority: Option<String>,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            color: color.into(),
            sort_order,
            default_destination,
            default_priority,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Entrada de evento histórico individual para una transición de descarga.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: i64,
    pub download_id: String,
    pub file_name: String,
    pub total_size: u64,
    pub transferred_bytes: u64,
    pub ed2k_hash: String,
    pub event_type: String, // "completed", "failed", "removed", "verified_pending_promotion", "resumed"
    pub status: String,
    pub duration_secs: u64,
    pub connection_attempts: u32,
    pub resume_count: u32,
    pub verification_result: String, // "verified_ok", "hash_mismatch", "unverified", "skipped"
    pub error_message: Option<String>,
    pub timestamp_unix_ms: i64,
}

/// Filtro de consulta para la lista de historial.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryFilter {
    pub event_type: Option<String>,
    pub status: Option<String>,
    pub from_timestamp_ms: Option<i64>,
    pub to_timestamp_ms: Option<i64>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Estadísticas agregadas de historial y transferencia.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStats {
    pub total_completed_count: u64,
    pub total_completed_bytes: u64,
    pub total_failed_count: u64,
    pub total_resumes_count: u64,
    pub total_integrity_failures: u64,
    // Ventanas de tiempo
    pub session_completed_bytes: u64,
    pub session_completed_count: u64,
    pub last_24h_completed_bytes: u64,
    pub last_24h_completed_count: u64,
    pub last_7d_completed_bytes: u64,
    pub last_7d_completed_count: u64,
}

/// Estado de filtrado para un servidor eD2K.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerFilterStatus {
    Allowed,
    BlockedByFilter,
    Disabled,
}

impl ServerFilterStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::BlockedByFilter => "blocked_by_filter",
            Self::Disabled => "disabled",
        }
    }
}

impl std::str::FromStr for ServerFilterStatus {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "blocked_by_filter" => Self::BlockedByFilter,
            "disabled" => Self::Disabled,
            _ => Self::Allowed,
        })
    }
}

/// Registro persistente de un servidor eD2K en almacenamiento local.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Ed2kServer {
    pub address: String, // "ip:port" as primary key
    pub ip: String,
    pub port: u16,
    pub name: String,
    pub description: String,
    pub ping: u32,
    pub preference: u32,
    pub failed_count: u32,
    pub dyn_ip: Option<String>,
    pub version: Option<String>,
    pub filter_status: ServerFilterStatus,
    pub last_seen_unix_ms: Option<i64>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

/// Estado de una fuente de par descubierta o contactada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerSourceStatus {
    Candidate,
    Allowed,
    Filtered,
    Rejected,
    Tried,
    Connected,
    Failed,
}

impl PeerSourceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Allowed => "allowed",
            Self::Filtered => "filtered",
            Self::Rejected => "rejected",
            Self::Tried => "tried",
            Self::Connected => "connected",
            Self::Failed => "failed",
        }
    }

    /// Semántica eMule `srclist`: una fuente cuya conexión falla,
    /// que responde «archivo ausente» o que está bloqueada por el filtro IP
    /// sale de la lista mostrada y pasa a la lista muerta (deadlist) — cf.
    /// `CUpDownClient::Disconnected` (`RemoveSource` + `AddDeadSource`).
    /// Las entradas `Failed`/`Rejected`/`Filtered` permanecen persistidas para
    /// el retardo de reintento, pero no se muestran ni se cuentan en
    /// `GetSourceCount()`.
    pub fn is_srclist_visible(&self) -> bool {
        !matches!(self, Self::Failed | Self::Rejected | Self::Filtered)
    }
}

impl std::str::FromStr for PeerSourceStatus {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "allowed" => Self::Allowed,
            "filtered" => Self::Filtered,
            "rejected" => Self::Rejected,
            "tried" => Self::Tried,
            "connected" => Self::Connected,
            "failed" => Self::Failed,
            _ => Self::Candidate,
        })
    }
}

/// Origen / vector de descubrimiento de una fuente de par.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerSourceOrigin {
    Server,
    Search,
    Manual,
    Kad,
    SourceExchange,
}

impl PeerSourceOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Search => "search",
            Self::Manual => "manual",
            Self::Kad => "kad",
            Self::SourceExchange => "source_exchange",
        }
    }
}

impl std::str::FromStr for PeerSourceOrigin {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "search" => Self::Search,
            "manual" => Self::Manual,
            "kad" => Self::Kad,
            "source_exchange" | "sx" => Self::SourceExchange,
            _ => Self::Server,
        })
    }
}

/// Motivos de fallo normalizados para los intentos de conexión de pares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerFailureReason {
    IpFilter,
    NetworkPolicy,
    BudgetExhausted,
    ConnectRefused,
    TimeoutConnect,
    TimeoutHandshake,
    TimeoutStatus,
    TimeoutQueue,
    TimeoutFirstData,
    TimeoutInactivity,
    InvalidHandshake,
    HashMismatch,
    InvalidRange,
    CorruptedData,
    FileNotFound,
    /// El par comparte el archivo pero ninguna de sus partes anunciadas es necesaria
    /// (estado de fuente "No Needed Parts" de eMule — mantenida en un enfriamiento prolongado).
    NoNeededParts,
    KillSwitchAborted,
    MissingUserHashForObfuscation,
    ObfuscationNegotiationFailed,
}

impl PeerFailureReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::IpFilter => "ip_filter",
            Self::NetworkPolicy => "network_policy",
            Self::BudgetExhausted => "budget_exhausted",
            Self::ConnectRefused => "connect_refused",
            Self::TimeoutConnect => "timeout_connect",
            Self::TimeoutHandshake => "timeout_handshake",
            Self::TimeoutStatus => "timeout_status",
            Self::TimeoutQueue => "timeout_queue",
            Self::TimeoutFirstData => "timeout_first_data",
            Self::TimeoutInactivity => "timeout_inactivity",
            Self::InvalidHandshake => "invalid_handshake",
            Self::HashMismatch => "hash_mismatch",
            Self::InvalidRange => "invalid_range",
            Self::CorruptedData => "corrupted_data",
            Self::FileNotFound => "file_not_found",
            Self::NoNeededParts => "no_needed_parts",
            Self::KillSwitchAborted => "kill_switch_aborted",
            Self::MissingUserHashForObfuscation => "missing_user_hash_for_obfuscation",
            Self::ObfuscationNegotiationFailed => "obfuscation_negotiation_failed",
        }
    }
}

/// Registro persistente de una fuente de par eD2K en almacenamiento local.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PeerSource {
    pub file_hash: String,
    pub address: String, // "ip:port"
    pub ip: u32,
    pub port: u16,
    pub origin: PeerSourceOrigin,
    pub discovered_at_unix_ms: i64,
    pub last_seen_unix_ms: i64,
    pub status: PeerSourceStatus,
    pub fail_count: u32,
    pub last_error: Option<String>,
    pub cooldown_until_unix_ms: Option<i64>,
    #[serde(default)]
    pub user_hash: Option<String>,
    #[serde(default)]
    pub crypt_options: u8,
    /// ID de compañero (buddy) Kad (hex) para fuentes LowID alcanzables solo a través de una
    /// retrollamada de compañero (`KADEMLIA_CALLBACK_REQ` retransmitida por este nodo compañero).
    #[serde(default)]
    pub buddy_id: Option<String>,
    /// Software de cliente + versión obtenidos del saludo eD2K/eMule
    /// (p. ej. "eMule 0.70b", "aMule 3.0b").
    #[serde(default)]
    pub software: Option<String>,
    /// Máscara de bits de disponibilidad por parte recibida mediante `OP_FILESTATUS`
    /// (`ceil(part_count/8)` bytes, bit activo = la fuente tiene la parte).
    #[serde(default)]
    pub availability: Option<Vec<u8>>,
    /// Servidor eD2K (IPv4, u32 little-endian) al que la fuente está conectada —
    /// `m_dwServerIP` de eMule. Descodificado de entradas de intercambio de fuentes y completado
    /// con el servidor que responde para resultados `OP_FOUNDSOURCES`. 0 = desconocido.
    /// Los LowID solo son únicos por servidor: una fuente `lowid:` solo es invocable mediante
    /// `OP_CALLBACKREQUEST` cuando coincide con nuestro servidor conectado
    /// (eMule `CServerConnect::IsLocalServer`).
    #[serde(default)]
    pub server_ip: u32,
    /// Puerto del servidor eD2K de la fuente — `m_nServerPort` de eMule. 0 = desconocido.
    #[serde(default)]
    pub server_port: u16,
    /// Puerto UDP Kad del par — `GetKadPort()` de eMule. Transportado por entradas
    /// de fuentes de búsqueda Kad (`KadSourceWire.udp_port`); necesario para enviar una
    /// `OP_DIRECTCALLBACKREQ` a una fuente con cortafuegos TCP pero UDP abierto
    /// (`BaseClient.cpp:1414`). 0 = desconocido.
    #[serde(default)]
    pub udp_port: u16,
}

impl PeerSource {
    /// Devuelve el hash de usuario descodificado de 16 bytes si es un valor hexadecimal válido.
    pub fn user_hash_bytes(&self) -> Option<[u8; 16]> {
        self.user_hash.as_deref().and_then(|s| {
            if s.len() != 32 {
                return None;
            }
            let mut bytes = [0u8; 16];
            for i in 0..16 {
                bytes[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
            }
            Some(bytes)
        })
    }

    /// Indica si este par admite ofuscación de protocolo (bit 0).
    pub fn supports_crypt(&self) -> bool {
        (self.crypt_options & 0x01) != 0
    }

    /// Indica si este par solicita ofuscación de protocolo (bit 1).
    pub fn requests_crypt(&self) -> bool {
        (self.crypt_options & 0x02) != 0
    }

    /// Indica si este par requiere estrictamente ofuscación de protocolo (bit 2).
    pub fn requires_crypt(&self) -> bool {
        (self.crypt_options & 0x04) != 0
    }

    /// Devuelve el ID Kad del compañero descodificado de 16 bytes si es un valor hexadecimal válido.
    pub fn buddy_id_bytes(&self) -> Option<[u8; 16]> {
        self.buddy_id.as_deref().and_then(|s| {
            if s.len() != 32 {
                return None;
            }
            let mut bytes = [0u8; 16];
            for i in 0..16 {
                bytes[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
            }
            Some(bytes)
        })
    }
}

/// Archivo compartido aprobado explícitamente y disponible para subida eD2K entrante.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SharedFile {
    pub file_hash: String,
    pub file_name: String,
    pub file_size: u64,
    pub file_path: String,
    /// Hash raíz AICH en caché (SHA-1) del contenido del archivo, calculado en la primera
    /// `OP_AICHFILEHASHREQ` para poder servirse a los pares sin volver a calcular el hash.
    #[serde(default)]
    pub aich_root: Option<[u8; 20]>,
    pub shared_at_unix_ms: i64,
    pub enabled: bool,
    /// Prioridad de subida (`PR_*` de eMule del archivo compartido): "verylow", "low",
    /// "normal", "high", "veryhigh" (= powershare). Utilizada como factor de puntuación
    /// en la cola de subida (semántica `GetFilePrioAsNumber`).
    #[serde(default = "default_up_priority")]
    pub up_priority: String,
    /// `true` cuando esta entrada es una descarga en curso (eMule `CPartFile`):
    /// solo `completed_parts` pueden servirse a los pares. Transitorio — nunca
    /// persistido en la tabla `shared_files`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_partial: bool,
    /// Índices de partes eD2K completamente finalizadas (9 728 000 B) para un
    /// recurso compartido parcial; vacío para archivos compartidos normales (todas las partes disponibles).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completed_parts: Vec<u16>,
    /// Total acumulado de bytes subidos para este archivo — persistido en
    /// `share_upload_stats`, completado por `shares.list` (`CKnownFile::statistic.GetTransferred()`
    /// de eMule).
    #[serde(default)]
    pub uploaded_bytes: u64,
    /// Tasa de subida actual agregada en las sesiones activas de este
    /// archivo (B/s) — medida, 0 cuando no hay ninguna ranura activa.
    #[serde(default)]
    pub upload_rate_bps: u64,
    /// Categoría asociada (`categories.id`) — `None` = sin clasificar. Para
    /// los recursos compartidos sintetizados desde una descarga (parcial o completada
    /// en la BD), refleja `downloads.category_id`; para los compartidos
    /// autónomos, persistido en `shared_files.category_id`.
    #[serde(default)]
    pub category_id: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn default_up_priority() -> String {
    "normal".to_string()
}

impl SharedFile {
    pub fn new(
        file_hash: impl Into<String>,
        file_name: impl Into<String>,
        file_size: u64,
        file_path: impl Into<String>,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        Self {
            file_hash: file_hash.into(),
            file_name: file_name.into(),
            file_size,
            file_path: file_path.into(),
            aich_root: None,
            shared_at_unix_ms: now,
            enabled: true,
            up_priority: "normal".to_string(),
            is_partial: false,
            completed_parts: Vec::new(),
            uploaded_bytes: 0,
            upload_rate_bps: 0,
            category_id: None,
        }
    }
}

/// Registro persistente de créditos concedidos a / obtenidos de un par verificado criptográficamente.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PeerCredit {
    /// Hash SHA-256 o MD4 de la clave pública del par (clave primaria).
    pub pubkey_hash: String,
    /// Bytes sin procesar de la clave pública (codificados en hexadecimal).
    pub pubkey_hex: String,
    /// Total de bytes subidos a este par por nosotros.
    pub uploaded_bytes: u64,
    /// Total de bytes descargados de este par por nosotros.
    pub downloaded_bytes: u64,
    /// Marca de tiempo de la primera autenticación.
    pub first_seen_unix_ms: i64,
    /// Marca de tiempo de la transferencia o autenticación más reciente.
    pub last_seen_unix_ms: i64,
}

impl PeerCredit {
    pub fn new(pubkey_hash: impl Into<String>, pubkey_hex: impl Into<String>) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        Self {
            pubkey_hash: pubkey_hash.into(),
            pubkey_hex: pubkey_hex.into(),
            uploaded_bytes: 0,
            downloaded_bytes: 0,
            first_seen_unix_ms: now,
            last_seen_unix_ms: now,
        }
    }

    /// Calcula el multiplicador de crédito canónico de eMule 0.72a
    /// (`CClientCredits::GetScoreRatio`, `ClientCredits.cpp`), delimitado a
    /// [1.0, 10.0].
    ///
    /// Tres valores candidatos, gana el *menor*:
    /// - `2 * downloaded / uploaded` (proporción, o 10.0 si no se ha subido nada)
    /// - `sqrt(downloaded / 1 MiB + 2)` (límite exponencial: 9.2 MiB → ~3.34,
    ///   100 MiB → ~10.0)
    /// - límite lineal para los primeros fragmentos: `(downloaded - 1 MiB) / 8 598 323
    ///   * 2.34 + 1.0` por debajo de ~9.2 MiB, si no 10.0
    ///
    /// Los pares que nunca nos enviaron nada (< 1 MiB) obtienen exactamente 1.0.
    ///
    /// Definiciones semánticas:
    /// - `downloaded_bytes`: bytes recibidos por nosotros desde este par.
    /// - `uploaded_bytes`: bytes enviados por nosotros a este par.
    pub fn credit_multiplier(&self) -> f64 {
        const ONE_MIB: f64 = 1_048_576.0;
        if self.downloaded_bytes < ONE_MIB as u64 {
            return 1.0;
        }
        let dl = self.downloaded_bytes as f64;
        let ratio = if self.uploaded_bytes > 0 {
            (dl * 2.0) / self.uploaded_bytes as f64
        } else {
            10.0
        };
        let sqrt_cap = (dl / ONE_MIB + 2.0).sqrt();
        let linear_cap = if dl < 9_646_899.0 {
            (dl - ONE_MIB) / 8_598_323.0 * 2.34 + 1.0
        } else {
            10.0
        };
        ratio.min(sqrt_cap.min(linear_cap)).clamp(1.0, 10.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credit_multiplier_canonical_vectors() {
        // Vector 1: Recibido 0, Enviado 0 => 1.0
        let mut credit = PeerCredit::new("hash1", "pub1");
        credit.downloaded_bytes = 0;
        credit.uploaded_bytes = 0;
        assert_eq!(
            credit.credit_multiplier(),
            1.0,
            "Expected floor 1.0 for 0/0"
        );

        // Vector 2 (fórmula eMule): Recibido 5 MiB, Enviado 0 →
        // min(10.0, sqrt(5+2)≈2.65, linear≈2.14) = ~2.14 — el límite lineal
        // domina por debajo de ~9.2 MiB. NO 10.0 como la proporción ingenua.
        credit.downloaded_bytes = 5 * 1_048_576;
        credit.uploaded_bytes = 0;
        let m = credit.credit_multiplier();
        assert!(
            (m - 2.1415).abs() < 0.01,
            "Expected ~2.14 for 5MiB/0, got {m}"
        );

        // Vector 3: Recibido < 1 MiB => suelo de 1.0 sin importar la proporción.
        credit.downloaded_bytes = 1000;
        credit.uploaded_bytes = 2000;
        assert_eq!(credit.credit_multiplier(), 1.0);

        // Vector 4: 100 MiB recibidos, 5 MiB enviados →
        // proporción = 2*100/5 = 40 → limitado por sqrt(100+2)≈10.1 → min = 10.0
        credit.downloaded_bytes = 100 * 1_048_576;
        credit.uploaded_bytes = 5 * 1_048_576;
        assert_eq!(credit.credit_multiplier(), 10.0);

        // Vector 5: intercambio simétrico 10 MiB / 10 MiB →
        // min(2.0, sqrt(12)≈3.46, 10.0) = 2.0
        credit.downloaded_bytes = 10 * 1_048_576;
        credit.uploaded_bytes = 10 * 1_048_576;
        let m = credit.credit_multiplier();
        assert!(
            (m - 2.0).abs() < 0.01,
            "Expected ~2.0 for 10MiB/10MiB, got {m}"
        );
    }

    #[test]
    fn peer_source_status_srclist_visibility_matches_emule() {
        // Fuentes vivas (srclist eMule): en espera, cola /
        // re-petición, conexión en curso, conectada.
        for s in [
            PeerSourceStatus::Candidate,
            PeerSourceStatus::Allowed,
            PeerSourceStatus::Tried,
            PeerSourceStatus::Connected,
        ] {
            assert!(s.is_srclist_visible(), "{s:?} debe permanecer visible");
        }
        // Lista muerta (deadlist eMule): fallo de conexión, FNF, filtro IP — persistidas
        // para el retardo pero ocultas de la lista.
        for s in [
            PeerSourceStatus::Failed,
            PeerSourceStatus::Rejected,
            PeerSourceStatus::Filtered,
        ] {
            assert!(!s.is_srclist_visible(), "{s:?} debe estar oculto");
        }
    }
}

/// Identidad persistida de amigo (`friends.met` de eMule): un hash de usuario al que se
/// concede una ranura de subida dedicada independientemente de la presión de la cola.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Friend {
    /// Hash de usuario eD2K (hex, minúsculas) del amigo.
    pub user_hash: String,
    /// Nombre para mostrar.
    pub name: String,
    /// Marca de tiempo de cuándo se añadió el amigo (ms unix).
    pub added_at_unix_ms: i64,
}

/// Estado de identificación de usuario segura para una sesión de par.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecureIdentState {
    None,
    Requested,
    Verified,
    Failed,
}

/// Elemento de biblioteca multimedia (película o serie de TV) gestionado por eMule-Rust Media Manager.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    pub id: String,
    pub tmdb_id: Option<u64>,
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<u64>,
    pub media_type: String, // "movie" | "tv"
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<u32>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub status: String, // "wanted" | "downloading" | "completed" | "unmonitored"
    pub monitored: bool,
    pub quality_profile: String,
    pub root_folder: String,
    pub file_path: Option<String>,
    pub download_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Episodio de una serie de televisión en la biblioteca multimedia.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeItem {
    pub id: String,
    pub media_id: String,
    pub season_number: u32,
    pub episode_number: u32,
    pub title: String,
    pub air_date: Option<String>,
    pub status: String, // "wanted" | "downloading" | "completed"
    pub monitored: bool,
    pub file_path: Option<String>,
    pub download_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Entrada de calendario que combina un episodio con la información de su serie principal.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEpisode {
    pub episode: EpisodeItem,
    pub series_title: String,
    pub series_poster_path: Option<String>,
    pub series_year: Option<u32>,
}

/// Configuración externa del indexador Torznab/Newznab.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIndexer {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub enabled: bool,
    pub created_at: i64,
}

/// Registro persistido de una notificación de usuario generada por el demonio.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationRecord {
    pub id: i64,
    pub kind: String,
    pub severity: String,
    pub category: String,
    pub title: String,
    pub body: String,
    pub metadata_json: Option<String>,
    pub is_read: bool,
    pub created_at_unix_ms: i64,
}

/// Filtro de consulta para listar el historial de notificaciones.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationFilter {
    pub category: Option<String>,
    pub unread_only: bool,
    pub limit: Option<usize>,
}

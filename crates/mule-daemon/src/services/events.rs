// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::rpc::types::JsonRpcNotification;
use crate::services::downloads::types::DownloadSnapshot;
use crate::services::peer_session::types::PeerSessionSnapshot;
use crate::services::server_session::types::ServerSessionSnapshot;
use crate::services::upload_queue::UploadSessionSnapshot;
use crate::services::upnp::UpnpStatusSnapshot;

/// Métricas agregadas del daemon emitidas a cadencia fija (~2 s) para que los clientes
/// puedan renderizar insignias de la barra superior y gráficos de tasa en tiempo real sin sondeo ni muestreo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatsTickSnapshot {
    /// Suma de `download_rate_bps` en todas las descargas.
    pub download_bps: u64,
    /// Suma de `upload_rate_bps` en las ranuras de subida activas.
    pub upload_bps: u64,
    /// Descargas actualmente en estado `downloading`.
    pub downloads_active: usize,
    /// Conexiones de pares activas totales (todos los archivos).
    pub peers_connected: usize,
    /// Si la sesión del servidor eD2K está conectada.
    pub server_connected: bool,
    /// Contactos totales en la tabla de enrutamiento Kad.
    pub kad_contacts: usize,
    /// Si el socket UDP Kad está enlazado.
    pub kad_udp_bound: bool,
    /// Bytes recibidos desde el inicio del daemon (suma de deltas de sesión por descarga).
    pub session_downloaded_bytes: u64,
    /// Bytes enviados desde el inicio del daemon (contador del gestor de subidas).
    pub session_uploaded_bytes: u64,
    /// Bytes recibidos desde la instalación — persistidos, reiniciables mediante
    /// `stats.resetLifetimeTotals`.
    pub lifetime_downloaded_bytes: u64,
    /// Bytes enviados desde la instalación — persistidos, reiniciables mediante
    /// `stats.resetLifetimeTotals`.
    pub lifetime_uploaded_bytes: u64,
}

/// Eventos fuertemente tipados de dominio y ciclo de vida emitidos a través del daemon.
///
/// Esta es la definición única del formato de cable de notificación: cada
/// evento conoce su nombre de `method` JSON-RPC y serializa su carga útil en
/// `params`. Los puntos de emisión deben pasar por [`DaemonEvent::send`] para que el
/// contrato documentado en `docs/architecture/rpc_protocol.md` §6 no pueda
/// desviarse.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum DaemonEvent {
    /// Se actualizó el progreso, velocidad o estado de una descarga.
    DownloadUpdated {
        sequence: u64,
        download: DownloadSnapshot,
    },
    /// Se añadió una descarga a la cola (cualquier origen: RPC, deep link, laboratorio).
    DownloadAdded {
        sequence: u64,
        download: DownloadSnapshot,
    },
    /// Se eliminó una descarga de la cola. Porta únicamente el identificador.
    DownloadRemoved { sequence: u64, id: String },
    /// Una descarga transitó a `completed` (verificada + trasladada).
    DownloadCompleted {
        sequence: u64,
        download: DownloadSnapshot,
    },
    /// Una descarga transitó a `failed`.
    DownloadFailed {
        sequence: u64,
        download: DownloadSnapshot,
        reason: Option<String>,
    },
    /// El estado de conexión o transferencia de una sesión de par cambió.
    PeerSessionUpdated {
        sequence: u64,
        session: PeerSessionSnapshot,
    },
    /// El estado de conexión de la sesión con el servidor eD2K cambió.
    ServerSessionUpdated {
        sequence: u64,
        session: ServerSessionSnapshot,
    },
    /// Las ranuras/cola de subida cambiaron o el progreso de subida avanzó (limitado
    /// ~500 ms). Porta la instantánea completa de activos+cola además de la
    /// tasa de subida agregada — los clientes renderizan sin calcular diferencias.
    UploadsUpdated {
        sequence: u64,
        active: Vec<UploadSessionSnapshot>,
        queue: Vec<UploadSessionSnapshot>,
        upload_rate_bps: u64,
    },
    /// Tick periódico de estadísticas agregadas (tema `stats`, cadencia ~2 s).
    StatsTick {
        sequence: u64,
        stats: StatsTickSnapshot,
    },
    /// El estado Kad routing/UDP cambió (contactos, bootstrap, estado
    /// del cortafuegos). Evento de aviso: los clientes vuelven a consultar `kad.status`.
    KadStatusChanged { sequence: u64 },
    /// Se registró una entrada en el historial. Evento de aviso: los clientes vuelven a consultar
    /// `history.list` / `history.stats`.
    HistoryEntryAdded { sequence: u64 },
    /// Los ajustes del daemon cambiaron. `keys` enumera las claves de configuración modificadas.
    SettingsChanged { sequence: u64, keys: Vec<String> },
    /// La política de red cambió (perfil, interruptor de corte, presupuestos). Evento
    /// de aviso: los clientes vuelven a consultar `network.getPolicy`.
    NetworkPolicyChanged { sequence: u64 },
    /// El estado de pasarela/mapeo UPnP cambió (habilitar, mapear, desmapear).
    /// `params` es la instantánea completa de `UpnpStatus` — sin `sequence`.
    UpnpStatusChanged { snapshot: UpnpStatusSnapshot },
    /// Una notificación fuera del conjunto tipado (resultados de búsqueda, fuentes
    /// encontradas, eventos de control como `event.snapshotRequired`).
    Custom {
        method: String,
        payload: serde_json::Value,
    },
}

impl DaemonEvent {
    /// Nombre del método de notificación JSON-RPC en el cable.
    pub fn method(&self) -> &str {
        match self {
            Self::DownloadUpdated { .. } => "event.downloadUpdated",
            Self::DownloadAdded { .. } => "event.downloadAdded",
            Self::DownloadRemoved { .. } => "event.downloadRemoved",
            Self::DownloadCompleted { .. } => "event.downloadCompleted",
            Self::DownloadFailed { .. } => "event.downloadFailed",
            Self::PeerSessionUpdated { .. } => "event.peerSessionUpdated",
            Self::ServerSessionUpdated { .. } => "server.sessionUpdated",
            Self::UploadsUpdated { .. } => "upload.uploadsUpdated",
            Self::StatsTick { .. } => "event.statsTick",
            Self::KadStatusChanged { .. } => "kad.statusChanged",
            Self::HistoryEntryAdded { .. } => "history.entryAdded",
            Self::SettingsChanged { .. } => "settings.changed",
            Self::NetworkPolicyChanged { .. } => "network.policyChanged",
            Self::UpnpStatusChanged { .. } => "upnp.statusChanged",
            Self::Custom { method, .. } => method.as_str(),
        }
    }

    /// Tema de suscripción para filtrado de `events.subscribe`; `None` marca
    /// eventos de control que omiten el filtro de temas.
    pub fn topic(&self) -> Option<&'static str> {
        match self {
            Self::DownloadUpdated { .. }
            | Self::DownloadAdded { .. }
            | Self::DownloadRemoved { .. }
            | Self::DownloadCompleted { .. }
            | Self::DownloadFailed { .. } => Some("downloads"),
            Self::PeerSessionUpdated { .. } => Some("peers"),
            Self::ServerSessionUpdated { .. } => Some("servers"),
            Self::UploadsUpdated { .. } => Some("uploads"),
            Self::StatsTick { .. } => Some("stats"),
            Self::KadStatusChanged { .. } => Some("kad"),
            Self::HistoryEntryAdded { .. } => Some("history"),
            Self::SettingsChanged { .. } => Some("settings"),
            Self::NetworkPolicyChanged { .. } | Self::UpnpStatusChanged { .. } => Some("network"),
            Self::Custom { method, .. } => match method.as_str() {
                "server.searchResult" | "server.foundSources" => Some("search"),
                "upnp.statusChanged" => Some("network"),
                "event.snapshotRequired" => None,
                _ => Some("other"),
            },
        }
    }

    /// Serializa a la trama de notificación JSON-RPC enviada a través del socket.
    pub fn to_wire_json(&self) -> Option<String> {
        let params = match self {
            Self::DownloadUpdated { sequence, download }
            | Self::DownloadAdded { sequence, download }
            | Self::DownloadCompleted { sequence, download } => serde_json::json!({
                "sequence": sequence,
                "download": download,
            }),
            Self::DownloadRemoved { sequence, id } => serde_json::json!({
                "sequence": sequence,
                "id": id,
            }),
            Self::DownloadFailed {
                sequence,
                download,
                reason,
            } => serde_json::json!({
                "sequence": sequence,
                "download": download,
                "reason": reason,
            }),
            Self::PeerSessionUpdated { sequence, session } => serde_json::json!({
                "sequence": sequence,
                "session": session,
            }),
            Self::ServerSessionUpdated { sequence, session } => serde_json::json!({
                "sequence": sequence,
                "session": session,
            }),
            Self::UploadsUpdated {
                sequence,
                active,
                queue,
                upload_rate_bps,
            } => serde_json::json!({
                "sequence": sequence,
                "active": active,
                "queue": queue,
                "uploadRateBps": upload_rate_bps,
            }),
            Self::StatsTick { sequence, stats } => serde_json::json!({
                "sequence": sequence,
                "stats": stats,
            }),
            Self::KadStatusChanged { sequence }
            | Self::HistoryEntryAdded { sequence }
            | Self::NetworkPolicyChanged { sequence } => serde_json::json!({
                "sequence": sequence,
            }),
            Self::SettingsChanged { sequence, keys } => serde_json::json!({
                "sequence": sequence,
                "keys": keys,
            }),
            Self::UpnpStatusChanged { snapshot } => {
                serde_json::to_value(snapshot).unwrap_or_default()
            }
            Self::Custom { payload, .. } => payload.clone(),
        };
        serde_json::to_string(&JsonRpcNotification::new(self.method(), params)).ok()
    }

    /// Difunde el evento a todos los suscriptores de WebSocket. Devuelve el
    /// recuento de receptores (0 cuando nadie escucha — no es un error).
    pub fn send(self, event_tx: &broadcast::Sender<String>) -> usize {
        match self.to_wire_json() {
            Some(json) => event_tx.send(json).unwrap_or(0),
            None => 0,
        }
    }
}

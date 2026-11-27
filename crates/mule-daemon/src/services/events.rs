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
    /// Se creó y persistió una nueva notificación para el usuario.
    NotificationCreated {
        sequence: u64,
        notification: mule_core::model::NotificationRecord,
    },
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
            Self::NotificationCreated { .. } => "event.notificationCreated",
            Self::Custom { method, .. } => method.as_str(),
        }
    }

    /// Tema de suscripción para filtrado de `events.subscribe`; `None` marca
    /// eventos de control que omiten el filtro de temas.
    pub fn topic(&self) -> Option<&'static str> {
        topic_for_method(self.method())
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
            Self::NotificationCreated {
                sequence,
                notification,
            } => serde_json::json!({
                "sequence": sequence,
                "notification": notification,
            }),
            Self::Custom { payload, .. } => payload.clone(),
        };
        serde_json::to_string(&JsonRpcNotification::new(self.method(), params)).ok()
    }

    /// Difunde el evento a todos los suscriptores de WebSocket. Devuelve el
    /// recuento de receptores (0 cuando nadie escucha — no es un error).
    pub fn send(self, event_tx: &broadcast::Sender<String>) -> usize {
        if event_tx.receiver_count() == 0 {
            return 0;
        }
        match self.to_wire_json() {
            Some(json) => event_tx.send(json).unwrap_or(0),
            None => 0,
        }
    }
}

/// Fuente única de verdad para asociar el nombre de método JSON-RPC de un evento
/// con su tópico de suscripción en `events.subscribe`.
pub fn topic_for_method(method: &str) -> Option<&'static str> {
    match method {
        "event.downloadUpdated"
        | "event.downloadAdded"
        | "event.downloadRemoved"
        | "event.downloadCompleted"
        | "event.downloadFailed" => Some("downloads"),
        "event.peerSessionUpdated" => Some("peers"),
        "server.sessionUpdated" => Some("servers"),
        "upload.uploadsUpdated" => Some("uploads"),
        "event.statsTick" => Some("stats"),
        "kad.statusChanged" => Some("kad"),
        "shares.changed" => Some("shares"),
        "history.entryAdded" => Some("history"),
        "settings.changed" => Some("settings"),
        "network.policyChanged" | "upnp.statusChanged" => Some("network"),
        "event.notificationCreated" => Some("notifications"),
        "server.searchResult" | "server.foundSources" => Some("search"),
        "autograb.progress" => Some("downloads"),
        "event.snapshotRequired" => None,
        _ => Some("other"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topic_for_method_mapea_todos_los_eventos() {
        assert_eq!(topic_for_method("event.downloadUpdated"), Some("downloads"));
        assert_eq!(topic_for_method("event.peerSessionUpdated"), Some("peers"));
        assert_eq!(topic_for_method("server.sessionUpdated"), Some("servers"));
        assert_eq!(topic_for_method("upload.uploadsUpdated"), Some("uploads"));
        assert_eq!(topic_for_method("event.statsTick"), Some("stats"));
        assert_eq!(topic_for_method("kad.statusChanged"), Some("kad"));
        assert_eq!(topic_for_method("shares.changed"), Some("shares"));
        assert_eq!(topic_for_method("history.entryAdded"), Some("history"));
        assert_eq!(topic_for_method("settings.changed"), Some("settings"));
        assert_eq!(topic_for_method("network.policyChanged"), Some("network"));
        assert_eq!(topic_for_method("upnp.statusChanged"), Some("network"));
        assert_eq!(
            topic_for_method("event.notificationCreated"),
            Some("notifications")
        );
        assert_eq!(topic_for_method("server.searchResult"), Some("search"));
        assert_eq!(topic_for_method("autograb.progress"), Some("downloads"));
        assert_eq!(topic_for_method("event.snapshotRequired"), None);
        assert_eq!(topic_for_method("custom.unknown"), Some("other"));
    }
}

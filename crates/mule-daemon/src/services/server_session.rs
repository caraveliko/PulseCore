// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Gestor de conexión al servidor eD2K.
//!
//! Gestiona como máximo 1 conexión TCP activa hacia un servidor eD2K.
//! Aplica la política de red, límites de tasa por cubeta de tokens (token bucket), cancelación instantánea por kill-switch
//! y decodificación acotada de tramas en red.

use std::net::SocketAddr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::info;

use mule_network_policy::SocketPermit;
use mule_protocol::ed2k::{CallbackRequest, Ed2kLimits, GetSourcesRequest, SearchQuery};
use mule_protocol::framing::RawPacket;

use crate::error::DaemonError;
use crate::services::downloads::DownloadService;

pub mod connection;
pub mod events;
pub mod types;

use connection::{spawn_connection_task, ActiveSession};
use events::emit_event;
pub use types::{ServerConnectionStatus, ServerSessionSnapshot};

/// Gestor seguro para subprocesos (thread-safe) para la única conexión al servidor eD2K.
#[derive(Clone)]
pub struct ServerSessionManager {
    inner: Arc<Mutex<ActiveSession>>,
    event_tx: broadcast::Sender<String>,
    sequence: Arc<AtomicU64>,
    downloads: Arc<std::sync::RwLock<Option<Arc<Mutex<DownloadService>>>>>,
    /// Escuchador entrante — `OP_CALLBACKREQUESTED` (devolución de llamada retransmitida por el
    /// servidor hacia nosotros como LowID) desencadena una conexión de retorno atendida por este servicio.
    inbound:
        Arc<std::sync::RwLock<Option<crate::services::inbound_listener::InboundListenerService>>>,
    /// Servicio Kad — un `OP_IDCHANGE` HighID le comunica nuestra IPv4 pública
    /// (anunciada en contactos `PUBLISH_SOURCE`).
    kad: Arc<std::sync::RwLock<Option<Arc<crate::services::kad::KadService>>>>,
    /// Preferir el puerto de ofuscación dedicado del servidor (`ST_TCPPORTOBFUSCATION`)
    /// cuando se anuncie — refleja el comportamiento de `IsServerCryptLayerConnection()` en eMule.
    prefer_obfuscation: Arc<std::sync::atomic::AtomicBool>,
    /// Relleno aleatorio máximo para el apretón de manos ofuscado (PAD_DIGITS en eMule).
    obfuscation_padding_max: Arc<std::sync::atomic::AtomicU8>,
    /// Último destino de conexión exitosa — empleado por el supervisor de reconexión
    /// automática (preferencia `Reconnect` en eMule). Limpiado ante desconexión iniciada por el usuario.
    last_target: Arc<std::sync::Mutex<Option<ServerTarget>>>,
}

/// Destino de servidor recordado para la reconexión automática (dirección + nombre visible).
#[derive(Clone)]
pub struct ServerTarget {
    /// Dirección del socket del servidor (`ip:puerto`).
    pub addr: SocketAddr,
    /// Nombre visible del servidor, si se conoce.
    pub name: Option<String>,
}

impl ServerSessionManager {
    /// Crea un nuevo `ServerSessionManager`.
    pub fn new(event_tx: broadcast::Sender<String>, sequence: Arc<AtomicU64>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ActiveSession::new())),
            event_tx,
            sequence,
            downloads: Arc::new(std::sync::RwLock::new(None)),
            inbound: Arc::new(std::sync::RwLock::new(None)),
            kad: Arc::new(std::sync::RwLock::new(None)),
            prefer_obfuscation: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            obfuscation_padding_max: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            last_target: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Devuelve el último destino de conexión para el supervisor de reconexión automática.
    pub fn last_target(&self) -> Option<ServerTarget> {
        self.last_target.lock().ok().and_then(|g| g.clone())
    }

    /// Limpia el destino recordado (servidor inactivo eliminado automáticamente, pruebas).
    pub fn clear_last_target(&self) {
        if let Ok(mut g) = self.last_target.lock() {
            *g = None;
        }
    }

    /// Habilita/deshabilita conexiones ofuscadas al servidor (`ST_TCPPORTOBFUSCATION`)
    /// y establece el relleno máximo del apretón de manos (0 = sin relleno, por defecto en eMule ≤ 254).
    pub fn set_obfuscation_preference(&self, enabled: bool, padding_max: u8) {
        self.prefer_obfuscation
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
        self.obfuscation_padding_max
            .store(padding_max, std::sync::atomic::Ordering::Relaxed);
    }

    /// Inyecta el servicio de escucha entrante empleado para conexiones de retorno
    /// `OP_CALLBACKREQUESTED`.
    pub fn set_inbound_listener(
        &self,
        svc: crate::services::inbound_listener::InboundListenerService,
    ) {
        *self.inbound.write().unwrap() = Some(svc);
    }

    /// Establece la referencia al servicio de descargas para la incorporación automática de fuentes encontradas.
    pub fn set_downloads(&self, downloads: Arc<Mutex<DownloadService>>) {
        if let Ok(mut d) = self.downloads.write() {
            *d = Some(downloads);
        }
    }

    /// Inyecta el servicio Kad — el HighID asignado por el servidor se le reenvía
    /// como nuestra IPv4 pública observada para los contactos `PUBLISH_SOURCE`.
    pub fn set_kad(&self, kad: Arc<crate::services::kad::KadService>) {
        if let Ok(mut k) = self.kad.write() {
            *k = Some(kad);
        }
    }

    /// Devuelve una instantánea de la sesión actual del servidor.
    pub async fn snapshot(&self) -> ServerSessionSnapshot {
        let guard = self.inner.lock().await;
        guard.snapshot.clone()
    }

    /// Devuelve true si una conexión con el servidor está activa o en curso.
    pub async fn is_active(&self) -> bool {
        let guard = self.inner.lock().await;
        matches!(
            guard.snapshot.status,
            ServerConnectionStatus::Connecting | ServerConnectionStatus::Connected
        )
    }

    /// Envía una consulta de búsqueda al servidor conectado.
    pub async fn search(&self, query: SearchQuery) -> Result<(), DaemonError> {
        let guard = self.inner.lock().await;
        if guard.snapshot.status != ServerConnectionStatus::Connected {
            return Err(DaemonError::Business(
                "Cannot search: server is not connected".into(),
            ));
        }
        let tx = guard.outgoing_tx.as_ref().ok_or_else(|| {
            DaemonError::Business("Cannot search: outgoing channel closed".into())
        })?;
        let limits = Ed2kLimits::default();
        let packet = query
            .encode(&limits)
            .map_err(|e| DaemonError::Business(format!("Failed to encode search query: {e}")))?;

        info!(
            "Sending eD2K OP_SEARCHREQUEST to server (term='{}', min_size={:?}, max_size={:?}, ext={:?}, payload_len={})",
            query.term,
            query.min_size,
            query.max_size,
            query.extension,
            packet.payload.len()
        );

        tx.send(packet)
            .await
            .map_err(|_| DaemonError::Business("Failed to send search request to server".into()))?;
        Ok(())
    }

    /// Envía una solicitud de obtención de fuentes al servidor conectado para un hash de archivo específico.
    pub async fn get_sources(&self, request: GetSourcesRequest) -> Result<(), DaemonError> {
        let guard = self.inner.lock().await;
        if guard.snapshot.status != ServerConnectionStatus::Connected {
            return Err(DaemonError::Business(
                "Cannot query sources: server is not connected".into(),
            ));
        }
        let tx = guard.outgoing_tx.as_ref().ok_or_else(|| {
            DaemonError::Business("Cannot query sources: outgoing channel closed".into())
        })?;
        let limits = Ed2kLimits::for_server();
        let packet = request.encode(&limits).map_err(|e| {
            DaemonError::Business(format!("Failed to encode get sources request: {e}"))
        })?;
        info!(
            "Sending eD2K OP_GETSOURCES (obfuscation={}) to server for file hash: {}",
            request.obfuscation,
            hex::encode(request.file_hash)
        );
        tx.send(packet).await.map_err(|_| {
            DaemonError::Business("Failed to send get sources request to server".into())
        })?;
        Ok(())
    }

    /// Envía un `OP_CALLBACKREQUEST` al servidor conectado, solicitándole que
    /// retransmita una petición de conexión a un cliente LowID (`client_id < 0x01000000`).
    /// El destino abrirá entonces una conexión TCP de retorno hacia nosotros.
    pub async fn request_callback(&self, client_id: u32) -> Result<(), DaemonError> {
        let guard = self.inner.lock().await;
        if guard.snapshot.status != ServerConnectionStatus::Connected {
            return Err(DaemonError::Business(
                "Cannot request callback: server is not connected".into(),
            ));
        }
        let tx = guard.outgoing_tx.as_ref().ok_or_else(|| {
            DaemonError::Business("Cannot request callback: outgoing channel closed".into())
        })?;
        let packet = CallbackRequest { client_id }.encode();
        info!("Sending eD2K OP_CALLBACKREQUEST to server for LowID client {client_id}");
        tx.send(packet).await.map_err(|_| {
            DaemonError::Business("Failed to send callback request to server".into())
        })?;
        Ok(())
    }

    /// Desconecta cualquier sesión activa y reinicia el estado.
    pub async fn disconnect(&self) -> Result<(), DaemonError> {
        self.disconnect_inner(false).await
    }

    /// Variante del interruptor de corte (`network.disconnectAll`): desmontaje idéntico
    /// pero conserva `last_target` para que el supervisor de reconexión pueda restaurar
    /// la sesión una vez que `network.enable` rearme la política.
    pub async fn disconnect_for_kill_switch(&self) -> Result<(), DaemonError> {
        self.disconnect_inner(true).await
    }

    async fn disconnect_inner(&self, keep_target: bool) -> Result<(), DaemonError> {
        let mut guard = self.inner.lock().await;
        if let Some(handle) = guard.abort_handle.take() {
            handle.abort();
        }
        guard.permit = None;
        guard.outgoing_tx = None;
        guard.snapshot = ServerSessionSnapshot::default();
        if !keep_target {
            // Desconexión iniciada por el usuario: sin reconexión automática a este destino.
            self.clear_last_target();
        }
        self.emit_notification(&guard.snapshot);
        Ok(())
    }

    /// Inicia una conexión a `target_addr` bajo el permiso `permit`.
    /// Rechaza si ya está conectando o conectado.
    #[allow(clippy::too_many_arguments)]
    pub async fn connect(
        &self,
        target_addr: SocketAddr,
        server_name: Option<String>,
        permit: SocketPermit,
        connect_timeout_ms: u64,
        handshake_timeout_ms: u64,
        client_port: u16,
        nickname: String,
        user_hash: [u8; 16],
    ) -> Result<(), DaemonError> {
        let mut guard = self.inner.lock().await;
        if matches!(
            guard.snapshot.status,
            ServerConnectionStatus::Connecting | ServerConnectionStatus::Connected
        ) {
            return Err(DaemonError::Business(
                "An eD2K server connection is already in progress or connected".into(),
            ));
        }

        let address_str = target_addr.to_string();
        if let Ok(mut lt) = self.last_target.lock() {
            *lt = Some(ServerTarget {
                addr: target_addr,
                name: server_name.clone(),
            });
        }
        guard.snapshot = ServerSessionSnapshot {
            status: ServerConnectionStatus::Connecting,
            server_address: Some(address_str.clone()),
            server_name,
            server_desc: None,
            client_id: None,
            is_high_id: false,
            welcome_message: None,
            current_users: None,
            current_files: None,
            connected_at_unix_ms: None,
            ping_ms: None,
        };
        self.emit_notification(&guard.snapshot);

        let (outgoing_tx, outgoing_rx) = mpsc::channel::<RawPacket>(32);
        guard.outgoing_tx = Some(outgoing_tx);

        let inner_clone = Arc::clone(&self.inner);
        let event_tx_clone = self.event_tx.clone();
        let sequence_clone = Arc::clone(&self.sequence);
        let downloads_clone = self.downloads.read().ok().and_then(|g| g.clone());
        let inbound_clone = self.inbound.read().ok().and_then(|g| g.clone());
        let kad_clone = self.kad.read().ok().and_then(|g| g.clone());
        let cancel_token = permit.cancellation_token();
        let prefer_obf = self
            .prefer_obfuscation
            .load(std::sync::atomic::Ordering::Relaxed);
        let pad_max = self
            .obfuscation_padding_max
            .load(std::sync::atomic::Ordering::Relaxed);

        let task = spawn_connection_task(
            target_addr,
            cancel_token,
            connect_timeout_ms,
            handshake_timeout_ms,
            client_port,
            nickname,
            user_hash,
            outgoing_rx,
            inner_clone,
            event_tx_clone,
            sequence_clone,
            downloads_clone,
            inbound_clone,
            kad_clone,
            prefer_obf,
            pad_max,
        );

        guard.abort_handle = Some(task.abort_handle());
        guard.permit = Some(permit);

        Ok(())
    }

    fn emit_notification(&self, snapshot: &ServerSessionSnapshot) {
        emit_event(&self.event_tx, &self.sequence, snapshot);
    }
}

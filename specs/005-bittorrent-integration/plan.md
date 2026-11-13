# Plan de implementación: Integración de BitTorrent y Magnet

**ID de funcionalidad**: `005-bittorrent-integration`  

## 1. Diseño modular
- Crear `crates/mule-torrent` como un crate independiente en el workspace.
- El crate envuelve `librqbit` en un servicio limpio y seguro entre hilos (`TorrentService`) que proporciona:
  - `start_session(output_dir, listen_port)`
  - `add_magnet(uri, output_dir, category)`
  - `add_torrent_file(bytes, output_dir, category)`
  - `list_torrents()` -> devuelve instantáneas normalizadas
  - `pause_torrent(hash)`, `resume_torrent(hash)`, `remove_torrent(hash, delete_files)`
- Añadir `torrent_service: Arc<TorrentService>` a `AppState` en `crates/mule-daemon`.
- Actualizar `crates/mule-daemon/src/rpc/transmission.rs` para enrutar las peticiones hacia `mule-core` o `mule-torrent`.
- Actualizar `web/index.html` para mostrar las insignias de protocolo (`[eD2k]` / `[Torrent]`) y aceptar enlaces magnet en el modal Añadir.
- Añadir pruebas de integración exhaustivas y verificar los límites de memoria.

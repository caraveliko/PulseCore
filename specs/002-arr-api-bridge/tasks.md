# Desglose de tareas: Puente de compatibilidad *arr (Transmission RPC)

- [x] Tarea 1: Crear el manejador de ruta Transmission RPC en `crates/mule-daemon/src/rpc/transmission.rs`
  - Implementar la comprobación de cabecera CSRF `X-Transmission-Session-Id` (devolver 409 cuando falte o no sea válida).
  - Implementar las estructuras de petición y respuesta para los métodos JSON-RPC (`session-get`, `torrent-get`, `torrent-add`, `torrent-remove`, `torrent-start`, `torrent-stop`, `session-set`, `free-space`).
- [x] Tarea 2: Registrar `/transmission/rpc` en `crates/mule-daemon/src/rpc/server.rs`
- [x] Tarea 3: Mapear los códigos de estado y propiedades de descarga:
  - Mapear `DownloadSnapshot` / `DownloadDetail` a los campos torrent de Transmission (`id`, `hashString`, `percentDone`, `status`, `downloadDir`, `files`).
- [x] Tarea 4: Añadir pruebas unitarias y de integración para Transmission RPC en `crates/mule-daemon/tests/transmission_rpc_tests.rs`.
- [x] Tarea 5: Verificar el handshake del cliente de prueba de Sonarr/Radarr y la simulación del ciclo de vida en vivo.

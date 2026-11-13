# Desglose de tareas: Integración de BitTorrent y Magnet

- [x] Tarea 1: Crear el crate `crates/mule-torrent` y configurar `Cargo.toml` con `librqbit`.
- [x] Tarea 2: Implementar el envoltorio `TorrentService` en `crates/mule-torrent/src/lib.rs`.
- [x] Tarea 3: Añadir `TorrentService` a `AppState` en `crates/mule-daemon`.
- [x] Tarea 4: Mejorar `crates/mule-daemon/src/rpc/transmission.rs` para el soporte dual de protocolos (`torrent-add`, `torrent-get`, `torrent-start`, `torrent-stop`, `torrent-remove`).
- [x] Tarea 5: Actualizar `web/index.html` para soportar enlaces magnet y mostrar insignias de protocolo.
- [x] Tarea 6: Añadir pruebas de integración en `crates/mule-daemon/tests/transmission_rpc_tests.rs` verificando la ingesta de magnet y el listado dual de protocolos.
- [x] Tarea 7: Ejecutar la suite de pruebas Playwright y comprobar la huella de memoria.

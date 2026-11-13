# Desglose de tareas: Interfaz web integrada y búsqueda global

- [x] Tarea 1: Añadir `rust-embed` y `mime_guess` a `crates/mule-daemon/Cargo.toml`.
- [x] Tarea 2: Implementar el servicio de recursos integrados en `crates/mule-daemon/src/rpc/server.rs`.
- [x] Tarea 3: Construir los componentes de interfaz de búsqueda en `web/index.html`.
- [x] Tarea 4: Conectar la interfaz de búsqueda a los métodos WebSocket `kad.searchKeywords` y `servers.search` + los eventos `server.searchResult`.
- [x] Tarea 5: Añadir prueba de integración de extremo a extremo (`test_embedded_web_ui_serving` en `crates/mule-daemon/tests/transmission_rpc_tests.rs`) validando la entrega de recursos integrados.

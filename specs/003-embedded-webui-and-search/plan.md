# Plan de implementación: Interfaz web integrada y búsqueda global

**ID de funcionalidad**: `003-embedded-webui-and-search`  

## 1. Plan técnico
- Añadir las dependencias `rust-embed` y `mime_guess` a `crates/mule-daemon`.
- Crear el manejador `embedded_web` que consulta `Asset::get` y devuelve las cabeceras MIME adecuadas.
- Actualizar `crates/mule-daemon/src/rpc/server.rs` para usar `embedded_web` como respaldo cuando `state.config.web_dir` sea `None` o el directorio no exista.
- Actualizar `web/index.html` para añadir la vista de la pestaña Búsqueda, los controles de búsqueda y la cuadrícula de resultados.
- Implementar la lógica JavaScript para gestionar `kad.searchKeywords` y las notificaciones WebSocket `server.searchResult`.

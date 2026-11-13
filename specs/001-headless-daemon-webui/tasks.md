# Tareas de implementación: Servidor multiplataforma e interfaz web integrada

**ID de funcionalidad**: `001-headless-daemon-webui`  
**Estado**: Completado  

---

## Lista de verificación de tareas

### Fase 1: Configuración del demonio y servicio Linux / Windows
- [x] `TASK-101`: Implementar la estructura `DaemonConfig` en `mule-daemon` y la persistencia de ajustes en caliente sobre SQLite (`KNOWN_SETTINGS_KEYS`).
- [x] `TASK-102`: Añadir los parámetros de línea de comandos (`--workdir`, `--downloads-dir`, `--bind`, `--ed2k-port`, `--kad-port`, `--no-tray`) en `crates/mule-daemon/src/main.rs`.
- [x] `TASK-103`: Habilitar la dirección de escucha configurable en `crates/mule-daemon/src/rpc/server.rs` (soportando LAN `0.0.0.0` con guardia anti-SSRF/CSRF y autenticación por token).
- [x] `TASK-104`: Escribir la plantilla `packaging/systemd/mule-daemon.service` con `Restart=always` y aislamiento de seguridad.
- [x] `TASK-105`: Verificar el cierre ordenado ante `SIGTERM`/`shutdown_notify` activando un checkpoint limpio de `.part.met` y SQLite.

### Fase 2: Recursos estáticos integrados en Axum
- [x] `TASK-201`: Verificar el enrutamiento del servidor web en `crates/mule-daemon/src/rpc/server.rs`.
- [x] `TASK-202`: Crear el manejador de recursos integrados en Axum sirviendo `web/index.html` con `rust-embed` y soporte opcional mediante `--web-dir`.
- [x] `TASK-203`: Añadir cabeceras de caché (`Cache-Control`) y detección MIME para los recursos integrados.
- [x] `TASK-204`: Gestionar el respaldo de enrutamiento en el lado del cliente (SPA -> `index.html`).

### Fase 3: Frontend del panel web (HTML5 + JavaScript puro)
- [x] `TASK-301`: Crear la estructura base de una interfaz web ligera y reactiva en `web/index.html` sin sobrecarga externa ni frameworks compilados.
- [x] `TASK-302`: Implementar el cliente WebSocket JSON-RPC 2.0 con reconexión automática a `/rpc`.
- [x] `TASK-303`: Construir la barra de estado: HighID, nodos Kad, indicadores de velocidad de subida/descarga.
- [x] `TASK-304`: Construir la tabla de descargas: Nombre de archivo, porcentaje de progreso, velocidad, fuentes, prioridad.
- [x] `TASK-305`: Construir el visualizador de mapa de partes (mostrando bloques de 9,28 MB descargados frente a los faltantes).
- [x] `TASK-306`: Construir el modal «Añadir enlace» con analizador y validación de enlaces ed2k.

### Fase 4: Validación y puertas de calidad
- [x] `TASK-401`: Ejecutar `cargo check --workspace` y probar la compilación del binario en Linux.
- [x] `TASK-402`: Verificar que el consumo de memoria RSS se mantiene por debajo de 60 MB (observado: ¡31,3 MB!).
- [x] `TASK-403`: Verificar el ciclo completo: La interfaz web responde con HTTP 200 y se autoconecta mediante `window.MULE_SESSION`.

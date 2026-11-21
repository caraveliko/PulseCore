# Notas de la Versión — Pulse Core v0.4.0-beta.4

**Fecha de publicación:** 21 de noviembre de 2026  
**Estado:** Versión beta — **Corrección crítica de memoria y visor de registros (`system.logs`)**, **ajuste efectivo de `jemalloc`**, **reducción de ruido en logs P2P** y **renovación visual de la WebUI con soporte PWA**.

---

## 1. Síntesis de la Versión

La versión **v0.4.0-beta.4** resuelve un fallo crítico de agotamiento de memoria y *livelock* en nodos Linux/Raspberry Pi sin swap provocado por la lectura completa de archivos de registro masivos en `system.logs` y la emisión a nivel `INFO` de cada paquete en la ruta de descarga. Además, corrige el símbolo de configuración de `jemalloc` para habilitar sus hilos de purga en segundo plano e incorpora la renovación visual completa de la WebUI con activos estáticos embebidos, manifiesto PWA y caché HTTP optimizada.

---

## 2. Novedades y Correcciones Principales

### Estabilidad de Memoria, Visor de Registros y `systemd` (`mule-daemon`)
- **Lectura inversa acotada en `system.logs` (`rpc/handlers/settings.rs`)**: `handle_logs` deja de cargar archivos `.log` completos en memoria y pasa a escanearlos desde el final en bloques de `64 KiB` (`LOG_READ_CHUNK_BYTES`), con techo global de `16 MiB` (`LOG_MAX_SCAN_BYTES`) y límite de `256 KiB` por línea (`LOG_MAX_LINE_BYTES`), preservando UTF-8 multibyte entre bloques y el orden cronológico ascendente.
- **Eliminación de spam `INFO` por paquete (`services/peer_session/transfer.rs`)**: las trazas de alta frecuencia en el bucle de descarga (`sending block request packet`, `new pending block(s) planned`, `OP_SENDINGPART*` y `OP_COMPRESSEDPART*`) pasan a nivel `DEBUG`, reduciendo drásticamente el crecimiento diario de `daemon.*.log`.
- **Configuración efectiva de `jemalloc` (`crates/mule-daemon/src/main.rs`)**: se corrige el símbolo exportado a `#[export_name = "_rjem_malloc_conf"]` (con `#[used]` y tipo `Option<&'static c_char>`) requerido por `tikv-jemalloc-sys`, activando `dirty_decay_ms:0,muzzy_decay_ms:0,background_thread:true`.
- **Plantilla `systemd` segura sin swap (`scripts/mule-daemon.service`)**: se retira `MemoryHigh=1536M` manteniendo `MemoryMax=2048M` para evitar *thrashing/livelock* perpetuo de memoria anónima sin disparar OOM ni `Restart=always`.

### Interfaz Web, Activos Estáticos, PWA y Despliegue (`web`, `rpc/server.rs`, `scripts`)
- **Tema visual y experiencia PWA**: incorporación de `web/assets/theme.css`, `web/assets/atmosphere.js`, fondos optimizados (`WebP`), ilustraciones de estado vacío (`SVG`), iconos `192`/`512`, `web/manifest.webmanifest` y corrección del apilamiento del menú Herramientas en `web/index.html`.
- **Caché diferenciada para recursos embebidos**: `embedded_asset_handler` sirve `/assets/*` y `manifest.webmanifest` con `Cache-Control: public, max-age=86400, must-revalidate`, conservando `no-cache, no-store, must-revalidate` en el HTML principal.
- **Despliegue limpio en SSH (`scripts/update_linux.sh`)**: archivado automático de carpetas heredadas `~/.local/web` en `~/.local/web.bak-legacy` para garantizar el uso de la WebUI embebida en el binario.

---

## 3. Verificación e Integridad

Todas las comprobaciones relevantes (`cargo clippy --workspace --all-targets --all-features -- -D warnings`, pruebas unitarias `logs_tests`, chequeo y compilación cruzada `aarch64-unknown-linux-gnu` y validación en vivo sobre Raspberry Pi 5 con `jemalloc_bg_thd` activo y respuesta de `system.logs` en ~22 ms) superadas antes de la publicación del paquete para **Windows x64**, **Linux x64** y **Linux ARM64 (`aarch64`)**.

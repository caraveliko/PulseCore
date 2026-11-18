# Notas de la Versión — Pulse Core v0.3.0-beta.4

**Fecha de publicación:** 18 de noviembre de 2026  
**Estado:** Versión beta — monitor detallado de ancho de banda en la WebUI, guardián de montajes NFS/NAS, mejoras P2P/Kad y wiki técnica completa en español.

---

## 1. Síntesis de la Versión

La versión **v0.3.0-beta.4** incorpora un monitor en tiempo real de ancho de banda y tráfico P2P en la interfaz web embebida, protección integral contra escrituras accidentales en disco local cuando el montaje NFS/NAS se encuentra desconectado, mejoras de estabilidad y persistencia en el motor P2P/Kad y la wiki oficial completa en español.

---

## 2. Novedades y Mejoras Principales

### Monitor de Ancho de Banda y Tráfico P2P (WebUI)
- **Acceso interactivo desde la barra de estado inferior**: al pulsar sobre el indicador global de bajada/subida (`#statusbar-speed-pill`) se abre un panel modal en tiempo real con telemetría completa y gráfica ampliada.
- **Gráfica SVG detallada e interactiva**: historial ampliado a 10 minutos (`300` muestras) con ventanas seleccionables (`1 min`, `2 min`, `5 min`, `10 min`), filtrado por serie (`Ambas`, `Solo Bajada`, `Solo Subida`), líneas de referencia de pico, escala automática y lectura de instante al pasar el cursor.
- **Tarjetas KPI y desglose por transferencia**: velocidades actuales, picos de sesión, medias de ventana, desglose por protocolo (`eD2K/Kad` frente a `BitTorrent`), volumen de sesión/histórico (`event.statsTick`), ranuras de subida activas/en cola, lista en vivo de descargas y subidas activas y botón rápido de **Modo Tortuga**.

### Protección y Verificación de Montajes NFS / NAS
- **Nuevo servicio `nfs_mount`**: inspección unificada de `/proc/mounts` y `/etc/fstab` en Linux (y rutas UNC/SMB fuera de Linux), resolución segura de enlaces simbólicos y detección de la trampa de directorio desmontado (`unmounted_fstab_trap`).
- **Guardián antes de escribir (`nasRequireNfs`)**: comprobación de disponibilidad del montaje y sonda TCP al puerto `2049` antes de crear carpetas o transferir archivos en `organizer`, `nas_transfer` y `settings`.
- **Visibilidad en Preferencias WebUI**: indicador del estado del montaje NFS detectado, puntos de montaje de red descubiertos en el sistema y ajuste en caliente para exigir montaje de red.

### Motor P2P, Kad y Sesiones de Pares
- **Persistencia del `KadID` local (`kad_local_id`)**: conservación de la identidad Kademlia entre reinicios del demonio y actualización sincronizada al rotar identidad.
- **Mejoras en enrutamiento Kad, ofuscación y sesiones**: optimizaciones en tablas de enrutamiento, manejo de fuentes en SQLite, negociación de ofuscación y transferencia en `peer_session`, `inbound_listener` y `server_session`.

### Documentación, Wiki y Marca
- **Wiki técnica oficial en español**: 12 manuales completos de usuario y referencia técnica enlazados desde el `README.md`.
- **Renovación visual**: nuevo logotipo SVG vectorial de enjambre de nodos y actualización integral de la portada del repositorio.

---

## 3. Verificación e Integridad

Todas las compilaciones para **Windows (x64)**, **Linux (x64)** y **Raspberry Pi (ARM64 / aarch64)** se compilan mediante GitHub Actions y se acompañan con su correspondiente suma SHA-256 y manifiesto firmado Ed25519 (`manifest.json`) cuando la clave de firma está configurada.

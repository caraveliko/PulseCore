# Especificación de funcionalidad: Puente de compatibilidad *arr (API de Transmission / qBittorrent)

**ID de funcionalidad**: `002-arr-api-bridge`  
**Estado**: Implementado  
**Versión objetivo**: `0.4.0`  
**Autor**: Antigravity & User  
**Creado**: 2026-10-29  

---

## 1. Resumen ejecutivo

Los ecosistemas de automatización en servidores domésticos (Sonarr, Radarr, Prowlarr) requieren protocolos estándar de clientes de descarga para encolar contenido multimedia automáticamente, supervisar el progreso de las transferencias e importar las descargas completadas en las carpetas de la biblioteca. Dado que eDonkey/eD2k carece de soporte nativo en el ecosistema *arr, los usuarios dependían históricamente de carpetas vigiladas frágiles y scripts externos en Python.

Esta especificación define una capa de compatibilidad nativa dentro de `mule-daemon` que implementa la **especificación Transmission RPC** (y el endpoint **Torznab** en `/torznab/api`), permitiendo que Sonarr, Radarr y Prowlarr configuren `eMule-Rust` como un cliente de descarga o indexador externo de forma directa.

> **Nota sobre la automatización nativa (`006-native-servarr-media-automation`)**: Pulse Core integra de forma nativa su propio motor Prowlarr/Radarr/Sonarr/Bazarr dentro de `mule-daemon`, por lo que ya **no es obligatorio** instalar contenedores o programas externos del ecosistema `*arr`. Los endpoints `/transmission/rpc` y `/torznab/api` de esta especificación se mantienen como puente opcional de compatibilidad para usuarios que prefieran conectar sus instancias externas existentes.

---

## 2. Planteamiento del problema y valor para el usuario

- **El problema**: Los scripts externos de vigilancia (como los que se ejecutan sobre aMule) pueden quedar bloqueados, fallar al detectar transferencias completadas o interpretar mal los estados de descarga.
- **El valor**:
  - Integración directa: Sonarr y Radarr se comunican directamente por HTTP.
  - Ciclo de vida nativo: Cuando una descarga alcanza el 100 % y supera la verificación AICH, el cliente *arr recibe el estado completado y gestiona el enlace duro (hardlink) o el movimiento de forma atómica.
  - Soporte de categorías: Etiqueta automáticamente las descargas por categoría (p. ej., `tv` o `movies`) para separar los directorios de destino.

---

## 3. Mapeo de protocolos (Transmission RPC -> eMule-Rust)

El puente expone un endpoint HTTP en `/transmission/rpc`:

| Método Transmission RPC | Equivalente eMule-Rust | Lógica de mapeo |
| :--- | :--- | :--- |
| `session-get` | `system.status` | Devuelve la versión del demonio, el directorio de descargas, los límites de velocidad y el ID de sesión |
| `torrent-get` | `downloads.list` | Mapea `id`, `name`, `totalSize`, `percentDone`, `rateDownload`, `status` |
| `torrent-add` | `downloads.addEd2kLink` | Extrae enlaces `ed2k://` de la URL o nombre de archivo y encola la descarga |
| `torrent-start` | `downloads.resume` | Reanuda la transferencia pausada |
| `torrent-stop` | `downloads.pause` | Pausa la transferencia activa |
| `torrent-remove` | `downloads.remove` | Elimina la transferencia (con `delete-local-data` opcional) |

### Mapeo de códigos de estado
- Transmission `0` (STOPPED) -> eMule `Paused`
- Transmission `3` (QUEUED) -> eMule `Waiting`
- Transmission `4` (DOWNLOADING) -> eMule `Downloading`
- Transmission `6` (SEEDING / COMPLETE) -> eMule `Completed`

---

## 4. Criterios de aceptación

1. **Configuración del cliente**: En Sonarr/Radarr dentro de *Settings > Download Clients*, al añadir un cliente «Transmission» en `http://<ip>:4660` con la ruta `/transmission/rpc`, el botón «Test» supera la prueba de conexión.
2. **Ingesta de transferencias**: El envío desde Prowlarr o de forma manual de un enlace `ed2k://` a través del endpoint de Transmission encola correctamente el elemento en `mule-core`.
3. **Seguimiento del progreso**: La pestaña Activity de Sonarr muestra el porcentaje de progreso activo y la velocidad de transferencia en tiempo real.
4. **Notificación de finalización**: Cuando la descarga alcanza el 100 %, Sonarr activa automáticamente su flujo de importación.

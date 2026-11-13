# Plan de implementación: Puente de compatibilidad *arr (Transmission RPC)

**ID de funcionalidad**: `002-arr-api-bridge`  
**Versión objetivo**: `0.4.0`  
**Estado**: Implementado  

## 1. Resumen de la arquitectura
Sonarr y Radarr se comunican con los clientes de descarga mediante el estándar Transmission RPC. Al implementar `/transmission/rpc` directamente en `mule-daemon`, `eMule-Rust` pasa a ser seleccionable directamente en Sonarr/Radarr como un cliente Transmission estándar.

```
+-------------------+                      +-----------------------+
|  Sonarr / Radarr  |                      |      mule-daemon      |
|                   |                      |                       |
|  Download Client  | --- HTTP POST -----> |  /transmission/rpc    |
|   (Transmission)  |     (with CSRF)      |  (Axum Route Handler) |
|                   | <--- JSON Resp ----- |                       |
+-------------------+                      +-----------+-----------+
                                                       |
                                           +-----------v-----------+
                                           |      mule-core        |
                                           |   DownloadService     |
                                           +-----------------------+
```

## 2. Requisitos y endpoints
1. Protección CSRF:
   - Comprobar la cabecera `X-Transmission-Session-Id`.
   - Si falta o no coincide, devolver `409 Conflict` con `X-Transmission-Session-Id: <token>`.
2. Métodos RPC soportados:
   - `session-get`: versión, directorios, velocidades, unidades.
   - `session-set`: directorio de descargas, límites.
   - `session-stats`: contadores, velocidades.
   - `free-space`: información de espacio en disco.
   - `torrent-get`: proyección de campos, filtro `ids`, mapeo de estado (0=Paused, 3=Waiting, 4=Downloading, 6=Seeding/Finished).
   - `torrent-add`: analiza enlaces ed2k o archivo/url, los encola en el núcleo.
   - `torrent-start` / `torrent-start-now`: reanuda descargas.
   - `torrent-stop`: pausa descargas.
   - `torrent-remove`: elimina de la cola o del disco.

## 3. Estrategia de pruebas
- Pruebas unitarias que verifiquen la serialización y deserialización JSON de las peticiones y respuestas de Transmission.
- Pruebas de integración que simulen el handshake de Sonarr (409 -> reintento con token -> `session-get` -> `torrent-add` -> `torrent-get` -> `torrent-remove`).

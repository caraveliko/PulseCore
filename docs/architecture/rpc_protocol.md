# Protocolo RPC del demonio — JSON-RPC 2.0 sobre WebSocket

Documento de referencia del contrato entre `mule-daemon` y sus clientes
(interfaz web integrada, `mule-mcp`, `mule-assistant`). `mule-cli` es una herramienta
de inspección local (hash, enlaces, `.part.met`) y no consume el RPC.

> Fuente de verdad del código: `crates/mule-daemon/src/rpc/`
> (`server.rs`, `methods.rs`, `types.rs`, `handlers/*`).

---

## 1. Transporte

| Elemento | Valor |
|---|---|
| Endpoint | `ws://<host>:<port>/rpc` |
| Health check | `GET /health` → `{"status":"healthy","daemonVersion":"x.y.z"}` |
| Subida de actualización | `POST /api/system/upload-update` (`Bearer <token>`, validación ELF64/PE y arquitectura) |
| UI web estática | `GET /` → `index.html` + recursos estáticos (`ServeDir` o `rust-embed` integrado — véase §8) |
| Config de sesión | `GET /session-config.js` → `window.MULE_SESSION` (endpoint+token), solo loopback |
| Framing | un mensaje de texto WebSocket = una petición **o** una respuesta **o** una notificación JSON |
| Codificación | JSON UTF-8 |

Cada petición entrante se despacha en una tarea `tokio::spawn`:
las respuestas pueden llegar **en cualquier orden** → el cliente
debe correlacionar mediante `id`. Un canal `mpsc` de capacidad 128 serializa los
mensajes salientes (respuestas + eventos); un cliente lento es desconectado
por contrapresión.

## 2. Envolventes (JSON-RPC 2.0)

```jsonc
// Petición
{"jsonrpc":"2.0","id":1,"method":"downloads.list","params":{}}

// Respuesta correcta
{"jsonrpc":"2.0","id":1,"result":{...}}

// Respuesta de error
{"jsonrpc":"2.0","id":1,"error":{"code":-32010,"message":"...","data":{...}}}

// Notificación del servidor (evento, sin id)
{"jsonrpc":"2.0","method":"<event>","params":{...}}
```

- Una petición **sin `id`** se trata como una notificación: no se
  emite ninguna respuesta (`methods.rs`: `let req_id = req.id?`).
- `id` es un `serde_json::Value`: se aceptan entero, cadena o null.

## 3. Handshake y autenticación

Primera llamada obligatoria:

```jsonc
{"jsonrpc":"2.0","id":0,"method":"system.hello","params":{
  "protocolVersion": 1,
  "token": "<auth_token si está configurado>"
}}
```

Respuesta:

```jsonc
{"jsonrpc":"2.0","id":0,"result":{
  "daemonVersion": "0.2.0-RC1",
  "protocolVersion": 1,
  "instanceId": "<uuid>",
  "sessionId": "<uuid>",
  "capabilities": ["downloads","download-events","servers","kad-contacts", ...],
  "workdirMode": "...",
  "workdirPath": "...",
  "defaultDownloadDir": "..."
}}
```

Reglas implementadas (`handle_hello`):
- `protocolVersion != PROTOCOL_VERSION` → error `-32001` (incompatibilidad de protocolo).
- Si `auth_token` está configurado en el demonio y `token` falta o es erróneo → `-32000`.
- Si `auth_token` está **vacío**, la sesión queda autenticada automáticamente
  (`AtomicBool::new(auth_token.is_empty())`).
- Cualquier método distinto de `system.hello` antes de autenticar → `-32000`.
- Los eventos de difusión (broadcast) solo se envían a sesiones autenticadas.

`events.subscribe` solo se atiende para sesiones autenticadas
(interceptado en `handle_socket`; antes de autenticar → `-32000`). Véase §6 para
la semántica de suscripción por tópicos.

## 4. Códigos de error

| Código | Constante | Significado |
|---|---|---|
| -32700 | `CODE_PARSE_ERROR` | JSON mal formado |
| -32600 | `CODE_INVALID_REQUEST` | Envolvente inválida |
| -32601 | `CODE_METHOD_NOT_FOUND` | Método desconocido |
| -32602 | `CODE_INVALID_PARAMS` | Parámetros inválidos (`data` puede detallar) |
| -32603 | `CODE_INTERNAL_ERROR` | Error interno |
| -32000 | `CODE_AUTH_FAILED` | Autenticación fallida / aún no autenticado |
| -32001 | `CODE_PROTOCOL_MISMATCH` | `protocolVersion` incompatible |
| -32002 | `CODE_FORBIDDEN_IN_INSTALLED_MODE` | Acción prohibida en modo instalado |
| -32003 | `CODE_UNSAFE_CLEANUP_TARGET` | Limpieza hacia una ruta no autorizada |
| -32004 | `CODE_FIXTURE_DOWNLOAD_MISMATCH` | Fixture de prueba inválido |
| -32005 | `CODE_LOOPBACK_PEER_UNAVAILABLE` | Par loopback no disponible |
| -32010 | `CODE_BUSINESS_ERROR` | Error de negocio genérico (NotFound, Core, Session…) |

`DaemonError` → `JsonRpcError`: `InvalidParams`→-32602,
`NotFound`/`Business`/`Core`/`Session`→-32010, `Io`/`Internal`→-32603.

## 5. Catálogo de métodos

Espacios de nombres estables (véase `methods.rs` para la lista exhaustiva mantenida):

| Espacio de nombres | Métodos representativos |
|---|---|
| `system.*` | `hello`, `status`, `stop`/`shutdown`, `diskSpace`, `nasInfo`, `logs`, `checkUpdate`, `getUpdateStatus`, `applyUpdate`, `rollbackUpdate` (véase `adr_auto_update.md`) |
| `notifications.*` | `list` (`{limit, unreadOnly, category}`), `markRead` (`{id}` o `{all: true}`), `clear`, `getConfig`, `setConfig`, `test` (`{channel}`) |
| `downloads.*` | `list`, `get`, `addEd2kLink`, `pause`, `resume`, `remove`, `setDestination`, `rename` (`{id, newName}` — disco + `shared_files` si ha finalizado), `setCategory`, `getSources`, `inspectPartMet`, `importPartMet`, `retryConnection` |
| `media.*` / `indexer.*` | Catálogo multimedia Servarr, búsqueda unificada Torznab/IMDb, automatización, limpieza y reemplazos (`media.*`, `indexer.*`) |
| `servers.*` | `list`, `connect`, `disconnect`, `status`, `search`, `querySources`, `importMet`, `updateFromUrl`, `setEnabled`, `delete`, `clear` |
| `sources.*` | `list`, `addManual`, `connect`, `disconnect`, `clear`, `status`/`swarmStatus` |
| `kad.*` | `status`, `startUdp`, `stopUdp`, `bootstrap`, `findNode`, `searchSources`, `searchKeywords`, `searchNotes`, `publish*`, `listContacts`, `importNodesDat` |
| `network.*` | `getPolicy`, `setProfile`, `enable`, `disconnectAll`, `getAuditLog`, `setBudgets` |
| `settings.*` / `limits.*` | `get`, `set`, `setRetentionDays`, `resetDownloadDir`, `listDirs`, `pickFolder`, `openFolder`, `exportBackup`, `restoreBackup` — véase `docs/SETTINGS.md` para todas las claves |
| `stats.*` | `resetLifetimeTotals` → `{reset: true}` — reinicia a cero los contadores acumulados «desde la instalación» (`lifetime_*_bytes`, memoria + SQLite); los totales de sesión se conservan |
| `shares.*` | `list` → `{sharedFiles[]}` (incl. parciales `isPartial`/`completedParts` + estadísticas de subida por archivo `uploadRateBps`/`uploadedBytes` — caudal medido agregado en los slots activos, total persistido en `share_upload_stats` — y `categoryId`/`upPriority`), `add`, `remove`, `rename` (`{fileHash, newName}` — archivo en disco + entrada, delega en `downloads.rename` cuando el hash respalda una descarga), `setEnabled`, `setUploadPriority` (ambos rechazan los parciales), `setCategory` (`{fileHash, categoryId|null}` — aceptado en parciales: prevalece la categoría de la descarga subyacente), `rescan` |
| `uploads.*` / `listener.*` | `list`, `status`, `cancel`; `start`, `stop`, `status` |
| `identity.*` / `credits.*` | `status`, `rotate`; `list`, `stats` |
| `upnp.*` | `status`, `mapPorts`, `unmapPorts`, `setEnabled` |
| `filter.*` | `getStatus`, `importFile`, `setEnabled`, `clear` — `enabled`/`filePath` persistidos (`ipfilter_*`) y recargados al arrancar |
| `history.*` / `categories.*` | `list`, `stats`, `clear`, `export`; CRUD + `reorder` |
| `subtitles.*` | `search` (devuelve `score`, `scoreReasons`, `fpsMilli`, `variant`, `releaseName`, `isAutoTranslated`, `isTrusted`), `download`, `check`, `autoFetch`, `sync` (`{mediaId, season?, episode?, lang?, mode?, offsetMs?, fromFps?, toFps?}`), `blacklistAndRetry` (`{mediaId, season?, episode?, provider, providerSubId, reason?, lang?}`) |
| `events.*` | `subscribe` (filtrado por tópicos, véase §6) |
| `lab.*` / `ed2k.*` / `debug.*` | condicionados por feature (`lab-loopback`, `ed2k-loopback`, `debug-rpc`) — **no documentar como API pública** |

Convenciones de nomenclatura vigentes: `namespace.verbo` en camelCase
(`downloads.addEd2kLink`). Mantener esta convención para cualquier nuevo
método; los alias existentes (`downloads.retry`, `swarm.status`…) se
mantienen por compatibilidad.

**Ciclo kill switch / reconexión** — `network.disconnectAll` cancela el
token compartido: todas las sesiones TCP de pares/servidor caen con su
limpieza completa (mapa de sesiones, bloques en vuelo, estados SQLite) y los
sockets UDP (Kad, servidor) se detienen explícitamente; los supervisores
(vigilante de servidor, `SwarmCoordinator`, rotador de identidad, cuota)
permanecen vivos pero inertes mientras el estado sea `Disabled`.
`network.enable` crea un token nuevo y relanza el listener TCP
entrante, los sockets UDP, el mantenimiento Kad, USS y el refresco UPnP;
al conservarse `last_target` durante el kill switch, el vigilante reconecta
automáticamente al último servidor si `server_reconnect` está activo.
`servers.connect` sobre una sesión activa cambia a la nueva
dirección (desconexión y luego conexión — semántica eMule); la misma
dirección devuelve de forma idempotente la instantánea actual.

## 6. Eventos (notificaciones del servidor)

Los eventos pasan por `state.event_tx` (broadcast tokio de `String`)
y se reenvían como notificaciones JSON-RPC a las sesiones
autenticadas. `DaemonEvent` (`services/events.rs`) es la fuente única
del formato de red: cada variante conoce su `method`, su `topic` de
suscripción y serializa su payload mediante `to_wire_json()`; todos los
emisores pasan por `DaemonEvent::send`. Formato de red = método con punto:

| method | params | topic | Frecuencia |
|---|---|---|---|
| `event.downloadUpdated` | `{sequence, download: DownloadSnapshot}` | `downloads` | alta (progreso) |
| `event.downloadAdded` | `{sequence, download: DownloadSnapshot}` | `downloads` | por alta |
| `event.downloadRemoved` | `{sequence, id}` | `downloads` | por eliminación |
| `event.downloadCompleted` | `{sequence, download: DownloadSnapshot}` | `downloads` | transición → `completed` |
| `event.downloadFailed` | `{sequence, download: DownloadSnapshot, reason}` | `downloads` | transición → `failed` |
| `event.peerSessionUpdated` | `{sequence, session: PeerSessionSnapshot}` | `peers` | media |
| `server.sessionUpdated` | `{sequence, session: ServerSessionSnapshot}` | `servers` | baja |
| `upload.uploadsUpdated` | `{sequence, active: [UploadSessionSnapshot], queue: [UploadSessionSnapshot], uploadRateBps}` | `uploads` | por mutación + progreso limitado ~500 ms |
| `event.statsTick` | `{sequence, stats: {downloadBps, uploadBps, downloadsActive, peersConnected, serverConnected, kadContacts, kadUdpBound, sessionDownloadedBytes, sessionUploadedBytes, lifetimeDownloadedBytes, lifetimeUploadedBytes}}` | `stats` | cadencia ~2 s, suspendido sin clientes WS conectados (la persistencia de contadores acumulados continúa independientemente) |
| `kad.statusChanged` | `{sequence}` (aviso → refrescar `kad.status`) | `kad` | por mutación (udp, bootstrap, contactos) + bucle de mantenimiento cuando `total_contacts` cambia realmente (≤30 s) |
| `shares.changed` | `{sequence}` (aviso → refrescar `shares.list`) | `shares` | por `add`/`remove`/`rescan`/`setEnabled`/`setUploadPriority`/`setCategory` (cubre mutaciones RPC y MCP) |
| `history.entryAdded` | `{sequence}` (aviso → refrescar `history.list`) | `history` | por entrada de historial |
| `settings.changed` | `{sequence, keys: [String]}` | `settings` | por `settings.set*` |
| `network.policyChanged` | `{sequence}` (aviso → refrescar `network.getPolicy`) | `network` | perfil, kill switch, presupuestos |
| `server.searchResult` | `{sequence, files: [...]}` | `search` | por búsqueda |
| `server.foundSources` | `{sequence, fileHash, sources}` | `search` | por búsqueda de fuentes |
| `upnp.statusChanged` | `UpnpStatus` | `network` | por acción UPnP |
| `event.notificationCreated` | `{sequence, notification: NotificationRecord}` | `notifications` | por nueva alerta persistida |
| `event.snapshotRequired` | `{skipped: u64}` | — (siempre reenviado) | rarísima (lag) |

`DownloadSnapshot` incluye `etaSeconds` (`Option<u64>`, segundos restantes
al ritmo actual) desde E5; `PeerSessionSnapshot.transferRateBps`
expone la velocidad por fuente.

**Visualización de partes (barra al estilo eMule 0.72a)** — la instantánea
expone tres campos para el renderizado preciso por bytes de la barra de disponibilidad:

- `parts[]` (`PartProgressSummary`): `index`, `offset`, `size`,
  `receivedBytes`, `state`, `availableSourceCount` y `receivedMask`
  (`u64` — mapa de bits de 64 celdas intraparte, ≈152 KiB/celda, calculado
  desde el `RangeSet` real mediante `contains`).
- `pendingRanges` (`[[start,end],…]` en bytes absolutos): rangos de los
  bloques en vuelo (`in_flight_blocks` del swarm, equivalente a
  `requestedblocks_list`), enriquecido en `downloads.list`,
  `downloads.get`, `broadcast_download_update` y las emisiones
  `transfer.rs`/`strategy.rs`; renderizado en amarillo superpuesto.
- `availableSourceCount`: número de fuentes confirmadas que han anunciado
  la parte mediante `OP_FILESTATUS` (`part_count == 0` = fuente completa,
  contada para todas las partes; máscaras caducadas > 30 min ignoradas —
  `SOURCE_FRESHNESS_MS`). La máscara se registra en `act.sources`
  en cada respuesta `FILESTATUS`, incluidos los keep-alives en cola
  de espera, con emisión `downloadUpdated` → la barra pasa de rojo
  a azul en directo en cuanto un par confirma sus partes.

El campo `sequence` (`u64` monótono compartido) permite al cliente detectar
huecos y resincronizar mediante `*.list`/`*.status`.

**Suscripción por tópicos** — `events.subscribe {topics: [...]}` se
intercepta en `handle_socket` (filtro por sesión, no por el demonio):

- `topics` ausente o `null` → se reenvía todo (retrocompatible).
- `topics: []` o lista → solo pasan los eventos cuyo `topic` está en la
  lista; `event.snapshotRequired` (sin tópico) pasa siempre.
- La correspondencia método → tópico está definida en
  `topic_for_method` (`services/events.rs`), reutilizada tanto por
  `DaemonEvent::topic` como por el filtro WebSocket (`events.subscribe`);
  los métodos `Custom` no listados recaen en el tópico `other`.

**Resincronización** — cuando una sesión consume demasiado lento y el
búfer de difusión descarta eventos (`RecvError::Lagged`), el demonio
emite `event.snapshotRequired {skipped}`: el cliente debe solicitar una
instantánea completa (`downloads.list`, `servers.list`, …) en lugar de
confiar en el flujo incremental.

**Por definir (hoja de ruta)**:
- Limitación/muestreo en el servidor de los eventos de progreso.

## 7. Versionado

- `PROTOCOL_VERSION` (`u32`, actualmente `1`): se incrementa en cada
  cambio **incompatibilizador** (renombrado, eliminación, cambio de tipo).
- Adiciones retrocompatibles (nuevo método, nuevo campo opcional,
  nueva capacidad): **sin** incremento de versión, anunciadas mediante
  `capabilities` en `system.hello` → el cliente realiza detección de capacidades.
- `daemonVersion` (semver del crate): informativo, no analizarlo
  para comprobar compatibilidad.

## 8. Seguridad / exposición de red

**UI web servida por el demonio** — cuando `web_dir` está configurado
(`--web-dir`, `MULE_WEB_DIR` o autodetección `<exe>/../web`,
`<exe>/web`) o mediante recursos integrados (`rust-embed`), el mismo listener Axum sirve la
interfaz estática (`ServeDir` + respaldo `index.html` para rutas SPA) y `GET /session-config.js`
con cabeceras `Cross-Origin-Resource-Policy: same-origin` y `Cache-Control: no-cache, no-store, must-revalidate`.
Esta última ruta emite `window.MULE_SESSION = {endpoint, token, instanceId, requiresPassword}`:
cuando existe `webuiPassword` y la petición procede de LAN (no loopback), oculta `token` y activa
`requiresPassword: true` hasta autenticarse en `POST /api/auth/login`. Todo el plano de control
está protegido por `control_plane_security_guard` (aislamiento estricto a LAN/loopback, validación
`Host` anti-DNS Rebinding y validación `Origin` anti-CSRF/CSWSH rechazando orígenes externos y `Origin: null`).

- El endpoint no es TLS: en HTTPS se requerirá `wss://`
  (terminación TLS delante de Axum: proxy inverso, o `rustls` nativo).
- `auth_token` transmitido en claro en `system.hello` → aceptable en
  loopback/LAN, obligatorio detrás de TLS si se expone. Nunca vincular
  `0.0.0.0` en claro en Internet: el token otorga el control total del
  demonio (añadir/eliminar descargas, `system.stop`).
- `session-config.js` entrega el token en claro — es intencionado y seguro
  únicamente porque la ruta está restringida a escucha loopback
  (misma máquina, sin fuga de red). Cualquier evolución hacia una escucha
  no loopback debe conservar esta guarda o exigir TLS.
- El mismo puerto sirve rutas HTTP (`/`, `/health`,
  `/session-config.js`): prever una lista de orígenes (comprobación CORS/Origin)
  si el demonio se expone fuera de loopback.
- Los manejadores `lab.*`/`ed2k.*`/`debug.*` se compilan bajo features —
  asegurarse de que ninguna compilación de producción los active.

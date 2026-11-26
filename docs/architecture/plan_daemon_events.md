# Plan — Eventos del demonio y datos en tiempo real

> Objetivo: eliminar el polling y los valores simulados en el lado de la UI exponiendo
> los eventos faltantes del demonio. Referencia: `rpc_protocol.md` §6.
>
> Regla transversal: todo nuevo evento pasa por `DaemonEvent`
> (`services/events.rs`) con `method()` + `topic()` explícitos, y el
> protocolo queda documentado en `rpc_protocol.md`.

## Fase E1 — Subidas en tiempo real (nivel 1) — ✅ entregada

### Demonio (Rust)

- [x] `DaemonEvent::UploadsUpdated { sequence, active, queue, upload_rate_bps }`
      (`UploadSessionSnapshot` en `services/upload_queue/`; unifica sesión + resumen activa/cola)
  - método `upload.uploadsUpdated`, tópico `uploads`
- [x] Emitir desde `UploadQueue`: inicio, progreso periódico (~500 ms), fin, cancelación, entrada/salida de cola
- [x] Añadir `upload_rate_bps` agregado a la instantánea de estado global
      (suma de los `upload_rate_bps` activos)

### Cliente / WebUI

- [x] Suscripción a `upload.uploadsUpdated` en la WebUI (`06_updates_notifications_scheduler.js`)
- [x] Actualización en vivo de `cachedUploadsActive` / `cachedUploadsQueued` y del modal de ancho de banda (`renderBandwidthModal`)
- [x] Caudal de subida conectado al estado global (`event.statsTick`)

## Fase E2 — Tick de estadísticas globales (nivel 2) — ✅ entregada

### Demonio

- [x] `DaemonEvent::StatsTick { sequence, stats }` — método
      `event.statsTick`, tópico `stats`, cadencia 1–2 s
  - payload: `downloadBps`, `uploadBps`, `peersConnected`,
    `downloadsActive`, `serverConnected`, `kadContacts`,
    `kadUdpBound`, `sessionDownloadedBytes`, `sessionUploadedBytes`,
    `lifetimeDownloadedBytes`, `lifetimeUploadedBytes`
    (acumulados «desde la instalación» persistidos en SQLite,
    reiniciados mediante `stats.resetLifetimeTotals`)
- [x] Temporizador en `state.rs` con suspensión si no hay suscriptores

### Cliente / WebUI

- [x] Suscripción a `event.statsTick` en la WebUI (`latestStatsTick` + actualización en vivo de ancho de banda)

## Fase E3 — Ciclo de vida de las descargas (nivel 3) — ✅ entregada

### Demonio

- [x] `DaemonEvent::DownloadAdded { sequence, download }`
      (`event.downloadAdded`, tópico `downloads`)
- [x] `DaemonEvent::DownloadRemoved { sequence, id }`
      (`event.downloadRemoved`)
- [x] `DaemonEvent::DownloadCompleted { sequence, download }`
      (`event.downloadCompleted`) — transición status→completed
- [x] `DaemonEvent::DownloadFailed { sequence, download, reason }`
      (`event.downloadFailed`)

### Cliente / WebUI

- [x] Manejadores `event.downloadAdded`, `event.downloadUpdated`, `event.downloadRemoved`, `event.downloadCompleted` y `event.downloadFailed` en `06_updates_notifications_scheduler.js`
- [x] Notificaciones/registros en vivo y refresco reactivo de descargas y biblioteca multimedia

## Fase E4 — Estados consultados por polling → eventos (nivel 4) — ✅ entregada

### Demonio

- [x] `kad.statusChanged` (tópico `kad`) — emitido al cambiar
      ID/contactos/buckets/estado UDP
- [x] `history.entryAdded` (tópico `history`)
- [x] `settings.changed` (tópico `settings`)
- [x] `network.policyChanged` (tópico `network`) — perfil, kill switch,
      presupuestos

### Cliente / WebUI

- [x] Suscripciones en la WebUI para `kad.statusChanged`, `history.entryAdded`, `settings.changed` y `network.policyChanged`

## Fase E5 — Detalle por par (nivel 5) — ✅ entregada

- [x] `download_rate_bps` en `DownloadSnapshot` y velocidades por par/fuente
- [x] Tabla de fuentes del detalle de descarga con velocidad en vivo
- [x] `etaSeconds` en `DownloadSnapshot` ligero (evita la llamada detail)

## Orden de implementación

E1 → E2 → E3 → E4 (E5 como extra). Cada fase = 3 commits:
demonio (Rust + pruebas), cliente y UI. Verificaciones: `cargo test` demonio.
`rpc_protocol.md` actualizado en cada fase (método, tópico, payload).

## Fase E6 — Completado de los contadores de barra lateral (post-E4) — ✅ entregada

Auditoría de las insignias de la barra lateral: todos los contadores ya eran reactivos
salvo dos, corregidos:

- [x] **`kad.statusChanged`** en el bucle de mantenimiento `kad.rs` (tick 30 s) cuando `total_contacts` cambia realmente — sin emisión en el primer tick para no intercalar un evento en las respuestas RPC en curso.
- [x] **`shares.changed`** (tópico `shares`) — notificación emitida por `handlers/shares.rs` en `add`/`remove`/`rescan`/`setEnabled`/`setUploadPriority`/`setCategory` y escuchada en la WebUI (`refreshShares()`).

# Plan — Eventos del demonio y datos en tiempo real

> Objetivo: eliminar el polling y los valores simulados en el lado de la UI exponiendo
> los eventos faltantes del demonio. Referencia: `rpc_protocol.md` §6.
>
> Regla transversal: todo nuevo evento pasa por `DaemonEvent`
> (`services/events.rs`) con `method()` + `topic()` explícitos, y el
> protocolo queda documentado en `rpc_protocol.md`.

## Fase E1 — Subidas en tiempo real (nivel 1) — ✅ entregada

### Demonio (Rust)

- [ ] `DaemonEvent::UploadSessionUpdated { sequence, session }`
      (`UploadSessionSnapshot` ya existe en `services/upload_queue.rs`)
  - método `upload.sessionUpdated`, tópico `uploads`
- [ ] `DaemonEvent::UploadsUpdated { sequence, uploads, queue }`
      (resumen lista activa + cola) — método `upload.uploadsUpdated`
- [ ] Emitir desde `UploadManager`: inicio, progreso periódico
      (1 s), fin, cancelación, entrada/salida de cola
- [ ] Añadir `upload_rate_bps` agregado a la instantánea de estado global
      (suma de los `upload_rate_bps` activos — cálculo ya realizado en
      `inbound_listener.rs:227`)

### Cliente (`mule_client` / WebUI)

- [ ] `Stream<UploadSessionModel> get uploadSessionEvents`
- [ ] `Stream<List<UploadSessionModel>> get uploadsEvents` (o instantánea)
- [ ] Cliente falso: simular una sesión de subida periódica

### Interfaz (UI)

- [ ] `UploadsController` (`view_models/uploads_controller.dart`) +
      `uploadsControllerProvider`
- [ ] `TransferRateTracker.uploadRate` alimentado por los eventos (reemplaza
      `() => 0`)
- [ ] Vista «Subidas» (nueva sección y ruta `/uploads`): sesiones activas
      (cliente, archivo, velocidad, estado) + cola de espera con rango/puntuación
- [ ] Insignia UL de la barra superior conectada al caudal real

## Fase E2 — Tick de estadísticas globales (nivel 2) — ✅ entregada

### Demonio

- [ ] `DaemonEvent::StatsTick { sequence, stats }` — método
      `event.statsTick`, tópico `stats`, cadencia 1–2 s
  - payload: `downloadBps`, `uploadBps`, `peersConnected`,
    `downloadsActive`, `serverConnected`, `kadContacts`,
    `kadUdpBound`, `sessionDownloadedBytes`, `sessionUploadedBytes`,
    `lifetimeDownloadedBytes`, `lifetimeUploadedBytes`
    (acumulados «desde la instalación» persistidos en SQLite,
    reiniciados mediante `stats.resetLifetimeTotals`)
- [ ] Temporizador en `state.rs` o servicio dedicado; suspensión si no hay
      ningún suscriptor al tópico `stats` (ahorro de CPU)

### Cliente

- [ ] `Stream<DaemonStatsModel> get statsEvents` + modelo `DaemonStatsModel`
- [ ] Fake: sinusoide/ráfaga simulada

### Interfaz (UI)

- [ ] `StatsController` exponiendo el último tick + `TransferRateTracker`
      alimentado por `statsEvents` cuando esté disponible (respaldo de muestreo local)
- [ ] Barra superior: insignias de servidor/Kad actualizadas por el tick
- [ ] Extensión futura: minipanel de inicio

## Fase E3 — Ciclo de vida de las descargas (nivel 3) — ✅ entregada

### Demonio

- [ ] `DaemonEvent::DownloadAdded { sequence, download }`
      (`event.downloadAdded`, tópico `downloads`)
- [ ] `DaemonEvent::DownloadRemoved { sequence, id }`
      (`event.downloadRemoved`)
- [ ] `DaemonEvent::DownloadCompleted { sequence, download }`
      (`event.downloadCompleted`) — transición status→completed
- [ ] `DaemonEvent::DownloadFailed { sequence, download, reason }`
      (`event.downloadFailed`)

### Cliente

- [ ] Streams tipados correspondientes; `DownloadItem` sin cambios (instantánea
      reutilizada en los eventos)

### Interfaz (UI)

- [ ] `downloads_store_controller`: aplicar Added/Removed sin diff
      de `listDownloads`
- [ ] `Completed`/`Failed` → notificación en bandeja/toast
- [ ] Contadores de la barra lateral reactivos de inmediato

## Fase E4 — Estados consultados por polling → eventos (nivel 4) — ✅ entregada

### Demonio

- [ ] `kad.statusChanged` (tópico `kad`) — emitido al cambiar
      ID/contactos/buckets/estado UDP
- [ ] `history.entryAdded` (tópico `history`)
- [ ] `settings.changed` (tópico `settings`)
- [ ] `network.policyChanged` (tópico `network`) — perfil, kill switch,
      presupuestos

### Cliente

- [ ] Streams: `kadStatusEvents`, `historyEvents`, `settingsEvents`,
      `networkPolicyEvents`

### Interfaz (UI)

- [ ] `KadController`: eliminar el temporizador de 3 s, escuchar
      `kadStatusEvents` (+ obtención inicial)
- [ ] `HistoryController`: insertar la entrada al recibir `entryAdded`
- [ ] `SettingsController`/`NetworkPolicyController`: aplicar los
      eventos (multicliente sincronizado)

## Fase E5 — Detalle por par (nivel 5) — ✅ entregada

- [ ] `download_rate_bps` por `PeerSessionSnapshot` (velocidad por fuente)
- [ ] Tabla de fuentes del detalle de descarga con velocidad en vivo
- [ ] `etaSeconds` en `DownloadSnapshot` ligero (evita la llamada detail)

## Orden de implementación

E1 → E2 → E3 → E4 (E5 como extra). Cada fase = 3 commits:
demonio (Rust + pruebas), cliente y UI. Verificaciones: `cargo test` demonio.
`rpc_protocol.md` actualizado en cada fase (método, tópico, payload).

## Fase E6 — Completado de los contadores de barra lateral (post-E4) — ✅ entregada

Auditoría de las insignias de la barra lateral: todos los contadores ya eran reactivos
salvo dos, corregidos:

- **`kad.statusChanged`** solo se emitía en los manejadores RPC —
  las mutaciones autónomas de la tabla (contactos aprendidos, desalojos)
  no emitían nada y la insignia solo se movía al abrir la
  sección. El bucle de mantenimiento `kad.rs` (tick 30 s) ahora emite
  el evento mediante `set_event_channel` cuando `total_contacts` cambia
  realmente — sin emisión en el primer tick para no intercalar
  un evento en las respuestas RPC en curso.
- **`shares.changed`** (tópico `shares`) — nueva notificación emitida por
  `handlers/shares.rs` en `add`/`remove`/`rescan`/`setEnabled`/
  `setUploadPriority`/`setCategory` (cubre las mutaciones RPC y MCP); nuevo
  stream `sharesEvents` en el lado del cliente (insignia «Compartidos» en vivo).

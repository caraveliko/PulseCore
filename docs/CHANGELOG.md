# Registro de cambios — eMuleRust

Todos los cambios notables de este proyecto se documentan en este archivo.
El formato se basa en [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/), y este proyecto se adhiere a [Semantic Versioning](https://semver.org/lang/es/).

## [Unreleased]

## [0.4.0-beta.3] - 2026-11-20

- **Sistema integral de notificaciones y centros de avisos (`mule-core`, `mule-daemon`, `mule-mcp`, `mule-assistant`, WebUI)**:
  - Persistencia SQLite dedicada (`persistence/sqlite/notifications.rs`) con centro histórico filtrable (`notifications.list`, `markRead`, `clear`, `getConfig`, `updateConfig`, `testChannel`) y empuje en tiempo real (`event.notificationCreated`).
  - Monitorización automática (`services/notifications/monitor.rs`) ante descargas completadas, errores críticos, recuperación de huérfanos, actualizaciones disponibles y capturas multimedia.
  - Canales push multiplataforma protegidos contra SSRF por `NetworkPolicyManager`: Webhook genérico, Telegram Bot API, Discord, Gotify, ntfy y Pushover (secretos enmascarados en RPC y persistidos con redacción segura).
  - Centro interactivo en la WebUI (`web/index.html`) con campana animada, filtro por estado/severidad, toasts no intrusivos y configuración de canales.
  - Seis herramientas MCP (`emule_list_notifications`, `emule_mark_notifications_read`, `emule_clear_notifications`, `emule_get_notifications_config`, `emule_update_notifications_config`, `emule_test_notification_channel`) integradas en `ToolProfile` (`read|safe|full`) y en `mule-assistant`.
- **Endurecimiento Kad DHT para WAN (`mule-protocol`, `mule-core`, `mule-daemon`)**:
  - Códec `KADEMLIA_FINDBUDDY_REQ` (`0x51`) y cliente saliente LowID Buddy (`services/kad/buddy_client.rs`): descubrimiento del buddy, enlace TCP con ofuscación RC4 opcional (`OP_HELLO`/`OP_HELLO_ANSWER`), latidos periódicos (`OP_BUDDYPING`/`OP_BUDDYPONG`) y respuesta ante `OP_CALLBACK`.
  - Protección global anti-Sybil en `KBucketTable` (`MAX_GLOBAL_CONTACTS_PER_IP = 1`, `MAX_GLOBAL_CONTACTS_PER_SUBNET_24 = 10`, exención loopback) y revalidación LRU de contactos más antiguos.
  - Limitador de caudal por opcode y dirección IP en `kad_udp`, emparejamiento estricto de transacciones UDP (`find_matching_tx_id` IP+puerto con respaldo NAT), codificación estándar de rechazo en `PublishRes` (`u8::MAX`) y límite de carga aceptada `<= 100`.
  - Publicación de palabras clave por lotes (`publish_keywords_batch`: hasta 150 archivos por palabra clave y 50 entradas por paquete con cadencia controlada), *self-lookup* periódico (`NODECOMPLETE` cada 4 h) y *jitter* de republicación (`<= 300 s`).
- **Arneses de pruebas consolidados y corredor inteligente (`scripts/smart_test.py`)**:
  - Agrupación de pruebas de integración en arneses unificados (`assistant_suite`, `cli_suite`, `core_suite`, `daemon_suite`, `daemon_ed2k_suite`, `daemon_lab_suite`, `filter_suite`, `format_suite`, `mcp_suite`, `protocol_suite`, `protocol_ed2k_suite`, `protocol_lab_suite`) para eliminar cuellos de botella de enlazado en `target/debug`.
  - Selector inteligente `--mode smart|fast|full` con comprobación sintáctica de la WebUI, detector B1 de duplicados (`find_duplicates.py`) y pruebas afectadas por Git.
- **Limpieza del repositorio y unificación de documentación en español**:
  - Traducción íntegra al español de España de los scripts auxiliares (`find_duplicates.py`, `dup_allowlist.txt`, `test_duplicates.ps1`, `test_phase_b_functional_bench.ps1`, `test_phase_d_public_network.ps1`) y de `mule-assistant`, actualización de `docs/INDEX.md`, `docs/architecture/rpc_protocol.md`, `docs/architecture/rapport_architecture_et_avancement.md` y la Wiki.

- **Sistema híbrido de actualización Linux / `systemd`, rollback O(1), CLI y WebUI**:
  - Endurecimiento de `UpdaterService` frente a la política de red (`NetworkPolicyManager`, interruptor de corte, aislamiento `LocalLab` y redirecciones 302 validadas contra SSRF), descarga oficial en *streaming* con progreso real, reporte de entorno (`platformTriple`, `managedBySystemd`, `canSelfUpdate`, `hasRollbackBackup`) y manejo claro de HTTP 404.
  - Nuevo endpoint autenticado `POST /api/system/upload-update` (`Bearer <token>`, límite 64 MiB) para instalar binarios locales compilados con validación estricta de cabecera ELF64/PE y arquitectura (`x86_64` frente a `aarch64`).
  - Respaldo O(1) mediante enlace duro `.old` en Unix y método JSON-RPC `system.rollbackUpdate` para revertir al binario anterior con validación previa de cabecera.
  - Sincronización `systemd` (`ReadWritePaths=... %h/.local/bin`, `RestartSec=2`, liberación explícita de `DirectoryLock` y salida ordenada bajo `INVOCATION_ID`/`NOTIFY_SOCKET`).
  - Nuevo subcomando `mule-cli update` (`status`, `check`, `upload`, `rollback`), script `scripts/update_linux.sh` (`--local`, `--rpi`, `--binary`, `--install-service`, `--ssh`, `--rollback`) e indicador inteligente en cabecera + modal Drag & Drop en `web/index.html`.
- **Política estricta «Solo Vídeo» (ADR 0006)**:
  - Fuente única de verdad en `mule_network_policy::content_policy` aplicada en admisión eD2K/Kad/Torznab, inspección de cabecera en vuelo (MD4 parte 0), importación `.part.met`, recuperación de huérfanos, compartidos y filtrado selectivo en torrents BitTorrent (admitiendo únicamente contenedores de vídeo válidos y subtítulos acompañantes dentro de torrents con vídeo).

Campaña de validación previa a la publicación (puertas QA A–D):

- **Cambio de marca completo a eMuleRust** (`e86cc01`): nombre mostrado, ejecutable, paquetes,
  identificadores `emulerust*`, variables de entorno `EMULERUST_*` (alias
  obsoletos `EMULE_SION_*` conservados + prefijo UPnP heredado `eMule-Rust`
  reconocido para concesiones existentes).
- **Interoperabilidad en vivo Tier 3 frente a las 3 referencias reales** (`aab2ac9` +
  adaptadores): 7/7 frente a eMule 0.50a, 7/7 frente a eMule 0.72a, 7/7 frente a aMule
  3.1.0 — SHA-256 fijados, configuraciones herméticas, loopback estricto.
- **Anti-SSRF en `servers.updateFromUrl` / `kad.bootstrapFromUrl`**
  (`fe729ed`): validación del esquema http/https + rechazo de las IP resueltas
  prohibidas por la política (enlace local, privada, pública fuera de perfil).
- **Cierre del demonio acotado** (`d0ed8bf`): desmapeo UPnP separado en
  `std::thread` + oneshot de 500 ms — sin bloqueos >5 s en pasarelas
  lentas (medido: ~100 ms al detener con mapeos activos).
- **Banco funcional Fase B** (`42bd5ed`): 134 llamadas RPC que cubren la
  superficie pública + rutas de error, 134 PASS / 0 FAIL sobre paquete
  QA loopback.
- **Banco de red real Fase D** (`8b932a3`):
  `test_phase_d_public_network.ps1` — 21 PASS / 0 FAIL frente a la red
  real (`server.met` real, sesión eD2K establecida, 95 contactos Kad,
  búsquedas reales, `disconnectAll`, desmapeo UPnP limpio).
- **Fixtures de prueba compartidos** (`be66b9b`): helpers entre crates
  movidos a `mule-test-support`, 8 grupos de duplicados eliminados.
- **Prueba de humo empaquetada** (`7a8a3d7`): drenaje continuo stdout/stderr del
  demonio (bloqueo de tubería ~64 KiB) + prueba de caída E2E determinista mediante
  `downloads.pause` (`63430e8`).

Capa de IA local: un modelo loopback (Ollama/llama.cpp vía `/api/chat`)
controla el cliente a través del registro de herramientas MCP existente — sin
duplicación de lógica de negocio, `mule-daemon` sigue siendo la autoridad de seguridad.

- **Pipeline de escritura en disco eD2K optimizado** (`mule-core`,
  `mule-daemon`, `mule-format`) — inspirado en las observaciones de E/S de
  libtorrent/rqbit:
  - **Staging en RAM de las partes** (`downloads/staged.rs`, modelo eMule
    `m_tData`): los bloques recibidos alimentan un búfer acotado por parte
    (`MAX_STAGED_PARTS` = 4, ≈ 37 MiB/descarga); la verificación MD4 al
    completarse se realiza en memoria — sin relecturas de 9,28 MiB por
    parte (`pread` ≈ 0 en recepción). Liberación al completarse,
    al desalojarse, en la recuperación AICH y al finalizar.
  - **Verificación final sin relectura**: `try_verified_root` deriva
    la raíz eD2K del hashset de partes ya verificadas
    (`ed2k_root_from_part_hashes`, mismas reglas que `Ed2kHasher` — parte
    vacía de eMule en múltiplos exactos); respaldo en `hash_reader` cuando el
    hashset es desconocido o contradictorio. Elimina ~`total_size` de lectura
    por descarga.
  - **E/S síncronas fuera del executor Tokio**: `inject_range_brief`
    (rutas comprimida y no comprimida), el volcado periódico
    (`flush_dirty_downloads` + `decay_stale_rates`) y
    `save_active_part_mets` pasan por `spawn_blocking`/`blocking_lock`
    — fin del posible bloqueo del executor bajo presión de disco.
  - **`received_ranges` en delta O(1)**: `append_received_range`
    (upsert `MAX(end_offset)`) sustituye el DELETE+INSERT global por bloque
    en el hot path; la reescritura completa pasa a ser una compactación con
    cadencia `PART_PERSIST_INTERVAL`.
  - **E/S posicionadas**: `PartFile::write_segment`/`read_range` mediante
    `seek_write`/`seek_read` (bucles contra escrituras/lecturas
    parciales), sin `seek` mutable sobre el descriptor compartido;
    contadores `PartFileIoStats` (ops/bytes leídos, escritos, fsync) registrados al
    completarse.
  - **Pragmas SQLite**: `busy_timeout` 5 s, `temp_store=MEMORY`,
    `cache_size` 20 MiB, `wal_autocheckpoint` ~32 MiB.

- **Perfiles de filtrado de herramientas MCP** (`mule-mcp`):
  - `ToolRisk { Read, Mutate, Destructive }` + `ToolProfile { read, safe,
    full }` clasificando las ~87 herramientas `emule_*` por verbo (con tabla
    de excepciones para las ambiguas); `--tool-profile` en CLI, `full` por
    defecto (retrocompatible).
  - Filtrado aplicado en **ambas** fronteras: `tools/list` filtrada *y*
    `tools/call` rechaza las herramientas fuera de perfil — un cliente externo no puede
    eludir una lista de presentación.
  - Prueba de exhaustividad: cualquier nueva herramienta sin clasificar rompe la compilación.

- **Nuevo crate `mule-assistant`** — orquestador LLM local:
  - Bucle agéntico acotado (`MAX_TOOL_ROUNDS`): `chat → tool_calls →
    policy → execute_tool → role:"tool" → repeat`; el mensaje del asistente
    completo (con sus `tool_calls`) se conserva en el historial antes de
    los resultados, conforme al formato nativo de Ollama (`tool_name`, ids
    internos opcionales).
  - **Política**: herramientas visibles = `list_tools ∩ allowlist curada ∩
    capacidades del demonio (tabla prefijo→categoría) ∩ perfil MCP`; recomprobada
    justo antes de cada ejecución, confirmación del usuario sobre argumentos
    exactos para las herramientas `Destructive`.
  - **Backend Ollama**: POST `/api/chat` en streaming, endpoint validado —
    HTTP, dirección IP literal **loopback únicamente** (`127.0.0.1`,
    `::1`, rechazo de nombres a resolver), redirecciones desactivadas
    (`redirect::Policy::none()`), proxies de entorno desactivados,
    timeouts configurados; `load_duration`/`eval_duration` registrados.
  - **Streaming**: `AssistantEvent` (`token`/`toolStart`/`toolResult`/
    `final`/`notice`/`error`) — tokens mostrados en tiempo real, llamadas
    a herramientas ensambladas, *luego* validadas y *luego* ejecutadas; `thinking` nunca
    expuesto; resultados de herramientas truncados para el LLM, completos para la UI.
  - **Transporte WS loopback** (`--serve`): JSON-RPC `assistant.hello`
    (token de sesión del demonio — sin autenticación paralela), `assistant.chat`,
    `assistant.confirm`, `assistant.cancel`; eventos como notificaciones,
    drenaje de eventos en cola al final del turno.
  - REPL (`chat`, `--prompt`) y `MockBackend` para pruebas 100 % sin conexión.

- **Panel Flutter** (`app_shell`, MVVM, histórico):
  - `AssistantClient` (`mule_client`): cliente JSON-RPC/WS del protocolo
    asistente; modelos `AssistantEvent`/`AssistantConfirmRequest`/`ChatEntry`.
  - `AssistantController`: estado de conversación, tokens en streaming,
    confirmación, cancelación; token resuelto desde `<workdir>/auth.token`
    mediante `assistant_token(_io|_stub)` — no disponible en web.
  - Sección «Asistente de IA» (ruta `/assistant`, barra lateral): burbujas de
    chat, tarjetas de herramientas, banner de confirmación, estado sin conexión con
    reintento; solo design tokens.

- **Escritorio Windows — bandeja del sistema e icono de barra de tareas** (`apps/desktop`,
  `app_shell`, histórico):
  - **Cierre hacia la bandeja del sistema** (comportamiento típico de cliente
    P2P): el botón de cierre / Alt+F4 oculta la ventana en la
    bandeja del sistema en lugar de salir — la aplicación, el estado y
    `mule-daemon` permanecen activos.
  - `setPreventClose` + nuevo coordinador `AppQuit`
    (`app_shell/lib/core/app_quit.dart`, Dart puro): el manejador
    `onExitRequested` responde `cancel` y activa la ocultación mediante
    `AppQuit.onHideToTray`. Necesario porque el motor Flutter consume
    `WM_CLOSE` para su mecanismo de `exit-request` — el evento `close` nunca
    llega a los listeners de `window_manager`.
  - Menú de bandeja localizado «Mostrar eMuleRust» / «Salir»;
    clic izquierdo o doble clic en el icono restaura la ventana con el
    foco; «Salir» levanta la intercepción y pasa por un cierre
    clásico — la parada ordenada del demonio (`supervisor.stop()` y luego
    salida) permanece en un único lugar.
  - Notificación única en el primer cierre indicando que la aplicación
    continúa en segundo plano (bandera `SharedPreferences` persistida).
  - **Icono genérico de barra de tareas corregido**: `local_notifier` declara
    el AUMID `eMuleRust`, que la barra de tareas resuelve mediante el acceso directo
    `eMuleRust.lnk` del menú Inicio — pero WinToast nunca recrea un
    acceso directo existente: tras mover el paquete portátil, el destino
    roto mostraba el icono genérico. `DownloadNotifier` elimina
    ahora el `.lnk` cuando no contiene la ruta del
    ejecutable actual (búsqueda ASCII y UTF-16LE), dejando que se
    recree con el destino correcto en cada arranque.

## [0.2.0-RC2] - 2026-09-25

Segunda release candidate — campaña completa de auditoría de red y comportamiento
frente a las fuentes oficiales de eMule 0.72a, validada mediante
pruebas de interoperabilidad en vivo frente al binario real.

- **Interoperabilidad en vivo con eMule 0.72a validada — 7/7 pruebas** (`mule-protocol`):
  el arnés Tier-3 (`EMULE_REFERENCE_BIN` + `EMULE_REFERENCE_PROFILE`)
  lanza el verdadero `emule.exe` 0.72a (SHA-256 fijado) en perfil aislado
  loopback y ejecuta handshake, estado de archivo, transferencias de bloques,
  reanudación y finalización — todas en verde frente al binario oficial.
  - **`OP_COMPRESSEDPART` (0x40) de 32 bits**: decodificador acotado
    (`<hash16><start u32><compressed_size u32><zlib>`) + codificador
    simétrico bajo `PR_EMULE` (`OP_EMULEPROT`, fiel a
    `UploadDiskIOThread.cpp`). Corrección de un fallo latente: el codificador
    I64 emitía `PR_EMULE_COMPRESSED` (0xD4) en lugar de `PR_EMULE`.
  - **Reensamblado zlib incremental** en el cliente loopback:
    acumulador por `(hash, inicio de bloque)` — eMule divide el flujo
    comprimido en tramos de ≤ 10 240 bytes transportados en varios
    paquetes; subrangos validados en el rastreador de bloques.
  - **Fallo de flujo corregido** en `receive_sending_part`: tras un
    paquete comprimido, el código caía en el `read()` bloqueante en lugar
    de drenar las partes ya producidas — interbloqueo en cualquier transferencia
    comprimida multipaquete.
  - Perfil de referencia parametrizable: `EMULE_REFERENCE_PROFILE`
    (`Emule<VV>a` → `client_version` esperado derivado), `AppVersion`
    inyectada en el `preferences.ini` generado para omitir
    el asistente modal de primer arranque.

- **Antiabuso alineado con eMule 0.72a** (`mule-daemon`):
  - **`CLIENTBANTIME` = 2 h**: los bloqueos de fuentes IP expiran como
    `IsBannedClient` (`ClientList.cpp`) en lugar de ser permanentes.
  - **`BADCLIENTBAN`**: `AddRequestCount` (re-ask del mismo archivo <
    `MIN_REQUESTTIME` = 10 min, 4 avisos) produce ahora un bloqueo
    IP real de 2 h + corte de la conexión (eMule `Ban()` →
    `AddBannedClient`), no solo puntuación 0 — exención
    `GetFriendSlot` respetada.
  - **`ComparePriorUserhash`**: las parejas `(IP, puerto)` se rastrean 2 h
    (`KEEPTRACK_TIME`, registradas en la verificación SUI) — una IP:puerto
    vista de nuevo con un userhash distinto se bloquea y desconecta.
  - **Filtrado UDP de pares**: `ipfilter` + puerto 0 aplicados a cada
    datagrama de par (`ClientUDPSocket.cpp:76`) y bloqueados rechazados;
    el listener TCP comprueba la IP bloqueada al aceptar y tras el hello.

- **Cobertura exhaustiva de opcodes** (`mule-protocol`, `mule-daemon`) —
  revisión mecánica de los ~95 `case OP_*` de eMule (`ListenSocket`,
  `ClientUDPSocket`, `UDPSocket`, `ServerSocket`): todo cubierto a
  excepción de `OP_PREVIEWANSWER` (anunciamos preview=0 y
  nunca emitimos `OP_REQUESTPREVIEW`, descarte silencioso fiel):
  - **Exploración de compartidos**: `OP_ASKSHAREDFILES{,ANSWER}`,
    `OP_ASKSHAREDDIRS{,ANSWER}`, `OP_ASKSHAREDFILESDIR{,ANSWER}`.
  - **Veredicto cortafuegos Kad TCP**: `OP_FIREWALLED{2}_REQ` saliente +
    veredicto `0xA8` entrante.
  - **Descripción de servidor**: `OP_SERVER_DESC_REQ/RES` (Lugdunum).
  - **`OP_PUBLICIP_REQ`** entrante + **`OP_OUTOFPARTREQS`** saliente en
    rotación de slot.
  - **`OP_PACKEDPROT` par (0xD4)**: desempaquetado entrante + respuestas de
    fuentes empaquetadas; Kad UDP empaquetado (0xE5).
  - **Callbacks LowID**: `OP_PORTTEST` TCP, relé buddy
    `OP_REASKCALLBACKTCP`/`OP_CALLBACK` (comprobación `~kad_id`), y
    `OP_DIRECTCALLBACKREQ` saliente con re-ask diferido
    (`m_bReaskPending`).
  - **Hashsets `FileIdentifier`**: `OP_HASHSETREQUEST2`/`ANSWER2`
    (identificador compuesto + opciones MD4/AICH, despacho bajo `PR_EMULE`).
  - **Rango de cola**: `OP_QUEUERANK` heredado (0x5C) entrante +
    corrección `OP_QUEUERANKING` emitido en `PR_EMULE` con el payload
    eMule de 12 bytes (`[rank u16][10 ceros]`) en lugar de `PR_ED2K`/u32.
  - **Hello connect-back**: `misc_options2` completo + puertos de prueba
    fuera de rango eMule.
  - **Opcodes 0x49/0x4D** (fase B): handshake, peticiones independientes y
    reasks alineados con `ListenSocket.cpp`/`ClientUDPSocket.cpp`.

- **Cola de subida persistente ante desconexiones** (`mule-daemon`):
  eMule `Disconnected` (`BaseClient.cpp:1169`) conserva un cliente en
  cola (`US_ONUPLOADQUEUE` → sin eliminación) hasta
  `MAX_PURGEQUEUETIME`. Ahora un cierre de socket conserva la entrada
  (`mark_disconnected`, puntuación y espera intactas) mientras que los cierres
  explícitos (`CANCELTRANSFER`, `END_OF_DOWNLOAD`, RPC) la retiran;
  el mejor inalcanzable que supera al mejor conectado se marca
  `add_next_connect` (`FindBestClientInQueue`) y su reconexión abre
  un slot inmediato (`AddClientToQueue`).

- **Comportamiento de descarga** (`mule-daemon`):
  - **Purga `RemoteQueueFull`** de las fuentes al 80 % de `maxSources`
    (`PartFile.cpp`).
  - **A4AF a nivel de socket**: `SwapToAnotherFile`/`DoSwap` sobre el
    socket en vivo.
  - **Ciclo re-ask de cola remota fiel**: gestión del socket y
    ventana UDP `(1 s, 2 min)` previa al re-ask TCP.

- **Kad alineado con eMule 0.72a** (`mule-protocol`, `mule-core`):
  expresiones de búsqueda restrictivas (`StartSearchTermsMatch`),
  publicación de palabras clave AICH, modelo `CKeyEntry`, identidad eMule
  0.72a, protocolo de red y constantes realineados.

- **Migración eMule completa** (`mule-format`, `mule-daemon`,
  `app_shell`):
  - **`known.met`**: analizador + importación.
  - **`clients.met`**: importación de créditos de pares hacia `peer_credits`.
  - **`emfriends.met` + `cancelled.met`** + identidad eMule
    (`cryptkey` + `userhash`).
  - **Asistente de migración** en Ajustes → Avanzado.
  - **Búsqueda cancelada**: marcado `IsCancelledFileByID` + insignia
    «Cancelado» en los resultados.

- **Refactorización — compartición de pruebas** (`mule-test-support`,
  `mule-protocol`): helpers de integración entre crates en
  `mule-test-support`, fixtures de protocolo comunes
  (`tests/common/`), salvaguarda antiduplicados
  (`scripts/test_duplicates.ps1`).

- **Prioridades de descarga/subida alineadas con eMule 0.72a** (`mule-daemon`,
  `app_shell`):
  - **Prioridad «auto» descarga** (`PR_AUTO` = 5): aceptada por
    `downloads.setPriority`, persistida en BD y en el `.part.met`
    (`FT_DLPRIORITY`, importación `PR_AUTO` ↔ `"auto"` — una prioridad
    automática importada ya no se convierte en manual). El peso
    efectivo se recalcula en cada tick del coordinador según
    `CPartFile::UpdateAutoDownPriority`: más de 100 fuentes conocidas →
    low, más de 20 → normal, si no → high; alimenta el rebalanceo
    A4AF (`ProcessA4AFClients`). Las prioridades manuales ignoran el
    recuento de fuentes.
  - **Prioridad «auto» subida**: aceptada por
    `shares.setUploadPriority`; un registro por archivo recalcula la
    prioridad efectiva según `CKnownFile::UpdateAutoUpPriority` en cada
    entrada/salida de uploader (más de 20 uploaders → low = 6, más de
    1 → normal = 7, si no → high = 9). Las puntuaciones de cola se
    refrescan en cada ordenación (espera × créditos × prioridad efectiva) —
    concesiones directas, cola, reasks, promociones, rotación y publicación
    al servidor usan todos el valor efectivo.
  - **Preferencias eMule**: `autoPriorityNewDownloads` (eMule
    `DAPPref`) y `autoPriorityUploads` (`UAPPref`), ambas `true`
    por defecto como eMule — las nuevas descargas y compartidos
    arrancan en modo auto. Una prioridad importada desde un
    `.part.met` prevalece sobre el valor global por defecto.
  - **UI**: opción «Auto» en el selector de prioridad de
    descargas y compartidos; interruptor
    `autoPriorityUploads` en Ajustes.
  - Regresiones cubiertas: umbrales `UpdateAutoDownPriority` límites
    exactos (20/21/100/101), `UpdateAutoUpPriority` transiciones (1/2/20/
    21 uploaders), manual desactiva auto, roundtrip `.part.met`
    `FT_DLPRIORITY`/`PR_AUTO`.

- **Alineación swarm con eMule 0.72a** (`mule-daemon`):
  - **Reconsulta de fuentes por canal** (en lugar de la ventana
    adaptativa 30 s/60 s/300 s): TCP `OP_GETSOURCES` cada
    `SERVERREASKTIME` = 15 min si fuentes < `GetMaxSourcePerFileSoft`
    (90 % del tope, ≤ 750); UDP `OP_GLOBGETSOURCES2` cada
    `UDPSERVERREASKTIME` = 30 min si fuentes < `GetMaxSourcePerFileUDP`
    (75 % del tope, ≤ 50); Kad `search_sources` a `KADEMLIAREASKTIME` = 1 h
    × `m_TotalSearchesKad` (backoff hasta 7 h), acotado por
    `KADEMLIATOTALFILE` = 5 búsquedas simultáneas y `KADEMLIAASKTIME`
    = 1 s entre inicios.
  - **Eliminación del asignador water-filling**: sin cuota por
    archivo — cada archivo conecta sus fuentes viables hasta su
    tope (`FT_MAXSOURCES` / `max_peers_per_download`), desempeñando el límite
    global `max_global_download_peers` el papel de
    `maxconnections` (`TooManySockets`). `swarm_allocator.rs` eliminado;
    `min_peers_per_download` solo sirve ya para el rebalanceo A4AF.

- **Observabilidad local completa** (`mule-daemon`, `mule-mcp`,
  `app_shell`, `mule_client`, `ui_kit`):
  - **Registros rotativos estructurados**: `data/logs/daemon.<date>.log` en
    JSONL (rotación diaria, 7 archivos retenidos, escritor
    no bloqueante `tracing-appender`); consola legible conservada;
    el antiguo `daemon.log` plano se elimina al arrancar.
  - **Captura de panics**: hook `std::panic` → `logs/crash.log`
    síncrono + `tracing::error!`.
  - **Nivel de log en caliente**: `settings.set` acepta `logLevel`
    (nivel simple o especificación `EnvFilter` completa), recargado sin
    reiniciar y nunca persistido; expuesto vía `settings.get` /
    `system.status` y mediante un selector Ajustes → Avanzado.
  - **Spans RPC**: `#[instrument]` en el despacho (`rpc.method`) y
    los manejadores de descargas (`download_id`, hash truncado).
  - **`mule-mcp`**: capa de archivo `logs/mcp.<date>.log` (misma rotación),
    stdout reservado para JSON-RPC.
  - **Higiene PII**: ~200 sitios de log convertidos — direcciones de
    pares/servidores seudonimizadas de forma determinista por sesión
    (`peer-NN:port`, `server-NN:port`, `peer-NN` para IPs desnudas) mediante
    `SessionPseudonymizer` (`mule-filter`); loopback legible en claro.
    Helpers `log_peer_addr` / `log_server_addr` / `log_addr_str` /
    `log_peer_ip` + nuevo `pseudonymize_ip`. Script de auditoría
    reutilizable `scripts/dev_audit_addr_logs.py`.
  - **Flutter**: `AppLogger` (`ui_kit`) — niveles, búfer circular de 500
    entradas, `debugPrint` solo en dev, hooks `FlutterError.onError` /
    `PlatformDispatcher.onError`; conectado en los `catch` de los
    controladores; exportación de diagnósticos (contadores en anonimizado, 50
    últimas entradas en completo).

- **Verbosidad en producción reducida** (`mule-daemon`): ~61 sitios
  `info` → `debug` en las rutas por paquete/por bloque — recepciones y
  envíos Kad (`kad_udp`, `routing_client`, `search_client`), bloques
  subida/descarga, log por petición RPC (polling UI), intentos de
  conexión del planificador por tick y resúmenes `[SOURCE_SCHED]`. El warn
  de fin de sesión de par se clasifica por gravedad: `HashMismatch` /
  `InvalidRange` / `CorruptedData` siguen en `warn`, la rotación normal
  (timeouts, rechazados, `NoNeededParts`, `IpFilter`…) pasa a `debug`.
  Permanecen en `info`: ciclo de vida (conexiones, handshakes, slots
  subida), hitos de descarga, buddy/callback Kad, failover de servidor,
  descartes de IP-filter, latido consolidado (1×/60 s).

- **Compartidos — escaneo recursivo de la carpeta Incoming** (`mule-daemon`):
  los archivos en subcarpetas del directorio de descargas
  se comparten ahora (semántica eMule); enlaces simbólicos y carpetas
  ocultas ignorados. Regresión: `test_shares_rescan_is_recursive`.

- **IP filter — correcciones RPC** (`mule-daemon`):
  `filter.importFile` devuelve ahora la misma forma `FilterStatus`
  que `getStatus`/`setEnabled`/`clear` (el cliente Dart analizaba el
  envoltorio `{success, report, stats}` como estado plano → la importación
  parecía no tener efecto); restauración al arrancar de `ipfilter_enabled=0`
  persistido (el valor por defecto `true` reactivaba el filtro al reiniciar).
  Regresión cubierta por `filter_rpc_tests.rs`.

- **Branding**: `EMule-Rust` → `eMuleRust` en todo el proyecto (~122 archivos:
  UI, metadatos Windows, docs, scripts). Identificadores técnicos
  conservados (`eMuleRust.exe`, `emulerust`, `PR_EMULE`…);
  compatibilidad UPnP con el antiguo prefijo de descripción conservada
  (`LEGACY_DESCRIPTION_PREFIX`).

- **UI descargas — barra superior armonizada** (`app_shell`): el título
  de filtro «Todas las descargas» se retira (redundante con la
  barra lateral y la pista de búsqueda); el campo de búsqueda ya no tiene
  tamaño fijo — ocupa 2/10 del espacio libre vía `Expanded(flex: 2)`
  mientras el grupo de acciones conserva 8/10 y solo hace scroll en
  ventana muy estrecha; alturas armonizadas a 36 px (búsqueda,
  monitor de transferencia, botón Añadir). Efecto ripple translúcido de las filas
  de descarga eliminado (splash/highlight `InkWell` desactivados,
  selección conservada).

- **Paleta estándar de la barra de partes** (`ui_kit`): los tonos
  pastel Tailwind se sustituyen por colores planos estándar —
  azul `#2196F3` recibido, verde `#4CAF50` verificado/progreso, rojo
  `#F44336` sin fuente, amarillo `#FFEB3B` bloque en vuelo y la fórmula de
  azul auténtica de eMule `RGB(0, 210-22·(n-1), 255)` para
  raro→común. Compartido con la tarjeta part-map.

- **Compilación — despliegue automático** (`scripts`): el paso Deploy de
  `build_windows_release.ps1` resuelve su destino vía `-DeployDir` >
  `$env:EMULERUST_DEPLOY_DIR` > archivo `.deploy-dir` en la raíz
  (gitignoreado, ruta de máquina); los procesos lanzados desde el destino
  se detienen antes de la copia (exe bloqueados); la copia combinada
  preserva `data\`, `downloads\` e `ipfilter*`.

- **Compartidos — purga de archivos eliminados del disco** (`mule-daemon`):
  un archivo borrado físicamente en la carpeta de descargas
  seguía listado en Compartidos (la fila `shared_files` sobrevivía — el
  escaneo Incoming solo añadía entradas). Ahora
  `list_all_shared_files` filtra en lectura los compartidos cuyo
  archivo ya no existe (ni UI, ni oferta al servidor, ni publicación Kad
  anuncian un archivo muerto), la descarga completada aún en memoria
  aplica la misma guarda `is_file()`, y `scan_download_dir_shares`
  elimina definitivamente las entradas obsoletas bajo la carpeta Incoming
  — únicamente si esta es accesible (una unidad desconectada no vacía
  la lista) y sin tocar los compartidos fuera de la carpeta. `shares.rescan`
  devuelve `{added, removed, dir}`. `get_shared_file` aplica la misma
  guarda (el intercambio de fuentes ya no se declara fuente de un archivo
  ausente). Regresión:
  `test_shares_rescan_prunes_files_deleted_from_disk`.

- **Descargas — eliminación del `.part` físico al arrancar** (`mule-daemon`):
  un `.part` borrado mientras el demonio estaba detenido dejaba los
  rangos persistidos en BD afirmando datos perdidos (archivo recreado
  vacío, ceros servidos hasta el desalojo ICH). Al arrancar, toda
  descarga no completada cuyo `.part` haya desaparecido — carpeta padre
  accesible, luego eliminación real y no unidad sin conexión — ve
  retirada su entrada (`delete_download` + `.part.met` huérfano limpiado,
  evento de historial «removed»). Regresión:
  `missing_part_file_removes_download_at_startup`.

- **Cola de subida — alineación con eMule 0.72a** (`mule-daemon`): auditoría
  completa frente a `UploadQueue.cpp`/`UploadClient.cpp` y correcciones de las
  divergencias de comportamiento:
  - **Número dinámico de slots** (`AcceptNewClient`): el tope ya no
    es un 100 fijo — más allá de los 4 slots garantizados
    (`max(MIN_UP_CLIENTS_ALLOWED,4)`), solo se abre un nuevo slot
    si el caudal medido aún alimenta a cada cliente por encima del mínimo
    (`datarate / minTarget`, objetivo por cliente ≤3 slots → 3 KiB/s si no
    `slots × 1 KiB/s` limitado a 25 KiB/s) **y** si la capacidad de subida
    configurada lo permite (`MaxSpeed / target` — sonda leída en vivo sobre
    `upload_bucket.rate_per_sec()`, por lo que tanto el límite `limits.set` como el
    valor controlado por USS se aplican). Antes: hasta 100 slots
    simultáneos → caudal por cliente diluido y cola nunca llena.
  - **`Process()` periódico** (`process_tick`, 1 s, independiente de los
    suscriptores UI): purga de entradas sin re-ask desde >1 h
    (`MAX_PURGEQUEUETIME`), promoción de los mejores clientes en cuanto un
    slot puede abrirse, y **rotación de sesiones** — un cliente que haya
    recibido más de `SESSIONMAXTRANS` (~9,7 MiB) vuelve a la cola cuando
    ningún slot puede abrirse (`CheckForTimeOver` + `ForceNewClient`);
    su inicio de espera original se restaura en su re-ask (`rotation_grace`,
    equivalente al `credits` de eMule — la puntuación acumulada sobrevive a la
    rotación).
  - **Promoción push**: el cliente promovido recibe `OP_ACCEPTUPLOADREQ`
    de inmediato (`AddUpNextClient` de eMule escribe en el socket en
    espera) mediante un `Notify` en la entrada de cola — antes, la
    promoción quedaba latente hasta el siguiente re-ask del cliente (~30 min).
  - **Una entrada por cliente**: deduplicación solo por `client_key` (eMule: un
    objeto cliente = una entrada); el re-ask actualiza archivo/prio/créditos
    y refresca `last_request`. Un cliente activo que solicita otro
    archivo conserva su slot (`IsDownloading` → `ACCEPTUPLOADREQ` +
    `SetUploadFileID`) en lugar de volver a la cola.
  - **Límites suave/duro**: `hard = soft + max(soft,800)/4`; entre ambos,
    solo los clientes con slot amigo o cuyo
    `combined(prio × créditos)` supera la media de la cola son admitidos
    (sustituye la regla anti-leecher 90 %, no presente en eMule).
  - **Orden de cola recalculado bajo demanda**: ordenación por `effective_score`
    vivo antes de la promoción, rango reportado y tick (eMule evalúa
    `GetScore` de forma continua, no en la inserción).
  - **Antiabuso**: `MIN_REQUESTTIME` (10 min) / `BADCLIENTBAN` (4) —
    los re-asks agresivos del mismo archivo bloquean al cliente (puntuación 0).
  - **`PR_POWER`** (powershare) → `GetFilePrioAsNumber` = 7 (rama
    `default` de eMule) en lugar de 18.
  Regresiones: 8 nuevas pruebas `upload_queue` (tope `MaxSpeed`,
  cambio de archivo, entrada única, rotación + espera preservada, notify,
  purga 1 h, soft/hard, bloqueo antiabuso).

- **Cola de descarga y pipeline — alineación con eMule 0.72a** (`mule-daemon`,
  `mule-protocol`): auditoría frente a `DownloadClient.cpp`/`DownloadQueue.cpp`
  y correcciones de las divergencias de comportamiento:
  - **`SendBlockRequests` multibloques** (`plan_next_blocks_for_peer64` +
    `fill_and_send_block_requests`): el demonio solo solicitaba un bloque
    a la vez por fuente — caudal limitado a ~180 KiB/RTT. Ahora
    `blockCount` adaptativo al caudal medido (clientes eMule <4 KiB/s → 1,
    <9 KiB/s → 2; >150 KiB/s → 9, >75 KiB/s → 6, si no 3) y hasta
    **3 rangos por paquete** `OP_REQUESTPARTS`/`_I64` (formato de red
    eMule, huecos sin usar a cero). Los bloques end-game siguen siendo un
    respaldo unitario (sin duplicación en lote).
  - **Re-ask de cola de 29 min** (`FILE_REASK_INTERVAL`, eMule
    `FILEREASKTIME`): la espera en cola de subida reemite
    `OP_SETREQFILEID_EMULE` + `OP_STARTUPLOADREQ` cada 29 min —
    el uploader remoto purga las entradas cuyo `LastUpRequest` supera
    1 h (`MAX_PURGEQUEUETIME`); el antiguo keep-alive `SETREQFILEID` cada
    60 s **no** refrescaba este contador → nuestro slot era purgado
    al cabo de una hora en los clientes eMule oficiales.
  - **Sin timeout de cola fijo**: sustituido por «silencio durante un
    ciclo de re-ask completo + `queue_timeout`» — cualquier paquete recibido (rango,
    promoción, respuesta al re-ask) prolonga la espera, igual que eMule que
    no tiene timeout de cola.
  - **`DOWNLOADTIMEOUT` 100 s + reencolado en el mismo socket**: los timeouts
    `first_data`/`inactivity` pasan de 25 s a 100 s; un estancamiento
    dispara `OP_CANCELTRANSFER` y luego **reentrada en cola en el mismo
    socket** (`wait_in_queue` compartido, mapa de bits de partes refrescado,
    secuenciador reconstruido) en lugar de matar la conexión — máx. 3 estancamientos
    consecutivos antes de abandonar.
  - **In-flight sin reasignación prematura**: TTL de los bloques en vuelo
    ampliado a 100 s y **refrescado en cada bloque recibido** — un bloque ya no
    se vuelve a pedir a otro par mientras una fuente lenta aún lo está
    sirviendo; liberación al completarse, al finalizar la sesión o al
    reencolar.
  - **`ProcessBlockPacket` fiel**: `size != (end−start)+header` o
    `end ≤ start` → violación de protocolo (desconexión); bloque fuera de
    pending → ignorado; `end` que desborda el rango → `RemoveBlockFromList`
    (bloque liberado) sin desconectar.
  - **`Pending_Block_Struct`**: `queued`/`recv_to` por bloque; flujos
    zlib por bloque en vuelo (mapa por desplazamiento inicial) en lugar de un
    descompresor global.
  Regresiones: `test_emule_pending_block_count_tiers`, 4 pruebas
  `plan_next_blocks_*` (disjuntos, saltar completados/en vuelo, sombra,
  end-game único), pruebas adversariales y dual-peer adaptadas.

- **UI web — estado eD2K real mostrado al conectar** (`app_shell`):
  `AppController.refresh()` llama ahora a
  `serversController.refreshSession()` (RPC `servers.status`), por lo que
  la UI refleja la sesión de servidor actual incluso cuando el evento
  `server.sessionUpdated` se difundió antes de acoplarse el
  cliente. Caso sistemático en web: el demonio se conecta al servidor
  eD2K durante la carga de la página → la UI seguía mostrando
  «Desconectado» hasta un clic manual, mientras el demonio ya estaba
  conectado. Regresión cubierta por `servers_connection_test.dart`.

## [0.2.0] - 2026-09-23

Versión final de la rama 0.2.x — incluye la totalidad de `0.2.0-RC1`
más las siguientes incorporaciones.

- **Publicación open source** (repositorio): adición de `LICENSE`
  (GPL-2.0-or-later), `CONTRIBUTING.md`, `SECURITY.md`; `.gitignore`
  generalizado a todas las apps Flutter (`**/build/`, `**/.dart_tool/`,
  artefactos Android, keystores); despersonalización de las configuraciones
  `.agents/` (ejemplos `*.example.json`); `DeployDir` del script de
  compilación convertido en opt-in (sin ruta personal por defecto).

- **Interfaz web servida por el demonio — conexión sin configuración**
  (`mule-daemon`, `apps/web`, `app_shell`, `scripts`):
  - **ServeDir integrado** (`tower-http`): el demonio sirve la compilación
    web en su puerto HTTP. Resolución de la carpeta:
    `--web-dir` / `MULE_WEB_DIR` > autodetección `<exe>/../web`
    (paquete `bin/` → `web/`), `<exe>/web`, `../../apps/web/build/web`
    (dev cargo). `index.html` como respaldo SPA (enlaces profundos
    `usePathUrlStrategy`).
  - **Autenticación transparente**: `GET /session-config.js` emite
    `window.MULE_SESSION` (endpoint + token) — respuesta limitada a las
    escuchas **loopback** para nunca exponer `auth_token` en una
    interfaz de red.
  - **App web**: lee `MULE_SESSION` al arrancar (conexión sin
    introducción manual cuando la sirve el demonio), nuevo parámetro `?token=`,
    campo token opcional en el formulario de conexión,
    `autoReconnect` activado en el cliente RPC.
  - **Desktop**: botón «Interfaz web» en la barra lateral → abre el
    navegador en la URL HTTP derivada del endpoint de sesión
    (`SystemActions.openUrl`, patrón condicional io/stub).
  - **Paquete**: `build_windows_release.ps1` compila `apps/web`
    (`-SkipWeb`, `-WebBaseHref`), copia los recursos bajo `web/`,
    genera `Interface Web.bat` en la raíz (inicia el demonio si
    falta y abre `http://127.0.0.1:4660/`), y añade
    `includesWebUi` a `version.json`.

- **Paleta suavizada de la barra de progreso de partes**
  (`ui_kit`, `app_shell`): se conserva la semántica `DrawStatusBar` de eMule
  pero los primarios saturados se sustituyen por tonos
  Tailwind ya usados por `AppTheme` — Slate 400 = recibido,
  Teal 400 = verificado, Red 400 = sin fuente, Amber 400 = bloque en vuelo,
  Green 400 = franja de progreso, degradado Sky 300→800 para
  raro→común. Las constantes `partHaveColor`/`partVerifiedColor`/
  `partMissingColor`/`partPendingColor`/`partProgressColor` son
  públicas y compartidas con la tarjeta part-map.

## [0.2.0-RC1] - 2026-09-23

- **Pestaña Compartidos — categorías, búsqueda y estadísticas de subida**
  (`mule-core`, `mule-daemon`, `mule_client`, `app_shell`):
  - **Categoría por archivo compartido**: nueva columna
    `shared_files.category_id` (migración `ALTER TABLE` idempotente) +
    RPC `shares.setCategory`. Un compartido sintetizado desde una descarga
    sigue la categoría de esta (`shared_files` o `downloads`
    según la fuente) — menú «etiqueta» en cada fila para
    asignar/retirar.
  - **Vista renovada**: archivos agrupados por categoría (cabeceras
    pastilla + nombre, orden `sortOrder`), grupo «Sin categoría» al
    final; dentro de cada grupo, orden por **total subido
    descendente** (el más servido arriba).
  - **Buscador**: campo en la barra de título, filtro por
    nombre de archivo y hash eD2K.
  - **Estadísticas de subida por archivo**: velocidad medida actual (↑ B/s) y
    total acumulado (Σ) mostrados en cada fila, actualizados mediante
    `uploadsEvents` con debounce.
  - `SharedFileModel` ampliado (`categoryId`, `upPriority`,
    `uploadedBytes`, `uploadRateBps`) + cliente RPC/fake; las
    mutaciones fake preservan todos los campos.

- **UI descargas — chip de prioridad modernizado**
  (`app_shell`): píldora redondeada tintada + iconos semánticos
  (doble flecha arriba/abajo, guion) y **3 colores distintos** —
  ámbar = High, azul cielo = Normal, pizarra = Low. El menú emergente
  reproduce los mismos iconos de colores.

- **UI descargas — barra de progreso deduplicada**
  (`app_shell`): el `LinearProgressIndicator` encima de
  `PartAvailabilityBar` era redundante — la barra de bloques ya dibuja
  la franja verde de avance de eMule (`DrawStatusBar`). Eliminada
  en los diseños completo y compacto; la barra de bloques pasa a 14 px
  (12 px compacto) para ocupar toda la altura.

- **Arranque Kad resiliente — cola de candidatos al estilo eMule
  `s_liBootstrapList`** (`mule-daemon`): un timeout en el primer
  bootstrap dejaba Kad muerto con una tabla de enrutamiento vacía — el
  mantenimiento solo se activaba si `total_contacts > 0`, por lo que nunca
  había reintentos.
  - **Cola de candidatos** (`bootstrap_candidates`, tope 50, deduplicada):
    cada par eD2K que anuncia un puerto UDP Kad se registra en ella en el
    handshake (tanto entrante como saliente), filtrado `kad_version >= 2`
    (`KADEMLIA_VERSION2_47a`) — los pares pre-0.47a no hablan
    KADEMLIA2.
  - **Bucle de drenaje**: tarea en segundo plano que vacía la cola a razón de un
    `BOOTSTRAP_REQ` cada **2 s** mientras la tabla esté vacía,
    **15 s** cuando tenga contactos no verificados — calcado de
    `CKademlia::Process` (Kademlia.cpp:287). Cada candidato solo se intenta
    una vez (`RemoveHead`); los handshakes realimentan la cola.
  - **Guarda `!IsConnected()` fiel**: el drenaje se ejecuta mientras ningún
    contacto haya respondido nunca (`HasHadContact` de eMule = contacto
    *verificado*), incluso si la tabla contiene zombis no verificados —
    antes, 50 contactos muertos habrían bloqueado el drenaje.
  - **Limitador compartido** `last_bootstrap` (= `m_tBootstrap` de eMule):
    envíos inmediatos en el handshake (10 s) y drenaje comparten la misma
    marca temporal — sin ráfagas de peticiones.
  - **`nodes.dat` autocargado** al arrancar (`<data_dir>/nodes.dat`) como
    candidatos bootstrap: los contactos demuestran su vitalidad
    respondiendo al drenaje en lugar de entrar como zombis en la tabla.
  - **Evento `kad.statusChanged`** emitido en cuanto un bootstrap aprende
    contactos — la insignia UI pasa a «Conectado» sin esperar al tick
    de 30 s.
  - Prueba: `test_bootstrap_drain_retries_candidates_until_success`
    (candidato muerto → timeout → candidato vivo → tabla poblada).

- **Límites internos alineados con eMule 0.72a** (`mule-daemon`):
  eliminación de topes fijos heredados de las primeras versiones de prueba.
  - **Slots de subida**: `MAX_ACTIVE_UPLOADS` pasa de **10 → 100** =
    `MAX_UP_CLIENTS_ALLOWED` de eMule (`Opcodes.h`) — el tope de 10
    impedía escalar en conexiones rápidas.
  - **Slots garantizados**: los 4 primeros slots se abren sin condición
    de caudal (`max(MIN_UP_CLIENTS_ALLOWED, 4)` de eMule
    `AcceptNewClient`) — antes el mínimo `minUploadKbs` podía
    congelar la subida desde el 2.º cliente.
  - **Promoción de cola**: `promote_waiting` aplica ahora la misma
    elegibilidad que `AcceptNewClient` (mínimo de caudal + veto USS) vía
    `can_open_slot` compartido — antes, la promoción ignoraba ambos
    controles.
  - **Ritmo de conexiones de fuentes**: `MAX_CONNECT_PER_TICK` 5 → 12 —
    eliminación del doble estrangulamiento (coordinador 5/3s ≈ 100/min *más
    restrictivo* que el presupuesto `rate_limit_per_min` 240/min). Un archivo
    por sí solo alcanza ahora el ritmo eMule `MaxConnPer5Sec`=20 (~4/s);
    el presupuesto token-bucket sigue siendo la salvaguarda global.
  - **Tope entrante**: `MAX_INBOUND_CONNECTIONS` (500, constante muerta)
    eliminado — el listener sigue el presupuesto `max_active_peer_connections`
    en vivo, como eMule donde todas las conexiones de pares cuentan en
    `maxconnections`. El tope por IP (5, anti-flood) se conserva.
  - **Tope suave SX**: `max_source_soft` y el tope de ingesta de
    respuestas SX recaen en el ajuste `maxPeersPerDownload` persistido
    en lugar de la constante 400 (eMule: `GetMaxSourcePerFile` →
    `thePrefs.maxsourceperfile`).
  - **Refactorización**: la comprobación de cuotas de sesión (global +
    por archivo + duplicado de dirección), duplicada entre `connect` y
    `accept_callback`, se factoriza en
    `SwarmInnerState::check_session_slot`.

- **Auditoría completa de ajustes — cableado, duplicados, límites eMule 0.72a**
  (`mule-daemon`, `mule-network-policy`, `app_shell`, `SETTINGS.md`):
  inventario de las 36 claves `settings.set`/`settings.get` y de todos los
  campos UI — cada campo está cableado (persistido + aplicado en caliente o
  releído en el tick) y presente en una sola sección.
  - `maxConnections`: el techo pasa de **1500 → 65000**
    (`MAX_PEER_CONNECTIONS_CAP`) — eMule no limita `maxconnections`
    en Windows NT+ (`GetMaxWindowsTCPConnections()` → `UNLIMITED`);
    65000 sigue siendo una salvaguarda contra errores tipográficos. El valor
    se **acota antes de persistir** (antes se persistía en bruto
    y luego era rechazado por `set_budgets` → incoherencia almacenado/aplicado).
  - `maxUploadQueue`: acotación `[10, 5000]` (eMule `QueueSize`) aplicada
    antes de persistir — almacenado = aplicado.
  - `settings.get` devuelve los valores **efectivos** (acotados) para
    `maxConnections`, `maxConnPerMin`, `maxUploadQueue` — fin de los
    valores persistidos fuera de rango vueltos a mostrar.
  - UI: límites de entrada añadidos (`maxQueue` ≤ 5000, cuota ≤ 1 048 576
    GiB, `maxConnections` ≤ 65 000); pistas aclaradas —
    `maxGlobalDownloadPeers` (cuota de asignación swarm) explícitamente
    distinguido de `maxConnections` (presupuesto de sockets);
    `minUploadKbs = 0` documentado como umbral desactivado (extensión —
    eMule fuerza `MinUpload ≥ 1`).
  - Desviaciones asumidas y documentadas en `SETTINGS.md`: `kadPort = 0`
    no se acepta — la desactivación UDP pasa por `udpEnabled`
    (equivalente al «puerto 0» de eMule, en interruptor dedicado).

- **Lista de fuentes alineada con `srclist`/deadlist de eMule** (`mule-core`,
  `mule-daemon`, `app_shell`, `mule_client`): el panel desplegado de una
  descarga mostraba *todas* las filas persistidas de
  `peer_sources`, incluidos los fallos en cooldown, las fuentes rechazadas
  (FNF) y las filtradas por IP — eMule las retira de `srclist` y las coloca
  en `DeadSourceList` invisible (`CUpDownClient::Disconnected` →
  `RemoveSource` + `AddDeadSource`). Nuevo predicado compartido
  `PeerSourceStatus::is_srclist_visible()` /
  `PeerSourceStatus.isListedInSourceList`: solo `candidate`, `allowed`,
  `tried`, `connected` siguen mostrándose; `failed`, `rejected`,
  `filtered` siguen persistidas para el retroceso pero quedan ocultas. Los
  pares ya mostrados en la sección «sesiones activas» dejan de
  repetirse en la lista (`PeerSourcesController::getVisibleSourcesForHash`,
  deduplicado por dirección). El contador `sourceCount` de las instantáneas solo
  cuenta ahora las fuentes visibles (como `GetSourceCount()`
  de eMule) y se refresca en cada transición de estado o eliminación.

- **USS alineado con eMule `LastCommonRouteFinder`** (`mule-daemon`): el
  objetivo de ping USS ya no es el servidor eD2K remoto sino el **último
  router común** — rutas ICMP trazadas (`IcmpSendEcho`, TTL creciente,
  sin privilegios) hacia una muestra de pares conocidos (servidor, clientes
  de subida, fuentes de descarga — IPs públicas, ≤ 8), y luego ping al salto
  más profundo compartido por todas las rutas (normalmente el router
  de agregación del ISP). Mide la congestión del enlace ascendente local como
  eMule, en lugar de un RTT extremo a extremo. Objetivo en caché 15 min, rerresuelto
  tras re-baseline; respaldo `uss.pingHost` y luego sonda TCP-connect hacia el
  servidor (plataformas sin `IcmpSendEcho`). `uss.pingTarget` expone
  ahora el esquema (`icmp:`/`tcp:`). Nuevo módulo `uss_icmp`,
  dependencia `windows` (específica de plataforma) añadida.

- **Campo «límite de envío» que volvía a su valor USS** (`mule-daemon`,
  `app_shell`, `mule_client`): cuando USS está activado, su bucle de control
  sobrescribe el bucket de subida en cada tick — un `limits.set` manual era
  sobrescrito en ~1 s y el campo mostraba el valor efectivo (p. ej. 51 KiB/s)
  en lugar del configurado. Dos correcciones: los campos de
  `RateLimitsTab` muestran ahora el límite *configurado* persistido
  (`uploadConfiguredBytesPerSec`/`downloadConfiguredBytesPerSec`) con un
  texto de ayuda indicando el valor efectivo («Controlado por USS: X KiB/s» o
  «limitado por el ratio eD2K»); y en el demonio, `limits.set` sobre
  la subida actualiza `uss.max_upload_bps` cuando USS está activo — el
  límite introducido pasa a ser el techo del control dinámico (semántica
  eMule: `MaxUpload` limita DynUp, 0 = techo ilimitado).

- **Guía lab oculta en release** (`app_shell`): la tarjeta «Guía del
  modo laboratorio» de Ajustes → Avanzado solo aparece si
  `isLabModeAvailableProvider` es verdadero (demonio portátil con capacidad
  `lab-tools`, o dev tools) — la variante `standard` no expone ningún
  método `lab.*`, por lo que la entrada era inútil en producción.

- **Monitor de transferencia unificado en la barra superior** (`app_shell`,
  `ui_kit`): los tres chips separados (caudales en vivo + sparkline, totales
  de sesión, acumulados de por vida) se fusionan en una única píldora
  segmentada por separadores finos — `↓ DL/s ↑ UL/s │ sparkline │
  sesión │ Σ lifetime`. Cifras en fuente tabular
  (`FontFeature.tabularFigures`): el ancho ya no oscila cuando el
  valor cambia. Kill switch activo → píldora tintada de rojo con transición
  animada (`AnimatedContainer`) y segmento de sesión sustituido por el estado
  «Desactivado». Sparkline enriquecido: línea base discreta cuando la
  gráfica está vacía + relleno de descarga en degradado vertical en lugar
  de un alfa plano.

- **Localización FR completa de la interfaz** (`app_shell`,
  `mule_client`): `app_fr.arb` contenía ~340 valores aún en inglés
  (barra lateral entera, insignias, diálogos) — todos traducidos; los valores
  franceses extraviados en `app_en.arb` se sustituyen por inglés
  real. ~150 literales fijos (FR y EN mezclados) migrados a
  `context.l10n`/`L10nLookup` — vistas, diálogos **y mensajes de
  estado/error de los 9 controladores** (resueltos mediante `L10nLookup.current`
  sincronizado con `LocaleController`, sin `BuildContext`).
  `l10n/enum_labels.dart` localiza los enums de `mule_client`
  (`DownloadStatus`, `SupervisorState`, `NetworkProfile`, prioridades…).
  Etiquetas técnicas conservadas intencionadamente: exportaciones de diagnóstico,
  identificadores de protocolo, rutas. Correcciones de diseño de paso:
  las etiquetas más largas hacían desbordar varios `Row`
  (`history_filter_bar`, `add_link_dialog`, `import_part_met_dialog`,
  `diagnostic_dialog`, `import_inspection_card`) — hechos flexibles
  (`Flexible`/`Expanded`/`FittedBox` + elipsis).

- **Nuevo icono de aplicación** (`Asset/generated-image.png`,
  1254×1254 RGBA) desplegado en todos los objetivos: `app_icon.ico`
  multirresolución 16→256 px (exe Windows vía `Runner.rc` + recurso tray),
  iconos web 192/512 + maskables + favicon, `AppIcon.appiconset` macOS
  e iOS regenerados.

- **Ratio subida/descarga eMule** (`mule-network-policy`, `mule-daemon`,
  `mule_client`): `emule_ud_ratio()` reproduce
  `Preferences::GetMaxDownloadInBytesPerSec` («don't be a Lam3r :)») —
  límite subida < 20 KiB/s → descarga efectiva limitada a 3× (<4 KiB/s),
  4× (<10 KiB/s) o 5× (<20 KiB/s) la subida; descarga «ilimitada» limitada
  también. El límite *configurado* persistido nunca se sobrescribe: el ratio
  es un tope recalculado en cada aplicación (`limits.set`,
  `apply_capacity_clamp`, arranque) y sigue al límite dinámico USS en cada
  tick (eMule `bDynamic` → caudal medido). `limits.get` expone
  `downloadConfiguredBytesPerSec`/`uploadConfiguredBytesPerSec` y
  `downloadRatioApplied`.

- **Ajustes → Conexión: interruptores Red eD2K / Red Kad**
  (`app_shell`): el selector de perfil de 3 opciones y la tarjeta
  «Perfil de red actual» se sustituyen por una tarjeta «Redes» con
  dos interruptores — `Red eD2K` (desactivado → confinamiento loopback
  `local-lab`) y `Red Kad` (activado → `public-ed2k-kad-preview`, activar
  Kad activa también eD2K). El contrato de seguridad permanece intacto: los
  interruptores proyectan simplemente el mismo `NetworkProfile` subyacente.

- **Tarjeta demonio: estado de red eD2K/Kad + servidor** (`app_shell`): la
  tarjeta «Daemon» en la parte inferior de la barra lateral muestra ahora dos indicadores
  de color — `eD2K` (HighID verde, LowID/conectando naranja, desconectado
  rojo, desde `server.session`) y `Kad` (conectado verde cuando el socket
  UDP tiene contactos, «Arrancando» naranja, «Detenido» rojo) — más el
  nombre del servidor eD2K actual (dirección como respaldo). El tooltip del modo
  compacto resume las tres informaciones.

- **Apodo por defecto «eMuleRust»** (`mule-daemon`, `mule_client`,
  `app_shell`): `current_nickname()`, `settings.get`, el respaldo del
  campo vaciado en Ajustes y el HELLO de prueba de cortafuegos anuncian
  «eMuleRust». El `TAG_SPECIAL_NAME` del HELLOANSWER entrante refleja
  ahora el apodo realmente configurado en lugar de una constante.

- **Estadísticas de subida por archivo compartido (pestaña Compartidos)** (`mule-core`,
  `mule-daemon`, `mule_client`, `app_shell`): cada fila muestra el
  caudal de subida actual del archivo (`↑ …`, teal cuando está activo) y el total
  acumulado enviado (`Σ …`) — columnas «Upload speed» / «Transferred»
  de eMule. El caudal es la suma de los caudales medidos de los slots activos en
  ese hash (`UploadManager::per_file_rates_bps`); el total se incrementa
  en cada bloque enviado, se hidrata al arrancar desde la nueva tabla SQLite
  `share_upload_stats` y se persiste por deltas en cada `statsTick`
  (funciona también para compartidos parciales, que no tienen fila
  `shared_files`). `shares.list` enriquece cada entrada con
  `uploadRateBps`/`uploadedBytes`; la vista se refresca en
  `upload.uploadsUpdated` (debounce 1 s) — el caudal vuelve a 0 cuando el
  último slot se cierra.

- **Renombrado de archivo (Descargas y Compartidos)** (`mule-core`,
  `mule-daemon`, `mule-mcp`, `mule_client`, `app_shell`): icono lápiz
  en la barra de acciones de cada descarga y en cada compartido
  completo — diálogo prerrellenado con validación (caracteres prohibidos en
  Windows, `.`/`..`). `downloads.rename` actualiza `display_name` +
  `target_path` (el `.part` UUID no se mueve; el nuevo nombre se aplica
  al finalizar); para una descarga terminada, el archivo en disco y
  la entrada `shared_files` se renombran. `shares.rename` hace lo mismo
  para los compartidos fuera de descargas y delega en `downloads.rename` cuando
  el hash respalda una descarga listada. Herramientas MCP `emule_rename_download` /
  `emule_rename_share` añadidas. Las filas conservan una alineación vertical
  constante (espacio ETA reservado, slot pausa/reanudación siempre presente).

- **Barra superior: totales acumulados + gráfica de subida + Diagnósticos retirado**
  (`mule-daemon`, `mule_client`, `app_shell`):
  - el chip «Diagnósticos» se retira de la barra superior (el diálogo sigue
    accesible desde la barra lateral y Ajustes → Avanzado);
  - el sparkline en tiempo real dibuja ahora dos curvas en la misma
    escala — descarga rellena (color de estado) + subida en línea teal;
  - a la derecha de los totales de sesión, un segundo chip «Σ ↓ / ↑» muestra
    los acumulados desde la instalación, persistidos en SQLite
    (`lifetime_downloaded_bytes` / `lifetime_uploaded_bytes`,
    acumulación por deltas de sesión en cada `statsTick`) y
    transportados por `lifetimeDownloadedBytes` /
    `lifetimeUploadedBytes` en `event.statsTick`;
  - nuevo RPC `stats.resetLifetimeTotals` + tarjeta «Reiniciar
    totales acumulados» en Ajustes → Avanzado (con confirmación) —
    los totales de sesión se conservan.

- **Eliminación de un archivo compartido desde la pestaña Compartidos**
  (`mule-daemon`, `mule-mcp`, `mule_client`, `app_shell`): cada fila
  de compartido completo incorpora un icono de papelera con diálogo de
  confirmación. `shares.remove` acepta el nuevo parámetro
  `deleteFile`: el archivo se borra del disco y la entrada
  `shared_files` se retira; cuando el compartido respalda una descarga
  completada, la eliminación pasa por `remove_download` (entrada de
  descarga retirada también — si no, resucitaría el compartido al
  reiniciar). Los compartidos parciales (descargas en curso) rechazan la
  eliminación y muestran un icono desactivado que remite a la pestaña
  Descargas. Sin `deleteFile`, el comportamiento histórico
  (dejar de compartir solamente) no cambia.

- **Totales de sesión en la barra superior** (`mule-daemon`,
  `mule_client`, `app_shell`): el chip «perfil de red»
  (`public-ed2k-kad-preview`…) — redundante con Ajustes → Conexión —
  se sustituye por los totales de sesión «↓ descargado / ↑ enviado».
  `event.statsTick` transporta dos nuevos campos
  (`sessionDownloadedBytes` = suma de los deltas `covered -
  session_bytes_start` de las descargas activas; `sessionUploadedBytes` =
  contador `total_uploaded_bytes` del `UploadManager`), reenviados por
  `DaemonStatsModel`. Al pulsar se abre Historial y estadísticas; cuando el
  kill switch está activado, el chip conserva su estado rojo «Disabled»
  y redirige a Ajustes → Conexión.

- **Eliminación de descargas rediseñada** (`mule-daemon`,
  `mule-mcp`, `app_shell`): para una descarga en curso, la
  confirmación retira ahora automáticamente los archivos temporales
  (`.part` + `.part.met`) sin casilla de verificación. Para una descarga
  terminada, una casilla «Eliminar también el archivo compartido en la carpeta
  de destino» (desmarcada por defecto) elimina el archivo final Y
  su entrada `shared_files` (registrada al completarse por
  `finalize_download`) — sin esta opción, el archivo sigue compartido.
  El parámetro RPC `downloads.remove delete_files` cubre ambos casos;
  el comportamiento `.part.met.keep` de los archivos conservados no cambia.

- **Ordenación de la lista de descargas** (`app_shell`): los elementos
  terminados bajan al final de la lista; las descargas en curso se
  ordenan por avance descendente (las más completas arriba) y luego por
  fecha de adición reciente en caso de empate. Nuevo comparador compartido
  `compareDownloadsForList` aplicado a `filteredDownloadsProvider` y al
  getter heredado `AppController.filteredDownloads` (antes ordenación por
  `createdAt` u orden de inserción según la ruta).

- **Caudales de subida en tiempo real corregidos** (`mule-daemon`): la columna
  «Velocidad» de la vista Subidas mostraba un valor fijo — el dispatcher
  informaba `upload_bucket.rate_per_sec().max(length)`, es decir, el
  *límite configurado* del token bucket (0 = ilimitado → el tamaño del bloque,
  ~180–210 KiB) y no la velocidad realmente servida. `UploadManager`
  mide ahora el caudal internamente por ventanas de muestreo
  (`RATE_WINDOW_MS` = 2 s, primera muestra a los 500 ms — mismo enfoque
  que `bytes_since_rate_check/elapsed` en descargas), y
  `effective_rate_bps(now)` hace decaer el caudal hacia 0 cuando un par
  deja de pedir bloques (convención `decay_stale_rates`). El bucle
  `statsTick` (2 s) emite además `upload.uploadsUpdated` mientras haya
  slots activos o en espera, por lo que la UI ve los caudales variar y
  caer sin esperar a un nuevo envío. Extra: `statsTick.uploadBps`,
  USS (`GetDatarate`) y la salvaguarda `MinUpload` aprovechan también el
  caudal medido en lugar del límite configurado.

- **Compartidos parciales, contadores de barra lateral en tiempo real, selectores nativos y
  barra de partes eMule 0.72a precisa por bytes** (`mule-daemon`,
  `mule_client`, `app_shell`, `ui_kit`):
  - **Descargas en curso siempre compartidas**:
    `shared_entry_for_active` ya no devuelve `None` cuando ninguna parte
    eD2K de 9,73 MiB está completa — toda descarga activa aparece en
    «Archivos compartidos» como `isPartial` desde su creación (eMule
    `CDownloadQueue::AddPartFilesToShare`: *«part files are always
    shared files»*); solo las partes completadas se anuncian y
    sirven (`PARTSTATUS`, rechazo de rangos no completados, publicación
    Kad/servidor sin cambios).
  - **Contadores de barra lateral en tiempo real**: nuevo `SharesController`
    (insignia «Compartidos» en vivo); `kad.statusChanged` se emite ahora
    desde el bucle de mantenimiento Kad cuando `total_contacts` cambia
    realmente (no solo desde los manejadores RPC — la insignia se actualiza
    sin abrir la sección, ≤30 s); nuevo aviso
    `shares.changed` en `add`/`remove`/`rescan`/`setEnabled`/
    `setUploadPriority` y stream `sharesEvents` en el cliente.
  - **Notificaciones transitorias eliminadas**: banner de estado superior
    (`StatusNotificationBanner` retirado) y todos los `SnackBar`
    inferiores; los errores al añadir enlaces pasan a mostrarse inline en el
    diálogo (los errores estructurados `setError` y los banners
    persistentes se conservan).
  - **Selectores de archivos nativos**: helper `core/file_picker_helper`
    (`pickFile`, espejo de `pickFolder`, `file_picker` 12.3) — botones
    «Examinar» en carpeta de descargas, carpeta temporal,
    `ipfilter.dat`, `server.met`, `nodes.dat` y la importación `.part.met`
    (campos `.met`, `.part`, carpeta destino).
  - **Barra de partes fiel a `CPartFile::DrawStatusBar`**:
    `PartAvailabilityBar` reescrita en `CustomPainter` preciso por bytes —
    pizarra = bytes recibidos (teal si parte verificada), rojo = hueco sin
    fuente confirmada, degradado azul `RGB(0, 210−22·(n−1), 255)`
    raro→común, amarillo `RGB(255,208,0)` = bloques en vuelo, franja verde
    3 px = progreso; paleta atenuada en pausa. Nuevos campos de
    instantánea: `PartProgressSummary.receivedMask` (mapa de bits de 64 celdas
    intraparte, ≈152 KiB/celda, calculado desde el `RangeSet` real) y
    `DownloadSnapshot.pendingRanges` (rangos de `in_flight_blocks`,
    enriquecido en `downloads.list`/`get`, `broadcast_download_update` y
    los puntos de emisión `transfer.rs`/`strategy.rs`). Tarjeta «eD2K Part
    Map» del detalle alineada (relleno intraparte, banda amarilla pending,
    leyenda «No source»).
  - **Disponibilidad real de las partes**: la máscara `OP_FILESTATUS`
    recibida de cada par (inicial *y* keep-alive en cola de espera) se
    registra ahora en `act.sources` mediante
    `record_source_availability` + emisión `downloadUpdated` — antes
    solo llegaba al almacén SQLite del panel Fuentes, dejando la
    barra totalmente roja. `compute_part_summaries` aplica la
    convención eD2K `part_count == 0` = fuente completa (eMule
    `ProcessFileStatus` → `memset(m_abyPartStatus, 1)`) e ignora las
    máscaras caducadas (> 30 min, `SOURCE_FRESHNESS_MS` compartido con
    `part_source_counts`). Como en eMule, las fuentes descubiertas pero
    nunca contactadas no cuentan — el rojo significa «sin
    fuente confirmada».
  - **Pruebas WS endurecidas**: `ed2k_transfer_tests` y
    `daemon_adversarial` leían `read.next()` sin filtrar el `id` —
    cualquier notificación broadcast intercalada (eventos reenviados a toda
    sesión autenticada) hacía fallar el `unwrap`; helper
    `read_response`/`next_response` filtrando por `id` utilizado ahora
    en todos esos archivos.

- **Auditoría del cableado de red eD2K — inyecciones faltantes, kill switch
  reversible y ajustes en vivo propagados** (`mule-daemon`, `mule-mcp`) —
  revisión completa de la ruta servidor/pares (RPC → servicios → handshake)
  y correcciones:
  - **Callback entrante cableado**: `ServerSessionManager` recibe
    `InboundListenerService` al arrancar — `OP_CALLBACKREQUESTED` (un par
    HighID nos contacta mientras somos LowID) dispara realmente
    el `connect_back` saliente implementado en el listener; el código
    existía pero el servicio nunca se inyectaba en producción.
  - **Kill switch reversible**: `network.disconnectAll` ya no aborta
    los supervisores (vigilante de reconexión, `SwarmCoordinator`,
    rotador de identidad, cuota de subida) — se autoinhiben mientras
    el estado sea `Disabled` — y los sockets UDP (Kad, servidor), no vinculados
    al token de cancelación, se detienen explícitamente. `network.enable`
    crea un token nuevo y relanza el listener TCP entrante, los sockets
    UDP, el mantenimiento Kad, USS y el refresco UPnP; al conservarse
    `last_target` en el corte (`disconnect_for_kill_switch`), el vigilante
    restaura la sesión de servidor como eMule `Reconnect`. USS vuelve a ser
    relanzable (la bandera `running` se libera al salir del bucle).
  - **`servers.connect` con semántica eMule**: cambio en caliente hacia otro
    servidor sin `disconnect` previo; la reconexión hacia la
    misma dirección es idempotente (se devuelve la instantánea actual).
  - **Ajustes de ofuscación propagados por todas partes**: la preferencia se
    aplica a la sesión de servidor al arrancar y en `settings.set`
    (`set_obfuscation_preference` nunca se llamaba en producción);
    el coordinador utiliza `effective_obfuscation()` (valores
    persistidos) para `OP_GETSOURCES` y la conexión a pares en lugar de los
    valores `config.*` fijados al arrancar; el apodo real
    (`current_nickname`) se usa en el failover de servidor y los
    hellos de pares (eliminación de literales `http://emule-project.net`
    y `eMuleRust`).
  - **Conexión a pares sin servidor (solo Kad)**: el coordinador
    conecta las fuentes directas incluso desconectado — `client_id=0` en
    el hello, como la ruta manual `sources.connect`; las fuentes
    `lowid:` y los callbacks de servidor siguen filtrados
    (`server_is_high_id=false`), inalcanzables por construcción.
  - **MCP `emule_search`**: `network:"both"` (enum del esquema) activa
    realmente Kad + servidor — antes solo `"all"` era reconocido y
    `"both"` devolvía cero resultados en silencio.
  - **Pruebas**: `secure_ident_tests` ya no se bloquea — el compartido apunta
    a un archivo temporal real en lugar de la ruta ficticia
    `C:\Shared\...` (un rango ilegible queda sin respuesta por
    diseño eMule, al no tener timeout el `read()` de la prueba);
    `server_connection_tests` cubre la nueva semántica idempotente
    de `servers.connect`.

- **Enlace RPC auditado — `upnp.statusChanged` cableado de extremo a extremo**
  (`mule-daemon`, `mule-mcp`, `mule_client`, `app_shell`) — auditoría completa
  del contrato JSON-RPC/WebSocket (métodos, eventos, handshake, supervisión)
  y correcciones:
  - **Demonio**: nueva variante tipada `DaemonEvent::UpnpStatusChanged`
    (método `upnp.statusChanged`, tópico `network`, payload = instantánea
    `UpnpStatus` completa). Los manejadores `upnp.*` pasan ahora por
    `DaemonEvent::send` — `services/events.rs` vuelve a ser la fuente única
    del formato de red para **todos** los eventos.
  - **Cliente Dart**: nuevo stream `MuleDaemonClient.upnpStatusEvents`,
    manejador `upnp.statusChanged` en `RpcDaemonClient`, implementaciones
    en `FakeMuleDaemonClient` y los mocks de pruebas; `UpnpStatusModel`
    expone `cgnatDetected` (campo de la instantánea antes ignorado).
  - **UI**: la tarjeta UPnP de Ajustes → Conexión se suscribe al flujo y
    se actualiza ante cualquier cambio asíncrono (reintentos de mapeo en
    segundo plano del demonio), ya no solo por acción del usuario.
  - **`mule-mcp`**: eliminación del listener muerto `kad.searchResult`
    (nunca emitido — los resultados `kad.searchKeywords` son síncronos).
  - Control completo: ~95 métodos despachados, todos los invocados
    por el cliente tienen manejador; `cargo check`/`clippy`,
    `rpc_contract` en verde.

- **Ciclo de vida `.part` / `.part.met` endurecido — paridad `CPartFile` eMule**
  (`mule-daemon`, `mule-protocol`) — auditoría completa de la gestión de
  archivos parciales y correcciones:
  - **Pausa efectiva en transferencia activa**: las sesiones de pares comprueban
    el estado de la descarga antes de cada bloque planificado (inicio de sesión y
    bucle de pipeline en `transfer.rs`) y se detienen limpiamente;
    `inject_range` solo devuelve a `Downloading` desde
    `Queued`/`WaitingForSources` — un bloque en vuelo sobre una descarga
    `Paused`/`Failed`/`Removed` se escribe sin despausar ni resucitar
    (semántica eMule: escrituras en vuelo terminadas, peticiones detenidas).
  - **Escritura garantizada antes de persistir**: si el descriptor `.part`
    falta, `inject_range` lo reabre bajo demanda; en caso de fallo,
    se devuelve el error en lugar de marcar el rango recibido sin bytes escritos
    (fin de los bucles fantasma de verify-fail al 100 %).
  - **Integridad de subida**: eliminación del respaldo sintético `0x77` —
    una lectura de disco fallida en un compartido deja la petición sin
    respuesta (comportamiento eMule) en lugar de envenenar al solicitante.
  - **Escaneo de huérfanos al arrancar**: `recover_orphan_part_mets` escanea
    el directorio temporal en busca de `*.part.met` desconocidos para la base (BD perdida o
    archivos depositados por eMule) y los importa por la ruta aislada
    habitual — verificación de hashsets, desalojo de partes corruptas,
    estado `Paused`, cero red. El `.part` se **adopta in situ**
    (`rename` atómico hacia `{uuid}.part`, sin copia), el met origen
    consumido; mets malformados ignorados con `warn`, nunca fallos fatales.
  - **Persistencia continua de los mets**: `save_active_part_mets` escribe los
    `.part.met` de las descargas no terminales — cadencia ~60 s en el
    latido del `SwarmCoordinator` (eMule `SavePartFile`) + volcado
    explícito en el cierre ordenado — una caída ya no deja un met
    desactualizado.
  - **Eliminación keep-files sin resurrección**: `downloads.remove`
    con `deleteFiles=false` renombra el compañero a `*.part.met.keep`
    (invisible al escaneo de huérfanos, siempre importable mediante
    `downloads.importPartMet` con `partPath` explícito); código muerto
    `cancel_download` eliminado.
  - **Pureza de `mule-protocol` restaurada** (ADR 0003): `ed2k::storage`
    (`Ed2kPartFileSink`, `.resume.json`) protegido tras `ed2k-loopback`,
    `serde_json` hecho opcional — el crate sigue sin E/S en compilación
    estándar.

- **Source Exchange (SX) alineado con eMule 0.72a** (`mule-protocol`,
  `mule-daemon`) — auditoría completa frente a `srchybrid`
  (`docs/ed2k/messages/source_exchange.md`):
  - **Negociación versión/opcode fiel**: la respuesta sigue
    `byUsedVersion = min(byRequestedVersion, 4)` en SX2 y pasa a
    `OP_ANSWERSOURCES` SX1 (versión `GetSourceExchange1Version`) cuando el
    solicitante no envía SX2 — antes `OP_ANSWERSOURCES2` v4 era
    sistemático y descartado por los clientes SX1/SX2<v4
    (`PartFile.cpp:3629-3646`).
  - **Subopcodes `OP_MULTIPACKET*`**: las peticiones SX incrustadas
    (`0x92`/`0xA4`/`0xA9`, hash transportado por la envolvente) se decodifican y
    despachan al mismo manejador — el caso típico de los clientes eMule,
    tanto entrante como en las sesiones de descarga. `OP_REQFILENAME` /
    `OP_FILESTATUS` / `OP_AICHFILEHASHREQ` reciben una
    `OP_MULTIPACKETANSWER` agrupada; la ext-info `0x58` (part-status del
    solicitante, leída según la `ExtendedRequestsVersion` negociada) alimenta
    el `reqstatus` SX.
  - **Filtrado en la ingesta (`AddClientSources`/`CanAddSource`)**:
    `userhash == own`, puerto 0, IP mala/LAN (política de red) o filtrada
    (`ipfilter`), LowID cuando estamos tras cortafuegos, propio `client_id`+servidor,
    tope `GetMaxSources` (por defecto 400); persistencia deduplicada.
  - **Selección de fuentes servidas (`CreateSrcInfoPacket`)**: únicamente
    `!HasLowID && IsValidSource` con máscara compatible y ≥ 1 parte útil
    para el solicitante, tope 501; silencio si `nCount == 0` o archivo desconocido
    (compartido **o** descarga parcial servida).
  - **Limitadores eMule**: respuestas limitadas por tasa `SOURCECLIENTREASKS`
    40 min/cliente (×4 salvo archivo parcial raro ≤ `RARE_FILE` = 50); peticiones
    gobernadas por `IsSourceRequestAllowed` (`SOURCECLIENTREASKF`,
    `CONNECTION_LATENCY`, `GetMaxSourcePerFileSoft`).
  - **Re-ask periódico**: `OP_REQUESTSOURCES(2)` vinculado al ciclo de
    reask (keep-alive de cola + bucle de transferencia) en lugar de solo al
    handshake.
  - **Red estricta**: endianness SX1 v<3 en orden de red corregida,
    versión 0/>4 descartada, `count × entry_size == len` exigido.
  - **Fallos permanentes**: denegación de permiso de red (`AddressViolation`/
    `IpFiltered`) marca la fuente como `Rejected` en lugar de hacer bucle
    `Permit denied` en el coordinador.

- **Kad DHT — endurecimiento de interoperabilidad eMule 0.72a** (auditoría de las
  fuentes oficiales `srchybrid/kademlia/` + diagnóstico del registro WAN):

  - *Respuestas de enrutamiento conformes*: `KADEMLIA2_REQ` (`0x21`) recibe
    ahora una verdadera respuesta `KADEMLIA2_RES` (`0x29`) con los contactos
    **verificados** más cercanos a `target` (eMule `GetClosestTo` tipo 2),
    en lugar de un `BOOTSTRAP_RES` vacío que era descartado como no solicitado
    por el demandante — el nodo ya no es desalojado de las tablas remotas.
    Como en eMule, la petición solo se sirve si el campo `uCheck` iguala
    nuestro KadID (descarte silencioso en caso contrario). `BOOTSTRAP_REQ` recibe nuestro
    KadID + puerto **TCP** eD2K (y no UDP — fallo de red corregido, el
    parámetro del codificador se renombra a `tcp_port`) + versión Kad **8** +
    hasta 20 contactos verificados diversificados (`spread_verified`).
  - *Aprendizaje de contactos entrantes*: la tabla de enrutamiento se
    inyecta en el listener UDP (`set_routing_table`); los emisores
    de `HelloReq`, `HelloRes`, `BootstrapRes` y `PublishSourceReq`
    validado alimentan la tabla — como `AddContact` de eMule que aprende
    desde cada paquete.
  - *Semántica `verified` fiel*: los contactos aprendidos en las
    respuestas de enrutamiento ya no se marcan como verificados (un bootstrap
    malicioso ya no puede inyectar «verificados» arbitrarios);
    promoción únicamente por respuesta solicitada (`mark_seen`) o
    handshake `HELLO_RES_ACK`. El bootstrap solo asume `verified` si
    la tabla está vacía (`bAssumeVerified` = `GetNumContacts()==0`).
  - *Tolerancia de publicación*: `PublishSourceReq`/`PublishKeywordReq`/
    `PublishNotesReq` entrantes se descartan silenciosamente cuando la
    clave objetivo está fuera de `SEARCHTOLERANCE` de nuestro KadID (comportamiento
    `KademliaUDPListener.cpp`) — el nodo ya no se convierte en un sumidero
    de índice universal.
  - *Identidad publicada correcta*: `publish_source` anuncia la IPv4
    **pública observada** (`FirewalledRes` → `observed_ip`, o HighID
    de conexión a servidor propagado vía `set_public_ipv4`) y el puerto UDP
    efectivo en lugar de la dirección de escucha — si no los indexadores eMule
    rechazaban nuestras publicaciones (`contact.ip == IP fuente`). Versión
    anunciada: 8.
  - *Búsquedas unificadas*: `find_node`, `search_notes` y `publish_note`
    pasan por `converge_closest_contacts` (convergencia iterativa
    ALPHA=5) en lugar del recorrido secuencial / tabla local.
  - *Interbloqueo corregido*: ciclo `in_flight` → `routing_table` (manejadores
    `HelloRes`/`BootstrapRes` reteniendo el mutex de transacciones durante
    `learn_contact`) frente a `routing_table` → `in_flight`
    (`kad.status` → `get_status` → `get_snapshot`) — congelaba todo el
    pipeline Kad en sesión WAN real (bucle recv muerto, mantenimiento y
    búsquedas bloqueadas). Orden de bloqueos corregido en ambos lados:
    `learn_contact` llamado tras liberar `in_flight`, y
    `get_status` ya no atraviesa ningún await reteniendo `routing_table`.
  - *Fuga de transacciones en vuelo*: `InFlightTxGuard` RAII retira
    la entrada `in_flight` al descartarse el futuro `send_*` (el timeout
    externo de 2,5 s de `converge_closest_contacts` cancelaba el futuro
    antes de su propio timeout interno de 3 s, dejando la entrada hasta
    saturar el tope 200 → silencio total de envíos). `insert_in_flight`
    barre además las entradas con antigüedad superior a `timeout+2 s`; las 4
    inserciones inline que eludían el tope se unifican.
  - *Respuestas de búsqueda multipaquete*: las `SEARCH_*_RES` llegan
    en varios datagramas (~50 entradas/paquete, hasta ~300 resultados
    en eMule) — la transacción ya no se retira en el primer paquete.
    `InFlightTransaction.sender` pasa a `mpsc` no acotado;
    `collect_search_batches` agrega los lotes en una ventana inactiva de
    350 ms (≤ 12 paquetes, ≤ 300 entradas). ~91 % de los resultados keyword
    se descartaban antes como no solicitados.

- **Opciones exclusivas de eMuleRust** (sin equivalente en eMule/aMule):
  - `paranoidMode`: slots de subida reservados a pares **ofuscados con
    firma Secure Ident verificada** — aplica la guarda delante de
    `request_upload_slot` (`OP_STARTUPLOADREQ`, `OP_REQUESTPARTS*`).
  - `identityRotationHours`: rotación periódica de la identidad —
    nueva clave RSA (`cryptkey.dat`), nuevo `user_hash` eD2K y
    nuevo `KadID` propagados en caliente (listener, SUI, firewall-check,
    tabla de enrutamiento Kad). Marca temporal persistida
    (`identity_last_rotation_unix`), vigilante 60 s. Los créditos se
    desvinculan intencionadamente en cada rotación.
  - `dailyUploadQuotaGb`: cuota volumétrica de subida por día —
    contador acumulativo persistido (`upload_quota_used_bytes`,
    `upload_quota_day`), guarda atómica que bloquea los nuevos slots en el
    umbral, reinicio al cambiar de día UTC (vigilante 30 s).
  - `swarmHealing`: sesga la planificación de bloques hacia las **partes
    más escasas del swarm** (contador de fuentes conocidas por parte,
    `Ed2kDownloadSequencer::with_part_scarcity`) — el cliente pasa a ser
    fuente de respaldo antes de que desaparezca una parte. Las fuentes
    completas (`part_count = 0`) cubren todas las partes; las máscaras
    caducadas (> 30 min) se ignoran. eD2K sigue siendo pull-only: sin push,
    desempate por bytes recibidos.

- **Persistencia corregida**:
  - `limits.set` ya persistía los 5 buckets; una actualización
    **parcial** reiniciaba en vivo los limitadores no proporcionados a los
    valores por defecto — la configuración parte ahora de la instantánea en vivo de los buckets.
    La UI también enviaba `serverRequestsPerSec`/`kadPacketsPerSec` fijos
    (10/50) — ahora envía los valores actuales.
  - **Filtro IP**: `filter.setEnabled` y `filter.importFile` no
    persistían nada — `ipfilter_enabled` + `ipfilter_path` se
    escriben ahora en SQLite y se recargan al arrancar
    (`filter.clear` olvida la ruta), como eMule recargando
    `ipfilter.dat`.

- **Valores por defecto alineados con eMule 0.72a**: cola de subida 5000
  (`QueueSize`), `minUploadKbs` 1 (`MinUpload`), conexiones/min 240
  (`MaxConnectionsPerFiveSeconds` = 20), UPnP desactivado por defecto,
  eliminación de servidores muertos tras el 1.er fallo (`DeadServerRetry`).
  UI Conexión: botones «Aplicar», las capacidades aceptan `0`
  (ilimitado).

- **Corrección compartidos tras reiniciar**: las descargas `Completed`
  persistidas en SQLite pero ausentes del mapa `active` (arranque limpio) se
  fusionan ahora en `list_all_shared_files` y se resuelven mediante
  `get_shared_file` — como eMule que rehashea sus archivos terminados en
  `CSharedFileList` al arrancar. Sin esta corrección, la pestaña Compartidos estaba
  vacía y los archivos completados no se podían servir en subida tras
  un reinicio. El archivo destino debe existir en disco.

- **Ajustes eMule completados** (`docs/SETTINGS.md` como referencia):
  - **Servidores** (sección Conexión): `serverReconnect` — vigilante 15 s
    que reconecta al último servidor por la ruta RPC normal;
    `serverAutoconnect` — conexión al mejor servidor al arrancar;
    `serverAutoremoveDead` — eliminación tras 3 fallos;
    `serverAutoupdateList` + `serverAutoupdatePath` — importación de un
    `server.met` local al arrancar (sin fetch HTTP, anti-SSRF);
    `safeConnect` — `downloads.resume` bloqueado mientras ni eD2K ni Kad
    estén disponibles.
  - **General**: `tempDir` (carpeta `.part` configurable),
    `ichEnabled` (verificación MD4 de partes), `rememberCancelled` /
    `rememberDownloaded` (filtrado de entradas de historial).
  - **Transferencias**: `minUploadKbs` (mínimo por slot de subida, eMule
    `MinUpload` — no abre slot por debajo de la media configurada),
    `autoPriorityNewDownloads` (archivos ≥ 1 parte en prioridad `high`).

- **Rediseño completo de Ajustes**: el diálogo cajón de sastre se sustituye
  por una **pestaña dedicada `/settings`** (`SettingsView`) con barra de secciones
  a la izquierda, estilo «Preferencias» de eMule:
  - **General**: carpeta de descargas, interruptor `shareDownloadDir`
    (compartición automática de la carpeta Incoming — nuevo ajuste expuesto vía
    `settings.get`/`settings.set`), retención del historial, idioma.
  - **Conexión**: perfil de red lab/WAN, banner kill switch, presupuestos de
    sockets, acceso a la configuración UPnP/puertos (antiguos `NetworkPolicyDialog` +
    `NetworkModeDialog` fusionados).
  - **Transferencias**: `maxGlobalDownloadPeers`, `maxPeersPerDownload`,
    `minPeersPerDownload` — ajustes RPC antes invisibles en UI.
  - **Seguridad**: filtro IP (`ipfilter.dat`) + límites de velocidad
    (antiguo `IpFilterDialog` integrado) + aviso anti-SSRF.
  - **Amigos**: friend slots (sección movida del antiguo diálogo).
  - **Avanzado**: diagnóstico, importación `.part.met`, guía lab, barra debug,
    versión del demonio.
  - Cliente: nuevo método `updateSettings(patch)` → `settings.set`.
  - `SettingsDialog` eliminado (código muerto); el elemento «Settings» de la
    barra lateral navega a `/settings`.

- **Estrategias «smart» de eMule 0.72a** (descarga + subida):
  - **Preview (primera/última parte)**: `Ed2kDownloadSequencer::with_preview_priority`
    sirve las partes 0 y N-1 con prioridad (`PR_VERYHIGH` eMule) para archivos
    multimedia/comprimidos, permitiendo la previsualización durante la descarga.
  - **End-game redundancy**: cuando quedan ≤ 3 bloques (≈ 552 KiB), un bloque
    ya en vuelo puede volverse a pedir a un segundo par (máx. 2 asignaciones,
    nunca al mismo par; fuera de end-game el solapamiento sigue prohibido) —
    conforme a `CPartFile::GetNextRequestedBlock`.
  - **Descarte `NoNeededParts`**: `PeerFailureReason::NoNeededParts` — un par cuya
    máscara de bits no cubre ningún bloque faltante se libera limpiamente y se marca
    `Rejected` temporalmente.
  - **Prioridades de archivo**: `Download.priority` persistido (columna SQLite
    `downloads.priority`, por defecto `normal`), RPC `downloads.setPriority`,
    ponderación del asignador de slots (low×1/normal×2/high×4/release×8),
    selector de prioridad en la expansión Fuentes de la UI.
  - **Créditos eMule persistidos**: los bytes descargados desde un par
    verificado SUI se contabilizan bajo `Md4(pubkey)` (volcado cada 8 MiB +
    fin de sesión); la subida ya estaba cableada — la fórmula canónica
    `min(10, 2·DL/UL)` se aplica ahora en ambos sentidos.
  - **Trickle slots + slot focusing**: `UploadSlotState::Trickle` — un slot
    < 1 KiB/s tras 60 s de gracia se degrada (deja de contar en
    `max_active`) y se promueve al mejor cliente en cola.
  - **Friend slots + antileecher mínimo**: tabla `friends` (equivalente
    `friends.met`), RPC `friends.list`/`add`/`remove`, slot dedicado que salta
    la cola (`uh:<userhash>`), rechazo de clientes no identificados cuando la cola
    está ≥ 90 % llena.
  - **A4AF (intercambio entre archivos)**: `a4af_rebalance` en cada tick del
    coordinador — un archivo bajo `min_peers` reclama una dirección conectada
    a un archivo de prioridad estrictamente inferior y saturado (1 intercambio/tick,
    log `[A4AF]`). Nuestras sesiones siguen siendo monoarchivo: el intercambio se hace por
    reconexión, no por multiplexación `OP_REQFILENAME`.

- **Carpeta «Incoming» compartida** (`incoming_share.rs`): al arrancar,
  cada archivo regular del directorio de descargas se hashea en eD2K
  (MD4 + hashset persistido) y se registra como archivo compartido — mismo
  comportamiento que la carpeta Incoming de eMule. Las entradas existentes
  conservan su prioridad/estado; desactivable mediante el ajuste
  `share_download_dir = "0"`. Artefactos de descarga excluidos
  (`.part`, `.met`, `.tmp`…). Log `[INCOMING_SHARE]`.
  - **Reescaneo manual**: RPC `shares.rescan` + cliente `rescanSharedFiles()`;
    el botón actualizar de la pestaña Compartidos dispara ahora un verdadero
    reescaneo de la carpeta Incoming (hash de nuevos archivos, ajustes
    `enabled`/`upPriority` existentes preservados) y recarga la lista,
    con SnackBar de resultado. Pruebas offline en
    `upload_inbound_tests.rs` (descubrimiento, preservación, opt-out).

- **Compartición de descargas en curso (eMule `CPartFile`)**: una
  descarga activa pasa a ser servible en cuanto se recibe una parte eD2K completa
  (9 728 000 B) — `get_shared_file` resuelve un `SharedFile`
  `is_partial` que apunta al `.part`, con `completed_parts` = índices
  de las partes verificadas. El dispatcher solo anuncia esas partes en
  `PARTSTATUS` (`OP_FILESTATUS`), rechaza cualquier rango fuera de partes completadas
  (`OP_REQUESTPARTS`) y nunca calcula hashset/AICH desde un archivo
  incompleto (el hashset del `.met` sigue sirviéndose, raíz AICH conocida
  únicamente). `shares.list` incluye ahora los parciales (insignia
  «parcial (n partes)» en la pestaña Compartidos, controles de
  prioridad/activación desactivados en ellos).

- **Ajustes eMule adicionales cableados** (`settings.get`/`settings.set`,
  aplicados en caliente):
  - **General**: `nickname` — apodo anunciado a servidores y pares
    (sustituye los valores fijos «eMuleRust»).
  - **Conexión → Protocolo y límites**: `obfuscationEnabled` /
    `obfuscationRequired` (cifrado de protocolo — listener entrante en vivo vía
    `set_obfuscation`, conexiones salientes vía `effective_obfuscation()`,
    byte connect-options buddy Kad actualizado); `maxConnections` y
    `maxConnPerMin` (presupuestos `network_policy` aplicados de inmediato);
    `ussEnabled` (Upload Speed Sense, persistido en `uss_config`).
  - **Transferencias**: `maxUploadQueue` — tamaño de la cola de espera de subida
    (eMule `QueueSize`, por defecto 500, acotado 10–5000).
  - **Conexión → Capacidades y puertos** (panel eMule completo):
    `capacityDownloadKbs`/`capacityUploadKbs` (capacidades de línea, la
    capacidad de emisión sirve de referencia a USS); `ed2kPort`/`kadPort`
    **rebind en caliente** (listener TCP detenido/reiniciado si activo, socket Kad
    UDP rebind, `kad.set_ed2k_tcp_port`, buddy connect-options,
    `upnp.set_ports`); `udpEnabled` (detiene/inicia `kad_udp` +
    `server_udp`, eMule «UDP desactivado»); `upnpEnabled` (map/unmap
    inmediato). Puertos efectivos en atómicos sobre `AppState`
    (`ed2k_port()`/`kad_port()`/`udp_enabled()`), persistidos y aplicados al
    arrancar (`state.rs` lee los ajustes antes de vincular) — todos los puntos
    de llamada (`servers`, `sources`, `swarm_coordinator`) usan ahora
    los valores efectivos.

- **Alineación fina con eMule 0.72a — cola de subida y gestión de fuentes**:
  - **Fórmula de crédito exacta** (`ClientCredits::GetScoreRatio`):
    `min(2·DL/UL, √(DL/1Mio + 2), tope lineal ~3,34 @ 9,2 Mio)` acotada a
    `[1.0, 10.0]` — sustituye el `2·DL/UL` aislado que sobreacreditaba a los pequeños
    descargadores; identidades inválidas/no verificadas → 1.0.
  - **Prioridad del archivo solicitado en la puntuación de cola**: `file_prio_num`
    por archivo compartido (`veryhigh/powershare`=18, `high`=9, `normal`=7,
    `low`=6, `verylow`=2) aplicado a la puntuación de espera; columna
    `shared_files.up_priority` + RPC `shares.setUploadPriority`.
  - **Bonificación «downloading»**: un cliente en curso de subida que vuelve a entrar en cola
    recibe ~+15 min de tiempo de espera (`GetScore(isdownloading)`), lo que
    estabiliza las sesiones activas.
  - **Puntuación 0 bad-guy**: los clientes bloqueados o con identificación rechazada
    (`IS_IDBADGUY`) permanecen en cola con puntuación 0 — nunca promovidos.
  - **Purga condicional al 80 %** (`CPartFile::Process`): cuando el número
    de fuentes conocidas alcanza el 80 % del `maxSources` del archivo, purga con
    cadencia 60 s de los LowID no rellamables, fuentes NNP («no needed
    parts») y fuentes con fallos repetidos (`fail_count ≥ 3`), log
    `[SOURCE_PURGE]`.
  - **UDP re-ask condicionado**: `OP_REASKFILEPING` solo se emite si la
    sesión en cola remota tiene más de ~20 min (último intento de conexión
    antiguo), además de la deduplicación de 21 min existente.
  - **`maxSources` por archivo**: `FT_MAXSOURCES` (0x13) leído en la importación
    `.part.met` y reescrito al guardar; columna `downloads.max_sources`,
    campo `maxSources` en las instantáneas, RPC `downloads.setMaxSources`;
    limita la cuota de pares, la puerta de reconsulta y el umbral de purga.
  - **UI cableada**: campo «Src máx» (diálogo de edición `FT_MAXSOURCES`)
    en la expansión Fuentes; chip de prioridad `DownloadPriorityChip`
    directamente en cada fila de descarga (diseños completo y compacto —
    sin necesidad de desplegar); pestaña **«Compartidos»** dedicada (`/shares`,
    `SharesView`) con selector de prioridad de subida (powershare) e
    interruptor activado/suspendido por archivo; sección «Amigos» en Ajustes
    (adición/eliminación de userhash → friend slot); columnas «Puntuación» y
    «Crédito ×n.nn» + insignia «bloqueado» en la cola de subida.
  - **A4AF por eventos + periódico**: `a4af_event_pass` en cada tick —
    una fuente marcada NNP/FNF ve borrado su cooldown en los demás
    archivos donde es candidata (equiv. `SwapToAnotherFile` inmediato);
    el rebalanceo completo pasa a la cadencia eMule de 8 min
    (`A4AF_FULL_INTERVAL`).

- **Alineación de protocolo con eMule 0.72a** (`mule-protocol`, `mule-daemon`):
  - **Referencia de verdad**: `D:\eMule0.72a-Sources\eMule0.72a-Sources\srchybrid`
    sustituye a eMule 0.50a como referencia normativa (docs `docs/ed2k/`,
    `AGENTS.md`, README actualizados; las menciones 0.50a restantes son de
    procedencia histórica asumida).
  - **Perfil `Emule072a`**: nueva variante `Ed2kCompatibilityProfile::Emule072a`,
    ahora el **perfil por defecto**. `Emule050a` se conserva como variante
    histórica (asignaciones de red idénticas).
  - **Opcodes AICH corregidos**: `0x12`–`0x15` (erróneos) → `0x9B`–`0x9E`
    (`opcodes.h:272-275`).
  - **Nuevos códecs**: `OP_HASHSETREQUEST2`/`ANSWER2` (`0xB1`/`0xB2`,
    `FileIdentifier` + opciones), familia `OP_MULTIPACKET*` (`0x92`/`0x93`/`0xA4`/
    `0xA9`/`0xB0`), `OP_QUEUERANK`/`QUEUERANKING`/`CHANGE_SLOT`, `OP_MESSAGE`,
    `OP_FILEDESC`, `OP_PORTTEST`, `OP_CHATCAPTCHAREQ`/`RES`, `OP_PEERCACHE_*`
    (raw/ignorados). `0x51`/`0x52` marcados *DEPRECATED* como en `opcodes.h:223-224`.
  - **Ext-info `0x58` determinista**: `Ed2kDecoder::extended_requests_version`
    lleva la versión `ExtendedRequestsVersion` negociada mediante `OP_EMULEINFO`
    (0 = ausente, 1 = bitfield, ≥2 = bitfield + fuentes completas) en lugar de
    la antigua heurística por subopcode. Cableada en `peer_session`
    (`status_query`, `transfer`) con la versión del par.
  - **Kad**: `send_bootstrap` inserta el nodo respondedor en los contactos
    (conforme a eMule); `search_sources` deduplica las fuentes por `IP:puerto`
    reportadas por varios contactos.
  - **Robustez de pruebas**: lectura de eventos `downloadAdded`/
    `downloadUpdated` tolerante al orden; fixtures `PeerSource`
    (`software`/`availability`); máscara `FILESTATUS` sobredimensionada realmente
    rechazada; clippy workspace a cero advertencias bajo `-D warnings`.

- **Fachada AppController reducida**: ~31 miembros delegados vueltos
  innecesarios eliminados (getters history/kad/servers/search/netpol/
  settings expuestos ahora vía sus providers). Permanecen: composición
  raíz (client, supervisor, subcontroladores), `initialize`/`refresh`/
  `dispose`, y los miembros aún consumidos (pruebas, `DeepLinkHandler`,
  `filteredDownloads`).

- **Desktop: notificaciones nativas de fin de descarga**:
  `DownloadNotifier` (`apps/desktop/lib/core/download_notifier.dart`)
  se suscribe a `downloadLifecycleEvents` y muestra un toast del SO vía
  `local_notifier` (Windows/Linux/macOS) en `completed` (nombre del archivo)
  y `failed` (nombre + motivo). No-op en las demás plataformas o si el
  notificador del SO no está disponible.

- **Web: botón «Cambiar de demonio»**: nuevo provider
  `onChangeDaemonProvider` (null por defecto — desktop/mobile). La barra lateral
  muestra una acción `link_off` cuando está definido; `eMuleRustApp`
  acepta `onChangeDaemon`. `apps/web` implementa `_disconnect()`:
  libera el controlador y vuelve al formulario de conexión prerrellenado
  con la URL actual.

- **Migración AppController → providers (fin)**: los ~18 widgets restantes
  (diálogos add-link/import/settings/network-policy/ip-filter/lab-guide/
  diagnostic/category/network-mode/download-detail, sidebar, top bar, list,
  row, empty state, details panel, downloads screen) consumen ahora
  los providers Riverpod (`ref.watch`/`ref.read`) en lugar de recibir
  `AppController` como parámetro. `AppNavSection`/`DownloadFilter` movidos
  a `shell_controller.dart` (reexportados por `app_controller.dart` por
  compatibilidad). `AppController.dispose` es idempotente — los subcontroladores
  son liberados por sus `ChangeNotifierProvider`. `AppController` queda
  únicamente como raíz de composición (router, `eMuleRustApp`, pruebas).


- **E5 — Detalle por par + ETA**: `PeerSessionSnapshot.transferRateBps`
  ya existía (mostrado en la tarjeta fuentes del detalle); adición de
  `etaSeconds` al `DownloadSnapshot` ligero (segundos restantes al ritmo
  actual, `None` si nulo/terminado) — poblado en `DownloadItem.etaSeconds`
  sin llamada `downloads.get`, mostrado en la fila de descarga
  (`ETA: 12m 34s` bajo la velocidad).

- **E4 — Fin del polling**: eventos del demonio `kad.statusChanged`,
  `history.entryAdded`, `settings.changed {keys}`, `network.policyChanged`
  (tópicos `kad`/`history`/`settings`/`network`, payload aviso = refetch).
  `AppState::emit_event` = punto de entrada único para los manejadores. En
  cliente: `kadStatusEvents`/`historyEvents`/`settingsEvents`/
  `networkPolicyEvents` + `SettingsChangedEvent`. En UI: el temporizador de 3 s
  de `KadController` se elimina (eventos + obtención al conectar),
  `HistoryController`/`SettingsController`/`NetworkPolicyController` se
  refrescan al recibir evento → sincronización multicliente en vivo.

- **E3 — Ciclo de vida de descargas**: eventos del demonio `event.downloadAdded` /
  `downloadRemoved` / `downloadCompleted` / `downloadFailed` (tópico
  `downloads`), emitidos de forma centralizada por `DownloadService` (`set_event_channel`
  — todas las rutas cubiertas: RPC, deep link, lab, transiciones internas).
  En cliente `DownloadLifecycleEvent` + `downloadLifecycleEvents`; en UI
  `AppController` aplica added/completed/failed vía `upsert`, removed vía
  `removeLocal`, y muestra banner de fin / error de fallo (toast compartido
  desktop + web).

- **E2 — Tick de estadísticas globales**: evento del demonio `event.statsTick`
  (tópico `stats`, ~2 s, suspendido sin suscriptor) con `downloadBps`,
  `uploadBps`, `downloadsActive`, `peersConnected`, `serverConnected`,
  `kadContacts`, `kadUdpBound`. En cliente `DaemonStatsModel` +
  `statsEvents`; en UI `StatsController` (provider dedicado) y el
  `TransferRateTracker` prefiere los caudales agregados del demonio cuando hay un tick
  disponible (respaldo: sumas locales).

- **E1 — Subidas en tiempo real**: evento del demonio `upload.uploadsUpdated`
  (tópico `uploads`, instantánea activa + cola + caudal agregado, emitido tras cada
  mutación del upload manager, progreso limitado ~500 ms). En cliente
  `MuleDaemonClient.uploadsEvents` + `UploadsSnapshotModel`; el cliente falso
  simula una subida activa ~50–250 kB/s + 1 en cola. En UI: nuevo
  `UploadsController` (provider dedicado), sección **Subidas** en la barra lateral
  (`/uploads`), `UploadsView` con tablas sesiones activas / cola de espera
  (cancelación vía `uploads.cancel`), y la insignia UL + sparkline de la barra
  superior conectados al caudal real (fin del `() => 0`).

- **Arquitectura compartida — `packages/app_shell`**: todo el código UI/app
  (`AppController`, providers, router, vistas/funcionalidades, l10n) se extrae de
  `apps/desktop` a `packages/app_shell` (puro de plataforma). Los puntos de
  contacto con el SO pasan por abstracciones: `DaemonLifecycle` (implementado
  por `DaemonSupervisor` desktop), `WorkDirInfo`/`WorkDirMode`, y los
  helpers condicionales `systemActions`/`pickFolder` (stub web / impl IO).
  `eMuleRustApp` es compartido; `apps/desktop/lib/main.dart` pasa a ser un
  punto de entrada ligero (window/tray/deep-links + supervisor).
- **`apps/web` — controlador remoto**: nueva app Flutter web que se
  conecta a un `mule-daemon` existente mediante WebSocket. Resolución de la URL
  por orden: `?daemon=ws://…` > `--dart-define=DAEMON_URL` > preferencia
  `daemon.url` persistida > formulario de conexión
  (`DaemonConnectScreen`, sugerencia `ws://<host>:4660/rpc`, validación
  ws/wss, `?daemon=` vacío reinicia). `flutter build web` + dry-run
  WASM validados.

- **Pulido Desktop (P6, inicio)**:
  - `file_picker` nativo sustituye al hack PowerShell en
    `lib/core/folder_picker.dart` (Windows/Linux/macOS/Android).
  - `window_manager`: `lib/core/window_setup.dart` restaura y persiste
    la geometría de ventana (mín. 760×500, shared_preferences `window.*`).
  - `tray_manager`: `lib/core/tray_setup.dart` añade el icono de bandeja
    (Show/Quit) con `assets/app_icon.ico`.
  - Rutas profundas (`lib/app/router.dart`): `/downloads/:id` abre el
    panel de detalle de una descarga (la URL pasa a ser la fuente de verdad
    para la selección — clic fila → `router.go`, cierre → vuelta a
    `/downloads`), `/settings` abre el diálogo Ajustes sobre la
    sección descargas, `usePathUrlStrategy()` activado en la web.
  - Gráfica de caudal: `TransferRateTracker` (`lib/app/`) muestrea el
    caudal agregado cada segundo (ventana deslizante 60 s, temporizador perezoso
    iniciado en el primer listener); `SpeedSparkline` (CustomPainter, sin
    dependencia de gráficas) mostrado en la insignia de velocidad de la barra superior.
  - Enlaces `ed2k://`: `app_links` + `lib/core/deep_link_handler.dart`
    (enlace inicial + stream → `addEd2kLink`); registro Windows mediante
    `windows/ed2k_protocol.reg` (o el instalador), `.desktop`
    `x-scheme-handler/ed2k` para Linux, intent-filter/URL types para
    Android/iOS.
  - Runners **Android** e **iOS** generados (`com.eMuleRust.app`):
    `AndroidManifest.xml` con permiso `INTERNET` + intent-filter
    `ed2k`, `Info.plist` con `CFBundleURLSchemes` `ed2k`, nombre de app
    `eMuleRust`. Los servicios desktop (`WindowSetup`, `TraySetup`) ya son
    no-op fuera de escritorio; `pickFolder` excluye iOS (sin selector de
    carpetas nativo).

### Añadido — Rediseño UI multiplataforma (rama `ui-ux`)

- **`packages/mule_client`**: extracción del cliente del demonio a un paquete
  Dart compartido (interfaz `MuleDaemonClient`, transporte `RpcDaemonClient`
  WS/JSON-RPC, `FakeMuleDaemonClient`, modelos de dominio, DTOs). Nuevo:
  reconexión automática opt-in (`autoReconnect`, backoff 0,5 s→10 s +
  re-`hello` + resync), keep-alive `system.status` cada 20 s
  (detección de conexiones zombi), gestión `event.snapshotRequired`,
  pruebas del cliente movidas al paquete.
- **`packages/ui_kit`**: sistema de diseño compartido — `AppTheme`,
  `Formatters`, `AppStrings`, `PartAvailabilityBar`.
- **Demonio — eventos RPC endurecidos**: `DaemonEvent` pasa a ser la fuente única
  del formato de red (`method()`, `topic()`, `send()` — usado por todos los
  emisores); `events.subscribe {topics}` filtra por sesión
  (`downloads`/`peers`/`servers`/`search`/`network`); un cliente rezagado
  recibe `event.snapshotRequired {skipped}` en lugar de perder eventos
  silenciosamente; `DownloadUpdatedEvent` (duplicado) eliminado.
- **UI desktop**: corrección del desbordamiento del selector de destino de
  búsqueda (scroll horizontal en modo compacto, espaciados ajustados);
  watchdog de búsqueda de 30 s cancelado en cuanto se reciben resultados.
- **Navegación `go_router` + shell adaptable (P3)**: `MaterialApp.router`

  con rutas declarativas por sección (`/downloads`, `/history`,
  `/servers`, `/search`, `/kad`), `ShellRoute` inyectando el contenido en el
  shell adaptable (`DownloadsScreen`: barra lateral ≥1100 px, raíl compacto
  750–1100 px, drawer <750 px). El `ShellController` sigue siendo la fuente de
  verdad de la sección: sincronización VM→URL mediante listener (`router.go`), sincronización
  URL→VM (deep links, listos para web) mediante adopción de la ruta en el
  shell. Las vistas de sección tienen ahora cada una su `ListenableBuilder`.
- **Clean architecture UI**: design tokens en `ui_kit`
  (`AppColors`/`AppSpacing`/`AppRadius`/`AppTextStyles`/`AppBreakpoints`/
  `AppDecorations` — ya no hay `Colors.*` codificados de forma fija en las vistas);
  `downloads_screen.dart` dividido en widgets dedicados (`DownloadsTopBar`,
  `DownloadsList`, `DownloadsEmptyState`, `DownloadDetailsPanel`,
  `StatusNotificationBanner`); convenciones en
  `docs/architecture/conventions_ui.md`.
- **i18n (P5)**: `gen_l10n` activado — `l10n.yaml`, ARB
  `lib/l10n/app_fr.arb` (plantilla = etiquetas actuales) + `app_en.arb`
  (traducción), salida `lib/l10n/generated/`. Selector de idioma FR/EN
  en Settings (`LocaleController` + `localeControllerProvider`, persistencia
  `shared_preferences` clave `app.locale`). Extensión `context.l10n`
  (+ `DownloadFilterL10n`). `AppStrings` (ui_kit) está obsoleto (`deprecated`).
  **Pasada completa**: ~400 literales visibles migrados a `context.l10n`
  en ~90 archivos (diálogos, tablas, barras de herramientas, banners, estados vacíos),
  con marcadores de posición `{param}` para los textos dinámicos. Los literales
  técnicos (URLs `ed2k://`, rutas, formatos puros interpolados) permanecen
  codificados de forma fija intencionadamente. Las pruebas que montan un `MaterialApp` en bruto deben
  proporcionar `AppLocalizations.localizationsDelegates` + `supportedLocales`
  (+ `locale: Locale('fr')` cuando verifican etiquetas FR).
- **Migración a Riverpod (P4, división del VM completada)**: `ProviderScope`
  en `eMuleRustApp` (`muleDaemonClientProvider`,
  `daemonSupervisorProvider`, `devToolsEnabledProvider`,
  `appControllerProvider`). El antiguo `DownloadsViewModel`, renombrado a
  `AppController` y movido a `lib/app/app_controller.dart` (1781 → ~740
  líneas) se divide en **10 controladores por subdominio**, cada uno en
  su archivo `features/<domaine>/view_models/` y expuesto mediante un
  `ChangeNotifierProvider` dedicado: `DownloadsStoreController` (almacén,
  eventos, CRUD, selección, part.met, diagnóstico), `HistoryController`,
  `KadController`, `SearchController` (watchdog 30 s, deduplicación por hash),
  `ServersController` (lista + sesión), `PeerSourcesController`,
  `CategoriesController`, `NetworkPolicyController` (kill switch, filtro
  IP, límites de tasa), `SettingsController`, `LabController`. El VM
  sigue siendo un orquestador ligero (conexión/supervisor, sección, banners)
  que delega y reenvía las notificaciones — sin ninguna vista ni prueba rota.
  Pendiente: migrar las vistas a los providers (`HistoryView` y `KadView`
  ya usan `Consumer*`), y después eliminar la fachada.
- **Documentación**: `docs/architecture/rpc_protocol.md` (contrato JSON-RPC completo)
  y `docs/architecture/plan_ui_multiplateforme.md` (hoja de ruta escritorio/
  móvil/web en 10 fases).

### Añadido — Notas Kad (comentarios/valoraciones de archivos) y herramientas MCP

- **Notas Kad — modelo de red y códec acotados** (`crates/mule-protocol/src/kad`): `KadNoteWire` (entrada `CEntry` con `m_bSource = false`), decodificadores/codificadores para `KADEMLIA2_SEARCH_NOTES_REQ` (`0x35`, `[target:16][file_size:u64]`), `KADEMLIA2_PUBLISH_NOTES_REQ` (`0x45`, `[target][source:16][tag_count:u8][tags]`), las respuestas de notas sobre el `KADEMLIA2_SEARCH_RES` unificado (`0x3B`, forma emitida por `CIndexed::SendValidNoteResult`) y el `KADEMLIA_SEARCH_NOTES_RES` heredado (`0x3A`). Listas de etiquetas genéricas acotadas: etiquetas conocidas decodificadas en `Ed2kTag`, desconocidas preservadas en `RawTag`, contador limitado a 32 — los comentarios siguen siendo datos de visualización no confiables.
- **Notas Kad — índice de almacenamiento** (`kad_udp/storage.rs`): mapa `target → source_id → entrada` (una nota por publicador y por archivo), TTL de 5 h, límite de 100 notas/archivo, filtrado de `file_size` en lectura (`SendValidNoteResult`), informe de `load` (0-100) en `KADEMLIA2_PUBLISH_RES`. Manejadores: `SearchNotesReq` atendido, `PublishNotesReq` rechazado cuando el nodo está tras cortafuegos UDP (guardia de eMule), respuesta `0x4B` enrutada hacia las transacciones de notas mediante `expected_response`.
- **Notas Kad — persistencia SQLite**: tabla `kad_notes_index` (`file_hash`,`source_id` PK) que almacena la lista de etiquetas en **blob de red** (`encode_note_tags_blob`/`decode_note_tags_blob`) — las etiquetas arbitrarias/desconocidas sobreviven idénticas a un reinicio; purga por TTL y rehidratación al arrancar el socket Kad.
- **Notas Kad — búsqueda y publicación salientes**: `send_search_notes`/`send_publish_notes` (transacciones `KadUdpService` con correlación in-flight); `KadService::search_notes` consulta a los 20 contactos más cercanos al hash del archivo y agrega las notas deduplicadas por `source_id`; `publish_note` envía una lista de etiquetas acotada (`filename`/`filesize`/`comment`/`rating`, comentario ≤ 512 B). RPC: `kad.searchNotes`, `kad.publishNote`. Detalle sutil: al compartirse `0x3B` entre fuentes/palabras clave/notas, el bucle UDP **redecodifica** el payload en bruto en forma de notas cuando una transacción espera `SearchNotesRes` (de lo contrario las listas de etiquetas se aplanarían en entradas de palabras clave).
- **MCP**: nuevas herramientas `emule_kad_search_notes` y `emule_kad_publish_note`; `DaemonClient::call` aplica un tiempo de espera de 60 s a los métodos `kad.search*`/`kad.publish*` (las búsquedas iterativas superan los 15 s en WAN), expuesto mediante `call_with_timeout`.
- Pruebas: códec de notas (ida y vuelta `0x35`/`0x45`/`0x3B`/`0x3A`, malformados, límite de etiquetas); almacén de notas (upsert, límite, filtro de tamaño, load); integración `kad_udp`: publish → `0x4B` load → search → `0x3B` con comentario/rating → filtro de tamaño.

### Añadido — Búsqueda global en servidor `GLOBSEARCHREQ3` (eMule 0.72a)

- **Códec** (`mule-protocol/src/ed2k/server_udp.rs`): `encode_glob_search_req3` (`0x90`) antepone al árbol de búsqueda REQ2 una etiqueta `CT_SERVER_UDPSEARCH_FLAGS` (`0x0E`) = `SRVCAP_UDP_NEWTAGS_LARGEFILES` (`0x01`) escrita en formato `WriteNewEd2kTag` (tipo `|0x80`, nombre de 1 byte) — declara al servidor el soporte de etiquetas de nuevo formato y de tamaños de 64 bits; `OP_GLOBSERVSTATRES` decodificado por completo (`challenge`, `users`, `files`, `udp_flags`, puertos de ofuscación TCP/UDP), constantes `SRV_UDPFLG_EXT_GETFILES`/`SRV_UDPFLG_LARGEFILES` expuestas.
- **Demonio** (`services/server_udp.rs`): caché de `udp_flags` por dirección UDP de servidor, alimentada por `OP_GLOBSERVSTATRES` (respuesta aceptada únicamente si el desafío coincide con un `GLOBSERVSTATREQ` emitido — guardia antisuplantación de eMule); `send_servstat_req` sondea a un servidor de capacidades desconocidas antes de la búsqueda; `send_glob_search` elige `REQ3` si `EXT_GETFILES` **y** `LARGEFILES` (lógica `SearchResultsWnd.cpp`), `REQ2` en caso contrario, y devuelve la forma emitida.
- **RPC**: `servers.search` devuelve ahora `udpTargets[]` (`{address, request: "REQ3"|"REQ2"|null}`) para observar la selección en pruebas UI/MCP, y acepta `udpOnly` (alcance «Global» de eMule: fan-out UDP hacia la lista de servidores sin búsqueda TCP en el servidor conectado).
- **UI escritorio (banco de pruebas)**: tercer destino de búsqueda «Global UDP» (`udpOnly`), banner informativo sin conexión requerida y resumen `UDP: n×REQ3, m×REQ2` mostrado en la notificación de estado tras el envío.
- Pruebas: red REQ3 (etiqueta NewEd2k + payload literal), decodificación `GLOBSERVSTATRES` completa y forma corta, selección REQ3/REQ2/sondeo por socket UDP real en loopback.

### Corregido — Búsqueda global UDP

- **Selección REQ1/REQ2/REQ3**: eMule envía `OP_GLOBSEARCHREQ` (`0x98`) a los servidores sin `SRV_UDPFLG_EXT_GETFILES` — esos servidores ignoraban silenciosamente el REQ2 que les enviábamos; la selección tiene ahora tres niveles (REQ3 → REQ2 → REQ1) y un servidor desconocido se sondea con `GLOBSERVSTATREQ` y luego recibe REQ1.
- **`OP_GLOBSEARCHRES` mal decodificado**: el payload UDP `0x99` es un flujo de entradas desnudo (`[hash16][clientid][port][tagcount][tags]` repetido), sin el prefijo `u32` count del `OP_SEARCHRESULT` TCP — los datagramas recibidos eran rechazados por el decodificador TCP; nuevo `decode_glob_search_res` validado sobre un datagrama real capturado.
- **Observabilidad**: espaciado de 50 ms entre envíos UDP (eMule espacia sus peticiones) y registros `unknown opcode`/`decode error` elevados a INFO en el socket server-UDP.

### Añadido — Disponibilidad por parte (mapa eMule)

- `downloads.getDetails` expone `availableSourceCount` por parte: número de fuentes conocidas cuya `PartAvailabilityMask` cubre la parte (`0` = parte no encontrada → descarga bloqueada).
- `downloads.list` devuelve ahora los `DownloadDetail` completos (estructura aplanada compatible) para que la lista muestre la barra de disponibilidad por parte sin llamada dedicada.
- UI: `PartAvailabilityBar` (segmento por parte posicionado a escala de los desplazamientos) bajo cada fila de descarga — rojo = ninguna fuente, tonos de azul = rara → común, verde azulado = verificada; tooltip `Part N · estado · Fuentes: X`. Colores compartidos con el mapa Part Map del detalle.

### Modificado — Capa de eventos RPC (para `packages/mule_client`)

- `DaemonEvent` pasa a ser la fuente única del formato de red (`services/events.rs`): cada variante lleva su `method`, su `topic` de suscripción y se serializa mediante `to_wire_json()`; todos los emisores pasan por `DaemonEvent::send` (`DownloadUpdatedEvent` eliminado).
- `events.subscribe` es realmente funcional: interceptado en `handle_socket`, muta el filtro de temas **por sesión** (`downloads`, `peers`, `servers`, `search`, `network`, `other`; `null` = todo, retrocompatible). No autenticado → `-32000` como cualquier otro método.
- `RecvError::Lagged` en el broadcast ya no se ignora: el demonio emite `event.snapshotRequired {skipped}` para forzar una resincronización completa en el cliente.
- Documentación: `docs/architecture/rpc_protocol.md` §6 documenta la tabla método → tema, la semántica de `events.subscribe` y `snapshotRequired`; §9 describe `packages/mule_client` (reconexión automática, keep-alive, resincronización).

### Corregido — Auditoría y rediseño UX de la página Ajustes (`/settings`)

Auditoría completa del cableado UI → cliente RPC → manejador del demonio → persistencia →
consumo real (referencia actualizada: `docs/SETTINGS.md`).

- **UPnP**: `RpcDaemonClient.mapUpnpPorts` ya no envía los puertos codificados
  de forma fija `39735/39737` — al omitir los parámetros, el demonio mapea los
  puertos efectivos de la instantánea (rastreados por `upnp.set_ports`).
- **`capacityDownloadKbs` funcional** (estaba inerte): límite de eMule
  (`Preferences.cpp`: límite DL ≤ capacidad × 9/10, UL ≤ capacidad × 4/5,
  `0` = ilimitado sin acotar) aplicado en `limits.set`
  (`clamp_to_capacity`), al cambiar la capacidad mediante `settings.set`
  (`apply_capacity_clamp`), y **normalizado al arranque** en `AppState::new`
  — un límite persistido por encima de la capacidad se acota y se reescribe
  en SQLite para reflejar el valor efectivo.
- **Entradas inválidas notificadas**: `int.tryParse` fallaba silenciosamente
  (clave omitida del parche, toast «actualizado» engañoso) — las secciones
  Conexión/Transferencias/Seguridad validan ahora cada campo (límites,
  no vacío) y notifican el error en el banner global.
- **Menores del demonio**: sonda de escritura sobre `tempDir` (como
  `defaultDownloadDir`), aviso `tracing` sobre las claves `settings.set`
  desconocidas, `maxUploadQueue` acotado en UI ≥ 10.

### Modificado — Localización completa de `/settings`

- ~120 nuevas claves ARB FR/EN (`settings*`, `network*`, `filter*`,
  `upnp*`, `friends*`, descripciones de perfiles de red); las 6 secciones,
  el raíl, las tarjetas de red (`NetworkStateBanner`, `UpnpStatusCard`,
  `NetworkModeStatusCard`, `RateLimitsTab`, `NetworkProfileSelector`,
  `IpFilterTab`, informe de importación) y los widgets reutilizados
  (`SettingsDirectorySection`, `SettingsFriendsSection`) pasan por
  `context.l10n` — el selector de idioma traduce ahora toda la pantalla.
- Mensajes de estado de los controladores (`SettingsController`,
  `NetworkPolicyController`) pasados a español/idioma correspondiente, coherentes con el resto
  del shell.
- Correcciones preexistentes: `btnCancel`/`cancel` FR eran «Cancel»
  → «Annuler»; `ajouter` EN era «Ajouter» → «Add».

### Modificado — UX de guardado y sincronización de los Ajustes

- **Comportamientos homogéneos**: Intro envía en todos los campos (incluido
  Transferencias y los límites de tasa); botones «Aplicar» por
  tarjeta en Conexión/Transferencias, icono de guardado en los campos
  libres.
- **Seguimiento de cambios + resincronización**: todos los `TextEditingController` se
  resincronizan ante `settings.changed` / cambios de política mediante
  `ref.listen` — un campo en edición (dirty) nunca es
  sobrescrito por un valor remoto.
- **Desplegable de retención robusto**: un valor persistido fuera de los elementos
  estándar (p. ej., 60 días mediante `emule_update_settings` MCP) se muestra en
  un elemento dedicado en lugar de lanzar una aserción en `DropdownButton`.
- **Kill switch**: diálogo de confirmación antes de cortar todos los
  sockets (`network.disconnectAll`).
- **Seguridad**: ya no hay spinner a pantalla completa en cada conmutación — barra
  de progreso lineal durante las operaciones.
- **Varios**: sección Amigos en `SettingsSectionScaffold`, `Scrollbar`
  explícita en el panel, interruptor de barra de depuración desactivado cuando las herramientas de
  desarrollo no están disponibles, nota explicativa capacidad ↔ límite en
  la pestaña de tasas.
- Pruebas UI actualizadas: `import_part_met_dialog_test` espera «Annuler»,
  `network_policy_ui_test` confirma el diálogo del kill switch.

### Modificado — Compartición parcial: publicación en servidor/Kad y protecciones

Fidelidad con eMule 0.72a para los `.part` en curso (`CPartFile`):

- **Oferta al servidor (`OP_OFFERFILES`)**: la lista enviada al iniciar sesión se ordena
  por prioridad de subida decreciente (`file_prio_num`, como
  `SendListToServer`) y se limita a 200; los archivos **incompletos**
  llevan ahora el par de marcadores `0xFCFCFCFC`/`0xFCFC` (frente a
  `0xFBFBFBFB`/`0xFBFB` para los completos) cuando el servidor soporta
  `SRV_TCPFLG_COMPRESSION` — `OfferedFile::for_server` toma un parámetro
  `is_partial` y exporta `OFFER_INCOMPLETE_CLIENT_ID`/`_PORT`.
- **Republicación Kad**: los parciales se publican como **fuentes**
  (`CKnownFile::PublishSrc` — los pares pueden descargar las partes ya
  completadas) pero se les deniega la publicación de *keywords* («someone
  else should have the full file»).
- **Mutaciones denegadas sobre parciales**: `shares.setEnabled` y
  `shares.setUploadPriority` devuelven un error de negocio sobre un
  `is_partial` — persistir una compartición transitoria congelaría su ruta
  `.part` en `shared_files` y la serviría más adelante como completa
  (eMule no autoriza dejar de compartir un parcial en curso).
- **`setUploadPriority` robustecido**: resolución `get_shared_file` y luego
  upsert — funciona también sobre descargas `Completed` nunca persistidas
  en `shared_files`.
- **Corrección cliente `shares.list`**: `RpcDaemonClient.listSharedFiles`
  esperaba una lista JSON desnuda mientras el demonio devuelve
  `{sharedFiles: [...]}` — la pestaña Compartidos mostraba una lista vacía;
  el analizador acepta ahora ambas formas.
- **UI Compartidos**: `SharesView` se suscribe a los eventos de ciclo de vida
  (refresco inmediato) y a los eventos de progreso (debounce de 3 s)
  para seguir las `completedParts` de los parciales.

## [0.1.9] - 2026-09-20

### Añadido — Interoperabilidad eMule 0.72a (funciones esenciales de la red eD2K/Kad)

Auditoría comparativa del demonio frente a las fuentes oficiales **eMule 0.72a** (`D:\eMule0.72a-Sources`), y posterior implementación de todas las funciones bloqueantes para transferencias eD2K/Kad reales:

- **Verificación incremental por parte (Hashset MD4)**: códec completo `OP_HASHSETREQUEST` (`0x51`) / `OP_HASHSETANSWER` (`0x52`) en `mule-protocol`; petición automática al conectar con una fuente; verificación MD4 al completarse cada parte de 9 728 000 bytes con descarte selectivo de las partes corruptas (`CPartFile::IsCorruptedPart`) en lugar del fallo global del archivo; tabla SQLite `file_part_hashes`; servicio `OP_HASHSETANSWER` en el lado de subida para los archivos compartidos (cálculo `spawn_blocking` + verificación del hash raíz).
- **AICH conectado a la red**: códec de los 4 opcodes (`OP_AICHREQUEST 0x12`, `OP_AICHANSWER 0x13`, `OP_AICHFILEHASHREQ 0x14`, `OP_AICHFILEHASHRES 0x15`); construcción/streaming del árbol de Merkle SHA-1 de eMule (`mule-format/src/aich/recovery.rs`) con la doble granularidad exacta (nodos > 1 parte divididos por `PARTSIZE`, ≤ 1 parte por `EMBLOCKSIZE`, bloques hoja alineados por parte: 52 × 184 320 + 143 360 bytes); consenso de confianza `CAICHUntrustedHash` (≥ 10 IP únicas enmascaradas `0x00F0FFFF`, ≥ 92 %); recuperación de parte corrupta a nivel de bloque de 180 KB mediante datos de recuperación (ident + hash); servicio de ambos opcodes en subida.
- **Publicación de compartidos al servidor (`OP_OFFERFILES 0x15`)**: envío tras `OP_IDCHANGE` con semántica `CreateOfferedFilePacket` — marcadores de finalización `0xFBFBFBFB`/`0xFBFB` bajo compresión de servidor, HighID frente a LowID, filtrado > 4 GiB salvo `SRV_TCPFLG_LARGEFILES` con etiquetas `FT_FILESIZE`/`FT_FILESIZE_HI`, mapeo de tipo de archivo eMule (`Audio`/`Video`/`Image`/`Doc`/`Pro`), lista vacía = keep-alive, límite 200 archivos.
- **Callback LowID**: `OP_CALLBACKREQUEST` (`0x1C`, `u32` desnudo), decodificación `OP_CALLBACKREQUESTED` (`0x35`, `IP+port` + forma extendida crypt/userhash) y `OP_CALLBACK_FAIL` (`0x36`); retención de fuentes LowID (`lowid:<client_id>`) en lugar de rechazo; registro de callbacks pendientes (TTL 5 min); adopción de conexiones entrantes que coincidan con un callback como sesiones de descarga (`PeerSessionHandle._permit: Option<SocketPermit>`); guardia eMule: sin callback si somos LowID.
- **Escritura `.part.met` compatible con eMule**: `PartMetFile::write_to`/`save` con el formato exacto `CPartFile::SavePartFile` (versión `0xE0`/`0xE2` largefile, hashset, etiquetas `FT_FILENAME`/`FT_FILESIZE`/`FT_TRANSFERRED`/`FT_AICH_HASH` base32, huecos en etiquetas nombradas `\x09<i>`/`\x0A<i>`); escritura atómica `.tmp` + rename; copias al añadir, en cada parte completada/descartada y al pausar; borrado al finalizar; **corrección del lector**: resolución del `FT_*` efectivo desde el primer byte del nombre de etiqueta (compatibilidad con huecos nombrados de eMule).
- **Kad — rol de nodo de almacenamiento**: `kad_udp/storage.rs`, almacén acotado y con expiración (TTL 5 h, 5 000 claves, 50 entradas/clave, 20 000 entradas); `PublishSourceReq` validado (IP emisor + puerto TCP obligatorios) y persistido, `SearchSourceReq`/`SearchKeywordReq` servidos desde el almacenamiento con paginación; filtrado de entradas de palabras clave malformadas; **republicación periódica** de los compartidos (fuente + palabras clave) hacia los 20 contactos más cercanos en cuanto la tabla de enrutamiento no está vacía y después cada 5 h (`KADEMLIAREPUBLISHTIMES`).
- **Fin de transferencia limpio**: tratamiento de `OP_CANCELTRANSFER` (`0x56`) entrante (liberación inmediata del slot de subida) y `OP_OUTOFPARTS` (`0x57`) entrante (fin de sesión elegante); emisión de `OP_OUTOFPARTS` cuando no quedan bloques por pedir (`SendOutOfPartReqsAndPacket`) y `OP_CANCELTRANSFER` al cancelar una transferencia en curso.
- **Subida comprimida (`OP_COMPRESSEDPART 0x40`)**: servicio de bloques zlib (`Z_BEST_SPEED`, fragmentos ≤ 10 240 bytes, formato `hash16 + u32 start + u32 tamaño_zlib` por paquete `OP_EMULEPROT`) cuando el par anuncia `nDataCompVer` (MiscOptions1 bits 20-23); respaldo `SENDINGPART` si no es rentable; exclusiones `ShouldCompressBasedOnFilename` (`zip/rar/7z/cbz/cbr/ogm/ace`, bloques ≤ 31 B); `has_compression_capability` lee ahora MiscOptions1 además de `OP_EMULEINFO`.
- **Múltiples slots de subida**: `UploadManager` soporta hasta `MAX_ACTIVE_UPLOADS` (10) slots concurrentes (`active_uploads: HashMap`) con promoción de la cola ponderada por créditos en cada liberación; `max_active` configurable; `uploads.list` devuelve todos los slots activos.
- **Socket cliente-UDP (eD2K UDP)**: canal `PR_EMULE` compartido con el socket Kad (multiplexación eMule); `OP_REASKFILEPING`/`OP_REASKACK`/`OP_FILENOTFOUND`/`OP_QUEUEFULL` (`0x90`–`0x93`) — entrante: respuesta con rango de cola (`GetWaitingClientByIP_UDP`, part-status según `m_byUDPVer` del solicitante), `OP_FILENOTFOUND`, `OP_QUEUEFULL` si la cola está casi llena, silencio en caso contrario; saliente: keep-alive ~21 min hacia las fuentes `Queued` que hayan anunciado un puerto UDP (`CT_EMULE_UDPPORTS` capturado en el handshake), deduplicación por `(IP, hash)`; rango aprendido mediante `OP_REASKACK` aplicado a la sesión tras validar los reasks pendientes (`UDPPacketPending`).
- **Planificador global de ancho de banda**: los bucles de transferencia de descarga regulan los bytes recibidos mediante `PolicyRateLimiters.download_bucket` (en bruto y zlib, sesiones salientes y callback); la subida existente adquiere ahora el bucket global en cuantos de ≤10 240 bytes (equidad entre slots tipo `UploadBandwidthThrottler`); límites configurables mediante la API `network` existente.
- **Servidor — lista, UDP y conmutación por fallo**: `OP_SERVERLIST` (`0x32`) decodificado y fusionado en `ed2k_servers` (preferencia 0, entradas `ip:port` deduplicadas por upsert); nuevo `ServerUdpService` en socket UDP dedicado (bind efímero, como `UDPSocket` de eMule) con códec `server_udp` — `OP_GLOBGETSOURCES2` (`0x94`) emitido hacia el servidor conectado (puerto UDP = TCP+4) y `OP_GLOBFOUNDSOURCES` (`0x9B`)/`OP_GLOBSERVSTATRES` (`0x97`) decodificados, fuentes persistidas como `PeerSource`; conmutación por fallo automática en `SwarmCoordinator` — reconexión hacia otro servidor `allowed` de la lista (máxima preferencia, exclusión del servidor caído, backoff 60 s global + 5 min por dirección) cuando cae la conexión con el servidor.
- **Republicación periódica de `OP_OFFERFILES`**: la lista de compartidos se reemite cada 30 min mientras la sesión de servidor está activa (captura del último `OP_IDCHANGE` para `client_id`/`tcp_flags`) — los archivos que pasan a ser compartibles durante la sesión se publican sin reconexión; un paquete `OP_OFFERFILES` vacío sirve de keep-alive (semántica `KeepConnectionAlive` de eMule).
- **Búsqueda UDP global** (`OP_GLOBSEARCHREQ2`/`GLOBSEARCHRES`): `servers.search` emite ahora la petición en TCP *y* en UDP (`0x92`, payload = expresión de búsqueda reutilizada literalmente como `SearchResultsWnd.cpp`) hacia el servidor conectado + hasta 9 servidores `allowed` de la lista (puerto UDP = TCP+4); `OP_GLOBSEARCHRES` se decodifica mediante el códec `OP_SEARCHRESULT` y se emite como notificación `server.searchResult` idéntica a la ruta TCP.
- **USS (Upload Speed Sense)**: `services/uss.rs` — límite de subida dinámico guiado por la latencia, ley de control fiel a `LastCommonRouteFinder` (`soll_ping` tolerancia % ×4 o 200 ms, `hping` sobre mediana móvil, divisores 1000 con rampa de 60 s, cadencia `max(125 ms, 6400/upload)`, reajuste base tras 60 fallos); sonda RTT mediante conexión TCP hacia el servidor conectado o `pingHost` configurado (los sockets raw ICMP de eMule exigirían permisos de administrador); `accept_new_client` compartido con `UploadManager` (`set_uss_gate`) — los nuevos clientes permanecen en cola cuando la latencia supera la tolerancia; configuración persistida (`uss_config`), expuesta y modificable mediante `limits.get`/`limits.set` (`uss{...}`).
- **Identidad de subida por userhash**: la cola de espera y los slots de subida usan ahora como clave `uh:<userhash>` (anuncio `OP_HELLO`) en lugar de `ip:port` — un par reconectado desde otra IP conserva su rango, y los clientes tras una misma IP/NAT siguen siendo identidades distintas (deduplicación por identidad como `CUploadQueue`); `release_slot` acepta la clave *o* la dirección `ip:port` (compatibilidad RPC `uploads.cancel`), `waiting_rank_by_ip` (respuestas `OP_REASKACK`) sigue funcionando mediante el `client_address` almacenado; los créditos siguen indexados por clave pública SUI.
- **Prueba de accesibilidad UDP Kad (`KADEMLIA2_FIREWALLUDP` / `OP_FWCHECKUDPREQ`)**: semántica `CUDPFirewallTester` — el demonio solicita hasta 2 contactos Kad verificados (versión ≥ 6, puerto TCP conocido, IP distintas) mediante una conexión TCP eD2K (`OP_HELLO` anunciando `KadVersion=6` en MiscOptions2, requisito `> KADEMLIA_VERSION5_48a`), envía `OP_FWCHECKUDPREQ` (`0xA7`, PR_EMULE: `u16 intern` + `u16 extern` + `u32 key`), y luego correlaciona los datagramas `0x62` (`u8 error` + `u16 puerto probado`) por IP candidata; un paquete válido (`error=0`, puerto = intern/extern anunciado) → estado **Open** (puerto externo NAT detectado → `use_extern`), todos los candidatos fallidos → **Firewalled**, ningún candidato → **Unverified**, presupuesto global ~6 min; paquetes no solicitados ignorados (regla `bRequested` de eMule); en el lado servidor, el despachador entrante responde a los `OP_FWCHECKUDPREQ` emitiendo `FIREWALLUDP` hacia el puerto interno y luego externo del solicitante; estado expuesto en `kad.status` (`udpFirewallStatus`, `udpFirewallVerified`, `externUdpPort`, `effectiveUdpPort`); el `KadVersion` anunciado en el hello normal pasa de 2 a 6 (el demonio sirve ahora el protocolo de pruebas); pruebas: códec 0x62, rastreador (entrega/no solicitado/IP errónea/puerto efectivo), e2e loopback mock peer → `Open`.
- **Descifrado de las respuestas `FIREWALLUDP` (compatibilidad WAN real)**: los pares eMule cifran el datagrama de respuesta con el esquema `EncryptSendClient` de `EncryptedDatagramSocket` — `RC4(MD5(receiverKey:u32 ‖ randKeyPart:u16))` a partir del byte 3, magic `0x395F2EC1`, cabecera `[proto][rand:u16][magic:u32][pad][recvKey:u32][sendKey:u32]` — lo que producía descartes «invalid protocol byte» en la WAN; el bucle de recepción `kad_udp` intenta ahora el descifrado (clave = la clave emisora enviada en cada `OP_FWCHECKUDPREQ`, conservada por el rastreador) antes de rechazar un primer byte desconocido, y los paquetes descifrados se reintegran en la ruta de decodificación normal; la clave de verificación UDP se genera por IP destino según `GetUDPVerifyKey` (MD5 de la clave Kad local ‖ IP, XOR de las palabras, no nula); pruebas e2e: el par simulado responde cifrado → `Open`.
- **Persistencia del almacenamiento Kad**: las entradas indexadas aceptadas (`PublishSourceReq`/`PublishKeywordReq`) se escriben en espejo en `kad_sources_index`/`kad_keywords_index` (TTL 5 h); al arrancar el socket Kad, las filas expiradas se purgan y luego el almacén en memoria se rehidrata mediante `list_all_kad_indexed_sources`/`list_all_kad_keyword_entries` preservando el TTL restante — el rol de nodo de almacenamiento sobrevive a los reinicios.
- **Buddy callback Kad (LowID mediante relé Kad)**: implementación de los tres roles eMule (`CClientList::GetBuddy` / `IncomingBuddy` / `TryToConnect CCS_KADCALLBACK`) —
  - *Relé*: `KADEMLIA_FINDBUDDY_REQ` (`0x51`) aceptado únicamente si el nodo está verificado sin cortafuegos (estado `FIREWALLUDP` Open) y sin buddy activo → `KADEMLIA_FINDBUDDY_RES` (`0x5A`, `tcp_port` + opciones); registro pendiente (TTL 120 s); adopción de la conexión TCP del solicitante como *enlace buddy* (`serve_buddy_link`) con keep-alive `OP_BUDDYPING`→`OP_BUDDYPONG`; `KADEMLIA_CALLBACK_REQ` (`0x52`) retransmitido por este enlace como `OP_CALLBACK` (`0x99`, `check16+file16+ip4+port2`); `OP_REASKCALLBACKUDP` (`0x94`, socket cliente-UDP) retransmitido como `OP_REASKCALLBACKTCP`.
  - *Solicitante*: `TAG_BUDDYHASH` (`0xF8`) extraído de las respuestas `SearchSourceRes` y persistido (`PeerSource.buddy_id`, columna SQLite `buddy_id`); las fuentes Kad LowID (`tcp_port=0` + buddy) se conservan bajo la dirección `kadcb:<ip>`; el `SwarmCoordinator` resuelve el buddy en la tabla de enrutamiento y emite `KADEMLIA_CALLBACK_REQ`; los callbacks entrantes se adoptan por IP remota (`pending_kad_callbacks`, TTL 5 min) como sesión de descarga.
  - *Devolución de llamada de servidor*: `OP_CALLBACKREQUESTED` entrante desencadena ahora una conexión saliente (`connect_back`) hacia la IP/puerto del solicitante — el handshake eD2K completo y luego el servicio de las peticiones de subida, en lugar de un simple registro.
  - Pruebas: códec `0x51`/`0x52`/`0x5A` (ida y vuelta + truncado), aceptación/rechazo `FINDBUDDY_REQ` (no verificado → rechazo silencioso), registro pendiente, descarte `CALLBACK_REQ` sin enlace buddy.
- **Descompresión de paquetes de servidor `OP_PACKEDPROT` (`0xD4`)**: los servidores eD2K comprimen con frecuencia `OP_SEARCHRESULT`/`OP_FOUNDSOURCES` en zlib — el decodificador los rechazaba (`CompressedProtocolUnsupported`), lo que hacía que estos servidores parecieran mudos y dejaba la búsqueda UI en espera indefinida. TCP: la sesión de servidor descomprime el payload antes del despacho (`Packet::UnPackPacket` de eMule, límite estricto de 250 KB, opcode preservado, proto restaurado a `PR_ED2K`); UDP: `decode_server_udp` descomprime los datagramas `0xD4` con el mismo límite (`take(250_001)`). Los paquetes malformados/truncados/demasiado grandes se registran y se ignoran sin romper el bucle de lectura. Pruebas: ida y vuelta zlib, corrupción/truncamiento, límite antibomba de descompresión (300 KB → rechazo, 250 KB → aceptado).
- **Conexión de servidor ofuscada (DH768 + RC4, eMule `ECS_PENDING_SERVER`)**: `ObfuscatedStream::connect_server_outgoing` implementa el handshake cliente↔servidor dedicado (distinto del handshake par-a-par por userhash) — envío `[marker][G^A (96B)][padlen][pad]` en claro, recepción de la clave pública DH del servidor, claves `RC4(MD5(S‖34))` envío / `RC4(MD5(S‖203))` recepción con drop-1024, y luego intercambio cifrado `[MAGICVALUE_SYNC][supported][requested][padlen][pad]` ↔ `[MAGICVALUE_SYNC][method][padlen][pad]`. El `ServerSessionManager` extrae `ST_TCPPORTOBFUSCATION`/`ST_UDPPORTOBFUSCATION` (`0x97`/`0x98`) de `OP_SERVERIDENT` (persistido por dirección entre reconexiones) y sigue la semántica `CServerSocket::ConnectTo`: intento ofuscado en el puerto dedicado (o el puerto normal si no se anuncia ninguno, una sola vez — `SetTriedCrypt`), respaldo en texto plano automático si falla el handshake, registros explícitos del modo utilizado. El login `OP_LOGINREQUEST` solo se envía tras finalizar el handshake; los helpers de sesión se generalizan al transporte `ObfuscatedStream<TcpStream>` (transparente en texto plano). Configurable mediante `set_obfuscation_preference` (activado por defecto, relleno configurable). Pruebas: handshake DH loopback completo (intercambio cifrado bidireccional), rechazo limpio de una respuesta no cifrada, pruebas unitarias de descompresión.
- **Búsqueda en servidor: fin del bloqueo indefinido de la UI** — un servidor que aceptaba la petición sin responder nunca dejaba `_isSearching` activo de por vida (reinicio solo ante evento `server.searchResult` o error RPC); un tiempo de espera de 30 s libera ahora el estado con un estado «terminada sin respuesta», sin impedir que los resultados tardíos alimenten la lista.
- **Bootstrap Kad desde pares entrantes**: el sembrado desde los clientes conectados solo existía en las conexiones salientes; los pares **entrantes** que anuncian un puerto Kad (`CT_EMULE_UDPPORTS`) desencadenan ahora también `maybe_bootstrap_from_peer` (limitado a 1×/min, tabla < 50 contactos).

### Añadido y Corregido — Secure User Identification (SUI)

- **Migración del `cryptkey.dat` heredado**: el antiguo blob binario de 80 bytes (identidad ficticia de versiones anteriores) se detecta (`LegacyKeyLoad::LegacyBlob`), se pone en cuarentena y se sustituye por una clave RSA-384 real con un registro `WARN` explícito — en lugar de desactivar SUI definitivamente. Un archivo de 80 bytes *de texto* sigue tratándose como corrupción (`Invalid`, sin regeneración).
- **Interoperabilidad de red real — orden de los paquetes SUI**: los clientes eMule reales envían con frecuencia `OP_SECIDENTSTATE (state=2)` antes que su `OP_PUBLICKEY`. El desafío se pone ahora en espera y se procesa en cuanto se recibe la clave (saliente `OutboundSui` + despachador entrante), en lugar de desecharse.
- **Firmante entrante**: `OP_SECIDENTSTATE` entrante ya se gestiona — un par que se conecta a nosotros y nos desafía recibe nuestra `OP_PUBLICKEY` (si `state=2`) y luego `OP_SIGNATURE` V1. `InboundListenerService` recibe `IdentityService` mediante `set_identity`.
- **Observabilidad SUI**: contadores compartidos `SuiStats` (desafíos emitidos, firmas enviadas/verificadas/rechazadas, anti-inundación, paquetes malformados) expuestos en solo lectura mediante RPC `identity.status` bajo `sui`.

## [0.1.8] - 2026-09-19

Versión de rendimiento y fidelidad protocolaria eMule: agregación multipar de las tasas de descarga, gestión conforme de las colas de espera remotas (Queue Ranking) y preservación de las cuotas de enjambre.

### Añadido y Optimizado
- **Agregación Multipar y Estabilización de la Tasa (`transfer.rs`)**:
  - Cálculo y sincronización en tiempo real de la tasa acumulada de descarga sobre el conjunto de sesiones activas para un mismo archivo (`sync_transfer_rates`).
  - Eliminación de los artefactos de visualización a 0 B/s: `RateResetGuard` recalcula la tasa restante al desconectarse un par en lugar de poner a cero la velocidad global.
  - Registro de telemetría `[DL_PROGRESS]` periódico claro que muestra la tasa global acumulada y la contribución por par activo.
- **Gestión Conforme de las Colas de Espera Remotas (`peer_session.rs`)**:
  - Detección adecuada de la desconexión en cola (`Disconnected in queue`): cuando los pares eMule cierran el socket TCP para preservar sus recursos manteniendo nuestro rango (`OP_QUEUERANKING`), la fuente permanece en estado `Allowed`.
  - Aplicación del plazo de reconsulta conforme a eMule (10 a 15 minutos) en lugar de una reconexión inmediata agresiva a los 60 s, evitando penalizaciones y expulsiones de la cola.
- **Protección del Tope de Cuota del Enjambre (`swarm_coordinator/strategy.rs`, `types.rs`)**:
  - Integración de las fuentes en cola (`Candidate`, `Tried`, `Allowed`) en el cálculo de fuentes viables, impidiendo el colapso del tope de asignación water-filling.
- **Inbound Listener y Upload Queue (`dispatcher.rs`, `upload_queue.rs`)**:
  - Procesamiento completo de las peticiones de subida entrantes (`OP_STARTUPLOADREQ`, `OP_ACCEPTUPLOADREQ`) y liberación dinámica de los slots de envío.
  - Registros de diagnóstico estratégicos (`[UPLOAD_REQUEST]`, `[UPLOAD_SLOT_GRANTED]`, `[UPLOAD_RELEASED]`).
- **Cartografía Global del Repositorio (`INDEX.md`)**:
  - Inventario exhaustivo y documentación de todos los archivos versionados del proyecto eMuleRust.

---

## [0.1.3-lab] - 2026-09-15

Versión de consolidación mayor que integra la implementación completa del protocolo eD2K real (Tier 3.1 a Tier 3.6), la inspección detallada y el Part Map (Fase 8.1), el historial y estadísticas locales con exportaciones censuradas (Fase 8.2), las categorías locales y la jerarquía de destino en 4 niveles (Fase 8.3), la importación defensiva de archivos `.part.met` de eMule con informe de auditoría (Fase 8.4), así como el empaquetado multisabor para Windows y la migración de esquema SQLite (Fase 8.5).

> **Formulación de acreditación de interoperabilidad y confinamiento:**  
> *«eMuleRust es una aplicación de laboratorio local. Las transferencias eD2K, la reanudación, la verificación por partes y la finalización se han validado frente a la referencia eMule 0.50a en un entorno estrictamente confinado a 127.0.0.1. No hay ningún servidor eD2K público, nodo Kad, par externo, tráfico UDP ni compresión de protocolo activados.»*

### Añadido
- **Protocolo eD2K Loopback y Cualificación eMule 0.50a (Tier 3.1 – 3.6)**:
  - Handshake completo eD2K y negociación de extensiones eMule (`ET_MOD_VERSION`, `ET_VERSION`, `ET_EXT_FEATURES`).
  - Declaración de disponibilidad de archivo (`FILE_STATUS_REQ` y `FILE_STATUS` con mapa de bits por bloque y fuente completa).
  - Secuenciación y planificación acotada en bloques de 9,28 MB descompuestos en subrangos de peticiones `REQUESTPARTS`.
  - Escritura en flujo continuo mediante `SENDINGPART` en un archivo temporal `.part`.
  - Control de integridad criptográfica MD4 por bloque y cálculo de la huella raíz eD2K global.
  - Gestión del residuo de frontera (p. ej. 9 728 001 bytes) y promoción atómica hacia `downloads/`.
  - Reanudación resiliente tras parada abrupta al ~50 % con persistencia SQLite y reanudación íntegra.
- **Inspección Detallada y Part Map Interactivo (Fase 8.1)**:
  - `DownloadDetailDialog`: identificadores completos, enlace canónico `ed2k://`, métricas de transferencia en directo.
  - Visualización Part Map en bloques coloreados con tooltips interactivos y desplazamientos exactos.
  - Exportación de diagnósticos censurados sin ningún token `auth.token`.
  - Modificación de destino objetivo con sonda de escritura (`write-probe`).
- **Historial Local y Estadísticas de Actividad (Fase 8.2)**:
  - Tabla SQLite `download_history` inmutable con índices temporales y de tipos de eventos.
  - Panel `HistoryView` con agregados sobre 4 ventanas (Total, 7 días, 24 h, Sesión actual).
  - Política de retención configurable (7, 30, 90 días o conservación indefinida) con poda automática.
  - Exportaciones censuradas JSON / CSV sin fuga de rutas privadas.
- **Categorías Locales y Resolución de Destino en 4 Niveles (Fase 8.3)**:
  - Tabla SQLite `download_categories` y asociación `category_id` en `downloads`.
  - Invariante absoluto de no destrucción: la eliminación de una categoría reasocia las descargas a `category_id = NULL` sin borrar ningún archivo.
  - Resolución de destino jerárquica: tarea > categoría > global SQLite > paquete.
- **Importación Defensiva de Archivos `.part.met` (Fase 8.4)**:
  - Decodificación de los formatos binarios eMule estándar (`0xE0`), archivos grandes (`0xE1`) y nuevos formatos (`0xE2`).
  - Inversión algorítmica de los huecos para calcular los rangos válidos realmente adquiridos.
  - Cálculo criptográfico MD4 por bloque con invalidación y reevaluación como hueco de los bloques corruptos.
  - Confinamiento estricto bajo `data/temp/<id>.part` e inicialización imperativa en estado `Paused` (`sourceCount: 0`).
  - Informe de auditoría legible con botón de copia al portapapeles y botón de cierre.
- **Empaquetado Windows Multisabor y Migración SQLite (Fase 8.5)**:
  - Sabor `standard` depurado de herramientas de laboratorio y sabor `qa-loopback` instrumentado.
  - Migración de esquema SQLite idempotente y no destructiva desde la Fase 7 certificada mediante prueba unitaria.
  - Portabilidad y actualización sin pérdida de las carpetas `data/` y `downloads/`.

---

## [0.1.2-lab] - 2026-09-14

Versión de consolidación que integra la separación estanca `data/` frente a `downloads/`, la persistencia del borrado en SQLite, la selección personalizada de la carpeta de descarga, el enriquecimiento de los diagnósticos y la canalización QA E2E automatizada Fase 7.3.

> **Formulación de acreditación de interoperabilidad:**  
> *«eMuleRust es una aplicación de laboratorio local. La compatibilidad eD2K está validada sobre un corpus estático y un par de referencia emulado en loopback cerrado. No se contacta con ningún servidor eD2K público, nodo Kad ni par externo.»*

### Añadido
- **Separación Estanca del Estado Interno (`data/`) y los Archivos Finalizados (`downloads/`)**:
  - `data/` contiene exclusivamente el estado interno: SQLite WAL (`mule.db`), sesión local (`session.json`), token secreto (`auth.token`), archivos parciales dispersos (`.part`).
  - `downloads/` aloja los archivos finalizados y validados, con creación automática si es necesario.
  - En modo portátil, la carpeta de descarga por defecto se resuelve hacia `<bundleRoot>/downloads` distinta de `data/`.
- **Selección Personalizada de la Carpeta de Destino (Browse...)**:
  - Integración del selector de carpetas nativo en el modal de adición de enlace eD2K (`AddDownloadDialog`).
  - Consideración de la ruta de destino personalizada en el método RPC `downloads.addEd2kLink`.
- **Persistencia del Borrado de Descarga en SQLite**:
  - El método RPC `downloads.remove` ejecuta un `DELETE FROM downloads WHERE id = ?` persistido en SQLite.
  - Borrado físico automático de los archivos de estado parciales asociados (`.part` y `.part.met`).
  - Garantía de no reaparición de la tarea tras detener y reiniciar el demonio (`test_downloads_remove_persistence_across_daemon_restart`).
- **Garantía de Inviolabilidad de las Descargas durante la Limpieza Lab**:
  - El método `lab.cleanupTestData` solo borra `<workdir>/fixtures/` y los archivos incompletos temporales; los archivos finalizados en `downloads/` nunca se alteran.
- **Ergonomía UI y Diagnósticos Avanzados**:
  - Visualización en forma de insignias (`Chip`) de las capacidades activas anunciadas por el demonio (`downloads`, `download-events`, `lab-tools`).
  - Abreviatura ergonómica de los identificadores UUIDv4 (`#abc1...`) en la tabla de descargas.
  - `DiagnosticDialog` enriquecido con enmascaramiento sistemático de las rutas sensibles y preservación del secreto cero.
- **Fase 7.3: Canalización E2E QA Automatizada (`scripts/test_qa_loopback_e2e.ps1`)**:
  - Validación hermética en un entorno temporal estéril fuera del repositorio.
  - Cobertura E2E completa: fixture nominal (1 MB), fixture en la frontera de parte (9 728 001 bytes), parada abrupta/reanudación de descarga con conservación de los bloques ya validados, y control criptográfico final.

---

## [0.1.1-lab] - 2026-09-14

Candidata a lanzamiento consolidada para el modo portátil (Fase 7.1) y las herramientas de laboratorio endurecidas (Fase 7.2).

### Añadido
- **Modo Portátil Híbrido y Cascada Determinista**: resolución unificada `--workdir` > `EMULERUST_WORKDIR` > `portable.flag` (`data/`) > `%LOCALAPPDATA%\eMuleRust` (con alias obsoleto `MULE_DATA_DIR`).
- **Doble Bloqueo del Modo Lab**: las funcionalidades de laboratorio (`LabGuideDialog`, inyección de eD2K sintético, limpieza segura) requieren tanto el modo portátil como la feature compilada `lab-tools`.
- **Taxonomía JSON-RPC 2.0 Extendida**: formalización de los códigos `-32000` (AuthFailed), `-32001` (ProtocolMismatch), `-32002` (ForbiddenInInstalledMode), `-32003` (UnsafeCleanupTarget), `-32004` (FixtureDownloadMismatch), `-32005` (LoopbackPeerUnavailable), `-32010` (BusinessError).
- **Garantía Cero Secretos en los Diagnósticos**: censura sistemática de los tokens de autenticación (`auth.token`) y de las claves privadas en `DiagnosticDialog`.
- **Política PowerShell Endurecida**: transición estricta hacia `-ExecutionPolicy RemoteSigned` (erradicación de `-ExecutionPolicy Bypass`).

### Modificado
- **Depuración de las Capacidades Estándar**: eliminación de `local-simulation` de la compilación por defecto (`default = []`). Solo se anuncian `["downloads", "download-events"]` por defecto.
- **Transparencia del Inventario del Paquete**: transición de 22 a 21 archivos en `SHA256SUMS.txt` tras la adopción exclusiva por Flutter Desktop de la versión binaria `AssetManifest.bin` (que sustituye al formato JSON redundante).

---

## [0.1.0-lab] - 2026-09-13

Primera versión interna de laboratorio para **eMuleRust** (cliente moderno eD2K offline/loopback).
Esta versión congela el estado de validación de interoperabilidad local, la canalización de almacenamiento, el protocolo acotado y el conjunto de herramientas de prueba.

### Resumen de los Niveles de Acreditación
- **Tier 1 (Corpus estático)**: **100 % Validado** — Decodificación/codificación canónica y determinista sobre fixtures de bytes congeladas (`ed2k_corpus_tests.rs`).
- **Tier 2 (Par de referencia emulado)**: **100 % Validado** — Handshake nominal, negociación de capacidades, peticiones/respuestas de estado y transferencia de bloques completos y residuales frente a un par de referencia local simulado (`ed2k_reference_interop_tests.rs`).
- **Tier 3 (Binario externo fijado)**: **Banco de pruebas disponible y auditado** — Banco de orquestación hermética listo (`tier3_loopback.rs`), prueba en vivo ignorada por defecto mientras no se proporcione un binario eMule/aMule verificado y fijado.

> **Formulación de acreditación de interoperabilidad:**
> *«Compatibilidad eD2K experimental validada sobre corpus estático y par de referencia emulado, en entorno loopback cerrado. El banco de pruebas Tier 3 para un binario eMule/aMule externo fijado está disponible, pero su prueba en vivo permanece ignorada por defecto mientras no se proporcione un binario verificado.»*

### Elementos Ausentes de Forma Intencionada
- **Cero conexiones de red públicas**: todos los sockets están rigurosamente confinados a `127.0.0.1` (loopback estricto).
- **Cero protocolo Kad / Kademlia / DHT**: ninguna petición DHT activa ni nodo de enrutamiento Kad.
- **Cero conexión a servidores eD2K**: ningún intercambio TCP/UDP hacia servidores de indexación públicos.
- **Cero subida / seeding**: ninguna publicación ni envío espontáneo de fragmentos a terceros.
- **Cero fuga de identidad del anfitrión**: anonimización estricta de todas las trazas de red e informes CI.

---

### Novedades y Adiciones

#### Seguridad Visual Unicode y Normalización
- Integración de `unicode-normalization` para la aplicación canónica de la forma Unicode NFC sobre las cadenas manipuladas.
- Función [`sanitize_unicode_for_display`](file:///d:/Projet/eMuleRust/crates/mule-protocol/src/ed2k/reference.rs): conservación de los valores en bruto para la comparación byte a byte y escape seguro en `\u{XXXX}` de los caracteres engañosos para los informes CI y terminales:
  - Anulaciones y aislamientos bidireccionales (`U+202A`..=`U+202E`, `U+2066`..=`U+2069`, `U+061C`, `U+200E`, `U+200F`).
  - Caracteres invisibles y espacios de anchura nula (`U+200B`..=`U+200D`, `U+2060`, `U+FEFF`, `U+00AD`).
  - Caracteres de control no gráficos (`c.is_control()`).
- Detección y rechazo automático en `TraceRecord::is_clean()` de cualquier trama que contenga anulaciones bidi o caracteres invisibles sin escapar.

#### Propiedad Formal de No Mutación
- Estructura [`SessionSnapshot`](file:///d:/Projet/eMuleRust/crates/mule-protocol/src/ed2k/session.rs) y método `Ed2kSession::snapshot()` para congelar el estado protocolario completo de una sesión.
- Contador de diagnósticos aislado `unhandled_packets_count`.
- Garantía formal y probada: el procesamiento de una trama `Raw` (`0xC5 PR_EMULE`, opcode ambiguo para el perfil activo, u opcode desconocido `0xFE`) no altera en ningún caso el estado de sesión, el rastreador de estado (`status_tracker`), el rastreador de bloques (`block_tracker`), la planificación de descarga, la persistencia SQLite ni los contadores de progreso.

#### Esquema de Trazas Normalizadas Legibles por Máquina
- Registros de trazas normalizadas con metadatos de acreditación en el nivel raíz:
  - `schemaVersion: 1`
  - `testTier: 2` (o `3` para Tier 3)
  - `referenceKind: "emulated-reference-peer"` (o `"external-reference-binary"`)
  - `compatibilityProfile: "Emule050a"` | `"Amule233"` | `"LegacyEdonkey"` | `"UnknownStrict"`
  - `protocolByte`, `opcode`, `direction`, `payloadLength`, `outcome`.
- Escritura segura de los artefactos CI mediante `DivergenceReport::write_ci_report_secure` en directorios temporales aislados (`tempfile::tempdir()`), con fallo previo que impide cualquier creación de archivo si una traza está comprometida.

#### Perfiles de Compatibilidad eD2K Explícitos
- Definición del enum `Ed2kCompatibilityProfile`:
  - `Emule050a`: opcodes documentados de eMule 0.50a (`0x47` RequestParts, `0x4F` SetReqFileId).
  - `Amule233`: perfil aislado para variantes aMule.
  - `LegacyEdonkey`: opcodes históricos de eDonkey 2000 (`0x4B` RequestParts, `0x49` FileStatusReq).
  - `UnknownStrict`: rechazo inmediato de cualquier ambigüedad (`AmbiguousOpcodeForProfile`).
- Matriz de decodificación multiperfil y gestión de los centinelas de red (`(0,0)` y `(u32::MAX, u32::MAX)`).

#### Canalización de Almacenamiento y Demonio Local
- Motor de descarga por intervalos disjuntos (`RangeSet`) con asignación dispersa (`part_file.rs`).
- Soporte de bloques residuales (p. ej.: archivo de 9 728 001 bytes compuesto por una parte de 9 728 000 bytes y un residuo de 1 byte).
- Demonio local supervisado `mule-daemon` con persistencia SQLite WAL, bloqueo exclusivo `fs4::FileExt::try_lock()` sobre `mule.lock`, y token secreto CSPRNG de 64 caracteres hexadecimales (`auth.token`).
- Supervisor Flutter Desktop con descubrimiento automático de sesión (`session.json`), reconexión por backoff exponencial y gestión de condición de carrera en el arranque.

---

### Comandos de Validación (100 % Sin Conexión)

```powershell
# 1. Verificación estática y compilación
cargo check --workspace --all-targets --all-features

# 2. Linter sin advertencias
cargo clippy --workspace --all-targets --all-features -- -D warnings

# 3. Conjunto completo de las 204 pruebas unitarias y de integración Rust (1 prueba Tier 3 en vivo ignorada por defecto)
cargo test --workspace --all-features

# 4. Análisis estático Flutter Desktop
cd apps/desktop
flutter analyze

# 5. Conjunto de las 31 pruebas Flutter Desktop
flutter test
```

### Recuento de Pruebas del Proyecto
- **Pruebas Rust**: **204 superadas**, 1 ignorada (Tier 3 en vivo), 0 fallidas.
- **Pruebas Flutter**: **31 superadas**, 0 fallidas.
- **Total general activo**: **235 pruebas validadas al 100 %**.

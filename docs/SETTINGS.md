# Ajustes de eMuleRust — Referencia completa

Todos los ajustes se persisten en la tabla SQLite `settings` y se
manipulan mediante RPC `settings.get` / `settings.set` (parche JSON en
camelCase). La mayoría se **aplican en caliente** — sin necesidad de reiniciar el
demonio. La columna «eMule» indica el equivalente en las
Preferencias de eMule 0.72a.

## Interfaz (`/settings`)

| Sección UI | Contenido |
|---|---|
| **General** | Apodo, carpeta de descargas, compartición de Incoming, retención de historial, idioma |
| **Conexión** | Kill switch, interruptores **Red eD2K / Red Kad** (perfil de red), cuotas de sockets, **capacidades y puertos**, ofuscación, conexiones máx., USS, UPnP |
| **Transferencias** | Fuentes máx. globales / por archivo / mín., cola de subida, cuota diaria, rescate de swarm |
| **Seguridad** | Filtro IP (`ipfilter.dat`), límites de velocidad, aviso anti-SSRF, opciones exclusivas (paranoid, rotación de identidad) |
| **Amigos** | Friend slots (`friends.*`) |
| **Avanzado** | Diagnóstico, importación `.part.met`, guía lab, barra de depuración, versión |

Todos los campos de ruta (carpeta de descargas, carpeta temporal,
`ipfilter.dat`, `server.met`, `nodes.dat`, `.part.met`/`.part`/carpeta
destino de importación) ofrecen un botón **«Examinar»** que abre el
selector de archivos/carpetas — ya no hace falta introducir la ruta a mano.

## Claves RPC

### General

| Clave `settings.set` | Clave persistida | Tipo | Por defecto | Efecto | eMule |
|---|---|---|---|---|---|
| `nickname` | `nickname` | string 1–64 | `"eMuleRust"` | Anunciado en el login de servidor y en los hellos a pares | `nick` |
| `defaultDownloadDir` | `default_download_directory` | path | carpeta portátil | Sonda de escritura validada; usado al añadir enlaces y al escanear Incoming | «Directorio» |
| `shareDownloadDir` | `share_download_dir` | bool | `true` | Escaneo + hash de los archivos de Incoming (arranque + `shares.rescan`) | carpeta Incoming compartida |
| `historyRetentionDays` | `history_retention_days` | u32 | `30` | Purga del historial aplicada inmediatamente | — |
| `tempDir` | `temp_dir` | path | carpeta demonio | Carpeta `.part` efectiva para nuevas descargas e importaciones | «Directorio temporal» |
| `ichEnabled` | `ich_enabled` | bool | `true` | Verificación MD4 de cada parte completada (desactivarlo omite la verificación intermedia) | `ICH` |
| `rememberCancelled` | `remember_cancelled` | bool | `true` | Entrada en el historial al cancelar | `RememberCancelledFiles` |
| `rememberDownloaded` | `remember_downloaded` | bool | `true` | Entrada en el historial al completar | `RememberDownloadedFiles` |
| `updateChannel` | `update_channel` | `"stable"` \| `"beta"` | `"stable"` | Canal de actualizaciones en GitHub Releases (`stable` = solo estables; `beta` = incluye prelanzamientos) | — |

### Subtítulos y automatización multimedia

| Clave `settings.set` | Clave persistida | Tipo | Por defecto | Efecto |
|---|---|---|---|---|
| `subtitlesAutoDownload` | `subtitles_auto_download` | bool | `true` | Descarga automática de subtítulos faltantes tras organizar o en el ciclo periódico |
| `subtitlesLanguage` | `subtitles_language` | string | `"spa"` | Idioma objetivo para descarga automática |
| `subtitlesVariant` | `subtitles_variant` | `"es-ES"` \| `"es-LA"` \| `"any"` | `"es-ES"` | Preferencia de variante de español (penaliza o descarta variantes incompatibles) |
| `subtitlesAllowHi` | `subtitles_allow_hi` | bool | `false` | Permite subtítulos para personas con discapacidad auditiva (*Hearing Impaired*) |
| `subtitlesMinScore` | `subtitles_min_score` | u32 (0–200) | `40` | Puntuación mínima exigida para descargar automáticamente un candidato |
| `subtitlesAutoSync` | `subtitles_auto_sync` | `"off"` \| `"fps_only"` \| `"embedded_sub"` | `"embedded_sub"` | Cascada de sincronización automática post-descarga (FPS / pista embebida) |
| `opensubtitlesApiKey` | `opensubtitles_api_key` | string | `""` | Clave opcional para la API REST v1 de OpenSubtitles.com |

### Servidores (sección Conexión)

| Clave | Persistida | Tipo | Por defecto | Efecto | eMule |
|---|---|---|---|---|---|
| `serverReconnect` | `server_reconnect` | bool | `true` | Vigilante de 15 s: reconexión al último servidor por la ruta RPC normal (política + kill switch aplicados) | `Reconnect` |
| `serverAutoconnect` | `server_autoconnect` | bool | `false` | En el primer tick, conexión al servidor con mejor `preference` no desactivado | `Autoconnect` |
| `serverAutoremoveDead` | `server_autoremove_dead` | bool | `true` | Elimina el objetivo tras 1 fallo de reconexión (eMule `DeadServerRetry` por defecto = 1) | `AutoDeadServer`/`DeadServerRetry` |
| `serverAutoupdateList` | `server_autoupdate_list` | bool | `false` | Importa `server_autoupdate_path` una vez por sesión | `AutoUpdateServerList` |
| `serverAutoupdatePath` | `server_autoupdate_path` | path ≤512 | `""` | Archivo `server.met` **local** (sin fetch HTTP por diseño anti-SSRF) | URL server.met |
| `safeConnect` | `safe_connect` | bool | `false` | `downloads.resume` rechazado mientras no haya ni servidor eD2K ni Kad (contactos > 0) disponibles | `SafeConnect` |

### Conexión

**Interruptores «Red eD2K» / «Red Kad»**: la sección se abre con
una tarjeta «Redes» con dos interruptores que proyectan el `NetworkProfile`
persistido (`network.setProfile`). `Red eD2K` desactivado → confinamiento
loopback `local-lab`; `Red Kad` activado → `public-ed2k-kad-preview`
(activar Kad activa automáticamente eD2K — «Kad sin eD2K» no existe
en el modelo). Los tres estados posibles siguen siendo: `local-lab`,
`public-ed2k-preview`, `public-ed2k-kad-preview`. El kill switch y el
pipeline anti-SSRF permanecen intactos.

| Clave | Persistida | Tipo | Por defecto | Efecto | eMule |
|---|---|---|---|---|---|
| `ed2kPort` | `ed2k_port` | u16 1–65535 | `39735` | **Rebind en caliente** del listener TCP + `kad.set_ed2k_tcp_port` + buddy connect-options + `upnp.set_ports` | Puerto TCP (eMule aleatoriza 4096–65095 en el 1.er arranque) |
| `kadPort` | `kad_port` | u16 1–65535 | `39737` | Rebind en caliente del socket Kad UDP | Puerto UDP (ídem, aleatorizado por eMule). **Diferencia**: eMule acepta 0 = UDP desactivado; aquí se desactiva con `udpEnabled` |
| `udpEnabled` | `udp_enabled` | bool | `true` | Detiene/inicia `kad_udp` + `server_udp` — equivalente a «puerto UDP = 0» en eMule | «UDP desactivado» |
| `upnpEnabled` | `upnp_enabled` | bool | `false` | `map_ports`/`unmap_ports` inmediato | `EnableUPnP` (por defecto eMule: desactivado) |
| `capacityDownloadKbs` | `capacity_download_kbs` | KiB/s | `0` | Capacidad de línea descendente — limita el DL al 90 % de la capacidad (clamp eMule `Preferences.cpp`), reaplicado en cada `limits.set` y normalizado al arrancar | Capacidad de recepción |
| `capacityUploadKbs` | `capacity_upload_kbs` | KiB/s | `0` | Capacidad ascendente — limita el UL al 80 % de la capacidad; referencia USS (`note_manual_limit`) | Capacidad de emisión |
| `obfuscationEnabled` | `obfuscation_enabled` | bool | config | Listener entrante en vivo (`AtomicBool`), conexiones salientes, byte buddy Kad | «Support obfuscated connections» |
| `obfuscationRequired` | `obfuscation_required` | bool | config | ídem — pares sin cifrado rechazados | «Require obfuscated connections» |
| `maxConnections` | `max_connections` | usize 1–65000 | `500` | `ResourceBudgets.max_active_peer_connections` (en vivo). Presupuesto de **sockets TCP de pares** — distinto de `maxGlobalDownloadPeers` (cuota de asignación) | MaxConnections (ilimitado en NT+, `GetMaxWindowsTCPConnections()` → `UNLIMITED`; 65000 = límite de seguridad) |
| `maxConnPerMin` | `max_conn_per_min` | u32 | `240` | `ResourceBudgets.rate_limit_per_min` (en vivo) | MaxConnPerFiveSeconds = 20 ≈ 240/min (granularidad min) |
| `ussEnabled` | `uss_config` (JSON) | bool | `false` | `UssService.update_config` — límite de subida guiado por latencia | «USS» |
| `paranoidMode` | `paranoid_mode` | bool | `false` | Slots de subida reservados a pares **ofuscados + firma SUI verificada**; rechaza clientes eMule clásicos (en vivo, `dispatcher`) | — (específico de eMuleRust) |
| `identityRotationHours` | `identity_rotation_hours` | h 0–8760 | `0` | Regenera periódicamente clave RSA + user_hash eD2K + KadID (propagación en caliente listener/Kad; las sesiones activas conservan la anterior). **Invalida los créditos** — identidad efímera. Marca temporal en `identity_last_rotation_unix` | — (específico de eMuleRust) |

### Transferencias

| Clave | Persistida | Tipo | Por defecto | Efecto | eMule |
|---|---|---|---|---|---|
| `maxGlobalDownloadPeers` | `max_global_download_peers` | usize | `500` | Tope global de conexiones de pares activas entre todos los archivos — limita los nuevos intentos de conexión | `MaxConnections` (`TooManySockets`) |
| `maxPeersPerDownload` | `max_peers_per_download` | usize | `400` | Tope de fuentes por archivo (sobrescribible vía `downloads.setMaxSources` / `FT_MAXSOURCES`) — cada archivo conecta sus fuentes hasta este límite | `MaxSourcesPerFile` |
| `minPeersPerDownload` | `min_peers_per_download` | usize | `5` | Umbral de inanición que dispara un intercambio A4AF desde un archivo menos prioritario | — (extensión de eMuleRust) |
| `maxUploadQueue` | `max_upload_queue` | usize 10–5000 | `5000` | `UploadManager.max_queue` (en vivo) | `QueueSize` (por defecto eMule 5000) |
| `minUploadKbs` | `min_upload_kbs` | KiB/s 0–100000 | `1` | Mínimo por slot: no se abre nuevo slot si la media baja del umbral (en vivo). **0 = umbral desactivado** — los slots se abren mientras no se alcance `max_active` (extensión; eMule fuerza `MinUpload ≥ 1`) | `MinUpload` (por defecto eMule 1, mínimo obligatorio) |
| `autoPriorityNewDownloads` | `auto_priority_new_downloads` | bool | `false` | Archivos ≥ 9 728 000 B creados con prioridad `high` | autoprioridad |
| `dailyUploadQuotaGb` | `daily_upload_quota_gb` | GiB 0–1048576 | `0` | Cuota volumétrica/día: al alcanzarse → no se conceden nuevas peticiones de slot (vigilante 30 s). Contador persistido `upload_quota_used_bytes`, reinicio al cambiar de día UTC (`upload_quota_day`) | — (específico de eMuleRust, sin equivalente en eMule) |
| `swarmHealing` | `swarm_healing` | bool | `false` | Sesga la selección de parte hacia las **partes más escasas del swarm** (contador de fuentes conocidas por parte) — el cliente actúa como fuente de respaldo antes de que desaparezca una parte. Desempate: bytes recibidos | — (específico de eMuleRust; eD2K es pull-only, no permite push) |

### Límites de velocidad (RPC `limits.set`, sección Seguridad)

Limitadores DL/UL en bytes/s aplicados a los buckets globales; distintos de las
*capacidades* (velocidad de línea, usadas por USS como referencia). Cada
campo proporcionado se **persiste** (`rate_limit_download_bps`,
`rate_limit_upload_bps`, `rate_limit_conn_per_min`,
`rate_limit_server_req_per_sec`, `rate_limit_kad_pkt_per_sec`) y se recarga
al arrancar en `AppState::new`. Una actualización **parcial** conserva
los demás limitadores (la configuración parte de la instantánea en vivo de los buckets, no de los
valores compilados por defecto).

**Clamp de capacidad** (semántica eMule `Preferences.cpp`): los límites DL/UL
se acotan al **90 %** / **80 %** de las capacidades declaradas — `0` (ilimitado)
nunca se acota, igual que si la capacidad es `0`. El clamp se aplica en
`limits.set`, en cualquier cambio de `capacity*Kbs` (`apply_capacity_clamp`,
repersistiéndose el valor efectivo), y **al arrancar**: un límite
persistido por encima de la capacidad se acota y se reescribe en SQLite. La interfaz
muestra el valor realmente aplicado (relectura de `limits.get`) y lo
indica en la pestaña de velocidades.

**Ratio subida/descarga** (eMule `GetMaxDownloadInBytesPerSec`,
`Preferences.cpp:694` — «don't be a Lam3r :)»): cuando el límite de subida
baja de **20 KiB/s**, la descarga *efectiva* se limita a un múltiplo
de la subida — **3×** si subida < 4 KiB/s, **4×** si < 10 KiB/s, **5×** si
< 20 KiB/s. Una descarga «ilimitada» (`0`) también se limita a
`coef × subida`. El valor **configurado** persistido permanece intacto (el ratio
se recalcula en cada aplicación, nunca se sobrescribe) — subir la subida
restaura la descarga sin tener que reintroducirla. Cuando USS está activo, el tope
sigue al límite dinámico en cada tick del bucle de control (eMule
usa la velocidad medida cuando DynUp está activo). `limits.get` expone
`downloadConfiguredBytesPerSec` / `uploadConfiguredBytesPerSec` (valores
configurados) y `downloadRatioApplied` (tope de ratio activo). La pestaña
muestra el valor **configurado** en los campos e indica el efectivo en
texto de ayuda cuando difiere (ratio eD2K o control USS).

**USS — objetivo de ping** (eMule `LastCommonRouteFinder`): cuando USS está
activado, el demonio traza rutas ICMP (`IcmpSendEcho`, TTL creciente —
sin privilegios) hacia una muestra de pares conocidos (servidor conectado,
clientes de subida activos, fuentes de descarga, ≤ 8 IPs públicas) y
hace ping al **salto más profundo común a todas las rutas** — normalmente
el router de agregación del ISP, donde se mide realmente la congestión
del enlace ascendente local. El último router común se recalcula como máximo
cada 15 min y tras 60 fallos de ping consecutivos (re-baseline).
Respaldo: `uss.pingHost` explícito, y luego sonda TCP-connect hacia el servidor
eD2K conectado (en plataformas sin `IcmpSendEcho`). El objetivo activo
es visible en `limits.get` → `uss.pingTarget` (`icmp:x.x.x.x` o
`tcp:ip:port`). Cuando USS está activo, el límite de envío introducido acota el
techo del control dinámico (`uss.max_upload_bps`, eMule: `MaxUpload`
limita DynUp; `0` = sin techo).

### Filtro IP (RPC `filter.*`, sección Seguridad)

| RPC | Persistida | Efecto |
|---|---|---|
| `filter.setEnabled` | `ipfilter_enabled` (bool) | Activa/desactiva el filtrado entrante+saliente; restaurado al arrancar |
| `filter.importFile` | `ipfilter_path` (path) | Carga un `ipfilter.dat` **local** (sin fetch HTTP, anti-SSRF); el archivo se recarga automáticamente al arrancar |
| `filter.clear` | `ipfilter_path` → `""` | Vacía los rangos y olvida la ruta persistida |

Al igual que eMule, que recarga `ipfilter.dat` en cada arranque, el filtro y
su estado activado sobreviven a los reinicios del demonio.

## Comportamientos destacables

- **Rebind TCP**: cambiar `ed2kPort` corta brevemente el listener (como
  eMule). Las conexiones entrantes activas se interrumpen.
- **UDP desactivado**: el UDP de cliente comparte el socket Kad — desactivar UDP detiene
  los tres servicios a la vez (Kad UDP, server UDP, client UDP).
- **Compartición parcial**: las descargas en curso aparecen en
  `shares.list` con `isPartial=true` **desde su creación** — eMule
  publica siempre los `.part` activos
  (`CDownloadQueue::AddPartFilesToShare`: *«part files are always
  shared files»*), incluso sin parte completa (`completedParts` vacío);
  solo se sirven las partes completadas (`PARTSTATUS`, rangos fuera de
  partes rechazados). Los parciales se publican como **fuentes Kad**
  (`PublishSrc`) pero se excluyen de la publicación de palabras clave; en
  el `OP_OFFERFILES` del servidor (ordenado por prioridad de subida, ≤ 200 entradas)
  llevan los marcadores `0xFCFCFCFC`/`0xFCFC`.
  `shares.setEnabled`/`setUploadPriority` los rechazan — una compartición
  transitoria persistida fijaría la ruta `.part` y la serviría como
  completa tras reiniciar. `shares.setCategory` sí se acepta:
  la categoría la lleva la descarga subyacente (columna
  `downloads.category_id`), no una entrada `shared_files`
  transitoria. Cada mutación `shares.*` emite la notificación
  `shares.changed` (insignia «Compartidos» en vivo).
- **Compartidos tras reiniciar**: las descargas `Completed` se leen desde
  SQLite incluso fuera del mapa `active` — la lista de compartidos sobrevive a los
  reinicios (el archivo destino debe existir en disco).
- **`shares.rescan`**: vuelve a escanear la carpeta Incoming (botón actualizar de
  la pestaña Compartidos); devuelve `{added, dir}`; respeta `shareDownloadDir`.
- **Arranque**: todos los overrides persistidos se releen en `AppState::new`
  antes del bind de los sockets — los valores sobreviven a los reinicios.
- **Vigilante de servidor** (`AppState::spawn_server_watcher`, tick 15 s):
  orden = reconnect (`last_target`) → autoconnect (mejor servidor);
  se omite si el kill switch está activo; 1 fallo → `delete_server` si
  `server_autoremove_dead`. La desconexión manual borra `last_target`.
- **ICH desactivado**: las partes completadas dejan de verificarse por MD4 — la
  finalización se basa en la cobertura de rangos y el hash global final.
  La recuperación AICH no se ve afectada (depende de `aich_root`).

## UX de la página `/settings`

- **Guardado**: los interruptores se aplican de inmediato; los campos
  de texto/numéricos se envían con Intro o el botón/icono «Aplicar».
- **Validación**: cualquier entrada inválida (fuera de rango, vacía) muestra un
  error localizado en el banner global — sin notificaciones de «actualizado»
  cuando no se ha aplicado nada.
- **Sincronización multicliente**: `settings.changed` resincroniza
  interruptores **y** campos de texto; un campo en edición (dirty)
  nunca se sobrescribe con un valor remoto.
- **Kill switch**: confirmado mediante un diálogo antes de `network.disconnectAll`.
  `network.enable` restaura los servicios de red (listener entrante,
  sockets UDP Kad/servidor, mantenimiento Kad, USS, actualización UPnP) y el
  vigilante reconecta al último servidor (`server_reconnect`).
- **Retención**: un valor persistido fuera de los elementos estándar {7, 30, 90, 0}
  (p. ej. vía MCP) se muestra en una opción dedicada del desplegable.

## Registro (daemon)

El demonio escribe sus trazas `tracing` en **`logs/`** bajo el workdir
(rotación diaria `daemon.<date>.log`, retención de 7 archivos) así
como en stdout, con marca de tiempo local RFC 3339. La escritura en archivo es
**no bloqueante** (`tracing-appender`) y en formato **JSONL** (nivel,
target, campos, spans); la consola sigue siendo legible en texto plano.

- **Panic**: cualquier panic se escribe de forma síncrona en
  `logs/crash.log` (el escritor no bloqueante puede estar ya muerto en el momento
  del fallo) y se emite mediante `tracing::error!`.
- **Nivel en caliente**: la clave `logLevel` de `settings.set` recarga el
  `EnvFilter` sin reiniciar (nivel simple «debug» o especificación completa
  «info,mule_daemon::kad_udp=warn»). **Solo en tiempo de ejecución** — nunca
  persistido (un `trace` que sobreviviera a un reinicio llenaría los archivos).
  `settings.get` y `system.status` devuelven la especificación actual; la interfaz
  expone un selector en Ajustes → Avanzado.
- **Filtro al inicio**: `RUST_LOG` sigue siendo la fuente inicial
  (`EnvFilter::try_from_default_env`), por defecto `info`.
- **Privacidad**: las direcciones de pares/servidores externos se
  seudonimizan de forma determinista por sesión (`peer-NN:port`,
  `server-NN:port` mediante `mule-filter`); loopback queda en claro para el
  laboratorio. El token de autenticación nunca se escribe en los registros.
- **Volumen en producción**: las trazas por paquete (Kad Ping/Pong/Req/Res,
  recepciones UDP en bruto) y por bloque (`UPLOAD_BLOCK`, peticiones de
  subbloques, progreso) están en nivel `debug`. Con el valor por defecto `info`, el
  registro solo contiene los eventos de sesión (handshakes, slots,
  hitos de descarga) y un latido consolidado cada 60 s.

El servidor MCP (`mule-mcp`) sigue el mismo esquema en su propio
`logs/mcp.<date>.log`; stdout queda reservado para JSON-RPC MCP.

# Índice de archivos del proyecto

Índice exhaustivo de los archivos versionados del repositorio **eMuleRust** — un cliente eDonkey/eMule escrito en Rust (demonio + crates de protocolo) con interfaz web integrada.

---

## Raíz

| Archivo | Función |
|---|---|
| `.gitignore` | Exclusiones Git (`target/`, `dist/`, `scratch/`, etc.) |
| `Cargo.toml` | Manifiesto del workspace Rust (miembros, dependencias compartidas) |
| `Cargo.lock` | Bloqueo de versiones de las dependencias Rust |
| `README.md` | Presentación general del proyecto y portal hacia `docs/` |
| `AGENTS.md` | Reglas para agentes de IA (cartografía, validación, reglas críticas) |
| `CLAUDE.md` | Puntero hacia `AGENTS.md` (compatibilidad Claude Code) |
| `GEMINI.md` | Puntero hacia `AGENTS.md` (compatibilidad Gemini CLI) |
| `.github/copilot-instructions.md` | Puntero hacia `AGENTS.md` (compatibilidad GitHub Copilot) |
| `deny.toml` | Configuración `cargo-deny` (auditoría de licencias/vulnerabilidades) |
| `LICENSE` | Licencia del proyecto: GPL-2.0-or-later (texto canónico FSF) |
| `AUTHORS` | Titular del copyright y mantenedor (Laurent Geynet "Loulach") |
| `NOTICE` | Atribuciones: fuentes adaptadas (eMule © Merkur, aMule, eDonkey2000, Kademlia) |
| `CONTRIBUTING.md` | Guía de contribución (flujo de ramas, validación, convenciones) |
| `SECURITY.md` | Política de seguridad y notificación de vulnerabilidades |
| `CODE_OF_CONDUCT.md` | Código de conducta comunitario (Contributor Covenant 2.1) |
| `.gitattributes` | Fines de línea deterministas (CRLF en scripts, binarios) |
| `.github/workflows/ci.yml` | CI GitHub Actions (Rust check/clippy/test) |
| `.github/workflows/docker.yml` | Pipeline de publicación multi-arquitectura (`amd64`/`arm64`) en GHCR |
| `.github/workflows/release.yml` | Pipeline de release en tag `v*` (compilación estándar + publicación GitHub Release) |
| `.github/dependabot.yml` | Actualizaciones automáticas de dependencias (cargo, actions) |
| `.github/ISSUE_TEMPLATE/` | Formularios de incidencias GitHub (bug, funcionalidad, contacto de seguridad) |
| `.github/PULL_REQUEST_TEMPLATE.md` | Lista de verificación PR (validación, fidelidad de protocolo, seguridad) |
| `Dockerfile` | Imagen Docker multi-etapa (`cargo-chef` + `debian:bookworm-slim`) |
| `.dockerignore` | Exclusiones del contexto de compilación Docker |
| `docker-compose.yml` | Orquestación Compose con volumen único para *hardlinks* y perfiles de red |
| `docker/Dockerfile.ci` | Imagen runtime ligera para publicaciones multi-arquitectura en CI |
| `docker/entrypoint.sh` | Punto de entrada del contenedor con soporte `PUID`/`PGID` |
| `Asset/pulse-core-logo.svg` | Logotipo vectorial oficial de Pulse Core |
| `web/index.html` | Interfaz web integrada (WebUI) servida por `mule-daemon` |
| `wiki/` | Documentación de usuario para la Wiki de GitHub |

## .agents/ — Configuración de agentes/MCP

| Archivo | Función |
|---|---|
| `.agents/mcp_config.json` | Configuración del servidor MCP |
| `.agents/plugins/emulerust/mcp_config.json` | Configuración MCP del plugin emulerust |
| `.agents/plugins/emulerust/plugin.json` | Manifiesto del plugin de agente |

## crates/ — Crates Rust

### `mule-assistant` — Orquestador LLM local

| Archivo | Función |
|---|---|
| `Cargo.toml` | Manifiesto (deps: `mule-mcp`, `reqwest`, `axum`) |
| `src/lib.rs` | Raíz de la biblioteca |
| `src/main.rs` | Binario: `chat` (REPL), `--prompt` (one-shot), `serve` (WS) |
| `src/config.rs` | `AssistantConfig` + validación de endpoint loopback |
| `src/error.rs` | `AssistantError` (`thiserror`) |
| `src/llm/mod.rs` | Trait `LlmBackend`, tipos de mensajes/`ToolCall`/`AssistantTurn` |
| `src/llm/ollama.rs` | Backend Ollama `/api/chat` (en streaming, solo loopback) |
| `src/llm/mock.rs` | `MockBackend` programado para pruebas sin conexión |
| `src/policy.rs` | Allowlist, tabla prefijo→capacidad del demonio, confirmación requerida |
| `src/tools_bridge.rs` | `ToolDefinition` MCP → esquema tool-calling; truncado de resultados |
| `src/session.rs` | Bucle agéntico, `AssistantEvent`, historial de Ollama |
| `src/repl.rs` | REPL de consola (eventos + confirmación `o/n`) |
| `src/ws.rs` | Servidor WS loopback (`assistant.hello/chat/confirm/cancel`) |
| `tests/assistant_suite/` | Suite consolidada: `main.rs`, `policy_tests.rs`, `session_tests.rs`, `ollama_backend_tests.rs`, `ws_tests.rs` |

### `mule-cli` — Interfaz de línea de comandos

| Archivo | Función |
|---|---|
| `src/main.rs` | Punto de entrada del binario CLI |
| `src/lib.rs` | Biblioteca del CLI |
| `src/commands/mod.rs` | Módulo de subcomandos |
| `src/commands/hash.rs` | Comando de cálculo de hash eD2K |
| `src/commands/import_amule.rs` | Importación desde perfiles/configuración de aMule |
| `src/commands/known_met.rs` | Inspección y volcado de archivos `known.met` |
| `src/commands/link.rs` | Comando de generación de enlaces eD2K |
| `src/commands/part_met.rs` | Comando de inspección de archivos `.part.met` |
| `src/commands/update.rs` | Subcomando `update` (`status`, `check`, `upload`, `rollback`) para gestionar actualizaciones vía HTTP/RPC |
| `tests/cli_suite/` | Suite consolidada: `main.rs`, `import_amule_tests.rs`, `make_link_tests.rs`, `update_command_tests.rs` |

### `mule-core` — Modelo de dominio y persistencia

| Archivo | Función |
|---|---|
| `src/lib.rs` | Raíz de la biblioteca |
| `src/error.rs` | Tipos de error del crate |
| `src/model.rs` | Modelos de datos centrales |
| `src/kad/mod.rs` | Módulo Kademlia |
| `src/kad/contact.rs` | Representación de un contacto Kad |
| `src/kad/kad_id.rs` | Identificador Kad (128 bits) |
| `src/kad/keyword_index.rs` | Índice de palabras clave Kad |
| `src/kad/routing_table.rs` | Tabla de enrutamiento Kad (k-buckets) |
| `src/kad/source_index.rs` | Índice de fuentes Kad |
| `src/kad/note_index.rs` | Índice de notas Kad |
| `src/persistence/mod.rs` | Módulo de persistencia |
| `src/persistence/traits.rs` | Traits de los almacenes de datos |
| `src/persistence/sqlite/mod.rs` | Implementación SQLite |
| `src/persistence/sqlite/schema.rs` | Esquema de la base SQLite |
| `src/persistence/sqlite/aich_hashes.rs` | Persistencia de árboles y hashes maestros AICH |
| `src/persistence/sqlite/cancelled.rs` | Persistencia de archivos cancelados (`cancelled.met`) |
| `src/persistence/sqlite/categories.rs` | Persistencia de categorías |
| `src/persistence/sqlite/credits.rs` | Persistencia de créditos (subida/descarga) |
| `src/persistence/sqlite/downloads.rs` | Persistencia de descargas |
| `src/persistence/sqlite/friends.rs` | Persistencia de amigos (`friends.met`) |
| `src/persistence/sqlite/history.rs` | Persistencia del historial |
| `src/persistence/sqlite/kad.rs` | Persistencia de datos Kad |
| `src/persistence/sqlite/media.rs` | Persistencia de biblioteca multimedia (`media_library`) |
| `src/persistence/sqlite/notifications.rs` | Persistencia del centro de notificaciones y estados de lectura |
| `src/persistence/sqlite/servers.rs` | Persistencia de servidores |
| `src/persistence/sqlite/shares.rs` | Persistencia de archivos compartidos (incl. `share_upload_stats`: totales de subida acumulados por hash) |
| `src/persistence/sqlite/sources.rs` | Persistencia de fuentes/pares |
| `src/persistence/sqlite/sui_cache.rs` | Caché SUI persistente de identidades verificadas |
| `src/ranges/mod.rs` | Módulo de gestión de intervalos |
| `src/ranges/range_set.rs` | Conjunto de intervalos (partes descargadas) |
| `src/storage/mod.rs` | Módulo de almacenamiento de archivos |
| `src/storage/finalizer.rs` | Finalización de archivos completos |
| `src/storage/part_file.rs` | Gestión de archivos `.part` |
| `src/verification/mod.rs` | Módulo de verificación de integridad |
| `src/verification/aich.rs` | Verificación AICH (árbol de hash) |
| `src/verification/ed2k.rs` | Verificación de hashes eD2K |

### `mule-core` — Pruebas

| Archivo | Función |
|---|---|
| `tests/core_suite/main.rs` | Arnés consolidado de `mule-core` |
| `tests/core_suite/k_buckets_tests.rs` | Pruebas de los k-buckets Kad y límites globales anti-Sybil |
| `tests/core_suite/part_file_and_persistence.rs` | Pruebas de archivos `.part` + persistencia |
| `tests/core_suite/range_set.rs` | Pruebas del `RangeSet` |
| `tests/core_suite/range_set_properties.rs` | Pruebas de propiedades del `RangeSet` |
| `tests/core_suite/sqlite_batch_operations_tests.rs` | Pruebas SQLite de operaciones por lotes |
| `tests/core_suite/sqlite_categories_tests.rs` | Pruebas SQLite categorías |
| `tests/core_suite/sqlite_corruption.rs` | Pruebas de recuperación ante corrupción SQLite |
| `tests/core_suite/sqlite_history_tests.rs` | Pruebas SQLite historial |
| `tests/core_suite/sqlite_kad_keywords_tests.rs` | Pruebas SQLite palabras clave Kad |
| `tests/core_suite/sqlite_kad_sources_tests.rs` | Pruebas SQLite fuentes Kad |
| `tests/core_suite/sqlite_peer_sources_tests.rs` | Pruebas SQLite fuentes de pares |
| `tests/core_suite/sqlite_servers_tests.rs` | Pruebas SQLite servidores |
| `tests/core_suite/sqlite_settings_tests.rs` | Pruebas SQLite ajustes |

### `mule-crypto` — Criptografía heredada y moderna (`PC26`)

| Archivo | Función |
|---|---|
| `Cargo.toml` | Manifiesto del crate |
| `src/lib.rs` | Raíz del crate (MD4, identificadores) |
| `src/cryptkey.rs` | Formato `cryptkey.dat` (DER + Base64 Crypto++) |
| `src/legacy_sui.rs` | Secure User Identification (RSA-384 PKCS#1 v1.5 + SHA-1) |
| `src/merkle_blake3.rs` | Árbol de integridad Merkle BLAKE3 (`PC26`) |
| `src/modern_identity.rs` | Identidad moderna Ed25519 / SUI v3 (`PC26`) |
| `src/pc26_handshake.rs` | Acuerdo de claves híbrido X25519 + ML-KEM-768 (`PC26`) |

### `mule-daemon` — Demonio principal

| Archivo | Función |
|---|---|
| `src/main.rs` | Punto de entrada del demonio (init logging `tracing` → stdout + `logs/daemon.<date>.log` JSONL rotativo 7 días no bloqueante, panic hook → `logs/crash.log`, filtro `RUST_LOG` + recarga en caliente vía `settings.set` clave `logLevel`, por defecto `info`; resolución `--web-dir`/`MULE_WEB_DIR` + autodetección de la carpeta UI web) |
| `src/lib.rs` | Raíz de la biblioteca del demonio |
| `src/config.rs` | Carga de configuración (incluida `web_dir` — UI web estática servida por el demonio) |
| `src/error.rs` | Errores del demonio |
| `src/lock.rs` | Bloqueo de instancia única |
| `src/session.rs` | Gestión de la sesión del demonio |
| `src/state.rs` | Estado compartido del demonio — puertos efectivos (`ed2k_port`/`kad_port`/`udp_enabled` en atómicos), `effective_obfuscation()`, `current_nickname()`, restauración de los ajustes persistidos al arrancar |
| `src/workdir.rs` | Gestión del directorio de trabajo |
| `src/ed2k/mod.rs` | Módulo eD2K del demonio |
| `src/ed2k/downloader.rs` | Descargador eD2K |
| `src/lab/mod.rs` | Módulo lab (experimental) |
| `src/lab/downloader.rs` | Descargador lab |
| `src/lab/source_service.rs` | Servicio de fuentes lab |

### `mule-daemon` — RPC

| Archivo | Función |
|---|---|
| `src/rpc/mod.rs` | Módulo RPC |
| `src/rpc/backup.rs` | Exportación e importación de copias de seguridad del perfil |
| `src/rpc/capabilities.rs` | Catálogo de capacidades expuestas por `system.hello`/`system.status` |
| `src/rpc/server.rs` | Servidor RPC (transporte) + servicio estático de la UI web (`ServeDir`, respaldo SPA `index.html`) + `GET /session-config.js` (inyecta endpoint+token, solo loopback) |
| `src/rpc/methods.rs` | Definición de los métodos RPC |
| `src/rpc/torznab.rs` | Cliente/analizador Torznab para indexadores Prowlarr/Jackett |
| `src/rpc/transmission.rs` | API compatible con clientes *arr (Transmission RPC) |
| `src/rpc/types.rs` | Tipos del protocolo RPC |
| `src/rpc/events.rs` | Eventos RPC (notificaciones en tiempo real) |
| `src/rpc/handlers/mod.rs` | Módulo de los manejadores RPC |
| `src/rpc/handlers/categories.rs` | Manejador: categorías |
| `src/rpc/handlers/downloads.rs` | Manejador: descargas (`downloads.*` incl. `rename`, `setDestination`, `inspectPartMet`/`importPartMet`) |
| `src/rpc/handlers/friends.rs` | Manejador: amigos (friend slots) |
| `src/rpc/handlers/history.rs` | Manejador: historial |
| `src/rpc/handlers/kad.rs` | Manejador: Kad |
| `src/rpc/handlers/lab.rs` | Manejador: lab |
| `src/rpc/handlers/media.rs` | Manejador: catálogo multimedia Servarr (`media.*`) |
| `src/rpc/handlers/network.rs` | Manejador: política de red (`network.*`) + límites de velocidad (`limits.get`/`set` — clamp capacidad eMule 90 %/80 %, ratio anti-leech `emule_ud_ratio` 3×/4×/5× por debajo de 20 KiB/s de subida) |
| `src/rpc/handlers/notifications.rs` | Manejador: notificaciones (`notifications.list`, `markRead`, `clear`, `getConfig`, `updateConfig`, `testChannel`) |
| `src/rpc/handlers/servers.rs` | Manejador: servidores |
| `src/rpc/handlers/settings.rs` | Manejador: ajustes — `settings.get`/`set` cubriendo todos los ajustes en tiempo de ejecución (puertos, UDP, UPnP, ofuscación, capacidades — clamp eMule 90 %/80 % sobre los límites, USS, cuotas, cola de subida) con aplicación en caliente (`apply_port_change`, `apply_udp_state`, `apply_capacity_clamp`); sonda de escritura en carpetas, advertencia sobre claves desconocidas |
| `src/rpc/handlers/shares.rs` | Manejador: archivos compartidos — `shares.list` (incl. parciales + estadísticas de subida `uploadRateBps`/`uploadedBytes` + `categoryId`/`upPriority`), `add`, `remove`, `rename`, `setEnabled`, `setUploadPriority` (mutaciones rechazadas en `is_partial`), `setCategory` (aceptada en parciales — categoría de la descarga subyacente), `rescan`; emite `shares.changed` en cada mutación |
| `src/rpc/handlers/sources.rs` | Manejador: fuentes |
| `src/rpc/handlers/upnp.rs` | Manejador: UPnP |
| `src/rpc/handlers/updater.rs` | Manejador: actualizaciones oficiales y rollback (`system.checkUpdate`, `getUpdateStatus`, `applyUpdate`, `rollbackUpdate`) |
| `src/rpc/handlers/updater_upload.rs` | Manejador HTTP autenticado `POST /api/system/upload-update` para binarios compilados locales |

### `mule-daemon` — Servicios

| Archivo | Función |
|---|---|
| `src/services/mod.rs` | Módulo de servicios |
| `src/services/auto_grabber.rs` | Captura automática periódica en indexadores Torznab (`media_automation`) |
| `src/services/cleaner.rs` | Limpieza y saneamiento de nombres multimedia |
| `src/services/compression_policy.rs` | Política de compresión en cable eD2K/Zstd |
| `src/services/disk_scanner.rs` | Escáner de disco para detectar archivos y cambios en biblioteca |
| `src/services/hole_punch.rs` | Servicio UDP Hole Punching para atravesar NAT |
| `src/services/incoming_share.rs` | Escaneo de la carpeta Incoming (arranque + `shares.rescan` manual): hash eD2K + compartición automática (opt-out `share_download_dir`) |
| `src/services/indexer_aggregator.rs` | Agregador de búsquedas Torznab multiproveedor |
| `src/services/downloads.rs` | Servicio de gestión de descargas |
| `src/services/downloads/cancelled_files.rs` | Registro y migración de archivos cancelados (`cancelled.met`) |
| `src/services/downloads/clients_met.rs` | Importación/exportación de créditos (`clients.met`) |
| `src/services/downloads/corruption_blackbox.rs` | Caja negra diagnóstica de partes corruptas |
| `src/services/downloads/emfriends_import.rs` | Importación de amigos heredados (`emfriends.met`) |
| `src/services/downloads/history_categories.rs` | Historial y categorías de las descargas |
| `src/services/downloads/operations.rs` | Operaciones sobre las descargas (pausa, reanudación…) |
| `src/services/downloads/part_met.rs` | Importación/gestión de los `.part.met` |
| `src/services/downloads/peers_servers.rs` | Fuentes de pares y servidores de las descargas |
| `src/services/downloads/staged.rs` | Staging acotado en RAM por parte verificable MD4 |
| `src/services/downloads/types.rs` | Tipos del servicio downloads (`PartProgressSummary` + `received_mask`, `compute_part_summaries`: fuente completa `part_count==0`, frescura 30 min) |
| `src/services/events.rs` | Bus de eventos internos |
| `src/services/hello_caps.rs` | Construcción y negociación de capacidades `OP_HELLO` y `PC26` |
| `src/services/identity.rs` | Identidad del cliente (user hash, ident) |
| `src/services/inbound_listener.rs` | Escucha de conexiones entrantes (ofuscación en vivo vía `set_obfuscation`) |
| `src/services/inbound_listener/connection.rs` | Aceptación de conexiones entrantes |
| `src/services/inbound_listener/dispatcher.rs` | Despacho de conexiones entrantes |
| `src/services/inbound_listener/share_browser.rs` | Exploración segura de archivos compartidos (`OP_ASKSHAREDFILES`) |
| `src/services/inbound_listener/subnet_guard.rs` | Protección de subredes entrantes anti-abuso |
| `src/services/inbound_listener/types.rs` | Tipos del listener |
| `src/services/kad.rs` | Servicio Kad (mantenimiento, *self-lookup*, revalidación LRU, *jitter* y canal `kad.statusChanged`) |
| `src/services/kad/buddy_client.rs` | Cliente saliente LowID Buddy (`FINDBUDDY_REQ`, enlace TCP, `OP_BUDDYPING`/`OP_BUDDYPONG`, `OP_CALLBACK`) |
| `src/services/kad/lookup.rs` | Búsquedas iterativas Kademlia sobre distancia XOR |
| `src/services/kad/nodes_dat.rs` | Bootstrap mediante `nodes.dat` |
| `src/services/kad/search_keywords.rs` | Búsqueda y publicación por lotes de palabras clave Kad (`publish_keywords_batch`) |
| `src/services/kad/search_sources.rs` | Búsqueda de fuentes Kad |
| `src/services/kad/types.rs` | Tipos del servicio Kad |
| `src/services/kad/firewall.rs` | Gestión del cortafuegos Kad (`FIREWALLED_UDP`, pruebas de conectividad) |
| `src/services/kad_udp.rs` | Servicio Kad UDP |
| `src/services/kad_udp/handlers.rs` | Manejadores de paquetes Kad UDP y emparejamiento de transacciones |
| `src/services/kad_udp/routing_client.rs` | Cliente de enrutamiento Kad (incl. `send_find_buddy`) |
| `src/services/kad_udp/search_client.rs` | Cliente de búsqueda Kad |
| `src/services/kad_udp/storage.rs` | Nodo de almacenamiento Kad (TTL, límites anti-flood) |
| `src/services/kad_udp/types.rs` | Tipos Kad UDP, limitador por opcode y `find_matching_tx_id` |
| `src/services/known_met_import.rs` | Importación heredada de `known.met` |
| `src/services/media_automation.rs` | Automatización multimedia Servarr |
| `src/services/media_metadata.rs` | Extracción de metadatos audiovisuales |
| `src/services/media_probe.rs` | Sonda local de medios y duración |
| `src/services/nas_transfer.rs` | Transferencia segura hacia NAS |
| `src/services/nfs_mount.rs` | Gestión de montajes NFS locales |
| `src/services/notifications/` | Sistema de notificaciones (`mod.rs`, `constants.rs`, `error.rs`, `http_client.rs`, `types.rs`, `channels.rs`, `monitor.rs`, `service.rs`) |
| `src/services/client_udp.rs` | Socket client-UDP eD2K (reasks/keep-alive 0x90-0x93) |
| `src/services/server_udp.rs` | Socket UDP de servidor (`GLOBGETSOURCES2`/`GLOBFOUNDSOURCES`) + failover en `swarm_coordinator/strategy.rs` |
| `src/services/uss.rs` | USS — límite de subida dinámico por latencia (ley `LastCommonRouteFinder`, puerta `accept_new_client` sobre la cola); reajusta el bucket de descarga vía `emule_ud_ratio` en cada tick (eMule `bDynamic`) |
| `src/services/uss_icmp.rs` | USS — traceroute ICMP Windows (`IcmpSendEcho*`) para el *Last Common Route Finder* de eMule: descubre el último router común y su latencia de referencia |
| `src/services/uss_ledbat.rs` | Controlador LEDBAT++ para saturación de subida de baja prioridad |
| `src/services/obfuscation_stream.rs` | Flujo de red ofuscado (ofuscación de protocolo) |
| `src/services/organizer.rs` | Organizador automático Plex/Jellyfin con enlaces duros |
| `src/services/pc26_stream.rs` | Transporte cifrado moderno `PC26` (ChaCha20-Poly1305 + Zstd) |
| `src/services/peer_session.rs` | Sesión con un par eD2K (+ helpers `pending_ranges*`: bloques en vuelo hacia `DownloadSnapshot.pendingRanges`) |
| `src/services/peer_session/a4af.rs` | Gestión de archivos secundarios A4AF en sesión de par |
| `src/services/peer_session/bdp_pipeline.rs` | Ventana adaptativa BDP para tubería de bloques |
| `src/services/peer_session/change_client_id.rs` | Reevaluación y cambio dinámico de ID con pares |
| `src/services/peer_session/connection.rs` | Conexión al par |
| `src/services/peer_session/events.rs` | Eventos de sesión de par |
| `src/services/peer_session/file_request.rs` | Petición de archivos y metadatos al par |
| `src/services/peer_session/handshake.rs` | Handshake eD2K con el par |
| `src/services/peer_session/happy_eyeballs.rs` | Conexión dual IPv6/IPv4 (Happy Eyeballs v2) |
| `src/services/peer_session/peer_scoring.rs` | Puntuación y reputación dinámica de pares |
| `src/services/peer_session/source_exchange.rs` | Intercambio de fuentes entre pares |
| `src/services/peer_session/status_query.rs` | Peticiones de estado de archivo — `OP_FILESTATUS` → `record_source_availability` + emite `downloadUpdated` (máscaras de partes, inicial y keep-alive en cola) |
| `src/services/peer_session/sui.rs` | Secure User Identification en la sesión de par |
| `src/services/peer_session/transfer.rs` | Transferencia de bloques con el par |
| `src/services/peer_session/types.rs` | Tipos de sesión de par |
| `src/services/quic_transport.rs` | Transporte QUIC RFC 9000 para túneles modernos |
| `src/services/scorer.rs` | Puntuador de lanzamientos multimedia (Torznab/Servarr) |
| `src/services/server_session.rs` | Sesión con un servidor eD2K |
| `src/services/server_session/connection.rs` | Conexión al servidor |
| `src/services/server_session/events.rs` | Eventos de sesión de servidor |
| `src/services/server_session/types.rs` | Tipos de sesión de servidor |
| `src/services/subtitles/` | Motor modular de subtítulos: búsqueda OpenSubtitles (`opensubtitles.rs`), puntuación (`scorer.rs`), validación SRT (`srt.rs`), sincronización (`sync.rs`) y fachada (`mod.rs`) |
| `src/services/swarm_coordinator.rs` | Coordinación del enjambre de pares |
| `src/services/swarm_coordinator/lowid_callback.rs` | Coordinación de *callbacks* LowID vía servidor y buddy |
| `src/services/swarm_coordinator/strategy.rs` | Estrategia de coordinación (rebalanceo A4AF incluido) |
| `src/services/swarm_coordinator/types.rs` | Tipos del coordinador |
| `src/services/upload_queue.rs` | Cola de espera de subida (trickle slots, friend slots, anti-leecher, tamaño configurable `set_max_queue`) |
| `src/services/upnp.rs` | Servicio UPnP (`set_enabled`, `set_ports`, map/unmap, refresco periódico) |
| `src/services/upnp/discovery.rs` | Descubrimiento de la pasarela UPnP |
| `src/services/upnp/ipv6_pinhole.rs` | Apertura IPv6 PCP/UPnP IGDv2 Pinholing |
| `src/services/upnp/mapping.rs` | Mapeo de puertos UPnP |
| `src/services/upnp/pcp_natpmp.rs` | Cliente PCP y NAT-PMP de respaldo |
| `src/services/upnp/types.rs` | Tipos UPnP |
| `src/services/updater/` | Servicio híbrido de actualizaciones (`constants.rs`, `error.rs`, `http_fetch.rs`, `installer.rs`, `manifest.rs`, `service.rs`, `verifier.rs`) con validación ELF64/PE, respaldo `.old` O(1) y rollback |

### `mule-daemon` — Pruebas (arneses consolidados)

| Archivo | Función |
|---|---|
| `tests/daemon_suite/` | Arnés principal (`main.rs`, `aich_tests.rs`, `category_rpc_tests.rs`, `content_policy_tests.rs`, `corruption_blackbox_tests.rs`, `download_details_and_destination_tests.rs`, `dual_portable_bundles.rs`, `ed2k_obfuscation_tests.rs`, `filter_rpc_tests.rs`, `hashset_tests.rs`, `history_rpc_tests.rs`, `kad_bootstrap_tests.rs`, `kad_hardening_tests.rs`, `kad_keywords_tests.rs`, `kad_rpc_tests.rs`, `kad_sources_tests.rs`, `kad_udp_tests.rs`, `known_met_import_tests.rs`, `lifecycle_and_locking.rs`, `migration_import_tests.rs`, `network_policy_rpc_tests.rs`, `notifications_tests.rs`, `orphan_recovery_tests.rs`, `part_met_rpc_tests.rs`, `part_met_write_tests.rs`, `peer_adversarial_tests.rs`, `peer_connection_tests.rs`, `rpc_contract.rs`, `secure_ident_tests.rs`, `server_connection_tests.rs`, `servers_rpc_tests.rs`, `swarm_dual_peer_tests.rs`, `transmission_rpc_tests.rs`, `updater_tests.rs`, `upload_inbound_tests.rs`, `upnp_and_wan_tests.rs`, `webui_auth_tests.rs`, `workdir_tests.rs`) |
| `tests/daemon_ed2k_suite/` | Arnés eD2K (`main.rs`, `ed2k_source_discovery.rs`, `ed2k_transfer_tests.rs`) |
| `tests/daemon_lab_suite/` | Arnés laboratorio (`main.rs`, `daemon_adversarial.rs`, `inter_daemon_transfer.rs`, `lab_rpc_tests.rs`) |

### `mule-filter` — Filtrado IP y limitación de caudal

| Archivo | Función |
|---|---|
| `src/lib.rs` | Raíz del crate |
| `src/filter.rs` | Motor de filtrado IP |
| `src/interval.rs` | Intervalos de direcciones IP |
| `src/parser.rs` | Analizador de archivos `ipfilter.dat` |
| `src/pseudonym.rs` | Soporte de seudónimos |
| `src/token_bucket.rs` | Token bucket (limitación de caudal) |
| `tests/filter_suite/` | Arnés consolidado: `main.rs`, `filter_tests.rs`, `token_bucket_tests.rs` |

### `mule-format` — Formatos de archivos eMule

| Archivo | Función |
|---|---|
| `src/lib.rs` | Raíz del crate |
| `src/error.rs` | Errores de formato |
| `src/fixtures.rs` | Datos de prueba reutilizables |
| `src/aich/mod.rs` | Módulo AICH |
| `src/aich/hash_tree.rs` | Árbol de hash AICH |
| `src/aich/proof.rs` | Pruebas AICH |
| `src/aich/recovery.rs` | Datos de recuperación AICH (ident+hash) |
| `src/ed2k/mod.rs` | Módulo formatos eD2K |
| `src/ed2k/hash.rs` | Cálculo del hash eD2K (MD4 por fragmentos) |
| `src/ed2k/link.rs` | Análisis/generación de enlaces `ed2k://` |
| `src/ed2k/source.rs` | Representación de una fuente eD2K |
| `src/kad/mod.rs` | Módulo formatos Kad |
| `src/kad/nodes_dat.rs` | Lectura/escritura de `nodes.dat` |
| `src/met/mod.rs` | Módulo archivos `.met` |
| `src/met/cancelled_met.rs` | Lectura/escritura de `cancelled.met` |
| `src/met/clients_met.rs` | Lectura/escritura de créditos en `clients.met` |
| `src/met/emfriends.rs` | Lectura/escritura de lista de amigos (`emfriends.met`) |
| `src/met/error.rs` | Errores de formato `.met` |
| `src/met/known_met.rs` | Lectura/escritura de compartidos (`known.met`) |
| `src/met/part_met.rs` | Estructura de los archivos `.part.met` |
| `src/met/preferences_dat.rs` | Lectura de identificador de cliente (`preferences.dat`) |
| `src/met/reader.rs` | Lector de archivos `.met` |
| `src/met/server_met.rs` | Análisis de `server.met` |
| `src/met/tags.rs` | Etiquetas de los archivos `.met` |
| `tests/format_suite/` | Arnés consolidado: `main.rs`, `aich_vectors.rs`, `ed2k_hash_vectors.rs`, `known_met_vectors.rs`, `link_vectors.rs`, `migration_met_vectors.rs`, `nodes_dat_tests.rs`, `official_file_hash.rs`, `part_met_vectors.rs`, `server_met_vectors.rs` |

### `mule-mcp` — Servidor MCP (integración de agentes de IA)

| Archivo | Función |
|---|---|
| `src/main.rs` | Punto de entrada del servidor MCP |
| `src/lib.rs` | Raíz del crate |
| `src/client.rs` | Cliente hacia el demonio |
| `src/protocol.rs` | Protocolo MCP (JSON-RPC) |
| `src/tools/mod.rs` | Módulo de herramientas MCP |
| `src/tools/downloads.rs` | Herramientas MCP: descargas |
| `src/tools/downloads/handlers.rs` | Manejadores de las herramientas downloads |
| `src/tools/downloads/schemas.rs` | Esquemas JSON de las herramientas downloads |
| `src/tools/kad.rs` | Herramientas MCP: Kad |
| `src/tools/notifications.rs` | Herramientas MCP: notificaciones (`list`, `mark_read`, `clear`, `get_config`, `update_config`, `test_channel`) |
| `src/tools/servers.rs` | Herramientas MCP: servidores |
| `src/tools/shares.rs` | Herramientas MCP: compartidos |
| `src/tools/shares/handlers.rs` | Manejadores de las herramientas shares |
| `src/tools/shares/schemas.rs` | Esquemas JSON de las herramientas shares |
| `src/tools/system.rs` | Herramientas MCP: sistema/estado |
| `src/tools/policy.rs` | `ToolRisk`/`ToolProfile`: clasificación y filtrado de herramientas (`--tool-profile read|safe|full`) |
| `tests/mcp_suite/` | Arnés consolidado: `main.rs`, `mcp_protocol_tests.rs`, `tool_profile_tests.rs` |

### `mule-network-policy` — Política de red y contenido

| Archivo | Función |
|---|---|
| `src/lib.rs` | Raíz del crate |
| `src/audit.rs` | Registro de auditoría de decisiones de red |
| `src/budgets.rs` | Presupuestos de red (ancho de banda) |
| `src/cancellation.rs` | Cancelación de operaciones de red |
| `src/content_policy.rs` | Política inmutable «Solo Vídeo» (ADR 0006): extensiones, inspección binaria y selección torrent |
| `src/fetch_guard.rs` | Guardias anti-SSRF compartidas para clientes HTTP del demonio |
| `src/manager.rs` | Gestor de política de red (incl. `emule_ud_ratio` — tope descarga anti-leech eMule) |
| `src/permit.rs` | Permisos de acceso de red |
| `src/profile.rs` | Perfiles de política (lab, WAN…) |
| `src/state.rs` | Estado de la política |
| `src/validator.rs` | Validador de política |
| `tests/policy_validation_tests.rs` | Pruebas de validación de política |

### `mule-protocol` — Protocolos eD2K / Kad

#### Framing y base

| Archivo | Función |
|---|---|
| `AGENTS.md` | Reglas de agentes específicas del crate (referencia eMule 0.72a `srchybrid`, límites `limits.rs`, determinismo de red) |
| `src/lib.rs` | Raíz del crate |
| `src/error.rs` | Errores de protocolo |
| `src/limits.rs` | Límites globales del protocolo |
| `src/traits.rs` | Traits comunes (encode/decode) |
| `src/types.rs` | Tipos comunes |
| `src/framing/mod.rs` | Módulo de encuadre de paquetes |
| `src/framing/codec.rs` | Códec de encuadre (cabeceras, tamaños) |
| `src/framing/packet.rs` | Estructura de paquete en bruto |

#### eD2K — codificación/decodificación

| Archivo | Función |
|---|---|
| `src/ed2k/mod.rs` | Módulo protocolo eD2K |
| `src/ed2k/client_udp.rs` | Códec client-UDP (reasks 0x90-0x93) |
| `src/ed2k/server_udp.rs` | Códec UDP servidor (`GLOBGETSOURCES2`, `GLOBFOUNDSOURCES`, `GLOBSERVSTATRES`) |
| `src/ed2k/opcodes.rs` | Opcodes de los mensajes eD2K |
| `src/ed2k/packet.rs` | Paquete eD2K |
| `src/ed2k/tags.rs` | Etiquetas eD2K (metadatos tipados) |
| `src/ed2k/peer_info.rs` | Información sobre un par |
| `src/ed2k/profile.rs` | Perfil cliente eD2K |
| `src/ed2k/protocol.rs` | Definición del protocolo eD2K |
| `src/ed2k/reference.rs` | Interoperabilidad con la implementación de referencia |
| `src/ed2k/limits.rs` | Límites específicos eD2K |
| `src/ed2k/loopback.rs` | Transporte loopback para pruebas locales |
| `src/ed2k/error.rs` | Errores eD2K |
| `src/ed2k/decoder.rs` | Decodificador eD2K (perfil `Emule072a` por defecto, `extended_requests_version` negociado) |
| `src/ed2k/decoder/handshake.rs` | Decodificación de mensajes de handshake |
| `src/ed2k/decoder/security.rs` | Decodificación de mensajes de seguridad |
| `src/ed2k/decoder/status.rs` | Decodificación de mensajes de estado |
| `src/ed2k/decoder/transfers.rs` | Decodificación de mensajes de transferencia |
| `src/ed2k/decoder/aich.rs` | Decodificación AICH (`0x9B`–`0x9E`) |
| `src/ed2k/decoder/extended.rs` | Decodificación eMule 0.72a extendida (HashSet2, MultiPacket*, cola, chat, CAPTCHA) |
| `src/ed2k/encoder.rs` | Codificador eD2K |
| `src/ed2k/encoder/handshake.rs` | Codificación del handshake |
| `src/ed2k/encoder/security.rs` | Codificación de seguridad |
| `src/ed2k/encoder/tags.rs` | Codificación de etiquetas |
| `src/ed2k/encoder/transfers.rs` | Codificación de transferencias |
| `src/ed2k/encoder/aich.rs` | Codificación AICH |
| `src/ed2k/encoder/extended.rs` | Codificación eMule 0.72a extendida |
| `src/ed2k/file_identifier.rs` | `Ed2kFileIdentifier` (referencia de archivo eMule 0.72a: hash, tamaño, AICH) |

#### eD2K — ofuscación

| Archivo | Función |
|---|---|
| `src/ed2k/obfuscation/mod.rs` | Módulo ofuscación de protocolo |
| `src/ed2k/obfuscation/dh.rs` | Intercambio de claves Diffie-Hellman |
| `src/ed2k/obfuscation/framing.rs` | Encuadre de tramas ofuscadas |
| `src/ed2k/obfuscation/keys.rs` | Derivación de claves |
| `src/ed2k/obfuscation/wire_cipher.rs` | Cifrador de ofuscación de cable (drop1024) |

#### eD2K — sesiones, servidor, intercambios

| Archivo | Función |
|---|---|
| `src/ed2k/session.rs` | Sesión eD2K (máquina de estados) |
| `src/ed2k/session/tracker.rs` | Seguimiento de estado de sesión |
| `src/ed2k/session/types.rs` | Tipos de sesión |
| `src/ed2k/sequencer.rs` | Secuenciador de bloques (least-received, preview first/last, end-game redundancy) |
| `src/ed2k/sequencer/sink.rs` | Receptor de bloques secuenciados |
| `src/ed2k/sequencer/types.rs` | Tipos del secuenciador |
| `src/ed2k/server.rs` | Protocolo cliente↔servidor |
| `src/ed2k/server/search.rs` | Búsqueda en servidor |
| `src/ed2k/server/sources.rs` | Petición de fuentes al servidor |
| `src/ed2k/server/types.rs` | Tipos servidor — `OfferedFile::for_server`: marcadores `0xFBFB` (completo) / `0xFCFC` (parcial `is_partial`) cuando `SRV_TCPFLG_COMPRESSION` |
| `src/ed2k/source_exchange.rs` | Intercambio de fuentes entre pares |
| `src/ed2k/source_exchange/codec.rs` | Códec del intercambio de fuentes |
| `src/ed2k/source_exchange/types.rs` | Tipos del intercambio de fuentes |
| `src/ed2k/storage.rs` | Interfaz de almacenamiento de bloques recibidos |
| `src/ed2k/storage/sink.rs` | Escritura de bloques |
| `src/ed2k/storage/types.rs` | Tipos de almacenamiento |

#### Kad

| Archivo | Función |
|---|---|
| `src/kad/mod.rs` | Módulo protocolo Kad |
| `src/kad/codec.rs` | Códec Kad |
| `src/kad/opcodes.rs` | Opcodes Kad |
| `src/kad/types.rs` | Tipos Kad |
| `src/kad/decoder.rs` | Decodificador Kad |
| `src/kad/decoder/routing.rs` | Decodificación de mensajes de enrutamiento |
| `src/kad/decoder/search.rs` | Decodificación de mensajes de búsqueda |
| `src/kad/encoder.rs` | Codificador Kad (incl. `FINDBUDDY_REQ`) |
| `src/kad/search_expr.rs` | Árbol de expresiones booleanas de búsqueda Kad |
| `src/kad/udp_crypt.rs` | Cifrado de paquetes Kad UDP |

#### Lab (protocolo experimental)

| Archivo | Función |
|---|---|
| `src/lab/mod.rs` | Módulo lab |
| `src/lab/in_flight.rs` | Seguimiento de bloques en vuelo |
| `src/lab/messages.rs` | Mensajes del protocolo lab |
| `src/lab/source_engine.rs` | Motor de fuentes lab |
| `src/lab/state_machine.rs` | Máquina de estados lab |

#### `mule-protocol` — Pruebas y fixtures

| Archivo | Función |
|---|---|
| `tests/common/` | Utilidades compartidas: `mod.rs`, `fixtures.rs`, `sha256.rs` |
| `tests/protocol_suite/` | Arnés principal (`main.rs`, `ed2k_aich_tests.rs`, `ed2k_corpus_tests.rs`, `ed2k_emule_compat_tests.rs`, `ed2k_extended_tests.rs`, `ed2k_hashset_tests.rs`, `ed2k_isolation_guard.rs`, `ed2k_opcode_disambiguation_tests.rs`, `ed2k_roundtrip_tests.rs`, `ed2k_sequencer_tests.rs`, `kad_aich_tests.rs`, `kad_codec_tests.rs`, `malformed_packets.rs`, `packet_codec.rs`, `protocol_limits.rs`, `server_codec_tests.rs`) |
| `tests/protocol_ed2k_suite/` | Arnés de transferencia eD2K (`main.rs`, `ed2k_block_transfer_tests.rs`, `ed2k_file_status_tests.rs`, `ed2k_part_resume_tests.rs`, `ed2k_session_tests.rs`) |
| `tests/protocol_lab_suite/` | Arnés protocolo experimental lab (`main.rs`, `in_flight_properties.rs`, `lab_adversarial.rs`, `lab_in_flight_and_blocks.rs`, `lab_state_machine.rs`, `loopback_transfer.rs`) |
| `tests/tier3_loopback.rs` | Pruebas loopback tier 3 |
| `tests/tier3_external_reference/mod.rs` | Módulo de pruebas de referencia externa |
| `tests/tier3_external_reference/ed2k_reference_interop_tests.rs` | Pruebas de interoperabilidad con la referencia |
| `tests/tier3_external_reference/environment.rs` | Entorno de prueba externa |
| `tests/tier3_external_reference/harness.rs` | Arnés de prueba externa |
| `tests/tier3_external_reference/report.rs` | Informe de pruebas externas |
| `tests/tier3_external_reference/README.md` | Documentación de pruebas de referencia externa |
| `tests/fixtures/ed2k/README.md` | Documentación de fixtures eD2K |
| `tests/fixtures/ed2k/*.bin` | Paquetes binarios capturados (`hello`, `file_status`, `request_parts`, `sending_part`…) |
| `tests/fixtures/ed2k/malformed/*.bin` | Paquetes malformados para pruebas negativas |
| `tests/fixtures/ed2k/manifest.json` | Manifiesto que describe los fixtures |
| `tests/fixtures/ed2k/reference_manifest.json` | Manifiesto de referencia |

### `mule-test-support` — Fixtures y utilidades compartidas de pruebas

| Archivo | Función |
|---|---|
| `Cargo.toml` | Manifiesto del crate de apoyo en pruebas (`dev-dependency`) |
| `src/lib.rs` | Raíz del crate de apoyo para pruebas entre crates |
| `src/daemon.rs` | Lanzamiento y ciclo de vida de demonios efímeros de prueba |
| `src/met_fixtures.rs` | Generadores deterministas de archivos `.part.met` / `.met` |
| `src/peer_fixtures.rs` | Helpers de pares y paquetes simulados |
| `src/rpc.rs` | Cliente JSON-RPC de prueba |
| `src/state.rs` | Constructores de `AppState` aislado para pruebas |
| `src/ws.rs` | Cliente WebSocket para pruebas de eventos en vivo |

### `mule-torrent` — Motor BitTorrent integrado

| Archivo | Función |
|---|---|
| `Cargo.toml` | Manifiesto del crate BitTorrent |
| `src/lib.rs` | Motor BitTorrent, sesiones y filtrado de archivos «Solo Vídeo» |
| `src/config.rs` | Configuración del motor BitTorrent |
| `src/blocklist.rs` | Filtrado IP compatible aplicado al tráfico BitTorrent |

## docs/ — Documentación

### Raíz de la documentación
| Archivo | Función |
|---|---|
| `docs/README.md` | Portal de orientación de la documentación técnica |
| `docs/INDEX.md` | Índice exhaustivo de los archivos del repositorio |
| `docs/ASSISTANT.md` | Guía de uso del asistente de IA local: instalación, modos REPL/prompt/serve, perfiles de herramientas, seguridad, protocolo WS, resolución de problemas |
| `docs/CHANGELOG.md` | Registro de cambios (Keep a Changelog) |
| `docs/docker.md` | Guía de despliegue y operación con Docker y Docker Compose |
| `docs/SETTINGS.md` | Referencia completa de los ajustes (`settings.get`/`set`): claves RPC/persistidas, valores por defecto, aplicación en caliente, equivalentes en eMule |

### docs/architecture/ — Arquitectura e informes
| Archivo | Función |
|---|---|
| `docs/architecture/rapport_architecture_et_avancement.md` | Informe detallado de arquitectura de software, Clean Architecture y estado de avance |
| `docs/architecture/rpc_protocol.md` | Contrato JSON-RPC 2.0/WebSocket demonio↔clientes: transporte, auth, métodos, eventos, códigos de error, versionado |
| `docs/architecture/plan_daemon_events.md` | Plan de eventos del demonio: subidas en tiempo real, `statsTick`, ciclo de vida de descargas, fin del polling (fases E1–E6) |
| `docs/architecture/adr_auto_update.md` | ADR: Sistema de actualización automática integrado (In-App Staging + Atomic Self-Replace, Ed25519, graceful shutdown) |
| `docs/architecture/adr_subtitulos_sincronizacion.md` | ADR: Motor de subtítulos precisos, validados, con puntuación de *releases* y sincronización post-descarga |
| `docs/architecture/conectividad_nat_quic_2026.md` | Especificación de conectividad 2026: cascada UPnP/PCP/NAT-PMP, IPv6 Pinhole, Happy Eyeballs v2, QUIC RFC 9000 y UDP Hole Punching |
| `docs/architecture/bittorrent_negociacion_2026.md` | Especificación de seguridad, IP filter, uTP, Dual-Stack IPv6 y capacidades en `mule-torrent` |

### docs/releases/ — Notas de versión detalladas
| Archivo | Función |
|---|---|
| `docs/releases/RELEASE_NOTES_0.1.0-lab.md` | Notas de la versión 0.1.0-lab |
| `docs/releases/RELEASE_NOTES_0.1.1-lab.md` | Notas de la versión 0.1.1-lab |
| `docs/releases/RELEASE_NOTES_0.1.2-lab.md` | Notas de la versión 0.1.2-lab |
| `docs/releases/RELEASE_NOTES_0.1.3-lab.md` | Notas de la versión 0.1.3-lab |
| `docs/releases/RELEASE_NOTES_0.1.8.md` | Notas de la versión 0.1.8 |
| `docs/releases/RELEASE_NOTES_0.2.0-RC1.md` | Notas de la versión 0.2.0-RC1 |
| `docs/releases/RELEASE_NOTES_0.2.0.md` | Notas de la versión 0.2.0 |
| `docs/releases/RELEASE_NOTES_0.2.0-RC2.md` | Notas de la versión 0.2.0-RC2 |
| `docs/releases/RELEASE_NOTES_0.2.0-RC3.md` | Notas de la versión 0.2.0-RC3 |
| `docs/releases/RELEASE_NOTES_0.3.0-beta.2.md` | Notas de la versión 0.3.0-beta.2 |
| `docs/releases/RELEASE_NOTES_0.3.0-beta.3.md` | Notas de la versión 0.3.0-beta.3 |
| `docs/releases/RELEASE_NOTES_0.3.0-beta.4.md` | Notas de la versión 0.3.0-beta.4 |
| `docs/releases/RELEASE_NOTES_0.4.0-beta.1.md` | Notas de la versión 0.4.0-beta.1 |
| `docs/releases/RELEASE_NOTES_0.4.0-beta.2.md` | Notas de la versión 0.4.0-beta.2 |
| `docs/releases/RELEASE_NOTES_0.4.0-beta.3.md` | Notas de la versión 0.4.0-beta.3 |
| `docs/releases/RELEASE_NOTES_0.4.0-beta.4.md` | Notas de la versión 0.4.0-beta.4 |
| `docs/releases/RELEASE_NOTES_0.4.0-beta.5.md` | Notas de la versión 0.4.0-beta.5 |
| `docs/releases/RELEASE_NOTES_0.4.5.md` | Notas de la versión 0.4.5 |

### docs/qa/ — Garantía de calidad y conformidad
| Archivo | Función |
|---|---|
| `docs/qa/PLAN_TEST_COMPLET.md` | Plan integral de pruebas y cobertura del proyecto |
| `docs/qa/QA_RECEPTION_PROTOCOL.md` | Protocolo de recepción QA |
| `docs/qa/compatibility-matrix.md` | Matriz de compatibilidad |
| `docs/qa/rapport_doublons_rust.md` | Informe de análisis estático y clones en Rust |
| `docs/qa/test-vectors.md` | Vectores de prueba documentados |

### docs/ed2k/ — Especificaciones de red eD2K y ADRs
| Archivo | Función |
|---|---|
| `ed2k/compatibility-notes.md` | Notas de compatibilidad eD2K |
| `ed2k/framing.md` | Documentación del encuadre eD2K |
| `ed2k/limits.md` | Límites del protocolo eD2K |
| `ed2k/message-catalog.md` | Catálogo de mensajes eD2K |
| `ed2k/opcodes.md` | Lista de opcodes eD2K |
| `ed2k/opcodes_matrix.md` | Matriz de opcodes |
| `ed2k/references.md` | Referencias externas |
| `ed2k/scope.md` | Alcance de la implementación eD2K |
| `ed2k/SUI_WIRE_FORMAT.md` | Formato de red SUI (Secure User Identification), citas de eMule 0.72a verificadas |
| `ed2k/SUI_SESSION_RECAP.md` | Resumen de sesiones de trabajo SUI |
| `ed2k/messages/op_filestatus.md` | Especificación del mensaje `OP_FILESTATUS` |
| `ed2k/messages/op_filestatusreq.md` | Especificación del mensaje `OP_FILESTATUSREQ` |
| `ed2k/messages/op_requestparts.md` | Especificación del mensaje `OP_REQUESTPARTS` |
| `ed2k/messages/op_sendingpart.md` | Especificación del mensaje `OP_SENDINGPART` |
| `ed2k/messages/source_exchange.md` | Especificación Source Exchange (SX/SX2, multipacket, limitadores eMule 0.72a) |
| `ed2k/decisions/0001-ed2k-read-only.md` | ADR: eD2K en solo lectura |
| `ed2k/decisions/0002-no-public-network.md` | ADR: sin red pública |
| `ed2k/decisions/0003-wire-lab-separation.md` | ADR: separación wire/lab |
| `ed2k/decisions/0004-public-ed2k-kad-production.md` | ADR: conectividad pública eD2K/Kad en producción |
| `ed2k/decisions/0005-negociacion-pares-2026.md` | ADR: negociación moderna de pares 2026 (capa compatible + extensión `PC26`) |
| `ed2k/decisions/0006-politica-contenido-solo-video.md` | ADR: política de contenido estricta «Solo Vídeo» (Opción E1) en 4 puertas sin procesos externos |
| `ed2k/negociacion_endurecimiento_2026.md` | Especificación de endurecimiento pre-handshake, SUI V2/caché, AICH, BDP, LEDBAT++ y peer scoring |
| `ed2k/criptografia_moderna_pc26.md` | Especificación criptográfica `PC26`: X25519 + ML-KEM-768, ChaCha20-Poly1305, Ed25519 SUI v3, BLAKE3 Merkle y Zstd |

## scripts/ — Scripts PowerShell, Python y Linux

| Archivo | Función |
|---|---|
| `build_windows_server.ps1` | Compilación y empaquetado de la edición Servidor Windows (`mule-daemon.exe`, `mule-cli.exe`, SBOM y `.zip`) |
| `dup_allowlist.txt` | Lista de excepciones autorizadas para el detector de clones exactos (`find_duplicates.py`) |
| `find_duplicates.py` | Análisis estático de funciones Rust duplicadas en los crates del workspace |
| `generate_sbom.ps1` | Generación del SBOM (Bill of Materials) |
| `generate_update_manifest.py` | Firma criptográfica Ed25519 y generación de `manifest.json` para actualizaciones |
| `mule-daemon.service` | Unidad `systemd --user` para Linux / Raspberry Pi (`ReadWritePaths` incluye `%h/.local/bin`, `RestartSec=2`) |
| `smart_test.py` | Corredor inteligente de pruebas y guardias locales (`--mode smart\|fast\|full`, sintaxis JS WebUI y clones B1) |
| `test_duplicates.ps1` | Guardia antiduplicación que falla si aparecen clones exactos fuera de `dup_allowlist.txt` |
| `test_packaged_bundle.ps1` | Prueba del paquete empaquetado |
| `test_phase_b_functional_bench.ps1` | Banco funcional JSON-RPC completo en loopback sin conexión (Fase B) |
| `test_phase_d_public_network.ps1` | Validación supervisada sobre red pública real (Fase D) |
| `test_qa_loopback_e2e.ps1` | Prueba E2E loopback QA |
| `test_rc_matrix.ps1` | Matriz de pruebas release candidate |
| `update_linux.sh` | Compilación local/cruzada (`aarch64`), subida HTTP autenticada, despliegue SSH, instalación de unidad `systemd` y rollback |
| `verify_all.ps1` | Script de verificación global (modos `-Mode smart \| fast \| full`) |

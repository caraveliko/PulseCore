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
| `.github/workflows/release.yml` | Pipeline de release en tag `v*` (compilación estándar + publicación GitHub Release) |
| `.github/dependabot.yml` | Actualizaciones automáticas de dependencias (cargo, actions) |
| `.github/ISSUE_TEMPLATE/` | Formularios de incidencias GitHub (bug, funcionalidad, contacto de seguridad) |
| `.github/PULL_REQUEST_TEMPLATE.md` | Lista de verificación PR (validación, fidelidad de protocolo, seguridad) |
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
| `tests/policy_tests.rs` | Pruebas de la política (capacidades, confirmaciones, exhaustividad) |
| `tests/session_tests.rs` | Pruebas e2e del bucle (`MockBackend` + demonio de prueba) |
| `tests/ollama_backend_tests.rs` | Pruebas del backend (servidor axum loopback falso) |
| `tests/ws_tests.rs` | Pruebas del transporte WS (chat, cancel, vaciado de eventos) |

### `mule-cli` — Interfaz de línea de comandos

| Archivo | Función |
|---|---|
| `src/main.rs` | Punto de entrada del binario CLI |
| `src/lib.rs` | Biblioteca del CLI |
| `src/commands/mod.rs` | Módulo de subcomandos |
| `src/commands/hash.rs` | Comando de cálculo de hash eD2K |
| `src/commands/link.rs` | Comando de generación de enlaces eD2K |
| `src/commands/part_met.rs` | Comando de inspección de archivos `.part.met` |
| `src/commands/update.rs` | Subcomando `update` (`status`, `check`, `upload`, `rollback`) para gestionar actualizaciones vía HTTP/RPC |
| `tests/make_link_tests.rs` | Pruebas del comando de creación de enlaces |
| `tests/update_command_tests.rs` | Pruebas loopback del subcomando `mule-cli update` |

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
| `src/persistence/sqlite/categories.rs` | Persistencia de categorías |
| `src/persistence/sqlite/credits.rs` | Persistencia de créditos (subida/descarga) |
| `src/persistence/sqlite/downloads.rs` | Persistencia de descargas |
| `src/persistence/sqlite/friends.rs` | Persistencia de amigos (`friends.met`) |
| `src/persistence/sqlite/history.rs` | Persistencia del historial |
| `src/persistence/sqlite/kad.rs` | Persistencia de datos Kad |
| `src/persistence/sqlite/servers.rs` | Persistencia de servidores |
| `src/persistence/sqlite/shares.rs` | Persistencia de archivos compartidos (incl. `share_upload_stats`: totales de subida acumulados por hash) |
| `src/persistence/sqlite/sources.rs` | Persistencia de fuentes/pares |
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
| `tests/k_buckets_tests.rs` | Pruebas de los k-buckets Kad |
| `tests/part_file_and_persistence.rs` | Pruebas de archivos `.part` + persistencia |
| `tests/range_set.rs` | Pruebas del `RangeSet` |
| `tests/range_set_properties.rs` | Pruebas de propiedades del `RangeSet` |
| `tests/sqlite_categories_tests.rs` | Pruebas SQLite categorías |
| `tests/sqlite_corruption.rs` | Pruebas de recuperación ante corrupción SQLite |
| `tests/sqlite_history_tests.rs` | Pruebas SQLite historial |
| `tests/sqlite_kad_keywords_tests.rs` | Pruebas SQLite palabras clave Kad |
| `tests/sqlite_kad_sources_tests.rs` | Pruebas SQLite fuentes Kad |
| `tests/sqlite_peer_sources_tests.rs` | Pruebas SQLite fuentes de pares |
| `tests/sqlite_servers_tests.rs` | Pruebas SQLite servidores |
| `tests/sqlite_settings_tests.rs` | Pruebas SQLite ajustes |

### `mule-crypto` — Criptografía heredada de eMule

| Archivo | Función |
|---|---|
| `Cargo.toml` | Manifiesto del crate |
| `src/lib.rs` | Raíz del crate (MD4, identificadores) |
| `src/cryptkey.rs` | Formato `cryptkey.dat` (DER + Base64 Crypto++) |
| `src/legacy_sui.rs` | Secure User Identification (RSA-384 PKCS#1 v1.5 + SHA-1) |

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
| `src/rpc/server.rs` | Servidor RPC (transporte) + servicio estático de la UI web (`ServeDir`, respaldo SPA `index.html`) + `GET /session-config.js` (inyecta endpoint+token, solo loopback) |
| `src/rpc/methods.rs` | Definición de los métodos RPC |
| `src/rpc/types.rs` | Tipos del protocolo RPC |
| `src/rpc/events.rs` | Eventos RPC (notificaciones) |
| `src/rpc/handlers/mod.rs` | Módulo de los manejadores RPC |
| `src/rpc/handlers/categories.rs` | Manejador: categorías |
| `src/rpc/handlers/downloads.rs` | Manejador: descargas (`downloads.*` incl. `rename`, `setDestination`, `inspectPartMet`/`importPartMet`) |
| `src/rpc/handlers/friends.rs` | Manejador: amigos (friend slots) |
| `src/rpc/handlers/history.rs` | Manejador: historial |
| `src/rpc/handlers/kad.rs` | Manejador: Kad |
| `src/rpc/handlers/lab.rs` | Manejador: lab |
| `src/rpc/handlers/network.rs` | Manejador: política de red (`network.*`) + límites de velocidad (`limits.get`/`set` — clamp capacidad eMule 90 %/80 %, ratio anti-leech `emule_ud_ratio` 3×/4×/5× por debajo de 20 KiB/s de subida) |
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
| `src/services/incoming_share.rs` | Escaneo de la carpeta Incoming (arranque + `shares.rescan` manual): hash eD2K + compartición automática (opt-out `share_download_dir`) |
| `src/services/downloads.rs` | Servicio de gestión de descargas |
| `src/services/downloads/history_categories.rs` | Historial y categorías de las descargas |
| `src/services/downloads/operations.rs` | Operaciones sobre las descargas (pausa, reanudación…) |
| `src/services/downloads/part_met.rs` | Importación/gestión de los `.part.met` |
| `src/services/downloads/peers_servers.rs` | Fuentes de pares y servidores de las descargas |
| `src/services/downloads/types.rs` | Tipos del servicio downloads (`PartProgressSummary` + `received_mask`, `compute_part_summaries`: fuente completa `part_count==0`, frescura 30 min) |
| `src/services/events.rs` | Bus de eventos internos |
| `src/services/identity.rs` | Identidad del cliente (user hash, ident) |
| `src/services/inbound_listener.rs` | Escucha de conexiones entrantes (ofuscación en vivo vía `set_obfuscation`) |
| `src/services/inbound_listener/connection.rs` | Aceptación de conexiones entrantes |
| `src/services/inbound_listener/dispatcher.rs` | Despacho de conexiones entrantes |
| `src/services/inbound_listener/types.rs` | Tipos del listener |
| `src/services/kad.rs` | Servicio Kad (canal `kad.statusChanged` emitido por el mantenimiento cuando `total_contacts` cambia) |
| `src/services/kad/nodes_dat.rs` | Bootstrap mediante `nodes.dat` |
| `src/services/kad/search_keywords.rs` | Búsqueda por palabras clave Kad |
| `src/services/kad/search_sources.rs` | Búsqueda de fuentes Kad |
| `src/services/kad/types.rs` | Tipos del servicio Kad |
| `src/services/kad/firewall.rs` | Gestión del cortafuegos Kad (`FIREWALLED_UDP`, pruebas de conectividad) |
| `src/services/kad_udp.rs` | Servicio Kad UDP |
| `src/services/kad_udp/handlers.rs` | Manejadores de los paquetes Kad UDP |
| `src/services/kad_udp/routing_client.rs` | Cliente de enrutamiento Kad |
| `src/services/kad_udp/search_client.rs` | Cliente de búsqueda Kad |
| `src/services/client_udp.rs` | Socket client-UDP eD2K (reasks/keep-alive 0x90-0x93) |
| `src/services/server_udp.rs` | Socket UDP de servidor (`GLOBGETSOURCES2`/`GLOBFOUNDSOURCES`) + failover en `swarm_coordinator/strategy.rs` |
| `src/services/uss.rs` | USS — límite de subida dinámico por latencia (ley `LastCommonRouteFinder`, puerta `accept_new_client` sobre la cola); reajusta el bucket de descarga vía `emule_ud_ratio` en cada tick (eMule `bDynamic`) |
| `src/services/uss_icmp.rs` | USS — traceroute ICMP Windows (`IcmpSendEcho*`) para el *Last Common Route Finder* de eMule: descubre el último router común y su latencia de referencia |
| `src/services/kad_udp/storage.rs` | Nodo de almacenamiento Kad (TTL, límites anti-flood) |
| `src/services/kad_udp/types.rs` | Tipos Kad UDP |
| `src/services/obfuscation_stream.rs` | Flujo de red ofuscado (ofuscación de protocolo) |
| `src/services/peer_session.rs` | Sesión con un par eD2K (+ helpers `pending_ranges*`: bloques en vuelo hacia `DownloadSnapshot.pendingRanges`) |
| `src/services/peer_session/connection.rs` | Conexión al par |
| `src/services/peer_session/events.rs` | Eventos de sesión de par |
| `src/services/peer_session/handshake.rs` | Handshake eD2K con el par |
| `src/services/peer_session/source_exchange.rs` | Intercambio de fuentes entre pares |
| `src/services/peer_session/status_query.rs` | Peticiones de estado de archivo — `OP_FILESTATUS` → `record_source_availability` + emite `downloadUpdated` (máscaras de partes, inicial y keep-alive en cola) |
| `src/services/peer_session/sui.rs` | Secure User Identification en la sesión de par |
| `src/services/peer_session/transfer.rs` | Transferencia de bloques con el par |
| `src/services/peer_session/types.rs` | Tipos de sesión de par |
| `src/services/server_session.rs` | Sesión con un servidor eD2K |
| `src/services/server_session/connection.rs` | Conexión al servidor |
| `src/services/server_session/events.rs` | Eventos de sesión de servidor |
| `src/services/server_session/types.rs` | Tipos de sesión de servidor |
| `src/services/swarm_coordinator.rs` | Coordinación del enjambre de pares |
| `src/services/swarm_coordinator/strategy.rs` | Estrategia de coordinación (rebalanceo A4AF incluido) |
| `src/services/swarm_coordinator/types.rs` | Tipos del coordinador |
| `src/services/upload_queue.rs` | Cola de espera de subida (trickle slots, friend slots, anti-leecher, tamaño configurable `set_max_queue`) |
| `src/services/upnp.rs` | Servicio UPnP (`set_enabled`, `set_ports`, map/unmap, refresco periódico) |
| `src/services/upnp/discovery.rs` | Descubrimiento de la pasarela UPnP |
| `src/services/upnp/mapping.rs` | Mapeo de puertos UPnP |
| `src/services/upnp/types.rs` | Tipos UPnP |
| `src/services/updater/` | Servicio híbrido de actualizaciones (`constants.rs`, `error.rs`, `http_fetch.rs`, `installer.rs`, `manifest.rs`, `service.rs`, `verifier.rs`) con validación ELF64/PE, respaldo `.old` O(1) y rollback |

### `mule-daemon` — Pruebas

| Archivo | Función |
|---|---|
| `tests/category_rpc_tests.rs` | Pruebas RPC categorías |
| `tests/daemon_adversarial.rs` | Pruebas adversariales del demonio |
| `tests/download_details_and_destination_tests.rs` | Pruebas detalles/destino de las descargas |
| `tests/dual_portable_bundles.rs` | Pruebas de paquetes portátiles dobles |
| `tests/ed2k_obfuscation_tests.rs` | Pruebas de ofuscación eD2K |
| `tests/ed2k_source_discovery.rs` | Pruebas de descubrimiento de fuentes eD2K |
| `tests/ed2k_transfer_tests.rs` | Pruebas de transferencia eD2K |
| `tests/filter_rpc_tests.rs` | Pruebas RPC de filtrado |
| `tests/history_rpc_tests.rs` | Pruebas RPC historial |
| `tests/inter_daemon_transfer.rs` | Pruebas de transferencia entre demonios |
| `tests/aich_tests.rs` | Pruebas AICH demonio (consenso, recuperación) |
| `tests/hashset_tests.rs` | Pruebas verificación hashset por parte |
| `tests/kad_bootstrap_tests.rs` | Pruebas de bootstrap Kad |
| `tests/kad_keywords_tests.rs` | Pruebas palabras clave Kad |
| `tests/kad_rpc_tests.rs` | Pruebas RPC Kad |
| `tests/kad_sources_tests.rs` | Pruebas fuentes Kad |
| `tests/kad_udp_tests.rs` | Pruebas Kad UDP |
| `tests/lab_rpc_tests.rs` | Pruebas RPC lab |
| `tests/lifecycle_and_locking.rs` | Pruebas ciclo de vida y bloqueo |
| `tests/network_policy_rpc_tests.rs` | Pruebas RPC política de red |
| `tests/orphan_recovery_tests.rs` | Pruebas recuperación de `.part.met` huérfanos, tombstone `.keep`, volcado de mets |
| `tests/part_met_rpc_tests.rs` | Pruebas RPC `part.met` |
| `tests/part_met_write_tests.rs` | Pruebas escritura `.part.met` |
| `tests/peer_adversarial_tests.rs` | Pruebas adversariales de pares |
| `tests/peer_connection_tests.rs` | Pruebas de conexión a pares |
| `tests/rpc_contract.rs` | Pruebas de contrato RPC |
| `tests/secure_ident_tests.rs` | Pruebas de identificación segura |
| `tests/server_connection_tests.rs` | Pruebas de conexión a servidores |
| `tests/servers_rpc_tests.rs` | Pruebas RPC servidores |
| `tests/swarm_dual_peer_tests.rs` | Pruebas enjambre a dos pares |
| `tests/upload_inbound_tests.rs` | Pruebas subida/conexiones entrantes |
| `tests/updater_tests.rs` | Pruebas loopback del sistema híbrido de actualizaciones, subida autenticada y rollback |
| `tests/upnp_and_wan_tests.rs` | Pruebas UPnP y WAN |
| `tests/workdir_tests.rs` | Pruebas del directorio de trabajo |

### `mule-filter` — Filtrado IP y limitación de caudal

| Archivo | Función |
|---|---|
| `src/lib.rs` | Raíz del crate |
| `src/filter.rs` | Motor de filtrado IP |
| `src/interval.rs` | Intervalos de direcciones IP |
| `src/parser.rs` | Analizador de archivos `ipfilter.dat` |
| `src/pseudonym.rs` | Soporte de seudónimos |
| `src/token_bucket.rs` | Token bucket (limitación de caudal) |
| `tests/filter_tests.rs` | Pruebas del filtrado |
| `tests/token_bucket_tests.rs` | Pruebas del token bucket |

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
| `src/met/error.rs` | Errores de formato `.met` |
| `src/met/part_met.rs` | Estructura de los archivos `.part.met` |
| `src/met/reader.rs` | Lector de archivos `.met` |
| `src/met/server_met.rs` | Análisis de `server.met` |
| `src/met/tags.rs` | Etiquetas de los archivos `.met` |
| `tests/aich_vectors.rs` | Vectores de prueba AICH |
| `tests/ed2k_hash_vectors.rs` | Vectores de prueba hash eD2K |
| `tests/link_vectors.rs` | Vectores de prueba enlaces eD2K |
| `tests/nodes_dat_tests.rs` | Pruebas `nodes.dat` |
| `tests/official_file_hash.rs` | Prueba de hash sobre archivo oficial |
| `tests/part_met_vectors.rs` | Vectores de prueba `.part.met` (lectura/escritura) |
| `tests/server_met_vectors.rs` | Vectores de prueba `server.met` |

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
| `src/tools/servers.rs` | Herramientas MCP: servidores |
| `src/tools/shares.rs` | Herramientas MCP: compartidos |
| `src/tools/shares/handlers.rs` | Manejadores de las herramientas shares |
| `src/tools/shares/schemas.rs` | Esquemas JSON de las herramientas shares |
| `src/tools/system.rs` | Herramientas MCP: sistema/estado |
| `src/tools/policy.rs` | `ToolRisk`/`ToolProfile`: clasificación y filtrado de herramientas (`--tool-profile read|safe|full`) |
| `tests/mcp_protocol_tests.rs` | Pruebas del protocolo MCP |
| `tests/tool_profile_tests.rs` | Pruebas de los perfiles de filtrado (exhaustividad, rechazo de ejecución) |

### `mule-network-policy` — Política de red

| Archivo | Función |
|---|---|
| `src/lib.rs` | Raíz del crate |
| `src/audit.rs` | Registro de auditoría de decisiones de red |
| `src/budgets.rs` | Presupuestos de red (ancho de banda) |
| `src/cancellation.rs` | Cancelación de operaciones de red |
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
| `src/kad/encoder.rs` | Codificador Kad |
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
| `tests/common/mod.rs` | Utilidades de prueba compartidas |
| `tests/common/sha256.rs` | SHA-256 para las pruebas |
| `tests/ed2k_aich_tests.rs` | Pruebas códec AICH (opcodes 0x9B-0x9E) |
| `tests/ed2k_block_transfer_tests.rs` | Pruebas de transferencia de bloques eD2K |
| `tests/ed2k_corpus_tests.rs` | Pruebas sobre corpus de paquetes |
| `tests/ed2k_emule_compat_tests.rs` | Pruebas de compatibilidad de red eMule |
| `tests/ed2k_extended_tests.rs` | Pruebas códecs eMule 0.72a extendidos (HashSet2, MultiPacket*, ext-info por versión negociada, CAPTCHA…) |
| `tests/ed2k_file_status_tests.rs` | Pruebas de estados de archivo |
| `tests/ed2k_hashset_tests.rs` | Pruebas códec hashset (0x51/0x52) |
| `tests/ed2k_isolation_guard.rs` | Guarda de aislamiento de pruebas eD2K |
| `tests/ed2k_part_resume_tests.rs` | Pruebas de reanudación de partes |
| `tests/ed2k_reference_interop_tests.rs` | Pruebas de interoperabilidad con la referencia |
| `tests/ed2k_roundtrip_tests.rs` | Pruebas roundtrip encode/decode |
| `tests/ed2k_sequencer_tests.rs` | Pruebas del secuenciador |
| `tests/ed2k_session_tests.rs` | Pruebas de sesión eD2K |
| `tests/in_flight_properties.rs` | Propiedades del seguimiento in-flight |
| `tests/kad_codec_tests.rs` | Pruebas del códec Kad |
| `tests/lab_adversarial.rs` | Pruebas adversariales lab |
| `tests/lab_in_flight_and_blocks.rs` | Pruebas in-flight y bloques lab |
| `tests/lab_state_machine.rs` | Pruebas máquina de estados lab |
| `tests/loopback_transfer.rs` | Pruebas de transferencia en loopback |
| `tests/malformed_packets.rs` | Pruebas de paquetes malformados |
| `tests/packet_codec.rs` | Pruebas del códec de paquetes |
| `tests/protocol_limits.rs` | Pruebas de los límites del protocolo |
| `tests/server_codec_tests.rs` | Pruebas del códec de servidor |
| `tests/tier3_loopback.rs` | Pruebas loopback tier 3 |
| `tests/tier3_external_reference/mod.rs` | Módulo de pruebas de referencia externa |
| `tests/tier3_external_reference/environment.rs` | Entorno de prueba externa |
| `tests/tier3_external_reference/harness.rs` | Arnés de prueba externa |
| `tests/tier3_external_reference/report.rs` | Informe de pruebas externas |
| `tests/tier3_external_reference/README.md` | Documentación de pruebas de referencia externa |
| `tests/fixtures/ed2k/README.md` | Documentación de fixtures eD2K |
| `tests/fixtures/ed2k/*.bin` | Paquetes binarios capturados (`hello`, `file_status`, `request_parts`, `sending_part`…) |
| `tests/fixtures/ed2k/malformed/*.bin` | Paquetes malformados para pruebas negativas |
| `tests/fixtures/ed2k/manifest.json` | Manifiesto que describe los fixtures |
| `tests/fixtures/ed2k/reference_manifest.json` | Manifiesto de referencia |

## docs/ — Documentación

### Raíz de la documentación
| Archivo | Función |
|---|---|
| `docs/README.md` | Portal de orientación de la documentación técnica |
| `docs/INDEX.md` | Índice exhaustivo de los archivos del repositorio |
| `docs/ASSISTANT.md` | Guía de uso del asistente de IA local: instalación, modos REPL/prompt/serve, perfiles de herramientas, seguridad, protocolo WS, resolución de problemas |
| `docs/CHANGELOG.md` | Registro de cambios (Keep a Changelog) |
| `docs/SETTINGS.md` | Referencia completa de los ajustes (`settings.get`/`set`): claves RPC/persistidas, valores por defecto, aplicación en caliente, equivalentes en eMule |

### docs/architecture/ — Arquitectura e informes
| Archivo | Función |
|---|---|
| `docs/architecture/rapport_architecture_et_avancement.md` | Informe detallado de arquitectura de software, Clean Architecture y estado de avance |
| `docs/architecture/rpc_protocol.md` | Contrato JSON-RPC 2.0/WebSocket demonio↔clientes: transporte, auth, métodos, eventos, códigos de error, versionado |
| `docs/architecture/plan_daemon_events.md` | Plan de eventos del demonio: subidas en tiempo real, `statsTick`, ciclo de vida de descargas, fin del polling (fases E1–E6) |
| `docs/architecture/adr_auto_update.md` | ADR: Sistema de actualización automática integrado (In-App Staging + Atomic Self-Replace, Ed25519, graceful shutdown) |
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
| `docs/releases/RELEASE_NOTES_0.3.0-beta.2.md` | Notas de la versión 0.3.0-beta.2 |
| `docs/releases/RELEASE_NOTES_0.3.0-beta.3.md` | Notas de la versión 0.3.0-beta.3 |
| `docs/releases/RELEASE_NOTES_0.3.0-beta.4.md` | Notas de la versión 0.3.0-beta.4 |
| `docs/releases/RELEASE_NOTES_0.4.0-beta.1.md` | Notas de la versión 0.4.0-beta.1 |
| `docs/releases/RELEASE_NOTES_0.4.0-beta.2.md` | Notas de la versión 0.4.0-beta.2 |

### docs/qa/ — Garantía de calidad y conformidad
| Archivo | Función |
|---|---|
| `docs/qa/QA_RECEPTION_PROTOCOL.md` | Protocolo de recepción QA |
| `docs/qa/compatibility-matrix.md` | Matriz de compatibilidad |
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

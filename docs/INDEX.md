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
| `CLAUDE.md` | Enlace físico hacia `AGENTS.md` (compatibilidad Claude Code) |
| `GEMINI.md` | Enlace físico hacia `AGENTS.md` (compatibilidad Gemini CLI) |
| `.github/copilot-instructions.md` | Enlace físico hacia `AGENTS.md` (compatibilidad GitHub Copilot) |
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
| `Asset/generated-image.png` | Imagen fuente del icono de la aplicación (1254×1254 RGBA) — adaptada a `app_icon.ico` Windows/tray e iconos web/macOS/iOS |

## .agents/ — Configuración de agentes/MCP

| Archivo | Función |
|---|---|
| `.agents/mcp_config.json` | Configuración del servidor MCP |
| `.agents/plugins/emulerust/mcp_config.json` | Configuración MCP del plugin emulerust |
| `.agents/plugins/emulerust/plugin.json` | Manifiesto del plugin de agente |

## apps/desktop/ — Aplicación Flutter (Windows, histórico)

### Núcleo de la aplicación (`lib/`)

| Archivo | Función |
|---|---|
| `lib/main.dart` | Punto de entrada: compone `app_shell` + `mule_client` |
| `lib/core/deep_link_handler.dart` | Enlaces `ed2k://` entrantes |
| `lib/core/download_notifier.dart` | Notificaciones del sistema sobre eventos de descarga |
| `lib/core/tray_setup.dart` | Icono y menú de la bandeja del sistema |
| `lib/core/window_setup.dart` | Configuración de la ventana nativa |
| `lib/services/daemon_supervisor.dart` | Supervisión del ciclo de vida del demonio (inicio/parada) |
| `lib/services/work_directory_resolver.dart` | Resolución del directorio de trabajo del demonio |

### Configuración Flutter / plataformas

| Archivo | Función |
|---|---|
| `pubspec.yaml` | Manifiesto del paquete Flutter (dependencias) |
| `pubspec.lock` | Bloqueo de dependencias Dart/Flutter |
| `analysis_options.yaml` | Reglas de análisis estático Dart |
| `.metadata` | Metadatos del proyecto Flutter |
| `README.md` | Documentación de la app de escritorio |
| `assets/app_icon.ico` | Icono de la aplicación |
| `android/`, `ios/`, `linux/`, `macos/`, `windows/` | Runners nativos generados por Flutter (solo `windows/` es el objetivo soportado: `windows/ed2k_protocol.reg` registra el protocolo `ed2k://`, `runner/` contiene el código Win32) |

### Pruebas Flutter (`test/`)

| Archivo | Función |
|---|---|
| `test/domain/download_item_test.dart` | Pruebas del modelo `DownloadItem` |
| `test/integration/live_daemon_e2e_test.dart` | Prueba E2E frente a un demonio real |
| `test/services/daemon_supervisor_test.dart` | Pruebas del supervisor del demonio |
| `test/services/work_directory_resolver_test.dart` | Pruebas del resolvedor de directorio de trabajo |
| `test/ui/*.dart` (18 archivos) | Pruebas UI: categorías, diagnósticos, detalles, pantallas downloads/history/Kad/search/servers, importación part.met, IP filter, política de red, fuentes de pares, responsive, rendimiento en listas grandes, settings, compartidos, barra de disponibilidad de partes (`part_availability_bar_test`) |

## apps/web/ — Aplicación Flutter Web (histórico)

| Archivo | Función |
|---|---|
| `lib/main.dart` | Punto de entrada de la app web — resolución del demonio: `?daemon=` > `window.MULE_SESSION` (inyectado por `session-config.js` cuando lo sirve el demonio) > dart-define > prefs > formulario; token mediante `?token=`/formulario/prefs |
| `lib/daemon_connect_screen.dart` | Pantalla de conexión al demonio (URL ws/wss + campo de token opcional) |
| `pubspec.yaml` / `pubspec.lock` / `analysis_options.yaml` | Configuración Flutter |
| `web/` | Plantilla base web (index.html + `<script src="session-config.js">`, manifiesto, iconos) |

## packages/app_shell/ — Shell UI compartido (MVVM, histórico)

Paquete Dart que contiene toda la interfaz: composición de la aplicación (`lib/app/`), servicios transversales (`lib/core/`), localización (`lib/l10n/`) y pantallas por funcionalidad (`lib/ui/features/`).

### Configuración y composición

| Archivo | Función |
|---|---|
| `pubspec.yaml` / `analysis_options.yaml` / `l10n.yaml` | Configuración del paquete (incluida generación i18n) |
| `lib/emulerust_app.dart` | Widget raíz de la aplicación |
| `lib/app/app_controller.dart` | Raíz de composición: sesión del demonio + cableado de controladores |
| `lib/app/providers.dart` | Providers Riverpod de la aplicación |
| `lib/app/router.dart` | Enrutamiento/navegación entre pantallas |
| `lib/app/shell_controller.dart` | Controlador del shell (estado global de la UI) |
| `lib/app/locale_controller.dart` | Controlador de idioma |
| `lib/app/stats_controller.dart` | Controlador de estadísticas globales |
| `lib/app/transfer_rate_tracker.dart` | Seguimiento de velocidades de transferencia |

### Servicios transversales (`lib/core/`)

| Archivo | Función |
|---|---|
| `lib/core/assistant_token.dart` (+`_io`, `_stub`) | Resolución del token de sesión para `assistant.hello` (`<workdir>/auth.token`) — solo escritorio |
| `lib/core/daemon_lifecycle.dart` | Ciclo de vida de la conexión al demonio |
| `lib/core/folder_picker.dart` (+`_io`, `_stub`) | Selector de carpetas (abstracción + impl nativa/stub) |
| `lib/core/file_picker_helper.dart` (+`_io`, `_stub`) | Selector de archivos nativo (`pickFile`, `file_picker` 12.3) — ipfilter.dat, server.met, nodes.dat, .part.met |
| `lib/core/system_actions.dart` (+`_io`, `_stub`) | Acciones del sistema (abrir carpeta, mostrar archivo, abrir URL en navegador) |
| `lib/core/work_dirs.dart` | Directorios de trabajo (downloads, incoming…) |

### Localización (`lib/l10n/`)

| Archivo | Función |
|---|---|
| `lib/l10n/l10n.dart` | Punto de acceso a las localizaciones |
| `lib/l10n/l10n_lookup.dart` | `L10nLookup`: acceso a `AppLocalizations` sin `BuildContext` (mensajes de los controladores), sincronizado con `LocaleController` |
| `lib/l10n/enum_labels.dart` | Extensiones localizadas sobre los enums de `mule_client` (`DownloadStatus`, `DaemonConnectionState`, `SupervisorState`, `NetworkProfile`, prioridades…) |
| `lib/l10n/app_fr.arb` / `app_en.arb` | Recursos de traducción |
| `lib/l10n/generated/` | Localizaciones generadas (`app_localizations*.dart`) |

### Funcionalidad `downloads` (`lib/ui/features/downloads/`)

| Archivo | Función |
|---|---|
| `view_models/downloads_store_controller.dart` | Store principal de descargas |
| `view_models/categories_controller.dart` | Controlador de categorías |
| `view_models/lab_controller.dart` | Controlador del laboratorio |
| `view_models/network_policy_controller.dart` | Controlador de la política de red |
| `view_models/peer_sources_controller.dart` | Controlador de fuentes de pares — `getVisibleSourcesForHash` filtra la deadlist de eMule (fallidos/rechazados/filtrados ocultos) |
| `views/downloads_screen.dart` | Pantalla principal de descargas |
| `widgets/downloads_list.dart`, `downloads_top_bar.dart`, `downloads_sidebar.dart`, `downloads_empty_state.dart` | Lista, barra de herramientas, barra lateral y estado vacío |
| `widgets/download_row.dart`, `download_details_panel.dart`, `download_detail_dialog.dart` | Fila de descarga y panel/diálogo de detalle |
| `widgets/download_sources_expansion.dart` | Expansión de fuentes de una descarga — solo muestra la srclist de eMule (vivas), no las entradas deadlist |
| `widgets/speed_sparkline.dart`, `debug_simulation_bar.dart` | Sparkline de velocidad, barra de simulación |
| `widgets/add_link_dialog.dart` + `widgets/add_link/` | Diálogo para añadir enlace ed2k (`ed2k_link_parser`, desplegables categoría/asignación, vista previa) |
| `widgets/category_edit_dialog.dart` + `widgets/category/` | Edición de categoría (selector de color, desplegable de prioridad) |
| `widgets/detail/` (16 archivos) | Tarjetas de detalle: cabecera, barra de acciones, identidad, métricas, mapa de partes, rutas, fuentes, diagnósticos, diálogos destino/eliminación, `peer_source_row` |
| `widgets/diagnostic_dialog.dart` + `widgets/diagnostic/` (8 archivos) | Tarjeta de diagnóstico: chips de capacidades, filas de información, formato, banner de perfil de red |
| `widgets/import/` (7 archivos) + `import_part_met_dialog.dart` | Importación defensiva `.part.met`: inspección, informe, aviso de seguridad |
| `widgets/lab/` (9 archivos) + `lab_guide_dialog.dart` | Asistente de laboratorio: pasos fixture/enlace/par/conexión/limpieza |
| `widgets/network/` (8 archivos) + `network_policy_dialog.dart`, `ip_filter_dialog.dart` | Pestañas IP filter y límites de velocidad, cuotas, banner de estado de red, registro de auditoría |
| `widgets/row/` (9 archivos) | Fila de descarga: variantes compacta/completa, insignias, iconos, barra de acciones, píldora de estado, chip de prioridad (`download_priority_chip.dart`), diálogo de renombrado (`rename_file_dialog.dart`) |
| `widgets/settings/` (5 archivos) | Bloques reutilizados por la página Ajustes: carpeta, idioma, retención, amigos, mensaje inline |
| `widgets/sidebar/` (9 archivos) | Barra lateral: cabecera de marca, elementos nav/categoría/filtro, tarjeta de estado del demonio (`sidebar_status_card` — versión, indicadores eD2K/Kad en color, nombre del servidor), eliminación de categoría |
| `widgets/incompatible_version_dialog.dart` | Aviso de versión del demonio incompatible |

### Funcionalidades `assistant`, `history`, `kad`, `search`, `servers`, `settings`, `shares`, `uploads`

| Archivo | Función |
|---|---|
| `assistant/view_models/assistant_controller.dart` + `views/assistant_view.dart` + `widgets/` (6 archivos) | Asistente de IA local (`/assistant`): chat en streaming, tarjetas de herramientas, banner de confirmación, estado sin conexión/reintento, barra de entrada; `AssistantController` gestiona conexión WS, entradas, confirmaciones y cancelación |
| `history/view_models/history_controller.dart` + `views/history_view.dart` + `widgets/` (9 archivos) | Historial: vista, barra de herramientas, filtro, panel de estadísticas, filas de entrada, diálogos detalle/exportación/borrado |
| `kad/view_models/kad_controller.dart` + `views/kad_screen.dart` + `widgets/` (12 archivos) | Kad: tabla de contactos, bootstrap (diálogo + URL), importación `nodes.dat`, insignias/estados, banner de seguridad |
| `search/view_models/search_controller.dart` + `views/search_screen.dart` + `widgets/` (7 archivos) | Búsqueda: formulario, tabla de resultados, banner de conexión, estados |
| `servers/view_models/servers_controller.dart` + `views/servers_screen.dart` + `widgets/` (12 archivos) | Servidores: tabla y tarjetas, importación `server.met`, barra de sesión, insignias, diálogos limpiar/actualizar |
| `settings/view_models/settings_controller.dart` + `views/settings_view.dart` + `views/sections/` (6 archivos) + `widgets/` (3 archivos) | Página Ajustes (`/settings`): barra General/Conexión/Transferencias/Seguridad/Amigos/Avanzado; apodo, compartición Incoming, **interruptores Red eD2K/Kad** (`network_toggles_card`), kill switch/cuotas, **capacidades y puertos TCP/UDP, interruptores UDP+UPnP**, ofuscación, conexiones máx., USS, límites de fuentes, cola de subida, IP filter, amigos, herramientas avanzadas |
| `shares/views/shares_view.dart` + `shares/view_models/shares_controller.dart` | Pestaña «Compartidos» (`/shares`): archivos servidos (persistidos + parciales `.part` con insignia «parcial», **descargas activas siempre publicadas desde su creación**), **velocidad de subida actual + total acumulado por archivo** (`uploadRateBps`/`uploadedBytes`), **agrupación por categoría** (`shared_files.category_id` vía `shares.setCategory`, orden por total subido descendente, «Sin categoría» al final), **buscador** (nombre + hash), renombrado (`shares.rename`), prioridad de subida (powershare), interruptor activado/suspendido, **botón reescanear Incoming** (`shares.rescan`); el controlador alimenta la insignia de la barra lateral en vivo |
| `uploads/view_models/uploads_controller.dart` + `views/uploads_view.dart` + `widgets/uploads_table.dart` | Subidas: vista y tabla de envíos |

## packages/mule_client/ — Cliente del demonio compartido (histórico)

| Archivo | Función |
|---|---|
| `lib/mule_client.dart` | Barrel: exportación única del paquete |
| `lib/data/assistant_client.dart` | Cliente WS/JSON-RPC de `mule-assistant` (`assistant.hello/chat/confirm/cancel`, eventos en streaming) |
| `lib/data/rpc_daemon_client.dart` | Cliente RPC real que se comunica con el demonio (WebSocket/JSON-RPC) |
| `lib/data/fake_daemon_client.dart` | Cliente del demonio simulado para pruebas/desarrollo sin demonio |
| `lib/data/dto/rpc_dto.dart` | Objetos de transferencia de datos para el protocolo RPC |
| `lib/domain/services/mule_daemon_client.dart` | Interfaz abstracta del cliente del demonio (contrato) |
| `lib/domain/models/assistant_models.dart` | Modelos: `AssistantEvent`, `AssistantConfirmRequest`, `ChatEntry`, estados de conexión del asistente |
| `lib/domain/models/category_item.dart` | Modelo: categoría de descarga |
| `lib/domain/models/daemon_event.dart` | Modelo: evento emitido por el demonio |
| `lib/domain/models/daemon_info.dart` | Modelo: información de estado del demonio |
| `lib/domain/models/download_item.dart` | Modelo: elemento de descarga (`parts[]` con `receivedMask` mapa de bits de 64 celdas, `pendingRanges` bloques en vuelo, `ByteRange`) |
| `lib/domain/models/filter_models.dart` | Modelos: filtros (IP filter, etc.) |
| `lib/domain/models/history_entry.dart` | Modelo: entrada de historial |
| `lib/domain/models/history_stats.dart` | Modelo: estadísticas de historial |
| `lib/domain/models/lab_models.dart` | Modelos: funcionalidades «lab» experimentales |
| `lib/domain/models/network_policy_models.dart` | Modelos: política de red |
| `lib/domain/models/part_met_models.dart` | Modelos: metadatos de archivos `.part.met` |
| `lib/domain/models/server_models.dart` | Modelos: servidores eD2K |
| `pubspec.yaml` | Manifiesto del paquete |
| `test/assistant_client_test.dart` | Pruebas del cliente asistente (servidor WS loopback falso) |
| `test/fake_daemon_client_test.dart` | Pruebas del cliente simulado |
| `test/rpc_daemon_client_test.dart` | Pruebas del cliente RPC |
| `README.md` | Uso, protocolo y comportamientos de transporte del cliente |

## packages/ui_kit/ — Sistema de diseño compartido (histórico)

| Archivo | Función |
|---|---|
| `lib/ui_kit.dart` | Barrel: exportación única del paquete |
| `lib/src/theme.dart` | `AppTheme`: colores de marca/estado, `dark()` ThemeData |
| `lib/src/tokens.dart` | Design tokens (espaciados, radios, colores) |
| `lib/src/formatters.dart` | `Formatters`: bytes, velocidades, ETA, porcentajes |
| `lib/src/strings.dart` | `AppStrings`: cadenas UI centralizadas (base i18n) |
| `lib/src/part_availability_bar.dart` | Barra `DrawStatusBar` eMule 0.72a precisa por bytes (CustomPaint), paleta estándar plana: azul = recibido, rojo = sin fuente, degradado cian→azul `RGB(0,g,255)` eMule raro→común, amarillo = en vuelo, verde = verificado y franja de progreso; constantes `partHaveColor`/`partVerifiedColor`/`partMissingColor`/`partPendingColor`/`partProgressColor` compartidas con la tarjeta part-map |
| `pubspec.yaml` | Manifiesto del paquete |
| `README.md` | Contenido y reglas del sistema de diseño |

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
| `tests/make_link_tests.rs` | Pruebas del comando de creación de enlaces |

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

### docs/walkthroughs/ — Guías de ejecución e interoperabilidad por fase
| Archivo | Función |
|---|---|
| `docs/walkthroughs/README.md` (o `INDEX.md`) | Índice maestro y tabla de orientación de todos los walkthroughs |
| `docs/walkthroughs/walkthrough_Phase_01_a_03c.md` | Guía de ejecución y endurecimiento del producto (Fases 1 a 3C) |
| `docs/walkthroughs/walkthrough_Tier3_eMule050a.md` | Interoperabilidad loopback con eMule 0.50a oficial (Tier 3.1 a 3.6) |
| `docs/walkthroughs/walkthrough_Phase_07.md` | Modo portátil, aislamiento `data/` frente a `downloads/` y persistencia SQLite (Fase 7) |
| `docs/walkthroughs/walkthrough_Phase_08.md` | Mapa de partes interactivo, historial local, categorías e importación `.part.met` (Fase 8) |
| `docs/walkthroughs/walkthrough_Phase_09.md` | Política de red defensiva en 7 capas, permisos RAII, Kill Switch e IP Filter (Fase 9) |
| `docs/walkthroughs/walkthrough_Phase_10.md` | Infraestructura de servidores eD2K, TCP Lugdunum, HighID/LowID, búsqueda y fuentes (Fase 10) |
| `docs/walkthroughs/walkthrough_Phase_11.md` | Swarm multifuente, asignador Water-Filling y arranque de baja latencia (Fase 11) |
| `docs/walkthroughs/walkthrough_Phase_12.md` | Fundamentos Kad DHT, KadId de 128 bits, K-Buckets y transporte UDP (Fase 12) |
| `docs/walkthroughs/walkthrough_Phase_13.md` | Source Exchange v4 (SX1/SX2), compresión zlib y servidor MCP de 83 herramientas (Fase 13) |
| `docs/walkthroughs/walkthrough_Phase_14.md` | Convergencia de búsqueda Kad DHT (`CUInt128`) y Clean Architecture completa (Fase 14) |
| `docs/walkthroughs/walkthrough_Phase_15.md` | Swarm multipares real, conformidad de colas de eMule y registros estratégicos (Fase 15) |
| — (Fases 16 y 17) | Documentadas en línea en `docs/architecture/rapport_architecture_et_avancement.md` §10/§11 + `docs/CHANGELOG.md` (interoperabilidad eMule 0.72a; fidelidad UI: compartidos parciales, contadores en tiempo real, barra de partes `DrawStatusBar`) |

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

### docs/plans/ — Planes de implementación

| Archivo | Función |
|---|---|
| `docs/plans/kad/00_index.md` | Hoja de ruta Kad DHT: diagnóstico verificado (eMule 0.72a), orden de los 9 pasos, exclusiones de protocolo |
| `docs/plans/kad/01_probing_lru.md` | Paso 1: `ContactVerifier` — sondeo de candidatos LRU devueltos por `insert_or_update` |
| `docs/plans/kad/02_hello_req.md` | Paso 2: emisión `HELLO_REQ` (verificación activa de contactos Kad2, ACK de 3 vías) |
| `docs/plans/kad/03_firewall_revalidation.md` | Paso 3: revalidación de cortafuegos UDP ante cambio de IP/NAT/expiración |
| `docs/plans/kad/04_bootstrap_health.md` | Paso 4: `BootstrapHealth` multiestado (Empty/Fragile/Building/Healthy) para el drenaje |
| `docs/plans/kad/05_udp_robustesse.md` | Paso 5: errores tipados `KadTxError`, reintento acotado, ventana de recogida deslizante, emparejamiento endurecido |
| `docs/plans/kad/06_contention_mutex.md` | Paso 6: instantánea de la tabla bajo bloqueo + ordenación fuera de bloqueo |
| `docs/plans/kad/07_recherche.md` | Paso 7: codificador de expresión restrictiva, multitérmino, paginación, coalescing compartido |
| `docs/plans/kad/08_buddy_client.md` | Paso 8: rol cliente buddy LowID (`FINDBUDDY_REQ`, enlace TCP, publicación LowID) |
| `docs/plans/kad/09_validation_interop.md` | Paso 9: golden packets eMule/aMule, proptest, laboratorio loopback (pérdida/duplicación/reordenación) |

## scripts/ — Scripts PowerShell

| Archivo | Función |
|---|---|
| `build_windows_release.ps1` | Compilación de release Windows (desktop + web empaquetada bajo `web/`, lanzador `Interface Web.bat`, `-SkipWeb`/`-WebBaseHref`) |
| `generate_sbom.ps1` | Generación del SBOM (Bill of Materials) |
| `generate_update_manifest.py` | Firma criptográfica Ed25519 y generación de `manifest.json` para actualizaciones |
| `test_packaged_bundle.ps1` | Prueba del paquete empaquetado |
| `test_qa_loopback_e2e.ps1` | Prueba E2E loopback QA |
| `test_rc_matrix.ps1` | Matriz de pruebas release candidate |
| `verify_all.ps1` | Script de verificación global |

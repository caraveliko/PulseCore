# Walkthrough — Fase 8.1 y 8.2: Inspección, Destinos, Historial y Estadísticas Locales

La Fase 8 enriquece el cliente de escritorio **eMuleRust** con una gestión local completa de las transferencias y de la auditoría, manteniendo estrictamente la frontera local loopback (`127.0.0.1`), la separación entre los datos temporales de trabajo (`data/temp/`) y los directorios finales (`downloads/`), así como una política de cero telemetría.

---

## 1. Vista detallada de descarga (`DownloadDetailDialog` - Fase 8.1)

Accesible mediante el icono de inspección en cada fila de descarga o mediante el botón **"Inspect Full Details"** del panel de selección.

### Funcionalidades principales:
- **Identificadores y Enlaces eD2K canónicos**:
  - UUID completo de la descarga con copia al portapapeles.
  - Hash eD2K MD4 con copia inmediata.
  - Enlace eD2K canónico (`ed2k://|file|<nombre>|<tamaño>|<hash>|/`) con copia en un clic.
- **Métricas de transferencia en directo**:
  - Tamaños: total, recibido, restante y bytes verificados por hash.
  - Tasa de transferencia instantánea, velocidad media de la sesión y estimación del tiempo restante (ETA).
  - Número de intentos de conexión y visualización del último error eventual.
- **Mapa de partes eD2K (Part Map)**:
  - Segmentación visual en bloques estándar de 9 728 000 bytes (incluida la última parte residual, p. ej.: 9 728 001 bytes).
  - Código de colores y leyenda interactiva:
    - `TealAccent`: Verificado (hash validado)
    - `BlueAccent`: Completado (pendiente de validación)
    - `AmberAccent`: En curso de descarga
    - `White12 / Border`: Faltante
  - Tooltips interactivos que muestran el índice de parte, el rango de bytes (`startOffset - endOffset`), el tamaño y el estado.
- **Rutas de almacenamiento y Destino objetivo**:
  - Ruta temporal confinada: `data/temp/<id>.part` (solo lectura segura).
  - Ruta de destino finalizada: modificación mediante diálogo con prueba de escritura (write-probe).
  - Información explícita: la modificación del destino solo afecta al ensamblaje final; el fichero `.part` permanece confinado en `data/temp/` durante la transferencia.
- **Exportación de diagnóstico saneada (Sanitized Diagnostics)**:
  - Generación de un informe de diagnóstico expurgado de cualquier token de autenticación (`auth.token`), con anonimización de los directorios de usuario (`[DOWNLOAD_DIR]`) y confirmación de confinamiento loopback.
- **Barra de acciones aislada (Action Bar)**:
  - **Pausar / Reanudar**.
  - **Reintentar la fuente loopback** (`downloads.retryConnection`).
  - **Abrir la ubicación finalizada**: estrictamente desactivado mientras la descarga no haya alcanzado el estado `completed`.
  - **Eliminar la descarga**: botón de eliminación aislado, que muestra un cuadro de diálogo de confirmación detallando la eliminación del registro SQLite y de los ficheros `.part` / `.part.met` temporales, preservando el fichero finalizado si ya está completado.

---

## 2. Ajustes de destino en dos niveles (`SettingsDialog` - Fase 8.1)

Accesible desde la barra lateral izquierda (`DownloadsSidebar`) mediante la entrada dedicada **Settings**.

### 1. Directorio predeterminado global:
- Almacenado de forma persistente en la tabla SQLite `settings` (`default_download_directory`).
- Validado antes de aplicarse mediante una sonda de escritura temporal (`.mule_write_probe_<uuid>`) que garantiza el permiso de escritura.
- Botón **"Reset to Portable Default"**: restablece inmediatamente la ruta hacia `<bundleRoot>/downloads`.
- Botón **"Open Folder"**: abre el directorio de destino en el explorador del sistema operativo.

### 2. Destino por descarga:
- Configurable desde la adición mediante la URL eD2K (`downloads.addEd2kLink` con `targetDirectory`).
- Modificable sobre la marcha mediante `downloads.setDestination`, actualizando la ruta de ensamblaje final sin mover el fichero `.part` en curso de escritura en `data/temp/`.

---

## 3. Historial y Estadísticas Locales (Fase 8.2)

La Fase 8.2 aporta una trazabilidad completa y métricas de actividad agregadas, almacenadas localmente en base de datos SQLite sin ningún componente de telemetría ni fuga de red.

### 1. Persistencia SQLite (`download_history`) en `mule-core`:
- **Tabla `download_history`** con columnas tipadas:
  - `id` (INTEGER PRIMARY KEY AUTOINCREMENT)
  - `download_id` (TEXT)
  - `file_name` (TEXT)
  - `total_bytes` (INTEGER)
  - `transferred_bytes` (INTEGER)
  - `ed2k_hash` (TEXT)
  - `event_type` (`completed`, `failed`, `removed`, `resumed`, `verified_pending_promotion`)
  - `status` (`downloading`, `paused`, `verifying`, `completed`, `failed`)
  - `duration_secs` (INTEGER)
  - `connection_attempts` (INTEGER)
  - `resume_count` (INTEGER)
  - `verification_result` (`verified_ok`, `hash_mismatch`, `unverified`, `skipped`)
  - `error_message` (TEXT nullable)
  - `timestamp_ms` (INTEGER)
- **Indexación de alto rendimiento**: `idx_history_timestamp` e `idx_history_event_type`.
- **Métodos CRUD y Agregaciones**:
  - `insert_history_entry(...)`
  - `query_history(filter)` con paginación (`limit`, `offset`) y rangos temporales (`from_timestamp_ms`, `to_timestamp_ms`).
  - `get_history_stats(session_start_ms)` calculando los volúmenes y conteos en las 4 ventanas temporales.
  - `prune_history(retention_days)` podando los registros obsoletos.
  - `clear_history()` para la purga voluntaria por parte del usuario.

### 2. Servicios y Puntos de acceso RPC en `mule-daemon`:
- Registro automático de las transiciones:
  - `finalize_download` -> `event_type: completed`, `verification_result: verified_ok`.
  - `remove_download` -> `event_type: removed`.
  - `resume_download` -> `event_type: resumed`, incrementando `resume_count`.
  - `update_status(Failed)` -> `event_type: failed`.
  - `update_status(Verifying)` -> `event_type: verified_pending_promotion`.
- Poda automática al arrancar según la retención configurada en la tabla `settings` (`history_retention_days`).
- Endpoints JSON-RPC 2.0:
  - `history.list`: lista filtrable y paginada.
  - `history.stats`: panel de métricas.
  - `history.clear`: purga completa con confirmación.
  - `history.export`: generación de informes CSV o JSON desensibilizados.
  - `settings.setRetentionDays`: configuración de la duración de conservación (7, 30, 90 días o 0 = indefinido).
- Publicación de la capacidad `"history"` en `system.hello`.

### 3. Interfaz Desktop Flutter (`HistoryView`):
- **Navegación lateral**: nueva sección **"History & Statistics"** bajo la rúbrica `AUDIT & ACTIVITY`.
- **Panel estadístico**:
  - *Total Completed*: número total de ficheros y volumen finalizado acumulado.
  - *Activity Windows*: reparto por ventanas (Sesión actual, Últimas 24h, Últimos 7 días).
  - *Resumes & Retries*: número total de reanudaciones y fallos registrados.
  - *Integrity Health*: detección de errores de hash y tasa de integridad (100 % hash verified o notificación).
- **Barra de herramientas y filtros**:
  - Filtrado dinámico por tipo de evento (`All Events`, `Completed`, `Failed`, `Resumed`, `Verifying`, `Removed`).
  - Contador de registros en tiempo real.
  - Botón **"Export Report"** (menú desplegable JSON / CSV).
  - Botón destructivo **"Clear History"** protegido por un diálogo de confirmación explícito.
- **Lista detallada de eventos**:
  - Insignia de color por estado / evento.
  - Nombre del fichero y hash eD2K.
  - Volumen transferido / total.
  - Duración formateada (`formatDuration`) y estadísticas de reanudación/intentos.
  - Insignia de resultado de verificación.
  - Marca temporal legible y botón de inspección modal detallada.
- **Exportación desensibilizada (Sanitized Export)**:
  - Anonimización de todas las rutas locales de usuario (`[DOWNLOAD_DIR]/<filename>`).
  - Cero fugas de tokens o identificadores sensibles.
  - Cuadro de diálogo con previsualización y copia rápida al portapapeles.
- **Configuración de retención en `SettingsDialog`**:
  - Menú desplegable que ofrece 7 días, 30 días (por defecto), 90 días o Indefinido.
  - Poda automática ejecutada al arrancar el daemon.

---

## 4. Categorías Locales y Jerarquía de Destino (Fase 8.3)

La Fase 8.3 dota a eMuleRust de un sistema completo de organización local de las descargas con asignación de prioridades y destinos por defecto.

### 1. Invariante absoluto de seguridad:
- **La eliminación de una categoría NUNCA elimina ningún fichero ni ninguna descarga.**
- Las descargas vinculadas a una categoría eliminada reciben automáticamente `category_id = NULL` y pasan a la vista virtual **«Sin categoría»**.

### 2. Jerarquía de resolución de destino en 4 niveles:
$$\text{targetDirectory (explícito)} > \text{defaultDestination (categoría)} > \text{defaultDownloadDir (global SQLite)} > \text{downloads/ (paquete portátil)}$$

### 3. Persistencia SQLite (`download_categories`):
- Tabla dedicada con `id`, `name`, `color` (`#RRGGBB`), `sort_order`, `default_destination`, `default_priority`, `created_at`, `updated_at`.
- Migración de la tabla `downloads` con la columna `category_id TEXT`.
- Indexación por `sort_order ASC`.

### 4. Endpoints RPC Daemon:
- `categories.list`, `categories.create`, `categories.update`, `categories.delete`, `categories.reorder`.
- `downloads.setCategory(downloadId, categoryId)`.
- Soporte de `categoryId` en `downloads.addEd2kLink`.
- Anuncio de la capacidad `"categories"` en `system.hello`.

### 5. Integración Flutter Desktop:
- Sección **CATEGORIES** en `DownloadsSidebar` con botón de adición `+`, vistas virtuales («All Downloads», «Sin categoría»), círculos de color, contadores y menú contextual (Modificar, Eliminar).
- Cuadro de diálogo `CategoryEditDialog` con paleta de colores, entrada hexadecimal y selector de carpeta.
- Indicador de categoría y menú contextual de asignación en cada fila `DownloadRow`.
- Selector de categoría en `DownloadDetailDialog` y `AddLinkDialog` (con prerrelleno de la carpeta objetivo).

---

## 5. Arquitectura y Frontera de seguridad

```
+-------------------------------------------------------------------------------------------------+
|                                         eMuleRust GUI                                          |
|   +-----------------------+   +----------------------+   +-------------------+   +----------+   |
|   |   SettingsDialog      |   |   HistoryView        |   | DownloadDetail    |   | Category |   |
|   |   - Destino global    |   |   - Stats 4 ventanas |   |   - Part map      |   | Dialog   |   |
|   |   - Retención hist.   |   |   - Filtros eventos  |   |   - Diagnósticos  |   | - Nombre |   |
|   |   - Confinamiento loc.|   |   - Export. informes |   |   - Categoría y   |   | - Color  |   |
|   +-----------------------+   +----------------------+   |     Destino       |   | - Carpeta|   |
|                                                          +-------------------+   +----------+   |
+------------------------------------------------+------------------------------------------------+
                                                 | JSON-RPC WebSocket (127.0.0.1:4660)
+------------------------------------------------v------------------------------------------------+
|                                           mule-daemon                                           |
|   - RPC: categories.list, create, update, delete, reorder, downloads.setCategory                |
|   - RPC: history.list, history.stats, history.clear, history.export                             |
|   - RPC: settings.get, settings.set, settings.setRetentionDays                                  |
|   - Resolución jerárquica de destino: Download > Category > Global SQLite > Bundle              |
|   - Confinamiento estricto 127.0.0.1 y disociación automática al eliminar una categoría         |
+-------------------------+---------------------------------------------------+-------------------+
                          |                                                   |
+-------------------------v-------------------------+   +---------------------v-------------------+
|               data/mule.db (SQLite)               |   |                data/temp/               |
|   - Tabla downloads (category_id incluido)        |   |   (Ficheros .part estrictamente conf.)  |
|   - Tabla download_categories (CRUD y orden)      |   +-----------------------------------------+
|   - Tabla settings (rutas y retención)            |   |                downloads/               |
|   - Tabla download_history (auditoría local)      |   |   (Ficheros finalizados tras validación)|
+---------------------------------------------------+   +-----------------------------------------+
```

- **Cero tráfico WAN y Cero telemetría**: todos los datos de historial, configuración y categorías residen exclusivamente en la base SQLite local `data/mule.db`.
- **Exportaciones saneadas**: rutas de usuario ocultas bajo `[DOWNLOAD_DIR]` y tokens de autenticación totalmente excluidos.

---

## 6. Resultados de las verificaciones y Pruebas

### Pruebas unitarias y de integración Rust (`cargo test --workspace`)
```
test result: ok. 7 passed (mule-cli make_link_tests)
test result: ok. 3 passed (mule-core part_file_and_persistence)
test result: ok. 7 passed (mule-core range_set)
test result: ok. 2 passed (mule-core range_set_properties)
test result: ok. 1 passed (mule-core sqlite_corruption)
test result: ok. 3 passed (mule-core sqlite_categories_tests)
test result: ok. 3 passed (mule-core sqlite_history_tests)
test result: ok. 2 passed (mule-core sqlite_settings_tests)
test result: ok. 4 passed (mule-daemon category_rpc_tests)
test result: ok. 3 passed (mule-daemon download_details_and_destination_tests)
test result: ok. 1 passed (mule-daemon dual_portable_bundles)
test result: ok. 1 passed (mule-daemon history_rpc_tests)
test result: ok. 6 passed (mule-daemon lifecycle_and_locking)
test result: ok. 2 passed (mule-daemon rpc_contract)
test result: ok. 18 passed (mule-protocol ed2k_corpus_tests)
test result: ok. 6 passed (mule-protocol ed2k_isolation_guard)
test result: ok. 45 passed (mule-protocol ed2k_roundtrip_tests)
test result: ok. 13 passed (mule-protocol ed2k_sequencer_tests)
test result: ok. 5 passed (mule-protocol malformed_packets)
test result: ok. 7 passed (mule-protocol packet_codec)
test result: ok. 6 passed (mule-protocol protocol_limits)
Total: Todas las pruebas Rust del espacio de trabajo han pasado con un 100 % de éxito.
```

### Análisis estático Flutter (`dart analyze`)
```
Analyzing lib, test...
No issues found!
```

### Pruebas de widgets y modelos Flutter (`flutter test`)
```
00:12 +70: All tests passed!
- test/ui/category_management_test.dart:
  - displays categories in sidebar with counts and virtual entries: OK
  - filters downloads when a category is selected in sidebar: OK
  - opens CategoryEditDialog and creates a new category: OK
  - assigns download category and updates category count: OK
  - deleting category preserves downloads and moves them to uncategorized: OK
- test/ui/history_view_test.dart:
  - navigates to History & Statistics section and displays dashboard: OK
  - filters history entries by event type: OK
  - shows sanitized export dialog: OK
  - clears history after destructive confirmation: OK
  - settings dialog displays history retention policy options: OK
- test/ui/download_detail_dialog_test.dart:
  - renders all metadata, part map, paths, and action bar: OK
  - copy redacted diagnostic generates safe sanitized output without secrets: OK
  - changing destination prompts dialog and updates destination: OK
  - remove download button opens isolated confirmation dialog: OK
- test/ui/settings_dialog_test.dart:
  - renders global download directory input and confinement notice: OK
  - saving new default directory updates setting and shows success: OK
  - resetting to portable default clears custom directory: OK
- test/ui/downloads_screen_test.dart:
  - renders sidebar with branding and download items: OK
  - filters list by status category: OK
  - search filters list by filename: OK
  - selecting item reveals details panel: OK
  - add eD2K link dialog validates and adds download: OK
  - displays status notification banner: OK
  - displays Loopback Lab Mode badge with tooltip and hides dev tools by default: OK
  - shows DEV badge and allows toggling debug bar when dev tools enabled: OK
  - renders informative empty state with dedicated icon and loopback notice: OK
  - dims sidebar filters with zero count except selected: OK
  - app exit request triggers supervisor stop cleanly: OK
- test/ui/large_list_performance_test.dart:
  - Large list virtualization with 500 items renders and scrolls without lag: OK
```

### Pipeline QA Loopback automatizado (`scripts/test_qa_loopback_e2e.ps1`)
```
==================================================
  Phase 7.3 : Automated QA Loopback E2E Pipeline  
==================================================

[1/7] Creating isolated test sandbox at: C:\Users\Lou\AppData\Local\Temp\eMuleRust-QaE2E-bff77102
 -> portable.flag verified in bundle root.
[2/7] Booting packaged daemon binary in portable mode:
 -> Loopback confinement verified on ws://127.0.0.1:4660/rpc.
 -> system.hello handshake success (Version: 0.1.2, Mode: Portable).
[3/7] Testing Vector 1: Nominal Transfer (sample_1mb):
 -> Confirmed finalized file in default downloads/: sample_1mb.bin (1,048,576 bytes) in 169 ms.
[4/7] Boundary Transfer (9,728,001 bytes) in 451 ms:
 -> Confirmed boundary file in custom downloads/: fixture_part_9728001.bin (9,728,001 bytes) in 451 ms.
[5/7] Crash at ~50% and Resumption (part_9728000):
 -> Forcible kill simulated, .part file retained in temp/ (7,188,480 bytes).
 -> Daemon restarted: recovered from SQLite, reached 100% completion and hash verified!
[6/7] Verifying lab.cleanupTestData security and non-deletion of finalized files:
 -> Cleanup freed 20,504,577 bytes, all finalized files intact and untouched.
[7/7] Graceful shutdown via system.stop:
 -> Daemon exited cleanly with code 0.
==================================================
  PHASE 7.3 AUTOMATED QA LOOPBACK E2E PASSED!     
==================================================
```

---

## 5. Importación experimental de ficheros `.part.met` (Fase 8.4)

La Fase 8.4 aporta la importación defensiva y segura de las descargas parciales o completas procedentes de eMule (`.part.met` y ficheros `.part` asociados), sin ninguna reanudación automática de la red y con confinamiento estricto bajo `data/temp/`.

### Principios y garantías de seguridad:
1. **Cero actividad de red automática**:
   - Toda descarga importada se inicializa obligatoriamente en estado `status: Paused` con `source_count: 0`.
   - No se dispara ningún paquete eD2K, ninguna conexión TCP ni búsqueda de fuentes durante la importación.
2. **Confinamiento y no alteración del origen (Sandbox Staging)**:
   - El fichero origen `.part` del usuario nunca se modifica, renombra ni mueve.
   - Los datos se copian de forma segura bajo `data/temp/<download_id>.part`.
3. **Validación defensiva de formatos binarios (`mule-format`)**:
   - Verificación estricta de los bytes mágicos de cabecera eMule (`0xE0` estándar, `0xE1` fichero grande, `0xE2` formato nuevo).
   - Inversión algorítmica de las etiquetas de huecos (`gaps`) mediante `compute_completed_ranges` para determinar los intervalos de bytes realmente adquiridos.
   - Control de integridad del tamaño: el fichero `.part` físico no puede superar el tamaño total declarado en el `.part.met`.
4. **Verificación criptográfica por parte (MD4)**:
   - Cada parte completa de 9 728 000 bytes se calcula (MD4) y se compara con el array de `part_hashes` del `.part.met`.
   - Si el hash coincide, la parte se registra como verificada (`verified_parts`).
   - Si el hash falla (datos corruptos), el rango correspondiente se invalida automáticamente y se expulsa del `RangeSet` (reclasificado como hueco/gap), incrementando `corrupted_parts`.
5. **Persistencia SQLite y Trazabilidad local**:
   - Registro en las tablas `downloads`, `received_ranges` y `verified_parts`.
   - Creación automática de una entrada en la tabla de auditoría `download_history` con `event_type: "imported"`.
6. **Contrato JSON-RPC y Capacidades**:
   - `downloads.inspectPartMet`: inspección preliminar no destructiva que devuelve los metadatos (nombre, tamaño, hash, número de partes, bytes estimados, existencia del `.part`).
   - `downloads.importPartMet`: conversión, staging, verificación y registro.
   - Capacidad `"import_part_met"` declarada en `system.hello` y `system.status`.
7. **Interfaz de usuario Flutter (`apps/desktop`)**:
   - `ImportPartMetDialog`: entrada de rutas, autodetección del `.part`, selección de categoría, resolución de destino objetivo (4 niveles), previsualización de inspección y botón de importación.
   - Banner de advertencia bilingüe y de seguridad: «The download will be staged into data/temp/ and initialized in Paused state. La importación no contacta con ningún par y la descarga permanecerá en pausa.»
   - Informe de auditoría de importación estructurado mostrado inmediatamente tras el éxito (`import_audit_report_text`) con estadísticas detalladas (partes verificadas, partes rechazadas, estado de confinamiento, estado de red en pausa).
   - Botón de copia del informe al portapapeles (`copy_report_button`) con confirmación visual mediante SnackBar.
   - Botón de finalización «Done» (`finish_import_button`) que cierra limpiamente el cuadro de diálogo.
   - Disparadores accesibles en el estado vacío (`empty_import_part_met_btn`) y en la barra lateral (`btn_import_part_met`).

---

## 6. Industrialización, Empaquetado Windows y Matriz QA (Fase 8.5)

La Fase 8.5 finaliza el ciclo completo de producto entregando archivos portátiles Windows x64 listos para usar, auditados y conformes a los estrictos requisitos de seguridad y reproducibilidad.

### 1. Dos Variantes Windows Distintas

| Característica | Variante Standard (`standard`) | Variante QA Loopback (`qa-loopback`) |
| :--- | :--- | :--- |
| **Archivo entregable** | `eMuleRust-v0.1.2-lab-standard-windows-x64.zip` | `eMuleRust-v0.1.2-lab-qa-loopback-windows-x64.zip` |
| **Carpeta de staging** | `dist/eMuleRust-Standard-Windows` | `dist/eMuleRust-Qa-Loopback-Windows` |
| **Característica `lab-tools`** | **Desactivada** (compilación sin código de prueba) | **Activada** (generación de fixtures y simulación) |
| **Métodos RPC `lab.*`** | Ausentes (-32601 MethodNotFound) | Presentes y funcionales |
| **Capacidades RPC** | `downloads`, `download-events`, `history`, `categories`, `import_part_met` | `downloads`, `download-events`, `history`, `categories`, `import_part_met`, `lab-tools` |
| **Uso** | Producción usuario final | Pruebas de integración y cualificación E2E |

### 2. Integridad Criptográfica y SBOM CycloneDX 1.5

- **Generación SBOM cuatripartita**:
  - `sbom/rust.cdx.json`: 191 dependencias Cargo auditadas.
  - `sbom/flutter.cdx.json`: 33 componentes Flutter / Dart.
  - `sbom/windows-bundle.cdx.json`: 16 artefactos Windows ensamblados.
  - `sbom/sbom.cdx.json`: 240 componentes consolidados en formato CycloneDX 1.5.
- **Auditoría de licencias**: 100 % de conformidad respecto a las reglas de `deny.toml` (`license-audit.json`, `license-audit.md`).
- **Control de integridad 4-way**: verificación de concordancia absoluta entre los ficheros en disco, `version.json`, `SHA256SUMS.txt` y el SBOM.

### 3. Migración de Esquema SQLite y Preservación de Datos (Fase 7 -> Fase 8.4)

- **Prueba automatizada de migración**: `test_phase7_to_phase8_schema_migration` en `sqlite_categories_tests.rs`.
- Una base SQLite de Fase 7 (sin columnas `category_id`, sin tablas de historial, categorías ni settings) se actualiza automáticamente mediante `Database::open`:
  - La columna `category_id` se añade de forma idempotente (`PRAGMA table_info`).
  - Las tablas `download_history`, `download_categories`, `settings` y sus índices se crean de forma transparente.
  - Todas las descargas existentes y sus rangos de datos (`received_ranges`) se conservan al 100 % con `category_id = NULL`.
- **Flujo de actualización portátil verificado**: la extracción de la nueva versión sobre una instalación existente preserva íntegramente el directorio `data/` (`state.db`, `temp/`) y la carpeta `downloads/`.

### 4. Resultados de las Verificaciones y Pruebas

#### Pruebas unitarias y de integración Rust (`cargo test --workspace`)
```
test result: ok. 100% passed across all crates:
- mule-cli: 7 passed
- mule-core: 21 passed (incluyendo migraciones SQLite, categorías, historial, settings, range_set, corrupción)
- mule-daemon: 20 passed (incluyendo RPC categorías, detalles y destino, historial, importación .part.met, locking)
- mule-format: 8 passed (vectores .part.met, etiquetas eMule, gaps)
- mule-protocol: 95 passed (ed2k_corpus, isolation guard, roundtrip, sequencer, malformed, codec, limits)
Total: 151 pruebas Rust superadas con éxito.
```

#### Análisis estático y Pruebas Flutter Desktop (`apps/desktop`)
```
flutter analyze : No issues found! (0 warnings, 0 errors, 0 lints)
flutter test    : 74 pruebas superadas con un 100 % de éxito.
- import_part_met_dialog_test.dart : verificación de entrada, inspección, creación confinada en estado Paused, visualización del informe de auditoría y copia al portapapeles.
- category_management_test.dart    : ciclo de vida completo de las categorías, disociación no destructiva.
- download_detail_dialog_test.dart : metadatos eD2K, part map, diagnósticos desensibilizados, cambio de destino.
- history_view_test.dart           : estadísticas 4 ventanas, filtros de eventos, exportaciones saneadas, retención.
- settings_dialog_test.dart        : sonda de escritura, directorio predeterminado global, restablecimiento portátil.
- large_list_performance_test.dart : virtualización fluida de 500 descargas simultáneas.
```

#### Smoke Test Post-Empaquetado (`scripts/test_packaged_bundle.ps1`)
```
==================================================
  eMuleRust Post-Packaging E2E Smoke Test        
  Flavors: standard & qa-loopback
==================================================
 -> Archivo SHA-256 verificado al 100 % frente a .zip.sha256.
 -> Extracción estéril en sandbox aislado bajo AppData/Local/Temp.
 -> 22/22 hashes de ficheros validados con exactitud mediante SHA256SUMS.txt.
 -> Proceso eMuleRust.exe iniciado e inicializado limpiamente sin cuelgues.
 -> Confinamiento loopback (127.0.0.1:4660) y token de autenticación de 256 bits verificados.
 -> Contratos JSON-RPC validados (system.hello, system.status, system.stop).
 -> Ausencia absoluta de los métodos lab.* en la variante standard (-32601 MethodNotFound).
 -> Presencia de los métodos lab.* verificada en la variante qa-loopback.
 -> Detención elegante y limpieza del fichero session.json confirmadas.
==================================================
  SMOKE TEST PASSED! Package is verified & sound. 
==================================================
```

#### Pipeline E2E QA Loopback (`scripts/test_qa_loopback_e2e.ps1`)
```
==================================================
  Phase 7.3 : Automated QA Loopback E2E Pipeline  
==================================================
[1/7] Extracción del archivo QA Loopback y validación de portable.flag.
[2/7] Arranque del daemon en modo portátil con confinamiento loopback estricto (127.0.0.1).
[3/7] Transferencia nominal (sample_1mb): completada y verificada en 181 ms en default downloads/.
[4/7] Transferencia de frontera (9 728 001 bytes): completada y verificada en 439 ms en custom downloads/.
[5/7] Simulación de fallo al ~50 %: kill forzado del daemon, persistencia del .part bajo data/temp/ y del estado SQLite, reinicio y reanudación hasta completarse al 100 % con verificación de integridad.
[6/7] Seguridad lab.cleanupTestData: purga de los fixtures temporales, ningún fichero completado es eliminado.
[7/7] Detención elegante mediante system.stop (código de salida 0).
==================================================
  PHASE 7.3 AUTOMATED QA LOOPBACK E2E PASSED!     
==================================================
```

#### Matriz de Entorno y Supervisión RC (`scripts/test_rc_matrix.ps1`)
```
==================================================
  eMuleRust RC Environment & Supervision Matrix  
==================================================
[1/6] Directorios Unicode, caracteres no ASCII y espacios : PASS
[2/6] Conflicto de puerto TCP 4660 y cambio dinámico a puerto efímero : PASS
[3/6] Bloqueo de exclusividad y prevención de colisión multiinstancia : PASS
[4/6] Recuperación robusta tras fallo abrupto con bloqueo obsoleto : PASS
[5/6] Ejecución autónoma del paquete reubicado fuera del repositorio Git : PASS
[6/6] Gestión limpia de errores en directorio de solo lectura : PASS
[7/7] Autorresolución del modo portátil mediante portable.flag sin argumento CLI : PASS
==================================================
  ALL 7 ENVIRONMENT & SUPERVISION MATRIX TESTS OK! 
==================================================
```

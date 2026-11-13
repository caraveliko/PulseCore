# Walkthrough — Fase 7.2: Recorrido de prueba local guiado y Ergonomía RC

La **Fase 7.2** dota a eMuleRust de un recorrido de prueba local loopback de extremo a extremo, utilizable directamente desde la interfaz gráfica **sin tocar jamás una terminal**, garantizando al mismo tiempo un aislamiento estricto en `127.0.0.1` y protegiendo las instalaciones estándar `%LOCALAPPDATA%`.

---

## 1. Detección acotada de la raíz del paquete (`portable.flag`)

Para eliminar cualquier riesgo de recorrido incontrolado del árbol de directorios (antifuga de configuración), la detección de la raíz del paquete está estrictamente acotada:
- Si `parent(executable)` se llama `bin` (sin distinguir mayúsculas y minúsculas): `bundleRoot = parent(parent(executable))`.
- En caso contrario: `bundleRoot = parent(executable)`.
- Fichero buscado: `bundleRoot / portable.flag`.
- **Cero recorrido recursivo ascendente** más allá de esta regla.

### Verificación
- **Rust**: `test_workdir_non_bin_subfolder_does_not_traverse_parent_for_portable_flag` en [crates/mule-daemon/tests/workdir_tests.rs](file:///d:/Projet/eMuleRust/crates/mule-daemon/tests/workdir_tests.rs).
- **Dart**: `WorkDirectoryResolver` en [apps/desktop/test/services/work_directory_resolver_test.dart](file:///d:/Projet/eMuleRust/apps/desktop/test/services/work_directory_resolver_test.dart).

---

## 2. Nuevos métodos JSON-RPC de laboratorio (`mule-daemon`)

El daemon expone ahora una suite completa de operaciones de laboratorio dedicadas al QA local:

| Método RPC | Descripción | Seguridad y Salvaguardas |
|---|---|---|
| `lab.status` | Devuelve el modo de workdir, la ruta, el indicador portátil, el par loopback activo y la lista de fixtures. | Solo lectura. |
| `lab.createFixture` | Genera un fichero determinista (`part_9728000`, `sample_1mb`, `text`) en `<workdir>/fixtures/`, calcula su hash eD2K raíz mediante `mule-format` y genera el enlace eD2K canónico. | Confinado a la carpeta fixtures. |
| `lab.startLoopbackPeer` | Inicia una instancia de `Ed2kTestPeer` en `127.0.0.1:0` compartiendo todos los fixtures generados. | Estrictamente enlazado a `127.0.0.1`. |
| `lab.stopLoopbackPeer` | Detiene el par de prueba loopback y libera el socket TCP. | Limpieza limpia. |
| `lab.cleanupTestData` | Elimina los fixtures locales y purga los ficheros `.part` incompletos en `<workdir>/temp` y `<workdir>/part`. | **Prohibido y rechazado** en modo `InstalledDefault` (`%LOCALAPPDATA%`). |

### Verificación Rust
- Pruebas de integración completas en [crates/mule-daemon/tests/lab_rpc_tests.rs](file:///d:/Projet/eMuleRust/crates/mule-daemon/tests/lab_rpc_tests.rs):
  - `test_lab_rpc_lifecycle_in_portable_mode` (creación de fixtures, enlace canónico, arranque/parada del par, limpieza).
  - `test_lab_cleanup_refused_in_installed_default_mode` (verificación de la protección antieliminación en modo instalado).
- **221 pruebas Rust validadas**, Clippy: 0 advertencias.

---

## 3. Interfaz de Usuario y Ergonomía RC

### Diálogo de Diagnóstico (`DiagnosticDialog`)
Accesible en un clic sobre la insignia **«Loopback Lab Mode»** en la cabecera o sobre la tarjeta de estado del daemon en la barra lateral.

- **Confinamiento de Red**: Tarjeta visual que recuerda la restricción estricta a `127.0.0.1` (cero tráfico WAN).
- **Workdir y Modo**: Insignia de color (`Portable`, `Explicit`, `Environment`, `InstalledDefault`) y ruta exacta de los datos.
- **Proceso y RPC**: PID del proceso gestionado, endpoint WebSocket (`ws://127.0.0.1:4660/rpc`), versión del protocolo, identificador de instancia y de sesión.
- **Copia rápida**: Botón «Copy Diagnostics» para copiar todos los diagnósticos al portapapeles para los informes de fallos.
- **Acceso directo**: Botón «Open Lab Guide» que lleva directamente al recorrido guiado.

### Recorrido de prueba local guiado (`LabGuideDialog`)
Un diálogo paso a paso en 5 etapas para ejecutar un ciclo completo de prueba sin terminal:
1. **Paso 1 — Generar un fichero de prueba**: Elección entre una muestra de 1 MB (`sample_1mb`), un chunk completo de 9,28 MB (`part_9728000`) o un texto ligero (4 KB).
2. **Paso 2 — Enlace eD2K y Adición**: Visualización del enlace eD2K canónico, botón «Copiar» y botón «Add to Downloads» que inyecta al instante la tarea en la cola de la aplicación.
3. **Paso 3 — Par de prueba loopback**: Botón iniciar/detener el par loopback en `127.0.0.1` con indicador de estado dinámico y número de ficheros compartidos.
4. **Paso 4 — Conectar la descarga**: Selección de la tarea objetivo y conexión al par `127.0.0.1` para disparar las transferencias de bloques y la verificación de integridad.
5. **Paso 5 — Limpieza de datos de prueba**: Purga segura de los ficheros de fixtures y de las descargas temporales (con diálogo de confirmación, reservado al modo portátil).

### Gating y reactividad visual
- **Protección del modo instalado**: Las herramientas de laboratorio quedan ocultas cuando la aplicación se ejecuta en modo estándar `%LOCALAPPDATA%`.
- **Disposición adaptable (`Wrap`)**: Todos los diálogos se adaptan sin ningún desbordamiento de renderizado (`RenderFlex overflow`), incluso en resoluciones compactas (800×600).
- **Acceso desde el estado vacío**: Cuando no hay ninguna descarga presente, se ofrece a los testers un botón dedicado «Guided Local Test» junto a «Add eD2K Link».

---

## 5. Corrección crítica: Persistencia de la eliminación de una descarga

### Problema identificado
En modo Standard (`%LOCALAPPDATA%`) o en modo Portable, eliminar una descarga desde la interfaz la hacía desaparecer visualmente de la pantalla actual. En cambio, al cerrar y volver a abrir eMuleRust, la descarga reaparecía de inmediato en la lista.

### Causa raíz
1. En `crates/mule-daemon/src/services/downloads.rs`, `remove_download` retiraba la descarga de la memoria RAM (`self.active.remove(id)`), pero **no ejecutaba ninguna consulta SQL de eliminación en la base SQLite local** (`mule.db`).
2. `Database` en `crates/mule-core/src/persistence/sqlite.rs` ni siquiera disponía de una función `delete_download`. Las tablas `downloads`, `received_ranges` y `verified_parts` conservaban indefinidamente el registro.
3. Al reiniciar el daemon, `db.get_all_downloads()` recargaba la entrada desde la base, y Flutter la recibía de nuevo durante `downloads.list`.
4. Además, los ficheros temporales `.part` quedaban huérfanos en el disco si el usuario no podía elegir eliminarlos.

### Correcciones implementadas
1. **Eliminación atómica en base de datos (`mule-core`)**:
   - Adición de `delete_download(&mut self, download_id: &Uuid) -> Result<bool, CoreError>` en [`sqlite.rs`](file:///d:/Projet/eMuleRust/crates/mule-core/src/persistence/sqlite.rs).
   - Transacción SQLite atómica que elimina las filas asociadas en `received_ranges`, `verified_parts` y después `downloads`.
   - Prueba unitaria dedicada: `test_database_delete_download_persists_across_reopen` en [`part_file_and_persistence.rs`](file:///d:/Projet/eMuleRust/crates/mule-core/tests/part_file_and_persistence.rs).
2. **Gestión del descriptor y de los ficheros temporales (`mule-daemon`)**:
   - Cierre explícito del descriptor de fichero de Windows (`active.part_file.take()`) antes de cualquier eliminación en disco, previniendo el error de bloqueo de Windows `ERROR_SHARING_VIOLATION` (código 32).
   - Eliminación sistemática en base de datos mediante `self.db.delete_download(id)`.
   - Eliminación del fichero `.part` físico en `temp/` si `delete_files == true`.
   - Prueba de contrato RPC: `test_downloads_remove_persistence_across_daemon_restart` en [`rpc_contract.rs`](file:///d:/Projet/eMuleRust/crates/mule-daemon/tests/rpc_contract.rs).
3. **Diálogo de confirmación enriquecido (`apps/desktop`)**:
   - Actualización de [`download_row.dart`](file:///d:/Projet/eMuleRust/apps/desktop/lib/ui/features/downloads/widgets/download_row.dart) con una casilla de verificación «Eliminar también los ficheros temporales/parciales del disco» (marcada por defecto).
   - Transmisión del booleano `deleteFiles` al daemon.

---

## 6. Resultados de las validaciones automatizadas

```
==================================================
  Suite de pruebas Rust (cargo test --workspace --all-features)
==================================================
-> 222 pruebas superadas, 0 fallos, 1 ignorada (Tier 3 live ref)
-> Clippy (--workspace --all-targets --all-features) : 0 advertencias

==================================================
  Suite de pruebas Flutter (flutter test & analyze)
==================================================
-> 49 pruebas de widgets e integración superadas, 0 fallos
-> Flutter analyze : No issues found!

==================================================
  Pipeline de empaquetado (scripts\build_windows_release.ps1)
==================================================
-> Paquete estándar autónomo : dist\eMuleRust-Standard-Windows (eMuleRust-v0.1.1-lab-standard-windows-x64.zip)
-> Paquete QA Loopback : dist\eMuleRust-Qa-Loopback-Windows (eMuleRust-v0.1.1-lab-qa-loopback-windows-x64.zip)
-> SBOM CycloneDX 1.5 consolidado : 240 componentes auditados
-> Hashes SHA-256 verificados y coherentes al 100 %

==================================================
  Smoke test post-empaquetado (scripts\test_packaged_bundle.ps1)
==================================================
-> Descompresión en sandbox estéril (%TEMP%)
-> Verificación de integridad SHA256SUMS.txt : 22/22 ficheros conformes
-> Arranque daemon, verificación sesión loopback y JSON-RPC : ÉXITO

==================================================
  Matriz de entorno y supervisión (scripts\test_rc_matrix.ps1)
==================================================
[1/7] Rutas con espacios y caracteres Unicode : OK
[2/7] Conflicto de puerto TCP 4660 y respaldo efímero : OK
[3/7] Bloqueo de exclusividad y anticolisión : OK
[4/7] Recuperación tras fallo abrupto y bloqueo huérfano : OK
[5/7] Paquete movido fuera del repositorio : OK
[6/7] Directorio no escribible : OK
[7/7] Modo portátil mediante portable.flag : OK
```

---

## 5. Endurecimiento de Seguridad y Gating Condicional (Validación Final)

En respuesta directa a los requisitos de endurecimiento de producción:

### A. Separación estricta de las características Cargo (`default = []`) y Capacidades depuradas
- `crates/mule-daemon/Cargo.toml` y `crates/mule-protocol/Cargo.toml` definen ahora `default = []`.
- Las rutas `lab.*` y el servidor loopback `Ed2kTestPeer` solo se compilan **si** se solicita expresamente la característica `lab-tools` (`cargo build --features lab-tools`).
- `debug.injectRange` está condicionado por `#[cfg(any(feature = "debug-rpc", feature = "lab-tools"))]`.
- La capacidad `local-simulation` se ha retirado por completo de la compilación estándar:
  - **Compilación estándar de producción**: `capabilities = ["downloads", "download-events"]`
  - **Compilación QA portátil (-LabMode)**: `capabilities = ["downloads", "download-events", "lab-tools"]`

### B. Taxonomía formal de los códigos de error JSON-RPC
De acuerdo con los requisitos de estabilidad del contrato de API:
| Código | Nombre canónico | Disparador / Descripción |
|---|---|---|
| `-32700` | `ParseError` | Trama JSON no válida. |
| `-32600` | `InvalidRequest` | Petición JSON-RPC no conforme. |
| `-32601` | `MethodNotFound` | Método desconocido o ruta de laboratorio llamada en una compilación de producción. |
| `-32602` | `InvalidParams` | Parámetros faltantes o mal tipados. |
| `-32603` | `InternalError` | Error interno del daemon. |
| `-32000` | `AuthenticationFailed` | Token de autenticación no válido o faltante. |
| `-32001` | `ProtocolMismatch` | Incompatibilidad de versión de protocolo en línea. |
| `-32002` | `ForbiddenInInstalledMode` | Operación prohibida porque el workdir está en modo estándar instalado `%LOCALAPPDATA%`. |
| `-32003` | `UnsafeCleanupTarget` | Ruta fuera de lista blanca, recorrido de directorio (`..`) o enlace simbólico / punto de unión detectado. |
| `-32004` | `FixtureDownloadMismatch` | Incoherencia entre el tamaño/hash de la descarga y el fixture. |
| `-32005` | `LoopbackPeerUnavailable` | Par loopback inactivo o inalcanzable en `127.0.0.1`. |
| `-32010` | `BusinessError` | Error de negocio genérico. |

### C. Doble condición de visualización en el lado Flutter (Dual-Gating)
- La interfaz Flutter solo muestra las herramientas de laboratorio si se cumplen dos condiciones simultáneas:
  1. `workdirMode == 'Portable'`
  2. `daemonInfo.capabilities.contains('lab-tools')`

### D. Protección absoluta contra la fuga de secretos (Diagnósticos)
- **Exportación Compartible Segura («Copy Shareable (Sanitized)»)**:
  - Rutas ocultas: `<portable-data>` o `<installed-data>` (no se filtra ningún nombre de usuario de Windows).
  - Identificadores truncados: formato `4b7e…8a31`.
  - Endpoint anonimizado: `ws://loopback:4660/rpc`.
  - Token de autenticación: estrictamente excluido (cero tokens).
- **Exportación Local Desarrollador («Copy Full (Local Dev)»)**: conserva las rutas completas para la depuración local en entorno controlado.
  - **Garantía de no divulgación verificada mediante prueba automatizada**: una prueba de widget unitaria dedicada en [`test/ui/diagnostic_dialog_test.dart`](file:///d:/Projet/eMuleRust/apps/desktop/test/ui/diagnostic_dialog_test.dart) valida formalmente que ni `auth.token`, ni ningún token secreto o cabecera de autenticación aparezca en el texto copiado.

### E. Precondiciones y cadena de progreso en el Paso 4 (`LabGuideDialog`)
- Verificación de la coherencia de tamaño entre la descarga seleccionada y el fixture generado (advertencia visual inmediata en caso de divergencia).
- Rechazo de conexión si la descarga ya está en `Completed` o si el par loopback está inactivo.
- Banner de progreso dinámico: estado de la tarea, caudal, bytes transferidos e insignia de finalización.

### F. Limpieza endurecida (`lab.cleanupTestData`)
- Bloqueo estricto en modo `InstalledDefault` (código de error JSON-RPC de negocio `-32002`).
- Rechazo inmediato de cualquier intento de escape fuera del workdir canónico o de recorrido de enlace simbólico (código de error `-32003`).
- Lista blanca estricta: solo se limpian `fixtures/`, `temp/` y `part/`; las carpetas `downloads/` e `incoming/` nunca se alteran.
- Devolución estructurada de un informe detallado (`cleanedFolders`, `deletedFilesCount`, `freedBytes`) mostrado de inmediato al tester.

---

## 6. Comparativa de los Dos Entregables Desacoplados (v0.1.1-lab)

| Propiedad / Criterio | Compilación Estándar de Producción | Compilación QA Loopback (`-Flavor qa-loopback`) |
|---|---|---|
| **Comando de generación** | `powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\build_windows_release.ps1 -Flavor standard` | `powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\build_windows_release.ps1 -Flavor qa-loopback` |
| **Carpeta de ensamblaje** | `dist\eMuleRust-Standard-Windows` | `dist\eMuleRust-Qa-Loopback-Windows` |
| **Árbol Git requerido** | 100 % limpio (`git status --porcelain` vacío) | 100 % limpio (`git status --porcelain` vacío) |
| **Manifiesto `version.json`** | `"distributionFlavor": "standard"`, `"labToolsEnabled": false` | `"distributionFlavor": "qa-loopback"`, `"labToolsEnabled": true` |
| **Capacidades anunciadas** | `["downloads", "download-events"]` | `["downloads", "download-events", "lab-tools"]` |
| **Rutas RPC `lab.*`** | Ausentes (devuelve `-32601 Method not found`) | Activas y registradas (`lab.status`, `lab.createFixture`, etc.) |
| **Par loopback `Ed2kTestPeer`** | No compilado | Presente, escuchando únicamente en `127.0.0.1:0` |
| **Simulación `debug.injectRange`** | No compilado | Presente para pruebas de bloques locales |
| **UI Flutter (Modo Portátil)** | Herramientas de laboratorio ocultas (`isLabModeAvailable == false`) | Herramientas de laboratorio y guía interactiva activas |
| **Verificación smoke test** | **Éxito** (22/22 hashes conformes, sesión 127.0.0.1, -32601 verificado) | **Éxito** (22/22 hashes conformes, sesión 127.0.0.1, lab.status OK) |
| **Matriz de robustez** | 7/7 pruebas superadas (`scripts\test_rc_matrix.ps1`) | 7/7 pruebas superadas (`scripts\test_rc_matrix.ps1`) |
| **Archivo entregable** | `eMuleRust-v0.1.1-lab-standard-windows-x64.zip` | `eMuleRust-v0.1.1-lab-qa-loopback-windows-x64.zip` |
| **Huella SHA-256** | `2f258ef49383aa23e7ce7af350544466369ef57f69c5b3add7bfdd7d8ca33332` | `02e58f05f808d4a2726a5476f0fdf94f3c54e9fa09b38790cde8156ac05bbde4` |
| **Etiqueta Git Sellada** | `v0.1.1-lab` (`6166c71`) | `v0.1.1-lab` (`6166c71`) |

---

## 7. Resolución del Diagnóstico, Propagación de DaemonInfo y Ergonomía de Descarga

### A. Causa raíz diagnosticada y corregida para los valores «N/A / Unknown»
1. **Ruptura de propagación `DaemonInfo`**: `DownloadsViewModel` solo inicializaba su campo interno `_daemonInfo` en el método `initialize()`. Sin embargo, durante el lanzamiento nominal de la aplicación supervisada, `supervisor != null`, por lo que `initialize()` nunca se invocaba. `DaemonSupervisor` se conectaba mediante `RpcDaemonClient.connect()`, que sí almacenaba los datos en `client.daemonInfo`, pero `DownloadsViewModel` no leía `client.daemonInfo` durante los eventos `SupervisorState.connected` o `instanceChanged`.
   - **Corrección**:
     - Adición del getter `DaemonInfo? get daemonInfo` en la interfaz abstracta [`MuleDaemonClient`](file:///d:/Projet/eMuleRust/apps/desktop/lib/domain/services/mule_daemon_client.dart).
     - Escucha activa en `DownloadsViewModel` para sincronizar `_daemonInfo = client.daemonInfo` en cada conexión y refresco.
     - Conservación de la instancia activa de sesión [`DaemonSessionFile`](file:///d:/Projet/eMuleRust/apps/desktop/lib/services/daemon_supervisor.dart) en el supervisor para proporcionar los PID, endpoints y versión del protocolo al instante.

2. **Preservación del `WorkDirMode` entre Flutter y el daemon hijo**:
   - Cuando Flutter lanzaba `mule-daemon.exe --workdir <dataDir>`, el daemon interpretaba la presencia de `--workdir` como `WorkDirMode::ExplicitCli`.
   - **Corrección**:
     - Adición del argumento `--mode / --workdir-mode` en el parseo CLI de [`mule-daemon`](file:///d:/Projet/eMuleRust/crates/mule-daemon/src/main.rs).
     - Detección automática en `mule-daemon`: si se proporciona `--workdir` sin `--mode`, el daemon comprueba si el directorio padre directo contiene `portable.flag` y promueve automáticamente el modo a `WorkDirMode::Portable`.
     - `DaemonSupervisor` pasa explícitamente `--mode Portable` al arrancar.
     - `isLabModeAvailable` acepta tanto la respuesta del daemon como la detección local del supervisor.

### B. Diálogo de diagnóstico enriquecido (`DiagnosticDialog`)
- **Etiquetas de capacidades (`Capabilities`)**: visualización elegante de cada capacidad en forma de chip, con destacado en teal para `lab-tools`.
- **Información dinámica completa**: modo portátil, ruta resuelta, PID real, endpoint WebSocket real, versión del daemon, identificador de instancia y de sesión.
- **Insignia de cabecera adaptable**:
  - Si el modo Lab está activo: **«Loopback Lab Mode»** en teal con icono de ciencia, más el botón de acceso directo a la guía de pruebas (`Icons.biotech`).
  - Si el modo Lab está ausente (compilación estándar o modo instalado): **«Diagnostics»** en blueGrey con icono de analítica.

### C. Abreviatura ergonómica de los identificadores de descarga
- En [`DownloadRow`](file:///d:/Projet/eMuleRust/apps/desktop/lib/ui/features/downloads/widgets/download_row.dart), el identificador UUID en bruto de 36 caracteres se abrevia ahora al formato `ID: 4dab69c4…` (8 caracteres + puntos suspensivos).
- **Tooltip nativo**: al pasar el ratón se muestra el UUID completo (`Full ID: 4dab69c4-aa9e-46fa-81e0-0ea0b9974f07`).
- **Botón de copia**: icono discreto que permite copiar el UUID completo al portapapeles en un clic con confirmación SnackBar.

### D. Validación global
- **Pruebas Flutter**: 50 pruebas superadas con éxito (`flutter test`).
- **Análisis estático**: 0 advertencias, 0 errores (`flutter analyze`).
- **Pruebas Rust**: 223 pruebas superadas con éxito (`cargo test --features "lab-tools,ed2k-loopback"`).
- **Smoke tests post-empaquetado**: 100 % validados en `standard` y `qa-loopback`.

---

## 8. Fase 7.3: Automatización QA Loopback E2E y Mejoras de Privacidad

### A. Privacidad UI y Censura de la Ruta de Datos (`DiagnosticDialog`)
- **Censura automática**: el método `_formatPathForDisplay` trunca las rutas de perfiles de usuario de Windows (`C:\Users\…\Desktop\…\data`) y Unix (`/home/…/...`), evitando así la divulgación involuntaria del nombre de usuario en capturas de pantalla de fallos o de QA.
- **Información emergente (`Tooltip`)**: al pasar el ratón sobre la línea `Path` se revela la ruta en bruto completa.
- **Botón de copia en línea**: un botón compacto `[Copy]` a la derecha de la línea copia al instante la ruta en bruto real al portapapeles con notificación visual (`Workdir path copied to clipboard`).

### B. Nuevo vector de frontera: `pattern_9728001` (9,28 MB + 1 byte)
- **Rust (`mule-daemon`)**: adición de `"pattern_9728001"` y `"part_9728001"` en [`handle_lab_create_fixture`](file:///d:/Projet/eMuleRust/crates/mule-daemon/src/rpc/methods.rs). Genera un fixture de 9 728 001 bytes (exactamente 1 parte completa + 1 parte residual de un byte) con patrón repetitivo `(i % 256) as u8`.
- **UI (`LabGuideDialog`)**: chip dedicado `9.28 MB + 1 Byte (pattern_9728001)` en el Paso 1 de la guía.
- **Prueba Rust**: `test_lab_create_fixture_pattern_9728001` en [`lab_rpc_tests.rs`](file:///d:/Projet/eMuleRust/crates/mule-daemon/tests/lab_rpc_tests.rs).

### C. Pipeline de automatización QA de extremo a extremo (`scripts/test_qa_loopback_e2e.ps1`)
Un script automatizado completo que valida sin ninguna intervención humana el paquete empaquetado real:
1. **Extracción en sandbox estéril**: descomprime el archivo ZIP QA en un directorio temporal aislado.
2. **Arranque y confinamiento**: inicia `mule-daemon.exe` en modo portátil, valida la exclusividad del socket `127.0.0.1` y el anuncio de `lab-tools`.
3. **Vector 1 — Transferencia nominal (`sample_1mb`)**:
   - Creación del fixture, adición de la descarga, arranque del par loopback.
   - Conexión de la descarga, transferencia y transición hacia `Completed` en ~170 ms.
   - Control del fichero finalizado en `<workdir>/downloads/` (tamaño e integridad eD2K).
4. **Vector 2 — Frontera de parte (`pattern_9728001`)**:
   - Creación y transferencia del fixture de 9 728 001 bytes.
   - Verificación de la finalización y del fichero finalizado.
5. **Vector 3 — Resiliencia, Fallo abrupto y Reanudación (`part_9728000`)**:
   - Arranque de la transferencia de un chunk de 9,28 MB.
   - Supervisión en tiempo real hasta ~50 % (~4,6 MB transferidos).
   - Detención forzada abrupta del daemon (`Stop-Process -Force`).
   - Verificación de la persistencia del fichero `.part` y del estado SQLite `mule.db`.
   - Reinicio del daemon empaquetado sobre la misma carpeta de datos: confirmación de que el progreso se conserva.
   - Reconexión al par loopback: reanudación de la transferencia hasta `Completed` (100 %).
   - Validación del fichero finalizado.
6. **Seguridad de la limpieza (`lab.cleanupTestData`)**:
   - Ejecución de `lab.cleanupTestData`.
   - Purga confirmada de los fixtures y de los ficheros `.part` temporales.
   - **Control crítico antipérdida de datos**: confirmación formal de que los 3 ficheros finalizados en `downloads/` **están intactos y nunca fueron eliminados**.
7. **Detención limpia e integridad**:
   - Detención controlada mediante `system.stop` y confirmación de cero contaminación del repositorio Git.

```
==================================================
  PHASE 7.3 AUTOMATED QA LOOPBACK E2E PASSED!     
==================================================
```

---

## 9. Separación estricta del estado interno (`data/`) y de las descargas finales (`downloads/`)

Para garantizar una separación clara entre el estado de ejecución interno del daemon y los ficheros finales visibles y manipulados por el usuario, la arquitectura separa formalmente `workdir` (`data/`) y `defaultDownloadDirectory` (`downloads/`) en 4 niveles de prioridad:

### A. Regla de resolución de directorios (4 Niveles)

| Modo | Carpeta interna `workdir` | Directorio predeterminado de descargas finales |
|---|---|---|
| **1. CLI explícito** | Argumento `--workdir <dir>` | Argumento `--downloads-dir <dir>` o `<workdir>/downloads` si se omite |
| **2. Variables de entorno** | `$EMULE_DATA_DIR` o `$MULE_DATA_DIR` | `$EMULE_DOWNLOADS_DIR` o `<workdir>/downloads` si se omite |
| **3. Modo Portátil (`portable.flag`)** | `<bundleRoot>/data` | `<bundleRoot>/downloads` (junto al paquete pero distinto de los binarios y del estado interno) |
| **4. Modo Instalado Estándar** | `%LOCALAPPDATA%\eMuleRust` (Unix: `~/.local/share/eMuleRust`) | `%USERPROFILE%\Downloads\eMuleRust` (Unix: `~/Downloads/eMuleRust`) |

### B. Modificaciones Backend Rust (`mule-daemon`)
- **[`workdir.rs`](file:///d:/Projet/eMuleRust/crates/mule-daemon/src/workdir.rs)**:
  - Introducción de la estructura `ResolvedDirectories { workdir: ResolvedWorkDir, default_download_dir: PathBuf }`.
  - Implementación de `WorkDirectoryResolver::resolve` y `resolve_with_context` aplicando los 4 niveles.
  - Pruebas unitarias completas en `tests/workdir_tests.rs` (8 pruebas superadas).
- **[`config.rs`](file:///d:/Projet/eMuleRust/crates/mule-daemon/src/config.rs) y [`main.rs`](file:///d:/Projet/eMuleRust/crates/mule-daemon/src/main.rs)**:
  - Soporte del argumento CLI `--downloads-dir <path>`.
  - Integración de `default_download_dir` en `DaemonConfig`.
- **Métodos RPC ([`methods.rs`](file:///d:/Projet/eMuleRust/crates/mule-daemon/src/rpc/methods.rs))**:
  - `system.hello` y `system.status` anuncian ahora `"defaultDownloadDir"`.
  - `downloads.addEd2kLink`: si `targetDirectory` se omite, está vacío o es igual a `"."`, el destino se resuelve automáticamente hacia `state.config.default_download_dir`.
  - `lab.cleanupTestData`: nunca toca el directorio `downloads/`.

### C. Modificaciones Interfaz Flutter (`apps/desktop`)
- **[`work_directory_resolver.dart`](file:///d:/Projet/eMuleRust/apps/desktop/lib/services/work_directory_resolver.dart)**:
  - Alineación exacta de la lógica de resolución Dart con la especificación Rust (4 niveles de prioridad).
- **[`daemon_info.dart`](file:///d:/Projet/eMuleRust/apps/desktop/lib/domain/models/daemon_info.dart) y DTOs**:
  - Recepción y parseo del campo `defaultDownloadDir`.
- **Diálogo de adición de enlace ([`AddLinkDialog`](file:///d:/Projet/eMuleRust/apps/desktop/lib/ui/features/downloads/widgets/add_link_dialog.dart))**:
  - Prerrelleno del campo carpeta de destino con el directorio de descarga resuelto por defecto.
  - Adición del botón nativo `[ Browse... ]` mediante diálogo del sistema PowerShell `FolderBrowserDialog` para seleccionar cómodamente un directorio personalizado.
- **Diálogo de diagnóstico ([`DiagnosticDialog`](file:///d:/Projet/eMuleRust/apps/desktop/lib/ui/features/downloads/widgets/diagnostic_dialog.dart))**:
  - Visualización disociada de **«Internal Data»** (`.../data`) y **«Downloads»** (`.../downloads`).
  - Botones de copia individuales en línea con información emergente explícita.
- **Guía de laboratorio ([`LabGuideDialog`](file:///d:/Projet/eMuleRust/apps/desktop/lib/ui/features/downloads/widgets/lab_guide_dialog.dart))**:
  - Tarjeta resumen «Storage Overview» que distingue el estado interno (`data/`) y los ficheros completados (`downloads/`), con botón directo «Open Folder».
  - Visualización precisa del destino del fichero en el Paso 4.
  - Mención explícita en el Paso 5 de que la limpieza no elimina los ficheros descargados en `downloads/`.

### D. Validación de los Scripts y Pipelines
- `scripts/test_packaged_bundle.ps1`: verifica la presencia y validez de `defaultDownloadDir` en `system.hello` y `system.status`.
- `scripts/test_qa_loopback_e2e.ps1`:
  - Valida el respaldo automático hacia `defaultDownloadDir` (Vector 1 y 3).
  - Valida la especificación de una carpeta personalizada `targetDirectory` (Vector 2).
  - Controla formalmente que `lab.cleanupTestData` no elimine ni los ficheros de `default downloads/` ni los de `custom downloads/`.

```
==================================================
  RESULTADOS DE VALIDACIÓN COMPLETA
==================================================
-> Rust : 223 pruebas de integración superadas (24/24 pruebas de integración + pruebas unitarias)
-> Flutter : 69 pruebas superadas (53 widgets/integración + 16 pruebas unitarias)
-> Flutter analyze : 0 incidencias
-> Smoke test QA Loopback : 100 % superado
-> Smoke test Standard : 100 % superado
-> QA Loopback E2E Pipeline (Fase 7.3) : 100 % superado (7/7 pasos)
```

# Notas de la Versión — eMuleRust 0.1.2-lab

## 1. Visión General y Contexto

La versión **0.1.2-lab** de **eMuleRust** constituye el hito de consolidación funcional y certificación de calidad (QA) local. Esta versión transforma el proyecto en un producto QA portátil, reproducible y verificable de extremo a extremo, desde la generación de fixtures hasta el fichero finalizado, sin recurrir a un terminal ni abrir ningún socket de red hacia el exterior.

> **Formulación de certificación de interoperabilidad:**  
> *«eMuleRust es una aplicación de laboratorio local. La compatibilidad eD2K está validada sobre corpus estático y par de referencia emulado en loopback cerrado. No se contacta con ningún servidor eD2K público, nodo Kad o par externo.»*

---

## 2. Novedades y Evoluciones Principales

### 2.1 Separación Estanca entre `data/` y `downloads/`
Con el fin de garantizar una organización limpia y conforme a las expectativas de los usuarios y de los entornos portátiles:
- **Carpeta interna `data/`**: Reservada estrictamente para el estado local del daemon y de la interfaz:
  - Base de datos SQLite WAL (`mule.db`, `mule.db-wal`, `mule.db-shm`).
  - Descubrimiento y token de sesión (`session.json`, `auth.token`).
  - Bloqueo de exclusividad de instancia (`mule.lock`).
  - Ficheros de estado temporales y dispersos en curso de ensamblaje (`.part` y `.part.met`).
- **Carpeta de destino `downloads/`**: Reservada exclusivamente para los ficheros descargados completados, verificados mediante huella criptográfica eD2K/AICH y finalizados (`target_path`).
- En modo portátil, la carpeta predeterminada de descargas se resuelve automáticamente hacia `<bundleRoot>/downloads`, distinta de `<bundleRoot>/data`.

### 2.2 Selección Personalizada de la Carpeta de Destino (`Browse...`)
- La interfaz Flutter Desktop ofrece ahora un selector nativo de carpeta en el cuadro de diálogo de adición (`AddDownloadDialog`).
- El usuario puede elegir cualquier carpeta local válida o dejar la carpeta `downloads/` predeterminada.
- La ruta seleccionada se valida, normaliza y transmite de forma segura al daemon mediante el método RPC `downloads.addEd2kLink`.

### 2.3 Persistencia Definitiva de la Eliminación en SQLite
- La eliminación de una descarga mediante la interfaz de usuario o la API JSON-RPC (`downloads.remove`) ejecuta ahora una instrucción SQL `DELETE FROM downloads WHERE id = ?`.
- Los ficheros temporales asociados (`.part` y `.part.met`) se eliminan de forma inmediata y física del disco en `data/`.
- La tarea eliminada no vuelve a aparecer nunca tras reiniciar el daemon (`test_downloads_remove_persistence_across_daemon_restart`).

### 2.4 Estanqueidad de la Limpieza de Laboratorio (`lab.cleanupTestData`)
- El mecanismo de purga de los datos de prueba elimina únicamente los ficheros de fixtures (`data/fixtures/`) y los fragmentos de descargas incompletas.
- **Regla absoluta**: Los ficheros finalizados situados en la carpeta `downloads/` (o cualquier carpeta personalizada) nunca resultan afectados por la limpieza de laboratorio.

### 2.5 Ergonomía de la Interfaz y Diagnóstico Seguro
- **Chips de capacidades del daemon**: La cabecera de la aplicación muestra etiquetas visuales (`Chip`) que indican las capacidades activas del daemon conectado (`downloads`, `download-events`, `lab-tools`).
- **Abreviatura de identificadores**: Los identificadores largos UUIDv4 de las descargas se abrevian de forma ergonómica (p. ej. `#abc1...`) en la tabla principal.
- **Diagnóstico enriquecido y seguro**: El cuadro de diálogo `DiagnosticDialog` muestra la versión del daemon, los estados de supervisión y censura rigurosamente los datos sensibles (rutas del sistema de usuario y tokens de autenticación `auth.token`).

### 2.6 Fase 7.3: Pipeline E2E QA Automatizado (`scripts/test_qa_loopback_e2e.ps1`)
Un arnés automatizado en PowerShell permite ejecutar un ciclo QA completo en un sandbox temporal externo:
1. Extracción del paquete sellado.
2. Arranque autónomo en modo portátil con verificación del socket loopback (`127.0.0.1`).
3. Descarga nominal de un fixture de 1 MB mediante el par loopback emulado.
4. Descarga de un fixture en la frontera de parte (9 728 001 bytes = 1 parte completa de 9 728 000 bytes + residuo de 1 byte).
5. Escenario de fallo/reanudación: interrupción forzada del daemon en curso de descarga, reinicio, reanudación sin pérdida de los bloques ya recibidos y finalización con verificación de integridad SHA-256.

---

## 3. Guía para los Testers Internos

Para validar el recorrido QA completo directamente desde la aplicación de laboratorio (compilación `qa-loopback`), sigue la secuencia guiada siguiente:

```
┌────────────────────────┐     ┌────────────────────────┐     ┌────────────────────────┐
│ 1. Generate Fixture    │ ──> │ 2. Add to Downloads    │ ──> │ 3. Start Loopback Peer │
└────────────────────────┘     └────────────────────────┘     └────────────────────────┘
                                                                           │
                                                                           ▼
┌────────────────────────┐     ┌────────────────────────┐     ┌────────────────────────┐
│ 6. Cleanup Test Data   │ <── │ 5. Verify Completed    │ <── │ 4. Connect to Peer     │
└────────────────────────┘     └────────────────────────┘     └────────────────────────┘
```

1. **Generar un fixture** (`Generate Fixture`): Crear un fixture sintético determinista (p. ej. `sample_1mb` o `pattern_9728001`).
2. **Añadir a las descargas** (`Add to Downloads`): Inyectar el enlace eD2K generado en la cola (con la carpeta `downloads/` predeterminada o selección personalizada).
3. **Iniciar el par loopback** (`Start Loopback Peer`): Activar el par de referencia emulado en `127.0.0.1`.
4. **Conectar la descarga** (`Connect Download to Peer`): Iniciar la transferencia loopback.
5. **Verificar la finalización** (`Verify Completed`): Observar la transición hacia el estado *Completed* y la presencia del fichero íntegro en la carpeta `downloads/`.
6. **Limpiar los datos de prueba** (`Cleanup Test Data`): Purgar los fixtures temporales sin afectar a los ficheros finalizados.

---

## 4. Preparación de la Fase Tier 3 (Binario Externo Fijado)

El proyecto integra la infraestructura necesaria para ejecutar el arnés Tier 3 en cuanto haya disponible un binario eMule/aMule externo:
- **Binario identificado y verificado**: Ejecutable oficial catalogado.
- **Huella SHA-256 fijada**: Control criptográfico estricto antes de cualquier lanzamiento.
- **Configuración hermética**: Ficheros `.ini` y directorios de prueba aislados en un sandbox temporal.
- **Confinamiento loopback exclusivo**: Únicamente `127.0.0.1`.
- **Desactivación total de las redes de terceros**: Kademlia (Kad) desactivado, conexión automática desactivada, lista de servidores eD2K vaciada.
- **Ejecución bajo demanda**: La prueba en vivo Tier 3 nunca se ejecuta en la suite normal y requiere un indicador explícito.

---

## 5. Especificaciones de los Dos Archivos de Release

### 5.1 Composición Invariante (22 Ficheros por Paquete Post-Signing)
Cada paquete Windows x64 contiene exactamente **22 ficheros** controlados en `SHA256SUMS.txt` y `version.json`:
1. `bin/mule-cli.exe`
2. `bin/mule-daemon.exe`
3. `data/app.so`
4. `data/flutter_assets/AssetManifest.bin`
5. `data/flutter_assets/FontManifest.json`
6. `data/flutter_assets/fonts/MaterialIcons-Regular.otf`
7. `data/flutter_assets/NativeAssetsManifest.json`
8. `data/flutter_assets/NOTICES.Z`
9. `data/flutter_assets/packages/cupertino_icons/assets/CupertinoIcons.ttf`
10. `data/flutter_assets/shaders/ink_sparkle.frag`
11. `data/flutter_assets/shaders/stretch_effect.frag`
12. `data/icudtl.dat`
13. `eMuleRust.exe`
14. `flutter_windows.dll`
15. `native_assets.json`
16. `portable.flag`
17. `sbom/flutter.cdx.json`
18. `sbom/license-audit.json`
19. `sbom/license-audit.md`
20. `sbom/rust.cdx.json`
21. `sbom/sbom.cdx.json`
22. `sbom/windows-bundle.cdx.json`

### 5.2 Detalles de las Distribuciones

#### A. Compilación Estándar Local
- **Carpeta de ensamblaje**: `dist/eMuleRust-Standard-Windows/`
- **Archivo sellado**: `eMuleRust-v0.1.2-lab-standard-windows-x64.zip` (14 599 546 bytes)
- **Huella SHA-256**: `f7efc509c577ffa62240a0798e6b41993f0a5883683cca39bbdaaeca7800a7cf`
- **Commit de procedencia**: `76005757e488523dfe4960da4569f7257cb80c8b`
- **Capacidades RPC anunciadas**: `["downloads", "download-events"]`
- **Rutas `lab.*`**: Inactivas (rechazo con `-32601 MethodNotFound`).
- **Texto de difusión recomendado**:
  > **eMuleRust v0.1.2-lab — Standard**  
  > Compilación interna local sin herramientas de laboratorio compiladas. Incluye el daemon local, la interfaz Flutter Desktop, la separación `data/` frente a `downloads/`, la persistencia SQLite y la gestión de descargas. Las rutas de prueba, el par loopback y las funciones de simulación no están presentes.

#### B. Compilación QA Loopback
- **Carpeta de ensamblaje**: `dist/eMuleRust-Qa-Loopback-Windows/`
- **Archivo sellado**: `eMuleRust-v0.1.2-lab-qa-loopback-windows-x64.zip` (14 815 380 bytes)
- **Huella SHA-256**: `05f27c1e08af97278d28444eb5be24e002c3c4b157098d054e4458769642c2cc`
- **Commit de procedencia**: `76005757e488523dfe4960da4569f7257cb80c8b`
- **Capacidades RPC anunciadas**: `["downloads", "download-events", "lab-tools"]`
- **Rutas `lab.*`**: Activas (`lab.status`, `lab.createFixture`, `lab.startLoopbackPeer`, etc.).
- **Texto de difusión recomendado**:
  > **eMuleRust v0.1.2-lab — QA Loopback**  
  > Compilación interna de validación que incluye las herramientas de laboratorio `lab-tools`. Los intercambios de prueba están estrictamente limitados a `127.0.0.1`; no se contacta con ningún servidor eD2K público, nodo Kad o par externo. Utilizar para los escenarios QA guiados y las pruebas E2E automatizadas.

---

## 6. Comandos de Validación y Reproducción

Todas las comprobaciones se ejecutan sin conexión y sin elevación de privilegios:

```powershell
# 1. Pruebas unitarias y de integración Rust
cargo test --workspace
cargo test -p mule-daemon --features lab-tools

# 2. Análisis estático y pruebas Flutter Desktop
cd apps/desktop
flutter analyze
flutter test
cd ../..

# 3. Ensamblaje de los paquetes Windows (Standard y QA Loopback)
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\build_windows_release.ps1 -Flavor standard
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\build_windows_release.ps1 -Flavor qa-loopback

# 4. Suites de validación post-empaquetado y matriz de robustez
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\test_packaged_bundle.ps1 -Flavor standard
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\test_packaged_bundle.ps1 -Flavor qa-loopback
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\test_rc_matrix.ps1

# 5. Pipeline E2E QA automatizado (Fase 7.3)
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\test_qa_loopback_e2e.ps1
```

---

## 7. Marcador Final de Validación

```
=== SCOREBOARD eMuleRust 0.1.2-LAB ===

Rust Crates :
  - mule-core           :  12 pruebas superadas
  - mule-format         :  19 pruebas superadas
  - mule-protocol       : 160 pruebas superadas (+ 1 prueba Tier 3 en vivo ignorada por defecto)
  - mule-daemon         :  13 pruebas superadas (nominal) / 27 pruebas superadas (--features lab-tools)
Total Rust              : 204 pruebas (nominal) / 218 pruebas (lab-tools), 0 fallos

Flutter Desktop         :  53 pruebas superadas, 0 fallos

Matriz Post-Empaquetado : 7/7 pruebas superadas (espacios/unicode, conflicto de puerto, bloqueo exclusivo, crash recovery, reubicación, read-only, autorresolución portátil).
Pipeline E2E QA         : 100 % superado (fixture nominal 1 MB, frontera 9 728 001 bytes, reanudación tras fallo, conformidad downloads/ frente a data/).
Censura en Diagnóstico  : 100 % garantizada (ningún auth.token ni clave privada en el portapapeles).
Política de Ejecución   : 100 % RemoteSigned, 0 % Bypass.
```

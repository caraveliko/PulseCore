# Notas de la Versión — eMuleRust 0.1.1-lab

## 1. Visión General y Contexto

La versión **0.1.1-lab** de **eMuleRust** constituye la Release Candidate (RC) consolidada y endurecida para las fases 7.1 (Modo Portátil y Ergonomía de Desarrollador) y 7.2 (Herramientas de Laboratorio Seguras y Experimentación Local).

Esta actualización aporta garantías estructurales reforzadas entre el producto nominal y las funcionalidades de laboratorio, formaliza la taxonomía de errores JSON-RPC, prohíbe cualquier fuga de secretos en las herramientas de diagnóstico y establece una política estricta de ejecución de scripts PowerShell (`-ExecutionPolicy RemoteSigned`).

---

## 2. Novedades y Evoluciones Principales

### 2.1 Resolución Híbrida y Determinista de la Carpeta de Trabajo (Workdir)
Se ha formalizado y sincronizado de forma idéntica una cascada única de resolución entre el daemon Rust (`crates/mule-daemon/src/workdir.rs`) y la aplicación Flutter (`apps/desktop/lib/services/work_directory_resolver.dart`):
1. Argumento CLI `--workdir <ruta>` (prioridad absoluta: CI, diagnóstico aislado, multiinstancia).
2. Variable de entorno `EMULERUST_WORKDIR` (configuración de desarrollador sin modificar los argumentos).
3. Detección de `portable.flag` en la raíz del paquete (modo autónomo ZIP/llave USB/QA, carpeta local `data/`).
4. Alternativa estándar UAC de Windows `%LOCALAPPDATA%\eMuleRust` (instalación clásica multiusuario).
*Retrocompatibilidad: el antiguo alias obsoleto `MULE_DATA_DIR` se reconoce temporalmente como solución de respaldo con advertencia en el registro.*

### 2.2 Doble Bloqueo de las Capacidades de Laboratorio
La exposición de las herramientas de prueba y simulación local en la interfaz de usuario está condicionada por un doble bloqueo estricto:
$$\text{isLabModeAvailable} = (\text{workdirMode} == \text{Portable}) \land (\text{capabilities}.\text{contains}(\text{"lab-tools"}))$$
- **Compilación Estándar (Producción)**: `default = []`. El daemon no compila ningún código de simulación (`lab-loopback`, `ed2k-loopback`, `debug-rpc`), solo anuncia `["downloads", "download-events"]` y rechaza cualquier llamada RPC `lab.*` con el código `-32601` (`MethodNotFound`).
- **Compilación QA / Laboratorio (`-LabMode`)**: Compila la característica `lab-tools`, anuncia `["downloads", "download-events", "lab-tools"]`.
- **Interfaz Flutter**: La guía de pruebas, el asistente de inyección de enlaces eD2K sintéticos y los botones de limpieza quedan estrictamente ocultos en modo instalado (`InstalledDefault`), incluso si se conectase accidentalmente un daemon experimental.

### 2.3 Taxonomía Formal de Errores JSON-RPC
Unificación y codificación de los códigos de error según la especificación JSON-RPC 2.0 y extensión de negocio aislada:
- Códigos estándar: `-32700` (ParseError), `-32600` (InvalidRequest), `-32601` (MethodNotFound), `-32602` (InvalidParams), `-32603` (InternalError).
- Códigos de aplicación eMuleRust:
  - `-32000`: `CODE_AUTH_FAILED` — Autenticación WebSocket no válida o ausente.
  - `-32001`: `CODE_PROTOCOL_MISMATCH` — Versión de protocolo incompatible.
  - `-32002`: `CODE_FORBIDDEN_IN_INSTALLED_MODE` — Intento de ejecución de un comando de laboratorio en modo instalado.
  - `-32003`: `CODE_UNSAFE_CLEANUP_TARGET` — Intento de eliminación fuera de la carpeta de pruebas autorizada u objetivo con recorrido de ruta / enlace simbólico.
  - `-32004`: `CODE_FIXTURE_DOWNLOAD_MISMATCH` — Descarga no elegible para los fixtures de prueba.
  - `-32005`: `CODE_LOOPBACK_PEER_UNAVAILABLE` — Par de prueba loopback no disponible.
  - `-32010`: `CODE_BUSINESS_ERROR` — Error de negocio genérico.
- **Seguridad del campo `data`**: El campo `data` de los errores es siempre un objeto estructurado y saneado (`{"reason": "...", ...}`), excluyendo formalmente cualquier token, secreto, ruta de usuario o clave criptográfica.

### 2.4 Garantía Cero Fugas de Secretos en las Herramientas de Diagnóstico
- La función «Diagnóstico» de la interfaz de usuario (`DiagnosticDialog`) genera informes completos saneados para la depuración local.
- La huella secreta `auth.token`, los tokens de autorización y las rutas que contienen nombres de usuarios del sistema se censuran sistemáticamente (`[REDACTED]`).
- Una prueba unitaria automatizada (`test/ui/diagnostic_dialog_test.dart`) valida formalmente que no aparezca ningún valor de token ni ocurrencia de `auth.token` en el portapapeles copiado.

### 2.5 Política de Scripting PowerShell Endurecida
- Prohibición total e incondicional de la opción `-ExecutionPolicy Bypass` en toda la cadena de CI, compilación, pruebas y documentación.
- Adopción exclusiva de `-ExecutionPolicy RemoteSigned` para el conjunto de scripts del repositorio (`scripts/build_windows_release.ps1`, `scripts/test_packaged_bundle.ps1`, `scripts/test_rc_matrix.ps1`, `scripts/generate_sbom.ps1`).

---

## 3. Entregables Diferenciados y Transparencia del Inventario (22 Ficheros por Paquete)

Con el fin de eliminar cualquier ambigüedad de procedencia y evitar la sobrescritura mutua de artefactos, el proceso de compilación genera dos distribuciones selladas diferenciadas en directorios y archivos dedicados.

### 3.1 Composición Invariante (22 Artefactos Post-Signing)
Cada paquete Windows x64 contiene exactamente **22 artefactos** referenciados en su propio fichero `SHA256SUMS.txt` y manifiesto `version.json`:
1. `bin/mule-cli.exe`
2. `bin/mule-daemon.exe`
3. `data/app.so`
4. `data/flutter_assets/AssetManifest.bin` (sustituye al formato JSON redundante)
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
15. `native_assets.json` (resolución de los activos nativos Flutter)
16. `portable.flag`
17. `sbom/flutter.cdx.json`
18. `sbom/license-audit.json`
19. `sbom/license-audit.md`
20. `sbom/rust.cdx.json`
21. `sbom/sbom.cdx.json`
22. `sbom/windows-bundle.cdx.json`

### 3.2 Especificaciones de los Dos Archivos de Release

#### A. Compilación Estándar Local
- **Carpeta de ensamblaje**: `dist/eMuleRust-Standard-Windows/`
- **Archivo sellado**: `eMuleRust-v0.1.1-lab-standard-windows-x64.zip` (14 488 636 bytes)
- **Huella SHA-256**: `381239d6d94c3d74e00fe0d737644574d4083e1a1dbcfa1bc80b1df9e0f79f81`
- **Manifiesto `version.json`**:
  ```json
  {
    "distributionFlavor": "standard",
    "labToolsEnabled": false
  }
  ```
- **Capacidades RPC anunciadas**: `["downloads", "download-events"]`
- **Rutas `lab.*`**: Ausentes (rechazo con `-32601 MethodNotFound`).
- **Texto de difusión recomendado**:
  > **eMuleRust v0.1.1-lab — Standard**  
  > Compilación interna local sin herramientas de laboratorio compiladas. Incluye el daemon local, la interfaz Flutter Desktop, la persistencia y las funciones de gestión de descargas. Las rutas de prueba, el par loopback y las funciones de simulación no están presentes.

#### B. Compilación QA Loopback
- **Carpeta de ensamblaje**: `dist/eMuleRust-Qa-Loopback-Windows/`
- **Archivo sellado**: `eMuleRust-v0.1.1-lab-qa-loopback-windows-x64.zip` (14 702 418 bytes)
- **Huella SHA-256**: `2360a460bb4fdd179bb9c43a5e4f0ca0a552492fead22153fc3e12e81e90ed59`
- **Manifiesto `version.json`**:
  ```json
  {
    "distributionFlavor": "qa-loopback",
    "labToolsEnabled": true
  }
  ```
- **Capacidades RPC anunciadas**: `["downloads", "download-events", "lab-tools"]`
- **Rutas `lab.*`**: Activas (`lab.status`, `lab.createFixture`, etc.).
- **Texto de difusión recomendado**:
  > **eMuleRust v0.1.1-lab — QA Loopback**  
  > Compilación interna de validación que incluye las herramientas de laboratorio `lab-tools`. Los intercambios de prueba están estrictamente limitados a `127.0.0.1`; no se contacta con ningún servidor eD2K público, nodo Kad o par externo. Utilizar únicamente para los escenarios QA guiados.

---

## 4. Certificación de Interoperabilidad y Compromisos Herméticos

La certificación formal de eMuleRust permanece rigurosamente invariable:

> **Formulación de certificación obligatoria:**
> *«Compatibilidad eD2K experimental validada sobre corpus estático y par de referencia emulado, en entorno loopback cerrado. El arnés Tier 3 para un binario eMule/aMule externo fijado está disponible, pero su prueba en vivo permanece ignorada por defecto mientras no se proporcione un binario verificado.»*

Los elementos voluntariamente ausentes se mantienen estrictamente:
- **Cero tráfico WAN / Internet**: Escuchas y conexiones restringidas a `127.0.0.1` (loopback estricto).
- **Cero protocolo Kad / DHT**: Ninguna rutina de enrutamiento distribuido.
- **Cero servidor central eD2K**: Ningún contacto con servidores públicos.
- **Cero subida / seeding**: Ninguna difusión espontánea de bloques hacia el exterior.

---

## 5. Comandos de Validación y Reproducción

Todas las comprobaciones se ejecutan sin conexión y sin elevación de privilegios:

```powershell
# 1. Pruebas unitarias y de integración Rust (con y sin lab-tools)
cargo test --workspace
cargo test -p mule-daemon --features lab-tools

# 2. Análisis estático y pruebas Flutter Desktop
cd apps/desktop
flutter analyze
flutter test
cd ../..

# 3. Generación del paquete estándar sellado (se requiere árbol limpio)
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\build_windows_release.ps1 -Flavor standard

# 4. Generación del paquete QA experimental (con herramientas de laboratorio)
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\build_windows_release.ps1 -Flavor qa-loopback

# 5. Pruebas automatizadas post-empaquetado (por flavor) y matriz de robustez
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\test_packaged_bundle.ps1 -Flavor standard
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\test_packaged_bundle.ps1 -Flavor qa-loopback
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\test_rc_matrix.ps1
```

---

## 6. Marcador Final de Validación

```
=== SCOREBOARD eMuleRust 0.1.1-LAB ===

Rust Crates :
  - mule-core           :  12 pruebas superadas
  - mule-format         :  19 pruebas superadas
  - mule-protocol       : 160 pruebas superadas (+ 1 prueba Tier 3 en vivo ignorada por defecto)
  - mule-daemon         :  13 pruebas superadas (nominal) / 27 pruebas superadas (--features lab-tools)
Total Rust              : 204 pruebas (nominal) / 218 pruebas (lab-tools), 0 fallos

Flutter Desktop         :  49 pruebas superadas (+18 nuevas pruebas cobertura lab y diagnóstico), 0 fallos

Matriz Post-Empaquetado : 7/7 pruebas superadas (espacios/unicode, conflicto de puerto, bloqueo exclusivo, crash recovery, reubicación, read-only, autorresolución portátil).
Censura en Diagnóstico  : 100 % garantizada (ningún auth.token ni clave privada en el portapapeles).
Política de Ejecución   : 100 % RemoteSigned, 0 % Bypass.
```

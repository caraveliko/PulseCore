# Notas de la Versión — eMuleRust 0.1.3-lab

## 1. Visión General y Contexto

La versión **0.1.3-lab** de **eMuleRust** constituye el hito de integración y consolidación principal del proyecto. Reúne:
1. **La finalización del protocolo eD2K real (Tier 3.1 a Tier 3.6)** validado frente al ejecutable oficial eMule 0.50a en entorno loopback hermético.
2. **La totalidad de las funcionalidades de Fase 8**: inspección detallada, Part Map interactivo, categorías locales con jerarquía de destino en 4 niveles, historial SQLite con retención y exportaciones censuradas, e importación experimental defensiva de ficheros `.part.met` con informe de auditoría.
3. **La industrialización y el empaquetado Windows certificados**: dos variantes estancas (`standard` y `qa-loopback`), validación de migración de esquema SQLite desde la Fase 7, portabilidad verificada y matriz de supervisión RC completa.

> **Declaración Explícita de Confinamiento y Certificación de Interoperabilidad:**  
> *«eMuleRust es una aplicación de laboratorio local. Las transferencias eD2K, la reanudación, la verificación por partes y la finalización se han validado frente a la referencia eMule 0.50a en un entorno estrictamente confinado a 127.0.0.1. No hay activado ningún servidor eD2K público, nodo Kad, par externo, tráfico UDP ni compresión de protocolo.»*

---

## 2. Novedades y Evoluciones Principales

### 2.1 Protocolo eD2K Loopback Real e Interoperabilidad eMule 0.50a (Tier 3.1 – 3.6)
- **Handshake eD2K canónico y Negociación**: Handshake completo eD2K con soporte de las etiquetas de capacidades eMule (`ET_MOD_VERSION`, `ET_VERSION`, `ET_EXT_FEATURES`).
- **Petición y Declaración de Estado de Fichero**: Decodificación y codificación de los mensajes `FILE_STATUS_REQ` y `FILE_STATUS` con bitfield por bloque y gestión de fuente completa (0 partes requeridas).
- **Planificación Binaria Acotada y Secuenciación**: Descomposición determinista en bloques de 9 728 000 bytes y subrangos de peticiones `REQUESTPARTS` (máximo 3 rangos contiguos de 10 KB).
- **Recepción y Escritura en Flujo (`SENDINGPART`)**: Escritura secuencial y resiliente de los bloques en el fichero `.part` temporal sin desbordamiento de memoria.
- **Control Criptográfico MD4 y Hash Raíz eD2K**: Cálculo de hash al vuelo por bloque de 9,28 MB y cálculo de la huella raíz eD2K global.
- **Residuo de Fin de Fichero y Promoción Atómica**: Soporte de las transferencias de frontera con residuo de 1 byte (p. ej. fixture de 9 728 001 bytes) y renombrado atómico hacia la carpeta `downloads/`.
- **Reanudación tras Fallo Forzado**: Demostración de reanudación sin corrupción tras detención abrupta al ~50 % de transferencia, conservando el estado SQLite y los bloques ya validados.

### 2.2 Vista Detallada, Part Map y Diagnósticos Saneados (Fase 8.1)
- **`DownloadDetailDialog`**: Acceso instantáneo a los metadatos eD2K (UUID, nombre, hash MD4, enlace canónico `ed2k://`, tamaño total, tamaño transferido, bytes verificados).
- **Part Map eD2K Interactivo**: Segmentación visual en bloques de 9,28 MB con código de color unificado (Verificado, Completado, En curso, Faltante) e información emergente interactiva con desplazamientos exactos.
- **Diagnósticos Desensibilizados**: Generación de un informe de diagnóstico expurgado de cualquier token de autenticación (`auth.token`) y anonimizando los directorios locales (`[DOWNLOAD_DIR]`).
- **Modificación de Destino Segura**: Cuadro de diálogo dedicado con sonda de escritura (`write-probe`) que valida los permisos del sistema de ficheros antes de confirmar.

### 2.3 Historial Local, Estadísticas y Retención Automática (Fase 8.2)
- **Tabla SQLite `download_history`**: Registro inmutable de cada evento del ciclo de vida (`completed`, `failed`, `removed`, `resumed`, `verified_pending_promotion`).
- **Panel `HistoryView`**: Seguimiento de volúmenes y conteos en 4 ventanas temporales distintas (Acumulado total, últimos 7 días, últimas 24 horas, Sesión actual).
- **Política de Retención Configurable**: Poda automática configurable mediante la interfaz (7, 30, 90 días o conservación indefinida).
- **Exportaciones Censuradas QA**: Exportación de eventos en formato JSON o CSV con anonimización de las rutas de usuario para compartir registros de forma segura.

### 2.4 Categorías Locales y Resolución Jerárquica de Destino (Fase 8.3)
- **Organización Visual por Categorías**: Asignación de colores hexadecimales, prioridades predeterminadas y directorios de destino específicos.
- **Garantía Absoluta de No Destrucción**: La eliminación de una categoría reasigna automáticamente las descargas con `category_id = NULL` (pasando a la vista «Sin categoría») sin tocar jamás los ficheros.
- **Resolución de Destino en 4 Niveles**:
  $$\text{targetDirectory (tarea)} > \text{defaultDestination (categoría)} > \text{defaultDownloadDir (global SQLite)} > \text{downloads/ (paquete portátil)}$$

### 2.5 Importación Defensiva de Ficheros `.part.met` eMule (Fase 8.4)
- **Formatos Soportados**: Decodificación robusta de las cabeceras estándar eMule (`0xE0`), ficheros grandes (`0xE1`) y formatos nuevos (`0xE2`).
- **Inversión de las Etiquetas de Gaps**: Cálculo algorítmico del complemento de los huecos para identificar con precisión los rangos de bytes válidos sin asumir continuidad arbitraria.
- **Verificación Criptográfica MD4 por Parte**: Comparación sistemática de cada parte de 9,28 MB frente a la tabla de hashes del `.part.met`. Los bloques corruptos se invalidan y reclasifican como huecos.
- **Confinamiento y Cero Tráfico de Red**:
  - El fichero de origen del usuario nunca se modifica ni se mueve.
  - Los datos se copian bajo `data/temp/<id>.part`.
  - La descarga se inicializa en estado **`Paused`** con `sourceCount: 0`. No se realiza ningún intento de conexión de red.
- **Informe de Auditoría de Importación**: Visualización tras la importación de una auditoría completa con estadísticas legibles, botón de copia al portapapeles y cierre explícito.

### 2.6 Empaquetado Multivariante, SBOM y Migración SQLite (Fase 8.5)
- **Dos Variantes Windows**:
  - **Standard (`standard`)**: Depurada de todo código de prueba (`lab-tools` desactivado, métodos `lab.*` devolviendo `-32601`).
  - **QA Loopback (`qa-loopback`)**: Equipada con las herramientas de inyección de fixtures y el servidor de referencia emulado en `127.0.0.1`.
- **Migración de Esquema SQLite Idempotente**: Compatibilidad hacia atrás certificada; apertura transparente de las bases de datos de Fase 7 con adición sin pérdida de la columna `category_id` y de las nuevas tablas.
- **Portabilidad y Actualización**: Sobrescritura de paquete binario probada sin alteración de los datos bajo `data/` y `downloads/`.

---

## 3. Especificaciones y Huellas de los Archivos Entregables

### 3.1 Archivos Oficiales v0.1.3-lab

| Variante | Fichero Archivo ZIP | Tamaño (bytes) | Huella SHA-256 |
| :--- | :--- | :--- | :--- |
| **Standard** | `eMuleRust-v0.1.3-lab-standard-windows-x64.zip` | 14 830 472 | `406F8A8DB03389D43DE20ECDC6AE7638226194B51C4422CEAE0D21D96FA3EB25` |
| **QA Loopback** | `eMuleRust-v0.1.3-lab-qa-loopback-windows-x64.zip` | 15 036 190 | `EDF14B8ABA85C90E393C01B8B988E984CC9010F5B365C55407206CA327A1DC70` |

### 3.2 Composición y Concordancia Cuatripartita
Cada archivo contiene **22 ficheros** rigurosamente controlados, verificados conformes al 100 % entre los ficheros físicos, `version.json`, `SHA256SUMS.txt` y el SBOM CycloneDX 1.5 (`sbom/sbom.cdx.json` que agrupa 240 componentes auditados frente a `deny.toml`).

---

## 4. Certificación de Referencia eMule 0.50a (Tier 3)

El binario oficial de referencia eMule 0.50a utilizado para la cualificación local y el endurecimiento del protocolo está fijado por su huella criptográfica:
- **Fichero ejecutable**: `emule.exe` (procedente de `eMule0.50a.zip`)
- **Huella SHA-256**: `06cdf814387f627a4bd05a0c68211f715bfa952423e8e8a462e1f47c11a4d20e`
- **Política de distribución**: De acuerdo con las reglas de confidencialidad y confinamiento de laboratorio, el ejecutable eMule externo, sus directorios de configuración y sus trazas en bruto no se distribuyen en el repositorio Git ni en los archivos de release. Únicamente se documentan las instrucciones de prueba de interoperabilidad loopback bajo variable de entorno (`$env:EMULE_REFERENCE_BIN`) en `docs/walkthroughTier3.md`.

---

## 5. Síntesis de Cualificación QA

| Banco de Pruebas | Herramienta / Comando | Resultado Certificado |
| :--- | :--- | :--- |
| **Pruebas Unitarias y de Integración Rust** | `cargo test --workspace` | **151 / 151 superadas (100 %)** |
| **Análisis Estático Flutter** | `flutter analyze` | **0 advertencias, 0 errores, 0 lint** |
| **Pruebas de Widgets y Modelos Flutter** | `flutter test` | **74 / 74 superadas (100 %)** |
| **Smoke Test Paquete Standard** | `scripts/test_packaged_bundle.ps1 -Flavor standard` | **Conforme y validado en sandbox** |
| **Smoke Test Paquete QA Loopback** | `scripts/test_packaged_bundle.ps1 -Flavor qa-loopback` | **Conforme y validado en sandbox** |
| **Pipeline E2E QA Loopback** | `scripts/test_qa_loopback_e2e.ps1` | **7 / 7 pasos superados con éxito** |
| **Matriz Supervisión y Entorno RC** | `scripts/test_rc_matrix.ps1` | **7 / 7 escenarios validados sin regresión** |
| **Migración de Esquema SQLite** | `sqlite_categories_tests::test_phase7_to_phase8_schema_migration` | **Migración transparente, cero pérdidas** |

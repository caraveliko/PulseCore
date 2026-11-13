# Notas de la Versión — eMuleRust 0.1.0-lab

## 1. Visión General

La versión **0.1.0-lab** constituye el primer hito estable y congelado del proyecto **eMuleRust**. Formaliza y bloquea todo el trabajo de decodificación en línea eD2K, el motor de almacenamiento disperso, la supervisión local y los mecanismos de validación de interoperabilidad sin depender de ninguna red externa.

---

## 2. Certificación de Interoperabilidad Honesta

El estado de certificación del proyecto se basa en una distinción estricta y formal en tres niveles:

| Nivel | Título | Estado | Qué queda demostrado | Qué no se afirma |
| :--- | :--- | :--- | :--- | :--- |
| **Tier 1** | Corpus Estático | **Validado (100 %)** | Conformidad sintáctica de los decodificadores/codificadores sobre fixtures de bytes congelados y reales. | Sin interacción dinámica con un proceso en ejecución. |
| **Tier 2** | Par Emulado | **Validado (100 %)** | Conformidad de intercambio (handshake, estado, partes, centinelas, residuos) con un modelo que simula las variantes oficiales en loopback. | Sin interoperabilidad observada con un binario oficial compilado. |
| **Tier 3** | Binario Externo Fijado | **Arnés Listo** | El arnés de orquestación hermética, neutralización de configuración y confinamiento loopback está auditado. | No se ejecuta ninguna prueba en vivo sin un binario externo verificado (prueba ignorada por defecto). |

> **Formulación de certificación obligatoria:**
> *«Compatibilidad eD2K experimental validada sobre corpus estático y par de referencia emulado, en entorno loopback cerrado. El arnés Tier 3 para un binario eMule/aMule externo fijado está disponible, pero su prueba en vivo permanece ignorada por defecto mientras no se proporcione un binario verificado.»*

---

## 3. Elementos Voluntariamente Ausentes

Para preservar la integridad del entorno de investigación y la seguridad absoluta del anfitrión, los siguientes componentes están **estricta y voluntariamente ausentes** de esta versión:
1. **Cero conectividad de red pública**: todos los sockets de escucha y conexión están confinados incondicionalmente a la interfaz `127.0.0.1`. Cualquier intento de enlace hacia una dirección que no sea loopback dispara un error inmediato.
2. **Cero enrutamiento Kad / Kademlia / DHT**: no se implementa ni se emite ningún intercambio DHT.
3. **Cero servidor de indexación eD2K**: ningún protocolo de sincronización con servidores eD2K públicos.
4. **Cero subida / seeding**: ninguna rutina de envío o compartición espontánea de bloques hacia otros pares.
5. **Cero fugas de identidad del anfitrión**: las trazas normalizadas JSON y los informes de CI se auditan frente a 13 vectores de fuga (rutas Windows/Unix, tokens de autenticación, anulaciones bidi, caracteres invisibles).

---

## 4. Dependencias Críticas y Versiones Fijadas

Los componentes del workspace se compilan y bloquean con las siguientes versiones exactas (`Cargo.lock`):

| Dependencia | Versión fijada | Uso |
| :--- | :--- | :--- |
| `tokio` | `1.53.1` | Runtime asíncrono multihilo para el daemon y las pruebas loopback |
| `axum` | `0.8.9` | Servidor WebSocket local para la API JSON-RPC del daemon |
| `rusqlite` | `0.33.0` | Persistencia local SQLite (modo WAL, integridad verificada) |
| `md4` | `0.10.2` | Cálculo de hashsets y hashes de ficheros eD2K históricos |
| `sha1` | `0.10.7` | Cálculo de árboles AICH de verificación de integridad de bloques |
| `fs4` | `1.1.0` | Bloqueo exclusivo entre procesos (`mule.lock`) |
| `bytes` | `1.12.1` | Búferes de memoria de alto rendimiento para el entramado en línea |
| `unicode-normalization` | `0.1.25` | Normalización Unicode canónica (Form C / NFC) |
| `serde` / `serde_json` | `1.0.229` / `1.0.139` | Serialización y trazas normalizadas JSON legibles por máquina |
| `rand` | `0.9.5` | Generación CSPRNG del token secreto de autenticación (`auth.token`) |
| `uuid` | `1.26.1` | Identificadores únicos de las descargas |
| `tempfile` | `3.27.0` | Aislamiento hermético de las pruebas y escritura segura de artefactos CI |

---

## 5. Comandos Completos de Validación (100 % Offline)

Todos los comandos siguientes deben ejecutarse con un 100 % de éxito sin acceso a Internet:

```powershell
# 1. Verificación de la compilación del workspace
cargo check --workspace --all-targets --all-features

# 2. Verificación estática del código (0 advertencias toleradas)
cargo clippy --workspace --all-targets --all-features -- -D warnings

# 3. Ejecución de las 204 pruebas unitarias, de integración y adversarias Rust
cargo test --workspace --all-features

# 4. Análisis estático del código Flutter Desktop
cd apps/desktop
flutter analyze

# 5. Ejecución de las 31 pruebas Flutter Desktop
flutter test
```

---

## 6. Marcador Final de Validaciones

```
=== SCOREBOARD eMuleRust 0.1.0-LAB ===

Rust Crates :
  - mule-core     :  12 pruebas superadas
  - mule-daemon   :  13 pruebas superadas
  - mule-format   :  19 pruebas superadas
  - mule-protocol : 160 pruebas superadas (+ 1 prueba Tier 3 en vivo ignorada por defecto)
Total Rust        : 204 pruebas superadas, 0 fallos

Flutter Desktop   :  31 pruebas superadas, 0 fallos

TOTAL DEL PROYECTO : 235 pruebas activas validadas al 100 %.
GUARDA ESTÁTICA    : 0 unwrap/expect en mule-protocol::ed2k.
LINTING            : cargo clippy sin advertencias (-D warnings).
AISLAMIENTO        : 100 % loopback 127.0.0.1 estricto.
ANONIMIZACIÓN      : 100 % validada mecánicamente (Unicode NFC + bidi/ctrl escapados).
```

---

## 7. Entregables de la Release Candidate (Industrialización Local)

El paquete autónomo Windows x64 se genera mediante `scripts/build_windows_release.ps1`:
- **Ejecutable Principal**: `eMuleRust.exe` (binario UI Flutter Desktop Windows autónomo).
- **Daemon en Segundo Plano**: `bin/mule-daemon.exe` (motor local supervisado).
- **Manifiesto de Trazabilidad**: `version.json` (commit Git exacto, marca temporal UTC, versiones de las toolchains, `gitDirty: false`).
- **Sumas de Comprobación**: `SHA256SUMS.txt` para los ficheros del paquete.
- **Inventario SBOM CycloneDX 1.5**:
  - `sbom/rust.cdx.json`: 191 componentes Rust.
  - `sbom/flutter.cdx.json`: 33 componentes Flutter.
  - `sbom/windows-bundle.cdx.json`: componentes binarios Windows ensamblados.
  - `sbom/sbom.cdx.json`: 239 componentes consolidados.
  - `sbom/license-audit.md`: informe de conformidad que confirma un 100 % de compatibilidad con `deny.toml`.
- **Archivo Sellado**: `eMuleRust-v0.1.0-lab-windows-x64.zip` (16 386 637 bytes) y su huella criptográfica `eMuleRust-v0.1.0-lab-windows-x64.zip.sha256`:
  `09023bb37bfb45d5a028ada95f5681a4e42df451badd04ca4b9d4ecc99836245`
- **Validación QA**: Protocolo detallado en [docs/QA_RECEPTION_PROTOCOL.md](docs/QA_RECEPTION_PROTOCOL.md) y scripts de prueba automatizados (`scripts/test_packaged_bundle.ps1`, `scripts/test_rc_matrix.ps1`).

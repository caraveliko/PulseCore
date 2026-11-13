# CLAUDE.md — Reglas para agentes de IA

## Visión general

**eMuleRust** (**Pulse Core**) es una reimplementación moderna de los protocolos P2P **eD2K**, **Kad DHT (Kademlia)** y **BitTorrent**:
- Motor y servidor en **Rust** (workspace Cargo, Tokio async, Axum HTTP/WebSocket, códecs de cable acotados)
- **Interfaz Web (WebUI) embebida** servida directamente por `mule-daemon` (`http://127.0.0.1:4660`)
- **Bandeja del sistema de Windows (Windows tray)** integrada en `crates/mule-daemon/src/tray.rs`
- Servidor **MCP** que expone ~87 herramientas para agentes de IA

Licencia: GPL-2.0-or-later. Plataformas objetivo: Windows 10/11 x64 y Linux (`x86_64` / `aarch64`).

## Mapa del repositorio

| Ruta | Rol |
| :--- | :--- |
| `crates/mule-protocol` | Códecs de cable eD2K/Kad, opcodes, entramado. **No modificar sin vector de prueba ni especificación `docs/ed2k/`** |
| `crates/mule-daemon` | Servidor Tokio/Axum (`kad`, `kad_udp`, `peer_session`, `inbound_listener`, asignador de enjambre), WebUI embebida, APIs RPC/REST y bandeja de Windows (`src/tray.rs`) |
| `crates/mule-core` | Dominio de negocio, motor de almacenamiento disperso |
| `crates/mule-crypto` | MD4, hashes eD2K, identificadores |
| `crates/mule-filter` | Filtrado IP (`ipfilter.dat`) |
| `crates/mule-network-policy` | Canalización anti-SSRF, aislamiento loopback, interruptor de corte (*kill switch*) |
| `crates/mule-mcp` | Servidor MCP (~87 herramientas) |
| `crates/mule-cli` | CLI de control |
| `crates/mule-format` | Formatos de archivo (`.met`, `.dat`…) |
| `crates/mule-torrent` | Motor BitTorrent integrado |
| `crates/mule-assistant` | Asistente y automatización integrada |
| `crates/mule-test-support` | Utilidades compartidas de pruebas de integración |
| `docs/` | Documentación: `docs/ed2k/` (especificaciones + ADRs), `docs/walkthroughs/`, `docs/qa/`, `docs/INDEX.md` (mapa) |
| `scripts/` | Scripts PowerShell de compilación y validación |

## Comandos de validación

Ejecutar antes de dar por terminado cualquier cambio:

```powershell
# Rust — lint (siempre, todo el workspace: detecta roturas entre crates)
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Rust — pruebas (100 % sin conexión): el/los crate(s) modificado(s)
cargo test -p <crate> --all-features
# ampliar al workspace si el cambio afecta a varios crates o a APIs públicas:
cargo test --workspace --all-features

# Validación completa
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\verify_all.ps1
```

## Reglas críticas

1. **Fidelidad protocolaria**: la referencia de verdad es el código fuente de **eMule 0.72a** (`D:\eMule0.72a-Sources\eMule0.72a-Sources\srchybrid`). Cualquier cambio en opcodes, entramado o comportamiento en cable debe verificarse contra esas fuentes y `docs/ed2k/`. Consultar los ADRs antes de modificar `mule-protocol`.
2. **Pruebas exclusivamente sin conexión**: ninguna prueba debe emitir tráfico de red saliente. Usar loopback (`test_qa_loopback_e2e.ps1` como referencia).
3. **Seguridad no negociable**: nunca debilitar la canalización anti-SSRF, el aislamiento loopback por defecto, el interruptor de corte atómico ni la seudonimización de pares (`mule-network-policy`). Nunca generar una versión pública con `-LabMode` / `--features lab-tools` — `test_packaged_bundle.ps1` verifica que el sabor `standard` no exponga ningún método `lab.*`.
4. **Windows-first**: instrucciones y scripts en PowerShell, no en bash (salvo despliegues Linux/aarch64 específicos). Rutas con `\` o entrecomillado adecuado.
5. **Dependencias**: revisar `deny.toml` antes de añadir una crate; preferir las dependencias del workspace (`[workspace.dependencies]`).
6. **Sin valores codificados a fuego (*hardcoded*)**: los ajustes de Rust (umbral, tiempo de espera, cuota, tamaño, reintentos) deben vivir en `DaemonConfig`, `ResourceBudgets`, `mule-protocol/src/limits.rs` o en una `const` documentada al principio del archivo. Un valor = una única fuente de verdad.
7. **Un componente por archivo**: toda función o bloque lógico que pueda vivir en su propio archivo debe tenerlo — sin archivos monolíticos ni literales sueltos en la lógica. Al crear un archivo, aplicar la sección «Constantes y ajustes (Rust)» siguiente.

## Convenciones de código

- **Idioma**: toda la documentación, los *Inner Doc Comments* (`//!`) de los archivos que componen el programa y todos los archivos Markdown (`.md`) **deben estar redactados exclusivamente en español de España** (no se admiten otros idiomas).
- **Errores en Rust**: `thiserror` para errores tipados; prohibido usar `unwrap()` fuera de las pruebas.
- **Registro (*Logging*)**: `tracing` (nunca `println!` dentro de los crates).
- **Asincronía**: Tokio; sin bloqueos síncronos en tareas asíncronas.
- **Arquitectura de interfaz**: servidor web embebido en `mule-daemon` (Axum + WebSockets + WebUI) y bandeja del sistema de Windows en `crates/mule-daemon/src/tray.rs`.
- **Advertencias (*Warnings*)**: el workspace compila con `-D warnings`; no se tolera ninguna advertencia.

## Constantes y ajustes (Rust)

### Sin números mágicos en la lógica

Todo valor de ajuste — umbral, tiempo de espera, cuota, tamaño, intervalo, número de reintentos — debe ser una `const` con nombre o un campo de configuración, nunca un literal en el cuerpo de una función. Un ajuste tiene **una única fuente de verdad**: si varios módulos lo necesitan, vive en el archivo compartido y no duplicado.

| Tipo de valor | Ubicación obligatoria |
| :--- | :--- |
| Ajuste operativo del daemon (pares, puertos, cuotas, capacidades, directorios) | `DaemonConfig` (`crates/mule-daemon/src/config.rs`); si es modificable en caliente → clave en `KNOWN_SETTINGS_KEYS` (`rpc/handlers/settings.rs`, p. ej. `minUploadKbs`) |
| Presupuestos y tiempos de espera de red (conexiones, límite de tasa, búferes) | `ResourceBudgets` (`crates/mule-network-policy/src/budgets.rs`) |
| Límites protocolarios / de cable (tamaños de trama, etiquetas, partes) | `mule-protocol/src/limits.rs` o `ed2k/limits.rs`; struct `*Limits` si el límite es inyectable |
| Constante interna de un único módulo | `const` documentada al inicio del archivo (p. ej. `INFLIGHT_BLOCK_TTL_MS`) |

Tolerados: `0`, `1`, índices de bucle, literales evidentes en las pruebas y formatos de visualización.

```rust
// Prohibido — el «10» es ilocalizable sin leer todo el archivo
if upload_kbs < 10 { ... }

// Correcto — el ajuste vive en un único lugar
/// Tasa mínima de subida por par (KiB/s).
pub const MIN_UPLOAD_KBS: u32 = 10;
if upload_kbs < MIN_UPLOAD_KBS { ... }
```

Modificar un ajuste nunca debe exigir rastrear la lógica: se abre el archivo de constantes o configuración, se cambia el valor y listo.

## Documentación asociada

- `docs/INDEX.md` — inventario exhaustivo de archivos
- `docs/architecture/rapport_architecture_et_avancement.md` — estado técnico
- `docs/CHANGELOG.md` — historial de versiones

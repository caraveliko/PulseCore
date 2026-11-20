# GEMINI.md — Reglas para agentes de IA

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

Ejecutar antes de dar por terminado cualquier cambio (aprovechando los *harnesses* consolidados y el corredor inteligente):

```powershell
# Validación inteligente según cambios locales (recomendada en iteraciones rápidas)
python3 scripts/smart_test.py --mode smart
python3 scripts/smart_test.py --mode fast

# Filtrar un módulo concreto dentro de un arnés consolidado
cargo test -p mule-daemon --test daemon_suite notifications_tests --all-features

# Rust — lint y pruebas completas (100 % sin conexión)
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --lib --tests

# Validación completa (o -Mode smart / -Mode fast)
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\verify_all.ps1
```

## Reglas críticas

1. **Fidelidad protocolaria**: la referencia de verdad es el código fuente de **eMule 0.72a** (`D:\eMule0.72a-Sources\eMule0.72a-Sources\srchybrid`). Cualquier cambio en opcodes, entramado o comportamiento en cable debe verificarse contra esas fuentes y `docs/ed2k/`. Consultar los ADRs antes de modificar `mule-protocol`.
2. **Pruebas exclusivamente sin conexión**: ninguna prueba debe emitir tráfico de red saliente. Usar loopback (`test_qa_loopback_e2e.ps1` como referencia).
3. **Seguridad no negociable**: nunca debilitar la canalización anti-SSRF, el aislamiento loopback por defecto, el interruptor de corte atómico ni la seudonimización de pares (`mule-network-policy`). Nunca generar una versión pública con `-LabMode` / `--features lab-tools` — `test_packaged_bundle.ps1` verifica que el sabor `standard` no exponga ningún método `lab.*`.
4. **Windows-first**: instrucciones y scripts en PowerShell, no en bash (salvo despliegues Linux/aarch64 específicos). Rutas con `\` o entrecomillado adecuado.
5. **Dependencias**: revisar `deny.toml` antes de añadir una crate; preferir las dependencias del workspace (`[workspace.dependencies]`).
6. **Política de contenido «Solo Vídeo» (invariante fijo)**: Pulse Core solo admite archivos de vídeo (y subtítulos complementarios dentro de torrents con vídeo) mediante la fuente única de verdad `mule_network_policy::content_policy` (ADR `docs/ed2k/decisions/0006-politica-contenido-solo-video.md`). Prohibido duplicar listas de extensiones o añadir opciones para desactivar esta política.

## Convenciones de código

- **Idioma**: toda la documentación, **TODOS los comentarios del código** (`//`, `///`, `//!`, `/* ... */`), cadenas de documentación de los archivos que componen el programa y todos los archivos Markdown (`.md`) **deben estar redactados exclusivamente en español de España** (no se admiten otros idiomas: ni francés, ni inglés, ni ningún otro).
- **Errores en Rust**: `thiserror` para errores tipados; prohibido usar `unwrap()` fuera de las pruebas.
- **Registro (*Logging*)**: `tracing` (nunca `println!` dentro de los crates).
- **Asincronía**: Tokio; sin bloqueos síncronos en tareas asíncronas.
- **Arquitectura de interfaz**: servidor web embebido en `mule-daemon` (Axum + WebSockets + WebUI) y bandeja del sistema de Windows en `crates/mule-daemon/src/tray.rs`.
- **Advertencias (*Warnings*)**: el workspace compila con `-D warnings`; no se tolera ninguna advertencia.

## Documentación asociada

- `docs/INDEX.md` — inventario exhaustivo de archivos
- `docs/architecture/rapport_architecture_et_avancement.md` — estado técnico
- `docs/CHANGELOG.md` — historial de versiones

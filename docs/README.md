# Portal de documentación — eMuleRust 🚀

Bienvenido a la documentación técnica de **eMuleRust**, una reimplementación moderna, modular y altamente segura de los protocolos **eD2K (eDonkey2000)** y **Kad DHT (Kademlia)**, junto con un servidor de herramientas **Model Context Protocol (MCP)** para agentes de IA.

El proyecto está estructurado en **Rust** (motor de descarga, códec de red, demonio asíncrono Tokio y servidor MCP) y una **interfaz web integrada** servida directamente por el demonio.

---

## 🧭 Plan de orientación de la documentación

Toda la documentación del proyecto está clasificada y organizada dentro de este directorio `docs/`:

```
docs/
├── README.md                      # Este portal de bienvenida general
├── INDEX.md                       # Inventario cartografiado de TODOS los archivos del repositorio
├── CHANGELOG.md                   # Registro de cambios (Keep a Changelog)
├── SETTINGS.md                    # Referencia de ajustes (settings.*, limits.*) y registro
├── ASSISTANT.md                   # Guía de uso del asistente de IA local
│
├── architecture/                  # Arquitectura del sistema e informes técnicos
│   ├── rapport_architecture_et_avancement.md
│   ├── rpc_protocol.md
│   └── plan_daemon_events.md
│
├── releases/                      # Notas de versión detalladas
│   ├── RELEASE_NOTES_0.1.0-lab.md
│   ├── RELEASE_NOTES_0.1.1-lab.md
│   ├── RELEASE_NOTES_0.1.2-lab.md
│   ├── RELEASE_NOTES_0.1.3-lab.md
│   └── RELEASE_NOTES_0.1.8.md
│
├── qa/                            # Cualificación, protocolos de prueba y conformidad
│   ├── QA_RECEPTION_PROTOCOL.md
│   ├── compatibility-matrix.md
│   └── test-vectors.md
│
└── ed2k/                          # Especificaciones de red eD2K y Decisiones de Arquitectura (ADR)
    ├── compatibility-notes.md
    ├── framing.md
    ├── limits.md
    ├── message-catalog.md
    ├── opcodes.md
    ├── opcodes_matrix.md
    ├── references.md
    ├── scope.md
    ├── decisions/
    └── messages/
```

---

## 📑 Acceso rápido por dominio

### 1. Arquitectura del sistema y demonio
* **[Informe global de arquitectura y avance](architecture/rapport_architecture_et_avancement.md)**: Visión general detallada de los crates Rust, el demonio y la arquitectura.
* **[Protocolo RPC del demonio](architecture/rpc_protocol.md)**: Contrato JSON-RPC 2.0/WebSocket — handshake, métodos, eventos, códigos de error y versionado.
* **[Plan de eventos del demonio](architecture/plan_daemon_events.md)**: Diseño de eventos en tiempo real y suscripciones por tópicos.
* **[Referencia de ajustes](SETTINGS.md)**: Todas las claves `settings.*`/`limits.*` — valores por defecto, efectos, equivalentes en eMule — además del registro del demonio (`logs/daemon.<date>.log` JSONL rotativo, `RUST_LOG`, `logLevel` en caliente).
* **[Cartografía global de archivos](INDEX.md)**: Directorio exhaustivo de cada archivo del repositorio.
* **[Registro de cambios](CHANGELOG.md)**: Historial de versiones y evoluciones.

### 2. Garantía de calidad y especificaciones de red
* **[Protocolo de recepción QA](qa/QA_RECEPTION_PROTOCOL.md)**: Guía de pruebas para la cualificación de una versión.
* **[Matriz de compatibilidad de protocolos](qa/compatibility-matrix.md)**: Síntesis de los perfiles `Emule072a`, `Emule050a` (histórico), `Amule233`, etc.
* **[Especificaciones eD2K y ADRs](ed2k/compatibility-notes.md)**: Decisiones de diseño y codificación de red.

---

## 🛠️ Comandos de verificación global (100 % sin conexión)

```powershell
# 0. Validación inteligente recomendada durante el desarrollo (crates y suites afectadas)
python3 scripts/smart_test.py --mode smart

# 1. Compilación y verificación del workspace completo
cargo check --workspace --all-targets --all-features

# 2. Linter Rust estricto sin advertencias
cargo clippy --workspace --all-targets --all-features -- -D warnings

# 3. Suite completa de pruebas Rust
cargo test --workspace --all-features --lib --tests
```

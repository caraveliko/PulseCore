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
│
├── architecture/                  # Arquitectura del sistema e informes técnicos
│   ├── rapport_architecture_et_avancement.md
│   ├── rpc_protocol.md
│   └── plan_daemon_events.md
│
├── walkthroughs/                  # Guías de ejecución e interoperabilidad (Fases 1 a 15)
│   ├── README.md                  # Índice maestro de walkthroughs
│   ├── walkthrough_Phase_01_a_03c.md
│   ├── walkthrough_Tier3_eMule050a.md
│   ├── walkthrough_Phase_07.md
│   ├── walkthrough_Phase_08.md
│   ├── walkthrough_Phase_09.md
│   ├── walkthrough_Phase_10.md
│   ├── walkthrough_Phase_11.md
│   ├── walkthrough_Phase_12.md
│   ├── walkthrough_Phase_13.md
│   ├── walkthrough_Phase_14.md
│   └── walkthrough_Phase_15.md
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

### 2. Guías técnicas por fase (Walkthroughs)
* **[Índice maestro de walkthroughs](walkthroughs/README.md)**
  * [Fases 1 a 3C](walkthroughs/walkthrough_Phase_01_a_03c.md): Fundamentos Rust, almacenamiento disperso, demonio local, bloqueos `fs4` y empaquetado.
  * [Tier 3 eMule 0.50a](walkthroughs/walkthrough_Tier3_eMule050a.md): Cualificación frente al binario oficial de eMule 0.50a en loopback.
  * [Fase 7 (v0.1.2-lab)](walkthroughs/walkthrough_Phase_07.md): Modo portátil, aislamiento `data/` frente a `downloads/` y persistencia SQLite.
  * [Fase 8](walkthroughs/walkthrough_Phase_08.md): Mapa de partes bloque a bloque, historial y estadísticas, categorías, importación defensiva `.part.met`.
  * [Fase 9](walkthroughs/walkthrough_Phase_09.md): Política de red defensiva en 7 capas, permisos RAII, Kill Switch y filtro IP `ipfilter.dat`.
  * [Fase 10](walkthroughs/walkthrough_Phase_10.md): Infraestructura de servidores eD2K (Lugdunum TCP, HighID/LowID, búsqueda y fuentes).
  * [Fase 11](walkthroughs/walkthrough_Phase_11.md): Swarm multifuente, asignador Water-Filling y arranque de baja latencia.
  * [Fase 12](walkthroughs/walkthrough_Phase_12.md): Fundamentos de Kademlia DHT (tabla K-Buckets LRU, transporte UDP, bootstrap local).
  * [Fase 13](walkthroughs/walkthrough_Phase_13.md): Source Exchange v4 (SX1/SX2), bloques comprimidos zlib y servidor MCP (83 herramientas de IA).
  * [Fase 14](walkthroughs/walkthrough_Phase_14.md): Convergencia de búsqueda Kad DHT (`CUInt128`, opcode universal `0x3B`) y Clean Architecture.
  * [Fase 15](walkthroughs/walkthrough_Phase_15.md): Swarm multipares real (`sync_transfer_rates`), respeto a las colas de eMule y registros estratégicos.

### 3. Garantía de calidad y especificaciones de red
* **[Protocolo de recepción QA](qa/QA_RECEPTION_PROTOCOL.md)**: Guía de pruebas para la cualificación de una versión.
* **[Matriz de compatibilidad de protocolos](qa/compatibility-matrix.md)**: Síntesis de los perfiles `Emule072a`, `Emule050a` (histórico), `Amule233`, etc.
* **[Especificaciones eD2K y ADRs](ed2k/compatibility-notes.md)**: Decisiones de diseño y codificación de red.

---

## 🛠️ Comandos de verificación global (100 % sin conexión)

```powershell
# 1. Compilación y verificación del workspace completo
cargo check --workspace --all-targets --all-features

# 2. Linter Rust estricto sin advertencias
cargo clippy --workspace --all-targets --all-features -- -D warnings

# 3. Suite completa de pruebas Rust
cargo test --workspace --all-features
```

# Walkthrough — Fase 12: Kademlia (Kad DHT) Mínimo y Transporte Loopback

Este documento registra la arquitectura, las especificaciones protocolarias, las implementaciones Rust y Flutter, así como la validación completa de la **Fase 12** de eMuleRust (Lotes 12.0a, 12.0b y 12.1).

---

## 1. Visión General y Alcance Realizado

La Fase 12 dota a eMuleRust del soporte completo de la tabla de enrutamiento distribuida Kademlia (Kad v2), estrictamente confinada en bucle local (`127.0.0.1` / perfil `LocalLab`), con cero fugas WAN no solicitadas y un kill switch instantáneo.

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        Interfaz Desktop (Flutter)                       │
│  ┌───────────────────────────────────────────────────────────────────┐  │
│  │                            KadView                                │  │
│  │  - Estado UDP (Activo / Inactivo), Banner Confinamiento Loopback  │  │
│  │  - ID Kad Local (128 bits), Contadores Paquetes TX / RX           │  │
│  │  - Estadísticas Buckets (128 buckets), Contactos Activos y Base   │  │
│  │  - Barra de acciones adaptable (Iniciar Bootstrap, Importar .dat) │  │
│  │  - Tabla paginada y filtrable de contactos con Ping de latencia   │  │
│  └──────────────────────────────────┬────────────────────────────────┘  │
└─────────────────────────────────────┼───────────────────────────────────┘
                                      │ WebSocket JSON-RPC
┌─────────────────────────────────────▼───────────────────────────────────┐
│                     mule-daemon (KadService)                            │
│  ┌─────────────────────────────┐     ┌───────────────────────────────┐  │
│  │       KadUdpService         │     │         KadService            │  │
│  │ - tokio::net::UdpSocket     │     │ - KBucketTable (128 buckets)  │  │
│  │ - InFlight Transactions     │◄───►│ - SQLite: kad_contacts        │  │
│  │ - Timeout 5s, TX ID 32-bit  │     │ - Iterative Search (alpha = 3)│  │
│  │ - KillSwitch Integration    │     │ - Nodes.dat Parser v0/v1/v2   │  │
│  └──────────────┬──────────────┘     └───────────────────────────────┘  │
└─────────────────┼───────────────────────────────────────────────────────┘
                  │ Datagramas UDP (≤ 1472 bytes)
┌─────────────────▼───────────────────────────────────────────────────────┐
│                   mule-protocol (KadCodec)                              │
│  - PR_KADEMLIA (0xE4), PR_KADEMLIA_PACKED (0xE5)                        │
│  - KADEMLIA_PING (0x60), KADEMLIA_PONG (0x61)                           │
│  - KADEMLIA_HELLO_REQ (0x11), KADEMLIA_HELLO_RES (0x19)                 │
│  - KADEMLIA2_BOOTSTRAP_REQ (0x21), KADEMLIA2_BOOTSTRAP_RES (0x29)       │
│  - KADEMLIA2_REQ (0x21), KADEMLIA2_RES (0x29)                           │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Desglose de los Lotes

### 2.1 Lote 12.0a — Datos, Persistencia, Fichero `nodes.dat` e Interfaz Kad (Zero Socket)
- **Modelo de Datos y Enrutamiento (`mule-core`)**:
  - Identificador Kad de 128 bits (`KadId`) con cálculo de distancia métrica XOR (`^`) y recuento de prefijo común (`leading_zeros`).
  - `KBucketTable`: 128 buckets, capacidad $K = 20$ contactos activos por bucket + caché de 10 sustitutos (LRU y desalojo controlado).
  - Persistencia SQLite en la tabla `kad_contacts` con inserción por lotes, paginación, filtrado y purga.
- **Formato `nodes.dat` (`mule-format`)**:
  - Decodificador/codificador defensivo para las versiones 0, 1 y 2 de `nodes.dat`.
  - Límites estrictos: tamaño de fichero $\le 5\text{ MiB}$, máximo 10 000 contactos, máximo 50 etiquetas por contacto.
- **Interfaz de Usuario (`apps/desktop`)**:
  - Creación de `KadView` y `KadStatusModel` con visualización de las métricas de la tabla DHT.

### 2.2 Lote 12.0b — Transporte UDP Loopback y Ciclo de Vida de las Transacciones
- **Codecs de Red (`mule-protocol`)**:
  - Magic bytes oficiales: `PR_KADEMLIA` (`0xE4`) y `PR_KADEMLIA_PACKED` (`0xE5`).
  - Tramas `KadPing`, `KadPong`, `KadHelloReq`, `KadHelloRes`.
  - Control estricto del tamaño de los datagramas: rechazo inmediato si $> 1472$ bytes.
- **Gestor UDP y Transacciones (`mule-daemon`)**:
  - `KadUdpService` sobre socket `UdpSocket` asíncrono enlazado bajo `ListenPermit`.
  - Seguimiento de peticiones en vuelo con ID de transacción aleatorio (32 bits), validación de dirección de origen y tiempo de espera de 5 segundos.
  - Actualización y verificación de los contactos en la tabla SQLite y los buckets al recibir un `Pong`.
  - Endpoints RPC: `kad.startUdp`, `kad.stopUdp`, `kad.pingContact`.

### 2.3 Lote 12.1 — Bootstrap Kad, Búsqueda Iterativa y Nodos Lab
- **Codecs Bootstrap y Find Node (`mule-protocol`)**:
  - Codecs para `KADEMLIA2_BOOTSTRAP_REQ` (`0x21`), `KADEMLIA2_BOOTSTRAP_RES` (`0x29`), `KADEMLIA2_REQ` (`0x21`) y `KADEMLIA2_RES` (`0x29`).
  - Acotación estricta: máximo 20 contactos por respuesta, protección contra el truncamiento.
- **Motor de Arranque y Búsqueda Iterativa (`mule-daemon`)**:
  - `KadService::bootstrap_from_node`: arranque directo mediante un nodo objetivo `IP:port` y registro de los contactos devueltos.
  - `KadService::find_node`: búsqueda iterativa con paralelismo $\alpha = 3$, consulta de los contactos más cercanos en métrica XOR hasta la convergencia.
  - Endpoints JSON-RPC: `kad.bootstrap` y `kad.findNode`.
- **UI y Diálogo Bootstrap (`apps/desktop`)**:
  - Diálogo interactivo que permite introducir IP y puerto de un nodo de arranque.
  - Barra de herramientas adaptable a ventanas compactas y anchas.

### 2.4 Lote 12.2 — Búsqueda de Fuentes e Indexación de Ficheros Kad
- **Codecs Wire Framing (`mule-protocol`)**:
  - Opcodes Kad2: `KADEMLIA2_SEARCH_SOURCE_REQ` (`0x22`), `KADEMLIA2_SEARCH_SOURCE_RES` (`0x2A`), `KADEMLIA2_PUBLISH_SOURCE_REQ` (`0x23`), `KADEMLIA2_PUBLISH_SOURCE_RES` (`0x2B`).
  - Estructuras en línea: `KadSearchSourceReq`, `KadSearchSourceRes`, `KadPublishSourceReq`, `KadPublishSourceRes`, `KadSourceWire`.
  - Límites estrictos: $\le 50$ fuentes por respuesta, tamaño de trama $\le 1472$ B.
- **Indexación y Persistencia (`mule-core`)**:
  - Tabla SQLite `kad_sources_index` con gestión TTL (`expires_at_unix_ms`), purga automática e indexación por clave hash.
  - Adición de la variante `PeerSourceOrigin::Kad` para identificar la procedencia Kad en el enjambre.
- **Motor de Búsqueda y Publicación (`mule-daemon`)**:
  - `KadUdpService::send_search_sources` y `send_publish_source` con transacciones tipadas y timeouts.
  - `KadService::search_sources`: selección de los 3 nodos más cercanos según la métrica XOR, consulta UDP, filtrado por política de red / IPFilter, conversión a `PeerSource` (origen `Kad`) e inyección automática en `DownloadService`.
  - `KadService::publish_source`: anuncio de un fichero ante los 20 nodos más cercanos a su hash.
  - Endpoints JSON-RPC: `kad.searchSources` y `kad.publishSource`.
- **Integración UI Desktop (`apps/desktop`)**:
  - Botón *"Solicitar a la red Kad"* en el diálogo de detalle de descarga (`DownloadDetailDialog`).
  - Insignia de origen `kad` en la lista de fuentes eD2K/Kad.
  - Soporte completo en `MuleDaemonClient`, `RpcDaemonClient`, `FakeDaemonClient` y `DownloadsViewModel`.

### 2.5 Lote 12.3 — Búsqueda por Palabras Clave en la DHT Kad (Kad Keyword Search, Publishing & Distributed Indexing)
- **Codecs Wire Framing (`mule-protocol`)**:
  - Opcodes Kad2: `KADEMLIA2_SEARCH_KEYWORD_REQ` (`0x24`), `KADEMLIA2_SEARCH_KEYWORD_RES` (`0x2C`), `KADEMLIA2_PUBLISH_KEYWORD_REQ` (`0x25`), `KADEMLIA2_PUBLISH_KEYWORD_RES` (`0x2D`).
  - Estructuras en línea: `KadSearchKeywordReq`, `KadSearchKeywordRes`, `KadPublishKeywordReq`, `KadPublishKeywordRes`, `KadKeywordEntryWire`.
  - Límites estrictos: $\le 50$ entradas de palabras clave por respuesta (`MAX_KAD_SEARCH_KEYWORDS`), tamaño de trama $\le 1472$ B.
- **Indexación por Palabras Clave y Persistencia (`mule-core`)**:
  - Normalización de palabras clave: extracción de palabras $\ge 3$ caracteres, normalización a minúsculas y hash MD4 de 128 bits (`KadId`).
  - Tabla SQLite `kad_keywords_index` con gestión TTL (`expires_at_unix_ms`), purga automática e indexación combinada `(keyword_hash, file_hash)`.
- **Motor de Búsqueda y Publicación (`mule-daemon`)**:
  - `KadUdpService::send_search_keyword` y `send_publish_keyword` con seguimiento de transacciones y respondedores entrantes.
  - `KadService::search_keywords`: extracción/hash del término, localización de los nodos más cercanos en la tabla de enrutamiento, consulta UDP, filtrado de política de red (`LocalLab` / IPFilter), deduplicación y devolución en formato compatible con `SearchResultFileModel`.
  - `KadService::publish_keyword`: segmentación del nombre de fichero en palabras clave y publicación ante los 20 nodos más cercanos a cada palabra clave.
  - Endpoints JSON-RPC: `kad.searchKeywords` y `kad.publishKeywords`.
- **Integración UI Desktop (`apps/desktop`)**:
  - Selector de destino de búsqueda en `SearchView` (`SegmentedButton`: *Servidor eD2K* / *Red Kad DHT*).
  - Adaptación dinámica del placeholder, del banner de estado (`Kad DHT Activo` / advertencia de inactividad) y de los botones de redirección.
  - Visualización de los resultados Kad en la tabla de búsqueda y descarga directa en 1 clic mediante enlace eD2K.
  - Soporte en `MuleDaemonClient`, `RpcDaemonClient`, `FakeDaemonClient`, `DownloadsViewModel` y suites de pruebas UI.

---

## 3. Matriz de Endpoints RPC Kad

| Método RPC | Parámetros | Resultado | Descripción |
| :--- | :--- | :--- | :--- |
| `kad.status` | Ninguno | `KadStatusModel` | Devuelve el estado completo (ID, socket, contadores, buckets) |
| `kad.startUdp` | `bindAddress` (opc) | `{ "boundAddress": "..." }` | Inicia el socket UDP (solo Loopback bajo `LocalLab`) |
| `kad.stopUdp` | Ninguno | `{ "stopped": true }` | Cierra inmediatamente el socket UDP y cancela las transacciones |
| `kad.ping` | `ip`, `port`, `kadId` | `{ "success": bool, "latencyMs": u64 }` | Emite un datagrama `Ping` y mide el RTT |
| `kad.bootstrap` | `ip`, `port` | `{ "status": "ok", "contactsDiscovered": u32 }` | Realiza el arranque inicial ante un nodo |
| `kad.findNode` | `targetKadId` | `[KadContactModel]` | Ejecuta una búsqueda iterativa ($\alpha=3$) |
| `kad.searchSources` | `fileHash`, `fileSize` (opc) | `{ "sources": [PeerSource], "count": u32 }` | Busca fuentes Kad para un hash y las inyecta en el enjambre |
| `kad.publishSource` | `fileHash`, `fileSize`, `tcpPort` (opc) | `{ "publishedCount": u32 }` | Publica este nodo como fuente de un fichero en Kad |
| `kad.searchKeywords` | `term`, `minSize` (opc), `maxSize` (opc) | `{ "results": [SearchResultFileModel], "count": u32 }` | Búsqueda de ficheros por palabras clave en Kad DHT |
| `kad.publishKeywords` | `fileHash`, `fileName`, `fileSize` | `{ "publishedCount": u32 }` | Indexa y publica las palabras clave de un fichero en Kad DHT |
| `kad.listContacts` | `limit` (opc) | `[KadContactModel]` | Lista los contactos registrados en la base SQLite |
| `kad.clearContacts` | Ninguno | `{ "clearedCount": u32 }` | Vacía la tabla de contactos |
| `kad.inspectNodesDat` | `filePath` | `NodesDatInspectReportModel` | Análisis defensivo de un fichero `nodes.dat` |
| `kad.importNodesDat` | `filePath` | `{ "importedCount": u32 }` | Importa los contactos de un fichero `nodes.dat` |

---

## 4. Resultados de Validación e Integridad

### A. Backend Rust
- `cargo test --workspace`: **100 % de las pruebas unitarias, de integración y de aislamiento superadas** en los 6 crates del workspace.
- `kad_sources_tests`: Prueba de integración de extremo a extremo validada (Bootstrap simulado $\to$ Publicación $\to$ Búsqueda $\to$ Inyección automática en `DownloadService`).
- `kad_keywords_tests`: Prueba de integración de extremo a extremo validada (Bootstrap simulado $\to$ Publicación palabras clave $\to$ Búsqueda palabras clave $\to$ Validación de resultados).
- `sqlite_kad_keywords_tests`: Validación de la inserción, persistencia y purga TTL en la base SQLite.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **0 warnings, 0 lint**.
- Cero fugas de socket fuera del loopback local `127.0.0.1`.

### B. Cliente Flutter
- `flutter analyze`: **0 advertencias, 0 errores**.
- `flutter test`: **93/93 pruebas en verde**.

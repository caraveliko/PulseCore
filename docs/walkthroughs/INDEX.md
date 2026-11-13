# Directorio Maestro de Guías y Walkthroughs — eMuleRust

Este documento constituye la tabla de orientación general y el índice cronológico exhaustivo de todas las guías de ejecución, interoperabilidad y validación técnica (**walkthroughs**) del proyecto **eMuleRust**, de la Fase 1 a la Fase 17.

---

## 1. Visión General Sinóptica de las Fases y Documentos

| Fase / Hito | Versión | Documento de Walkthrough | Temáticas Clave | Estado de Validación |
| :--- | :--- | :--- | :--- | :--- |
| **Fases 1 a 3C** | `v0.1.0-lab` | [walkthrough_Phase_01_a_03c.md](walkthrough_Phase_01_a_03c.md) | Cimientos Rust (`mule-format`, `mule-core`), daemon Tokio, UI Flutter inicial, endurecimiento del producto, bloqueos `fs4`, tokens CSPRNG, empaquetado Windows. | ✅ Certificado y Congelado |
| **Tier 3.1 a 3.6** | `v0.1.3-lab` | [walkthrough_Tier3_eMule050a.md](walkthrough_Tier3_eMule050a.md) | **Interoperabilidad eMule 0.50a Oficial**: handshake en línea, bitfields, streaming de bloques de 9,28 MB, residuo de 1 byte, reanudación tras fallo y promoción atómica. | ✅ Certificado (100 % Loopback) |
| **Fase 7.2** | `v0.1.2-lab` | [walkthrough_Phase_07.md](walkthrough_Phase_07.md) | Modo portátil híbrido, separación estanca `data/` frente a `downloads/`, persistencia de eliminaciones en SQLite, pipeline QA E2E automatizado. | ✅ Certificado y Sellado |
| **Fase 8** (8.1 a 8.5) | `v0.1.3-lab` | [walkthrough_Phase_08.md](walkthrough_Phase_08.md) | Part Map bloque a bloque, historial local y estadísticas, categorías jerárquicas en 4 niveles, importación defensiva `.part.met`, empaquetado doble variante standard/qa-loopback. | ✅ Certificado y Sellado |
| **Fase 9** (9.0 y 9.1) | `v0.1.3-lab` | [walkthrough_Phase_09.md](walkthrough_Phase_09.md) | Política de red defensiva en 7 etapas, permisos RAII, Kill Switch atómico, anti-SSRF, filtrado IP dicotómico (`ipfilter.dat`), Token Bucket, seudonimización. | ✅ Validado (Crates dedicados) |
| **Fase 10** (10.0 a 10.2) | `v0.1.3-lab` | [walkthrough_Phase_10.md](walkthrough_Phase_10.md) | Decodificación binaria `server.met` (`0xE0`), sesión TCP servidor eD2K real (Lugdunum), HighID/LowID, búsqueda en catálogo de servidor y resolución de fuentes. | ✅ Validado e Integrado en UI |
| **Fase 11** | `v0.1.4` / `v0.1.5` | [walkthrough_Phase_11.md](walkthrough_Phase_11.md) | Enjambre multifuente autónomo, asignador dinámico **Weighted Max-Min Fair-Share Water-filling**, arranque de baja latencia (30s/60s/300s), etiquetas fijas eD2K `0x11..0x7F`. | ✅ Validado y Fusionado |
| **Fase 12** | `v0.1.4` | [walkthrough_Phase_12.md](walkthrough_Phase_12.md) | Cimientos Kademlia DHT: estructura `KadId` de 128 bits, tabla de 128 K-Buckets con caché LRU, transporte UDP asíncrono, arranque bootstrap y pantalla Kad UI. | ✅ Validado y Sellado |
| **Fase 13** | `v0.1.6` | [walkthrough_Phase_13.md](walkthrough_Phase_13.md) | **Source Exchange v4 (SX1 y SX2)** descentralizado, transferencia de bloques comprimidos zlib (`OP_COMPRESSEDPART`) y servidor Agente IA nativo **MCP (83 herramientas)**. | ✅ Validado y Fusionado |
| **Fase 14** | `v0.1.7` | [walkthrough_Phase_14.md](walkthrough_Phase_14.md) | **Convergencia de la búsqueda Kad DHT**: métrica DWORDs `CUInt128`, corrección opcode universal `0x3B`, eliminación del stall decay y **Clean Architecture** completa. | ✅ Validado y Fusionado |
| **Fase 15** | `v0.1.8` | [walkthrough_Phase_15.md](walkthrough_Phase_15.md) | **Descarga multipar real**: agregación de velocidad en directo (`sync_transfer_rates`), respeto de colas eMule (`Disconnected in queue` cooldown 10-15m), cola de subida, registros estratégicos, `INDEX.md`. | ✅ Validado en Condiciones Reales |
| **Fase 16** | Unreleased | [CHANGELOG](../CHANGELOG.md) + [rapport §10](../architecture/rapport_architecture_et_avancement.md) | **Interoperabilidad eMule 0.72a**: hashset por parte, AICH de red completo, `OP_OFFERFILES`, callback LowID, escritura `.part.met`, nodo de almacenamiento Kad + republicación, `OP_CANCELTRANSFER`/`OP_OUTOFPARTS`, subida comprimida `OP_COMPRESSEDPART`, multirranuras de subida. | ✅ Pruebas unitarias/integración en verde (loopback/lab) |
| **Fase 17** | Unreleased | [CHANGELOG](../CHANGELOG.md) + [rapport §11](../architecture/rapport_architecture_et_avancement.md) | **Fidelidad de comportamiento eMule**: compartidos parciales publicados desde su creación, contadores de barra lateral en tiempo real (`shares.changed`, `kad.statusChanged` mediante mantenimiento), notificaciones transitorias eliminadas, selectores de ficheros nativos (`file_picker`), barra de partes precisa al byte estilo `DrawStatusBar` (`receivedMask`, `pendingRanges`, disponibilidad `OP_FILESTATUS` de los pares en cola incluida). | ✅ Pruebas unitarias/integración en verde (Rust + Flutter) |

---

## 2. Guía Temática de Navegación

### A. Protocolos de Red y en Línea
* Para comprender el protocolo eD2K oficial y la cualificación frente al binario eMule 0.50a:  
  👉 Consultar [walkthrough_Tier3_eMule050a.md](walkthrough_Tier3_eMule050a.md).
* Para el intercambio de fuentes descentralizado (SX1/SX2) y los bloques comprimidos zlib:  
  👉 Consultar [walkthrough_Phase_13.md](walkthrough_Phase_13.md).
* Para la infraestructura cliente-servidor eD2K (Lugdunum, login, HighID/LowID, búsqueda):  
  👉 Consultar [walkthrough_Phase_10.md](walkthrough_Phase_10.md).
* Para la implementación completa de Kademlia (Kad DHT):  
  👉 Consultar [walkthrough_Phase_12.md](walkthrough_Phase_12.md) (Cimientos) y [walkthrough_Phase_14.md](walkthrough_Phase_14.md) (Convergencia y Opcode 0x3B).

### B. Motor de Descarga y Algoritmos de Enjambre
* Para el funcionamiento del almacenamiento disperso (`PartFile`), de los intervalos (`RangeSet`) y del Part Map:  
  👉 Consultar [walkthrough_Phase_08.md](walkthrough_Phase_08.md).
* Para el asignador de enjambre y el reparto equitativo Water-Filling:  
  👉 Consultar [walkthrough_Phase_11.md](walkthrough_Phase_11.md).
* Para la agregación de velocidad en directo y la gestión de colas de espera remotas eMule:  
  👉 Consultar [walkthrough_Phase_15.md](walkthrough_Phase_15.md).

### C. Seguridad, Confinamiento y Protección de Datos (DLP)
* Para la política de red en 7 capas, el anti-SSRF, el Kill Switch y el filtrado IP:  
  👉 Consultar [walkthrough_Phase_09.md](walkthrough_Phase_09.md).
* Para la separación estanca de los directorios de datos (`data/`) y de los ficheros promovidos (`downloads/`):  
  👉 Consultar [walkthrough_Phase_07.md](walkthrough_Phase_07.md).

### D. Arquitectura de Software y Herramientas de Agentes IA
* Para la refactorización Clean Architecture y la modularización de todos los crates:  
  👉 Consultar [walkthrough_Phase_14.md](walkthrough_Phase_14.md).
* Para la caja de herramientas Agente IA Model Context Protocol (MCP):  
  👉 Consultar [walkthrough_Phase_13.md](walkthrough_Phase_13.md).
* Para el asistente IA local (LLM loopback → herramientas MCP): instalación, modos REPL/serve, perfiles de filtrado y panel Flutter:  
  👉 Consultar [../ASSISTANT.md](../ASSISTANT.md).
* Para el inventario cartografiado de cada fichero del código fuente:  
  👉 Consultar [../INDEX.md](../INDEX.md).

---

## 3. Comandos Globales de Cualificación (100 % Reproducibles Sin Conexión)

```powershell
# 1. Compilación y análisis estático estricto sin advertencias
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings

# 2. Ejecución íntegra de la suite de pruebas Rust (204+ pruebas unitarias y de integración)
cargo test --workspace --all-features

# 3. Análisis estático y validación de la suite de pruebas Flutter Desktop
cd ../../apps/desktop
flutter analyze
flutter test
```

# Referencias Históricas y Bases de Código eD2K / eMule

Este documento recopila los repositorios fuente, revisiones históricas y archivos de referencia aprovechados para la especificación del protocolo eD2K en eMuleRust.

> **Atribuciones:** las menciones de copyright de los proyectos referenciados (eMule © Merkur / eMule Dev Team, aMule © aMule Team…) están centralizadas en el archivo [`NOTICE`](../../NOTICE) en la raíz del repositorio.

---

## 1. Repositorios y Árboles de Código Fuente de Referencia

### 1.1 eMule Official Archive (SourceForge)
- **Proyecto**: eMule (C++ / MFC para Windows)
- **Versión de referencia**: v0.72a — copia local verificada: `D:\eMule0.72a-Sources\eMule0.72a-Sources\srchybrid`
- **Fuente**: Archivo oficial del proyecto eMule en SourceForge (`https://sourceforge.net/projects/emule/`)
- **Archivos clave estudiados**:
  - `srchybrid/ListenSocket.cpp`: Recepción y encaminamiento de las tramas de red brutas, decodificación de la cabecera (protocol byte, packet len, opcode).
  - `srchybrid/BaseClient.cpp`: Lógica de par compartida en 0.72a (`SendHelloPacket:678`, `ProcessHelloPacket:342`, `SendHelloAnswer:892`, `ProcessHelloAnswer:351`, SUI `Send/ProcessSecIdentStatePacket:2076-2143`). *(En 0.50a, esta lógica estaba en `UpDownClient.cpp`, ausente en 0.72a.)*
  - `srchybrid/DownloadClient.cpp`: Descarga (`SendBlockRequests:812`, `ProcessBlockPacket:956`, `ProcessFileStatus:509`).
  - `srchybrid/Packet.cpp`: Formato binario de las tramas, escritura y lectura de tipos escalares, little-endianness, estructura de las cabeceras.
  - `srchybrid/Tag.cpp`: Definición de los formatos TLV (Type-Length-Value) eD2K y de las etiquetas especiales (1 byte vs. nombre textual).
  - `srchybrid/Opcodes.h`: Catálogo exhaustivo de opcodes para los protocolos `0xE3`, `0xC5` y `0xD4`.

### 1.2 irwir/eMule (Continuación Open-Source moderna)
- **Repositorio**: `https://github.com/irwir/eMule`
- **Ramas / Revisiones examinadas**:
  - Rama principal `master` (commits de mantenimiento de 64 bits y correcciones de compilación MSVC).
  - Referencia de alineación para las modificaciones recientes sobre extensiones eMule (AICH, protocolos extendidos, negociación de paquetes).
- **Archivos clave**:
  - `src/UpDownClient.cpp`: Normalización del handshake `OP_HELLO` y `OP_HELLOANSWER`.
  - `src/Tag.cpp`: Gestión de tags extendidos eMule 0.42+ (flag MSB en el tipo de tag).

### 1.3 aMule (Cliente multiplataforma C++ / wxWidgets)
- **Repositorio**: `https://github.com/amule-project/amule`
- **Archivos clave estudiados**:
  - `src/DownloadClient.cpp` y `src/UploadClient.cpp`: Implementación portable del apretón de manos, compatible con Linux/macOS.
  - `src/ECPacket.cpp` y `src/Packet.cpp`: Endianness y serialización/deserialización segura de enteros little-endian.
  - `src/Tag.cpp`: Decodificación portable de cadenas UTF-8 frente a codificaciones de sistema de 8 bits (ISO-8859-1 / Windows-1252).

---

## 2. Tabla de Correspondencia de Tipos Fundamentales

| Concepto Wire eD2K | Representación Fuente C++ | Tipo eMuleRust (`mule-protocol`) | Notas |
| :--- | :--- | :--- | :--- |
| **Protocol Byte** | `uint8` (`0xE3`, `0xC5`, `0xD4`) | `u8` (`PR_ED2K`, `PR_EMULE`, `PR_EMULE_COMPRESSED`) | 1.er byte de cada trama |
| **Packet Length** | `uint32` (little-endian) | `u32` | Longitud = 1 (opcode) + payload |
| **Opcode** | `uint8` | `u8` | Código de operación |
| **User Hash / File Hash** | `uchar[16]` | `[u8; 16]` / `PeerId` | Huella MD4 histórica |
| **Client ID** | `uint32` (little-endian) | `u32` | HighID (IPv4 little-endian) o LowID (< 16777216) |
| **Port** | `uint16` (little-endian) | `u16` | Puerto TCP del par remoto |
| **Tag Count** | `uint32` (little-endian) | `u32` | Número de metadatos TLV asociados |
| **Tag Name** | `uint16 len` + `string` o `uint8 special_id` | `TagName::Text(String)` o `TagName::Id(u8)` | Nombre de etiqueta estándar o ID abreviado |

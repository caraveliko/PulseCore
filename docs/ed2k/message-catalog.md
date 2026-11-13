# Fichas Normalizadas de los Mensajes eD2K Pasivos

Este catálogo describe los formatos en el cable (wire layout), las cotas numéricas, las reglas de decodificación y las referencias de código para cada mensaje contemplado en la Fase 5.

---

## 1. Ficha: `OP_HELLO` (`0x01`)

### Dirección
Bidireccional (generalmente Cliente $\rightarrow$ Par al establecer la conexión TCP).

### Protocol Byte
`0xE3` (`PR_ED2K`).

### Versión y Extensiones
- eDonkey2000 estándar v1: Hash de usuario + Client ID + Puerto + Contador de tags + Tags + Servidor IP/Puerto.
- Extensiones eMule: inclusión de etiquetas de versión (`CT_VERSION`, `CT_EMULE_VERSION`), opciones diversas (`CT_EMULE_MISCOPTIONS`).

### Wire Format
```text
Offset  Type          Champ                  Description
0       u8            Opcode                 0x01 (OP_HELLO)
1       [u8; 16]      User Hash              Huella MD4 del cliente
17      u32 LE        Client ID              ID de red eD2K (HighID o LowID)
21      u16 LE        TCP Port               Puerto de escucha TCP del cliente
23      u32 LE        Tag Count              Número de tags TLV siguientes (N)
27      Tag[N]        Tags                   Secuencia ordenada de tags TLV
...     u32 LE        Server IP              IP del servidor conectado (o 0.0.0.0 si no conectado)
...     u16 LE        Server Port            Puerto del servidor conectado (o 0 si no conectado)
```

### Cotas y Límites
- Tamaño máximo de trama: 65 536 bytes (64 KiB).
- Número máximo de tags: 256 tags.
- Longitud máxima de una cadena de tag: 1 024 bytes.

### Decodificación eMuleRust
- Decodificado en la estructura pasiva `Ed2kHello`.
- Los tags reconocidos (`Name`, `Version`, `Port`) se tipan.
- Los tags desconocidos se preservan en `unknown_tags: Vec<RawTag>`.
- Error si la trama está truncada (`UnexpectedEof`), si el número de tags supera 256 (`TooManyTags`), o si una cadena supera 1024 bytes (`OversizedString`).

### Fixtures Asociadas
- `hello_v1_minimal.bin` (SHA-256 en el manifiesto): apretón de manos válido con 0 tags.
- `hello_v1_tags.bin` (SHA-256 en el manifiesto): apretón de manos con tags de nombre, versión y puerto.
- `malformed/hello_truncated_header.bin`: cabecera truncada.
- `malformed/hello_truncated_tag.bin`: tag incompleto.
- `malformed/oversized_string.bin`: tag de cadena abusivo (> 1024 bytes).
- `malformed/too_many_tags.bin`: contador de tags (> 256).

### Referencias
- eMule 0.72a: `srchybrid/BaseClient.cpp:678` `CUpDownClient::SendHelloPacket`, `BaseClient.cpp:342` `CUpDownClient::ProcessHelloPacket`
- aMule: `src/DownloadClient.cpp:CDownloadClient::SendHelloPacket`

---

## 2. Ficha: `OP_HELLOANSWER` (`0x4C`)

### Dirección
Par remoto $\rightarrow$ Cliente iniciador (en respuesta a `OP_HELLO`).

### Protocol Byte
`0xE3` (`PR_ED2K`).

### Wire Format
Idéntico a `OP_HELLO`, con el opcode `0x4C`:
```text
Offset  Type          Champ                  Description
0       u8            Opcode                 0x4C (OP_HELLOANSWER)
1       [u8; 16]      User Hash              Huella MD4 del par remoto
17      u32 LE        Client ID              ID de red eD2K del par remoto
21      u16 LE        TCP Port               Puerto de escucha TCP del par remoto
23      u32 LE        Tag Count              Número de tags TLV siguientes (N)
27      Tag[N]        Tags                   Secuencia ordenada de tags TLV
...     u32 LE        Server IP              IP del servidor eD2K al que está conectado el par
...     u16 LE        Server Port            Puerto del servidor eD2K
```

### Cotas y Límites
- Tamaño máximo de trama: 65 536 bytes (64 KiB).
- Número máximo de tags: 256 tags.
- Longitud máxima de cadena: 1 024 bytes.

### Decodificación eMuleRust
- Decodificado en la estructura `Ed2kHelloAnswer`.
- Misma tolerancia sobre los tags desconocidos (`unknown_tags: Vec<RawTag>`).

### Fixtures Asociadas
- `hello_answer.bin` (SHA-256 en el manifiesto): respuesta válida.

### Referencias
- eMule 0.72a: `srchybrid/BaseClient.cpp:892` `CUpDownClient::SendHelloAnswer`, `BaseClient.cpp:351` `CUpDownClient::ProcessHelloAnswer`
- aMule: `src/DownloadClient.cpp:CDownloadClient::ProcessHelloAnswer`

---

## 3. Ficha: `OP_FILESTATUS` (`0x50`)

### Dirección
Par remoto $\rightarrow$ Cliente (en respuesta a `OP_FILESTATUSREQ`).

### Protocol Byte
`0xE3` (`PR_ED2K`).

### Wire Format
```text
Offset  Type          Champ                  Description
0       u8            Opcode                 0x50 (OP_FILESTATUS)
1       [u8; 16]      File Hash              Huella MD4 del archivo en cuestión
17      u16 LE        Part Count             Número total de partes (9 728 000 bytes)
19      u8[M]         Status Bitfield        Máscara binaria de disponibilidad: ceil(Part Count / 8) bytes
```

### Cotas y Límites
- `Part Count` máximo: 65 535 partes (representa hasta ~637 GB para partes de 9,5 MB).
- Tamaño mínimo requerido: $17 + \lceil\text{Part Count} / 8\rceil$ bytes.
- Tamaño máximo de trama: 65 536 bytes (suficiente para $65535 / 8 = 8192$ bytes de máscara).

### Decodificación eMuleRust
- Decodificado en `Ed2kFileStatus { file_hash, part_count, status_bitfield }`.
- **`Part Count == 0` = fuente completa** (convención eD2K: la fuente posee el archivo entero y no envía bitfield — eMule `ProcessFileStatus` materializa `memset(m_abyPartStatus, 1, …)`). `compute_part_summaries` cuenta una fuente así para todas las partes.
- La máscara recibida alimenta `availableSourceCount` (barra de partes eMule) mediante `record_source_availability`, incluido en keep-alive en cola de espera; las máscaras caducadas > 30 min se ignoran.
- Rechazo si la trama está truncada respecto al tamaño deducido de `Part Count`.

### Fixtures Asociadas
- `file_status.bin` (SHA-256 en el manifiesto): estado de partes de un archivo multiparte.

### Referencias
- eMule 0.72a: envío inline `srchybrid/ListenSocket.cpp:404` (respuesta a `OP_SETREQFILEID`), recepción `srchybrid/DownloadClient.cpp:509` `CUpDownClient::ProcessFileStatus`
- aMule: `src/UploadClient.cpp:CUploadClient::SendFileStatus`

---

## 4. Ficha: `OP_SETREQFILEID` (`0x47`)

### Dirección
Cliente $\rightarrow$ Par remoto.

### Protocol Byte
`0xE3` (`PR_ED2K`).

### Wire Format
```text
Offset  Type          Champ                  Description
0       u8            Opcode                 0x47 (OP_SETREQFILEID)
1       [u8; 16]      File Hash              Huella MD4 del archivo solicitado
```
Longitud total de la trama: 17 bytes (1 byte opcode + 16 bytes hash).

---

## 5. Ficha: `OP_REQUESTPARTS` (`0x4B`)

### Dirección
Cliente $\rightarrow$ Par remoto.

### Protocol Byte
`0xE3` (`PR_ED2K`).

### Wire Format
```text
Offset  Type          Champ                  Description
0       u8            Opcode                 0x4B (OP_REQUESTPARTS)
1       [u8; 16]      File Hash              Huella MD4 del archivo solicitado
17      u32[3] LE     Start Offsets          3 desplazamientos de inicio de bloques (180 KiB)
29      u32[3] LE     End Offsets            3 desplazamientos de fin de bloques (180 KiB)
```
Tamaño total en el cable: 41 bytes.

---

## 6. Ficha: `OP_SENDINGPART` (`0x46`)

### Dirección
Par remoto $\rightarrow$ Cliente.

### Protocol Byte
`0xE3` (`PR_ED2K`).

### Wire Format
```text
Offset  Type          Champ                  Description
0       u8            Opcode                 0x46 (OP_SENDINGPART)
1       [u8; 16]      File Hash              Huella MD4 del archivo transferido
17      u32 LE        Block Start Offset     Desplazamiento de inicio del bloque en el archivo
21      u32 LE        Block End Offset       Desplazamiento de fin del bloque en el archivo
25      u8[...]       Payload Data           Bytes brutos del bloque (End Offset - Start Offset)
```
Cotas máximas: bloque estándar $\le$ 184 320 bytes (180 KiB).
Tamaño máximo de trama: 262 144 bytes (256 KiB).

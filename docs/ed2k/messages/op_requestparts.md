# Ficha de Especificación: `OP_REQUESTPARTS` (`0x4B`)

## 1. Visión de Conjunto

El mensaje `OP_REQUESTPARTS` permite a un cliente eD2K pedir a un par remoto uno o varios rangos de bytes (bloques de 180 KiB = 184 320 bytes máx.) para un archivo dado, identificado por su hash MD4 de 16 bytes.

En la arquitectura eMuleRust, este mensaje se **utiliza exclusivamente para las transferencias en bucle local cerrado (loopback)** mediante la feature `ed2k-loopback`. Es objeto de un seguimiento estricto por el componente `BlockRequestTracker` según una clave compuesta `[file_hash, start, end]`.

---

## 2. Propiedades del Protocolo

| Propiedad | Valor |
| :--- | :--- |
| **Opcode Hexadecimal** | `0x47` (Canónico eMule `OP_REQUESTPARTS`), `0x4B` (Histórico alternativo) |
| **Nombre Simbólico** | `OP_REQUESTPARTS` / `OP_REQUESTPARTS_EMULE` |
| **Protocolo de Red** | `PR_ED2K` (`0xE3`) |
| **Dirección** | Cliente $\rightarrow$ Par remoto |
| **Estado de Ejecución** | Loopback transfer-only (Fase 6.4 / 6.5) |
| **Tope de Bloque Unitario** | 184 320 bytes (180 KiB) |

---

## 3. Wire Format (Formato en el Cable)

El formato canónico de eMule 0.72a estructura la petición de hasta 3 bloques simultáneos en una misma trama de 40 bytes de payload:

```text
Offset  Type          Champ                  Description
0       u8            Opcode                 0x47 (canónico eMule) o 0x4B (histórico)
1       [u8; 16]      File Hash              Huella MD4 (16 bytes) del archivo requerido
17      u32[3] LE     Start Offsets          3 desplazamientos de inicio de bloques (u32 little-endian)
29      u32[3] LE     End Offsets            3 desplazamientos de fin de bloques (u32 little-endian, exclusivo)
```

- **Tamaño de trama estándar (3 slots)**: 41 bytes en el cable (1 byte protocolo `0xE3` + 4 bytes de longitud `0x28, 0x00, 0x00, 0x00` (40) + 1 byte opcode `0x47` o `0x4B` + 40 bytes payload).
- **Semántica de las Cotas en el Cable**: En las fuentes oficiales de eMule (`DownloadClient.cpp`, `ListenSocket.cpp`), `endpos` es exclusivo: `endpos = block->EndOffset + 1`. La longitud del bloque transferido es exactamente `end - start`.
  - Constructor canónico en `Ed2kBlockRange`: `from_wire_bounds(start, end)` mapeando directamente los valores del cable hacia `[start, end)` tras validar `start < end`.
  - Helpers de compatibilidad: `from_legacy_inclusive_wire_bounds(start, end_inclusive)` para antiguas variantes donkey que transmiten una cota inclusiva.
- **Centinelas para slots no utilizados**:
  - eMule transmite `start == 0 && end == 0` para los slots residuales no pedidos.
  - El decodificador acepta igualmente el centinela `start == u32::MAX && end == u32::MAX`.
- **Formato compacto (1 slot, legacy Donkey)**: 25 bytes en el cable (longitud 24: 16 bytes hash + 4 bytes start + 4 bytes end).

---

## 4. Reglas de Validación y Cotas Numéricas

1. **Relación de desplazamiento**: Para cada rango pedido, `start < end`.
2. **Cota de tamaño de bloque**: `end - start <= 184_320` bytes (tamaño máximo estándar de un bloque eD2K).
3. **Cota de archivo**: `end <= file_size`.
4. **Hash Nulo Rechazado**: Un `file_hash` de 16 bytes a cero `[0u8; 16]` se rechaza estrictamente (`ZeroUserHash` o `MalformedField`).

---

## 5. Precondiciones de Estado y Máquina de Estados

1. **Sesión `Ready`**: `OP_REQUESTPARTS` solo puede emitirse si la sesión está en el estado `Ready { peer }`.
2. **Disponibilidad Confirmada**: La parte que cubre el intervalo `[start..end]` debe haberse confirmado disponible mediante `OP_FILESTATUS` recibido previamente.
3. **In-Flight Tracker (`BlockRequestTracker`)**:
   - Cada rango está indexado por `BlockRequestKey { file_hash, start, end }`.
   - Cualquier duplicado o solapamiento de rango en vuelo se rechaza con `DuplicateBlockRequest`.
   - Tope de peticiones en vuelo: máximo 8 simultáneas. Superación rechazada con `InFlightCapacityReached`.
   - En caso de desconexión o cierre, todas las peticiones en vuelo se limpian (`cancel_all()`).

---

## 6. Referencias Históricas
- eMule 0.72a: `srchybrid/opcodes.h:213`: `#define OP_REQUESTPARTS 0x47`
- eMule 0.72a: `srchybrid/DownloadClient.cpp:812`: `CUpDownClient::SendBlockRequests()`
- eMule 0.72a: `srchybrid/ListenSocket.cpp:510`: `CListenSocket::ProcessPacket()` (case `OP_REQUESTPARTS`)
- aMule: `src/UploadClient.cpp:CUploadClient::ProcessBlockRequests()`

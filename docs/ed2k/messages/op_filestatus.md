# Ficha de Especificación: `OP_FILESTATUS` (`0x50`)

## 1. Visión de Conjunto

El mensaje `OP_FILESTATUS` lo emite un par eD2K en respuesta a una petición `OP_FILESTATUSREQ`. Anuncia el número total de partes que constituyen el archivo así como una máscara binaria (bitfield) que indica qué partes posee y comparte ese par.

En la arquitectura eMuleRust, este mensaje desempeña dos roles:

- **Producción (`peer_session`)**: cada `FILESTATUS` recibido — respuesta inicial *y* keep-alive en cola de espera — registra la máscara en el almacén de disponibilidad (`record_source_availability`) y dispara `downloadUpdated`. Es la fuente única de la barra de partes (`availableSourceCount` → degradado rojo/azul al estilo `CPartFile::DrawStatusBar`): solo cuentan las fuentes **que han respondido** (eMule `GetPartStatus() != NULL`), las máscaras caducadas > 30 min se ignoran, y `part_count == 0` se trata como **fuente completa** (eMule `ProcessFileStatus` → `memset(m_abyPartStatus, 1, …)`).
- **Lab / control (`Ed2kSession`)**: la máquina de estados dedicada permanece estrictamente de observación en bucle local — alimenta las estadísticas de disponibilidad (`sourceCount = 1`) sin iniciar nunca una petición de bloque (`OP_REQUESTPARTS`), sin reservar búfer, y sin abrir ni escribir en un archivo `.part`.

---

## 2. Propiedades del Protocolo

| Propiedad | Valor |
| :--- | :--- |
| **Opcode Hexadecimal** | `0x50` |
| **Nombre Simbólico** | `OP_FILESTATUS` |
| **Protocolo de Red** | `PR_ED2K` (`0xE3`) |
| **Dirección** | Par remoto $\rightarrow$ Cliente |
| **Estado de Ejecución** | Producción (`peer_session`, disponibilidad de partes) + loopback control-only (`Ed2kSession`, Fase 6.3) |
| **Transferencia de Datos Asociada** | Ninguna (metadato puro) |

---

## 3. Wire Format (Formato en el Cable)

```text
Offset  Type          Champ                  Description
0       u8            Opcode                 0x50 (OP_FILESTATUS)
1       [u8; 16]      File Hash              Huella MD4 (16 bytes) del archivo en cuestión
17      u16 LE        Part Count             Número total de partes del archivo (9 728 000 bytes / parte)
19      u8[M]         Availability Bitmask   Máscara de bits: M = ceil(Part Count / 8) bytes
```

### 3.1 Cálculo de la Longitud de la Máscara
Para un número de partes $N = \text{part\_count}$:
$$\text{expected\_mask\_bytes} = \left\lceil \frac{N}{8} \right\rceil = \text{usize::from}(N).\text{div\_ceil}(8)$$

- Si $N = 0$, la máscara esperada tiene 0 bytes (máscara vacía).
- Si $1 \le N \le 8$, la máscara esperada tiene 1 byte.
- Si $N = 12$, la máscara esperada tiene 2 bytes.
- Si $N = 53$, la máscara esperada tiene 7 bytes.

### 3.2 Orden de los Bits en cada Byte (LSB-First)
En el protocolo estándar eDonkey2000 / eMule, las partes están indexadas a partir de 0 y dispuestas según el orden **LSB-first (Least Significant Bit first)** dentro de cada byte:
- La parte $i$ ($0 \le i < N$) se localiza en:
  - **Byte**: $i / 8$ (índice base 0 en la máscara binaria)
  - **Bit**: $i \pmod 8$ (bit de peso $2^{i \pmod 8}$)
- Fórmula de verificación:
  $$\text{has\_part}(i) = (\text{raw\_bytes}[i / 8] \ \& \ (1 \ll (i \pmod 8))) \neq 0$$

### 3.3 Regla Estricta de los Bits No Utilizados (Trailing Bits)
Cuando $N \pmod 8 \neq 0$, el último byte de la máscara contiene $8 - (N \pmod 8)$ bits sin asignar en sus posiciones de mayor peso.
- **Política eMuleRust**: **Rechazo Estricto**. Todos los bits más allá del índice $(N \pmod 8) - 1$ en el último byte **deben ser obligatoriamente nulos** (`0`).
- Si un bit no utilizado está puesto a 1, el mensaje se rechaza inmediatamente con el error `Ed2kErrorKind::InvalidBitfieldTrailingBits`.

---

## 4. Precondiciones de Estado y Reglas de Sesión

1. **Estado Requerido**: La sesión debe estar en el estado `Ready { peer }`. Cualquier estado recibido fuera de este estado se rechaza (`Ed2kSessionError::ResponseBeforeReady`).
2. **Correlación Estricta con una Petición Pending**:
   - El `file_hash` recibido debe corresponder exactamente a una petición en espera en el `FileStatusTracker`.
   - Si no hay ninguna petición en espera para ese hash, la respuesta se rechaza como no solicitada (`Ed2kSessionError::UnsolicitedFileStatus`).
   - Una respuesta válida retira inmediatamente la petición del tracker. Una segunda respuesta para el mismo hash se rechaza por tanto como duplicado no solicitado.
3. **Tope de Partes**:
   - `part_count` no debe superar el tope de seguridad configurado (`limits.max_parts`, máximo 65 535). Cualquier superación provoca `Ed2kErrorKind::MalformedField`.
4. **Caso Particular `part_count == 0`**:
   - Decodificación pasiva: aceptado para observación histórica (devuelve una máscara vacía).
   - Sesión lab (`Ed2kSession`): rechazado con `Ed2kSessionError::ZeroPartCountInActiveSession` porque un archivo real eD2K compartido posee al menos una parte.
   - **Producción (`peer_session`)**: interpretado como **fuente completa** — eMule `ProcessFileStatus` materializa una máscara llena (`memset(m_abyPartStatus, 1, …)`); `compute_part_summaries` cuenta una fuente así para **todas** las partes (`availableSourceCount`).

---

## 5. Decodificación y Gestión de Errores

| Condición Detectada | Error Tipado | Acción |
| :--- | :--- | :--- |
| Trama más corta que $19 + \lceil N/8 \rceil$ | `Ed2kErrorKind::UnexpectedEof` | Fallo de trama |
| Trama con bytes más allá de la máscara | `Ed2kErrorKind::MalformedField` | Rechazo de trama |
| $N > \text{limits.max\_parts}$ | `Ed2kErrorKind::MalformedField` | Rechazo de trama |
| Bit residual a 1 en el último byte | `Ed2kErrorKind::InvalidBitfieldTrailingBits` | Rechazo de trama |
| Hash MD4 no pedido o ya resuelto | `Ed2kSessionError::UnsolicitedFileStatus` | Rechazo de sesión |
| Estado recibido antes de completar el handshake | `Ed2kSessionError::ResponseBeforeReady` | Rechazo de sesión |
| Protocolo comprimido 0xD4 | `Ed2kSessionError::CompressedProtocolUnsupported` | Rechazo de sesión |

---

## 6. Fixtures de Prueba

- **`file_status.bin`**: Trama de 26 bytes (part_count = 12, bitfield `ff0f`, partes 0..=11 disponibles).
- **`file_status_multi_parts.bin`**: Trama de 27 bytes (part_count = 10, bitfield `8303`, partes 0, 1, 7, 8, 9 disponibles).
  - Byte 0: `0x83` = `0b1000_0011` (bits 0, 1, 7)
  - Byte 1: `0x03` = `0b0000_0011` (bits 0, 1 = partes 8, 9; bits 2..=7 nulos).
  - SHA-256 registrado en `manifest.json`.

---

## 7. Referencias Históricas

- **eMule 0.72a**: envío inline en `srchybrid/ListenSocket.cpp:404` (respuesta `OP_FILESTATUS` a `OP_SETREQFILEID`), recepción `srchybrid/DownloadClient.cpp:509`: `CUpDownClient::ProcessFileStatus()`
- **eMule 0.50a** *(histórico)*: `srchybrid/UpDownClient.cpp`: `CUpDownClient::SendFileStatus()`, `CUpDownClient::ProcessFileStatus()`
  ```cpp
  Packet* packet = new Packet(OP_FILESTATUS, 16 + 2 + reqfile->GetPartCount());
  // Rellena el hash MD4, partCount en uint16 LE, y el bitfield
  ```
- **aMule 2.3.3**: `src/UploadClient.cpp`: `CUploadClient::SendFileStatus()`

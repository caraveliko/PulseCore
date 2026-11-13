# Ficha de Especificación: `OP_FILESTATUSREQ` (`0x49`)

## 1. Visión de Conjunto

El mensaje `OP_FILESTATUSREQ` permite a un cliente eD2K consultar a un par remoto sobre su posesión y disponibilidad de las distintas partes (bloques de 9 728 000 bytes) de un archivo identificado por su hash MD4 de 16 bytes.

En la arquitectura eMuleRust, este mensaje se **utiliza exclusivamente para el descubrimiento de disponibilidad en bucle local cerrado (loopback)**. No desencadena ninguna transferencia de datos, ninguna reserva de slot, ni ninguna escritura en un archivo `.part`.

---

## 2. Propiedades del Protocolo

| Propiedad | Valor |
| :--- | :--- |
| **Opcode Hexadecimal** | `0x4F` (Canónico eMule `OP_SETREQFILEID`), `0x49` (Histórico alternativo `OP_FILESTATUSREQ`) |
| **Nombre Simbólico** | `OP_SETREQFILEID` / `OP_FILESTATUSREQ` |
| **Protocolo de Red** | `PR_ED2K` (`0xE3`) |
| **Dirección** | Cliente $\rightarrow$ Par remoto |
| **Estado de Ejecución** | Loopback control-only (Fase 6.3 / 6.5) |
| **Transferencia de Datos Asociada** | Ninguna (disponibilidad pura) |

---

## 3. Wire Format (Formato en el Cable)

El formato es estrictamente determinista y de longitud fija:

```text
Offset  Type          Champ                  Description
0       u8            Opcode                 0x4F (canónico eMule) o 0x49 (histórico)
1       [u8; 16]      File Hash              Huella MD4 (16 bytes) del archivo requerido
```

- **Tamaño del payload bruto**: exactamente 17 bytes (1 byte de opcode + 16 bytes de hash).
- **Tamaño de la trama encapsulada**: 22 bytes (1 byte protocolo `0xE3` + 4 bytes de tamaño LE `0x11, 0x00, 0x00, 0x00` + 17 bytes de payload).

---

## 4. Precondiciones de Estado y Máquina de Estados

1. **Estado Requerido**: La sesión debe estar estrictamente en el estado `Ready { peer }`.
   - Si se intenta emitir `OP_FILESTATUSREQ` antes del estado `Ready` (p. ej. durante `Connecting`, `AwaitingHello` o `AwaitingHelloAnswer`), la sesión rechaza la operación con el error `Ed2kSessionError::RequestBeforeReady`.
2. **Correlación de Peticiones en Vuelo**:
   - Cada petición se registra en el `FileStatusTracker` con su instante de envío (`sent_at: Instant`).
   - La emisión de una segunda petición para un hash ya en espera ante el mismo par se rechaza con `Ed2kSessionError::DuplicateFileStatusRequest`.
   - El número máximo de peticiones en vuelo por par está acotado (tope configurable, por defecto: 16) para evitar el agotamiento de memoria. Cualquier superación genera `Ed2kSessionError::TooManyPendingRequests`.

---

## 5. Decodificación y Gestión de Errores

- **EOF Inesperado (Trama truncada)**: Si el payload tiene menos de 17 bytes, retorno de error `Ed2kErrorKind::UnexpectedEof { needed: 17, offset: ... }`.
- **Bytes Superfluos (Trama demasiado larga)**: Cualquier byte presente más allá de los 16 bytes de hash se considera una anomalía de protocolo y se rechaza con `Ed2kErrorKind::MalformedField`.
- **Hash Nulo**: Un hash compuesto únicamente de ceros `[0u8; 16]` se rechaza como no válido.

---

## 6. Fixtures de Prueba

- **`file_status_req.bin`**: Trama binaria válida de 22 bytes con hash `0xCC` repetido 16 veces.
  - SHA-256 registrado en `manifest.json`.
  - Procedencia: generado según la especificación formal eD2K.

---

## 7. Referencias Históricas

- **eMule 0.72a**: `CUpDownClient::SendFileStatusRequest()` ya no existe — `0x49` es allí `OP_END_OF_DOWNLOAD` (`opcodes.h:215`). El estado del archivo se obtiene mediante `OP_SETREQFILEID`, respuesta `OP_FILESTATUS` (`ListenSocket.cpp:404`), tratada en `DownloadClient.cpp:509` (`CUpDownClient::ProcessFileStatus`).
- **eMule 0.50a** *(histórico)*: `srchybrid/UpDownClient.cpp`: `CUpDownClient::SendFileStatusRequest()`
  ```cpp
  Packet* packet = new Packet(OP_FILESTATUSREQ, 16);
  md4cpy(packet->pBuffer, reqfile->GetFileHash());
  SendPacket(packet, true);
  ```
- **aMule 2.3.3**: `src/DownloadClient.cpp`: `CDownloadClient::SendFileStatusRequest()`

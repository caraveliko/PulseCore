# Ficha de Especificación: `OP_SENDINGPART` (`0x46`)

## 1. Visión de Conjunto

El mensaje `OP_SENDINGPART` transporta un bloque de datos binarios de archivo desde un par remoto hacia el cliente receptor, en respuesta a una petición previa formulada mediante `OP_REQUESTPARTS`.

En la arquitectura eMuleRust:
- Este mensaje está **estrictamente correlacionado** con una petición en vuelo registrada en `BlockRequestTracker`.
- Los bytes recibidos se validan y luego se escriben exclusivamente a través de la abstracción de almacenamiento `DownloadSink`.
- El tamaño del archivo físico `.part` y el cálculo del hash eD2K progresan de forma determinista y atómica mediante SQLite WAL.

---

## 2. Propiedades del Protocolo

| Propiedad | Valor |
| :--- | :--- |
| **Opcode Hexadecimal** | `0x46` |
| **Nombre Simbólico** | `OP_SENDINGPART` |
| **Protocolo de Red** | `PR_ED2K` (`0xE3`) |
| **Dirección** | Par remoto $\rightarrow$ Cliente |
| **Estado de Ejecución** | Loopback transfer-only (Fase 6.4) |
| **Tamaño Máximo de Trama** | 262 144 bytes (256 KiB) |
| **Tamaño Estándar de Bloque** | $\le$ 184 320 bytes (180 KiB) |

---

## 3. Wire Format (Formato en el Cable)

```text
Offset  Type          Champ                  Description
0       u8            Opcode                 0x46 (OP_SENDINGPART)
1       [u8; 16]      File Hash              Huella MD4 (16 bytes) del archivo transferido
17      u32 LE        Block Start Offset     Desplazamiento de inicio del bloque en el archivo (u32 little-endian)
21      u32 LE        Block End Offset       Desplazamiento de fin del bloque en el archivo (u32 little-endian)
25      u8[...]       Payload Data           Bytes brutos del bloque (longitud = End Offset - Start Offset)
```

- **Tamaño de la cabecera de carga útil**: 24 bytes (16 bytes de hash + 4 bytes start + 4 bytes end).
- **Tamaño total de la trama**: `5 (cabecera framing) + 1 (opcode) + 24 (cabecera block) + data.len() = 30 + data.len()` bytes.
- **Verificación de igualdad estricta**: El número de bytes de datos restantes en la trama debe corresponder **exactamente** a `end_offset - start_offset`. Cualquier desviación dispara el error `BlockPayloadMismatch`.

---

## 4. Reglas de Decodificación y Rechazos de Seguridad

1. **Trama Truncada**: Si la carga útil tiene menos de 24 bytes, error `UnexpectedEof`.
2. **Orden de Desplazamiento No Válido**: Si `start_offset >= end_offset`, rechazo inmediato con `InvalidBlockOffsets`.
3. **Discordancia de Tamaño**: Si `(end_offset - start_offset) as usize != data.len()`, rechazo inmediato con `BlockPayloadMismatch`.
4. **Desbordamiento de Capacidad de Trama**: Si la trama supera `ED2K_MAX_FRAME_DEFAULT` (256 KiB) o si el bloque supera 184 320 bytes, rechazo con `FrameTooLarge`.
5. **Bloque No Solicitado**: Si ningún bloque correspondiente a `(file_hash, start, end)` está en vuelo en el tracker de sesión, rechazo inmediato con `UnsolicitedBlock`.
6. **Bloque Duplicado**: Una segunda entrega para un rango ya tratado se ignora y no incrementa el progreso.
7. **Trama Comprimida 0xD4**: Rechazo inmediato con `CompressedProtocolUnsupported`.

---

## 5. Integración `DownloadSink` y Persistencia

Tras la validación por la sesión y el `BlockRequestTracker`:
1. El bloque se convierte en `ReceivedBlock { file_hash, start_offset: start as u64, data }`.
2. Se transmite a `DownloadSink::write_block()`.
3. La escritura física se realiza en el archivo `.part` temporal.
4. El intervalo se registra en `RangeSet` y se valida en SQLite (WAL).
5. Se notifica el evento reactivo `event.downloadUpdated` a los clientes locales.

---

## 6. Referencias Históricas
- eMule 0.72a: `srchybrid/ListenSocket.cpp:573`: `CListenSocket::ProcessPacket()` (case `OP_SENDINGPART`)
- eMule 0.72a: `srchybrid/DownloadClient.cpp:956`: `CUpDownClient::ProcessBlockPacket()`
- aMule: `src/DownloadClient.cpp:CDownloadClient::ProcessBlockPacket()`

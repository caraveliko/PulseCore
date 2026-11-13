# Matriz de Opcodes y Protocolos eD2K / eMule

## 1. Contexto e Histórico de las Variantes

El protocolo eDonkey2000 original y sus extensiones sucesivas (eMule, aMule, cDonkey, MLDonkey) introdujeron evoluciones, reasignaciones y formatos alternativos según las generaciones de software y los bytes de protocolo (`0xE3`, `0xC5`, `0xD4`).

Esta matriz formaliza las atribuciones exactas, las direcciones, los formatos en el cable y el nivel de soporte en **eMuleRust**, basándose en las fuentes de referencia versionadas:
- **eMule 0.72a** *(referencia de verdad actual)*: `D:\eMule0.72a-Sources\eMule0.72a-Sources\srchybrid` (`opcodes.h`, `DownloadClient.cpp`, `ListenSocket.cpp`, `UploadClient.cpp`, `kademlia/`).
- **eMule 0.50a** *(referencias históricas citadas en las tablas)*: `SomeSupport/eMule` (`srchybrid/opcodes.h`, `DownloadClient.cpp`, `ListenSocket.cpp`, `UploadClient.cpp`).
- **aMule 2.3.3**: `src/DownloadClient.cpp`, `src/UploadClient.cpp`.
- **eDonkey2000**: Especificaciones históricas obtenidas por ingeniería inversa del protocolo pasivo v1.

---

## 2. Matriz Comparativa de los Opcodes y Formatos Wire

| Mensaje Simbólico | Variante y Fuente | Protocolo Wire | Opcode Wire | Dirección | Estructura y Layout Payload | Soporte eMuleRust | Regla / Invariante Asociado |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **`OP_HELLO`** | Estándar eD2K y eMule 0.72a | `PR_ED2K` (`0xE3`) | `0x01` | Cliente $\rightarrow$ Par | Hash (16B), ClientID (4B LE), TCP Port (2B LE), Tag count (4B LE), Tags, Server IP (4B LE), Server Port (2B LE) | **Completo** | Rechazo si hash nulo o versión incompatible |
| **`OP_HELLO_ANSWER`** | Estándar eD2K y eMule 0.72a | `PR_ED2K` (`0xE3`) | `0x4C` | Par $\rightarrow$ Cliente | Hash (16B), ClientID (4B LE), TCP Port (2B LE), Tag count (4B LE), Tags, Server IP (4B LE), Server Port (2B LE) | **Completo** | Preservación de los tags desconocidos sin deserialización insegura |
| **`OP_SETREQFILEID`** | eMule 0.72a (`opcodes.h:221`) y aMule 2.3.3 | `PR_ED2K` (`0xE3`) | `0x4F` | Cliente $\rightarrow$ Fuente | Hash MD4 (16B). Desencadena el envío de `OP_FILESTATUS` (`0x50`) por la fuente. | **Completo** | Formato canónico 17B (opcode + hash). Sin acceso a disco. |
| **`OP_FILESTATUSREQ`** | eDonkey legacy / docs alternativos | `PR_ED2K` (`0xE3`) | `0x49` | Cliente $\rightarrow$ Fuente | Hash MD4 (16B). *(En eMule 0.72a, 0x49 = `OP_END_OF_DOWNLOAD` y 0x4D = `OP_CHANGE_CLIENT_ID`, `opcodes.h:215,219`).* | **Completo (compatibilidad)** | Decodificado y codificado de manera transparente como `FileStatusReq`. |
| **`OP_FILESTATUS`** | Estándar eD2K y eMule 0.72a | `PR_ED2K` (`0xE3`) | `0x50` | Fuente $\rightarrow$ Cliente | Hash MD4 (16B), Part count (2B LE), Bitfield LSB-first ($\lceil part\_count / 8 \rceil$ bytes). | **Completo** | Validación estricta de los bits finales residuales (deben valer 0). `part_count == 0` = fuente completa; alimenta `availableSourceCount` (barra de partes) mediante `record_source_availability`, inicial + keep-alive en cola. |
| **`OP_REQUESTPARTS`** | eMule 0.72a (`opcodes.h:213`) y aMule 2.3.3 | `PR_ED2K` (`0xE3`) | `0x47` | Cliente $\rightarrow$ Fuente | Hash MD4 (16B), 3× u32 LE start, 3× u32 LE end exclusivo (`endpos = block->EndOffset + 1`). Centinelas `(0, 0)` o `(u32::MAX, u32::MAX)`. (40B total). | **Completo** | Rastreado en `BlockRequestTracker` (máx. 8 en vuelo, cero solapamiento). |
| **`OP_REQUESTPARTS`** | eDonkey legacy / docs históricos | `PR_ED2K` (`0xE3`) | `0x4B` | Cliente $\rightarrow$ Fuente | 40B (3 slots) o 24B (1 slot: hash 16B + start 4B + end 4B). *(En eMule 0.72a, 0x4B = `OP_ASKSHAREDFILESANSWER`, `opcodes.h:217`).* | **Completo (compatibilidad)** | Decodificado de manera transparente en ambos formatos 24B y 40B. |
| **`OP_SENDINGPART`** | Estándar eD2K y eMule 0.72a | `PR_ED2K` (`0xE3`) | `0x46` | Fuente $\rightarrow$ Cliente | Hash MD4 (16B), start (4B LE), end (4B LE exclusivo), chunk bytes (`end - start` bytes, máx. 180 KiB). | **Completo** | Escritura exclusiva mediante `DownloadSink` en `.part` bajo bloqueo WAL. |
| **`OP_COMPRESSEDPART`** | Extensión eMule zlib | `PR_EMULE` (`0xC5`) o `PR_EMULE_COMPRESSED` (`0xD4`) | `0x40` (`0xA1` variante I64) | Fuente $\rightarrow$ Cliente | Hash MD4 (16B), start (4B/8B LE), tamaño comprimido total (4B LE), chunk zlib ≤ 10 240 B (enviado bajo `OP_EMULEPROT` en eMule). | **Completo (bidireccional)** | Decodificación incremental `flate2` lado descarga; emisión zlib `Z_BEST_SPEED` lado subida si `nDataCompVer` (MiscOptions1 bits 20-23); alternativa `SENDINGPART` si no es rentable. |
| **`OP_HASHSETREQUEST`** | eMule 0.72a (*DEPRECATED* `opcodes.h:223`, reemplazado por `OP_HASHSETREQUEST2` `0xB1`) | `PR_ED2K` (`0xE3`) | `0x51` | Cliente $\rightarrow$ Fuente | Hash MD4 (16B). Desencadena `OP_HASHSETANSWER`. | **Completo** | Enviado al conectar si el hashset es desconocido; servido para archivos compartidos con verificación del hash raíz. |
| **`OP_HASHSETANSWER`** | eMule 0.72a (*DEPRECATED* `opcodes.h:224`, reemplazado por `OP_HASHSETANSWER2` `0xB2`) | `PR_ED2K` (`0xE3`) | `0x52` | Fuente $\rightarrow$ Cliente | Hash MD4 (16B), `u16` count, N× hash MD4 por parte. | **Completo** | Persistido (`file_part_hashes`); verificación incremental y desalojo de las partes corruptas. |
| **`OP_AICHFILEHASHREQ` / `RES`** | eMule | `PR_EMULE` (`0xC5`) | `0x9E` / `0x9D` | Cliente $\leftrightarrow$ Fuente | Req: hash16. Res: hash16 + raíz SHA-1 (20B). | **Completo** | Consenso de confianza `CAICHUntrustedHash` (≥10 IP únicas enmascaradas, ≥92 %). |
| **`OP_AICHREQUEST` / `ANSWER`** | eMule | `PR_EMULE` (`0xC5`) | `0x9B` / `0x9C` | Cliente $\leftrightarrow$ Fuente | Req: hash16 + `u16` part + master20. Ans: entradas `(ident, hash)` 16/32-bit o hash16 solo (fallo). | **Completo** | Recuperación a nivel de bloque 180 KiB sobre árbol de Merkle de doble granularidad. |
| **`OP_OFFERFILES`** | eD2K servidor | `PR_ED2K` (`0xE3`) | `0x15` | Cliente $\rightarrow$ Servidor | `u32` count + por archivo hash16 + `u32` client_id + `u16` port + tags. | **Completo** | Semántica `CreateOfferedFilePacket`: marcadores compresión, HighID/LowID, tags `FT_FILESIZE_HI` > 4 GiB, keep-alive vacío, tope 200. |
| **`OP_CALLBACKREQUEST`** | eD2K servidor | `PR_ED2K` (`0xE3`) | `0x1C` | Cliente $\rightarrow$ Servidor | `u32` client_id desnudo. | **Completo** | Fuentes LowID `lowid:<id>` llamadas mediante relevo servidor; adopción de las conexiones entrantes correspondientes. |
| **`OP_CALLBACKREQUESTED` / `FAIL`** | eD2K servidor | `PR_ED2K` (`0xE3`) | `0x35` / `0x36` | Servidor $\rightarrow$ Cliente | `IP4 + port2` (+ opcional `crypt u8 + userhash16`). | **Completo** | `CALLBACKREQUESTED` desencadena una conexión saliente (`connect_back` mediante `InboundListenerService`, handshake eD2K completo y luego servicio de peticiones); `CALLBACK_FAIL` registrado. |
| **`OP_QUEUERANKING`** | eMule 0.72a | `PR_EMULE` (`0xC5`) | `0x60` | Fuente $\rightarrow$ Cliente | Exactamente 12 bytes: `u16` rango + 10 bytes reservados (`ProcessEmuleQueueRank`). | **Completo (codec)** | Decodificado/codificado; propagación de sesión por conectar en el lado daemon. |
| **`OP_QUEUERANK`** | eD2K legacy | `PR_ED2K` (`0xE3`) | `0x5C` | Fuente $\rightarrow$ Cliente | `u32` rango de cola de espera. | **Completo (codec)** | Forma legacy del rango de cola. |
| **`OP_CHANGE_SLOT`** | eD2K legacy | `PR_ED2K` (`0xE3`) | `0x5B` | Bidireccional | `hash16` — tolerado/ignorado por eMule 0.72a. | **Completo (codec)** | Decodificador estricto 16 bytes. |
| **`OP_MESSAGE`** | eD2K | `PR_ED2K` (`0xE3`) | `0x4E` | Bidireccional | `u16` len + mensaje; `len` debe consumir el payload exactamente. | **Completo (codec)** | Chat par-a-par. |
| **`OP_FILEDESC`** | eMule 0.72a | `PR_EMULE` (`0xC5`) | `0x61` | Fuente $\rightarrow$ Cliente | `u8` rating + `u16` len + descripción. | **Completo (codec)** | Valoración/comentario de archivo. |
| **`OP_PORTTEST`** | eD2K | `PR_ED2K` (`0xE3`) | `0xFE` | Bidireccional | `u8` estado (eMule responde `0x12`). | **Completo (codec)** | Prueba de conectividad TCP. |
| **`OP_CHATCAPTCHAREQ` / `RES`** | eMule 0.72a | `PR_EMULE` (`0xC5`) | `0xA5` / `0xA6` | Bidireccional | Req: `u8` tag_count + tags + bitmap BMP (128–4096 B en eMule). Res: `u8` estado. | **Completo (codec)** | Antispam chat; cotas bitmap por aplicar en el lado daemon. |
| **`OP_HASHSETREQUEST2` / `ANSWER2`** | eMule 0.72a (`opcodes.h:288-289`) | `PR_EMULE` (`0xC5`) | `0xB1` / `0xB2` | Cliente $\leftrightarrow$ Fuente | `<FileIdentifier>` + `u8` opciones (bit0 MD4, bit1 AICH); respuesta: `FileIdentifier` + opciones + hashsets (`hash16+u16+N×16` / `hash20+u16+N×20`). | **Completo (codec)** | Reemplaza `0x51`/`0x52` deprecated; opciones vacías rechazadas como eMule. |
| **`OP_MULTIPACKET` / `EXT` / `EXT2`** | eMule 0.72a (`0x92`/`0xA4` deprecated, `0xA9` activo) | `PR_EMULE` (`0xC5`) | `0x92` / `0xA4` / `0xA9` | Cliente $\rightarrow$ Fuente | Referencia de archivo (`hash16` / `hash16+u64` / `FileIdentifier`) + sub-opcodes `0x58`(+ext-info condicional a `ExtendedRequestsVersion`)/`0x4F`/`0x81`/`0x83`(+ver u8+opts u16)/`0x9E`. | **Completo (codec)** | `DownloadClient.cpp:259-345`. |
| **`OP_MULTIPACKETANSWER` / `EXT2`** | eMule 0.72a | `PR_EMULE` (`0xC5`) | `0x93` / `0xB0` | Fuente $\rightarrow$ Cliente | Mismo envoltorio; subrespuestas `0x59`(nombre)/`0x50`(estado sin hash)/`0x9D`(raíz AICH 20B). | **Completo (codec)** | `ListenSocket.cpp:1030-1101`. |
| **`OP_PEERCACHE_*`** | eMule (defunct 0.72a) | `PR_EMULE` (`0xC5`) | `0x94`–`0x96` | Bidireccional | Opaco — ignorados por eMule 0.72a. | **Completo (raw)** | Decodificados en `RawPacket`, ignorados aguas arriba. |
| **`OP_CANCELTRANSFER` / `OP_OUTOFPARTS`** | eD2K | `PR_ED2K` (`0xE3`) | `0x56` / `0x57` | Bidireccional | Payload vacío. | **Completo** | Liberación inmediata del slot de subida; fin de sesión elegante; emitido al cancelar y al finalizar peticiones. |
| **`OP_REASKFILEPING` / `REASKACK` / `FILENOTFOUND` / `QUEUEFULL`** | Cliente UDP eMule (`ClientUDPSocket`) | `PR_EMULE` (`0xC5`) sobre UDP | `0x90`–`0x93` | Bidireccional | Ping: `hash16` + `u16` status + `u16` complete. Ack: `[bitfield]` + `u16` rango. FNF/Full: vacío. | **Completo** | Socket UDP compartido con Kad; keep-alive ~21 min hacia fuentes `Queued`; rango validado frente a reasks pendientes; `QUEUEFULL` si cola casi llena. |
| **`OP_SERVERLIST`** | eD2K servidor | `PR_ED2K` (`0xE3`) | `0x32` | Servidor $\rightarrow$ Cliente | `u8` count + `count × (u32 ip_le + u16 port)`. | **Completo** | Fusionado en `ed2k_servers` (upsert deduplicado, preferencia 0); entradas no válidas `ip=0`/`port=0` rechazadas. |
| **`OP_GLOBGETSOURCES2` / `GLOBFOUNDSOURCES`** | Servidor UDP (`UDPSocket`) | `PR_ED2K` (`0xE3`) sobre UDP | `0x94` / `0x9B` | Cliente $\leftrightarrow$ Servidor | Req: `hash16 + u32 tamaño`. Res: bloques `hash16 + u8 count + count × (u32 ip + u16 port)`. | **Completo** | Socket UDP dedicado (bind efímero); peticiones hacia servidor conectado (UDP = TCP+4); fuentes persistidas como `PeerSource` (mismos filtros que TCP, LowID conservado). |
| **`OP_GLOBSERVSTATREQ` / `RES`** | Servidor UDP | `PR_ED2K` (`0xE3`) sobre UDP | `0x96` / `0x97` | Cliente $\leftrightarrow$ Servidor | Req: vacío. Res: `u32 users + u32 files`. | **Completo (codec)** | `RES` decodificado y registrado; `REQ` codificado pero no emitido periódicamente. |
| **`OP_GLOBSEARCHREQ` / `RES`** | Servidor UDP | `PR_ED2K` (`0xE3`) sobre UDP | `0x90`/`0x92`/`0x98` / `0x99` | Cliente $\leftrightarrow$ Servidor | Búsqueda global UDP (payload = expresión `OP_SEARCHREQUEST` verbatim; RES layout `OP_SEARCHRESULT`). | **Completo (REQ2)** | `servers.search` emite `GLOBSEARCHREQ2` hacia servidor conectado + servidores `allowed` (UDP = TCP+4); `GLOBSEARCHRES` decodificado y emitido en `server.searchResult`. `GLOBSEARCHREQ3` (tags newtags) no emitido. |
| **`OP_EMULE_INFO`** | Extensión eMule | `PR_EMULE` (`0xC5`) | `0x01` | Bidireccional | Handshake extendido de capacidades eMule. | **No requerido (Fase 6)** | Ignorado o reservado para negociaciones de funcionalidades futuras. |

---

## 3. Convención de las Cotas de Bloques en el Cable

### Fórmula eMule de Referencia (`DownloadClient.cpp`)
En la implementación oficial de eMule:
```cpp
// srchybrid/DownloadClient.cpp:812 — CUpDownClient::SendBlockRequests() (eMule 0.72a)
filedata->WriteUInt32(block->StartOffset);
filedata->WriteUInt32(block->EndOffset + 1);
```
La cota `EndOffset` en la memoria interna de eMule es inclusiva (`0..184319`), pero **el valor escrito en el cable se incrementa en 1 (`EndOffset + 1`)**, lo que la convierte en una cota **estrictamente exclusiva** en el cable:
$$\text{bytes\_count} = \text{wire\_end} - \text{wire\_start}$$

### Consideración en eMuleRust
- **Constructor canónico**: `Ed2kBlockRange::from_wire_bounds(start, end)` verifica `start < end` y transmite directamente el intervalo semiabierto `[start, end)`.
- **Compatibilidad legacy**: `Ed2kBlockRange::from_legacy_inclusive_wire_bounds(start, end_inclusive)` permite convertir fuentes documentales o clientes antiguos que transmiten una cota inclusiva (`end_inclusive.checked_add(1)`).
- **Bloque residual de fin de archivo**:
  Para un archivo de 9 728 001 bytes (`FIXTURE_ONE_PART_PLUS_ONE`), el último bloque es `[9728000, 9728001)`:
  - `start = 9_728_000`
  - `end = 9_728_001`
  - `len = 1` byte.

---

## 4. Niveles de Validación y Reproducibilidad

1. **Tier 1: Conformidad Sintáctica de Corpus (Sin Red)**:
   Verificación estática bit a bit frente al catálogo `manifest.json` con sumas de comprobación SHA-256 (`crates/mule-protocol/tests/ed2k_corpus_tests.rs`).
2. **Tier 2: Conformidad por Emulación de Referencia (`EmulatedReferencePeer`)**:
   Banco de pruebas en proceso sobre socket loopback `127.0.0.1:0` reproduciendo fielmente las reglas wire observadas en eMule 0.50a / aMule 2.3.3 (`crates/mule-protocol/tests/ed2k_reference_interop_tests.rs`).
3. **Tier 3: Interoperabilidad Observada contra Binario de Referencia Externo**:
   Arnés automatizado que controla un ejecutable compilado y fijado de eMule o aMule en bucle local con perfil de prueba efímero (definido en `reference_manifest.json`).

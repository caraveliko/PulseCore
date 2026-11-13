# Ficha de Especificación: Source Exchange (SX / SX2)

## 1. Visión de Conjunto

El Source Exchange permite a dos clientes eD2K/eMule intercambiar directamente
las direcciones de fuentes de un archivo, sin pasar por los servidores ni por Kad.
Es el principal mecanismo de distribución del enjambre: cada fuente contactada
redistribuye las fuentes que conoce.

Referencia de verdad: **eMule 0.72a** — `srchybrid/Opcodes.h:254-257`,
`ListenSocket.cpp:970-1011` (sub-opcodes multipacket) y `1165-1251`
(petición standalone), `PartFile.cpp:3602-3712` (`CreateSrcInfoPacket`),
`PartFile.cpp:3714-3890` (`AddClientSources`), `PartFile.cpp:2368-2405`
(`CanAddSource`), `DownloadClient.cpp:212-255` (`IsSourceRequestAllowed`).

---

## 2. Opcodes y Versionado

| Opcode | Nombre | Sentido | Descripción |
| :--- | :--- | :--- | :--- |
| `0x81` | `OP_REQUESTSOURCES` | req | Petición SX1: hash desnudo |
| `0x82` | `OP_ANSWERSOURCES` | resp | Respuesta SX1 (versiones 1-4 detectadas por tamaño de entrada) |
| `0x83` | `OP_REQUESTSOURCES2` | req | Petición SX2: `<ver><options><hash>` |
| `0x84` | `OP_ANSWERSOURCES2` | resp | Respuesta SX2: `<ver><hash><count>` |

- `SOURCEEXCHANGE2_VERSION = 4` (`Opcodes.h:254`).
- Protocolo: `PR_EMULE` (`0xC5`); respuestas comprimidas zlib en
  `PR_EMULE_COMPRESSED` (`0xD4`) cuando el payload supera ~354 bytes
  (`PackPacket`, `PartFile.cpp:3707-3708`).

### Negociación de la respuesta (`PartFile.cpp:3629-3646`)

- `bIsSX2Packet && byRequestedVersion > 0` → `OP_ANSWERSOURCES2` a
  `byUsedVersion = min(byRequestedVersion, 4)`.
- Si no → `OP_ANSWERSOURCES` (SX1) a `GetSourceExchange1Version()`
  (misc1 bits 12-15 o tag `ET_SOURCEEXCHANGE` `0x23` de `OP_EMULEINFO`).
- Responder SX2 v4 a un cliente SX1 es una desviación en el cable: descarta el paquete.

### Versión de la petición emitida

- El par anuncia SX2 (`SupportsSourceExchange2`, misc2 bit 10) →
  `OP_REQUESTSOURCES2` v4.
- Si no SX1 (`GetSourceExchange1Version > 1`) → `OP_REQUESTSOURCES`.

---

## 3. Wire Format

### `OP_REQUESTSOURCES` (`0x81`)

```text
Offset  Type          Champ       Description
0       [u8; 16]      File Hash   Huella MD4 del archivo
```

### `OP_REQUESTSOURCES2` (`0x83`)

```text
Offset  Type          Champ       Description
0       u8            Version     Versión SX pedida (típ. 4)
1       u16 LE        Options     Reservado (eMule envía 0)
3       [u8; 16]      File Hash   Huella MD4 del archivo
```

### `OP_ANSWERSOURCES` (`0x82`, SX1)

```text
0       [u8; 16]      File Hash
16      u16 LE        Count       Número de fuentes
18      Entry[N]      Sources     N entradas (tamaño depende de la versión)
```

La versión SX1 no va en el cable: eMule la deduce del tamaño
(`PartFile.cpp:3777-3827`) — 12 B/entrada → v1, 28 B → v3, 29 B → v4.
Cualquier otro tamaño residual → paquete descartado.

### `OP_ANSWERSOURCES2` (`0x84`, SX2)

```text
0       u8            Version     byUsedVersion (1-4, nunca 0 ni >4)
1       [u8; 16]      File Hash
17      u16 LE        Count
19      Entry[N]      Sources
```

### Entrada fuente

| Campo | Tamaño | Versiones |
| :--- | :--- | :--- |
| Peer ID / IP | 4 B | todas — network order si v<3, «hybrid» LE si v≥3 |
| TCP Port | 2 B LE | todas |
| Server IP | 4 B LE | todas |
| Server Port | 2 B LE | todas |
| User Hash | 16 B | v≥2 (SX1 v3) |
| Crypt options | 1 B | v=4 |

Tamaños: v1 = 12 B, v2/v3 = 28 B, v4 = 29 B.

Convención LowID: `id < 16 777 216` (`IsLowID`). Una entrada LowID designa
un `client_id` de servidor (campo ServerIP/Port = su servidor), no una IP.

---

## 4. Sub-opcodes `OP_MULTIPACKET*`

eMule empaqueta sus peticiones de archivo en `OP_MULTIPACKET` (`0x92`),
`OP_MULTIPACKET_EXT` (`0xA4`, hash + tamaño u64) u `OP_MULTIPACKET_EXT2`
(`0xA9`, `FileIdentifier`). El hash de archivo vive en el **envoltorio**;
los sub-opcodes `0x81` (sin payload) y `0x83` (`<ver u8><options u16>`,
sin hash) se refieren a él (`ListenSocket.cpp:970-1011`).

Sub-opcodes reconocidos (`decode_multipacket_request`):

| Opcode | Sub-request | Respuesta esperada |
| :--- | :--- | :--- |
| `0x58` | `FileName` (+ ext-info part-status si `ExtendedRequestsVersion > 0`) | `OP_REQFILENAME_ANSWER` + `OP_FILESTATUS` en multipacket-answer |
| `0x4F` | `SetReqFileId` | — |
| `0x81` | `RequestSources` | `OP_ANSWERSOURCES` (paquete separado) |
| `0x83` | `RequestSources2` | `OP_ANSWERSOURCES2` (paquete separado) |
| `0x9E` | `AichFileHash` | `OP_AICHFILEHASHRES` en multipacket-answer |

Las respuestas de archivo se agrupan en `OP_MULTIPACKETANSWER` (`0x93`) /
`OP_MULTIPACKETANSWER_EXT2` (`0xB0`); las respuestas SX salen en paquetes
independientes como eMule.

---

## 5. Filtrado en la ingesta (`AddClientSources` / `CanAddSource`)

Aplicado en `handle_sx_answer` (`mule-daemon/.../source_exchange.rs`):

- hash del archivo ≠ hash esperado → paquete ignorado;
- `version == 0` o `> 4` (SX2), tamaño `count × entry_size ≠ len` → descartar
  (`PartFile.cpp:3732-3770`);
- `user_hash == own_user_hash` → descartar (`DownloadQueue.cpp:479-484`);
- HighID: `port == 0`, IP no válida/LAN/loopback (`IsGoodIP`, aquí la
  política de red `mule-network-policy`, más estricta), `ipfilter` → descartar;
- self-check: `client_id` HighID + nuestro puerto, o LowID igual a nuestro
  `client_id` en nuestro servidor → descartar (`PartFile.cpp:2380-2403`);
- LowID cuando estamos tras cortafuegos (`IsFirewalled`) → descartar — el
  callback es irrealizable; si no persistido `lowid:<id>`;
- tope: parada en `GetMaxSources()` (`max_sources` del archivo, por defecto 400
  = `GetMaxSourcePerFileDefault`).

Las fuentes admitidas se persisten deduplicadas (`PRIMARY KEY
file_hash+address`, upsert preservando `user_hash`/`crypt_options`/
`fail_count`/`cooldown`) y luego son distribuidas por el swarm coordinator.

## 6. Selección de fuentes servidas (`CreateSrcInfoPacket`)

`select_sx_response_sources`:

- `!HasLowID() && IsValidSource()`: solo las fuentes verificadas
  (estados `Connected`/`Allowed`) con IP filtrable válida;
- `srcstatus`: máscara de disponibilidad presente y cuya longitud
  corresponde a `ceil(part_count/8)`;
- `reqstatus`: ≥ 1 parte que el solicitante no tiene (su `OP_FILESTATUS`
  o la ext-info multipacket `0x58`); si es desconocida, ≥ 1 parte cualquiera;
- `nCount > 500` → 501 entradas máximo;
- `!nCount` → **ninguna respuesta**; archivo desconocido (ni compartido, ni
  descarga) → silencio (`CheckFailedFileIdReqs`).

## 7. Throttles

| Temporizador | Valor | Rol |
| :--- | :--- | :--- |
| `SOURCECLIENTREASKS` | 40 min | cooldown petición por cliente (×4 = 160 min no raro) |
| `SOURCECLIENTREASKF` | 5 min | cooldown respuesta por archivo (×4 = 20 min no raro) |
| `MINCOMMONPENALTY` | 4 | multiplicador archivos no raros |
| `RARE_FILE` | 50 | umbral «archivo raro» (fuentes conocidas) |
| `CONNECTION_LATENCY` | 22,05 s | margen en las comparaciones de tiempo |

Lado **respuestas** (`SetLastSrcReqTime`, `ListenSocket.cpp:1204-1215`):
nunca respondido → OK; part-file raro (≤ 50 fuentes) → > 40 min;
si no → > 160 min.

Lado **peticiones** (`IsSourceRequestAllowed`, `DownloadClient.cpp:212-255`):
capacidad SX requerida, `GetSourceCount < GetMaxSourcePerFileSoft`
(`max_sources × 9/10`), cláusulas rara (`≤ RARE_FILE/5`, `≤ RARE_FILE`,
completitud del par excluida) y común (160 min + 20 min).

## 8. Re-ask periódico

eMule vincula `OP_REQUESTSOURCES` al ciclo `SendFileRequest` (reask
~29-40 min). eMuleRust emite la petición en los keep-alives de cola
de espera y cada minuto en el bucle de transferencia
(`maybe_send_sx_request`), únicamente si `IsSourceRequestAllowed` pasa
— luego `SetLastAskedForSources` + `SetLastAnsweredTimeTimeout` espejo.

## 9. Decodificación eMuleRust

- `decode_request_sources`: cotas estrictas (≥16 B / ≥19 B), opcode
  verificado, protocolos `PR_EMULE`/`PR_ED2K`.
- `decode_answer_sources`: versión SX2 ∈ [1, 4] (0 y >4 descartados como
  eMule), sniff SX1 por tamaño exacto, `count × entry_size == len` exigido,
  `count ≤ MAX_SX_SOURCES_PER_PACKET` (1000), endianness v<3 (network
  order → hybrid LE), descompresión zlib `PR_EMULE_COMPRESSED`.
- `decode_multipacket_request`: envoltorio `Hash`/`HashAndSize`/
  `FileIdentifier`, ≤ 32 sub-opcodes, ext-info `0x58` leído según
  `ExtendedRequestsVersion` negociada.
- `encode_answer_sources(ans, is_sx2, force_compress)`: opcode y diseño
  según la versión negociada; compresión solo si > 354 B salvo
  `force_compress`.

## 10. Referencias

- eMule 0.72a: `srchybrid/PartFile.cpp`, `ListenSocket.cpp`,
  `DownloadClient.cpp`, `Opcodes.h`, `DownloadQueue.cpp`.
- Implementación: `crates/mule-protocol/src/ed2k/source_exchange/` (codec,
  tipos, pruebas round-trip) y
  `crates/mule-daemon/src/services/peer_session/source_exchange.rs`
  (ingesta, servicio, throttles, pruebas).

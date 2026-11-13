# Catálogo de Opcodes Históricos eD2K y eMule

Este documento recopila los opcodes estándar del protocolo eDonkey2000 (`0xE3`), de las extensiones eMule (`0xC5`) y del protocolo comprimido (`0xD4`).

---

## 1. Protocolo eDonkey2000 Estándar (`PR_ED2K = 0xE3`)

Estos mensajes constituyen la base de interconexión par-a-par original definida por MetaMachine y extendida por eMule.

| Opcode Hex | Nombre Simbólico | Dirección | Rol y Semántica |
| :---: | :--- | :--- | :--- |
| `0x01` | `OP_HELLO` | Bidireccional | Apretón de manos inicial: anuncio del hash de usuario, puerto de escucha y capacidades |
| `0x05` | `OP_REJECT` | Servidor $\rightarrow$ Cliente | Rechazo del último comando (payload vacío — `(null)` en `Opcodes.h`); eMule se limita a registrarlo, el servidor cierra el socket justo después (`ServerSocket.cpp:572`) |
| `0x1C` | `OP_CALLBACKREQUEST` | Cliente $\rightarrow$ Servidor | Petición al servidor para retransmitir una invitación hacia un cliente LowID |
| `0x35` | `OP_CALLBACKREQUESTED` | Servidor $\rightarrow$ Cliente | Relevo servidor: IP + puerto (opcional: crypt options + user hash) de un cliente LowID que nos llama |
| `0x32` | `OP_SERVERLIST` | Servidor $\rightarrow$ Cliente | Servidores adicionales anunciados: `u8 count` y luego `count × (u32 ip_le + u16 port)` |
| `0x36` | `OP_CALLBACK_FAIL` | Servidor $\rightarrow$ Cliente | Fallo del relevo de callback LowID |
| `0x94` | `OP_GLOBGETSOURCES2` | Cliente $\rightarrow$ Servidor (UDP) | Petición UDP de fuentes: `hash16 + u32 tamaño` (puerto servidor = TCP+4) |
| `0x96` | `OP_GLOBSERVSTATREQ` | Cliente $\rightarrow$ Servidor (UDP) | Petición UDP de estadísticas de servidor (payload vacío) |
| `0x97` | `OP_GLOBSERVSTATRES` | Servidor $\rightarrow$ Cliente (UDP) | Estadísticas: `u32 users + u32 files` |
| `0x99` | `OP_GLOBSEARCHRES` | Servidor $\rightarrow$ Cliente (UDP) | Resultados de búsqueda global UDP (mismo diseño que `OP_SEARCHRESULT`) |
| `0x9B` | `OP_GLOBFOUNDSOURCES` | Servidor $\rightarrow$ Cliente (UDP) | Fuentes por archivo: bloques repetidos `hash16 + u8 count + count × (u32 ip + u16 port)` |
| `0x4C` | `OP_HELLOANSWER` | Par $\rightarrow$ Cliente | Respuesta al apretón de manos inicial con la información del par remoto |
| `0x15` | `OP_OFFERFILES` | Cliente $\rightarrow$ Servidor | Anuncio de archivos compartidos al servidor |
| `0x46` | `OP_SENDINGPART` | Par $\rightarrow$ Cliente | Transmisión de un bloque de datos de archivo (offset + payload) |
| `0x47` | `OP_REQUESTPARTS` (eMule) / `OP_SETREQFILEID` (eDonkey) | Cliente $\rightarrow$ Par | eMule: petición de bloques (3 slots, payload 40 B); eDonkey legacy: hash de archivo objetivo |
| `0x48` | `OP_FILEREQANSNOFILE`| Par $\rightarrow$ Cliente | Indica que el archivo solicitado no es o ya no es compartido por el par |
| `0x49` | `OP_END_OF_DOWNLOAD` (eMule) / `OP_FILESTATUSREQ` (eDonkey) | Cliente $\rightarrow$ Par | eMule: fin de descarga (`hash16`) — el cliente se retira de la cola de subida si el hash coincide (`ListenSocket.cpp:537`); eDonkey legacy: petición de estado de partes |
| `0x4B` | `OP_REQUESTPARTS` (eDonkey) / `OP_ASKSHAREDFILESANSWER` (eMule) | Cliente $\rightarrow$ Par | eDonkey: petición de bloques; eMule: respuesta «archivos compartidos» |
| `0x4D` | `OP_CHANGE_CLIENT_ID` (eMule) / `OP_END_OF_DOWNLOAD` (eDonkey) | Cliente $\rightarrow$ Par | eMule: cambio de ID cliente (`u32 newUserID + u32 newServerIP` — `ListenSocket.cpp:590`); eDonkey legacy: fin de descarga |
| `0x4F` | `OP_SETREQFILEID` (eMule) | Cliente $\rightarrow$ Par | Declaración del hash de archivo objetivo de la descarga — respuesta `OP_FILESTATUS` |
| `0x50` | `OP_FILESTATUS` | Par $\rightarrow$ Cliente | Envío de la máscara de disponibilidad de las partes del archivo |
| `0x51` | `OP_HASHSETREQUEST` | Cliente $\rightarrow$ Par | Petición de la lista de hashes de partes (hashset) para un archivo multiparte |
| `0x52` | `OP_HASHSETANSWER` | Par $\rightarrow$ Cliente | Respuesta que contiene la lista de hashes MD4 de 16 bytes de las partes |
| `0x54` | `OP_STARTUPLOADREQ` | Cliente $\rightarrow$ Par | Petición de entrada en la cola de espera de envío del par remoto |
| `0x55` | `OP_ACCEPTUPLOADREQ`| Par $\rightarrow$ Cliente | Aceptación: se asigna un slot de envío y la transferencia puede comenzar |
| `0x56` | `OP_CANCELTRANSFER` | Bidireccional | Cancelación de la transferencia o retirada de la cola de espera |
| `0x57` | `OP_OUTOFPARTS` | Cliente $\rightarrow$ Par | Indica que el cliente ya no busca partes en este par |
| `0x58` | `OP_REQFILENAME` | Cliente $\rightarrow$ Par | Petición del nombre de archivo asociado al hash |
| `0x59` | `OP_REQFILENAME_ANSWER` | Par $\rightarrow$ Cliente | Respuesta con el nombre de archivo |
| `0x4E` | `OP_MESSAGE` | Bidireccional | Chat par-a-par: `u16 len` + mensaje |
| `0x5B` | `OP_CHANGE_SLOT` | Bidireccional | Petición de cambio de slot (`hash16`) — tolerado/ignorado por eMule 0.72a |
| `0x5C` | `OP_QUEUERANK` | Par $\rightarrow$ Cliente | Rango de cola de espera `u32` (forma eD2K legacy) |
| `0xFE` | `OP_PORTTEST` | Bidireccional | Prueba de conectividad: `u8` estado — tratado también en el canal cliente-UDP (véase §2) |

---

## 2. Protocolo eMule Extendido (`PR_EMULE = 0xC5`)

Introducido por las versiones de eMule para añadir capacidades avanzadas (compresión, AICH, negociación de protocolo e intercambios de fuentes).

| Opcode Hex | Nombre Simbólico | Dirección | Rol y Semántica |
| :---: | :--- | :--- | :--- |
| `0x01` | `OP_EMULEINFO` | Bidireccional | Negociación de las funcionalidades modernas de eMule (versiones, opciones de red) |
| `0x02` | `OP_EMULEINFOANSWER` | Par $\rightarrow$ Cliente | Respuesta a la negociación de funcionalidades de eMule |
| `0x9B` | `OP_AICH_REQUEST` | Cliente $\rightarrow$ Par | Petición de datos de recuperación AICH de una parte corrupta (`hash16 + u16 part + master_hash20`) |
| `0x9C` | `OP_AICH_ANSWER` | Par $\rightarrow$ Cliente | Datos de recuperación: entradas `(ident, hash SHA-1)` de 16/32 bits — o `hash16` solo en caso de fallo |
| `0x9E` | `OP_AICHFILEHASHREQ` | Cliente $\rightarrow$ Par | Petición del hash raíz AICH (SHA-1) de un archivo |
| `0x9D` | `OP_AICHFILEHASHRES` | Par $\rightarrow$ Cliente | Respuesta: hash de archivo + hash raíz AICH (`hash16 + sha1_20`) |
| `0x40` | `OP_COMPRESSEDPART` | Par $\rightarrow$ Cliente | Bloque zlib (`hash16 + u32 start + u32 tamaño comprimido + chunk ≤ 10 240 B`), enviado bajo `OP_EMULEPROT` |
| `0x90` | `OP_REASKFILEPING` | Cliente $\rightarrow$ Par (UDP) | Keep-alive hacia una fuente en cola: `hash16` + info extendida (`u16` part-status + `u16` complete sources para UDP ver > 3) |
| `0x91` | `OP_REASKACK` | Par $\rightarrow$ Cliente (UDP) | Rango de cola de espera: `[u16 count + bitfield]` (UDP ver > 3) + `u16` rango |
| `0x92` | `OP_FILENOTFOUND` | Par $\rightarrow$ Cliente (UDP) | El archivo solicitado no está compartido (payload vacío) |
| `0x93` | `OP_QUEUEFULL` | Par $\rightarrow$ Cliente (UDP) | Cola de espera del par saturada (payload vacío) |
| `0x94` | `OP_REASKCALLBACKUDP` | Cliente $\rightarrow$ Par (UDP) | Re-ask retransmitido al buddy Kad de una fuente LowID: `buddy_id16 + file_hash16 + info extendida` — el relevo reescribe `[ip:4][port:2]` en los bytes 10–15 y transfiere en `OP_REASKCALLBACKTCP` (no confundir con `0x94` TCP = `OP_PEERCACHE_QUERY`, *defunct*) |
| `0x95` | `OP_DIRECTCALLBACKREQ` | Cliente $\rightarrow$ Par (UDP) | Petición de retorno directo hacia un par **Kad-firewalled**: `u16 tcp_port + userhash16 + u8 connect_options` — el receptor marca `tcp_port` en saliente (eMule `TryToConnect`, 1 petición/IP/180 s) |
| `0xFE` | `OP_PORTTEST` | Bidireccional (UDP) | Prueba de conectividad UDP: exactamente `u8 0x12` — eMule acusa recibo mediante su socket de prueba TCP dedicado (`SendPortTestReply('1')`), sin respuesta UDP |
| `0x21` | `OP_SOURCESREQUEST` | Cliente $\rightarrow$ Par | Petición de fuentes conocidas para un archivo (Source Exchange) |
| `0x22` | `OP_SOURCESANSWER` | Par $\rightarrow$ Cliente | Respuesta que contiene la lista de pares para un archivo |
| `0x60` | `OP_QUEUERANKING` | Par $\rightarrow$ Cliente | Posición en la cola de subida de eMule: exactamente 12 bytes (`u16` rango + 10 reservados, `ProcessEmuleQueueRank`) |
| `0x61` | `OP_FILEDESC` | Par $\rightarrow$ Cliente | Valoración/comentario del archivo: `u8` rating + `u16 len` + descripción |
| `0x92` | `OP_MULTIPACKET` | Cliente $\rightarrow$ Par | Petición agrupada (*deprecated* en 0.72a, reemplazada por `0xA9`): `hash16` + sub-opcodes |
| `0x93` | `OP_MULTIPACKETANSWER` | Par $\rightarrow$ Cliente | Respuesta agrupada: `hash16` + subrespuestas (`0x59` nombre, `0x50` estado, `0x9D` raíz AICH) |
| `0x94`–`0x96` | `OP_PEERCACHE_*` | Bidireccional | Caché de pares ofuscada (*defunct* en 0.72a — decodificados en raw, ignorados) |
| `0xA4` | `OP_MULTIPACKET_EXT` | Cliente $\rightarrow$ Par | Multipacket extendido (*deprecated* en 0.72a): `hash16 + u64 tamaño` + sub-opcodes |
| `0xA5`/`0xA6` | `OP_CHATCAPTCHAREQ`/`RES` | Bidireccional | Antispam de chat: petición (tags + bitmap BMP 128–4096 B) / respuesta `u8` estado |
| `0xA9` | `OP_MULTIPACKET_EXT2` | Cliente $\rightarrow$ Par | Multipacket activo 0.72a: `FileIdentifier` + sub-opcodes (`0x58`+ext-info, `0x4F`, `0x81`, `0x83`, `0x9E`) |
| `0xB0` | `OP_MULTIPACKETANSWER_EXT2` | Par $\rightarrow$ Cliente | Respuesta agrupada EXT2: `FileIdentifier` + subrespuestas |
| `0xB1`/`0xB2` | `OP_HASHSETREQUEST2`/`ANSWER2` | Cliente $\leftrightarrow$ Par | Hashsets mediante `FileIdentifier` + `u8` opciones (bit0 MD4, bit1 AICH) — reemplaza `0x51`/`0x52` deprecated |

> [!NOTE]
> La ext-info adjunta al sub-opcode `0x58` de los `MULTIPACKET*` depende de la
> `ExtendedRequestsVersion` **negociada** (`OP_EMULEINFO`): `0` = ausente,
> `1` = `u16` part count + bitfield, `>=2` = + `u16` complete sources.
> Soportada por `Ed2kDecoder::extended_requests_version` (por defecto `2`, eMule 0.72a).

---

## 3. Protocolo Comprimido (`PR_EMULE_COMPRESSED = 0xD4`)

| Opcode Hex | Nombre Simbólico | Dirección | Semántica |
| :---: | :--- | :--- | :--- |
| `0x40` | `OP_COMPRESSEDPART` | Par $\rightarrow$ Cliente | Bloque de datos comprimido con el algoritmo zlib/deflate |

> [!NOTE]
> La descompresión zlib `0xD4`/`OP_COMPRESSEDPART` ya está soportada en ambos sentidos: decodificación en el lado descarga (flujo incremental `flate2`) y emisión en el lado subida cuando el par anuncia `nDataCompVer` (MiscOptions1 bits 20-23) — con alternativa `OP_SENDINGPART` si la compresión no resulta rentable, fiel a `CreatePackedPackets`.

# Ficha de Especificación: `OP_END_OF_DOWNLOAD` (`0x49`) / `OP_CHANGE_CLIENT_ID` (`0x4D`)

Referencia de verdad: eMule 0.72a (`srchybrid/Opcodes.h`, `srchybrid/ListenSocket.cpp`).

## Problema de desambiguación

Dos parejas de opcodes comparten los mismos valores en el cable según el perfil del cliente:

| Valor | eMule 0.72a (`Opcodes.h`) | eDonkey2000 legacy |
| :--- | :--- | :--- |
| `0x49` | `OP_END_OF_DOWNLOAD` | `OP_FILESTATUSREQ` |
| `0x4D` | `OP_CHANGE_CLIENT_ID` | `OP_END_OF_DOWNLOAD` |

La resolución está dirigida exclusivamente por `Ed2kCompatibilityProfile` — nunca por heurística.

## `OP_END_OF_DOWNLOAD` — `0x49` (perfiles eMule/aMule)

### Layout

| Offset | Tipo | Campo | Notas |
| :--- | :--- | :--- | :--- |
| 0 | `u8` | Protocolo | `0xE3` (`PR_ED2K`) |
| 1 | `u32` LE | Longitud payload | `17` (`1 + 16`) |
| 5 | `u8` | Opcode | `0x49` |
| 6 | `[u8; 16]` | Hash archivo | MD4 eD2K del archivo terminado |

### Semántica eMule (`ListenSocket.cpp:537`)

- El **downloader** notifica al **uploader** que ha terminado el archivo.
- Recepción: `if (size >= 16 && md4equ(client->GetUploadFileID(), packet))` → `RemoveFromUploadQueue(client)`; si no `CheckFailedFileIdReqs(packet)` (contador antiflood, 6 fallos → strike, 2 strikes → baneo).
- eMule 0.72a **nunca emite** este mensaje (opcode legacy conservado en recepción).
- El socket **no** lo cierra eMule — la sesión continúa.

### Bajo perfil `LegacyEdonkey`

El mismo valor `0x49` es `OP_FILESTATUSREQ` (petición de estado de las partes, payload `hash16` idéntico). El decodificador `decode_file_status_req_with_profile` rechaza `0x49` bajo los perfiles eMule y rechaza `0x4F` bajo `LegacyEdonkey`.

## `OP_CHANGE_CLIENT_ID` — `0x4D` (perfiles eMule/aMule)

### Layout (`Opcodes.h:219` — `<ID_old 4><ID_new 4>`)

| Offset | Tipo | Campo | Notas |
| :--- | :--- | :--- | :--- |
| 6 | `u32` LE | `newUserID` | Nuevo ID ed2k del cliente |
| 10 | `u32` LE | `newServerIP` | IP del servidor que atribuyó el ID |

### Semántica eMule (`ListenSocket.cpp:590`)

- `IsLowID(newUserID)` → actualización del ID **únicamente** si el servidor `newServerIP` es conocido (`serverlist->GetServerByIP`).
- `newUserID == client->GetIP()` → el cliente tiene un HighID → `SetUserIDHybrid(ntohl(newUserID))`.
- Datos adicionales tolerados (registro debug únicamente).
- eMule 0.72a **tampoco emite nunca** este mensaje.

## Política de emisión en el lado Rust

- `OP_END_OF_DOWNLOAD` (`0x49`) se emite cuando termina una sesión de descarga (`peer_session/transfer.rs`) — el payload `hash16` permite a un uploader eMule/aMule liberar nuestra entrada de cola.
- `OP_CHANGE_CLIENT_ID` (`0x4D`) **nunca se emite**; en recepción se decodifica y se registra (actualización de ID innecesaria: la identidad de subida está indexada por `client_key` userhash/IP, no por el ID ed2k).

## Vector de prueba

`PR_ED2K` + `len=17` + `0x49` + `hash16`:

```
E3 11 00 00 00 49 4A 52 49 53 54 45 52 4D 49 4E 41 54 45 44 21 21
```

- Perfil `Emule072a`: decodificado como notificación end-of-download (no `FileStatusReq`).
- Perfil `LegacyEdonkey`: decodificado como `FileStatusReq`.

`PR_ED2K` + `len=9` + `0x4D` + `u32(0x01020304)` + `u32(0x0A0B0C0D)`:

```
E3 09 00 00 00 4D 04 03 02 01 0D 0C 0B 0A
```

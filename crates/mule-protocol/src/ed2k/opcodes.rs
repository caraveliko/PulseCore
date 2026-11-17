// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Códigos de operación históricos (opcodes) para eD2K, extensiones de eMule y cargas útiles comprimidas.

// ---------------------------------------------------------------------------
// Códigos de operación estándar de eD2K (protocolo 0xE3)
// ---------------------------------------------------------------------------

/// Paquete de inicialización del saludo (`0x01`).
pub const OP_HELLO: u8 = 0x01;
/// Petición de inicio de sesión del cliente al servidor (`0x01`).
pub const OP_LOGINREQUEST: u8 = 0x01;

/// El servidor rechaza la última orden del cliente (`0x05`, carga útil vacía).
///
/// `OP_REJECT` de `Opcodes.h` de eMule — enviado p. ej. cuando una orden usa el protocolo
/// incorrecto (paquete sin comprimir a un servidor que solo admite compresión) o cuando el
/// servidor rechaza el inicio de sesión. `ServerSocket.cpp` únicamente lo registra; el servidor
/// suele cerrar el socket él mismo justo después.
pub const OP_REJECT: u8 = 0x05;

/// Respuesta de estado del servidor con recuentos de usuarios y ficheros (`0x34`, heredado `0x14`).
pub const OP_SERVERSTATUS: u8 = 0x34;
/// Código de operación de respuesta de estado de servidor heredado (`0x14`).
pub const OP_SERVERSTATUS_LEGACY: u8 = 0x14;

/// Identificación y metadatos del servidor (`0x41`).
pub const OP_SERVERIDENT: u8 = 0x41;

/// Mensaje del servidor que contiene saludos o texto de aviso (`0x38`).
pub const OP_SERVERMESSAGE: u8 = 0x38;

/// El servidor asigna el ID de cliente y los indicadores TCP (`0x40`).
pub const OP_IDCHANGE: u8 = 0x40;

/// Petición de búsqueda enviada por el cliente al servidor (`0x16`).
pub const OP_SEARCHREQUEST: u8 = 0x16;

/// Resultado de búsqueda enviado por el servidor al cliente (`0x33`).
pub const OP_SEARCH_RESULT: u8 = 0x33;

/// Petición de obtención de fuentes enviada por el cliente al servidor para un fichero (`0x19`).
pub const OP_GETSOURCES: u8 = 0x19;

/// Petición de obtención de fuentes con ofuscación enviada por el cliente al servidor (`0x23`).
pub const OP_GETSOURCES_OBFU: u8 = 0x23;

/// Respuesta con fuentes encontradas enviada por el servidor al cliente (`0x42`).
pub const OP_FOUNDSOURCES: u8 = 0x42;

/// Respuesta con fuentes encontradas con ajustes de ofuscación y hashes de usuario opcionales (`0x44`).
pub const OP_FOUNDSOURCES_OBFU: u8 = 0x44;

/// Paquete de respuesta al saludo (`0x4C`).
pub const OP_HELLO_ANSWER: u8 = 0x4C;
/// Alias de OP_HELLO_ANSWER (`0x4C`).
pub const OP_HELLOANSWER: u8 = 0x4C;

/// Anuncio de ficheros compartidos al servidor (`0x15`).
pub const OP_OFFERFILES: u8 = 0x15;

/// Solicita al servidor retransmitir una petición de conexión a un cliente LowID (`0x1C`).
pub const OP_CALLBACKREQUEST: u8 = 0x1C;

/// Notificación de callback retransmitida por el servidor al destino LowID (`0x35`).
pub const OP_CALLBACKREQUESTED: u8 = 0x35;

/// Fallo del servidor al retransmitir una petición de callback (`0x36`).
pub const OP_CALLBACK_FAIL: u8 = 0x36;

/// Paquete de transferencia de bloque de datos (`0x46`).
pub const OP_SENDINGPART: u8 = 0x46;

/// Petición canónica de eMule para rangos de bloques en una parte (`0x47`).
pub const OP_REQUESTPARTS_EMULE: u8 = 0x47;

/// Asignación heredada del hash de fichero solicitado (`0x47`).
pub const OP_SETREQFILEID: u8 = 0x47;

/// Respuesta que indica que el fichero solicitado no está compartido (`0x48`).
pub const OP_FILEREQANSNOFILE: u8 = 0x48;

/// Petición de mapa de bits de disponibilidad de partes — eDonkey2000 heredado (`0x49`).
/// En eMule 0.72a este valor en cable es `OP_END_OF_DOWNLOAD`; ambos se
/// desambiguan mediante `Ed2kCompatibilityProfile`.
pub const OP_FILESTATUSREQ: u8 = 0x49;

/// Notificación de fin de descarga de eMule (`0x49`) — `<hash16>`; el subidor
/// retira al cliente de su cola de subida (`ListenSocket.cpp:537`).
/// Mismo valor en cable que `OP_FILESTATUSREQ` heredado — nunca coinciden ambos bajo un mismo
/// `Ed2kCompatibilityProfile`.
pub const OP_END_OF_DOWNLOAD: u8 = 0x49;

/// Petición de rangos de bloques específicos en una parte — eDonkey2000 heredado (`0x4B`).
/// En eMule 0.72a este valor en cable es `OP_ASKSHAREDFILESANSWER`.
pub const OP_REQUESTPARTS: u8 = 0x4B;

/// Notificación de cambio de ID de cliente de eMule (`0x4D`) —
/// `<u32 newUserID><u32 newServerIP>` (`ListenSocket.cpp:590`).
/// eDonkey2000 heredado utilizaba este valor en cable para `OP_END_OF_DOWNLOAD`.
pub const OP_CHANGE_CLIENT_ID: u8 = 0x4D;

/// Asignación canónica de eMule del hash de fichero solicitado / petición de estado de fichero (`0x4F`).
pub const OP_SETREQFILEID_EMULE: u8 = 0x4F;

/// Respuesta con mapa de bits de disponibilidad de partes (`0x50`).
pub const OP_FILESTATUS: u8 = 0x50;

/// Petición de conjunto de hashes (hashset) para fichero multipartición (`0x51`).
pub const OP_HASHSETREQUEST: u8 = 0x51;

/// Respuesta con carga útil del conjunto de hashes (hashset) (`0x52`).
pub const OP_HASHSETANSWER: u8 = 0x52;

/// Petición de entrada en la cola de subida (`0x54`).
pub const OP_STARTUPLOADREQ: u8 = 0x54;

/// Aceptación de petición de subida y asignación de slot (`0x55`).
pub const OP_ACCEPTUPLOADREQ: u8 = 0x55;

/// Cancelación de transferencia o expulsión de la cola (`0x56`).
pub const OP_CANCELTRANSFER: u8 = 0x56;

/// Señal de que el cliente ya no necesita partes (`0x57`).
pub const OP_OUTOFPARTS: u8 = 0x57;

/// Petición de nombre de fichero por hash (`0x58`).
pub const OP_REQFILENAME: u8 = 0x58;

/// Respuesta con nombre de fichero (`0x59`).
pub const OP_REQFILENAME_ANSWER: u8 = 0x59;

/// Respuesta de posición en la cola de subida (`0x60`).
pub const OP_QUEUERANKING: u8 = 0x60;

/// Petición de exploración de ficheros compartidos del par — lista completa (`0x4A`, carga útil vacía,
/// `ListenSocket.cpp:651` de eMule `OP_ASKSHAREDFILES`). Siempre en PR_ED2K.
pub const OP_ASKSHAREDFILES: u8 = 0x4A;

/// Respuesta a la exploración de ficheros compartidos del par (`0x4B`): `<u32 count><entradas ofrecidas>`.
/// Comparte el valor en cable con el `OP_REQUESTPARTS` de eDonkey heredado — eMule los separa
/// por dirección: esta respuesta solo viaja de anfitrión de compartidos → explorador, mientras
/// que `OP_REQUESTPARTS` solo viaja de descargador → subidor en PR_ED2K.
pub const OP_ASKSHAREDFILESANSWER: u8 = 0x4B;

/// Petición de directorios compartidos (`0x5D`, carga útil vacía).
pub const OP_ASKSHAREDDIRS: u8 = 0x5D;

/// Petición de ficheros en directorio (`0x5E`): `<u16 len><dir utf8>`; el directorio es un
/// nombre simulado (`GetPseudoDirName` de eMule — nunca la ruta real).
pub const OP_ASKSHAREDFILESDIR: u8 = 0x5E;

/// Respuesta de directorios compartidos (`0x5F`): `<u32 count>(<u16 len><dir utf8>)*`.
pub const OP_ASKSHAREDDIRSANS: u8 = 0x5F;

/// Respuesta de ficheros en directorio (`0x60`):
/// `<u16 len><dir utf8><u32 count><entradas ofrecidas>` — siempre PR_ED2K.
/// Comparte el valor en cable con `OP_QUEUERANKING`, que solo se utiliza en PR_EMULE.
pub const OP_ASKSHAREDFILESDIRANS: u8 = 0x60;

/// Respuesta de acceso denegado (`0x61`, carga útil vacía) — siempre PR_ED2K.
/// Comparte el valor en cable con `OP_FILEDESC`, que solo se utiliza en PR_EMULE.
pub const OP_ASKSHAREDDENIEDANS: u8 = 0x61;

/// Lista de servidores enviada por el servidor conectado (`0x32`).
/// Carga útil: `u8 count` + `count` × (`u32 ip` + `u16 port`).
pub const OP_SERVERLIST: u8 = 0x32;

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Códigos de operación UDP del servidor (protocolo 0xE3 sobre UDP, UDPSocket de eMule)
// ---------------------------------------------------------------------------

/// Petición de búsqueda global, formato extendido de etiquetas (`0x90`):
/// `<u32 tag_count><CTag CT_SERVER_UDPSEARCH_FLAGS>` + expresión de búsqueda.
pub const OP_GLOBSEARCHREQ3: u8 = 0x90;
/// Petición de búsqueda global (`0x92`): expresión de búsqueda sin procesar, misma carga útil que
/// `OP_SEARCHREQUEST` (eMule reutiliza el paquete de búsqueda tal cual).
pub const OP_GLOBSEARCHREQ2: u8 = 0x92;
/// Petición de fuentes global con tamaño de fichero (`0x94`): `hash16 + u32 size`.
pub const OP_GLOBGETSOURCES2: u8 = 0x94;
/// Petición de búsqueda global, formato heredado (`0x98`): expresión de búsqueda sin procesar.
pub const OP_GLOBSEARCHREQ: u8 = 0x98;
/// Petición de estadísticas del servidor (`0x96`, carga útil vacía).
pub const OP_GLOBSERVSTATREQ: u8 = 0x96;
/// Respuesta de estadísticas del servidor (`0x97`): `u32 users + u32 files`.
pub const OP_GLOBSERVSTATRES: u8 = 0x97;
/// Resultado de búsqueda global (`0x99`): entradas de ficheros resultantes de la búsqueda.
pub const OP_GLOBSEARCHRES: u8 = 0x99;
/// Respuesta global de fuentes encontradas (`0x9B`): repetido
/// `hash16 + u8 count + count` × (`u32 ip` + `u16 port`).
pub const OP_GLOBFOUNDSOURCES: u8 = 0x9B;

// ---------------------------------------------------------------------------
// Códigos de operación UDP de cliente a cliente (protocolo 0xC5, ClientUDPSocket de eMule)
// ---------------------------------------------------------------------------

/// Mantenimiento de conexión / repetición de petición enviada por UDP a una fuente en cola (`0x90`).
/// Carga útil: `<HASH 16>` (+ información extendida opcional para versión UDP > 2).
pub const OP_REASKFILEPING: u8 = 0x90;

/// Respuesta UDP que transporta el rango en la cola (`0x91`).
/// Carga útil: `[u16 part_status_count + bitfield]` (versión UDP > 3) + `u16 rank`.
pub const OP_REASKACK: u8 = 0x91;

/// Respuesta UDP: el fichero solicitado no está compartido (`0x92`, carga útil vacía).
pub const OP_FILENOTFOUND: u8 = 0x92;

/// Respuesta UDP: la cola de subida remota está llena (`0x93`, carga útil vacía).
pub const OP_QUEUEFULL: u8 = 0x93;

/// Repetición de petición de callback UDP retransmitida a través de un buddy de Kad (`0x94`).
pub const OP_REASKCALLBACKUDP: u8 = 0x94;
/// Petición de callback directo UDP (`0x95`, `OP_DIRECTCALLBACKREQ` de eMule):
/// `<tcp_port u16><userhash 16><connect_options u8>` — enviada a un par tras
/// cortafuegos en Kad solicitándole abrir una conexión TCP saliente.
pub const OP_DIRECTCALLBACKREQ: u8 = 0x95;

/// Notificación de callback retransmitida por buddy a través del enlace TCP de buddy (`0x99`):
/// `[buddy_id:16][file_id:16][requester_ip:4][requester_tcp:2]`.
pub const OP_CALLBACK: u8 = 0x99;
/// Formato TCP de la repetición de petición retransmitida por buddy (`0x9A`):
/// `[requester_ip:4][requester_udp:2][file_id:16][partstatus][complete_src:2]`.
pub const OP_REASKCALLBACKTCP: u8 = 0x9A;
/// Ping de mantenimiento de conexión del enlace de buddy (`0x9F`, carga útil vacía).
pub const OP_BUDDYPING: u8 = 0x9F;
/// Pong de mantenimiento de conexión del enlace de buddy (`0xA0`, carga útil vacía).
pub const OP_BUDDYPONG: u8 = 0xA0;

// ---------------------------------------------------------------------------
// Códigos de operación extendidos de eMule (protocolo 0xC5)
// ---------------------------------------------------------------------------

/// Intercambio de capacidades del saludo extendido de eMule (`0x01`).
pub const OP_EMULE_INFO: u8 = 0x01;
/// Alias canónico de `OP_EMULE_INFO`.
pub const OP_EMULEINFO: u8 = OP_EMULE_INFO;

/// Respuesta de capacidades del saludo extendido de eMule (`0x02`).
pub const OP_EMULE_INFO_ANSWER: u8 = 0x02;
/// Alias canónico de `OP_EMULE_INFO_ANSWER`.
pub const OP_EMULEINFOANSWER: u8 = OP_EMULE_INFO_ANSWER;

/// Petición de datos de recuperación de parte AICH (`0x9B`, eMule 0.72a `OP_AICHREQUEST`,
/// `opcodes.h:272`): `<HASH 16><uint16 part><AICH master hash 20>`.
pub const OP_AICH_REQUEST: u8 = 0x9B;

/// Respuesta de datos de recuperación AICH (`0x9C`, eMule `OP_AICHANSWER`,
/// `opcodes.h:273`): `<HASH 16><uint16><HASH 20><data>` — solo el hash en caso de fallo.
pub const OP_AICH_ANSWER: u8 = 0x9C;

/// Petición del hash maestro (raíz) AICH de un fichero (`0x9E`, `OP_AICHFILEHASHREQ` de eMule,
/// `opcodes.h:275`). Obsoleta en origen en favor de
/// `FileIdentifier` (`OP_MULTIPACKET_EXT2` / `OP_HASHSETREQUEST2`), aún
/// atendida por 0.72a.
pub const OP_AICHFILEHASHREQ: u8 = 0x9E;

/// Respuesta de hash maestro AICH (`0x9D`, `OP_AICHFILEHASHANS` de eMule,
/// `opcodes.h:274`): `<HASH 16><AICH root 20>`.
pub const OP_AICHFILEHASHRES: u8 = 0x9D;

/// Petición de fuentes al par (SX1 - heredado, `0x81`).
pub const OP_REQUESTSOURCES: u8 = 0x81;

/// Respuesta de fuentes al par (SX1 - heredado, `0x82`).
pub const OP_ANSWERSOURCES: u8 = 0x82;

/// Petición de fuentes al par (SX2, `0x83`).
pub const OP_REQUESTSOURCES2: u8 = 0x83;

/// Respuesta de fuentes al par (SX2, `0x84`).
pub const OP_ANSWERSOURCES2: u8 = 0x84;

/// Intercambio de clave pública para identificación segura de usuario (`0x85`).
pub const OP_PUBLICKEY: u8 = 0x85;

/// Intercambio de firma para identificación segura de usuario (`0x86`).
pub const OP_SIGNATURE: u8 = 0x86;

/// Estado de la identificación segura de usuario (`0x87`).
pub const OP_SECIDENTSTATE: u8 = 0x87;

/// Petición de comprobación de cortafuegos UDP de Kad (`0xA7`, enviada por PR_EMULE tras el
/// hello TCP). Carga útil: `u16 intern_kad_port` + `u16 extern_kad_port` +
/// `u32 udp_verify_key` (`SendFirewallCheckUDPRequest` de eMule).
pub const OP_FWCHECKUDPREQ: u8 = 0xA7;

/// Acuse de recibo de comprobación TCP de cortafuegos Kad (`0xA8`, `OP_KAD_FWTCPCHECK_ACK` de eMule).
pub const OP_KAD_FWTCPCHECK_ACK: u8 = 0xA8;

/// Consulta de IP pública (`0x97`, PR_EMULE): carga útil vacía — el par nos pregunta
/// en qué IP lo vemos (`OP_PUBLICIP_REQ` de eMule). Comparte su valor en cable
/// con `OP_GLOBSERVSTATRES` en el canal UDP del servidor.
pub const OP_PUBLICIP_REQ: u8 = 0x97;

/// Respuesta de IP pública (`0x98`, PR_EMULE): `<u32 ip>` — la IP del solicitante
/// vista desde nuestro socket (`OP_PUBLICIP_ANSWER` de eMule). Comparte su valor
/// en cable con `OP_GLOBSEARCHREQ` en el canal UDP del servidor.
pub const OP_PUBLICIP_ANSWER: u8 = 0x98;

// Códigos de operación extendidos de eMule 0.72a (PR_EMULE `0xC5`, TCP)

/// Mensaje de chat entre pares (`0x4E`): `<len u16><bytes del mensaje>` (`OP_MESSAGE` de eMule).
pub const OP_MESSAGE: u8 = 0x4E;

/// Notificación de cambio de slot de subida (`0x5B`): `<HASH 16>` (`OP_CHANGE_SLOT` de eMule).
pub const OP_CHANGE_SLOT: u8 = 0x5B;

/// Informe de posición en cola heredado (`0x5C`): `<rank u32>` (`OP_QUEUERANK` de eMule).
pub const OP_QUEUERANK: u8 = 0x5C;

/// Valoración/comentario de fichero (`0x61`): `<rating u8><len u16><desc>` (`OP_FILEDESC` de eMule).
pub const OP_FILEDESC: u8 = 0x61;

/// Petición de ficheros agrupada (`0x92`, obsoleta en origen): `<HASH 16>` + subcódigos de operación
/// (`OP_MULTIPACKET` de eMule).
pub const OP_MULTIPACKET: u8 = 0x92;

/// Respuesta a petición de ficheros agrupada (`0x93`, obsoleta en origen): `<HASH 16>` +
/// subcódigos de operación de respuesta (`OP_MULTIPACKETANSWER` de eMule).
pub const OP_MULTIPACKETANSWER: u8 = 0x93;

/// Consulta de fuentes en caché de pares (`0x94`, en desuso en origen — decodificada como opaca).
pub const OP_PEERCACHE_QUERY: u8 = 0x94;
/// Respuesta de caché de pares (`0x95`, en desuso en origen — decodificada como opaca).
pub const OP_PEERCACHE_ANSWER: u8 = 0x95;
/// Acuse de recibo de caché de pares (`0x96`, en desuso en origen — decodificada como opaca).
pub const OP_PEERCACHE_ACK: u8 = 0x96;

/// Petición de ficheros agrupada con tamaño de 64 bits (`0xA4`, obsoleta en origen):
/// `<HASH 16><size u64>` + subcódigos de operación (`OP_MULTIPACKET_EXT` de eMule).
pub const OP_MULTIPACKET_EXT: u8 = 0xA4;

/// Petición de CAPTCHA antispam en el chat (`0xA5`): `<tag_count u8><tags><mapa de bits BMP>`
/// (`OP_CHATCAPTCHAREQ` de eMule).
pub const OP_CHATCAPTCHAREQ: u8 = 0xA5;

/// Resultado de CAPTCHA de chat (`0xA6`): `<status u8>` — 0 resuelto, 1 fallido
/// (`OP_CHATCAPTCHARES` de eMule).
pub const OP_CHATCAPTCHARES: u8 = 0xA6;

/// Petición de ficheros agrupada con `FileIdentifier` (`0xA9`):
/// `<FileIdentifier>` + subcódigos de operación (`OP_MULTIPACKET_EXT2` de eMule, `opcodes.h:286`).
pub const OP_MULTIPACKET_EXT2: u8 = 0xA9;

/// Respuesta a petición de ficheros agrupada con `FileIdentifier` (`0xB0`):
/// `<FileIdentifier>` + subcódigos de operación de respuesta (`OP_MULTIPACKETANSWER_EXT2` de eMule,
/// `opcodes.h:287`).
pub const OP_MULTIPACKETANSWER_EXT2: u8 = 0xB0;

/// Petición de conjunto de hashes con `FileIdentifier` (`0xB1`):
/// `<FileIdentifier><options u8>` — bit0 = conjunto MD4, bit1 = conjunto AICH
/// (`OP_HASHSETREQUEST2` de eMule, `opcodes.h:288`).
pub const OP_HASHSETREQUEST2: u8 = 0xB1;

/// Respuesta de conjunto de hashes con `FileIdentifier` (`0xB2`):
/// `<FileIdentifier><options u8>[<md4 set>][<aich set>]`
/// (`OP_HASHSETANSWER2` de eMule, `opcodes.h:289`).
pub const OP_HASHSETANSWER2: u8 = 0xB2;

/// Prueba de conexión TCP (`0xFE`, `OP_PORTTEST` de eMule): `<status u8>`,
/// eMule responde `0x12`.
pub const OP_PORTTEST: u8 = 0xFE;

// ---------------------------------------------------------------------------
// Códigos de operación de protocolo comprimido (protocolo 0xD4)
// ---------------------------------------------------------------------------

/// Carga útil de bloque de datos comprimido (`0x40`).
pub const OP_COMPRESSEDPART: u8 = 0x40;

// ---------------------------------------------------------------------------
// Códigos de operación de transferencia de 64 bits (ficheros grandes) — variantes `*_I64` de eMule.
// La disposición en cable sustituye desplazamientos u32 por u64 (opcodes.h).
// ---------------------------------------------------------------------------

/// Bloque de datos comprimido con desplazamiento inicial de 64 bits (`0xA1`,
/// `<HASH 16><von 8><size 4><data size>`).
pub const OP_COMPRESSEDPART_I64: u8 = 0xA1;

/// Bloque de datos sin comprimir con desplazamientos de 64 bits (`0xA2`,
/// `<HASH 16><von 8><bis 8><data von-bis>`).
pub const OP_SENDINGPART_I64: u8 = 0xA2;

/// Petición de bloque con desplazamientos de 64 bits (`0xA3`,
/// `<HASH 16><von[3] 8*3><bis[3] 8*3>`).
pub const OP_REQUESTPARTS_I64: u8 = 0xA3;

/// Devuelve un nombre simbólico legible para humanos para un código de operación bajo un protocolo dado.
pub fn opcode_name(protocol: u8, opcode: u8) -> &'static str {
    match protocol {
        super::protocol::PR_ED2K => match opcode {
            OP_HELLO => "OP_HELLO / OP_LOGINREQUEST",
            OP_HELLO_ANSWER => "OP_HELLO_ANSWER",
            OP_REJECT => "OP_REJECT",
            OP_SERVERMESSAGE => "OP_SERVERMESSAGE",
            OP_IDCHANGE => "OP_IDCHANGE",
            OP_SERVERSTATUS => "OP_SERVERSTATUS",
            OP_SERVERIDENT => "OP_SERVERIDENT",
            OP_SEARCHREQUEST => "OP_SEARCHREQUEST",
            OP_SEARCH_RESULT => "OP_SEARCH_RESULT",
            OP_GETSOURCES => "OP_GETSOURCES",
            OP_GETSOURCES_OBFU => "OP_GETSOURCES_OBFU",
            OP_FOUNDSOURCES => "OP_FOUNDSOURCES",
            OP_FOUNDSOURCES_OBFU => "OP_FOUNDSOURCES_OBFU",
            OP_OFFERFILES => "OP_OFFERFILES",
            OP_CALLBACKREQUEST => "OP_CALLBACKREQUEST",
            OP_CALLBACKREQUESTED => "OP_CALLBACKREQUESTED",
            OP_CALLBACK_FAIL => "OP_CALLBACK_FAIL",
            OP_SENDINGPART => "OP_SENDINGPART",
            OP_SETREQFILEID => "OP_SETREQFILEID",
            OP_FILEREQANSNOFILE => "OP_FILEREQANSNOFILE",
            OP_FILESTATUSREQ => "OP_FILESTATUSREQ/OP_END_OF_DOWNLOAD",
            OP_REQUESTPARTS => "OP_REQUESTPARTS",
            OP_CHANGE_CLIENT_ID => "OP_CHANGE_CLIENT_ID",
            OP_FILESTATUS => "OP_FILESTATUS",
            OP_HASHSETREQUEST => "OP_HASHSETREQUEST",
            OP_HASHSETANSWER => "OP_HASHSETANSWER",
            OP_STARTUPLOADREQ => "OP_STARTUPLOADREQ",
            OP_ACCEPTUPLOADREQ => "OP_ACCEPTUPLOADREQ",
            OP_CANCELTRANSFER => "OP_CANCELTRANSFER",
            OP_OUTOFPARTS => "OP_OUTOFPARTS",
            OP_REQFILENAME => "OP_REQFILENAME",
            OP_REQFILENAME_ANSWER => "OP_REQFILENAME_ANSWER",
            OP_SENDINGPART_I64 => "OP_SENDINGPART_I64",
            OP_REQUESTPARTS_I64 => "OP_REQUESTPARTS_I64",
            OP_MESSAGE => "OP_MESSAGE",
            OP_CHANGE_SLOT => "OP_CHANGE_SLOT",
            OP_QUEUERANK => "OP_QUEUERANK",
            OP_PORTTEST => "OP_PORTTEST",
            _ => "OP_UNKNOWN_ED2K",
        },
        super::protocol::PR_EMULE => match opcode {
            OP_EMULE_INFO => "OP_EMULE_INFO",
            OP_EMULE_INFO_ANSWER => "OP_EMULE_INFO_ANSWER",
            OP_AICH_REQUEST => "OP_AICH_REQUEST",
            OP_AICH_ANSWER => "OP_AICH_ANSWER",
            OP_AICHFILEHASHREQ => "OP_AICHFILEHASHREQ",
            OP_AICHFILEHASHRES => "OP_AICHFILEHASHRES",
            OP_REQUESTSOURCES => "OP_REQUESTSOURCES",
            OP_ANSWERSOURCES => "OP_ANSWERSOURCES",
            OP_REQUESTSOURCES2 => "OP_REQUESTSOURCES2",
            OP_ANSWERSOURCES2 => "OP_ANSWERSOURCES2",
            OP_FWCHECKUDPREQ => "OP_FWCHECKUDPREQ",
            OP_KAD_FWTCPCHECK_ACK => "OP_KAD_FWTCPCHECK_ACK",
            OP_QUEUERANKING => "OP_QUEUERANKING",
            OP_FILEDESC => "OP_FILEDESC",
            OP_MULTIPACKET => "OP_MULTIPACKET",
            OP_MULTIPACKETANSWER => "OP_MULTIPACKETANSWER",
            OP_PEERCACHE_QUERY => "OP_PEERCACHE_QUERY",
            OP_PEERCACHE_ANSWER => "OP_PEERCACHE_ANSWER",
            OP_PEERCACHE_ACK => "OP_PEERCACHE_ACK",
            OP_MULTIPACKET_EXT => "OP_MULTIPACKET_EXT",
            OP_CHATCAPTCHAREQ => "OP_CHATCAPTCHAREQ",
            OP_CHATCAPTCHARES => "OP_CHATCAPTCHARES",
            OP_MULTIPACKET_EXT2 => "OP_MULTIPACKET_EXT2",
            OP_MULTIPACKETANSWER_EXT2 => "OP_MULTIPACKETANSWER_EXT2",
            OP_HASHSETREQUEST2 => "OP_HASHSETREQUEST2",
            OP_HASHSETANSWER2 => "OP_HASHSETANSWER2",
            OP_REQUESTPARTS_I64 => "OP_REQUESTPARTS_I64",
            OP_SENDINGPART_I64 => "OP_SENDINGPART_I64",
            OP_COMPRESSEDPART => "OP_COMPRESSEDPART",
            OP_COMPRESSEDPART_I64 => "OP_COMPRESSEDPART_I64",
            _ => "OP_UNKNOWN_EMULE",
        },
        super::protocol::PR_EMULE_COMPRESSED => match opcode {
            OP_COMPRESSEDPART => "OP_COMPRESSEDPART",
            OP_COMPRESSEDPART_I64 => "OP_COMPRESSEDPART_I64",
            OP_ANSWERSOURCES => "OP_ANSWERSOURCES",
            OP_ANSWERSOURCES2 => "OP_ANSWERSOURCES2",
            _ => "OP_UNKNOWN_COMPRESSED",
        },
        _ => "OP_UNKNOWN_PROTOCOL",
    }
}

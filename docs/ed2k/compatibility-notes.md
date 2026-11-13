# Notas de Compatibilidad y Variantes Históricas eD2K

Este documento registra las particularidades de formato, variantes de codificación y divergencias observadas entre las implementaciones históricas eMule, aMule y las especificaciones originales de eDonkey2000.

---

## 1. Identificadores Numéricos frente a Nombres de Tags Textuales

En eDonkey2000 y las versiones iniciales de eMule:
- El nombre de un tag comienza con un entero little-endian de 16 bits que indica su longitud en bytes.
- Si dicha longitud vale **1**, el byte siguiente no es una cadena ASCII de un carácter, sino un **identificador especial estandarizado** (`Special Tag ID`):
  - `0x01`: Nombre de usuario / Nombre de archivo (`CT_NAME` / `FT_FILENAME`)
  - `0x02`: Tamaño de archivo (`FT_FILESIZE`)
  - `0x03`: Tipo de archivo (`FT_FILETYPE`)
  - `0x11`: Versión del cliente (`CT_VERSION`)
  - `0x5A`: Versión eMule compatible (`CT_EMULE_VERSION`)
  - `0x5B`: Indicadores de capacidades extendidas (`CT_EMULE_MISCOPTIONS1`)
  - `0x5C`: Indicadores de capacidades extendidas 2 (`CT_EMULE_MISCOPTIONS2`)
  - `0xFB`: Puerto UDP eMule (`CT_EMULE_UDPOPTIONS`)
- Si la longitud es superior a 1, se trata de una cadena textual (p. ej.: `"muleversion"` o `"modversion"`).

### Evolución eMule 0.42+ (Extended Tags)
En las versiones modernas de eMule, un mecanismo de optimización compacta los tags codificando indicadores binarios en el bit de mayor peso del tipo de tag (`tag_type & 0x80`), evitando escribir la longitud 1 para los identificadores especiales.
Nuestro decodificador pasivo soporta tanto la sintaxis clásica con prefijo de longitud como la captura de etiquetas extendidas.

---

## 2. Codificación de Cadenas de Caracteres

Históricamente:
- Los clientes eDonkey originales utilizaban la codificación local ANSI/Windows-1252 o ISO-8859-1 del usuario.
- A partir de eMule 0.47a y con aMule 2.x, la codificación estándar por defecto para los seudónimos y nombres de archivo pasó a ser **UTF-8**.
- **Gestión eMuleRust**: El decodificador intenta una conversión UTF-8 estricta. En caso de secuencia no válida, recurre a una conversión con reemplazo sin fallo fatal (`String::from_utf8_lossy`), garantizando la resiliencia frente a pares ANSI antiguos.

---

## 3. HighID frente a LowID en el Client ID

El campo `Client ID` (little-endian de 32 bits) transporta la dirección de red del par:
- **HighID**: El par es accesible directamente desde el exterior en su puerto TCP. El `Client ID` es igual a la dirección IPv4 pública del par reordenada como entero little-endian de 32 bits. Un valor $\ge 16\,777\,216$ ($2^{24}$) caracteriza un HighID.
- **LowID**: El par está detrás de un cortafuegos o un router NAT sin redirección de puertos. El `Client ID` es un identificador arbitrario asignado por el servidor eD2K ($< 16\,777\,216$). Dos pares en LowID no pueden iniciar una conexión directa sin pasar por un servidor intermediario.

---

## 4. IP y Puerto del Servidor al Final del Handshake

En `OP_HELLO` y `OP_HELLOANSWER`:
- Tras la lista de tags, eMule transmite la dirección IPv4 (4 bytes LE) y el puerto (2 bytes LE) del servidor eD2K al que el cliente está conectado actualmente.
- Si el cliente funciona sin estar conectado a un servidor central (modo Kademlia puro o sin conexión), eMule y aMule escriben `0.0.0.0:0` (6 bytes nulos).
- Nuestro decodificador pasivo extrae estos campos opcionales sin presuponer su significado de red.

---

## 5. Desviaciones asumidas frente a eMule 0.72a

Desviaciones voluntarias o simplificaciones documentadas tras la auditoría de
fidelidad (`AUD-Bxx` — referencia `srchybrid`, eMule 0.72a):

- **Identidad anunciada = eMule 0.72a** (`CT_EMULE_VERSION = 72<<10`,
  `client_version = 72`) alineada con la referencia — véase §13 para los
  bits `MISCOPTIONS2` voluntariamente no anunciados.
- **Timeout de cola de espera**: el daemon abandona un par en cola
  silencioso tras `FILEREASKTIME + 15 min` (`PeerTimeouts::queue_timeout`).
  eMule no tiene ningún timeout de cola — el socket vive hasta la muerte TCP.
  Aproximación adoptada para acotar los sockets inactivos.
- **Reask UDP a cadencia fija (~21 min)** en lugar del modelo eMule (ping UDP
  en la ventana de 2 min previa al reask TCP, throttle ~20 min). La
  guarda «>3 paquetes y >30 % de fallos → detener UDP» es en cambio
  fiel (`DownloadClient.cpp:1340`).
- **Reasks standalone**: los reasks se emiten en paquetes simples
  (`OP_REQFILENAME`, `OP_SETREQFILEID`, `OP_STARTUPLOADREQ`) en lugar de
  agruparse en `OP_MULTIPACKET*` — comportamiento que los pares eMule
  esperan de los clientes que no siempre anuncian la capacidad.
- **NNP (No Needed Parts)**: la sesión intenta primero un swap A4AF
  socket-level (§11); sin candidato se cierra y la fuente se vuelve a
  solicitar tras ~58 min (`FILEREASKTIME * 2`), mientras que eMule mantiene
  `DS_NONEEDEDPARTS` en el socket abierto. Mismo ciclo de reask.
- **A4AF socket-level implementado (§11)**: `SwapToAnotherFile`/`DoSwap`
  portado — el socket reemite la secuencia `SendFileRequest` para el
  archivo objetivo en lugar de reconectar.
- **Chat y exploración de compartidos no implementados**: `OP_MESSAGE`,
  `OP_ASKSHAREDFILES*`, `OP_QUEUERANK (0x5C)`, `OP_CHANGE_SLOT (0x5B)`
  se decodifican o ignoran sin lógica de negocio.
- **Preview no anunciada** (bit 0 `MiscOptions1` = 0): al contrario que
  una versión anterior, la capacidad ya no se anuncia puesto que
  `OP_REQUESTPREVIEW`/`OP_PREVIEWANSWER` no están implementados.
- **`OP_CHANGE_CLIENT_ID` (0x4D) registrado, no aplicado**: eMule
  actualiza `m_nUserIDHybrid`/la IP del servidor del par; el daemon acota
  el parsing (2×u32) y registra sin modificar el estado — suficiente ya que
  los clientes modernos ya no emiten el opcode.

### Correcciones de alineación (auditoría fase B)

- `OP_END_OF_DOWNLOAD` emitido/enrutado en `0x49` (eMule); `0x4D` decodificado
  como `OP_CHANGE_CLIENT_ID` (`Opcodes.h:218-219`). El perfil eDonkey
  legacy conserva `0x49` = `OP_FILESTATUSREQ` mediante el despacho por
  perfil (`profile.rs`).
- Inbound `0x49` → retirada del par de la cola de subida si el hash
  corresponde al archivo subido (`ListenSocket.cpp:537`); `0x4F`
  sigue siendo el `SETREQFILEID` canónico.
- `OP_REQFILENAME (0x58)` standalone servido fuera de multipacket: respuesta
  `OP_REQFILENAMEANSWER`, lectura acotada de la ext-info part-status
  (`ProcessExtendedInfo`), `OP_FILEREQANSNOFILE` si el archivo es
  desconocido o parcial sin parte completa (`ListenSocket.cpp:310`),
  contador anti-flood `CheckFailedFileIdReqs` → baneo tras 4 fallos.
- Handshake: `MultiPacket=1`, `FileIdent=1`, `Preview=0`,
  `ET_SOURCEEXCHANGE=3` (`BaseClient.cpp:715`); respuesta inbound
  `EMULEINFO → HELLOANSWER` en el orden de eMule, `client_id` y campos
  de servidor reales (ID asignado por el servidor actual).
- `OP_SETREQFILEID` no emitido para los archivos ≤ 1 parte
  (`DownloadClient.cpp:366`) — la respuesta `OP_REQFILENAMEANSWER`
  basta para establecer la completitud.

---

## 6. Kad — desviaciones asumidas y correcciones de alineación (auditoría fase B, pasada 2)

### Desviaciones asumidas

- **Respuesta `0x3B` sin tag discriminador**: una entrada en formato
  plano legacy (sin `PUBLISHINFO`/`KADAICHHASHRESULT`) sigue decodificándose por
  la ruta de compatibilidad pre-v9; eMule ≥ 0.50a emite siempre la
  taglist completa.
- **Tabla de enrutamiento plana 128 × K** en lugar del árbol de zonas
  binario de eMule (`RoutingZone` — división acotada por `KK=5`/`KBASE=4`,
  mayor profundidad cerca de nuestro propio KadID). La capacidad por bin
  está alineada (`K=10`) pero la topología de cobertura difiere.
- **Tags publicados no persistidos al reiniciar**: las entradas
  keywords rehidratadas desde SQLite recuperan `file_name`/`file_size`/
  `source_count` pero no su taglist completa — las búsquedas
  restrictivas sobre metadatos de terceros solo coincidirán con estas entradas
  tras su republicación (eMule no persiste su índice en absoluto).
- **Convergencia `find_node`**: parada anticipada en cuanto 12 candidatos
  están en `SEARCHTOLERANCE` (≥ 2 iteraciones); eMule continúa
  hasta agotar `m_mapPossible` o `SEARCHFILE_TOTAL=300`.
- **Respuesta `0x3B` sin entradas ambigua**: sin tag discriminador,
  un resultado vacío se decodifica como `SearchKeywordRes` — indiferente
  en el lado solicitante (0 entradas en ambos casos).
- **`KADEMLIA2_PUBLISH_RES_ACK (0x4C)`**: emitido como respuesta cuando el
  byte de opciones lo solicita; un `0x4C` entrante se rechaza como «unknown
  opcode» al igual que en eMule («for future use»).

### Correcciones de alineación

- **`KADEMLIA2_SEARCH_KEY_REQ (0x33)`**: diseño en el cable corregido a
  `[keyword_hash 16][start_pos u16]` — sin `sender_id` terminal;
  bit 15 = búsqueda restrictiva, offset = `start_pos & 0x7FFF`
  (`KademliaUDPListener.cpp:1057` `Process_KADEMLIA2_SEARCH_KEY_REQ`).
- **Expresiones restrictivas aplicadas**: `kad::search_expr`
  parsea el árbol `SSearchTerm` (`CreateSearchExpressionTree` — AND/OR/
  NOT binarios, hojas string/meta/numeric u32+u64, profundidad máx.
  24) y lo evalúa como `CKeyEntry::SearchTermsMatch`: términos de nombre
  de archivo pre-tokenizados (`TokenizeOptQuotedSearchTerm`, comillas
  preservadas, subcadena insensible a mayúsculas, multipalabra = AND), igualdad
  de cadenas de metadatos (caso especial `fileformat` → extensión del
  nombre), relaciones enteras sobre los tags publicados con `filesize`
  virtual (`GetIntTagValue`). Expresión malformada → datagrama
  rechazado como el `try/catch` de eMule; `start_pos` pagina **después**
  del filtrado (`Indexed.cpp` `SendValidKeywordResult`). La taglist
  publicada se conserva en `KadKeywordEntryWire::tags` y se reemite
  verbatim en los resultados (`WriteTagList`).
- **`KADEMLIA2_PUBLISH_RES (0x4B)`**: byte de opciones opcional tras
  `[hash16][status]` — bit 0 → reenvío de un datagrama `0x4C` desnudo
  (`KademliaUDPListener.cpp:1438` `Process_KADEMLIA2_PUBLISH_RES`).
- **Fragmentación de las respuestas de búsqueda**: empaquetado voraz de las
  entradas en datagramas `KADEMLIA2_SEARCH_RES` consecutivos ≤
  `UDP_KAD_MAXFRAGMENT = 1420` (`Opcodes.h:134`,
  `Indexed.cpp:601/707` `SendValid{Keyword,Source}Result`) —
  antes un datagrama único de varios KiB sufría la
  fragmentación IP.
- **Índice de almacenamiento**: TTL diferenciados — fuentes 5 h
  (`KADEMLIAMAXINDEX=50 000` claves), palabras clave 24 h, notas 24 h;
  cotas `KADEMLIAMAXSOURCEPERFILE=1 000`,
  `KADEMLIAMAXENTRIES=60 000`, `KADEMLIAMAXNOTESPERFILE=150`,
  resultados servidos `iMaxResults=300` (`Opcodes.h:78-85`,
  `Indexed.cpp:619`).
- **Enrutamiento/búsqueda**: `K_BUCKET_SIZE = 10` (`Defines.h:42`, era 20),
  `ALPHA_QUERY = 3` (`:45`, era 5), semilla de búsqueda 50 contactos
  (`Search.cpp:172` `GetClosestTo(3, …, 50, …)`, era 32).

---

## 7. PartFile — desviaciones asumidas y correcciones de alineación (auditoría fase B, pasada 3)

### Desviaciones asumidas

- **Selección de bloques**: `GetNextRequestedBlock`/`CreateBlockRequests`
  de eMule (`DownloadClient.cpp:1950+`) combina frecuencia de parte,
  prioridad preview, `sameChunk`, porcentaje de completitud, número de
  clientes en transferencia y `bandwidthScore`. El secuenciador Rust
  (`ed2k/sequencer.rs`) aplica un orden «rareza / preview /
  progresión» simplificado — comportamiento cercano pero no idéntico;
  sin riesgo en el cable (los pares ven peticiones lícitas).
- **Baneo por corrupción** de alcance solo de sesión: `clientlist->AddBannedClient`
  en eMule también se acota a la sesión, pero
  `AddTrackClient` (seguimiento de puntuación entre archivos) no tiene
  equivalente — un par baneado queda como `Rejected` por archivo y no puede
  volver a ingerirse ni marcarse, sin puntuación persistente.
- **`.part.met`**: los tags `FT_CORRUPTEDPARTS`, `FT_STATUS`,
  `FT_LASTSEENCOMPLETE`, `FT_DL_ACTIVE_TIME`, `FT_PARTFILENAME` y el
  blob `FT_AICHHASHSET` no se escriben — corrupción y partes
  verificadas viven en SQLite; el hashset AICH se vuelve a obtener mediante
  recuperación/votos si es necesario tras reiniciar.
- **Sin `UncompressFile`/`SetZoneIdentifier`** al completarse
  (atributo sparse NTFS + zona MOTW) — específico de Windows, sin efecto
  protocolario.
- **Sin buffering de 30 s** (`m_BufferedData_list`/`FlushBuffer`):
  las escrituras son síncronas con volcado inmediato — más seguro
  (sin datos volátiles), misma semántica observable.

### Correcciones de alineación

- **Colisión de destino**: `CollisionStrategy::Overwrite`
  reemplazado por `Rename` — `CPartFile::PerformFileComplete` genera
  `name(1).ext`, `name(2).ext`, … incrementando un sufijo `(N)`
  existente; la ruta resuelta se persiste (`target_path`) y
  se registra en `shared_files` bajo su nombre real.
- **Escritura sobre parte completa rechazada**: `WriteToBuffer` de eMule
  ignora cualquier dato que toque una parte ya hasheada
  (`IsComplete(start|end part)` → `return 0`). `inject_range` hace lo
  mismo para toda parte cubierta por `ranges` — un bloque en vuelo que llegue
  tras completarse la parte ya no sobrescribe datos verificados.
- **ICH**: la verificación MD4 de parte es incondicional (eMule
  `HashSinglePart` en cada parte completada); `ich_enabled` ya no
  condiciona la verificación sino únicamente la elección entre cola de recuperación
  AICH por bloques y desalojo completo
  (`IsCorruptedPart && IsICHEnabled`, `PartFile.cpp:4158`).
- **`CorruptionBlackBox`** (`CorruptionBlackBox.cpp`): procedencia
  `(rango, remitente)` por parte, estados
  `Received/Verified/Corrupted` con división de los registros ante
  solapamiento; `EvaluateData` condena al remitente cuando
  `corrupto/(corrupto+verificado) > CBB_BANTHRESHOLD = 32 %` (cada
  registro corrupto contado ≥ `EMBLOCKSIZE` = 180 KiB). El baneo
  desconecta la sesión y marca la fuente como `Rejected`.
- **`CanAddSource`**: una fuente igual a nuestro propio LowID (origen
  servidor) o a nuestro `HighID:port` público se ignora; los clientes
  baneados se filtran desde `upsert_peer_sources` y antes de marcar
  (`IsBannedClient`).

## 8. Cliente UDP (`ClientUDPSocket.cpp`) — desviaciones asumidas y correcciones (auditoría fase B, pasada 3b)

### Desviaciones asumidas

- **Datagramas siempre en claro**: eMule puede cifrar el canal
  cliente-UDP (`ShouldReceiveCryptUDPPackets` + `CreateSafeKeyAndIV`);
  emitimos en claro y no decodificamos los datagramas ofuscados
  entrantes (registrados en `debug`, ignorados).
- **Sin emisión `OP_REASKCALLBACKUDP`**: para un re-ask de una
  fuente **LowID** en cola, eMule retransmite a través del buddy de la fuente.
  Nuestras sesiones en cola conservan su socket TCP — el re-ask TCP cubre
  el caso (el callback inicial `kadcb:` ya pasa por
  `KADEMLIA_CALLBACK_REQ`).
- **Ext-info del ping entrante no aplicada**: el part-status /
  complete-sources del solicitante (`ProcessExtendedInfo`) no se
  almacena — solo tiene efecto sobre la disponibilidad anunciada a los demás
  clientes de nuestra cola de subida, estadística no expuesta.
- **`OP_PORTTEST` sin respuesta**: eMule acusa el test `0x12` mediante su
  socket TCP de prueba dedicado (`SendPortTestReply('1')`), no en UDP;
  no distribuimos la herramienta de test de puertos — decodificado, registrado.
- **Purga en masa frente a por unidad**: eMule retira **1** fuente
  `RemoteQueueFull` por minuto (y 1 NNP / 40 s, 1 LowToLowIP / 30 s)
  mediante `m_lastpurgetime` compartido; nuestra pasada `SOURCE_PURGE_INTERVAL`
  (60 s) retira de golpe todo el presupuesto `count - umbral + 1` —
  cadencia idéntica, descenso hacia `umbral - 1` más rápido.

### Correcciones de alineación

- **`OP_REASKFILEPING`**: el rango ya no se revela a cualquier par que
  comparta la IP — eMule `GetWaitingClientByIP_UDP` + comprobación
  `md4equ(reqfilehash, sender->GetUploadFileID())` reproducido:
  correspondencia exacta IP **+ puerto UDP anunciado** (almacenado por entrada
  de cola), tolerancia «IP única», silencio ante ambigüedad multi-NAT
  (`bSenderMultipleIpUnknown`) y ante discordancia de hash. La fila
  `client_udp_port` se alimenta desde el HELLO del cliente.
- **`SetLastUpRequest`**: un ping aceptado refresca
  `last_request_unix_ms` de la entrada — la purga
  `MAX_PURGEQUEUETIME` (1 h) ya no afecta a un cliente que solo sobrevive
  mediante re-asks UDP.
- **`OP_REASKACK` UDP ver > 3**: el bloque part-status
  (`u16 count` + bitfield) se decodifica (`UdpPartStatus`) y se aplica a
  la disponibilidad de la fuente (`ProcessFileStatus` eMule) —
  rareza `m_SrcPartFrequency` + persistencia `availability`.
- **`OP_QUEUEFULL`**: marca `remote_queue_full` + rango 0 en la
  sesión en cola (`SetRemoteQueueFull` + `UDPReaskACK(0)`) — antes,
  el resultado se almacenaba sin consumidor.
- **`OP_FILENOTFOUND`**: deadlist — la fuente pasa a `Failed`
  (persistido, fuera de srclist) y la sesión en cola finaliza
  (`UDPReaskFNF` → `RemoveSource` + `m_DeadSourceList`).
- **`OP_DIRECTCALLBACKREQ` (0x95)**: decodificado
  (`u16 tcp_port + userhash16 + u8 conn_opts`), limitado a 1 petición/IP/
  180 s (`AllowCalbackRequest`), aceptado solo cuando nuestro estado de
  cortafuegos UDP Kad es `Firewalled` (`CKademlia::IsFirewalled()`) —
  entonces `connect_back` marca al par en saliente
  (`TryToConnect`) y atiende la sesión como un entrante.
- **Purga `RemoteQueueFull` al 80 %** (`PartFile.cpp:2285-2295`): las
  sesiones `remote_queue_full` (indicador activado por `OP_QUEUEFULL` UDP)
  se unen a los criterios de `purge_excess_sources` — eliminación de la
  fila fuente + `disconnect_peer` de la sesión en vivo (`RemoveSource`
  destruye el download client). Correspondencia de dirección exacta y luego IP
  sola para las fuentes `lowid:`/`kadcb:`. Corrección anexa: en
  igualdad `count == umbral` eMule sigue purgando → presupuesto
  `count - umbral + 1`.

## 9. Ancho de banda / USS (`UploadBandwidthThrottler.cpp`, `UploadQueue.cpp`, `LastCommonRouteFinder.cpp`) — auditoría fase B, pasada 4

### Fidelidad verificada (sin corrección necesaria)

- **Ley de control USS** (`services/uss.rs`): `soll_ping`, `hping`,
  divisores, mediana móvil, línea base de 10 sondas, suelo de 20 ms,
  tolerancia %/ms, límites min/max, re-baselining tras 60 fallos —
  conforme a `LastCommonRouteFinder.cpp` (`Ping()` + hilo de control).
- **Cadencia de sondeo**: ~1 % del caudal de subida, acotada 125–1000 ms
  (idéntica a eMule).
- **Descubrimiento de objetivo**: traceroute ICMP a TTL crecientes mediante
  `IcmpSendEcho` (API sin privilegios de Windows, como eMule), último salto
  común a las rutas — `uss_icmp::last_common_host`.
- **`AcceptNewClient`** (`upload_queue.rs::can_open_slot`): veto USS,
  4 slots garantizados, fórmula del caudal objetivo por cliente
  (`GetTargetClientDataRate`), guarda de caudal medido frente a caudal mínimo —
  portado fielmente.
- **`MinUpload`**: `MIN_UPLOAD_BPS = 1024` = valor por defecto eMule `MinUpload`
  (1 KiB/s, acotado ≥ 1 en `Preferences.cpp:1942`).
- **Ratio UL/DL** (`emule_ud_ratio`): coeficientes 3/4/5 y umbral
  20 KiB/s — espejo de `GetMaxDownloadInBytesPerSec`.
- **USS desactivado → restauración del límite manual** en el bucket de
  subida; bucket de descarga reducido mediante el ratio en cada tick USS.

### Correcciones de alineación

- **Pacing `ForceNewClient`** (`UploadQueue.cpp:417-421`): 1 s mínimo
  entre dos aperturas de slot mientras el caudal medido < 100 KiB/s
  (`m_nLastStartUpload` soportado por `last_upload_start_ms`) — antes, una
  cola llena abría todos los slots permitidos en ráfaga al arrancar.
  Nunca temporización bajo `MIN_UP_CLIENTS_ALLOWED` (2) ni por encima
  de 100 KiB/s medidos. Aplicado a ambas rutas de apertura
  (`AddClientToQueue` directo + promoción desde la cola).

### Desviaciones asumidas

- **Sin `GetSlotLimit` / round-robin por tick**: eMule planifica
  el envío socket por socket en una ventana temporal de 100 ms
  (hasta `GetSlotLimit()` sockets estándar por ciclo, paquetes
  ≥ 536/1300 B). Nuestro modelo: escritores por conexión drenando un
  token bucket global compartido — saturación equivalente, equidad por
  conexión diferente (sin round-robin explícito entre sockets
  estándar y trickle).
- **Rama dinámica de `GetMaxDownloadInBytesPerSec(bDynamic)`**:
  eMule utiliza el `GetDatarate()` **medido** cuando DynUp está activo y
  la cola no está vacía; nosotros utilizamos el límite configurado/actual USS
  (`ul_eff`). Desviación pequeña: bajo carga el valor USS converge hacia el
  caudal medido.
- **Fallback de sonda USS = TCP-connect RTT**: eMule también puede hacer ping
  en UDP (socket UDP bruto) cuando ICMP falla; nosotros recurrimos a una
  conexión TCP hacia un par Kad/servidor. Fuera de Windows, ICMP ausente →
  fallback TCP sistemático.
- **Contabilización del caudal**: eMule distingue file-data y overhead
  protocolario en `GetDatarate`; nuestro `effective_rate_bps` cuenta
  los bytes de socket (overhead incluido) — sobreestima ligeramente el payload.

## 10. Persistencia `.met` (`mule-format`) — auditoría fase B, pasada 4

### Fidelidad verificada

- **`.part.met`**: versiones `0xE0`/`0xE1`/`0xE2` (large file),
  cabecera `mod_date + filehash + u16 part_count + part_hashes`,
  tags gap nombrados `\x09<idx>`/`\x0A<idx>` (`SavePartFile`),
  `FT_FILESIZE_HI` fusionado en lectura, escritura atómica tmp+rename,
  `FT_AICH_HASH` base32, `FT_DLPRIORITY`/`FT_MAXSOURCES` escritos y
  releídos al importar (`PR_AUTO` restaura el modo auto). Importación eMule →
  Rust: gaps invertidos en rangos completados, partes rehasheadas, discordancia
  desalojada hacia gaps — conforme a `LoadPartFile` + `UpdatePartsInfo`.
- **`server.met`** y **`nodes.dat`**: formatos cubiertos.

### Desviaciones asumidas (ya documentadas §7, recordatorio)

- Tags `.part.met` no escritos: `FT_STATUS`, `FT_CORRUPTEDPARTS`
  (corrupción = gaps + blackbox SQLite), `FT_LASTSEENCOMPLETE`,
  `FT_DL_ACTIVE_TIME`, `FT_PARTFILENAME` (ruta destino en SQLite),
  `FT_AICHHASHSET` (hashset parcial reconstruido mediante recuperación AICH).
  eMule lee nuestros archivos sin estos tags (gaps + hashset MD4 bastan para
  `UpdatePartsInfo`).
- **`known.met` no implementado**: la persistencia de archivos compartidos
  vive en SQLite (`shared_files`), sin archivo compatible con eMule ni
  herramienta de importación `known.met` → Rust. Consecuencia: las estadísticas
  all-time (transferred/requested/accepted) y las marcas temporales de
  última publicación Kad de un eMule existente no migran — los
  archivos se vuelven a compartir desde cero (raíz AICH recalculada en el primer hash
  completo).
- Sin purga parcial `ShouldPartiallyPurgeFile`: sin `known.met`,
  no hay metadatos de larga duración que podar — la poda de entradas
  SQLite sigue la política local.

---

## 11. A4AF socket-level (`SwapToAnotherFile`/`DoSwap`) — auditoría fase B, pasada 5

### Implementación (`peer_session/a4af.rs`, `connection.rs`)

- `run_status_transfer_with_a4af`: bucle compartido por
  `run_peer_connection` y `run_callback_session` — ante `PeerError`
  `NoNeededParts` o `FileNotFound` (`is_swap_eligible`, eMule llama a
  `SwapToAnotherFile` desde `ProcessFileStatus`, `SendBlockRequests`,
  `UDPReaskFNF`, `OP_FILEREQANSNOFILE` `ListenSocket.cpp:428`), el mismo
  socket reproduce `query_file_status_and_enter_queue` para el archivo
  objetivo en lugar de morir. Acotado a `MAX_A4AF_SWAPS = 4` swaps
  consecutivos (eMule no tiene cota — `DontSwapTo` + temporización de
  reask; nuestro tope cubre el ping-pong patológico).
- `find_swap_target`: candidatos = descargas activas
  (`queued`/`waiting_for_sources`/`downloading`, `PS_READY|PS_EMPTY`
  eMule) que listan la dirección como fuente, por debajo de su `max_sources`
  (`GetSourceCount() < GetMaxSources()`). Ordenación: fuentes limpias
  (`m_OtherRequests_list`) antes que marcadas NNP
  (`m_OtherNoNeeded_list`), luego prioridad de descarga
  (`RightFileHasHigherPrio`) y después hash determinista.
  `Rejected`/`Filtered` excluidos; **cooldown aplicado a todos los
  estados** — una fuente NNP (`Allowed` + 58 min) no es reelegida
  en la misma cadena (`DontSwapTo`/`GetTimeUntilReask`).
- `apply_swap`: contabilidad `DoSwap` — FNF → `Rejected` (deadlist,
  `bRemoveCompletely=true`), NNP → `Allowed` + cooldown 58 min
  (`FILEREASKTIME * 2`), archivo objetivo → `Connected`
  (`SwapTo->srclist.AddTail(this)`).
- `try_a4af_swap`: purga los `in_flight_blocks` del par
  (`m_PendingBlocks_list` abandonado), actualiza la instantánea de sesión
  (`SetRequestFile`), emite el evento.
- **Corrección anexa**: el desmontaje de sesión escribe el resultado sobre
  `snapshot.file_hash` (archivo servido en último lugar), no sobre el hash
  marcado — eMule `RemoveSource` se aplica a `m_reqfile`; sin eso la
  contabilidad del swap quedaba sobrescrita y la fuente objetivo permanecía
  `Connected` para siempre.
- **Corrección anexa**: `WaitingForSources` serializa
  `"waiting_for_sources"` — el filtro de estados del swap coincide con los
  nombres reales de `DownloadStatus::as_str`.

### Test de integración

`test_peer_connect_a4af_socket_swap_on_fnf` (mock TCP loopback):
`OP_FILEREQANSNOFILE` sobre A → el mismo socket recibe `OP_REQFILENAME`/`OP_SETREQFILEID`
para B, transferencia real (≥1024 B inyectados en B), fuente A `rejected`.

### Desviaciones asumidas

- Sin `ProcessA4AFClients` periódico (`ClientList.cpp:846`) ni
  swap proactivo al descubrir una nueva fuente
  (`CheckAndAddSource`/`CheckAndAddKnownSource`): nuestros swaps siguen
  desencadenados por NNP/FNF en sesión, no por reevaluación periódica
  de la lista A4AF de un cliente inactivo — cubierto por el re-marcado normal.
- La discordancia de hash AICH (`ProcessAICHFileHash` eMule → swap
  `bRemoveCompletely`) no hace swap: nuestro modelo AICH es por votación
  (`record_aich_root_vote`) — un voto divergente no mata la sesión.
- Sin `DontSwapTo` temporal ni `aggressiveSwapping`: el tope
  `MAX_A4AF_SWAPS` + exclusión del hash actual + cooldowns ya acotan
  el ping-pong.

## 12. LowID↔LowID (`DS_LOWTOLOWIP`/`DS_WAITCALLBACK`, `CanDoCallback`, `IsLocalServer`) — auditoría fase B, pasada 6

Referencias: `Emule.cpp:1056-1085` (`IsFirewalled`/`CanDoCallback`),
`BaseClient.cpp:1342-1457` (`TryToConnect`), `PartFile.cpp:2368-2403`
(`CanAddSource`, ingestión), `PartFile.cpp:2242-2261` (purga DS_LOWTOLOWIP
30 s), `ServerConnect.cpp:519` (`IsLocalServer`),
`DownloadClient.cpp:191-203` (aparcamiento `m_bReaskPending`).

### Implementado

- **`CallbackCtx`** (`swarm_coordinator/lowid_callback.rs`): contexto de
  accesibilidad por tick — `server_connected`, `server_high_id`, endpoint
  del servidor conectado, `kad_open` (`CKademlia::IsConnected() &&
  !IsFirewalled()` = socket UDP activo + `UdpFirewallStatus::Open`).
- **`can_do_callback`**: portado exacto de `CemuleApp::CanDoCallback` —
  sin Kad abierto: `server_high_id` solo; Kad abierto + nosotros LowID:
  rechazo de los LowID **del mismo servidor** (retransmisión sobre nosotros mismos = violación
  de protocolo, baneo en eMule).
- **`is_local_server`**: los LowIDs solo son únicos por servidor —
  `OP_CALLBACKREQUEST` solo se emite para las fuentes cuyo servidor
  registrado (`PeerSource.server_ip`/`server_port`, nuevos campos
  persistidos, rellenados por SX en el cable y por el endpoint del servidor
  que responde TCP/UDP) coincide con nuestro servidor conectado. `server_ip == 0` →
  local solo si `origin == Server`; `server_port == 0` → coincidencia IP
  sola (respuestas UDP globales).
- **`DS_LOWTOLOWIP`**: fuente `lowid:`/`kadcb:` sin `can_do_callback`
  → ignorada al marcar (aparcada, reevaluada en cada tick cuando nuestro estado
  cambia — `DS_ONQUEUE` eMule). Las `lowid:` extranjeras (servidor
  diferente) mientras somos accesibles → `Rejected` (deadlist), como
  eMule `Disconnected("LowID: No Callback Option available")`.
- **Rechazo en la ingestión** (`IsFirewalled` = ni HighID ni Kad abierto):
  `DownloadService.accept_lowid_sources` refrescado por el coordinador,
  aplicado en el embudo `upsert_peer_sources` para todos los canales
  (`CPartFile::CanAddSource` `IsLowID && IsFirewalled` → descartar); el
  filtro SX existente gana también la exención `kad_open` (`SxLocalIds`).
- **`DS_WAITCALLBACK`**: callbacks pendientes existentes (TTL 5 min,
  adopción entrante por `client_id`/`IP`) — sin cambios, conforme.
- **Purga**: `unreachable_lowid` extendido a `!can_do_callback` (cubre el
  caso mismo-servidor-LowID); eMule purga los DS_LOWTOLOWIP a ≥80 %.

### Desviaciones asumidas

- **`OP_DIRECTCALLBACKREQ` saliente no implementado**: eMule intenta en
  primer lugar un callback UDP directo sobre `GetConnectIP():GetKadPort()` cuando
  la fuente ya ha anunciado su IP real + `SupportsDirectUDPCallback` —
  información que una fuente `lowid:` aprendida por SX/servidor no lleva
  (`GetConnectIP() == 0` en eMule también). El caso solo aplica a
  LowIDs ya conectados en entrante (su IP real conocida) — en nuestro caso
  se convierten en fuentes `ip:port`. `OP_DIRECTCALLBACKREQ` entrante
  sí está tratado (rate-limit, gate Kad-firewalled, connect-back).
- **`m_bReaskPending`** (`DownloadClient.cpp:193-198`): eMule retrasa
  el re-ask de descarga de un LowID **en nuestra cola de subida**
  esperando su conexión entrante reutilizable — nuestro modelo solo tiene un
  rol por socket (upload XOR download), la conexión entrante no puede
  servir de sesión de descarga; aparcar equivale al resultado.
- **Cadencia de purga unificada 60 s** frente a eMule 30 s (LOWTOLOWIP) / 40 s
  (NNP) / 60 s (RQF) — eMule comparte `m_lastpurgetime` entre los tres
  criterios, nuestro intervalo único está entre esos valores.
- Callbacks pendientes de servidor no purgados al reconectar servidor — el
  TTL de 5 min cubre el cambio de espacio de nombres LowID.

---

## 13. AICH-keyword / Kad v9 / identidad 0.72a — auditoría fase B, pasada 7

### Implementado

- **`KADEMLIA_VERSION = 0x0a`** (`KADEMLIA_VERSION_CURRENT = 10`,
  `Opcodes.h:32`): anunciado en el hello Kad, `OUR_KAD_VERSION`,
  `KadContactWire.version` y los bits `MISCOPTIONS2`.
- **Identidad 0.72a**: `CT_EMULE_VERSION = 72<<10` y
  `client_version = 72` (`m_uCurVersionShort = MAKELONG(72,0)`).
- **`TAG_KADAICHHASHPUB (0x36)`**: BSOB de 20 bytes publicado en
  `KADEMLIA2_PUBLISH_KEY_REQ` únicamente hacia los contactos
  `version >= KADEMLIA_VERSION_AICH (9)` (`Search.cpp` —
  `PreparePacketForTags`) cuando el archivo compartido tiene un `aich_root`.
- **`TAG_PUBLISHINFO (0x33)`**: `UINT32` emitido en los resultados de
  búsqueda únicamente — recuento de los publicadores conocidos
  (`Indexed.cpp` `WriteTagListWithPublishInfo`).
- **`TAG_KADAICHHASHRESULT (0x37)`**: histograma BSOB
  `[count][popularity u8][hash20]×n` emitido en los resultados, limitado
  a 11 entradas (cota eMule ~250 B de carga), votos con popularidad nula
  filtrados.
- **Modelo `CKeyEntry` en `kad_udp::storage`**: por (keyword,
  file_hash) — registro de publicadores deduplicados por IP (refresco al
  republicar, TTL 24 h `KADEMLIAREPUBLISHTIMEK`, tope 100), votos AICH
  por publicador agregados en histograma, popularidad de nombres de archivo
  (`GetCommonFileName` — empate → más antiguo), confianza
  `GetTrustValue` por agregación `/24` (`PUBLISHPOINTSSPERSUBNET=10`)
  con contadores globales coherentes al refrescar/expirar/purgar; entradas
  trusted servidas antes que las no-trusted (`SendValidKeywordResult`,
  `start_pos` tras el filtrado).
- **Filtrado de tags resultado**: `0x33`/`0x37` (y duplicados `0x36`
  más allá del primero) extraídos del almacenamiento — `m_listTag` conserva
  `TAG_SOURCES` y los tags estándar; `FILENAME`/`FILESIZE` viven
  en los campos dedicados (`Indexed.cpp` interceptación al publicar).
- **Orden de tags en la emisión** (`WriteTagListWithPublishInfo`):
  `FILENAME`, `FILESIZE`, taglist almacenada, `PUBLISHINFO`,
  `KADAICHHASHRESULT`.

### Desviaciones asumidas

- **Bits `MISCOPTIONS2` no anunciados**: bit 11 (Captcha — decodificado pero
  sin respuesta implementada) y bit 12 (`DirectUDPCallback` saliente —
  cf. §12). Anunciar una capacidad no implementada degradaría
  la interoperabilidad real.
- **Voto AICH por IP, no por publisher-ID**: el registro deduplica
  por dirección IP (eMule utiliza `CKadEntry` indexado IP+puerto; la
  deduplicación por IP sola es ligeramente más estricta — varios
  clientes NAT detrás de una IP cuentan como un publicador, como el agregado
  `/24` de eMule ya los aproxima).
- **`GetCommonFileName` empate → más antiguo**: eMule conserva el
  primer nombre con popularidad máxima — un renombrado solitario no
  reemplaza el nombre consensuado (test `keyword_store_lookup_and_dedup`
  alineado con esta semántica).

---

## 14. `known.met` — parser e importación de migración (auditoría fase B, pasada 8)

### Implementado

- **`mule_format::met::known_met`** — lectura fiel de
  `CKnownFileList::LoadKnownFiles` (`KnownFileList.cpp:73`):
  cabeceras `MET_HEADER (0x0E)` / `MET_HEADER_I64TAGS (0x0F)`, diseño
  `[date u32][hash16][u16 parts][hashes][u32 tagcount][tags]`,
  extracción de todos los campos de `LoadTagsFromFile` (FILENAME,
  FILESIZE, ATTRANSFERRED±HI, ATREQUESTED, ATACCEPTED, ULPRIORITY con
  resolución `PR_AUTO → PR_HIGH`, KADLASTPUBLISHSRC/NOTES, FLAGS,
  AICH_HASH base32, LASTSHARED). Verificación final fiel
  (`HasExpectedMD4HashCount`: `floor(size/PARTSIZE)+1`, y
  `CalculateMD4HashByHashSet`: raíz = hash único o MD4 de la
  concatenación) — los registros defectuosos se cuentan como `dropped_records`
  al igual que eMule los elimina. Cotas: `MAX_PART_HASH_COUNT`
  (`MAX_EMULE_FILE_SIZE/PARTSIZE+1`), registros ≤ 500 k, tags ≤ 1 k.
- **`mule-cli InspectKnownMet`** — inspección del archivo (registros,
  descartes, metadatos por entrada).
- **`shares.importKnownMet {metPath, directory?, verify?}`** —
  migración: unión `(fileName, fileSize)` (única clave común,
  known.met no almacena ruta) sobre los archivos compartibles de la
  carpeta; registro del compartido bajo el hash eMule, part hashes,
  `aich_root`, prioridad de subida (`PR_*` → cadenas) y estadísticas
  `share_upload_stats`. Compartidos existentes (coincidencia por hash): enriquecido
  solo con `aich_root`+stats, nunca reinicio de la configuración del usuario.

### Desviaciones asumidas

- **`verify` por defecto `false`**: la importación confía en los
  hashsets escritos por eMule — evitar re-hashear toda la carpeta
  Incoming es precisamente el interés de la herramienta. El propio eMule
  re-hashea siempre (`CAddFileThread::Run` — `m_tUTClastModified` se
  ignora); `verify=true` reproduce este comportamiento y solo retiene
  los registros cuyo hash recalculado coincide (`mismatched`).
- **Sin hash-cache persistente por ruta**: eMule tampoco lo usa
  en 0.72a — `known.met` solo se consulta como índice de
  archivos ya hasheados, nunca para omitir el hasheo al arrancar.
- **`FT_AICHHASHSET` (0x35) leído pero no conservado**: el hashset de
  recuperación AICH por parte es verificado por eMule y luego servido a los pares;
  nuestro modelo solo conserva la raíz — el conjunto de recuperación se
  reconstruiría desde el archivo si fuera necesario.

---

## 15. Cola de espera remota — cadencia, multipacket, identidad y duración de vida del socket (pasada 9)

### Cadencia del re-ask UDP — fiel a `PartFile.cpp:2291-2294`

El ping `OP_REASKFILEPING` ya no sigue una cadencia fija ~21 min:
`wait_in_queue` publica la fecha límite del próximo re-ask TCP
(`PeerSessionSnapshot.next_tcp_reask_unix_ms` +
`queued_peers[addr].next_tcp_reask_unix_ms`), y la estrategia solo emite el
ping en la ventana `(1 s, 2 min)` previa a esa fecha límite, con
`LastTriedToConnect + 20 min` mínimo y `!IsFirewalled()`. Las guardas
de fallo UDP (`> 3` intentos, ratio `> 30 %`) se conservan.

### `OP_MULTIPACKET*` — `SendFileRequest` agrupado (`DownloadClient.cpp:266`)

- `PeerSxCaps.multipacket` (`MultiPacketLevel` Basic/Ext/Ext2) deducido de los
  bits `SupportMultiPacket` (MISCOPTIONS1.1), `SupportExtMultiPacket`
  (MISCOPTIONS2.5), `SupportsFileIdentifiers` (MISCOPTIONS2.13) — prevalece
  la variante más alta anunciada, como en eMule.
- `file_request.rs` construye el paquete `SendFileRequest` único:
  `REQFILENAME` (+extinfo cuando `ext_req > 0`), `SETREQFILEID` **omitido para
  ≤ 1 parte** (regla eMule conservada), `REQUESTSOURCES(2)` (planificado mediante
  `planned_sx_request` factorizado, mismas puertas `IsSourceRequestAllowed`),
  `AICHFILEHASHREQ`.
- `OP_MULTIPACKETANSWER*` entrante se descompone en subpaquetes
  (`expand_multipacket_answer` + `VecDeque<RawPacket>`) que reutilizan
  los manejadores existentes — prueba socket-level
  (`test_peer_connect_multipacket_file_request`).

### `OP_CHANGE_CLIENT_ID` aplicado (`ListenSocket.cpp:590`)

El payload `[newUserID u32][newServerIP u32]` ya no solo se
registra: `LowID + servidor conocido` (serverlist → puerto) o
`newID == IP del par` (HighID) actualizan `server_ip`/`server_port`
de la fuente (`lowid:<id>` y/o `ip:port`) mediante
`update_peer_source_server` — los metadatos utilizados por
`IsLocalServer`. Contenido desconocido o truncado → ignorado, como eMule.

### Duración de vida del socket en cola — `CheckTimeOut` fiel

- eMule mata el socket tras `CONNECTION_TIMEOUT` (**40 s**) sin NINGÚN
  tráfico (`OnSend`/`OnReceive`/`SendPacket` reinician `timeout_timer`,
  `ListenSocket.cpp:120-155`); `DS_ONQUEUE` sobrevive **lógicamente** con
  `socket == NULL`. Nuestro `queue_timeout` (antes 15 min de silencio
  post-reask) adopta esta semántica: 40 s de inactividad,
  reiniciado en cada envío Y recepción.
- La muerte del socket en cola (descarte local o cierre remoto) ya
  **no** es un fallo: la fuente queda `Allowed` con cooldown =
  fecha límite de re-ask (`GetTimeUntilReask`), y la entrada `queued_peers`
  conserva el estado lógico — el ping UDP solo sale en este estado
  (`!(socket && socket->IsConnected())`, `DownloadClient.cpp:1344`),
  es decir nunca mientras viva una sesión TCP. Solo FNF/NNP
  (resultados terminales) eliminan la entrada — eMule mantiene `DS_ONQUEUE`
  para cualquier otra desconexión (`BaseClient.cpp:1126-1135`).

### Desviaciones asumidas

- **Socket compartido multi-rol**: eMule reutiliza una conexión aún
  viva (subida/chat) para el re-ask TCP sin reconectar; nuestras
  sesiones son mono-rol — el re-ask es siempre una reconexión,
  salvo si el par envía paquetes > 40 s (entonces la rama re-ask
  interna actúa como la ruta `DS_CONNECTED` de eMule).
- **`queued_peers` solo en memoria**: el rango de cola no se
  persiste entre reinicios (eMule tampoco lo restaura — un
  cliente reiniciado vuelve a `TryToConnect`).
- **Backoff frente a fecha límite al morir en transferencia**: una desconexión en
  `DS_DOWNLOADING` vuelve a pasar a `DS_ONQUEUE` en eMule (reask en fecha límite);
  mantenemos `Failed`+backoff (cadencia comparable ≤ 20 min).

## 16. Migración eMule — `emfriends.met`, `cancelled.met`, identidad (pasada 10)

Tres formatos de migración completan `known.met` (§14): amigos, archivos
cancelados e identidad (clave RSA + user hash) — la continuidad de los créditos.

### `emfriends.met` → `friends`

- Parser `mule_format::met::emfriends` fiel a `CFriend::LoadFromFile`
  (`Friend.h:42-43`): registro `[userhash 16][lastUsedIP u32][lastUsedPort
  u16][lastSeen u32][lastChatted u32][tagCount u32][tags]`; `FF_NAME`
  (0x01, primer valor retenido — `m_strName.IsEmpty()` eMule) y
  `FF_KADID` (0x02) reconocidos, otros tags ignorados.
- `friends.importEmfriendsMet {metPath}`: registros `!HasUserhash`
  (amigos solo-IP) reportados pero no importados — nuestro slot de amigo está
  indexado por hash, como la búsqueda `IsFriend` de eMule. `add_friend`
  es un upsert: la reimportación refresca el nombre (`updated`), hashes
  registrados ante el gestor de subida (`set_friend`).

### `cancelled.met` → `cancelled_files`

- Parser `mule_format::met::cancelled_met` fiel a `LoadCancelled`
  (`KnownFileList.cpp:167-194`): `0x0F` = `[seed u32][count u32]
  {[hash 16][tagCount u8][tags]}`, `0x0E` obsoleto = hashes reales.
  `cancelled_mask(seed, hash)` = `MD5(seed_le || hash)` — eMule no
  almacena nunca el hash real.
- Tabla `cancelled_files(masked_hash, seed)`: la semilla **acompaña a cada
  entrada** porque un archivo importado conserva su `m_dwCancelledFilesSeed`;
  las entradas `0x0E` se enmascaran bajo **nuestra** semilla al importar —
  conversión idéntica a la carga de eMule.
- `IsCancelledFileByID` → `is_cancelled_file`: recalcula la máscara
  bajo cada semilla conocida; **siempre falso** si `remember_cancelled`
  está desactivado (`IsRememberingCancelledFiles` eMule — misma puerta que
  el historial).
- `AddCancelledFileID` → `remove_download` registra el hash enmascarado
  de las descargas **no terminadas** (`CPartFile::Delete` no
  se aplica a los PartFile finalizados en eMule).
- Semántica eMule respetada: «cancelled» es un **marcado** de los
  resultados de búsqueda (`downloads.isCancelled`), no un bloqueo.

### Identidad — `cryptkey.dat` + `preferences.dat`

- eMule genera el user hash **aleatoriamente** y lo persiste en
  `preferences.dat` (`Preferences_Ext_Struct`: `[version u8][userhash
  16][WINDOWPLACEMENT]`, `Preferences.h:78-86`) — independiente de la clave
  RSA. Nuestro user hash derivado (`MD4(pubkey)` + magic bytes 0x0E/0x6F)
  no puede por tanto recuperar un hash eMule existente: la importación lee
  ambos archivos.
- `mule_format::met::preferences_dat`: extracción acotada del user hash
  (17 primeros bytes) + `is_bad_hash` (`OtherFunctions.h:380-383`) —
  un hash degenerado almacenado = archivo corrupto, rechazado.
- `identity.importEmuleIdentity {cryptkeyPath?, preferencesDatPath?,
  replace?}`: `cryptkey.dat` validado (`Base64(PKCS#1 DER)` RSA-384,
  mismo formato que el nuestro — copia atómica); user hash →
  `userhash.dat` (16 bytes brutos). `replace:true` requerido si los
  archivos existen — sobrescribir una identidad pierde sus créditos.
- `IdentityService::user_hash()`: la anulación importada prevalece (verbatim —
  los marcadores eMule ya están dentro y es la clave de los créditos);
  si no, el hash derivado. `rotate_key` **elimina** `userhash.dat`: el
  hash importado pertenece a la clave eMule reemplazada.
- Propagación en caliente idéntica a `rotate_identity` (`propagate_identity`
  factorizado): hellos, desafíos SUI, cortafuegos Kad, KadID — los
  créditos eMule son válidos sin reiniciar.

### Desviaciones asumidas

- `emfriends.met`: `lastUsedIP/Port`, `lastSeen`, `lastChatted`,
  `kadId` leídos pero no persistidos — nuestro `Friend` no los modela.
- `preferences.dat`: solo se importa el user hash — el resto del
  struct (ubicación de ventana) es GUI MFC.
- Marcado de los resultados de búsqueda conectado: `emit_search_results`
  añade `"cancelled"` por archivo (TCP `OP_SEARCHRES` y UDP
  `OP_GLOBSEARCHRES` mediante `search_result_cancelled_flags` →
  `is_cancelled_file`) — el marcado eMule `IsCancelledFileByID`
  (`m_MyListCtrlItem`); la API `downloads.isCancelled` sigue disponible.

### `clients.met` → `peer_credits`

- Parser `mule_format::met::clients_met` fiel a `LoadList`
  (`ClientCredits.cpp:156-233`): `[version u8][count u32]` y luego
  `CreditStruct` empaquetado — `0x12`: 119 B (`ClientCredits.h:41-52`),
  `0x11` (29a): 38 B sin pubkey. Otra versión → rechazo del archivo,
  como eMule.
- `credits.importClientsMet {metPath}`: nuestros créditos están indexados por
  `MD4(pubkey)` — `abySecureIdent[..nKeySize]` proporciona exactamente la
  clave RSA necesaria; registros `nKeySize == 0` reportados como
  `skippedNoPubkey` (créditos débiles eMule, no reclamables bajo SUI).
  Entradas `tLastSeen < now - 150 d` eliminadas al importar — el descarte de
  eMule al cargar (`ClientCredits.cpp:207-213`).
- Es el eslabón que hace útil `identity.importEmuleIdentity`: el
  user hash importado + la clave RSA importada permiten a los pares
  verificar SUI, y `clients.met` restaura lo que nos deben.

### Formatos eMule cubiertos — balance

| Archivo | Estado |
|---|---|
| `known.met` | ✅ parser + `shares.importKnownMet` (§14) |
| `part.met` | ✅ parser + `downloads.importPartMet` |
| `server.met` | ✅ parser + `servers.importMet` |
| `nodes.dat` | ✅ parser + `kad.importNodesDat` |
| `emfriends.met` | ✅ parser + `friends.importEmfriendsMet` |
| `cancelled.met` | ✅ parser + `downloads.importCancelledMet` |
| `clients.met` | ✅ parser + `credits.importClientsMet` |
| `cryptkey.dat` | ✅ `identity.importEmuleIdentity` |
| `preferences.dat` | ✅ user hash mediante `identity.importEmuleIdentity` |
| `shareddir.dat` | ⚠️ no previsto — sin funcionalidad de múltiples «carpetas compartidas» |
| `preferences.ini` | ⚠️ no previsto — ajustes GUI eMule fuera de alcance (gran superficie, bajo valor) |
| `addresses.dat`, `webservices.dat`, `statistics.*`, `AC_*.dat` | ⚠️ no previstos — GUI/configuración eMule marginal |

## 17. Callback UDP directo (`OP_DIRECTCALLBACKREQ`) + `m_bReaskPending` — auditoría fase B, pasada 11

### Sentido del mecanismo

`OP_DIRECTCALLBACKREQ` (`0x95`, `Opcodes.h:296`) sirve para contactar con un par
**TCP-inaccesible pero UDP-abierto** (NAT full-cone): el emisor — él mismo
TCP-accesible — envía un datagrama UDP al puerto Kad del destinatario,
que entonces marca una conexión TCP de retorno (`TryToConnect` paso 4,
`BaseClient.cpp:1403-1415`).

### Emisión (rol del solicitante)

- Condición eMule: `SupportsDirectUDPCallback() && GetUDPPort() != 0 &&
  GetConnectIP() != 0` — `SupportsDirectUDPCallback()` exige
  `m_fDirectUDPCallback && HasValidHash() && GetKadPort()`
  (`updownclient.h:211`). En nuestro caso: `PeerSource.crypt_options & 0x08`
  (byte connect-options, bit 3) + `udp_port != 0` + user hash presente.
- Payload en el cable `[PR_EMULE][0x95][tcp_port u16 LE][userhash 16][conn_opts u8]`
  (`CSafeMemFile` = little-endian) — codificador
  `client_udp::encode_direct_callback_req`, constante
  `CONNECT_OPT_DIRECT_CALLBACK = 0x08`.
- Orden eMule `TryToConnect`: **direct callback → server callback →
  buddy callback**. La estrategia intenta el direct callback antes que el relevo
  buddy (`kadcb:`) y recurre al buddy si el socket UDP local está
  ausente (`send_direct_callback_req` → `false`).
- El byte `conn_opts` = `GetMyConnectOptions(true, false)`: bits 0-2 crypt
  únicamente — el bit direct-callback queda a 0 (pedimos que nos llamen,
  no somos el receptor de retorno).
- Cifrado: eMule pasa `bEncrypt = ShouldReceiveCryptUDPPackets()`
  (capacidad UDP-crypt del **destinatario**) — nuestro `client_udp` envía en
  claro como el resto de sus datagramas (reask ping incluido): desviación
  asumida y documentada, la ofuscación UDP de cliente no está implementada.

### Recepción (rol del par tras cortafuegos)

- Puerta eMule: `AllowCalbackRequest(ip)` (ventana 180 s/IP,
  `ClientList.cpp:913-928`) y luego `CKademlia::IsRunning() &&
  CKademlia::IsFirewalled()` (`ClientUDPSocket.cpp:373`).
  `CKademlia::IsFirewalled()` = veredicto **Kad-TCP** (`CPrefs::
  GetFirewalled`, `Kademlia.cpp:324`) — NO el probador UDP: el receptor
  típico tiene el TCP cerrado y el UDP abierto, por lo que el estado UDP no debe
  bloquear la recepción. eMule vale `firewalled` por defecto mientras 2 contactos
  no hayan confirmado el TCP; nuestro proxy = `!is_high_id` (LowID
  asignado por el servidor) — `UdpFirewallStatus` ya no interviene (corrección
  de inversión: la puerta anterior exigía `Firewalled` UDP, lo que habría
  excluido precisamente a los receptores objetivo).
- Después `FindClientByUserHash`/`AddClient` + `SetConnectOptions(opts,
  true, false)` + `SetDirectUDPCallbackSupport(false)` + `TryToConnect()`
  → en nuestro caso: `InboundListenerService::connect_back` con `our_id`
  (el ID real de la sesión de servidor, como eMule envía su ID real en
  `TryToConnect`).

### Capacidad anunciada

- Saliente eMule (`BaseClient.cpp:1007`, `GetMyConnectOptions` bit 3 /
  MISCOPTIONS2 bit 12): `theApp.IsFirewalled() && KadRunning &&
  !IsFirewalledUDP(true) && IsVerified()` — neto: **sin HighID eD2K**
  + UDP Kad probado abierto. Bits 12 de los hellos (outbound + answer inbound)
  y bit 3 del `TAG_ENCRYPTION` publicado (`Search.cpp:692` —
  `GetMyConnectOptions(true, true)`) siguen esta condición.
- `PeerSource.udp_port` (columna SQLite añadida, `ALTER TABLE` idempotente
  + `CASE WHEN excluded.udp_port != 0` sticky) = eMule `GetKadPort()`:
  alimentado por `KadSourceWire.udp_port` (búsqueda), el tag
  `CT_EMULE_UDPPORTS` del hello (`peer_udp_port`), 0 si no.
- Ingesta Kad: una fuente `tcp_port == 0` con el bit direct-callback se
  conserva **sin buddy** (tipo 6, `DownloadQueue.cpp:1596`) — antes se
  exigía `buddy_id`, lo que descartaba estas fuentes.

### `m_bReaskPending` (`DownloadClient.cpp:185-206`)

- Al re-ask de un LowID **ya solicitado** (`GetLastAskedTime() > 0` ↔
  entrada `queued_peers`) **y presente en nuestra cola de subida**
  (`US_ONUPLOADQUEUE` ↔ `waiting_rank_by_ip`), eMule no marca:
  `DS_ONQUEUE` + `m_bReaskPending`, y deja al par un ciclo para venir
  con su propio re-ask de subida. El indicador se consume en la conexión
  entrante (`ConnectionEstablished` → `SendFileRequest` inmediato,
  `BaseClient.cpp:1552`); la rama `!CanDoCallback` (LowID↔LowID) ya
  está cubierta por nuestra puerta `can_do_callback` (pasada 9).
- Implementación: `QueuedPeerState.reask_pending` +
  `peer_session.defer_reask_for_queued_peer(ip, hash, next)` (una sola
  activación, `!m_bReaskPending`) / `take_reask_pending(ip)` (consumido en la
  conexión entrante en `dispatch_requests` → `OP_DIRECTCALLBACKREQ`
  inmediato + `register_pending_kad_callback`). La fecha límite
  `next_tcp_reask_unix_ms` se empuja un ciclo `FILE_REASK_INTERVAL`
  para mantener coherente la ventana UDP.
- Límite: solo opera para las fuentes con IP real conocida (`kadcb:`) —
  las fuentes `lowid:` no llevan la IP real del par (el campo `ip`
  contiene el ID LowID) y no pueden emparejarse con una entrada de
  cola de subida por IP, como haría el `Compare` por userhash de eMule
  si se conociera el hash (las fuentes de servidor no lo llevan).

### Restos

- El byte `TAG_ENCRYPTION` 0xF3 de los contactos publicados (**bit 3**) ya
  se respeta en la ingesta y en la emisión; `KadPublishSourceReq.
  encryption` expuesto en la decodificación (round-trip probado).
- eMule comparte el indicador `m_bReaskPending` con el socket saliente
  (`ConnectionEstablished` → `SendFileRequest` sobre el socket entrante);
  nuestro split inbound/outbound hace el equivalente mediante callback inmediato.

## 18. Paridad `CT_EMULE_MISCOPTIONS2` en todos los hellos — auditoría fase B, pasada 12

### Divergencia corregida

El hello del **connect-back** (`inbound_listener/connection.rs`, tras un
`OP_CALLBACKREQUESTED`/`OP_DIRECTCALLBACKREQ`) solo ponía los bits
crypt (7-9) y omitía el tag entero sin ofuscación — el par remoto
veía `dwEmuleTags` sin el bit misc2 → cliente clasificado como «viejo eMule».
eMule reutiliza el mismo `SendHelloPacket` para esta conexión.

- `services/hello_caps.rs::misc_options2()` — fuente única de la palabra
  misc2: `FileIdent<<13 | DirectUDPCallback<<12 | SrcExch2<<10 |
  Requires<<9 | Requests<<8 | Supports<<7 | ExtMultiPkt<<5 |
  LargeFiles<<4 | KadVersion<<0`. Bit 11 (chat CAPTCHA) siempre 0.
- Aplicado a los 3 hellos: outbound (`peer_session/handshake.rs`),
  respuesta inbound y connect-back (`inbound_listener/connection.rs`).
- Bit 12 = `tcp_low_id && udp_verified_open` (verified && Open),
  semántica `GetMyConnectOptions(true,true)` + `!IsFirewalledUDP &&
  IsVerified` de `BaseClient.cpp:1007`.
- El hello minimal del test de cortafuegos UDP (`kad/firewall.rs`) permanece
  aligerado por diseño (socket de diagnóstico, no una sesión de par).

### Higiene de las fixtures de prueba

- Los literales de puertos en el rango eMule (`4661`-`4665`, `4672`, `4673`)
  de las fixtures **sintéticas** se desplazan al rango efímero
  `49661`-`49675` (≥ 49152): estos puertos están históricamente filtrados por
  algunos ISP y el propio eMule sortea un puerto aleatorio
  (`Preferences.cpp:1969`, `GetRandomTCPPort` = `4096+rand%61000`) en el
  primer arranque — nunca 4662 por defecto.
- Conservados voluntariamente: los vectores **capturados de eMule real**
  (`ed2k_corpus_tests.rs`, `manifest.json`, `server_codec_tests.rs`,
  `server_met_vectors.rs`) donde el puerto forma parte de la fidelidad en el cable,
  y `4660` = el puerto RPC por defecto del daemon (fuera de protocolo eMule).
- Valor por defecto de producción sin cambios: `DaemonConfig` ya escucha en
  `39735`/`39737` (fuera del rango eMule, en la banda de aleatorización).

## 19. Exploración de archivos compartidos (`OP_ASKSHARED*`) — auditoría fase B, pasada 13

### Constatación

Nuestro `MISCOPTIONS1` anunciaba `NoViewSharedFiles = 0` (compartidos visibles,
valor por defecto eMule `vsfaEverybody`) pero el despachador inbound descartaba las tres
peticiones de exploración — un par eMule que «exploraba» nuestros compartidos recibía
silencio. Implementado: `ListenSocket.cpp:651-825`.

### Comportamiento (fiel a eMule 0.72a)

| Petición (PR_ED2K) | Respuesta | Rechazo |
| :--- | :--- | :--- |
| `OP_ASKSHAREDFILES` `0x4A` (vacío) | `0x4B`: `<u32 count><offered>*` | **nunca 0x61** — eMule responde la lista vacía (`ListenSocket.cpp:659-683`) |
| `OP_ASKSHAREDDIRS` `0x5D` (vacío) | `0x5F`: `<u32 count>(<u16 len><dir>)*` | `0x61` vacío (`ListenSocket.cpp:701`) |
| `OP_ASKSHAREDFILESDIR` `0x5E`: `<u16 len><dir>` | `0x60`: `<u16 len><dir eco><u32 count><offered>*` | `0x61` vacío |

- **Puerta `CanSeeShares()`** — ajuste `viewSharedFiles` =
  `everybody`/`friends`/`nobody` (persistido `view_shared_files`, por defecto
  `everybody` como eMule). `friends` filtra sobre el user hash del par
  (`CFriend` tiene clave en `m_abyUserHash`).
- **Pseudodirectorios** — nunca una ruta real en el cable: basename del
  directorio contenedor (`GetPseudoDirName`, deduplicado), luego
  `!Incomplete Files` si se comparten partes y luego `!Other` para los
  compartidos sin directorio (`SendSharedDirectories`,
  `BaseClient.cpp:2916`).
- **`0x60` eco** — la cadena solicitada se devuelve verbatim
  (`strOrgReqDir`) aunque el pseudodirectorio sea desconocido → `count = 0`
  (`ListenSocket.cpp:738-787`).
- **Entradas ofrecidas** — variante cliente de `CreateOfferedFilePacket`
  (`pServer == NULL`): `client_id`/`port` reales **únicamente** en HighID,
  `FT_FILESIZE` en `TAGTYPE_UINT64` solo si el par anuncia large-files
  (misc2 bit 4), si no la entrada >4 GiB se salta; `FT_FILERATING`
  opcional (0..5 → 0..255, `SharedFileList.cpp:960`) — no emitido, nuestro
  modelo no tiene valoración.
- **Archivos parciales** — los `.part` activos aparecen bajo
  `!Incomplete Files` (PS_READY + filtro large-files en eMule;
  `is_partial` por nuestra parte — el servicio de bloques sigue acotado a las partes
  verificadas, ya garantizado por la ruta de subida).

### Colisiones de opcodes (documentadas)

`0x4B`/`0x60`/`0x61` comparten su valor con `OP_REQUESTPARTS`,
`OP_QUEUERANKING` y `OP_FILEDESC`. eMule los separa por dirección
(respuestas host→browser) y por byte de protocolo (`QUEUERANKING`/`FILEDESC`
se emiten en `OP_EMULEPROT` — `UploadClient.cpp:565/591`). Todas nuestras
respuestas salen en PR_ED2K (`Packet(tempfile)` → `OP_EDONKEYPROT`).

### Desviación asumida

Payload `0x5E` truncado (`<u16 len>` fuera de límites): eMule lanza una
excepción `CFileException` y no responde nada; nosotros respondemos `0x61`
(denied) — acotado, determinista y sin decodificación parcial.

### Pruebas

- Codec (`mule-protocol::ed2k::shared_files`): diseño `0x4B`/`0x5F`/`0x60`,
  `0x61` vacío, decodificación `0x5E` acotada (truncamiento, bytes finales tolerados),
  reglas HighID/large-file/rating.
- Daemon (`share_browser`): dedup de pseudodirectorios, `!Incomplete`/`!Other`,
  puerta large-files.
- Integración `test_inbound_shared_files_browse` (loopback): hello →
  `0x4A`→`0x4B` (hash del archivo presente), `0x5D`→`0x5F`, `0x5E` sobre
  `!Incomplete Files`→`0x60` eco + lista vacía, luego
  `viewSharedFiles=nobody`→`0x61`.

## 20. Veredicto cortafuegos TCP Kad (`FIREWALLED{2}_REQ` saliente + `0xA8` entrante) — auditoría fase B, pasada 14

### Constatación

Respondíamos a las pruebas de los demás (`FIREWALLED_REQ`/`REQ2` → `FIREWALLED_RES`
+ connect-back + `OP_KAD_FWTCPCHECK_ACK`/`FIREWALLED_ACK_RES`), pero nunca
**pedíamos** ninguna prueba nosotros mismos e ignorábamos los veredictos recibidos —
`GetFirewalled()` quedaba indeterminado, el bit 1 de `KADMISCOPTIONS` nunca
activado.

### Máquina de estados implementada (`CPrefs` + `CClientList`)

Estado en `FirewallTestTracker` (`kad_udp/types.rs`):

| Campo | eMule | Rol |
| :--- | :--- | :--- |
| `tcp_acks` | `m_uFirewalled` | ACKs de connect-back vistos en la ronda |
| `tcp_answers` | `m_uRecheckip` | `FIREWALLED_RES` recibidos (ronda activa mientras < 4, `KADEMLIAFIREWALLCHECKS`) |
| `tcp_last_state` | `m_bLastFirewallState` | instantánea del veredicto al abrir la ronda (init `true`) |
| `pending_tcp_ips` | `listFirewallCheckRequests` | IPs solicitadas, TTL 180 s |
| `next_tcp_recheck` | `m_tNextFirewallCheck` | recomprobación horaria |

- `tcp_is_firewalled()` = `GetFirewalled()`: `< 2 ACKs` → último estado mientras
  dure la ronda, `firewalled` una vez devueltas las 4 respuestas.
- El arranque comienza en ronda activa (`tcp_answers = 0`) — la primera comprobación
  «done on connect» de eMule es gratuita para nosotros: cada `HELLO_REQ`/
  `HELLO_RES` entrante desencadena `FirewalledCheck` hacia el remitente.
- `FIREWALLED2_REQ` (Kad ≥ 7): `<u16 tcp><u128 user_hash><u8 opts>` —
  **user hash eD2K** (`GetClientHash`), no el KadID aleatorio; legacy
  `FIREWALLED_REQ` (`[u16 tcp]`, 0x50) para los contactos v6−. Nuevo
  codificador `encode_firewalled_req` añadido (la v2 ya existía).
- `FIREWALLED_RES` recibido: ignorado si la IP no está en `pending_tcp_ips`
  (eMule descarta «unrequested firewall response»), si no `observed_ip`
  + tick de ronda.
- `KADEMLIA_FIREWALLED_ACK_RES` (UDP, < v7): `inc_tcp_acks()` directo,
  como eMule.
- `OP_KAD_FWTCPCHECK_ACK` (0xA8, PR_EMULE, entrante TCP): nueva rama
  del despachador inbound, puerta `is_tcp_check_ip(ip)` → `inc_tcp_acks()`,
  si no ignorado (`Unrequested` debug log, como eMule).
- `HELLO_RES` anuncia ahora bit 1 de `KADMISCOPTIONS` cuando
  `tcp_is_firewalled()` — la superficie en el cable visible del veredicto.
- Bucle de mantenimiento 30 s: `next_tcp_recheck` vencida →
  `start_tcp_recheck()` + relanzamiento del test UDP (eMule `RecheckFirewalled`
  rearma ambos a `HR2S(1)`).

### Desviación asumida

eMule reemite un REQ en cada HELLO de un mismo contacto mientras la ronda
está en curso; saltamos el reenvío si la IP ya tiene una petición < 180 s en curso —
`AddKadFirewallRequest` sigue siendo idempotente, pero sin spam de REQ.

### Pruebas

- Unitarias (`types.rs`): ciclo de vida de la ronda (activa al arranque, veredicto
  último-estado durante la carrera, firewalled tras 4 respuestas sin ACK),
  cambio a 2 ACKs, instantánea `start_tcp_recheck`, TTL de las IPs.
- Integración `test_kad_tcp_firewall_check_round` (loopback UDP):
  HELLO_REQ → HELLO_RES **+ FIREWALLED2_REQ** (payload port/hash/opts
  verificados), `FIREWALLED_RES` tick + `observed_ip`, 2× `ACK_RES` → veredicto
  abierto.
- Integración `test_inbound_kad_fw_tcpcheck_ack` (loopback TCP): IP
  registrada → `0xA8` PR_EMULE → `tcp_acks` contados, veredicto cambia.

## 21. Descripción de servidor (`OP_SERVER_DESC_REQ/RES`) — auditoría fase B, pasada 15

Referencia: `UDPSocket.cpp:410-508`, `Opcodes.h:204-208`,
`CServerList::GetServerByIPUDP`, `CServer::SetLastDescPingedCount`.

### Constatación

El canal UDP de servidor enviaba `GLOBSERVSTATREQ` y trataba la respuesta,
pero se detenía ahí: eMule encadena con `OP_SERVER_DESC_REQ` (0xA2) para
recuperar nombre de visualización, descripción, dynIP y versión del eserver —
los campos `name`/`description`/`dyn_ip`/`version` de `Ed2kServer`
permanecían vacíos de por vida.

### Protocolo implementado

- **Petición**: desafío u32 = `(rand16 << 16) | INV_SERV_DESC_LEN`
  (`0xF0FF`) — los dos primeros bytes en el cable forman una longitud de
  cadena no válida, lo que permite al servidor distinguir el nuevo
  formato del antiguo (eserver ≥ 16.45 responde `[challenge][u32
  tag_count][tags]`).
- **Cadencia**: `desc_ping_counts` reproduce `LastDescPingedCount` — ++ en
  cada `GLOBSERVSTATRES` aceptada, `DESC_REQ` mientras < 2, reinicio a 2
  (≈ un ping de cada dos pide la descripción).
- **Respuesta formato nuevo**: aplicada únicamente si el desafío
  corresponde a una petición en curso (`m_uDescReqChallenge`) — las respuestas
  espontáneas se ignoran (caso de servidores múltiples misma IP, cf. nota
  eMule).
- **Respuesta formato antiguo**: `[u16 len][desc][u16 len][name]` — eMule
  lee la **descripción en primer lugar** (el comentario `Opcodes.h:205`
  documenta el orden inverso; el código prevalece, reproducimos el mismo
  orden — comportamiento «bug-compatible»).
- **Tags**: `ST_SERVERNAME`/`ST_DESCRIPTION`/`ST_DYNIP` (str) y
  `ST_VERSION` (str o u32 → renderizado `"maj.min"` con menor de 2 dígitos,
  `"%u.%02u"`). `ST_AUXPORTSLIST` ignorado como eMule.
- **`dyn_ip`**: conservado únicamente si no es una dirección IPv4
  numérica (`inet_addr() == INADDR_NONE` en eMule) — si no un servidor
  podría anunciarse bajo la IP de otro.
- **Resolución del servidor**: `GetServerByIPUDP` — misma IP y
  `udp_port == tcp_port + 4`; respuesta de un desconocido → descartar.

### Pruebas

- Codec `mule-protocol`: diseño del desafío, formato antiguo, formato nuevo
  con tags (nombre str + dynIP + versión empaquetada u32 `"17.15"`).
- Loopback `server_udp.rs`: STATRES aceptada → `DESC_REQ` emitido con
  desafío `*F0FF`, respuesta con desafío erróneo ignorada, respuesta
  concordante actualiza `Ed2kServer`, 2.º ciclo → sin petición (reinicio).

## 22. `OP_OUTOFPARTREQS` saliente + `OP_PUBLICIP_REQ` entrante — auditoría fase B, pasada 16

Referencia: `ListenSocket.cpp:583` (`OUTOFPARTREQS` entrante) y
`:1290-1310` (`PUBLICIP_*`), `UploadClient.cpp:484-496`,
`UploadQueue.cpp:309-313`, `BaseClient.cpp:2526-2543`.

### `OP_OUTOFPARTREQS` (0x57, PR_ED2K, vacío)

- **Recepción**: ya cubierta (alias `OP_OUTOFPARTS`) — la sesión de
  descarga termina limpiamente y la fuente vuelve al pool, lo que
  cubre el `DS_DOWNLOADING → DS_ONQUEUE` de eMule.
- **Emisión** (brecha corregida): cuando `process_tick` rotaba un slot
  (`CheckForTimeOver`, sesión > `SESSIONMAXTRANS` y cola disputada), el
  par quedaba conectado esperando bloques nunca enviados — la
  desincronización exacta que describe el comentario de eMule. A partir de ahora
  `rotation_notify(client_key)` entrega al despachador un `Notify` creado para
  las sesiones activas; al desalojar `process_tick` lo dispara y la
  conexión empuja `0x57` (PR_ED2K, vacío) en el socket aún abierto.
  `release_slot` purga la entrada sin notificar (fin normal ≠ rotación).
- El par **no** se vuelve a encolar automáticamente — regresa en su
  re-ask, como antes (desviación asumida frente a `AddClientToQueue(this, true)`:
  nuestra cola distingue clientes por `client_key`, re-ask casi gratuito).

### `OP_PUBLICIP_REQ` / `OP_PUBLICIP_ANSWER` (0x97/0x98, PR_EMULE)

- Entrante: `PUBLICIP_REQ` vacío → `PUBLICIP_ANSWER [u32]` con los bytes
  de red brutos de la IP del solicitante (`PokeUInt32(client->GetIP())`).
- Saliente: **voluntariamente ausente** — eMule 0.72a ya no emite la
  petición (la llamada `SendPublicIPRequest` en `ConnectionEstablished`
  lleva tiempo comentada; nuestra IP pública proviene del servidor
  `IdChange`, de UPnP y de Kad como en eMule).
- Colisiones asumidas: 0x97/0x98 son también `GLOBSERVSTATRES`/
  `GLOBSEARCHREQ` en el canal UDP de servidor — desambiguación por canal.

### Pruebas

- Unitaria `rotation_notify_fires_on_timeover_eviction`: handle ausente
  para un cliente no activo, señalado al desalojar, consumido después.
- Loopback `test_inbound_publicip_req_echoes_requester_ip`: HELLO +
  `PUBLICIP_REQ` → `PUBLICIP_ANSWER` `[7F 00 00 01]`.

## 23. `OP_PACKEDPROT` par (0xD4) + Kad UDP empaquetado (0xE5) — auditoría fase B, pasada 17

Referencia: `Packets.cpp:200-244` (`PackPacket`/`UnPackPacket`),
`EMSocket.cpp` + `ListenSocket.cpp:1742-1750` (unpack socket-level antes del
despacho), `KademliaUDPListener.cpp:1875/1891/1904` (pack `>200` en salida
UDP), `ClientUDPSocket.cpp:93-120` (unpack 0xE5 entrante, tope 250000),
`KnownFile.cpp:1149` / `PartFile.cpp:3708` (`ANSWERSOURCES >354` empaquetado),
`SharedFileList.cpp:858` (`OFFERFILES` servidor, puerta `SRV_TCPFLG_COMPRESSION`).

### Canal par TCP — unpack entrante (brecha corregida)

- `PacketCodec` dejaba pasar las tramas `0xD4` como proto desconocido y
  los bucles de lectura las ignoraban → cualquier respuesta comprimida de un
  par eMule (típicamente las grandes `OP_ANSWERSOURCES`) se descartaba.
- `mule_protocol::ed2k::packed::unpack_owned` = normalización socket-level
  fiel: proto `0xD4` → inflate zlib acotado a **50000** bytes
  (`uMaxDecompressedSize` por defecto de `UnPackPacket`), re-etiquetado
  `PR_EMULE`, opcode conservado. Payload corrupto/truncado/oversize → paquete
  descartado (sesión conservada — eMule cierra el socket; divergencia
  defensiva asumida).
- Aplicado a **todos** los bucles de lectura de par: despachador inbound,
  hello entrante/saliente, connect-back, enlace buddy, test de cortafuegos Kad,
  `status_query` (×2), `transfer`. Las antiguas condiciones
  `protocol == 0xD4` en las ramas de despacho (que aceptaban el proto
  sin inflar el payload — nunca alcanzadas) se han retirado.
- Payload zlib válido que se infla a 0 bytes: aceptado (eMule despacha un
  paquet vacío) — sin rechazo `is_empty`.

### Canal par TCP — pack saliente `OP_ANSWERSOURCES{,2}` (brecha corregida)

- `maybe_pack` aplica la puerta `size > 354` (opcode incluido) +
  conservación únicamente si es estrictamente menor, idéntico a
  `PackPacket`. La respuesta SX sale en `[0xD4][len][opcode][zlib]`.
- `OFFERFILES` hacia servidor: ya cubierto en otra parte (codec de servidor
  existente, puerta flag `SRV_TCPFLG_COMPRESSION`).

### Canal Kad UDP (brecha corregida)

- **Entrante**: `0xE5` ya aceptado por `is_valid_kad_protocol`, pero
  el inflate del decodificador estaba **sin acotar** (bomba de descompresión UDP)
  — reemplazado por `unpack_payload` tope 250000, fallo → datagrama descartado
  (eMule hace lo mismo).
- **Saliente**: `pack_kad_datagram` = `KademliaUDPListener::SendPacket` —
  todo datagrama `[0xE4][opcode][payload]` cuyo payload supera 200
  bytes se empaqueta en `[0xE5][opcode][zlib]` si la compresión ahorra;
  aplicado a los ~27 sitios `send_to` (manejadores, enrutamiento, búsqueda, callback).
  El relevo buddy TCP (`OP_CALLBACK`) no es un datagrama Kad —
  no empaquetado, fiel.

### Pruebas

- Unitarias `packed.rs`: round-trip, rechazo incompresible, tope de
  descompresión, zlib corrupto/truncado, re-etiquetado `0xD4→PR_EMULE`,
  `maybe_pack` (puertas tamaño + ganancia), `pack_kad_datagram` (re-etiquetado 0xE5,
  datagrama pequeño sin cambios, no-Kad ignorado).
- Loopback `test_inbound_packed_packet_is_unpacked_and_dispatched`:
  `PUBLICIP_REQ` empaquetado → `PUBLICIP_ANSWER`; junk packed descartado sin
  romper la sesión.

## 24. `OP_PORTTEST` TCP + relevo buddy `OP_REASKCALLBACKTCP`/`OP_CALLBACK` — auditoría fase B, pasada 18

Referencia: `ListenSocket.cpp:623` (`CHANGE_SLOT`), `:1285-1325`
(`PEERCACHE_*`, `PUBLICIP_ANSWER`, `PORTTEST`), `:1326-1360`
(`OP_CALLBACK`), `:1391-1479` (`OP_REASKCALLBACKTCP`),
`ClientUDPSocket.cpp:199-225` (`REASKCALLBACKUDP` → relevo),
`KademliaUDPListener.cpp:1757-1790` (`CALLBACK_REQ` → `OP_CALLBACK`).

### `OP_PORTTEST` (0xFE, PR_ED2K) — brecha corregida

- Entrante TCP: respuesta inmediata `0xFE + payload 0x12` («TCP
  reachable»). eMule marca después el socket `m_bPortTestCon` y un
  resultado UDP externo puede retransmitirse mediante `SendPortTestReply` — este
  relevo UDP→TCP **no** está implementado: herramienta de prueba externa, fuera de
  alcance par-a-par (divergencia asumida, el ack basta para los
  probadores habituales).
- UDP `0xFE`: sin cambios (solo registro — es la misma herramienta).

### No-op fieles verificados

- `OP_CHANGE_SLOT`: «sometimes sent by Hybrid», rama eMule vacía —
  ignorar es fiel.
- `OP_PEERCACHE_{QUERY,ANSWER,ACK}`: *DEFUNCT* en 0.72a
  (contadores de estadísticas únicamente) — ignorar es fiel.
- `OP_MESSAGE` y opcodes chat IRC: no implementados por diseño.

### `OP_REASKCALLBACKTCP` (0x9A, PR_EMULE) — brecha corregida (uploader)

- Recibido **únicamente** del enlace TCP del buddy activo (`GetBuddy() != client`
  → ignorado; comparación por `ActiveBuddy.remote_addr.ip()`).
- Payload `[destip u32][destport u16][reqhash 16][extinfo…]` → la respuesta
  sale en **UDP al solicitante original** (`destip:destport`), no al
  buddy: `OP_REASKACK` (rango, sin part-status — versión UDP del par
  desconocida vía el relevo), `OP_FILENOTFOUND` si hash no compartido,
  `OP_QUEUEFULL` si cola casi llena (`+50 > max`), silencio ante
  `FileMismatch`/`Ambiguous` (eMule fuerza entonces una conexión TCP).
- El sentido inverso (nosotros = buddy retransmite `REASKCALLBACKUDP` UDP → TCP)
  ya estaba implementado.

### `OP_CALLBACK` (0x99, PR_EMULE) — brecha corregida (LowID receptor)

- Payload `[check u128][fileid u128][ip u32][tcp u16]`; `check` en el
  cable = `~kad_id` del destinatario (el buddyID publicado en eMule ya es
  la inversión — `check XOR 0xFF…FF == GetKadID()`). Sin verificación
  «es nuestro buddy» — la inversión sirve de autenticación, como
  eMule.
- Puertas fieles: Kad en ejecución requerido; `fileid` conocido (compartido o en
  descarga) si no `CheckFailedFileIdReqs`-like ignora.
- Acción: el solicitante `ip:tcp` se inserta como fuente `origin=kad`
  del archivo — el coordinador marca la conexión (`TryToConnect`
  eMule).

### Pruebas

- Loopback `test_inbound_reaskcallbacktcp_from_buddy_relays_udp_reply`:
  sin buddy → silencio; buddy activo + socket UDP enlazado → datagrama
  `FILENOTFOUND` (`[0xC5][0x92]`) recibido en la dirección del solicitante.
- Loopback `test_inbound_callback_registers_requester_as_source`:
  check erróneo ignorado, check correcto → fuente `127.0.0.1:49777` inscrita.

## 25. Hashsets `FileIdentifier` — `OP_HASHSETREQUEST2`/`ANSWER2`

Referencias: `UploadClient.cpp:510-552` (`SendHashsetPacket`),
`DownloadClient.cpp:2200-2244` (`SendHashSetRequest`),
`DownloadClient.cpp:708` (`ProcessHashSet`), `FileIdentifier.cpp:76`
(`CompareRelaxed`), `FileIdentifier.cpp:239-283` (`WriteMD4HashsetToFile`,
`WriteHashSetsToPacket`, `WriteAICHHashsetToFile`).

### `OP_HASHSETREQUEST2` (0xB1, PR_EMULE) — brecha corregida

- Anunciar `SupportsFileIdentifiers` (MISCOPTIONS2 bit 13) en nuestros hellos
  hacía alcanzable este opcode para los pares modernos: eMule lo prefiere
  sistemáticamente a `OP_HASHSETREQUEST` (`if (SupportsFileIdentifiers())`).
- Payload: `<FileIdentifier><opts u8>` — bit0 pide el hashset MD4, bit1
  el hashset AICH por parte. Ninguna opción conocida → rechazo (eMule:
  advertencia + sin respuesta).
- Búsqueda `GetFileByIdentifier(.., bStrict=false)` = `CompareRelaxed`:
  MD4 igual, tamaño igual cuando ambos bandos lo conocen, raíz AICH
  igual ídem. Discordancia → silencio (eMule lanza `FNF` mediante
  `CheckFailedFileIdReqs`; nuestra rama v1 permanece silenciosa — conservado).

### `OP_HASHSETANSWER2` (0xB2, PR_EMULE)

- Formato: `<FileIdentifier><opts u8>[<md4 set>][<aich set>]`; el set MD4
  = `[md4 16][count u16][16B]*n` (`WriteMD4HashsetToFile`), el set AICH =
  `[root 20][count u16][20B]*n` con un hash por parte eD2K de 9 728 000
  bytes (`GetTheoreticalAICHPartHashCount` = 0 para `size <= PARTSIZE`).
- Servicio: MD4 mediante la caché DB o cálculo `hash_reader` + persistencia
  (ruta compartida con la rama v1); AICH mediante `shared_aich_tree` +
  `aich_part_hashes` (nuevo extractor mule-format de las raíces de
  subárboles de parte). Bit omitido cuando el set no está disponible — idéntico
  a eMule (`byOptions` solo refleja lo que realmente se escribe).
- Lado descargador: emitimos `OP_HASHSETREQUEST2` cuando el par anuncia
  `supports_file_identifiers` (campo `PeerSxCaps.file_identifiers` procedente del
  HELLO); el identificador lleva MD4 + tamaño + raíz AICH de confianza.
  `want_aich` sigue siendo falso: el set AICH por parte no se persiste — la
  recuperación pasa por `OP_AICHREQUEST` (ya implementado).
- Recepción `0xB2`: `handle_hashset_answer` decodifica y luego aplica el set
  MD4 por la ruta v1; un set AICH no solicitado se registra y se ignora.

### Pruebas

- Loopback `test_inbound_hashsetrequest2_serves_md4_set`: REQ2 con
  FileIdentifier {md4+size} → ANSWER2 decodificada, set MD4 servido desde la
  caché, bit AICH omitido.
- Unitaria `part_hashes_return_part_subtree_roots` (mule-format): los
  hashsets por parte son las raíces de los subárboles que cubren cada parte
  ; archivo <= 1 parte → set vacío (eMule `GetTheoreticalAICHPartHashCount`).

## 26. Rango de cola — `OP_QUEUERANK`/`OP_QUEUERANKING`

Referencias: `ListenSocket.cpp:482` (`OP_QUEUERANK` → `ProcessEdonkeyQueueRank`),
`ListenSocket.cpp:1157` (`OP_QUEUERANKING` → `ProcessEmuleQueueRank`),
`DownloadClient.cpp:1978-2000`, `UploadClient.cpp:558` (`SendRankingInfo`).

### Formatos en el cable (corrección de fidelidad)

- `OP_QUEUERANK` (0x5C, PR_ED2K): `<rank u32>` — legacy eDonkey, nunca
  emitido por eMule 0.72a, tratado inbound como `QUEUERANKING`.
- `OP_QUEUERANKING` (0x60, PR_EMULE): **exactamente 12 bytes**
  `<rank u16><10 ceros>`. Nuestra emisión era `PR_ED2K` + `u32` (5 bytes)
  — un par eMule leía `PeekUInt16` correctamente pero el proto/encuadre
  divergía; corregido a `PR_EMULE` + 12 bytes en ambos sitios de
  `SendRankingInfo`.
- Recepción: `handle_queue_rank_packet` unifica ambos opcodes mediante
  los decodificadores acotados (rechazo `QUEUERANKING != 12 bytes` como eMule),
  actualiza `queue_rank` de la sesión y emite el evento de par. Conectado
  en `status_query` (cola de espera) y `transfer` (durante la
  descarga — eMule acepta el rango en cualquier estado).
- `CheckQueueRankFlood` (anti-flood de rangos no solicitados): no portado —
  nuestros bucles solo leen QUEUERANK durante o después de una cola real.

## §27 — Persistencia de la cola de subida a través de las desconexiones

Comportamiento eMule verificado (`BaseClient.cpp:1169`, `UploadQueue.cpp`): un
cliente en `US_ONUPLOADQUEUE` **no se destruye** cuando su socket muere —
`Disconnected` devuelve `bDelete = false` para este estado. La entrada
`waitinglist` sobrevive hasta `MAX_PURGEQUEUETIME` (1 h) con su
`WaitStartTime` intacto: la puntuación sigue creciendo durante la ausencia.
En `FindBestClientInQueue`, un solicitante en espera sin socket no puede ser
promovido (`ACCEPTUPLOADREQ` se escribe en la conexión), pero si supera
al mejor cliente conectado eMule activa `m_bAddNextConnect` — al
reconectar (`AddClientToQueue`, `UploadQueue.cpp:537`), el cliente sale
de la cola y es promovido **inmediatamente** (`AddUpNextClient`), sin
esperar un slot libre.

Nuestra implementación anterior eliminaba la entrada de cola al morir el
socket (`release_slot` en la limpieza de conexión) — la puntuación de espera
se perdía en cada microdesconexión. Corregido:

- `QueuedUpload.connected` / `add_next_connect` reflejan ambos estados
  eMule; `mark_disconnected` libera el posible slot activo pero conserva
  la entrada (`connected = false`), mientras que `release_slot` queda reservado
  para fines explícitos (`OP_CANCELTRANSFER`, `OP_END_OF_DOWNLOAD`, RPC).
- `promote_locked` solo promueve las entradas conectadas y activa
  `add_next_connect` en el mejor inaccesible que supera al mejor
  conectado — misma evaluación sin disponibilidad de slot que `FindBestClientInQueue`.
- El dedup de `request_upload_slot` vuelve a poner `connected = true` y, si
  `add_next_connect` estaba activado, concede un slot inmediato incluso con cola llena
  (como `AddUpNextClient` incondicional).

Divergencia asumida documentada: eMule `CheckForTimeOver` tiene una rama
tiempo/puntuación cuando `FullChunkTransfers` está desactivado (pref no por defecto,
`Preferences.cpp:2164` defecto `true`); implementamos la rama por
defecto `SESSIONMAXTRANS` (bytes). La rotación por puntuación más allá de
`SESSIONMAXTIME` (1 h) no está portada.

Referencias: `BaseClient.cpp:1167-1174` (`bDelete`), `UploadQueue.cpp:105-152`
(`FindBestClientInQueue`), `UploadQueue.cpp:534-549` (reconexión +
`AddUpNextClient`), `UploadQueue.cpp:769-815` (`CheckForTimeOver`).

## 28. OP_COMPRESSEDPART (0x40) — recepción de bloques comprimidos de 32 bits

Constatación en vivo (binario eMule 0.72a fijado, SHA-256
`b7dd6276…4efd6`): en cuanto nuestro hello anuncia el indicador de compresión, eMule
responde a `OP_REQUESTPARTS` con `OP_COMPRESSEDPART` (`0x40`) bajo
`PR_EMULE` (`0xC5`) para los desplazamientos < 4 GiB — **nunca** `OP_SENDINGPART`
ni `PR_EMULE_COMPRESSED`. El flujo zlib de un bloque se trocea en paquetes
≤ 10 240 bytes, cada uno repitiendo el **tamaño total** del flujo en
`compressed_size` (`UploadDiskIOThread.cpp:449-456`).

Trama: `<file_hash 16><start u32 LE><compressed_size u32 LE><zlib…>`
(cabecera 24 bytes; 28 bytes con `start u64` para `0xA1`).

Correcciones aportadas:

- `Ed2kCompressedPart` (u32) + `decode_compressed_part` (rechazo cabecera
  < 24 B, rechazo `compressed_size < data.len()` — el tamaño declarado es
  el total del flujo, nunca inferior al fragmento transportado).
- `encode_compressed_part` bajo `PR_EMULE` — el codificador `I64` emitía
  `PR_EMULE_COMPRESSED` (0xD4) por error; eMule envía `OP_EMULEPROT`.
- `Ed2kLoopbackClient`: acumulador zlib por `(file_hash, block_start)`
  (`flate2::Decompress`, `FlushDecompress::Sync`), producción acotada a
  `ED2K_BLOCK_SIZE`, correlación de subrangos mediante
  `block_tracker.validate_and_remove` — misma mecánica que el despachador
  daemon (que ya gestionaba estos opcodes en la entrada).
- `receive_sending_part`: vuelve a iterar tras cada trama decodificada antes del
  `read()` bloqueante — si no las tramas ya en búfer y las partes
  descomprimidas quedaban en espera mientras el par aguardaba la
  petición siguiente (interbloqueo de la transferencia comprimida multipaquete).

Validación: 7/7 pruebas en vivo `--ignored` en verde contra el binario real
0.72a, incluidas las 3 transferencias de bloques que expiraban antes de este parche
(single block, part0+resume, file boundary).

## 29. Antiabuso: baneos IP 2 h, flood de peticiones, userhash rastreado

Alineación sobre la superficie de baneo eMule 0.72a (`ClientList::Ban` /
`AddBannedClient` — `CLIENTBANTIME` = 2 h, `Opcodes.h:123`):

- **Expiración**: nuestros baneos IP expiran tras 2 h en lugar de ser permanentes
  — `is_source_banned` comprueba la marca temporal (purga perezosa). Fiel a
  `ClientList.cpp:350` (`curTick < dwBantime + CLIENTBANTIME`).
- **Ingreso**: el TCP entrante verifica ahora `IsBannedClient` al
  aceptar (`connection.rs`, eMule `ListenSocket.cpp:1987`/`2102`); el UDP de
  par descarta puerto 0 + ipfilter al despachar y banea en
  `handle_datagram` (`ClientUDPSocket.cpp:76`). El filtrado IP TCP ya se
  aplicaba antes de leer la primera trama.
- **`AddRequestCount`** (`UploadClient.cpp:596`, llamado en
  `AddClientToQueue`): ya existía — re-ask del mismo archivo < 10 min
  (`MIN_REQUESTTIME`) incrementa `bad_requests`, `>= BADCLIENTBAN` (4) →
  `banned`. Completado: la entrada baneada empuja ahora la IP a
  `pending_bans`, vaciado por el despachador hacia `DownloadService::ban_source`
  **y** la conexión se corta (`break` → `mark_disconnected`), como
  `Ban()` → `Disconnected`. Exención amigo (`!GetFriendSlot()`) añadida.
- **`ComparePriorUserhash`** (`ClientList.cpp:386`, llamado
  `BaseClient.cpp:614`): seguimiento `(IP, puerto TCP anunciado) → userhash` 2 h
  (`KEEPTRACK_TIME`), rellenado al presentar la clave pública SUI
  (`ProcessPublicKeyPacket` = `AddTrackClient`, `BaseClient.cpp:1899`).
  Una reconexión desde el mismo (IP, puerto) bajo otro userhash → baneo +
  descarte al recibir el hello (`connection.rs`).

No portado: `IsLeecher`/heurísticas mod-string ya no existen en 0.72a;
el baneo de flood Kad (`PacketTracking.cpp:192`, deuda de tokens < −3 min)
no tiene equivalente — nuestro `in_flight` tracker ya acota por petición.

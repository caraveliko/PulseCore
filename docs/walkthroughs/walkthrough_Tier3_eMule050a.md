# Walkthrough - Interoperabilidad Tier 3 con eMule 0.50a Oficial

Este documento certifica y registra la resolución completa y el endurecimiento del handshake eD2K real (Tier 3) entre nuestra implementación Rust **eMuleRust** y el ejecutable oficial de referencia **eMule 0.50a** (`D:\eMule0.50a\emule.exe`, SHA-256: `06cdf814387f627a4bd05a0c68211f715bfa952423e8e8a462e1f47c11a4d20e`), estrictamente confinado en el loopback local `127.0.0.1`.

---

## 1. Diagnósticos y Causas Raíz Resueltas

### Causa 1: El byte de longitud de hash `0x10` faltante en `OP_HELLO`
- **Observación**: eMule cerraba de inmediato el socket TCP tras recibir el primer paquete.
- **Explicación**: En la especificación oficial de eMule/eDonkey, la carga útil del paquete `OP_HELLO` (`0x01`) comienza obligatoriamente por un byte `0x10` (tamaño 16 en decimal) que indica la longitud del User Hash que sigue inmediatamente. Por el contrario, `OP_HELLOANSWER` (`0x4C`) no posee este prefijo.
- **Resolución**: Adición del prefijo `0x10` en `Ed2kEncoder::encode_hello` para los perfiles eMule, llevando la trama mínima a exactamente 39 bytes.

### Causa 2: Modo directorio portátil (`UsePublicUserDirectories = 2`)
- **Observación**: En Windows moderno (Vista/7/10/11), eMule ignoraba ocasionalmente el `preferences.ini` local generado en el sandbox `%TEMP%\.tmp...` y conmutaba a `%LOCALAPPDATA%\eMule\config`, enlazando un puerto aleatorio en lugar del puerto efímero de la prueba.
- **Explicación**: El código fuente oficial de eMule (`Preferences.cpp`) lee el valor de registro `HKCU\Software\eMule\UsePublicUserDirectories`:
  - `0`: Modo multiusuario (`%LOCALAPPDATA%`).
  - `1`: Modo usuario público.
  - `2`: Modo directorio del ejecutable (modo portátil estricto).
- **Resolución**: El arnés aplica automáticamente `reg add "HKCU\Software\eMule" /v UsePublicUserDirectories /t REG_DWORD /d 2 /f` antes del lanzamiento, garantizando un confinamiento 100 % hermético en el sandbox temporal.

### Causa 3: Conflicto de instancia única en Windows (Single-Instance Lock)
- **Observación**: Si existía un proceso eMule huérfano en segundo plano, cualquier nueva instancia terminaba de inmediato con el código de salida 0.
- **Resolución**: Limpieza proactiva de los procesos huérfanos antes del lanzamiento y en la guarda RAII `Tier3ProcessGuard`.

### Causa 4: El anuncio concurrente `PR_EMULE` (`0xC5 0x01`) y Etiquetas extendidas
- **Observación**: eMule 0.50a emite simultáneamente `OP_EMULEINFO` (`0xC5 0x01`) y `OP_HELLOANSWER` (`0xE3 0x4C`).
- **Resolución**: Tolerancia y conservación de los paquetes `PR_EMULE` en el estado `AwaitingHelloAnswer`, decodificación de las etiquetas canónicas de eMule (`CTAG_EMULE_UDPPORTS = 0xF9`, `CTAG_EMULE_MISCOPTIONS1 = 0xFA`, `CTAG_EMULE_UDPOPTIONS = 0xFB`, `CTAG_EMULE_MISCOPTIONS2 = 0xFE`) y rellenado completo de `Ed2kPeerInfo`.

---

## 2. Estructura Wire Validada de `OP_HELLO` (39 bytes)

| Desplazamiento | Tamaño | Campo | Valor en línea |
| :--- | :--- | :--- | :--- |
| **0** | 1 | Protocolo eD2K | `0xE3` (`PR_ED2K`) |
| **1..4** | 4 | Longitud de la carga útil (LE) | `0x22 0x00 0x00 0x00` (34 bytes) |
| **5** | 1 | Opcode | `0x01` (`OP_HELLO`) |
| **6** | 1 | Prefijo tamaño User Hash | `0x10` (16 bytes) |
| **7..22** | 16 | User Hash conforme a eMule | `[.. 0x0E .. 0x6F ..]` (16 bytes) |
| **23..26** | 4 | Client ID | `0x00 0x00 0x00 0x00` (Low ID / no asignado) |
| **27..28** | 2 | Puerto TCP local (LE) | Puerto efímero asignado |
| **29..32** | 4 | Tag count | `0x00 0x00 0x00 0x00` (0 etiquetas) |
| **33..36** | 4 | Server IP | `0x00 0x00 0x00 0x00` (ningún servidor) |
| **37..38** | 2 | Server Port | `0x00 0x00` (ningún puerto) |

**Total exacto**: $1 + 4 + 34 = 39\text{ bytes}$.

---

## 3. Trazas Reales de Ejecución Validada frente a eMule 0.50a

Comando ejecutado:
```powershell
powershell -ExecutionPolicy Bypass -File scratch\run_tier3.ps1
```

```text
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.29s
     Running tests\tier3_loopback.rs (target\debug\deps\tier3_loopback-7797ba361086664e.exe)

running 1 test
test test_tier3_live_external_reference_loopback_handshake ... Target loopback addr: 127.0.0.1:57633
Reference binary work path: "%TEMP%\\.tmp<sandbox_handshake>"
Child PID: 18788
Attempt 1: child running = true
Attempt 1 failed: Timeout(500ms)
Attempt 2: child running = true
Attempt 2 failed: Timeout(500ms)
Attempt 3: child running = true
TCP connections for PID 18788:

LocalAddress LocalPort  State
------------ ---------  -----
0.0.0.0          57633 Listen

Connected on attempt 3!
Tier 3 live reference handshake successful! Peer info: Ed2kPeerInfo {
    user_hash: [REDACTED_PEER_HASH_16_BYTES],
    client_id: 0,
    tcp_port: 57633,
    name: Some("RefPeer050a"),
    version: Some(60),
    emule_version: None,
    misc_options1: Some(873677342),
    misc_options2: Some(11449),
    emule_udp_port: Some(57634),
    server_ip: 0,
    server_port: 0,
    tags: [
        Ed2kTag { name: Id(1), value: String("RefPeer050a") },
        Ed2kTag { name: Id(17), value: UInt32(60) },
        Ed2kTag { name: Id(249), value: UInt32(57634) },
        Ed2kTag { name: Id(250), value: UInt32(873677342) },
        Ed2kTag { name: Id(254), value: UInt32(11449) },
        Ed2kTag { name: Id(251), value: UInt32(51200) }
    ],
    unknown_tags: []
}
Files in work_path:
  "%TEMP%\\.tmp<sandbox_handshake>\\config"
  "%TEMP%\\.tmp<sandbox_handshake>\\emule.exe"
  "%TEMP%\\.tmp<sandbox_handshake>\\incoming"
  "%TEMP%\\.tmp<sandbox_handshake>\\lang"
  "%TEMP%\\.tmp<sandbox_handshake>\\preferences.ini"
  "%TEMP%\\.tmp<sandbox_handshake>\\skins"
  "%TEMP%\\.tmp<sandbox_handshake>\\temp"
Files in config_dir:
  "%TEMP%\\.tmp<sandbox_handshake>\\config\\cryptkey.dat"
  "%TEMP%\\.tmp<sandbox_handshake>\\config\\downloads.txt"
  "%TEMP%\\.tmp<sandbox_handshake>\\config\\preferences.ini"
  "%TEMP%\\.tmp<sandbox_handshake>\\config\\server.met"
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 5.75s
```

---

## 4. Pruebas Unitarias de Congelación de Prueba

Cinco pruebas unitarias garantizan ahora la no regresión de esta compatibilidad a nivel de `cargo test --workspace` sin necesitar un binario externo:

1. **`test_tier3_op_hello_wire_framing_emule050a_compliance`**:
   Valida al bit la serialización y deserialización del paquete `OP_HELLO` de 39 bytes bajo el perfil `Emule050a`.
2. **`test_tier3_op_hello_answer_wire_framing_emule050a_compliance`**:
   Valida la ausencia del prefijo `0x10` en `OP_HELLOANSWER` y la decodificación sin pérdida de las etiquetas extendidas eMule (`CTAG_EMULE_UDPPORTS`, `CTAG_EMULE_MISCOPTIONS1`).
3. **`test_session_awaiting_hello_answer_accepts_pr_emule_extension_before_hello_answer`**:
   Valida que la máquina de estados de sesión decodifique `Ed2kMessage::EmuleInfo` antes de `OP_HELLOANSWER`, asocie las capacidades al par y pase inmediatamente a `Ready`.
4. **`test_tier3_op_emule_info_roundtrip_nominal`**:
   Valida el parseo y la codificación hermética de `OP_EMULEINFO` y sus métodos de detección de capacidades (`has_compression_capability`, `has_source_exchange_capability`).
5. **`test_tier3_op_emule_info_oversized_tag_count_fails` y `test_tier3_op_emule_info_truncated_payload_fails`**:
   Garantiza el rechazo seguro y acotado de los paquetes truncados o con contadores de etiquetas abusivos.

---

## 5. Tier 3.1: Post-Handshake Pasivo y Multipuerto

El hito **Tier 3.1** se ha ejecutado y validado con éxito en 2 iteraciones consecutivas sobre puertos y sandboxes dinámicos:

### Capacidades Reales Anunciadas por eMule 0.50a Oficial

En `OP_EMULEINFO` (`0xC5 0x01`), eMule 0.50a anuncia:
- **`client_version` = 80** (`0x50` en hexadecimal, correspondiente a eMule **0.50a** en notación BCD).
- **`protocol_version` = 1** (eMule compatible extensiones versión 1).
- **7 etiquetas extendidas decodificadas sin pérdida (0 etiquetas desconocidas)**:
  - `TAG_SPECIAL_EMULE_EXT_COMPRESSION` (`0x20` = 32): valor 1 (compresión zlib disponible).
  - `TAG_SPECIAL_EMULE_EXT_UDPPORT` (`0x22` = 34): valor 4.
  - `TAG_SPECIAL_EMULE_EXT_UDPVER` (`0x21` = 33): puerto UDP efímero de la prueba.
  - `TAG_SPECIAL_EMULE_EXT_SOURCEEXCHANGE` (`0x23` = 35): versión 3 (soporte SX).
  - `TAG_SPECIAL_EMULE_EXT_COMMENTS` (`0x24` = 36): versión 1.
  - `TAG_SPECIAL_EMULE_EXT_EXTENDEDREQUEST` (`0x25` = 37): versión 2.
  - `TAG_SPECIAL_EMULE_EXT_COMPATIBLECLIENT` (`0x27` = 39): versión 3.

### Invariantes Verificados en Ejecución Real

1. **Disciplina Pasiva Pura**: Las capacidades de compresión e intercambio de fuentes se observan mediante `peer_info.has_compression_capability()` y `peer_info.has_source_exchange_capability()`, pero el cliente permanece en modo baseline y no activa ninguna negociación espontánea.
2. **Watchdog Pasivo Acotado (1s)**: Mantenimiento de la conexión TCP en escucha pasiva durante 1 segundo sin tráfico inesperado ni desconexión del ejecutable eMule.
3. **Confinamiento de Red 100 % Loopback**: Verificación mediante `Get-NetTCPConnection` de que el proceso objetivo no establece ninguna conexión de red no loopback.
4. **Cierre TCP Elegante (FIN)**: Llamada a `client.close().await` enviando el paquete TCP `FIN` y cerrando la sesión limpiamente.
5. **Teardown Hermético y Multi-Sandbox**: Ejecución en 2 iteraciones independientes con regeneración de los puertos efímeros (p. ej. 60963/60964 y luego 60971/60972) y purga de los procesos huérfanos.

```text
running 2 tests
test test_tier3_live_external_reference_loopback_handshake ... ok
test test_tier3_live_passive_post_handshake_multi_port_and_teardown ... 
=== Starting Tier 3.1 Iteration 1/2 ===
Iteration 1: Assigned TCP port 60963, UDP port 60964
Iteration 1: Work path: "%TEMP%\\.tmp<sandbox_iter1>"
Iteration 1: Decoded OP_EMULEINFO with 7 tags, compression=true, source_exchange=true
Iteration 1: Holding passive connection for 1s watchdog...
=== Iteration 1/2 Completed Successfully ===

=== Starting Tier 3.1 Iteration 2/2 ===
Iteration 2: Assigned TCP port 60971, UDP port 60972
Iteration 2: Work path: "%TEMP%\\.tmp<sandbox_iter2>"
Iteration 2: Decoded OP_EMULEINFO with 7 tags, compression=true, source_exchange=true
Iteration 2: Holding passive connection for 1s watchdog...
=== Iteration 2/2 Completed Successfully ===
ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 21.59s
```

---

## 6. Tier 3.2: Petición de Disponibilidad y Estado de Fichero (OP_SETREQFILEID_EMULE y OP_FILESTATUS)

El hito **Tier 3.2** da un paso esencial: tras el handshake eD2K, el arnés consulta al ejecutable oficial **eMule 0.50a** sobre la disponibilidad de las partes de un fichero compartido, y después sobre un fichero desconocido, permaneciendo estrictamente confinado al loopback `127.0.0.1` bajo disciplina pasiva absoluta.

### Fixture Determinista y Huella Sellada

Se generó un fixture de prueba directamente en el directorio compartido hermético `%TEMP%\.tmp...\incoming\` (sin ningún contacto con los directorios del anfitrión o de los usuarios):
- **Fichero**: `tier3_status_fixture_9728001.bin`
- **Tamaño exacto**: $9\,728\,001\text{ bytes}$ ($1\text{ parte eD2K completa de } 9\,280\,000\text{ bytes} + 448\,001\text{ bytes de resto} = 2\text{ partes}$).
- **Patrón determinista**: bytes consecutivos cíclicos `(0..=255)`.
- **Huella eD2K canónica sellada**: `695b81b1727666818988524bd7fc823c`.

### Descubrimiento Protocolario Clave: Semántica de `part_count == 0` en eMule Oficial

La ejecución inicial reveló un comportamiento protocolario fundamental documentado en el código fuente oficial de eMule/aMule (`ClientTCPSocket.cpp` y `DownloadClient.cpp`):

1. **Emisión de `OP_FILESTATUS` (`0x50`) por eMule**:
   ```cpp
   case OP_SETREQFILEID: {
       data_out.WriteUInt8(OP_FILESTATUS);
       if (reqfile->IsPartFile()) {
           static_cast<CPartFile*>(reqfile)->WritePartStatus(&data_out);
       } else {
           data_out.WriteUInt16(0);  // <-- CKnownFile (fichero terminado/compartido) escribe uint16(0) !
       }
       break;
   }
   ```
2. **Recepción de `OP_FILESTATUS` (`0x50`) por el cliente eMule**:
   ```cpp
   uint16 nED2KPartCount = data->ReadUInt16();
   if (!nED2KPartCount) {
       m_nPartCount = m_reqfile->GetPartCount();
       m_downPartStatus.setsize(m_nPartCount, 1); // Marca TODAS las partes como disponibles !
       m_bCompleteSource = true;                 // Par identificado como fuente completa
   }
   ```

**Regla protocolaria formal**:
- Para un fichero completo compartido (`CKnownFile`), eMule **no envía ninguna máscara binaria** por el cable de red. La trama mide exactamente **18 bytes** (16 bytes de hash MD4 + 2 bytes nulos `0x00 0x00` para `part_count = 0`).
- En la semántica eD2K, `part_count == 0` no significa «ningún bloque disponible», sino al contrario: **«fuente completa que posee el 100 % de los bloques»**.
- Únicamente los ficheros en curso de descarga (`CPartFile`) transmiten un `part_count > 0` seguido de $\lceil\text{part\_count} / 8\rceil$ bytes de máscara binaria.

3. **Tratamiento de fichero inexistente (`OP_FILEREQANSNOFILE` `0x48`)**:
   Cuando el hash solicitado no existe ni en `sharedfiles` ni en `downloadqueue`, eMule responde de inmediato con el opcode `0x48` (`OP_FILEREQANSNOFILE`) y el hash de 16 bytes.

### Trazas Reales Validadas frente a eMule 0.50a Oficial

```powershell
powershell -ExecutionPolicy Bypass -File scratch\run_tier3.ps1
```

```text
running 3 tests
test test_tier3_live_external_reference_loopback_handshake ... ok
test test_tier3_live_file_status_request_and_parts_availability ... 
Tier 3.2: Created fixture at "%TEMP%\\.tmp<sandbox_fixture>\\incoming\\tier3_status_fixture_9728001.bin"
Tier 3.2: Sealed eD2K Hash: 695b81b1727666818988524bd7fc823c
Attempt 1: child running = true
Attempt 1 failed: Timeout(500ms)
Attempt 2: child running = true
Connected on attempt 2!
Tier 3.2: Handshake successful with peer: Some("RefPeer050a")
Tier 3.2: Sending OP_SETREQFILEID_EMULE (0x4F) for fixture...
Tier 3.2: Received OP_FILESTATUS:
  File Hash : 695b81b1727666818988524bd7fc823c
  Part Count: 0
  Bitfield  : []
Tier 3.2: Successfully received OP_FILEREQANSNOFILE (0x48) for unknown hash!
Tier 3.2: File status and parts availability successfully verified against eMule 0.50a!
ok
test test_tier3_live_passive_post_handshake_multi_port_and_teardown ... 
=== Starting Tier 3.1 Iteration 1/2 ===
Iteration 1: Assigned TCP port 55060, UDP port 55061
Iteration 1: Work path: "%TEMP%\\.tmp<sandbox_iter1>"
Iteration 1: Decoded OP_EMULEINFO with 7 tags, compression=true, source_exchange=true
Iteration 1: Holding passive connection for 1s watchdog...
=== Iteration 1/2 Completed Successfully ===
=== Starting Tier 3.1 Iteration 2/2 ===
Iteration 2: Assigned TCP port 55067, UDP port 55068
Iteration 2: Work path: "%TEMP%\\.tmp<sandbox_iter2>"
Iteration 2: Decoded OP_EMULEINFO with 7 tags, compression=true, source_exchange=true
Iteration 2: Holding passive connection for 1s watchdog...
=== Iteration 2/2 Completed Successfully ===
ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 30.50s
```

### Invariantes y Alcance Validados

| Dominio | Invariante Verificado |
| :--- | :--- |
| **Petición Fixture** | Emisión canónica de `OP_SETREQFILEID_EMULE` (`0x4F`) sobre 21 bytes (`0xE3 0x11 0x00 0x00 0x00 0x4F <HASH 16>`). |
| **Respuesta Fichero Conocido** | Recepción y decodificación de `OP_FILESTATUS` (`0x50`) sobre 18 bytes con `part_count = 0` y máscara vacía. |
| **Semántica Disponibilidad** | `status.is_complete() == true`, `status.has_part(0) == true`, `status.has_part(1) == true`. |
| **Respuesta Fichero Desconocido** | Emisión para `[0xAA; 16]`, recepción inmediata de `OP_FILEREQANSNOFILE` (`0x48`), conversión a `FileNotFound` y limpieza del rastreador en vuelo. |
| **Disciplina Pasiva Estricta** | 0 peticiones de bloque (`OP_REQUESTPARTS`), 0 bytes de carga útil descargados, 0 tráfico UDP/Kad, 0 zlib (`0xD4`). |
| **Confinamiento de Red** | Verificación automática `Get-NetTCPConnection` que confirma 0 sockets fuera de loopback. |
| **Aislamiento Hermético** | Todos los ficheros de prueba se crean en subcarpetas efímeras `%TEMP%\.tmp...` limpiadas por RAII. |

---

## 7. Tier 3.3: Secuenciación de Descarga sin Carga Útil (OP_REQUESTPARTS_EMULE Planning)

El hito **Tier 3.3** formaliza, valida y sella toda la lógica de planificación y secuenciación que precede a cualquier transferencia de datos:
- Selección determinista de las partes disponibles en función del estado.
- Segmentación en bloques estándar eD2K ($\le 184\,320\text{ bytes}$) y alineación estricta sobre la frontera de parte ($9\,728\,000\text{ bytes}$).
- Cobertura completa de la primera parte y del resto de un byte para el fixture de $9\,728\,001\text{ bytes}$.
- Validación estricta de los límites, desbordamientos y prohibición de bloques a caballo entre dos partes.
- Prueba de admisibilidad de todas las partes para una fuente completa (`part_count == 0`).
- Prueba de rechazo absoluto de cualquier petición si se recibió `OP_FILEREQANSNOFILE` (`0x48`).
- **Disciplina pasiva absoluta**: codificación de la trama `OP_REQUESTPARTS_EMULE` (`0x47`) únicamente en memoria local, sin transmisión hacia eMule 0.50a.

### Arquitectura del Secuenciador (`Ed2kDownloadSequencer`)

El nuevo componente `Ed2kDownloadSequencer` (`crates/mule-protocol/src/ed2k/sequencer.rs`) aplica los siguientes invariantes canónicos:
1. **Fuente completa**: Cuando `status.is_complete()` es verdadero (`part_count == 0`), todas las partes $0..\text{total\_parts}$ se declaran disponibles.
2. **Segmentación de la parte 0 ($9\,728\,000\text{ bytes}$)**:
   - Descompuesta en exactamente 53 bloques: 52 bloques completos de $184\,320\text{ bytes}$ + 1 bloque residual de $143\,360\text{ bytes}$ (`[9_584_640, 9_728_000)`).
   - El bloque final se alinea al byte exacto sobre la frontera $9\,728\,000$, sin desbordar jamás sobre la parte siguiente.
3. **Segmentación de la parte 1 ($1\text{ byte}$ de resto)**:
   - Descompuesta en 1 bloque de $1\text{ byte}$ (`[9_728_000, 9_728_001)`).
   - Cubre exactamente el último byte del fixture.
4. **Matriz de rechazo estricto**:
   - Rechazo de `start >= end`.
   - Rechazo de superación del tamaño del fichero.
   - Rechazo de bloques que superen $184\,320\text{ bytes}$.
   - Rechazo inmediato de cualquier bloque que cruce una frontera de parte (p. ej. `[9_727_000, 9_729_000)` devuelve `CrossPartBoundary`).
   - Rechazo de peticiones con más de 3 rangos (`TooManyRanges`).
5. **Rechazo de fichero desconocido**:
   - Si el estado es `FileNotFound` (`OP_FILEREQANSNOFILE`), `Ed2kDownloadSequencer::from_file_status_result` falla de inmediato y prohíbe cualquier planificación de bloque.

### Trazas Reales Validadas frente a eMule 0.50a Oficial

```powershell
powershell -ExecutionPolicy Bypass -File scratch\run_tier3.ps1
```

```text
running 4 tests
test test_tier3_live_download_sequencing_without_payload ... 
Tier 3.3: Download sequencing and block request planning without payload successfully verified against eMule 0.50a!
ok
test test_tier3_live_external_reference_loopback_handshake ... ok
test test_tier3_live_file_status_request_and_parts_availability ... ok
test test_tier3_live_passive_post_handshake_multi_port_and_teardown ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 35.98s
```

### Invariantes y Alcance Validados

| Dominio | Invariante Verificado |
| :--- | :--- |
| **Selección Determinista** | `sequencer.available_parts() == vec![0, 1]` para el fichero de 9 728 001 bytes procedente del estado completo eMule. |
| **Alineación Frontera** | Bloque 52 de Part 0 termina en $9\,728\,000$ exactos. Bloque 0 de Part 1 cubre $9\,728\,000..9\,728\,001$. |
| **Framing Wire Local** | Paquetes `OP_REQUESTPARTS_EMULE` (`0x47`) construidos sobre 40 bytes de carga útil (hash 16 + 3 start offsets + 3 end offsets), conformes a eMule 0.50a. |
| **Disciplina Pasiva Absoluta** | 0 paquetes de bloque emitidos en el socket TCP; `pending_block_count` de la sesión permanece en 0. |
| **Rechazo Fichero Desconocido** | Recepción de `OP_FILEREQANSNOFILE` (`0x48`) $\rightarrow$ rechazo estricto de cualquier planificación de bloque. |
| **Confinamiento de Red** | Verificación `Get-NetTCPConnection` que confirma 0 sockets fuera de loopback. |

---

## 8. Tier 3.4: Transferencia Real de un Primer Bloque No Comprimido Bipartito (Single Bounded Block Transfer)

El hito **Tier 3.4** realiza el primer intercambio real de carga útil (payload de datos) entre el ejecutable oficial **eMule 0.50a** y nuestro cliente **eMuleRust**, en condiciones estrictamente acotadas y herméticas:
- Fichero sintético sellado: `tier3_status_fixture_9728001.bin` ($9\,728\,001\text{ bytes}$, hash eD2K sellado: `695b81b1727666818988524bd7fc823c`).
- Petición delimitada a un único bloque estándar: Part 0 Bloque 0, es decir el rango `[0, 184_320)` ($184\,320\text{ bytes}$).
- Protocolo: `PR_ED2K` (`0xE3`), paquetes `OP_SENDINGPART` (`0x46`).

### Descubrimientos Protocolarios Oficiales eMule 0.50a

1. **Prerrequisito de la cola de subida (`UploadQueue`)**:
   En la implementación C++ de eMule (`UploadQueue.cpp` y `ClientTCPSocket.cpp`), un cliente solicitante no puede emitir directamente `OP_REQUESTPARTS` sin haber obtenido previamente una ranura de subida (`US_UPLOADING`).
   - El cliente transmite primero `OP_STARTUPLOADREQ` (`0x54`) con el hash del fichero (17 bytes).
   - eMule asigna la ranura disponible y responde con `OP_ACCEPTUPLOADREQ` (`0x55`).
   - Una vez aceptado, el cliente transmite su petición de bloque `OP_REQUESTPARTS_EMULE` (`0x47`).

2. **Fragmentación de tramas en tramos de 10 240 bytes**:
   eMule 0.50a no emite los $184\,320\text{ bytes}$ en un único paquete gigante, sino que divide la transmisión en tramos de red de $10\,240\text{ bytes}$ (`EMULE_DATA_BUFFER` = 10 KB):
   $$\frac{184\,320}{10\,240} = 18\text{ tramos consecutivos de }10\,240\text{ bytes}$$
   Cada tramo se encapsula en un paquete `OP_SENDINGPART` (`0x46`) con sus desplazamientos propios: `[0..10240)`, `[10240..20480)`, ..., `[174080..184320)`.

3. **Ensamblaje y Detección de Duplicados (`Ed2kBlockSink`)**:
   El nuevo receptor en memoria `Ed2kBlockSink` reserva un búfer predimensionado al tamaño exacto del fichero ($9\,728\,001\text{ bytes}$), valida cada tramo mediante el secuenciador, rechaza cualquier solapamiento o duplicado (`DuplicateBlock`) y certifica la cobertura completa mediante `is_range_covered(0, 184_320)`.

### Trazas Reales Validadas frente a eMule 0.50a Oficial

```powershell
powershell -ExecutionPolicy Bypass -File scratch\run_tier3.ps1
```

```text
running 5 tests
test test_tier3_live_download_sequencing_without_payload ... Tier 3.3: Download sequencing and block request planning without payload successfully verified against eMule 0.50a!
ok
test test_tier3_live_external_reference_loopback_handshake ... ok
test test_tier3_live_file_status_request_and_parts_availability ... Tier 3.2: File status and parts availability successfully verified against eMule 0.50a!
ok
test test_tier3_live_passive_post_handshake_multi_port_and_teardown ... ok
test test_tier3_live_single_uncompressed_block_transfer_from_emule050a ... 
Tier 3.4: Requesting upload slot via OP_STARTUPLOADREQ (0x54)...
Tier 3.4: OP_ACCEPTUPLOADREQ (0x55) confirmed! Upload slot granted.
Tier 3.4: Transmitting single OP_REQUESTPARTS_EMULE (0x47) for [0, 184_320)...
Tier 3.4: Awaiting OP_SENDINGPART (0x46) chunks for [0, 184_320)...
Tier 3.4: Received OP_SENDINGPART chunk #1: [0..10240), 10240 bytes (total: 10240/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #2: [10240..20480), 10240 bytes (total: 20480/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #3: [20480..30720), 10240 bytes (total: 30720/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #4: [30720..40960), 10240 bytes (total: 40960/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #5: [40960..51200), 10240 bytes (total: 51200/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #6: [51200..61440), 10240 bytes (total: 61440/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #7: [61440..71680), 10240 bytes (total: 71680/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #8: [71680..81920), 10240 bytes (total: 81920/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #9: [81920..92160), 10240 bytes (total: 92160/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #10: [92160..102400), 10240 bytes (total: 102400/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #11: [102400..112640), 10240 bytes (total: 112640/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #12: [112640..122880), 10240 bytes (total: 122880/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #13: [122880..133120), 10240 bytes (total: 133120/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #14: [133120..143360), 10240 bytes (total: 143360/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #15: [143360..153600), 10240 bytes (total: 153600/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #16: [153600..163840), 10240 bytes (total: 163840/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #17: [163840..174080), 10240 bytes (total: 174080/184,320)
Tier 3.4: Received OP_SENDINGPART chunk #18: [174080..184320), 10240 bytes (total: 184320/184,320)
Tier 3.4: Block MD4 Oracle: 1d4196d9f9fa78d00b59ead840171f15
Tier 3.4: Single bounded block transfer successfully verified against eMule 0.50a!
ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 44.19s
```

### Invariantes y Alcance Validados

| Dominio | Invariante Verificado |
| :--- | :--- |
| **Entrada en Cola de Subida** | Emisión de `OP_STARTUPLOADREQ` (`0x54`), recepción y validación de `OP_ACCEPTUPLOADREQ` (`0x55`) por eMule 0.50a. |
| **Petición Única** | Una única petición `OP_REQUESTPARTS_EMULE` (`0x47`) para el bloque `[0, 184_320)`. Invariante `pending_block_count == 1`. |
| **Recepción y Segmentación** | 18 tramos consecutivos de 10 240 bytes recibidos bajo `OP_SENDINGPART` (`0x46`), totalizando $184\,320\text{ bytes}$. |
| **Conformidad Byte a Byte** | El bloque ensamblado coincide al 100 % con el prefijo del fichero sintético de prueba (`tier3_status_fixture_9728001.bin`). |
| **Oráculo MD4 Local** | El oráculo MD4 sobre el bloque recibido produce `1d4196d9f9fa78d00b59ead840171f15`, idéntico al oráculo sobre el prefijo de referencia. |
| **Rechazo de Duplicado** | Cualquier intento de reescritura en `Ed2kBlockSink` se rechaza estrictamente con `DuplicateBlock`. |
| **Disciplina Pasiva Absoluta** | Ningún bloque adicional solicitado, `pending_block_count == 0` al finalizar la transferencia. |
| **Confinamiento de Red** | Escaneo `Get-NetTCPConnection` que certifica 0 sockets fuera de `127.0.0.1`. |
| **Cierre Limpio** | Envío de TCP FIN (`client.close().await`) y detención limpia del proceso eMule. |

---

## 9. Tier 3.5: Finalización de la Parte 0 con Reanudación Controlada e Interrupción Abrupta (Part 0 Transfer, Controlled Resume & Interruption)

El hito **Tier 3.5** valida por primera vez la transferencia completa de una parte eD2K íntegra (Part 0, es decir $9\,728\,000\text{ bytes} = 53\text{ bloques}$) desde el ejecutable oficial **eMule 0.50a** (`D:\eMule0.50a\emule.exe`), con interrupción simulada, persistencia física en un fichero `.part`, diario de estado atómico `.resume.json`, cálculo de huecos sin redundancia, reanudación multisesión y verificación de integridad MD4 transmitida desde el disco.

### Alcance e Invariantes Tier 3.5

- **Part 0 Estrictamente Aislada**: 53 bloques ($9\,728\,000\text{ bytes}$ = 52 bloques completos de $184\,320\text{ bytes}$ + 1 bloque residual de $143\,360\text{ bytes}$).
- **Part 1 Residual Excluida**: El único byte residual de la Part 1 (`[9_728_000, 9_728_001)`) nunca fue solicitado ni escrito (reservado para Tier 3.6).
- **Ventana Deslizante (Pipelining)**: Como máximo 3 peticiones de bloques activas en vuelo simultáneamente (`max_in_flight = 3`), respetando el límite de congestión eD2K oficial.
- **Almacenamiento Físico `.part` y Diario `.resume.json`**:
  - `Ed2kPartFileSink` preasigna / dimensiona el fichero físico al tamaño exacto ($9\,728\,001\text{ bytes}$).
  - Los tramos recibidos se escriben en disco en su posición absoluta con `seek(SeekFrom::Start(start))`.
  - `Ed2kPartResumeState` registra los intervalos recibidos fusionados canónicamente (`completed_ranges`) y los bytes acumulados.
  - Guardado atómico mediante escritura en un fichero temporal (`.tmp`), `sync_all()` y luego renombrado atómico hacia `.resume.json` para garantizar la inmunidad ante fallos.
- **Escenario de Interrupción Controlada (Fase 1)**:
  - Descarga exacta de los 20 primeros bloques ($3\,686\,400\text{ bytes}$).
  - Corte abrupto de la conexión TCP (`client1.close().await`, cierre de socket).
  - Validación en disco: el fichero `.part` y el diario `.resume.json` contienen fielmente los $3\,686\,400\text{ bytes}$ y el intervalo único fusionado `[(0, 3_686_400)]`.
- **Escenario de Reanudación Hermética (Fase 2)**:
  - Reapertura mediante `Ed2kPartFileSink::open_or_resume`, verificación de coherencia del hash eD2K y del tamaño.
  - Cálculo determinista de los huecos mediante `sequencer.plan_missing_blocks(0, resumed_sink.completed_ranges())`:
    - Exactamente 33 bloques faltantes identificados (bloques 20 a 52, desplazamientos `[3_686_400..9_728_000)`).
    - Cero peticiones emitidas para los bloques ya poseídos 0..19.
  - Nueva conexión hacia eMule 0.50a, reserva de ranura de subida (`OP_STARTUPLOADREQ` -> `OP_ACCEPTUPLOADREQ`).
  - Descarga de los 33 bloques restantes mediante la ventana deslizante.
  - Finalización al 100 % de la Part 0 ($9\,728\,000\text{ bytes}$).
- **Validación Criptográfica MD4 en Flujo (Streamed Disk MD4)**:
  - El método `sink.compute_part_md4(0, &sequencer)` lee la Part 0 en disco mediante búferes de 64 KiB directamente en el hasher MD4 sin saturar la RAM.
  - Resultado obtenido: `240568858a65f85133d858f538d17467`, estrictamente idéntico al oráculo calculado sobre los bytes de referencia del fixture.

### Trazas Reales Validadas frente a eMule 0.50a Oficial

```powershell
powershell -ExecutionPolicy Bypass -File scratch\run_tier3.ps1
```

```text
running 6 tests
test test_tier3_live_download_sequencing_without_payload ... ok
test test_tier3_live_external_reference_loopback_handshake ... ok
test test_tier3_live_file_status_request_and_parts_availability ... ok
test test_tier3_live_passive_post_handshake_multi_port_and_teardown ... ok
test test_tier3_live_single_uncompressed_block_transfer_from_emule050a ... ok
test test_tier3_live_part0_transfer_with_resume_and_interruption ... 
Tier 3.5: Expected Part 0 MD4 oracle: 240568858a65f85133d858f538d17467
Tier 3.5: Session 1 requesting upload slot via OP_STARTUPLOADREQ (0x54)...
Tier 3.5: Session 1 upload slot granted!
Tier 3.5: Phase 1: Downloading first 20 blocks (3,686,400 bytes) with sliding window (max 3 in-flight)...
Tier 3.5: Phase 1 verified: exactly 20 blocks (3,686,400 bytes) written and verified on disk.
Tier 3.5: Simulating abrupt network interruption (closing Session 1 TCP connection)...
Tier 3.5: Atomic .resume.json journal verified on disk after interruption.
Tier 3.5: Phase 2: Opening existing .part file and .resume.json journal...
Tier 3.5: Gap calculation verified: exactly 33 missing blocks planned [3,686,400..9,728,000).
Tier 3.5: Session 2 requesting upload slot via OP_STARTUPLOADREQ (0x54)...
Tier 3.5: Session 2 upload slot granted!
Tier 3.5: Phase 2: Downloading remaining 33 blocks with sliding window (max 3 in-flight)...
Tier 3.5: Part 0 download 100% completed across 2 sessions!
Tier 3.5: Streaming Part 0 from disk in 64 KiB buffers to verify MD4 checksum...
Tier 3.5: Part 0 MD4 verified: 240568858a65f85133d858f538d17467 == 240568858a65f85133d858f538d17467
Tier 3.5: Part 0 transfer with resume and interruption successfully verified against eMule 0.50a!
ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 62.65s
```

### Síntesis de las Garantías e Invariantes Demostrados

| Dominio | Prueba / Invariante Validado |
| :--- | :--- |
| **Persistencia en Disco** | Fichero `.part` escrito por `Ed2kPartFileSink` con posicionamiento `seek`, sin corrupción ni truncamiento. |
| **Diario Atómico** | Fichero `.resume.json` actualizado de forma transaccional (`.tmp` + `sync_all` + rename). |
| **Fusión de Intervalos** | Los tramos adyacentes y consecutivos se fusionan automáticamente en rangos canónicos `(start, end)`. |
| **Resistencia a Fallos** | Tras interrupción TCP abrupta en el bloque 20, reapertura exacta y reanudación íntegra. |
| **Cálculo de Huecos** | `plan_missing_blocks` calcula los 33 bloques restantes (bloques 20..52) y excluye el 100 % de los bloques ya poseídos (0..19). |
| **Pipelining / Ventana** | `download_blocks_pipelined` mantiene una ventana activa acotada ($\le 3$ peticiones simultáneas). |
| **Part 0 Completada** | Exactamente $9\,728\,000\text{ bytes}$ recibidos y ensamblados en disco. |
| **Exclusión Part 1** | Ningún byte de la Part 1 (`[9_728_000..9_728_001)`) fue solicitado o escrito. |
| **Oráculo MD4 en Flujo** | El hash MD4 calculado en streaming desde el disco es estrictamente idéntico al oráculo del fixture (`240568858a65f85133d858f538d17467`). |
| **Aislamiento y Pureza** | 0 panics, 0 `unwrap()`, 0 `expect()` en `src/ed2k/`, 0 sockets fuera de loopback `127.0.0.1`. |

---

## 10. Tier 3.6: Finalización de Frontera de Fichero, Cálculo Raíz eD2K Multiparte, Promoción Transaccional Resistente a Fallos y Limpieza (File Boundary Finalization, Multi-Part Root Hashing, Two-Phase Promotion & Cleanup)

El hito **Tier 3.6** concluye con éxito el ciclo completo de adquisición, control de integridad y entrega física de un fichero multiparte eD2K frente al ejecutable oficial **eMule 0.50a** (`D:\eMule0.50a\emule.exe`), estrictamente confinado en `127.0.0.1` en loopback.

### Alcance e Invariantes Validados

1. **Cruce y Respeto de la Frontera de Parte (Boundary Invariance)**:
   - Fichero objetivo: `tier3_status_fixture_9728001.bin` ($9\,728\,001\text{ bytes}$, es decir 2 partes eD2K).
   - Part 0: $9\,728\,000\text{ bytes}$ ($9\,500\text{ KiB}$, 53 bloques).
   - Part 1: $[9\,728\,000, 9\,728\,001)$ ($1\text{ byte}$ residual único `0x00`).
   - La petición `OP_REQUESTPARTS_EMULE` (`0x47`) se emite exclusivamente para `[9_728_000, 9_728_001)`: ninguna petición cruza jamás la frontera de los $9\,728\,000\text{ bytes}$.
   - Recepción de `OP_SENDINGPART` (`0x46`) que contiene exactamente 1 byte de datos.
   - Alcanzada la cobertura integral del fichero: $9\,728\,001\text{ bytes}$, intervalo fusionado `[(0, 9_728_001)]`.

2. **Cálculo Criptográfico del Hash Raíz eD2K Multiparte (Full Streamed Multi-Part Root Hashing)**:
   - Según la especificación canónica eD2K / eMule:
     - Si `total_parts == 1`, el hash de fichero es la huella MD4 de la Part 0.
     - Si `total_parts > 1`, cada parte física (incluida la parte residual final) se calcula individualmente mediante MD4 ($16\text{ bytes}$). Las huellas de partes concatenadas ($N \times 16\text{ bytes}$) se someten entonces a un MD4 global para producir el hash raíz eD2K:
       $$\text{Root MD4} = \text{MD4}\Big(\text{MD4}(\text{Part } 0) \;\parallel\; \text{MD4}(\text{Part } 1) \;\parallel\; \dots \;\parallel\; \text{MD4}(\text{Part } N-1)\Big)$$
   - Cálculo emitido en streaming directo desde el disco en bloques de 64 KiB sin carga completa en memoria:
     - Part 0 Streamed MD4: `240568858a65f85133d858f538d17467`
     - Part 1 Streamed MD4: `47c61a0fa8738ba77308a8a600f88e4b`
     - Hash Raíz Calculado: `MD4(Part 0 || Part 1) = ` **`695b81b1727666818988524bd7fc823c`**
     - Coincidencia 100 % byte a byte con el hash eD2K sellado del fixture oficial.

3. **Máquina de Estados de Promoción en Dos Fases y Resiliencia ante Fallos (`Ed2kPartDownloadStatus`)**:
   - Estados declarados:
     - `Downloading`: Adquisición activa de los tramos en el fichero `.part`.
     - `VerifiedPendingPromotion`: Descarga 100 % completada, hash raíz eD2K verificado en disco con éxito, diario actualizado y sincronizado (`sync_all`), fichero pendiente de promoción local.
     - `Promoted`: Fichero transferido a su directorio destino final (`downloads/`), diario `.resume.json` y fichero `.part` purgados con éxito.
   - **Fase 1 (Verificación y Consignación Atómica)**: `mark_verified_pending_promotion` cambia el estado en el diario transaccional `.resume.json`.
   - **Prueba de Fallo Simulado**: El proceso cliente se interrumpe bruscamente (conexión TCP cerrada, receptor descargado de la memoria).
   - **Reanudación Local Sin Red**: Al reiniciar, `Ed2kPartFileSink::open_or_resume` detecta el estado `VerifiedPendingPromotion` en disco y permite finalizar la promoción sin necesitar la menor reconexión de red ni volver a pedir un solo byte a eMule.
   - **Fase 2 (Promoción Atómica, Anticolisión y Limpieza)**:
     - `promote_to_destination` libera limpiamente el handle de Windows (`file.take()`) para prevenir cualquier bloqueo de fichero (`ERROR_SHARING_VIOLATION`).
     - Verificación anticolisión: rechaza sobrescribir un fichero existente con un error explícito.
     - Desplazamiento atómico (mediante renombrado intermedio `.part_promoted.tmp`) hacia el directorio `downloads/tier3_promoted_final.bin`.
     - Eliminación del fichero original `.part` y eliminación del diario compañero `.resume.json`.

### Trazas Reales de Ejecución frente a eMule 0.50a Oficial

```powershell
$env:EMULE_REFERENCE_BIN = 'D:\eMule0.50a\emule.exe'; $env:EMULE_REFERENCE_SHA256 = '06cdf814387f627a4bd05a0c68211f715bfa952423e8e8a462e1f47c11a4d20e'; cargo test -p mule-protocol --test tier3_loopback --features ed2k-reference-loopback -- test_tier3_live_file_boundary_finalization_and_promotion --ignored --nocapture
```

```text
running 1 test
Tier 3.6: Launching reference eMule 0.50a with shared 9,728,001 bytes fixture...
Tier 3.6: Requesting upload slot via OP_STARTUPLOADREQ (0x54)...
Tier 3.6: Upload slot granted!
Tier 3.6: Phase 1: Downloading Part 0 (53 blocks, 9,728,000 bytes)...
Tier 3.6: Phase 1 verified: Part 0 (9,728,000 bytes) downloaded and present on disk.
Tier 3.6: Phase 2: Planning and requesting Part 1 residual block [9,728,000, 9,728,001)...
Tier 3.6: Phase 2 verified: Part 1 residual block received; 100% file coverage achieved!
Tier 3.6: Phase 3: Verifying in-disk multi-part eD2K root hash computation...
Tier 3.6: Phase 3 verified: In-disk root eD2K hash 695b81b1727666818988524bd7fc823c matches sealed oracle byte-for-byte!
Tier 3.6: Phase 4: Step A - Marking VerifiedPendingPromotion in atomic journal...
Tier 3.6: Phase 4: Step B - Simulating abrupt crash / network disconnection...
Tier 3.6: Phase 4: Step C - Recovering locally from disk without contacting eMule...
Tier 3.6: Phase 4: Step D - Atomically promoting .part to 'C:\Users\Lou\AppData\Local\Temp\.tmpGe9L2G\downloads'...
Tier 3.6: Phase 4 verified: Two-phase promotion and cleanup completed successfully!
Tier 3.6: File boundary finalization, full root eD2K hash, and two-phase promotion successfully verified against eMule 0.50a!
test test_tier3_live_file_boundary_finalization_and_promotion ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 10 filtered out; finished in 17.13s
```

### Ejecución Global de la Suite Completa Tier 3 (7 Pruebas Referencia en Vivo)

```powershell
powershell -ExecutionPolicy Bypass -File scratch\run_tier3.ps1
```

```text
running 7 tests
test test_tier3_environment_initialization_and_neutralized_configs ... ok
test test_tier3_live_download_sequencing_without_payload ... ok
test test_tier3_live_external_reference_loopback_handshake ... ok
test test_tier3_live_file_boundary_finalization_and_promotion ... ok
test test_tier3_live_file_status_request_and_parts_availability ... ok
test test_tier3_live_part0_transfer_with_resume_and_interruption ... ok
test test_tier3_live_single_uncompressed_block_transfer_from_emule050a ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 80.69s
```

### Tabla Resumen de los Invariantes Validados

| Dominio | Prueba / Invariante Validado |
| :--- | :--- |
| **Frontera de Parte** | Petición estrictamente acotada para Part 1 `[9_728_000, 9_728_001)` (1 byte), sin cruce de frontera de parte. |
| **Finalización 100 %** | Ensamblaje exacto de los $9\,728\,001\text{ bytes}$ del fixture en disco en el intervalo canónico `[(0, 9_728_001)]`. |
| **Hash Raíz Multiparte** | Cálculo en streaming de $\text{MD4}(\text{Part 0}) \parallel \text{MD4}(\text{Part 1})$ dando exactamente `695b81b1727666818988524bd7fc823c`. |
| **Promoción Transaccional** | Transición segura `Downloading` $\to$ `VerifiedPendingPromotion` $\to$ `Promoted`. |
| **Resiliencia ante Fallos** | Reanudación tras interrupción abrupta directamente en `VerifiedPendingPromotion` sin llamada de red. |
| **Protección Anticolisión** | Rechazo estricto a sobrescribir un fichero de destino ya presente. |
| **Gestión Windows Handles** | Liberación del handle de fichero antes del renombrado para evitar cualquier bloqueo de E/S. |
| **Limpieza RAII** | Eliminación automática del fichero `.part` y del diario `.resume.json` una vez confirmada la promoción en disco. |
| **Confinamiento de Red** | Escaneo `Get-NetTCPConnection` certificando 0 sockets no loopback durante toda la ejecución. |
| **Calidad del Código** | 0 `unwrap()`, 0 `expect()`, 0 panics en `src/ed2k/`, verificado por `ed2k_isolation_guard`. |

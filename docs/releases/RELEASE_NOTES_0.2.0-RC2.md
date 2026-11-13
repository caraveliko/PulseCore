# Notas de la Versión — eMuleRust v0.2.0-RC2

**Fecha de publicación:** 25 de septiembre de 2026  
**Estado:** Release Candidate — auditoría en línea/de comportamiento eMule 0.72a
exhaustiva, interoperabilidad en vivo demostrada, migración eMule completa

---

## 1. Síntesis de la Versión

La versión **v0.2.0-RC2** cierra una campaña de auditoría de extremo a extremo del
tráfico en línea y del comportamiento frente a las fuentes oficiales de eMule 0.72a
(`srchybrid`): cobertura exhaustiva de los opcodes TCP/UDP de par y servidor,
antiabuso fiel (`CLIENTBANTIME`, `BADCLIENTBAN`,
`ComparePriorUserhash`), cola de subida persistente ante desconexiones y —
punto culminante — **validación en vivo frente al binario real eMule 0.72a**:
7/7 pruebas de interoperabilidad, incluidas transferencias de bloques comprimidos
reales.

1. **Interoperabilidad en vivo eMule 0.72a: 7/7** — el arnés Tier-3 lanza el
   `emule.exe` oficial (SHA-256 fijado, perfil loopback aislado) y
   valida handshake, file-status, transferencias de bloques, reanudación y
   finalización. Se encontraron y corrigieron dos fallos reales en línea
   gracias a estas pruebas:
   - **`OP_COMPRESSEDPART` (0x40)**: el cliente de pruebas se saltaba todo
     paquete `PR_EMULE` — sin embargo, eMule transporta ahí los bloques zlib. Decodificador
     acotado + reensamblaje incremental del flujo comprimido (tramos de
     ≤ 10 240 bytes en varios paquetes) + subrangos validados.
   - **Fallo de flujo**: recepción bloqueante en lugar de drenar las partes
     descomprimidas ya producidas — interbloqueo en transferencia multipaquete.
   - Corrección anexa: `COMPRESSEDPART{,_I64}` emitidos bajo `PR_EMULE`
     (`OP_EMULEPROT`), no `PR_EMULE_COMPRESSED`.

2. **Cobertura de opcodes casi total** — diff mecánico de los ~95 `case`
   de eMule: exploración de compartidos, veredicto de cortafuegos Kad TCP,
   `SERVER_DESC_REQ/RES`, `PUBLICIP_REQ`, `OUTOFPARTREQS`, `PACKEDPROT`
   de par + Kad UDP empaquetado, `PORTTEST`, callbacks LowID
   (`REASKCALLBACKTCP`/`CALLBACK`/`DIRECTCALLBACKREQ`), hashsets
   `FileIdentifier` (`HASHSETREQUEST2/ANSWER2`), `QUEUERANK` heredado +
   `QUEUERANKING` en carga útil de 12 bytes `PR_EMULE`. Único ausente:
   `PREVIEWANSWER` — nunca solicitado puesto que se anuncia preview=0.

3. **Antiabuso eMule 0.72a**: bloqueos de IP limitados a `CLIENTBANTIME` (2 h)
   en lugar de permanentes; `AddRequestCount`/`BADCLIENTBAN` → bloqueo de IP real
   + desconexión (exención de amigo); `ComparePriorUserhash` sobre `(IP, port)`
   seguido 2 h vía SUI; `ipfilter` aplicado a los datagramas UDP de par y al
   accept TCP.

4. **Cola de subida fiel a las desconexiones**: una entrada en cola sobrevive
   a la muerte del socket hasta `MAX_PURGEQUEUETIME` (puntuación intacta);
   el mejor inalcanzable dominante se marca `add_next_connect` y su
   reconexión abre una ranura inmediata — fiel a
   `Disconnected`/`FindBestClientInQueue`/`AddClientToQueue`.

5. **Migración eMule**: parsers e importaciones `known.met`, `clients.met`
   (créditos), `emfriends.met`, `cancelled.met`, identidad completa
   (cryptkey + userhash), asistente de migración en Ajustes y
   marcado «Cancelado» de los resultados de búsqueda
   (`IsCancelledFileByID`).

6. **Kad**: expresiones de búsqueda restrictivas
   (`StartSearchTermsMatch`), publicación de palabras clave AICH, modelo
   `CKeyEntry`, identidad eMule 0.72a.

7. **Comportamiento de descarga**: purga de las fuentes `RemoteQueueFull` al
   80 % de `maxSources`, A4AF a nivel de socket (`SwapToAnotherFile`),
   ciclo de reconsulta de cola remota fiel, ventana UDP `(1 s, 2 min)`
   previa al re-ask TCP.

8. **Calidad de las pruebas**: compartición en `mule-test-support` y
   `tests/common/`, salvaguarda antiduplicados (`test_duplicates.ps1`).

---

## 2. Comandos de Validación

```powershell
# Verificación completa de compilación y lints
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Suite de pruebas Rust (100 % sin conexión)
cargo test --workspace --all-features

# Interop en vivo (opcional — requiere el binario eMule fijado)
$env:EMULE_REFERENCE_BIN = "D:\eMule0.72a\eMule0.72a\emule.exe"
$env:EMULE_REFERENCE_SHA256 = "<sha256 del binario>"
$env:EMULE_REFERENCE_PROFILE = "Emule072a"
cargo test -p mule-protocol --test tier3_loopback --all-features -- --include-ignored

# Validación de la aplicación Flutter Desktop
cd apps/desktop
flutter analyze
flutter test
```

---

## 3. Migraciones

- **`peer_credits`**: importación `clients.met` hacia la tabla SQLite —
  ninguna acción manual, migración in situ al arrancar.
- **Identidad eMule**: `cryptkey`/`userhash` importados — la reputación
  (créditos vistos por los pares) se conserva desde el primer arranque.
- **Asistente de migración**: Ajustes → Avanzado guía la importación de los
  `.met` desde una instalación de eMule existente.

---

## 4. Notas de Seguridad

- Los bloqueos de IP quedan ahora **acotados a 2 h** (`CLIENTBANTIME`) —
  fidelidad eMule; el comportamiento es *menos* restrictivo que antes en
  duración, pero *más* estricto en alcance (bloqueos propagados a las rutas
  TCP entrante y UDP de par).
- Ningún método `lab.*` en la variante `standard` — el paquete empaquetado
  se verifica mediante `test_packaged_bundle.ps1`.
- Pipeline anti-SSRF, aislamiento loopback, kill switch atómico y
  seudonimización de pares (`mule-network-policy`) sin cambios; las
  pruebas en vivo Tier-3 permanecen confinadas al loopback.

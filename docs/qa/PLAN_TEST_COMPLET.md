# Plan de Pruebas Completo — eMuleRust v0.2.0-RC2

Plan de validación en 5 fases, desde el banco automatizado hasta la interconexión
en red real. Cada fase es una **compuerta (gate)**: no se pasa a la siguiente
si la anterior no está en verde.

| Fase | Alcance | Automatizable | Estado |
| :--- | :--- | :--- | :--- |
| **A — Compilación e integridad** | Compilación, lint, pruebas unitarias/de integración, empaquetado | 100 % | ⏳ en curso |
| **B — Banco funcional** | Todas las funciones del daemon en loopback | ~90 % (RPC automatizables) | ⏳ |
| **C — Interoperabilidad binarios reales (Tier 3)** | eMule 0.50a, eMule 0.72a, aMule 3.1.0 en loopback cerrado | Arnés completo | ✅ **7/7 × 3 referencias** |
| **D — Red real** | eD2K público + Kad | Manual supervisado | ⏳ autorizado (ADR 0004) |
| **E — Go/No-Go** | Lista de comprobación final antes de push/release | Checklist | ⏳ |

---

## Fase A — Compilación e Integridad (gate 1)

| # | Prueba | Comando | Criterio de éxito |
| :--- | :--- | :--- | :--- |
| A.1 | Lint workspace | `cargo check --workspace --all-targets --all-features` | 0 errores |
| A.2 | Clippy estricto | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 warnings |
| A.3 | Suite Rust completa | `cargo test --workspace --all-features` | 100 % superado, sin conexión |
| A.4 | Flutter | `cd apps/desktop; flutter analyze; flutter test` | 0 incidencias, pruebas en verde |
| A.5 | Antiduplicación + todo en uno | `scripts\verify_all.ps1` | Código de salida 0 |
| A.6 | Build release | `scripts\build_windows_release.ps1` y luego flavor `qa-loopback` | `eMuleRust.exe` + ZIP + SHA256SUMS + SBOM generados |
| A.7 | Smoke test empaquetado | `scripts\test_packaged_bundle.ps1` (standard) y `-Flavor qa-loopback` | PASSED, daemon exit 0, `lab.*` ausente en standard |
| A.8 | Pipeline E2E loopback | `scripts\test_qa_loopback_e2e.ps1` | PASSED |
| A.9 | Matriz RC | `scripts\test_rc_matrix.ps1` | Todas las filas en verde |

> ✅ Estado a 17/10: A.1–A.4, A.6–A.7 (standard) ya validados en el commit `7a8a3d7`.
> Pendiente: A.5, A.6 (qa-loopback), A.7 (qa-loopback), A.8, A.9.

---

## Fase B — Banco Funcional (gate 2)

Todo se controla mediante WebSocket JSON-RPC en loopback — automatizable con
`Invoke-WsRpc` (véase `test_qa_loopback_e2e.ps1`).

### B.1 Identidad y Secure User Identification

| # | Caso | Verificación |
| :--- | :--- | :--- |
| B.1.1 | Generación RSA-384 | `cryptkey.dat` creado en el primer arranque, clave pública MD4 estable |
| B.1.2 | Handshake SUI entre 2 instancias locales | Firma verificada en ambos lados, `sui=true` |
| B.1.3 | Persistencia de créditos | `credits.list`/`credits.stats` coherentes tras reiniciar |
| B.1.4 | Importación de identidad eMule | `identity.importEmuleIdentity` sobre un `cryptkey.dat`/`preferences.dat` exportado de eMule 0.72a |
| B.1.5 | Importación clients.met | `credits.importClientsMet` sobre el `clients.met` de eMule |

### B.2 Ciclo de vida de las descargas

| # | Caso | Verificación |
| :--- | :--- | :--- |
| B.2.1 | Añadir mediante enlace `ed2k://` | Parseo estricto + etiquetas desconocidas preservadas |
| B.2.2 | Creación `.part` disperso + `.part.met` | Diseño conforme a `CPartFile` (gaps con nombre) |
| B.2.3 | Descarga en loopback (flavor qa) | Partes completas, progreso exacto |
| B.2.4 | Verificación por parte | Hash eD2K por parte + hash raíz conformes a `test-vectors.md` |
| B.2.5 | Corrupción inyectada → AICH | La prueba de Merkle detecta el bloque de 180 KiB defectuoso, redescarga selectiva |
| B.2.6 | Pausa / reanudación / eliminación | Transiciones de estado + persistencia SQLite |
| B.2.7 | Reanudación tras kill -9 del daemon | `save_active_part_mets` ha fijado el estado, reanudación sin pérdida |
| B.2.8 | Finalización | Fichero movido a `downloads/`, hash final == origen |

### B.3 Interoperabilidad de formatos de ficheros (sin red)

| # | Caso | Verificación |
| :--- | :--- | :--- |
| B.3.1 | Importar `.part.met` eMule 0.50a | `import_part_met` → reanudación exacta de las partes ya presentes |
| B.3.2 | Ídem eMule 0.72a | Informe de auditoría Fase 8.4 limpio |
| B.3.3 | Ídem aMule (formato `.part.met` aMule) | Gaps/traducciones correctas |
| B.3.4 | `known.met` / `emfriends.met` | Importación sin errores, datos aprovechados |
| B.3.5 | `nodes.dat` Kad | `kad.importNodesDat` → contactos enrutables |

### B.4 Compartidos y subida

| # | Caso | Verificación |
| :--- | :--- | :--- |
| B.4.1 | Cálculo de hash en carpeta entrante | Límites `ED2K_PART_SIZE` respetados |
| B.4.2 | Cola de subida | Ranura asignada, ranking de cola creciente |
| B.4.3 | USS (Upload Speed Sense) | ICMP/medición de latencia → ajuste del caudal |
| B.4.4 | `upload-single-slot` | Caudal por ranura acotado |

### B.5 Fuentes, servidores, búsqueda

| # | Caso | Verificación |
| :--- | :--- | :--- |
| B.5.1 | Source Exchange v4 (SX1/SX2) | Fuentes propagadas entre 2 daemons en loopback |
| B.5.2 | A4AF (Ask For Another File) | Cambio de fuente al mejor fichero |
| B.5.3 | server.met / conexiones de servidor (loopback) | Servidor simulado en loopback: inicio de sesión, búsqueda, fuentes |
| B.5.4 | Búsqueda por palabras clave | Indexación/respuesta conforme a `SEARCHTOLERANCE` |

### B.6 Red interna y seguridad

| # | Caso | Verificación |
| :--- | :--- | :--- |
| B.6.1 | Pipeline anti-SSRF | Rechazos 127.x/privadas/multicast en las rutas prohibidas |
| B.6.2 | `ipfilter.dat` | Par filtrado rechazado en el hello Y en el dial |
| B.6.3 | Kill switch | Conmutación instantánea, cero fugas |
| B.6.4 | Límites de tasa | Presupuestos `ResourceBudgets` respetados bajo carga |
| B.6.5 | Seudonimización | userhash con sal distinto por sesión |
| B.6.6 | Ofuscación | Handshake ofuscado + `obfuscationRequired` rechaza tráfico en claro |

### B.7 Kad DHT (loopback)

| # | Caso | Verificación |
| :--- | :--- | :--- |
| B.7.1 | Bootstrap entre 2+ daemons | Contactos intercambiados, tabla de enrutamiento rellena |
| B.7.2 | Búsqueda iterativa | Convergencia `SEARCHTOLERANCE` |
| B.7.3 | Publicar/Buscar fuentes y palabras clave | Ida y vuelta completa en DHT |
| B.7.4 | `kad.listContacts` / `clearContacts` | Introspección exacta |
| B.7.5 | Etiqueta de comprobación de cortafuegos | Etiqueta `eMuleRust` anunciada |

### B.8 MCP, CLI, supervisor

| # | Caso | Verificación |
| :--- | :--- | :--- |
| B.8.1 | Contrato de las 87 herramientas MCP | `mule-mcp`: cada herramienta → JSON-RPC válido |
| B.8.2 | `mule-cli` | Cada comando ↔ mismo resultado que RPC |
| B.8.3 | Asistente local | Perfiles de herramientas, orquestador loopback |
| B.8.4 | Matriz del supervisor | QA_RECEPTION_PROTOCOL §5 (puerto ocupado, 2.ª instancia, kill, acentos, cierre) |
| B.8.5 | Recarga en caliente de ajustes | `settings.set` aplicado en caliente + persistido |

---

## Fase C — Interoperabilidad Binarios Reales en Loopback (gate 3)

Arnés existente: `crates/mule-protocol/tests/tier3_external_reference/`
(característica `ed2k-reference-loopback`, pruebas con `#[ignore]`, lista de comprobación de 10
condiciones — véase su README).

### C.0 Preparación común (por binario de referencia)

1. **Fijación criptográfica** — calcular y fijar el SHA-256:
   ```powershell
   Get-FileHash 'D:\eMule0.50a\emule.exe' -Algorithm SHA256
   Get-FileHash 'D:\eMule0.72a\eMule0.72a\emule.exe' -Algorithm SHA256
   Get-FileHash 'D:\aMule-3.1.0-Windows-x64\amule-portable-x64\bin\amule.exe' -Algorithm SHA256
   ```
2. Declarar `reference_manifest.json` (proyecto, versión, SHA-256, perfil
   de compatibilidad, limitaciones conocidas).
3. **Configuración hermética** del cliente de referencia: `EnableKad=0`,
   `AutoConnect=0`, `AutoUpdateServerList=0`, UPnP desactivado, webserver desactivado,
   carpeta `config/` dedicada copiada al sandbox — nunca la configuración
   original.
4. Solo puertos efímeros en loopback, `--test-threads=1`, timeouts ≤ 3 s.
5. Espacio IP neutro: ambos pares se ven como `127.0.0.1`.

### C.1 Matriz por cliente

| Prueba | eMule 0.50a | eMule 0.72a | aMule 3.1.0 | Qué queda demostrado |
| :--- | :-: | :-: | :-: | :--- |
| HELLO / HELLOANSWER | ✅ | ✅ | ✅ | Handshake, etiquetas, versiones mutuamente reconocidas |
| `OP_HELLO` perfil extendido | — | ✅ | ✅ | Protocolo extendido, bits de características |
| Petición de fichero + FileAnswer | ✅ | ✅ | ✅ | `OP_FILEREQANSNOFILE`/`OP_FILEREQANSWER` conformes |
| Envío de partes eMule → eMuleRust | ✅ | ✅ | ✅ | Decodificación `OP_SENDINGPART` real |
| Envío de partes eMuleRust → eMule | ✅ | ✅ | ✅ | La interfaz de eMule muestra el progreso correcto |
| `OP_COMPRESSEDPART` | ✅ | ✅ | ✅ | Interoperabilidad de streaming zlib |
| Ranking de cola remoto | ✅ | ✅ | ✅ | Posición en cola realista |
| AICH (prueba + petición) | — | ✅ | ✅ | Interoperabilidad Merkle en 180 KiB |
| Ofuscación | — | ✅ | ✅ | Cifrado de protocolo |
| Source Exchange v4 | — | ✅ | ✅ | SX1/SX2 frente a implementación real |
| Reanudación cruzada `.part.met` | ✅ | ✅ | ✅ | Fichero compartido entre ambos mundos |

> ✅ **EJECUTADO el 03/03** — las 7 pruebas en vivo pasan en las tres
> referencias fijadas:
> | Binario | SHA-256 (prefijo) | Perfil | Resultado |
> | :--- | :--- | :--- | :--- |
> | `D:\eMule0.50a\emule.exe` | `06CDF814…` | `Emule050a` | 7/7 |
> | `D:\eMule0.72a\eMule0.72a\emule.exe` | `B7DD6276…` | `Emule072a` | 7/7 |
> | `D:\…\amule-portable-x64\bin\amuled.exe` | `6D3B1309…` | `Amule310` | 7/7 |
>
> Adaptaciones entregadas para aMule: `ReferenceKind` (binario ejecutado in situ
> — DLLs adyacentes — con `-c <config_dir>`), `amule.conf` neutralizado +
> `shareddir.dat` (indexación del fixture), EC confinado a `127.0.0.1`,
> `taskkill` genérico, aserciones de versión dependientes del perfil
> (aMule codifica de forma distinta → registrado como divergencia «info»).
> Manifiesto rellenado: `tests/fixtures/ed2k/reference_manifest.json`.

```powershell
# Ejecución (tras preparar el manifiesto)
$env:EMULE_REFERENCE_BIN = "D:\eMule0.50a\emule.exe"
$env:EMULE_REFERENCE_SHA256 = "06CDF814387F627A4BD05A0C68211F715BFA952423E8E8A462E1F47C11A4D20E"
$env:EMULE_REFERENCE_PROFILE = "Emule050a"   # Emule072a | Amule310
cargo test -p mule-protocol --test tier3_loopback --features ed2k-reference-loopback -- --ignored --test-threads=1
```

---

## Fase D — Red Real eD2K/Kad (gate 4)

> ✅ **Autorizado**: el ADR `0004-public-ed2k-kad-production.md` sustituye
> al ADR 0002 (fase de construcción terminada → producción). El perfil
> `public-ed2k-kad-preview` pasa a ser el modo público soportado. La pureza
> de `mule-protocol` y el confinamiento por defecto siguen vigentes.
> UPnP: `upnp.setEnabled` explícito; el desmapeo al detenerse está acotado a
> 500 ms (fix `d0ed8bf`).

| # | Paso | Verificación |
| :--- | :--- | :--- |
| D.1 | `server.met` obtenido de una fuente fiable | Lista cargada sin conexión desde fichero |
| D.2 | Conexión a 1 servidor público de confianza | Inicio de sesión OK, **HighID** obtenido vía UPnP (o LowID documentado) |
| D.3 | Bootstrap Kad | `kad.bootstrap` desde `nodes.dat` de eMule → contactos reales |
| D.4 | Búsqueda real por palabras clave | Resultados recibidos, fuentes listadas |
| D.5 | Descarga real | Únicamente contenido libremente redistribuible (ISO de Linux, corpus de pruebas eD2K, creaciones propias) |
| D.6 | Subida observada | Ranuras abiertas, progreso hacia pares reales |
| D.7 | Prueba de resistencia 2–4 h | Memoria estable, sin fugas de handles/sockets, registros sin errores fatales |
| D.8 | Apagado | `system.stop` → desmapeo UPnP verificado en el router, cero procesos residuales |

---

## Fase E — Go/No-Go antes de push/release

- [x] Fase A: 100 % en verde (`verify_all.ps1`, clippy, pruebas del workspace,
      Flutter 123/123, matriz RC 7/7, smoke test del paquete estándar)
- [x] Fase B: banco funcional **134/134 PASS** (commit `42bd5ed`)
- [x] Fase C: interoperabilidad demostrada en las **3 referencias** — 7/7 frente a
      eMule 0.50a, 7/7 frente a eMule 0.72a, 7/7 frente a aMule 3.1.0 (SHA-256
      fijados en `reference_manifest.json`)
- [x] Fase D parcial: `test_phase_d_public_network.ps1` **21/21 PASS**
      (importación de 16 servidores reales, sesión eD2K establecida en LowID,
      95 contactos Kad, búsquedas eD2K+Kad reales, disconnectAll,
      mapeo/desmapeo UPnP limpio). **Pendiente sin cubrir**: D.5
      descarga real, D.6 subida observada, D.7 soak de 2–4 h —
      a valorar en el Go/No-Go (sesión de usuario prolongada).
- [x] `version.json.gitDirty == false` en el paquete final (build
      `42bd5ed`, SHA-256 `6b48d120…`)
- [ ] CHANGELOG actualizado para los fixes `d0ed8bf` / `7a8a3d7` / `63430e8`
      / `fe729ed` / `8b932a3`
- [ ] `git push -u origin master`

### Observación Fase D — LowID

La sesión de servidor se estableció en **LowID** (clientId < 0x01000000)
mientras que UPnP informa `tcpMapped=true` en la puerta de enlace. Hipótesis:
prueba de callback TCP del servidor ejecutada antes de finalizar el mapeo, o
puerto externo ≠ interno. Comportamiento funcionalmente aceptable para
una RC (LowID funciona, degradado), a documentar en las notas de la
versión si se confirma.

## Notas

- La certificación oficial actual (README Tier 3) indica: *«no se reivindica
  interoperabilidad en vivo mientras Tier 3 no haya corrido contra un
  binario fijado»*. El plan Fase C levanta exactamente esta reserva.
- No reutilizar nunca los `config/` originales de los clientes de referencia
  (contienen identidades e historiales reales) — siempre copias
  herméticas.
- Todas las pruebas de las Fases A–C permanecen sin conexión; solo la Fase D toca
  Internet.

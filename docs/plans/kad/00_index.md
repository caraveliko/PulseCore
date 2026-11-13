# Plan de mejora Kad — Índice

> **Estado**: planificado — ninguna etapa implementada.
> Cada etapa es un commit/PR independiente, compilable y probado de forma aislada.
> Restricción absoluta: **compatibilidad en el cable eMule 0.72a** — cada cambio es o bien una política local invisible en el cable, o bien un comportamiento que eMule habla nativamente (verificado en `D:\eMule0.72a-Sources\srchybrid`).

---

## Diagnóstico verificado (estado actual)

### Conforme a eMule — no tocar

| Comportamiento | Verificación |
|---|---|
| Datagramas Kad UDP en claro (`0xE4`) | eMule no ofusca el tráfico Kad ordinario; RC4 reservado a los paquetes con clave (cortafuegos `FIREWALLUDP`, verify keys) |
| Sin `SEARCHTOLERANCE` en `SEARCH_*_REQ` entrantes | `KademliaUDPListener.cpp:1052-1095` — eMule sirve el índice sin comprobación de distancia; la tolerancia solo existe en los `PUBLISH_*_REQ` (líneas 1188, 1286, 1519) |
| Puerta del drenaje bootstrap (`has_verified`) | `Kademlia.cpp:287` — `!IsConnected()` = `HasHadContact()`; cadencias 2 s/15 s idénticas (`kad.rs:586-599`) |
| `KadId` aleatorio persistido, rotación únicamente mediante `identityRotationHours` (por defecto 0) | eMule genera también un KadID aleatorio persistido |
| Promoción desde replacement cache al desalojar | `KBucket::remove` ya lo hace (`routing_table.rs:98-108`) |

### Carencías reales — objeto de este plan

| Carencia | Ubicación |
|---|---|
| Candidato LRU devuelto por `insert_or_update` nunca sondeado | `routing_table.rs:82-95` — retorno no consumido en ningún sitio (7 sitios: `kad.rs:187/290/353/539/819`, `handlers.rs:1025`, `lookup.rs:152`) |
| `HELLO_REQ` nunca emitido (codificador presente, cero llamadores) | `encoder.rs:124-163` vs. `routing_client.rs` |
| Drenaje bootstrap se detiene en el primer contacto verificado (fiel pero frágil) | `kad.rs:586-595` |
| Estado cortafuegos congelado tras conclusión (puerta `!fw.verified`) | `kad.rs:694-705` |
| Ventana de recogida search fija 350 ms | `search_client.rs:19-47` |
| `send_search_sources` fija en duro `start_pos=0` | `search_client.rs:85` |
| Búsqueda: una sola palabra clave, sin expresión restrictiva saliente, sin paginación | `search_keywords.rs` |
| Coalescing `in_flight_searches` devuelve `Ok(vec![])` al duplicado | `kad.rs:45`, `search_sources.rs:24-54` |
| `publish_keyword` anuncia `source_count: 1` constante | `search_keywords.rs:230` |
| Buddy: rol relevo solo, sin rol cliente LowID | `types.rs:251-346` |
| Emparejamiento de transacciones por IP sola | `kad_udp.rs` receive loop ~l.478 |
| `find_closest` = recogida + ordenación bajo el Mutex global | `routing_table.rs:229-251` |

### Exclusiones definitivas (probadas no-eMule o imposibles)

- ❌ Tolerancia en búsquedas entrantes — eMule no hace ninguna, añadir una diverge.
- ❌ Nonce de correlación en el cable — Kad2 no transporta ningún nonce arbitrario.
- ❌ Ofuscación global de los datagramas Kad — no estándar, rompe la interoperabilidad.
- ❌ Rotación periódica agresiva del KadID — ya desactivada por defecto.
- ❌ Enum `BucketInsertOutcome` — `Option<KadContact>` basta; la carencia es el consumidor, no el tipo.

---

## Orden de ejecución

| # | Etapa | Archivo | Dependencias |
|---|---|---|---|
| 1 | Probing LRU (`ContactVerifier`) | [01_probing_lru.md](01_probing_lru.md) | Ninguna — `udp.ping()` existe |
| 2 | `HELLO_REQ` saliente | [02_hello_req.md](02_hello_req.md) | Etapa 1 (el verifier aloja la lógica) |
| 3 | Revalidación cortafuegos | [03_firewall_revalidation.md](03_firewall_revalidation.md) | Ninguna — entregable como quick win desde la etapa 1 |
| 4 | Salud bootstrap (`BootstrapHealth`) | [04_bootstrap_health.md](04_bootstrap_health.md) | Etapas 1–3 (métricas producidas por el probing) |
| 5 | Robustez UDP (errores tipados, reintento, ventana deslizante, emparejamiento) | [05_udp_robustesse.md](05_udp_robustesse.md) | Etapa 4 (la taxonomía de errores sirve para `Degraded`) |
| 6 | Contención (instantánea + ordenación fuera de bloqueo) | [06_contention_mutex.md](06_contention_mutex.md) | Ninguna — pequeña, intercalar entre dos |
| 7 | Búsqueda (expresión restrictiva, multitérmino, paginación, coalescing) | [07_recherche.md](07_recherche.md) | Etapa 5 (la paginación depende del transporte fiable) |
| 8 | Buddy cliente LowID | [08_buddy_client.md](08_buddy_client.md) | Contactos verificados estables + emparejamiento endurecido (etapas 1, 5) |
| 9 | Validación interop (golden packets, proptest, laboratorio loopback) | [09_validation_interop.md](09_validation_interop.md) | Transversal — hitos vinculados a cada etapa |

```text
1 Probing ──► 2 HELLO_REQ ──┐
3 Firewall  ────────────────┼─► 4 BootstrapHealth ──► 5 UDP ──► 7 Recherche ──► 8 Buddy
                            └─► 6 Contention (libre)      ▲
                            9 Validation (continua) ──────┘
```

---

## Restricciones transversales (recordatorio AGENTS.md)

- **Ningún magic number**: umbrales/timeouts → `const` documentada al principio del archivo; ajustes operacionales → `DaemonConfig`; presupuestos de red → `ResourceBudgets`; cotas en el cable → `limits.rs`.
- **Un componente por archivo**: `contact_verifier.rs`, `bootstrap_health.rs`, `buddy_client.rs` son archivos dedicados.
- **Pruebas sin conexión únicamente**: loopback, sin tráfico saliente.
- **`mule-protocol`**: `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]`, round-trip obligatorio, verificación frente a `srchybrid` + `docs/ed2k/` antes de cualquier cambio en el cable.
- **Seguridad**: no debilitar nunca anti-SSRF, aislamiento loopback, kill switch, seudonimización.
- **Código y doc en francés**, `thiserror` para los errores, `tracing` para los registros, cero warnings.

## Validación por etapa

```powershell
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
# + cargo test -p mule-protocol --all-features  (etapas que tocan el protocolo: 2, 7a, 8)
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\verify_all.ps1
```

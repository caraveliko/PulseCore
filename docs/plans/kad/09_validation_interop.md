# Etapa 9 — Validación interop (transversal)

> **Naturaleza**: continua — cada etapa entregada incluye sus pruebas; esta etapa cubre las herramientas transversales.
> **Restricción**: todo es sin conexión — loopback, corpus de archivos, propiedades. Cero tráfico saliente.

## 9a. Golden packets (`mule-protocol/tests/`)

- Corpus `crates/mule-protocol/tests/fixtures/kad/*.bin` — capturas reales eMule/aMule (cifrado retirado: `0xE4` en claro).
- Prueba: para cada archivo, `decode` → `encode` → bytes idénticos (round-trip determinista, exigido por `mule-protocol` AGENTS).
- Prioridad para los paquetes tocados por las etapas: `HELLO_*`, `SEARCH_*_RES` multiforma, `FINDBUDDY_*`, `PUBLISH_*`.

## 9b. Pruebas de propiedad (`proptest` ya en dev-dep workspace)

| Propiedad | Objetivo |
|---|---|
| `xor_distance` simétrica, no negativa, `dist(a,a)=0` | `kad_id.rs` |
| `bucket_index_for` ∈ `0..128`, monótona frente a distancia | `routing_table.rs` |
| `is_within_search_tolerance` ↔ `Get32BitChunk(0) ≤ SEARCHTOLERANCE` (boundary DWORD ordering) | `kad_id.rs` |
| `from_hash_be` ∘ `to_hash_be` = identidad | `kad_id.rs` |
| Paginación: `start_pos` arbitrario → el decodificador acota las entradas | `search_expr.rs`/decoder |

## 9c. Laboratorio multinodo loopback

Extensión de `scripts/test_qa_loopback_e2e.ps1` + arnés `mule-daemon` (feature `lab-loopback` únicamente):

- **Pérdida**: descarte aleatorio acotado de datagramas → la tabla sobrevive (probing + mark_failed).
- **Duplicación**: mismo paquete recibido 2× → dedup transacción, sin doble `mark_seen`.
- **Retardo/reordenamiento**: paquetes diferidos → la ventana deslizante recoge correctamente.
- **Cambio de NAT simulado**: respuesta `FIREWALLED_RES` con IP diferente → revalidación disparada (etapa 3).
- Aserción final: convergencia bootstrap hasta un umbral de contactos verificados incluso con semillas parcialmente muertas.

## 9d. Fuzzing acotado de los decodificadores

`cargo-fuzz` o `proptest` a nivel de byte sobre los puntos de entrada `kad/decoder/*`: tamaños de campos, `tag counts`, profundidad de árbol de expresión — nunca reserva proporcional a un campo no validado (`limits.rs`).

## Hitos por etapa

| Etapa | Validación añadida |
|---|---|
| 1–2 | Probe/HELLO loopback + corpus `HELLO_*` |
| 3 | Prueba `mark_stale` sobre IP cambiada |
| 4 | Pruebas `assess` sobre tablas sintéticas |
| 5 | Escenarios pérdida/dup/reordenamiento |
| 7 | Corpus `SEARCH_*` multiforma + round-trip expresión |
| 8 | Escenario buddy completo loopback |

## Criterio global de salida

`scripts/verify_all.ps1` en verde + cero divergencias encode/decode sobre corpus real + tabla Kad que sobrevive a los escenarios de caos loopback.

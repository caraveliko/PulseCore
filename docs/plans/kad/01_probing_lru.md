# Etapa 1 — Probing LRU (`ContactVerifier`)

> **Dependencias**: ninguna (`KadUdpService::ping` ya existe).
> **Riesgo interop**: nulo — `PING`/`PONG` (0x60/0x61) son estándar eMule.

## Objetivo

Cuando un bucket está lleno y se propone un nuevo contacto, `insert_or_update` coloca el nuevo en la replacement cache y devuelve el **LRU** (menos recientemente visto) como candidato de sondeo. Hoy este retorno se ignora en todas partes: los contactos muertos nunca se reemplazan por los candidatos frescos. Es el defecto de liveness n.º 1.

## Estado verificado

- `KBucket::insert_or_update` → `Option<KadContact>`: `Some(lru)` cuando el bucket está lleno — `routing_table.rs:82-95`.
- `KBucket::remove` promociona ya `replacements.pop_back()` → `contacts.push_back` — `routing_table.rs:98-108`.
- `KBucketTable::mark_failed` incrementa `fail_count`, desaloja en el umbral — `routing_table.rs:170-203`.
- `mark_seen` reinicia `verified`/`fail_count` — `routing_table.rs:207-213`.
- `udp.ping(addr)` completo con transacción `Pong` — `routing_client.rs:20-88`.
- `learn_contact` (punto caliente): `handlers.rs:1005-1026`, llamado desde ~4 sitios, contexto handler struct línea ~41.

## Cambios

### 1a. Nuevo archivo `crates/mule-daemon/src/services/kad/contact_verifier.rs`

```rust
/// Candidato de sondeo LRU devuelto por un bucket lleno.
pub struct ProbeQueue {
    queue: VecDeque<KadContact>,
    queued: HashSet<KadId>,              // dedup
    cooldowns: HashMap<KadId, Instant>,  // cooldown por contacto
}
```

- `ProbeQueue::push(contact)` — ignora si ya está en cola o en cooldown; cota `PROBE_QUEUE_CAP` (descarta los más antiguos en cabeza si está saturada).
- `ProbeQueue::take_next()` — retira y marca el cooldown.
- `ContactVerifier`: tarea Tokio — bucle `select!` sobre cancelación + despertar periódico; extrae los candidatos; `Semaphore::new(PROBE_CONCURRENCY)` acota los `ping` concurrentes; en la respuesta → `table.mark_seen(kad_id, now)`; en fallo → `table.mark_failed(kad_id)` (el desalojo + promoción son automáticos en `KBucket::remove`).

### 1b. Cableado

| Sitio | Acción |
|---|---|
| `kad.rs` struct `KadService` | Campo `probe_queue: Arc<tokio::sync::Mutex<ProbeQueue>>` |
| `kad.rs:187, 290, 353, 539, 819` | `if let Some(lru) = table.insert_or_update(c) { probe_queue.push(lru) }` — empujar **después** de liberar el bloqueo de la tabla |
| `lookup.rs:152` | Ídem |
| `handlers.rs` contexto (~l.41) | Campo `probe_queue`; `learn_contact` (l.1025) captura el retorno y empuja |
| `kad.rs:start_background_maintenance` | `tokio::spawn` del worker con `cancel_token` |

**Orden de los bloqueos**: nunca `probe_queue.lock()` mientras `routing_table.lock()` — empujar en un `Vec` tampón y luego drenar fuera de bloqueo (coherente con la regla existente «`in_flight` liberado antes de `learn_contact`»).

## Constantes (cabecera de `contact_verifier.rs`, documentadas)

| Nombre | Valor | Rol |
|---|---|---|
| `PROBE_CONCURRENCY` | `6` | Sondeos UDP simultáneos máx. |
| `PROBE_COOLDOWN` | `15 min` | Plazo mín. entre dos sondeos del mismo KadID |
| `PROBE_QUEUE_CAP` | `128` | Cola acotada — antiflood sobre paquetes entrantes |

## Pruebas

- `mule-core` (existente por completar): bucket lleno → insert → `Some(lru)` devuelto; `mark_failed` repetido → desalojo → candidato de la replacement cache promocionado a `contacts`.
- `mule-daemon` (loopback): socket UDP ficticio que no responde → el contacto es desalojado y reemplazado; socket que responde `Pong` → `mark_seen` (verified).
- Cooldown: dos inserciones del mismo candidato en la ventana → un solo sondeo.

## Criterios de finalización

- [ ] Un LRU muerto es reemplazado por un candidato de la replacement cache **sin acción manual**.
- [ ] Un LRU vivo pasa a `mark_seen` y permanece.
- [ ] La cola está acotada y deduplicada; ningún sondeo bajo cooldown.
- [ ] Registros `tracing` estructurados: `kad.probe.attempt/ok/fail/promoted`.

## Referencia eMule

`CRoutingZone::OnBigTimer` / `CContact::CheckingType` — eMule sondea el LRU y arranca un temporizador de desalojo; el nuevo contacto espera en la zona de reemplazo.

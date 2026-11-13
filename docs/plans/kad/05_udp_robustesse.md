# Etapa 5 — Robustez UDP

> **Dependencias**: etapa 4 (el cálculo `Degraded` consume la taxonomía de errores creada aquí).
> **Riesgo interop**: nulo — todo es local (clasificación, reintento saliente, emparejamiento, ventana de recogida).

## 5a. Taxonomía de errores

`kad_udp/types.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum KadTxError {
    #[error("timeout")] Timeout,
    #[error("policy denied: {0}")] PolicyDenied(String),
    #[error("socket io: {0}")] SocketIo(String),
    #[error("malformed response")] Malformed,
    #[error("cancelled")] Cancelled,
}
```

Los `send_*` (`routing_client.rs`, `search_client.rs`, `publish_*`) devuelven `Result<T, KadTxError>` en lugar de `String`. **Regla de uso** en los llamadores (`lookup.rs`, `bootstrap_*`, verifier):

| Error | Acción sobre el contacto |
|---|---|
| `Timeout`, `Malformed` | `mark_failed` (el par es culpable) |
| `SocketIo`, `PolicyDenied`, `Cancelled` | **nada** — fallo local, no penalizar nunca al par |

## 5b. Ventana de recogida deslizante

`search_client.rs:29-47` — `collect_search_batches`:

```rust
// Antes: timeout(SEARCH_RESP_IDLE_WINDOW, rx.recv()) — fija
// Después: fecha límite global + idle rearmado en cada paquete válido
let deadline = Instant::now() + SEARCH_COLLECT_DEADLINE;
loop {
    match timeout_at(deadline.min(now + SEARCH_RESP_IDLE_WINDOW), rx.recv()).await {
        Ok(Some(Ok(msg))) if let Some(more) = extract(msg) => { acc.extend(more); /* idle rearmado por el bucle */ }
        ...
    }
}
```

Constantes: `SEARCH_RESP_IDLE_WINDOW = 350 ms` (sin cambios, pero se vuelve *deslizante*), `SEARCH_COLLECT_DEADLINE = 2 s` (tope absoluto), límites `12` paquetes / `300` entradas sin cambios.

## 5c. Reintento acotado con jitter

`routing_client.rs` — helper interno `send_with_retry(fut_factory)`: **un** reintento tras `RETRY_DELAY + jitter` (`RETRY_JITTER_MS = 400`) únicamente sobre `Timeout`, únicamente para `ping` / `send_bootstrap` / `send_find_node`. **Nunca** sobre `SEARCH_*`/`PUBLISH_*` (eMule no responde cuando no tiene nada → reintento inútil; amplificación).

Atención: cada intento = nueva transacción (`tx_id`, `InFlightTxGuard`) — no reutilizar el mismo `tx_id`.

## 5d. Emparejamiento de transacciones endurecido

`kad_udp.rs` bucle de recepción (~l.478-519) — hoy emparejamiento por IP sola:

1. Buscar primero las transacciones con `target == SocketAddr exact (ip, port)` + `expected_response` + `expected_target_hash` correspondientes.
2. Alternativa `(ip, opcode, hash)` únicamente si **un solo** candidato (NAT cambia el puerto origen — eMule tolera).
3. Si no: descartar + `tracing::warn!(kad.tx_ambiguous = ...)` — nunca enrutar arbitrariamente.

**Sin nonce**: Kad2 no transporta ningún nonce arbitrario — la correlación sigue siendo `(ip, port, opcode, target_hash)`.

## Pruebas

- `KadTxError`: timeout ficticio → `Timeout`; socket cerrado → `SocketIo`; el llamador no hace `mark_failed` sobre `SocketIo`.
- Recogida: paquetes en t=0, t=500 ms, t=900 ms → todos recogidos (idle rearmado); silencio >350 ms → fin; tope 2 s respetado.
- Emparejamiento: dos transacciones misma IP puertos diferentes → cada una recibe su respuesta; dos transacciones mismo (ip,puerto,opcode) → descartar + warn.
- Reintento: socket que descarta el 1.er datagrama → éxito en el 2.º; sin 3.er intento.

## Criterios de finalización

- [ ] Ningún `mark_failed` por error local.
- [ ] Paquetes tardíos (<2 s) recogidos; idle corto sigue siendo posible para búsqueda rápida.
- [ ] Colisión de transacciones observable en registro, nunca enrutada al azar.

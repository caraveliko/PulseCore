# Etapa 2 — `HELLO_REQ` saliente

> **Dependencias**: etapa 1 (el `ContactVerifier` consume la primitiva).
> **Riesgo interop**: nulo — HELLO es el mecanismo de verificación estándar Kad ≥ 8; la cadena ACK de 3 vías ya está gestionada en el lado entrante.

## Objetivo

`KadCodec::encode_hello_req` existe (`encoder.rs:124-163`, `codec.rs:42-67`) pero **ningún llamador**: los contactos aprendidos solo se verifican activamente mediante `ping` (que demuestra la IP, no la identidad Kad). HELLO verifica KadID + versión + puertos — es el canal eMule para «verificar activamente los nuevos candidatos útiles».

## Cambios

### 2a. `crates/mule-daemon/src/services/kad_udp/routing_client.rs`

`pub async fn send_hello(&self, target: SocketAddr, peer_version: u8) -> Result<KadHelloRes, String>` — misma plantilla que `ping` (l.20-88):

1. Transacción `KadExpectedResponse::HelloRes` (variante existente, `types.rs:58`).
2. `KadCodec::encode_hello_req(&local_id, udp_port, OUR_KAD_VERSION, tcp_port, misc_options)`.
3. Al recibir `HelloRes` → enviar `encode_hello_res_ack(&local_id)` (3-way, `KademliaUDPListener.cpp`: HELLO_RES espera un ACK que instala la reciprocidad).
4. Devolver la estructura decodificada (puertos/ID vistos por el par).

### 2b. `contact_verifier.rs` — política de sondeo

| Caso | Primitiva |
|---|---|
| Contacto **no verificado**, `version >= MIN_KAD2_HELLO_VERSION` | `send_hello` (demuestra IP + identidad + versión) |
| Contacto **verificado** (LRU de bucket lleno) o versión < umbral | `ping` (existente) |

**⚠️ A verificar antes de implementar**: cota de versión exacta en `KademliaUDPListener.cpp` / `Kademlia.cpp` — eMule solo envía HELLO a los contactos Kad2 (`KADEMLIA_VERSION` ≥ umbral por confirmar en `Protocol.h`/opcodes). Poner la constante `MIN_KAD2_HELLO_VERSION` al principio de `contact_verifier.rs`.

### 2c. `handlers.rs`

Verificar que `HelloRes` entrante (fuera de transacción esperada) y `HelloResAck` entrante alimentan `learn_contact` con `mark_seen` — comportamiento ya presente por confirmar.

## Pruebas

- Loopback: nodo ficticio responde `HelloRes` → `send_hello` devuelve la estructura + ACK emitido (capturar el datagrama saliente, opcode `0x22`).
- Verifier: contacto no verificado Kad ≥ umbral → HELLO; versión inferior → ping.

## Criterios de finalización

- [ ] `HELLO_REQ` emitido y secuencia `REQ → RES → ACK` observada en loopback.
- [ ] Un contacto no verificado promocionado a `verified` tras HELLO exitoso.
- [ ] Ningún HELLO hacia pares Kad1 (versión < umbral).

## Referencia eMule

`KademliaUDPListener.cpp` `Process_KADEMLIA2_HELLO_REQ/RES/RES_ACK`; `CRoutingZone` utiliza HELLO para verificar un contacto antes de la inserción definitiva.

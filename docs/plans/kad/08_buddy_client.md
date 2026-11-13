# Etapa 8 — Buddy cliente LowID

> **Dependencias**: etapas 1 + 5 — exige contactos **verificados** estables y un emparejamiento de transacciones endurecido. Es la pieza más pesada: colocarla al final.
> **Riesgo interop**: `FINDBUDDY_REQ`/`CALLBACK_REQ` son estándar en el cable en eMule — compatible por construcción, **pero** verificar el diseño exacto en `KademliaUDPListener.cpp` + `Packets.cpp` antes de codificar.

## Objetivo

Hoy solo existe el rol **relevo** (`BuddyRegistry`, `types.rs:251-346`: pending/active, `BUDDY_PENDING_TTL`). Un nodo **LowID** no puede publicar sus fuentes ni ser llamado de vuelta — necesita encontrar y mantener un buddy.

## Estado verificado

- `BuddyRegistry`: `register_pending`, `take_pending`, `set_active`, `clear_active`, `send_to_buddy` — lado **relevo** únicamente.
- `FINDBUDDY_RES` / `CALLBACK_REQ`: decodificación existente (por confirmar en `decoder/`).
- Detección LowID: ya conocida en el lado eD2K (HighID vs. LowID) — utilizada para la publicación de fuente.
- `persist_indexed_source` / publish path: ¿`KadIndexedSource` ya lleva `buddy_id` (nullable)? Por verificar — si no, añadir el campo.

## Cambios

### 8a. `mule-protocol` — `encode_findbuddy_req`

`encoder.rs` + `types.rs` wire: `FINDBUDDY_REQ` (0x51) — diseño por verificar en `KademliaUDPListener.cpp` (`Process_KADEMLIA_FINDBUDDY_REQ`). Prueba round-trip + vector si está disponible.

### 8b. `kad_udp/buddy_client.rs` *(nuevo archivo)*

```rust
pub struct BuddyClientState {
    pub our_buddy: Option<OurBuddy>,
}

pub struct OurBuddy {
    pub buddy_id: [u8; 16],
    pub buddy_addr: SocketAddr,       // par relevo
    pub link_tx: mpsc::UnboundedSender<Vec<u8>>, // nuestro socket TCP hacia él
    pub expires: Instant,
}
```

- `send_findbuddy(target, buddy_id) -> Result<FindBuddyRes, KadTxError>` — transacción sobre `FINDBUDDY_REQ`.
- Selección de candidato: `verified && version >= MIN_BUDDY_VERSION && UDP-open` — los criterios exactos están en eMule (`CClientList`/`Kademlia`) — por verificar.

### 8c. `kad.rs` — establecimiento del enlace

1. `is_low_id()` (desde el estado eD2K) → si LowID, buscar un candidato.
2. `send_findbuddy` → `FINDBUDDY_RES` OK → **abrir una conexión TCP saliente** hacia el buddy (peer_session dedicada, marcada «buddy link»).
3. Enlace mantenido: TTL, un solo buddy a la vez (espejo `BuddyRegistry`), failover ante pérdida (clear → re-candidato).

### 8d. Publicación LowID (`search_sources.rs` / publish path)

Publicar una fuente LowID (`tcp_port=0` + `buddy_id` informado) **únicamente si `our_buddy` activo** — nunca basándose únicamente en un `buddy_id` recibido. El `CALLBACK_REQ` entrante debe poder ser retransmitido por el buddy mediante el enlace TCP.

## Pruebas

- Loopback: nodo LowID ficticio → `send_findbuddy` → RES → enlace TCP abierto → publicación con `buddy_id`.
- Failover: kill del enlace → reselección de otro candidato.
- Nunca publish LowID sin `our_buddy` activo.

## Criterios de finalización

- [ ] Fuente LowID publicada con `buddy_id` válido y accesible mediante callback retransmitido.
- [ ] Un solo buddy; failover ante pérdida del enlace.
- [ ] Cero publish LowID huérfano.

## Referencia eMule

`KademliaUDPListener.cpp` `Process_KADEMLIA_FINDBUDDY_REQ/CALLBACK_REQ`; `CClientList::m_pBuddy`; `OP_REASKCALLBACKUDP` → `OP_REASKCALLBACKTCP`.

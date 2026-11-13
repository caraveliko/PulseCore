# AGENTS.md — mule-protocol (reglas específicas)

> Estas reglas complementan el [AGENTS.md raíz](../../AGENTS.md) y prevalecen aquí.

`mule-protocol` es la crate más sensible del workspace: define el entramado, los códecs y los límites del protocolo de cable eD2K/Kad. Cualquier error aquí rompe la interoperabilidad con eMule 0.72a o abre un fallo de análisis (*parsing*).

**Referencia de verdad**: fuentes de eMule 0.72a en `D:\eMule0.72a-Sources\eMule0.72a-Sources\srchybrid`. En caso de duda sobre un comportamiento en cable, leer el código C++ original (p. ej. `Packets.cpp`, `ClientUDPSocket.cpp`, `kademlia/`).

## Restricciones impuestas por el código

- `#![forbid(unsafe_code)]` — no es posible usar `unsafe`; no lo añadas nunca.
- `#![deny(missing_docs)]` — todo elemento público debe estar documentado.
- Cero E/S, cero red: esta crate es pura (datos → bytes, bytes → datos).

## Reglas de modificación

1. **Especificación primero**: antes de tocar un opcode, una etiqueta o un entramado, leer la especificación correspondiente en `docs/ed2k/` (`opcodes.md`, `framing.md`, `message-catalog.md`, `messages/`) y los ADR en `docs/ed2k/decisions/`.
2. **Límites acotados**: toda decodificación debe respetar `limits.rs` (`MAX_FRAME_BYTES`, `MAX_STRING_BYTES`, `MAX_TAGS_PER_MESSAGE`, `PacketLimits`). Nunca realizar una asignación proporcional a un campo de longitud no validado.
3. **Determinismo**: la codificación debe ser determinista (se espera ida y vuelta `encode(decode(x)) == x`). Las constantes de cable (`ED2K_BLOCK_SIZE`, `ED2K_PART_SIZE`, ordenación *little-endian*) no se cambian sin un vector de prueba de compatibilidad.
4. **Errores**: usar `FramingError`/`ProtocolError` (`thiserror`) — nunca `panic!`/`unwrap()` sobre datos de red.
5. **Característica `lab-loopback`**: el módulo `lab` está reservado para pruebas loopback; no añadir nada que pueda filtrarse a producción.
6. **API pública**: `lib.rs` reexporta ampliamente (`pub use`); cualquier renombrado de tipo público es un cambio incompatible (*breaking change*) — verificar los usos en `mule-daemon` y `mule-mcp`.

## Pruebas esperadas

- Todo nuevo mensaje/opcode: prueba de ida y vuelta codificador/decodificador + vector binario conocido si está disponible en `docs/ed2k/`.
- Todo límite: prueba de rechazo por encima del límite (se agradece *fuzzing* con `proptest`, ya presente como dev-dependency del workspace).
- `cargo test -p mule-protocol --all-features` debe pasar antes de cualquier *commit*.

## Arquitectura interna

| Módulo | Rol |
| :--- | :--- |
| `ed2k/` | Códecs eD2K: `decoder/`, `encoder/`, `opcodes.rs`, `tags.rs`, `session`, `source_exchange`, `obfuscation` |
| `kad/` | Mensajes DHT Kademlia (distancia de 128 bits `CUInt128`) |
| `framing/` | `PacketCodec` / `RawPacket` — entramado de cabecera de 5 bytes |
| `limits.rs` | Límites de seguridad y trait `PacketLimits` |
| `traits.rs` | Traits de planificación desacoplados |
| `types.rs` | Tipos de cable compartidos |

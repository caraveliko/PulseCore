# Notas de la Versión — Pulse Core v0.4.0-beta.1

**Fecha de publicación:** 19 de noviembre de 2026  
**Estado:** Versión beta — arquitectura moderna de negociación de pares 2026 (`PC26`), transporte QUIC P2P autenticado por Ed25519, criptografía post-cuántica híbrida, conectividad NAT/IPv6 y endurecimiento integral de `mule-torrent`.

---

## 1. Síntesis de la Versión

La versión **v0.4.0-beta.1** introduce el salto generacional hacia el estándar **Pulse Core 2026** (documentado en el [ADR 0005](../ed2k/decisions/0005-negociacion-pares-2026.md)): endurecimiento completo sobre la pila clásica retrocompatible con eMule 0.72a / aMule, negociación moderna `PC26` con degradación automática, transporte QUIC multipunto, cascada NAT IPv4/IPv6 y alineación de seguridad y red de `mule-torrent`.

---

## 2. Novedades y Mejoras Principales

### Capa 1 — Endurecimiento e Interoperabilidad eD2K (100 % compatible con eMule 0.72a / aMule)
- **Protección pre-handshake y cuotas por subred (`subnet_guard.rs`)**: tiempos de espera dedicados (`3 s` primer byte, `5 s` negociación de ofuscación) y límite simultáneo de conexiones entrantes por subred (`/24` en IPv4 y `/48` en IPv6, con loopback exento).
- **Constructor unificado `OP_HELLO` / `OP_EMULEINFO` (`hello_caps.rs`)**: alineación homogénea anunciando eMule `0.72a` tanto en conexiones entrantes como salientes.
- **SUI V2, caché de claves verificadas y persistencia AICH (`sui_cache.rs`, `aich_hashes.rs`)**: caché persistente en SQLite de claves RSA-384 verificadas (permitiendo solicitar `state = 1` en reconexiones), soporte completo de firma SUI V2 y almacenamiento persistente de hashsets de partes AICH (`OP_HASHSETANSWER2`).
- ***Pipelining* adaptativo por BDP, LEDBAT++ pasivo y puntuación anti-Sybil**: ajuste dinámico de ventana (`3..=36` bloques) según ancho de banda y RTT suavizado, estimador LEDBAT++ pasivo en USS (`uss_ledbat.rs`), puntuación con diversidad por subred y *backoff* con *jitter* (`peer_scoring.rs`), más política actualizada de archivos no compresibles (`compression_policy.rs`).

### Capa 2 — Perfil Moderno Pulse Core 2026 (`PC26`) y Conectividad Avanzada
- **Transporte cifrado oportunista `PC26-AEAD`**: intercambio efímero **X25519** e **híbrido post-cuántico X25519 + ML-KEM-768 (FIPS 203)** con `HKDF-SHA256` y cifrado autenticado `ChaCha20-Poly1305` iniciado por marcador no colisionante `"PC26"`, con repliegue automático a RC4 heredado.
- **Identidad Ed25519 (SUI v3), Zstd acotado y Merkle BLAKE3/SHA-256**: firma persistente Ed25519 (`pulse_identity.key`) vinculada al `transcript_hash` de la sesión, compresión Zstd acotada y verificación por árbol de Merkle de 32 bytes.
- **Conectividad NAT, Dual-Stack IPv6, QUIC P2P y Perforación UDP**:
  - Cascada de apertura de puertos **UPnP IGD → PCP (RFC 6887) → NAT-PMP (RFC 6886)** y *pinholes* IPv6 (`WANIPv6FirewallControl:1`).
  - Conexión saliente Dual-Stack con **Happy Eyeballs v2 (RFC 8305)** (`250 ms` IPv6 → IPv4) y oyente Dual-Stack IPv6/IPv4.
  - Transporte **QUIC RFC 9000** (`ALPN = "pulse-ed2k-2026"`) sobre TLS 1.3 con verificación SPKI Ed25519 sin CA externa y coordinador de perforación simultánea UDP (`PCHP`).

### Motor BitTorrent (`mule-torrent`) y Estabilidad en CI
- **Política unificada de red y filtro IP**: exportación atómica del `IpFilter` de eD2K al formato `.p2p` de `librqbit`, enlace atómico del *kill switch* (`CancellationToken`), validación anti-SSRF en `add_url`, soporte activo de **uTP (BEP 29)**, Dual-Stack IPv6, `fastresume` en `bt_session` y UPnP coordinado.
- **Estabilización determinista en pruebas CI**: corte controlado sin `TCP RST` en `LabSourceService` para `inter_daemon_transfer` y espera completa de subpaquetes `OP_SENDINGPART` en `secure_ident_tests`.

---

## 3. Verificación e Integridad

Los binarios y paquetes para **Windows (x64)**, **Linux (x64)** y **Raspberry Pi (ARM64 / aarch64)** se compilan mediante GitHub Actions acompañados de sus sumas SHA-256 y del manifiesto firmado Ed25519 (`manifest.json`) cuando la clave de firma está configurada.

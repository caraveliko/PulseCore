# ADR 0005: Negociación Moderna de Pares 2026 (Pulse Core / `PC26`)

## Estado
Aceptado

## Fecha
2026-11-19

## Contexto
La pila eD2K/Kad heredada de eMule 0.72a y la configuración inicial de `librqbit` en `mule-torrent` presentan carencias estructurales frente a las redes P2P modernas de 2026:

1. **Criptografía obsoleta en cable y SUI**: la ofuscación clásica emplea RC4-drop1024 con derivación MD5 a partir de `target_user_hash` (sin secreto perfecto hacia adelante ni integridad AEAD, e imposible de usar de forma oportunista cuando se desconoce el hash del par en Kad). La identificación segura (SUI) heredada utiliza RSA-384 con PKCS#1 v1.5 y SHA-1.
2. **Exposición pre-handshake y divergencias en `OP_HELLO`**: el receptor entrante carece de cuotas por subred y de tiempos de espera dedicados en la lectura del primer byte y del saludo de ofuscación; además, el saludo entrante anunciaba versión `0.70a` mientras que el saliente anunciaba `0.72a`.
3. **SUI sin caché ni firma V2 completa**: el cliente anuncia soporte V1+V2 (`SecIdent = 3`), pero firma siempre en V1 y solicita `state = 2` (clave + firma) en cada reconexión. Tampoco persiste los conjuntos de hashes de partes AICH recibidos mediante `OP_HASHSETANSWER2`.
4. **Planificación y control de congestión de la era ADSL**: el *pipelining* de bloques tiene techo en 9 bloques, el retroceso por fallo no distingue causa ni aplica *jitter*, no hay protección de diversidad por subred y USS depende únicamente de sondas ICMP/TCP explícitas.
5. **Barreras de conectividad de 2026 (CGNAT / IPv6)**: UPnP IGDv1 IPv4 por sí solo no cubre routers modernos con PCP/NAT-PMP ni cortafuegos IPv6, ni aprovecha transporte UDP multiplexado (QUIC) con perforación simultánea.
6. **Alineación de seguridad en `mule-torrent`**: el motor BitTorrent debe compartir exactamente el mismo filtro IP (`mule-filter`), interruptor de corte (*kill switch*) atómico y protección anti-SSRF (`mule-network-policy`) que el motor eD2K/Kad, habilitando además uTP y Dual-Stack IPv6.

## Decisión
Adoptar una arquitectura de modernización en **dos capas estrictamente separadas y retrocompatibles**:

1. **Capa 1 — Endurecimiento y optimización 100 % compatibles con eMule 0.72a / aMule**:
   - Endurecimiento pre-handshake (`PRE_HANDSHAKE_FIRST_BYTE_TIMEOUT`, `PRE_HANDSHAKE_OBFU_TIMEOUT`) y cuotas de subred (`/24` IPv4 y `/48` IPv6, eximiendo loopback).
   - Constructor unificado de `OP_HELLO` / `OP_EMULEINFO` anunciando de forma homogénea eMule `0.72a` (`72 << 10`).
   - Caché persistente en SQLite de claves públicas SUI verificadas (`sui_peer_pubkeys`) para solicitar `state = 1` en reconexiones, y soporte completo de firma SUI V2 respetando las reglas de interoperabilidad de `SUI_WIRE_FORMAT.md`.
   - Persistencia de hashsets de partes AICH (`file_aich_part_hashes`).
   - *Pipelining* adaptativo por producto ancho de banda × retardo (BDP) manteniendo los niveles clásicos para pares lentos (`<= 150 KiB/s`), estimador pasivo LEDBAT++ como complemento de USS, puntuación de pares con diversidad de subred y *backoff* diferenciado con *jitter*, y lista ampliada de extensiones incompresibles.

2. **Capa 2 — Perfil moderno Pulse Core 2026 (`PC26`) con degradación automática**:
   - Negociado únicamente mediante etiquetas TLV estándar en `OP_HELLO` (`"pc26_caps"`, `"pc26_ed25519"`, `"pc26_ipv6"`, `"pc26_quic"`) y mediante un prefijo mágico oportunista no colisionante (`"PC26"`, `0x50 0x43 0x32 0x36`).
   - Transporte cifrado AEAD `ChaCha20-Poly1305` con intercambio efímero **X25519** o **híbrido post-cuántico X25519 + ML-KEM-768 (FIPS 203)** y derivación `HKDF-SHA256`, permitiendo cifrado oportunista sin conocer previamente el `user_hash`.
   - Identidad moderna **Ed25519** (`pulse_identity.key`) para SUI v3 mediante vinculación de canal (*channel binding*) sobre el `transcript_hash` SHA-256 de la sesión.
   - Verificación de integridad mediante árbol Merkle **BLAKE3** (y equivalente **SHA-256**) y compresión **Zstd** acotada.
   - Cascada de apertura de puertos **UPnP IGD → PCP (RFC 6887) → NAT-PMP (RFC 6886)** más apertura de *pinholes* IPv6 (`WANIPv6FirewallControl:1`), **Happy Eyeballs v2 (RFC 8305)** para Dual-Stack IPv6/IPv4, transporte **QUIC (RFC 9000)** autenticado por clave pública Ed25519 P2P y coordinador de perforación simultánea UDP (*UDP Hole Punching*).
   - Integración completa de `mule-filter`, `mule-network-policy`, uTP (BEP 29), Dual-Stack IPv6, `fastresume` y protección anti-SSRF en `mule-torrent`.

3. **Pureza modular por crate**:
   - `mule-crypto`: primitivas criptográficas puras (Ed25519, X25519, ML-KEM-768, ChaCha20-Poly1305, HKDF-SHA256, Merkle BLAKE3/SHA-256), sin E/S de red.
   - `mule-protocol`: códecs y entramado puros (etiquetas TLV `pc26`, marcadores/métodos en `obfuscation/framing.rs`, códec `zstd` acotado), sin sockets ni E/S de red.
   - `mule-core`: esquema SQLite y repositorios puros (`sui_cache`, `aich_hashes`, migración `peer_sources`).
   - `mule-daemon` y `mule-torrent`: transporte, sockets, QUIC, UPnP/PCP/NAT-PMP, planificación y políticas bajo `mule-network-policy`.

## Consecuencias
- **Positivas**: interoperabilidad intacta con clientes clásicos eMule 0.72a / aMule; seguridad criptográfica moderna (PFS, AEAD, resistencia post-cuántica, Ed25519) entre nodos compatibles; mayor tasa de conexión bajo CGNAT/IPv6 y mayor rendimiento en enlaces de fibra de alta velocidad.
- **Negativas**: incremento controlado de superficie de código y dependencias pure-Rust (`blake3`, `ml-kem`, `rcgen`, `quinn`, `rustls`, `zstd`), mitigado mediante aislamiento estricto por módulo, límites duros de memoria/trama y pruebas 100 % sin conexión.

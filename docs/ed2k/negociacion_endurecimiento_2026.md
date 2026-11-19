# Endurecimiento de Negociación e Interoperabilidad eD2K (Estándar 2026)

Estado: **Normativo — Capa 1 (100 % compatible con eMule 0.72a / aMule)**

Este documento especifica las mejoras de seguridad, rendimiento y fiabilidad aplicables de forma inmediata sobre el protocolo eD2K estándar sin alterar los opcodes ni el formato de cable heredados.

---

## 1. Endurecimiento pre-handshake y cuotas por subred

Para mitigar ataques *Slowloris*, agotamiento de descriptores y ataques Sybil desde una misma subred sobre `InboundListenerService`:

1. **Tiempos de espera dedicados pre-handshake** (antes del timeout general de `OP_HELLO`):
   - `PRE_HANDSHAKE_FIRST_BYTE_TIMEOUT = 3 s`: lectura del primer byte de protocolo/ofuscación.
   - `PRE_HANDSHAKE_OBFU_TIMEOUT = 5 s`: negociación completa de ofuscación RC4 o `PC26-AEAD`.
2. **Cuotas por subred** (`SubnetQuotaGuard`):
   - IPv4: máximo `16` conexiones entrantes simultáneas por prefijo `/24`.
   - IPv6: máximo `16` conexiones entrantes simultáneas por prefijo `/48` (normalizando direcciones IPv4 mapeadas `::ffff:a.b.c.d` a `/24`).
   - Las direcciones *loopback* (`127.0.0.0/8`, `::1`) quedan exentas para no interferir con los bancos de pruebas locales.

---

## 2. Unificación del constructor `OP_HELLO` / `OP_EMULEINFO`

Ambos sentidos de conexión (entrante en `inbound_listener` y saliente en `peer_session::handshake`) deben utilizar un único constructor en `services::hello_caps`:

- `ED2K_CLIENT_VERSION = 0x3C`
- `EMULE_VERSION_TAG = 72 << 10` (anuncio homogéneo de eMule `0.72a`).
- Mismo cálculo de `CT_EMULE_MISCOPTIONS1`, `CT_EMULE_MISCOPTIONS2` y `CT_EMULE_UDPPORTS`, eliminando la divergencia histórica donde el lado entrante anunciaba `0.70a`.

---

## 3. SUI V2, caché de claves verificadas y persistencia AICH

1. **Caché de claves públicas SUI verificadas (`sui_peer_pubkeys`)**:
   - Indexada por `user_hash` (hexadecimal de 32 caracteres).
   - Cuando una firma SUI (V1 o V2) supera la verificación RSA-384, la clave DER PKCS#1 (`<= 80` bytes) y su hash MD4 se persisten con su marca temporal y última IP observada.
   - Al emitir un desafío `OP_SECIDENTSTATE` en una conexión posterior frente a un par cuyo `user_hash` figura en caché:
     - Se precarga su clave pública y se emite `state = 1` (`IS_SIGNATURENEEDED`), ahorrando el envío redundante de `OP_PUBLICKEY`.
     - Si el par no está en caché, se mantiene `state = 2` (`IS_KEYANDSIGNEEDED`).
2. **Firma SUI V2 saliente y entrante**:
   - Se mantiene el cumplimiento estricto de `docs/ed2k/SUI_WIRE_FORMAT.md`:
     - Si el par anuncia soporte V1 (`(peer_sec_ident & 1) != 0`) y no solicita explícitamente V2 exclusivo, se responde en V1 por defecto para máxima compatibilidad con clientes estrictos.
     - Si el par anuncia soporte exclusivo V2 (`(peer_sec_ident & 2) != 0 && (peer_sec_ident & 1) == 0`) o en modo preferente V2 cuando ambos extremos la soportan sin conflicto, se firma la carga útil V2 (`challenger_pubkey || challenge_u32_le || ip_u32_le || byChaIPKind`) usando `CRYPT_CIP_LOCALCLIENT = 20`, `CRYPT_CIP_REMOTECLIENT = 10` o `CRYPT_CIP_NONECLIENT = 30`.
3. **Persistencia de hashsets de partes AICH (`file_aich_part_hashes`)**:
   - Al decodificar `OP_HASHSETANSWER2` con `aich_set = Some(set)`, se valida el número de hashes de parte recibidos (cada uno de 20 bytes) y se persisten ordenados por índice en SQLite.

---

## 4. *Pipelining* adaptativo por BDP y LEDBAT++ pasivo

1. **Ventana adaptativa de peticiones de bloques (`adaptive_pending_block_count`)**:
   - Se conserva intacta la curva clásica de eMule para `peer_bps <= 150 KiB/s` (`1`, `2`, `3` o `6` bloques).
   - Para pares rápidos (`peer_bps > 150 KiB/s`), se calcula el número objetivo a partir del producto **ancho de banda × RTT suavizado (BDP)** medido por sesión mediante un seguidor EWMA (`α = 0.25`):
     $$\text{bloques} = \operatorname{clamp}\left(\left\lceil \frac{\text{peer\_bps} \cdot \text{srtt\_ms}}{1000 \cdot 184\,320} \right\rceil + 2,\ 9,\ 36\right)$$
   - Esto permite saturar enlaces de fibra modernos sin sobrecargar pares domésticos lentos.
2. **LEDBAT++ pasivo como complemento de USS (`uss_ledbat.rs`)**:
   - Mantiene una ventana deslizante de retardo base (`base_delay`) dividida en 10 cubetas de 1 minuto (mínimo observado) y estima el retardo de cola (*queuing delay*) como `current_delay - base_delay`.
   - Con un objetivo `TARGET_QUEUING_DELAY_MS = 60 ms`, modula la cuota dinámica de subida cuando el sondeo ICMP tradicional está bloqueado o no disponible.

---

## 5. Puntuación inteligente de pares, *backoff* diferenciado y compresión selectiva

1. **Retroceso exponencial diferenciado con *jitter* (`calculate_backoff_for_reason`)**:
   - `TimeoutQueue`: enfriamiento corto inicial (60 s base, tope 10 min).
   - `ConnectRefused` / `TimeoutConnect` / `TimeoutHandshake` / `TimeoutStatus`: exponencial estándar (60 s base, tope 20 min).
   - `ObfuscationNegotiationFailed` / `InvalidHandshake`: penalización media (5 min base, tope 40 min).
   - `CorruptedData` / `HashMismatch`: penalización severa (60 min base, tope 120 min).
   - Se aplica un *jitter* acotado de ±20 % para evitar sincronización de reconexiones masivas (*thundering herd*).
2. **Puntuación de fuentes y diversidad anti-Sybil (`PeerScoreTracker`)**:
   - Combina fiabilidad (tasa de éxitos frente a fallos), rendimiento observado, menor RTT, verificación criptográfica SUI/Ed25519, soporte de transporte moderno `PC26` y penalización progresiva cuando múltiples candidatos comparten el mismo prefijo `/24` IPv4 o `/48` IPv6.
3. **Política de compresión (`compression_policy.rs`)**:
   - Amplía la lista de extensiones incompresibles de eMule con formatos multimedia y archivadores modernos (`.mkv`, `.mp4`, `.m4v`, `.webm`, `.avif`, `.webp`, `.heic`, `.zst`, `.lz4`, `.xz`, `.opus`, `.flac`, etc.) para evitar desperdiciar CPU en compresión inútil.

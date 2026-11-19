# Criptografía Moderna e Identidad `PC26` (Extensión Retrocompatible)

Estado: **Normativo — Capa 2 (negociada entre pares compatibles con degradación automática a eMule 0.72a)**

Esta especificación define el perfil criptográfico moderno **Pulse Core 2026 (`PC26`)** sobre eD2K, diseñado para proporcionar secreto perfecto hacia adelante (PFS), cifrado autenticado (AEAD), resistencia híbrida post-cuántica (ML-KEM-768), cifrado oportunista sin conocer el `user_hash` y autenticación vinculada al canal mediante Ed25519.

---

## 1. Anuncio de capacidades en `OP_HELLO` / `OP_HELLO_ANSWER`

Las capacidades `PC26` se transportan mediante etiquetas TLV estándar con nombre textual (`TagName::Text`), ignoradas de forma segura por los clientes clásicos que no las reconozcan:

| Nombre de etiqueta | Tipo TLV | Carga útil | Significado |
| :--- | :--- | :--- | :--- |
| `"pc26_caps"` | `TAGTYPE_UINT32` (`0x03`) | Máscara de bits LE de 32 bits | Capacidades `PC26` admitidas por el par |
| `"pc26_ed25519"` | `TAGTYPE_BLOB` (`0x07`) | 32 bytes | Clave pública Ed25519 del par |
| `"pc26_ipv6"` | `TAGTYPE_BLOB` (`0x07`) | 18 bytes (`ip_v6[16] \|\| puerto_tcp_u16_le`) | Punto de enlace TCP IPv6 alcanzable |
| `"pc26_quic"` | `TAGTYPE_UINT16` (`0x08`) | `puerto_quic_u16_le` | Puerto UDP donde escucha transporte QUIC (`pulse-ed2k-2026`) |

### Mapa de bits de `"pc26_caps"` (`u32`)

| Bit | Constante | Significado |
| :--- | :--- | :--- |
| `0` (`0x0001`) | `PC26_CAP_X25519_CHACHA` | Transporte AEAD X25519 + HKDF-SHA256 + ChaCha20-Poly1305 |
| `1` (`0x0002`) | `PC26_CAP_HYBRID_MLKEM768` | Intercambio híbrido post-cuántico X25519 + ML-KEM-768 (FIPS 203) |
| `2` (`0x0004`) | `PC26_CAP_ED25519_SUI_V3` | Autenticación moderna SUI v3 con vinculación de canal Ed25519 |
| `3` (`0x0008`) | `PC26_CAP_ZSTD` | Compresión acotada Zstd en bloques de transferencia |
| `4` (`0x0010`) | `PC26_CAP_BLAKE3_MERKLE` | Verificación por árbol de Merkle BLAKE3 |
| `5` (`0x0020`) | `PC26_CAP_QUIC` | Transporte QUIC RFC 9000 (`ALPN = pulse-ed2k-2026`) |
| `6` (`0x0040`) | `PC26_CAP_IPV6` | Soporte Dual-Stack IPv6 y anuncio en `"pc26_ipv6"` |
| `7` (`0x0080`) | `PC26_CAP_UDP_HOLE_PUNCH` | Coordinación de perforación simultánea UDP |

---

## 2. Protocolo de transporte cifrado `PC26-AEAD`

### 2.1 Prefijo oportunista en el cable y compatibilidad RC4

A diferencia de la ofuscación clásica de eMule (que exige conocer el `target_user_hash` MD4 antes de abrir el canal), `PC26-AEAD` soporta negociación **oportunista** al inicio de la conexión TCP:

- Marcador mágico: `PC26_OPPORTUNISTIC_MAGIC = [0x50, 0x43, 0x32, 0x36]` (`"PC26"`).
- El byte `0x50` no colisiona con `PR_ED2K` (`0xE3`), `PR_EMULE` (`0xC5`) ni `PR_EMULE_COMPRESSED` (`0xD4`).
- `generate_semi_random_marker` excluye también `0x50` al iniciar conexiones RC4 salientes.
- Si una conexión entrante clásica RC4 empieza casualmente por `0x50` pero sus 3 bytes siguientes no son `"C26"`, el receptor los reinyecta como los 3 primeros bytes de `random_key_part` para completar la detección RC4 sin falsos rechazos.

### 2.2 Mensajes de acuerdo de claves

Identificadores de suite criptográfica:
- `ENM_PC26_X25519_CHACHA = 0x10`
- `ENM_PC26_HYBRID_MLKEM768_CHACHA = 0x11`

Flujo en el cable (enteros sin signo en *little-endian*):

1. **Iniciador → Respondedor (`ClientHello`)**:
   ```text
   [MAGIC "PC26" (4B)] [hello_len u16 LE] [version u8 = 1] [suite u8]
   [x25519_eph_pub (32B)] [client_nonce (16B)] [mlkem768_ek (0B o 1184B)]
   ```
2. **Respondedor → Iniciador (`ServerHello`)**:
   ```text
   [server_len u16 LE] [version u8 = 1] [suite u8]
   [x25519_eph_pub (32B)] [server_nonce (16B)] [mlkem768_ct (0B o 1088B)]
   ```
   - Si el iniciador solicita `0x11` (híbrido) pero el respondedor tiene deshabilitada la negociación post-cuántica (`allow_pq = false`), el respondedor degrada limpiamente seleccionando `suite = 0x10` en `ServerHello` y omite `mlkem768_ct`.

### 2.3 Derivación de claves (`HKDF-SHA256`)

- Secreto compartido clásico: `ss_x25519 = X25519(priv_local, pub_remota)` (32 bytes; se rechaza cualquier secreto nulo).
- Si la suite negociada es `0x11`: `ss_pq = ML-KEM-768` (32 bytes); `ikm = ss_x25519 || ss_pq` (64 bytes). Si es `0x10`: `ikm = ss_x25519` (32 bytes).
- Huella del saludo: `transcript_hash = SHA256(ClientHello_body || ServerHello_body)`.
- Extracción y expansión con `HKDF-SHA256`:
  - `prk = HKDF-Extract(salt = transcript_hash, ikm = ikm)`
  - `key_init_to_resp = HKDF-Expand(prk, info = "pc26-init-to-resp-v1", len = 32)`
  - `key_resp_to_init = HKDF-Expand(prk, info = "pc26-resp-to-init-v1", len = 32)`

### 2.4 Entramado de registros AEAD (`ChaCha20-Poly1305`)

Una vez completado `ServerHello`, todo el flujo TCP se divide en registros cifrados acotados:

```text
[plaintext_len u16 LE (1..=16384)] [ciphertext (plaintext_len bytes)] [poly1305_tag (16B)]
```

- **Nonce de 96 bits**: `0x00000000 || seq_u64_le`, incrementado estrictamente en `1` por cada registro y sentido (sin reutilización posible; error fatal si alcanza `u64::MAX`).
- **Datos asociados (AAD)**: `version_u8 || suite_u8 || direction_u8 || plaintext_len_u16_le`.

---

## 3. Identidad moderna Ed25519 y SUI v3 (*Channel Binding*)

- El nodo genera y persiste una clave PKCS#8 Ed25519 en `pulse_identity.key` (texto Base64, escritura atómica y cuarentena en caso de corrupción, análoga a `cryptkey.dat`).
- **Vinculación de canal (SUI v3)**:
  - Carga útil firmada: `"pulsecore-sui-v3:" || role_tag || transcript_hash[32]`, donde `role_tag` es `0x01` (firma del iniciador) o `0x02` (firma del respondedor).
  - La firma Ed25519 de 64 bytes demuestra posesión de la clave pública anunciada en `"pc26_ed25519"` vinculada al intercambio criptográfico exacto de la sesión actual, impidiendo ataques *MitM* y *relay* incluso cuando la IP del par cambia bajo CGNAT.

---

## 4. Árbol de Merkle BLAKE3 / SHA-256 y compresión Zstd

- **Árbol binario de Merkle (`merkle_blake3.rs`)**:
  - Hojas con separación de dominio (`0x00 || block_bytes`) y nodos internos (`0x01 || left || right`), siguiendo las mejores prácticas contra ataques de segundo preimagen.
  - Tamaño de bloque por defecto: `184_320` bytes (180 KiB, alineado con subbloques AICH) y soporte explícito para `16_384` bytes (alineado con BEP 52).
- **Compresión Zstd acotada (`zstd_codec.rs`)**:
  - Compresión de nivel configurable (por defecto nivel 3) y descompresión estrictamente acotada por `max_out` bytes (rechazo inmediato de tramas que superen el límite para prevenir bombas de descompresión).

---

## 5. Vectores de prueba normativos

Los vectores deterministas de verificación cubren:
1. Ronda de etiquetas TLV `"pc26_caps"`, `"pc26_ed25519"`, `"pc26_ipv6"` y `"pc26_quic"` dentro de `OP_HELLO` y `OP_HELLO_ANSWER` (tolerados como etiquetas TLV estándar).
2. Derivación `HKDF-SHA256` sobre `transcript_hash`, cifrado/descifrado `ChaCha20-Poly1305` y rechazo ante manipulación de 1 bit en AAD, longitud o etiqueta Poly1305.
3. Acuerdo híbrido `X25519 + ML-KEM-768` y degradación `0x11 → 0x10` cuando `allow_pq = false`.
4. Firma/verificación Ed25519 SUI v3 ligada a `transcript_hash` y rechazo ante `role_tag` o `transcript_hash` distinto.
5. Construcción y prueba de inclusión `Blake3MerkleTree` y `Sha256MerkleTree` para archivos vacíos, de 1 bloque, potencia de 2 y número impar de bloques.

# Identificación Segura de Usuario (SUI) — Formato Wire y Especificación de Compatibilidad

Estado: **Fase A1 implementada** — SUI bidireccional completo: verificación RSA real entrante, respuesta a desafíos saliente, **y** emisión de desafío saliente (rol verificador) cuando el par anuncia `SecIdent` en MiscOptions1. Capacidad `m_bySupportSecIdent` negociada en ambos sentidos. Vectores binarios Tier 3 aún por producir. Véase §6 para el estado de implementación.

## 1. Fuentes de referencia (fijadas)

| Fuente | Archivo | Contenido utilizado |
|---|---|---|
| eMule oficial (repo `irwir/eMule`, master @ 2026-09) | `opcodes.h` | Opcodes, `RSAKEYSIZE` |
| eMule oficial | `ClientCredits.cpp` / `ClientCredits.h` | `CreateSignature`, `VerifyIdent`, `cryptkey.dat`, `CreditStruct` |
| eMule oficial | `BaseClient.cpp` | `SendPublicKeyPacket`, `SendSignaturePacket`, `ProcessPublicKeyPacket`, `ProcessSignaturePacket`, `SendSecIdentStatePacket`, `ProcessSecIdentStatePacket` |
| aMule (repo `amule-project/amule`, master) | `src/ClientCreditsList.cpp` | Confirmación cruzada: esquema Crypto++ idéntico |

## 2. Hallazgos confirmados

### Hallazgo: esquema de firma — **RSA PKCS#1 v1.5 + SHA-1**, no MD4 ni PSS

**Estado:** Confirmado (eMule + aMule concordantes)

- `m_pSignkey = new CryptoPP::RSASSA_PKCS1v15_SHA_Signer(filesource)` (eMule 0.72a `ClientCredits.cpp:404`, aMule `ClientCreditsList.cpp:InitalizeCrypting`)
- `CryptoPP::RSASSA_PKCS1v15_SHA_Verifier` para la verificación.
- Tamaño de clave: `RSAKEYSIZE = 384` bits (eMule 0.72a `opcodes.h:96`, aMule `Constants.h:49`).
- Longitud de firma = longitud del módulo = **48 bytes** para RSA-384. La validación de tamaño debe derivarse del módulo del par (`modulus_len_bytes`), nunca de un rango fijo `48..=96`.

**Implicación eMuleRust:** el crate `rsa` de RustCrypto soporta `Pkcs1v15Sign` + `Sha1`. ✅ Spike validado: `rsa 0.9.10` genera claves de 384 bits (`RsaPrivateKey::new`) y acepta `from_components`; `Pkcs1v15Sign::new::<Sha1>()` exige la entrada **pre-hasheada** (20 bytes) — véase `mule-crypto::legacy_sui`.

### Hallazgo: contenido firmado — la clave pública del *challenger* precede al desafío

**Estado:** Confirmado

Al contrario de la hipótesis "desafío + IP", el búfer firmado es (eMule 0.72a `ClientCredits.cpp:447` `CClientCreditsList::CreateSignature` / aMule `ClientCreditsList.cpp:331-375`):

```
V1 :  pubkey_challenger[keylen] || challenge u32 LE            → keylen + 4 bytes
V2 :  pubkey_challenger[keylen] || challenge u32 LE
      || challenge_ip u32 LE || byChaIPKind u8                 → keylen + 9 bytes
```

- `pubkey_challenger` = clave pública wire del par que emitió el desafío (lado firmante: `pTarget->GetSecureIdent()`; lado verificador: `m_abyMyPublicKey`).
- `challenge` = `m_dwCryptRndChallengeFor` emitido por el challenger en `OP_SECIDENTSTATE` (recibido como `m_dwCryptRndChallengeFrom` por el firmante).
- `challenge_ip` / `byChaIPKind`: véase §5.

**Implicación:** `encode_emule_signed_payload` debe concatenar `pubkey || challenge || [ip || kind]`, en ese orden, LE para los enteros.

### Hallazgo: formato `cryptkey.dat` — DER + Base64, no un blob binario

**Estado:** Confirmado (eMule 0.72a `ClientCredits.cpp:422` `CClientCreditsList::CreateKeyPair`, aMule `ClientCreditsList.cpp:CreateKeyPair`)

- Generación: `InvertibleRSAFunction::Initialize(rng, 384)` y luego `privkey.DEREncode()` en un `Base64Encoder` → **el archivo es texto Base64 que contiene el DER Crypto++ de la clave privada** (`RSASSA_PKCS1v15_SHA_Signer` = `InvertibleRSAFunction` serializada: n, e, d, p, q, dp, dq, u en BER).
- Lectura: `FileSource(..., true, new Base64Decoder)` → decodifica Base64 y luego parsea BER/DER.
- Clave pública wire: `pubkey.DEREncode(asink)` en un búfer de 80 bytes = **estructura BER `SEQUENCE { n INTEGER, e INTEGER }`** (CryptoPP `GetMaterial().Save` produce la misma codificación). Esta forma DER (≤ 80 bytes) es la que circula en `OP_PUBLICKEY` y se hashea para la identidad de crédito.
- Comportamiento archivo vacío → regeneración; ausente → `CreateKeyPair`. La política eMuleRust (cuarentena en lugar de sobrescritura) sigue siendo más segura y se conserva.

### Hallazgo: `OP_SECIDENTSTATE` — semántica del byte `state`

**Estado:** Confirmado (eMule 0.72a `BaseClient.cpp:2076-2143` `SendSecIdentStatePacket`/`ProcessSecIdentStatePacket`)

Wire: `<state u8><challenge u32 LE>` — **nSize debe ser exactamente 5**, si no rechazo silencioso.

| `state` | Enum interno | Significado para el *receptor* del paquete |
|---|---|---|
| 0 | `IS_UNAVAILABLE` | El challenger no soporta SUI — abandonar |
| 1 | `IS_SIGNATURENEEDED` | El challenger ya tiene nuestra clave pública: enviar `OP_SIGNATURE` solo |
| 2 | `IS_KEYANDSIGNEEDED` | El challenger no tiene nuestra clave: enviar `OP_PUBLICKEY` y luego `OP_SIGNATURE` |

- Emitido solo si `CryptoAvailable()` y (`SecIDKeyLen == 0` → 2, o `m_dwLastSignatureIP != GetIP()` → 1). Si `nValue == 0`, **no se envía ningún paquete** (sin state 0 explícito salvo cripto no disponible en el lado challenger → de hecho 0 nunca se emite en esta ruta; `m_SecureIdentState = IS_UNAVAILABLE` es también el valor inicial).
- Desafío generado por `rand()+1` en eMule — **no criptográficamente seguro**; eMuleRust debe usar `OsRng` manteniéndose compatible (es solo un u32 para releer).

### Hallazgo: `OP_SIGNATURE` V2 — byte `byChaIPKind` al final

**Estado:** Confirmado (eMule 0.72a `BaseClient.cpp:1949-1980` envío, `:2016-2035` verificación incluyendo `:2028` para la forma V2)

```
OP_SIGNATURE : <siglen u8><signature[siglen]>            ← V1 : pBuffer[0] == nSize-1
OP_SIGNATURE : <siglen u8><signature[siglen]><kind u8>   ← V2 : pBuffer[0] == nSize-2
```

- V2 solo se acepta si `(m_bySupportSecIdent & 2) != 0` (capacidad anunciada en los info-packets; el valor del tag = 3 cuando `CryptoAvailable()`, bits: `&1` = v1, `&2` = v2).
- V1 es el valor por defecto; V2 solo se usa si el par no soporta V1 (`(m_bySupportSecIdent & 1) == 1 → bUseV2 = false`).
- `byChaIPKind`: `CRYPT_CIP_REMOTECLIENT=10` (IP del par utilizada — caso LowID), `CRYPT_CIP_LOCALCLIENT=20` (nuestro client ID HighID), `CRYPT_CIP_NONECLIENT=30` (IP=0).

### Hallazgo: vinculación IP (`useIP`)

**Estado:** Confirmado (eMule `ClientCredits.cpp:VerifyIdent`)

- `CRYPT_CIP_LOCALCLIENT`: `ChallengeIP = dwForIP` = IP del firmante tal como la observa el verificador.
- `CRYPT_CIP_REMOTECLIENT`: `ChallengeIP =` IP pública del verificador (alternativa `GetED2KID()` si LowID sin IP pública conocida).
- Tras la verificación: `Verified(dwForIP)` almacena `m_dwIdentIP = dwForIP`; un par identificado que cambia de IP pasa a `IS_IDBADGUY` (créditos congelados) hasta reidentificación.
- Antiflood: **una sola firma aceptada por IP** (`m_dwLastSignatureIP == GetIP()` → descartar).

### Hallazgo: indexación de créditos

**Estado:** Confirmado — corrección de una hipótesis anterior

- `GetCredit(m_achUserHash)`: la tabla `clients.met` está indexada por el **user_hash MD4** del par (16 bytes), no por el hash de la clave pública.
- La clave pública verificada se almacena *dentro* del `CreditStruct` (`abySecureIdent[80]` + `nKeySize`): la primera verificación fija la clave; `SetSecureIdent` rechaza cualquier cambio de clave una vez `nKeySize != 0`.
- Efecto de seguridad: si un par ya acreditado presenta una nueva clave, `VerifyIdent` fallará (clave fijada ≠ firma) → `IS_IDFAILED`, y en una nueva fijación, los créditos anteriores se reinician a 1/1 (`Verified()`).

## 3. Error de interoperabilidad corregido

~~`crates/mule-protocol/src/ed2k/opcodes.rs` invierte los dos opcodes oficiales~~ — **corregido**: `OP_SIGNATURE = 0x86`, `OP_SECIDENTSTATE = 0x87`, alineados con `opcodes.h:259-260` (eMule 0.72a). Constantes, comentarios codificador/decodificador y pruebas roundtrip actualizados (46/46 en verde).

## 4. Formato de los mensajes wire (confirmado)

```
OP_PUBLICKEY     0x85 : <keylen u8><pubkey_der[keylen]>         (keylen ≤ 80, nSize ∈ [10..250])
OP_SECIDENTSTATE 0x87 : <state u8><challenge u32 LE>            (nSize == 5 estrictamente)
OP_SIGNATURE     0x86 : <siglen u8><sig[siglen]>[<kind u8>]     (nSize ∈ [10..250])
```

## 5. Estado de implementación (A1)

### `mule-crypto` (nuevo crate)

| Módulo | Rol |
|---|---|
| `legacy_sui.rs` | `build_sui_signing_payload` (V1/V2), `sign_sui_payload`, `verify_sui_signature` (RSASSA-PKCS1v15-SHA1, tamaño derivado del módulo), `parse_peer_public_key` (PKCS#1 DER ≤ 80 B) |
| `cryptkey.rs` | `cryptkey.dat`: Base64→DER PKCS#1→RSA-384; carga `Loaded/Missing/Invalid/LegacyBlob`, cuarentena con marca temporal, escritura atómica (close antes de rename, Windows-safe) |

#### Migración del blob legacy de 80 bytes

Las versiones anteriores del daemon escribían un `cryptkey.dat` de **80 bytes binarios aleatorios** (ninguna clave RSA recuperable). `load_cryptkey` lo distingue de una corrupción real: exactamente 80 bytes **y** presencia de bytes fuera del alfabeto Base64 → `LegacyKeyLoad::LegacyBlob`. El archivo se pone en cuarentena igualmente y luego `IdentityService` **genera una nueva clave RSA-384** con un `WARN` de migración explícito — el antiguo blob no era una identidad real, nada que preservar. Un archivo de 80 bytes *texto* (alfabeto Base64 únicamente) sigue siendo `Invalid` clásico (SUI desactivado, sin regeneración). Caso real verificado: actualización por sobrescritura conservando `data/`.

### Inbound (`mule-daemon::inbound_listener::dispatcher`)

- `OP_PUBLICKEY`: clave ≤ 80 B almacenada; desafío `u32` emitido **únicamente si la identidad local es utilizable**; ningún crédito antes de verificar.
- `OP_SECIDENTSTATE`: **rol firmante entrante** — el par entrante también puede *desafiarnos*. Respuesta = nuestra `OP_PUBLICKEY` si `state=2`, y luego `OP_SIGNATURE` V1 mediante `IdentityService` (inyectado por `set_identity`). Si el desafío llega **antes** que la `OP_PUBLICKEY` del par, se deja en espera (`pending_peer_challenge`) y se trata en cuanto se recibe la clave — el orden de paquetes no está garantizado en eMule (observado en la red real: varios pares envían el desafío primero).
- `OP_SIGNATURE`: verificación RSA real — desafío consumido en todos los casos (antirrepetición), antiflood una firma/IP (`last_sig_ip`), byte `kind` V2 extraído (`10`→IP local, `20`→IP del par, `30`→0, otro→rechazo), payload V1/V2 reconstruido y luego `verify_sui_signature`.
- Éxito → clave fijada en el registro de crédito + `credit_multiplier`; fallo → `SecIdentState(0)` + estado purgado, `WARN` con motivo.

### Outbound (`mule-daemon::peer_session::sui::OutboundSui`)

- **Negociación**: `SecIdent=3` anunciado en MiscOptions1 bits 16-19 del HELLO saliente y del HELLO_ANSWER entrante (0 si `cryptkey.dat` ausente/en cuarentena), conforme a `BaseClient.cpp:1010` (`CryptoAvailable() ? 3 : 0`). La capacidad del par se expone mediante `Ed2kPeerInfo::secure_ident_support()`.
- **Firmante**: conectado en los bucles status/queue/transfer — memoriza `OP_PUBLICKEY` del par, responde a `OP_SECIDENTSTATE` (`state=2` → nuestra `OP_PUBLICKEY` primero, luego `OP_SIGNATURE` V1 mediante `IdentityService::sign_sui_payload`). Una sola respuesta por conexión. **Orden de paquetes tolerado**: un desafío recibido antes que la `OP_PUBLICKEY` del par se difiere (`pending_challenge`) y se trata al recibir la clave — error de interoperabilidad corregido tras observar clientes eMule reales emitiendo `state=2` inmediatamente tras el handshake.
- **Verificador**: tras el handshake, si el par ha anunciado `SecIdent != 0` y tenemos clave, emitimos `OP_SECIDENTSTATE(state=2, challenge)` (eMule `InfoPacketsReceived`/`SendSecIdentStatePacket`). `OP_SIGNATURE` recibido → payload `our_pubkey || challenge [|| ip || kind]` verificado con la clave del par, kind V2 gestionado (`20`→IP del par, `10`→IP local, `30`→0). Resultado registrado `INFO`/`WARN`.

### Observabilidad (`SuiStats`)

Contadores atómicos compartidos (`mule-daemon::services::identity::SuiStats`), incrementados inbound y outbound, expuestos en solo lectura mediante RPC `identity.status` bajo el objeto `sui`: `challengesIssuedIn`, `challengesIssuedOut`, `signaturesSent`, `signaturesVerified`, `signaturesRejected`, `signaturesRejectedFlood`, `malformedPackets`. No se expone ningún dato privado (clave, desafío).

### Pruebas

- `test_inbound_secure_ident_verified_with_real_rsa`: RSA-384 real → crédito creado + bytes subidos atribuidos.
- `test_outbound_secure_ident_answers_peer_challenge`: mock eMule verifica criptográficamente nuestra firma.
- `test_outbound_secure_ident_challenges_peer_when_advertised`: el par anuncia `SecIdent=3` → el daemon emite `SecIdentState(state=2)` no solicitado.
- Antiguas pruebas con firmas ficticias: siempre rechazadas (verificación real).

### Reservas Tier 3 (a confirmar frente a eMule/aMule real)

- ✅ **Desafío antes de `OP_PUBLICKEY` (orden de paquetes)**: confirmado en red real — varios pares eMule emiten `OP_SECIDENTSTATE(state=2)` inmediatamente tras el handshake, antes de su clave. El firmante debe diferir el desafío hasta recibir la clave (implementado inbound + outbound).
- ✅ **Formato `cryptkey.dat` legacy eMuleRust**: confirmado — el antiguo archivo de 80 bytes binarios no es Base64/DER; detectado como `LegacyBlob` y migrado (cuarentena + nueva clave).

- Endianness `ip_u32` V2 (`from_be_bytes` + LE actualmente).
- `kind=10` usa la IP del socket local; eMule usa la IP pública — LowID/CGNAT sin validar.
- Endianness exacta de `dwIP` en `VerifyIdent` (posible convención eD2K invertida).
- `cryptkey.dat` escrito por `rsa` es PKCS#1 estándar; concordancia byte a byte con Crypto++ `DEREncode` (orden de componentes, versión `0`/`1` two-prime) por confirmar mediante fixture real.

## 6. Pendiente / por completar en A0

- [ ] Vectores hexadecimales: captura Tier 3 eMule 0.72a (V1 + V2, HighID y LowID).
- [ ] Fixture `cryptkey.dat` real producida por eMule, parseada byte a byte (orden exacto de los componentes BER en `InvertibleRSAFunction::DEREncode`).
- [x] Tag de capacidad `m_bySupportSecIdent`: MiscOptions1 bits 16-19, valor `3` (v1|v2) — anunciado y parseado (`Ed2kPeerInfo::secure_ident_support`). Pendiente: aceptar firmas V2 entrantes solo si el par ha anunciado `& 2` (BaseClient.cpp:2028).
- [ ] Comportamiento `ProcessSecIdentStatePacket` para `state > 2` (no gestionado → desafío leído igualmente) — por decidir.
- [ ] Fuzzing de las cotas `nSize < 10 || nSize > 250` observadas en el lado eMule.

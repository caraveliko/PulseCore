# Resumen de sesión — Cifrado e identidad criptográfica eMuleRust

Fecha: 2026-09-19
Alcance: auditoría del código existente, especificación wire SUI derivada de las fuentes eMule/aMule, correcciones aplicadas, arquitectura objetivo.

> **Nota (actualización)**: la sección 1 describe el estado *anterior* a la implementación. Desde entonces, el SUI está implementado por completo — clave RSA-384 real en `cryptkey.dat` (con migración automática del blob legacy de 80 bytes mediante `LegacyKeyLoad::LegacyBlob`), firma/verificación reales inbound y outbound, desafíos diferidos cuando `OP_SECIDENTSTATE` llega antes que `OP_PUBLICKEY`, y contadores `SuiStats` expuestos mediante `identity.status`. Estado actual: `SUI_WIRE_FORMAT.md` §5.

---

## 1. Estado actual del código (auditoría)

### 1.1 `IdentityService` — identidad ficticia

Archivo: `crates/mule-daemon/src/services/identity.rs`

- `cryptkey.dat` actual = **80 bytes aleatorios** (CSPRNG), magic bytes `0x01 0x00`. Ningún par RSA real.
- El mismo valor sirve de "clave pública" (`public_key_bytes()`) y de base para el `user_hash` (MD4 + magic bytes eMule `out[5]=14`, `out[14]=111`).
- `status()` anuncia `key_bits: 384` — cosmético, no existe ninguna operación RSA.
- Instanciado en `AppState` (`state.identity`), utilizado por `peer_session`, `server_session`, `inbound_listener` para el `user_hash`.
- **Ninguna firma saliente**: `public_key_bytes()` no se llama en ningún sitio. El daemon nunca se autentica.

### 1.2 Secure-ident entrante — simulación

Archivo: `crates/mule-daemon/src/services/inbound_listener/dispatcher.rs` (líneas ~56-135)

- `OP_PUBLICKEY` recibido → MD4 de la clave, búsqueda `get_peer_credit`, envío `SecIdentState(state=1, challenge=rand u32)`.
- `OP_SIGNATURE` recibido → **"verificación" = comprobación de longitud `(48..=96)` únicamente**. Ninguna criptografía. Un blob aleatorio del tamaño correcto autentica al par y desbloquea `credit_multiplier`.
- Éxito → `SecIdentState(state=2)` + multiplicador aplicado a la cola de subida.

### 1.3 Capa de protocolo — correcta

`crates/mule-protocol/src/ed2k/`: codificadores/decodificadores puros para `OP_PUBLICKEY`, `OP_SIGNATURE`, `OP_SECIDENTSTATE` (`encoder/security.rs`, `decoder/security.rs`), tipos `Ed2kPublicKey`/`Ed2kSignature`/`Ed2kSecIdentState`, integrados en `Ed2kMessage` y la sesión. Sin criptografía en los codecs — buena separación.

### 1.4 Error corregido en esta sesión

**Opcodes invertidos** en `crates/mule-protocol/src/ed2k/opcodes.rs`:

| Opcode | eMule oficial (`opcodes.h`) | eMuleRust (antes → después) |
|---|---|---|
| `OP_PUBLICKEY` | `0x85` | `0x85` ✓ |
| `OP_SIGNATURE` | **`0x86`** | `0x87` → `0x86` ✓ |
| `OP_SECIDENTSTATE` | **`0x87`** | `0x86` → `0x87` ✓ |

Corrección aplicada: constantes + doc comments (`opcodes.rs`, `packet.rs`, `encoder.rs`, `encoder/security.rs`, `decoder.rs`, `decoder/security.rs`, comentarios de prueba). Suite `ed2k_roundtrip_tests`: **46/46 pruebas en verde**.

---

## 2. Especificación wire SUI — confirmada desde las fuentes

Referencia completa: `docs/ed2k/SUI_WIRE_FORMAT.md`. Fuentes: repo `irwir/eMule` (`opcodes.h`, `ClientCredits.cpp`, `BaseClient.cpp`) y `amule-project/amule` (`ClientCreditsList.cpp`) — ambas implementaciones concuerdan.

### 2.1 Esquema criptográfico

- **RSA-384 bits** (`RSAKEYSIZE = 384`), **PKCS#1 v1.5 + SHA-1** — `CryptoPP::RSASSA_PKCS1v15_SHA_Signer`/`Verifier`. Ni MD4, ni PSS, ni RSA bruto.
- Firma = longitud del módulo: **48 bytes** para RSA-384. La validación de tamaño debe derivar del módulo del par recibido, no de un rango fijo.

### 2.2 `cryptkey.dat` real

- Clave privada `InvertibleRSAFunction` serializada en **DER y luego codificada en Base64** — archivo de texto, no binario.
- Contenido DER: componentes `n, e, d, p, q, dp, dq, u` (secuencia BER).
- Clave pública wire = `pubkey.DEREncode()` en ≤ 80 bytes = `SEQUENCE { n INTEGER, e INTEGER }`. Este blob DER es el que circula en `OP_PUBLICKEY` y el que se fija en los créditos.
- Política eMuleRust: archivo corrupto → **cuarentena con marca temporal**, nunca sobrescritura silenciosa (pérdida de créditos históricos).

### 2.3 Contenido firmado

```
V1 : pubkey_challenger[keylen] || challenge u32 LE                          → keylen + 4
V2 : pubkey_challenger[keylen] || challenge u32 LE || ip u32 LE || kind u8  → keylen + 9
```

- El firmante firma **la clave pública del challenger** (vinculación bilateral), seguida del desafío recibido.
- `challenge`: u32, emitido por el challenger en `OP_SECIDENTSTATE` (`m_dwCryptRndChallengeFor` en el lado emisor, `From` en el lado receptor). eMule utiliza `rand()+1` (no criptográfico) — eMuleRust usará `OsRng`, compatible.

### 2.4 Mensajes wire

```
OP_PUBLICKEY     0x85 : <keylen u8><pubkey_der[keylen]>          nSize ∈ [10..250]
OP_SECIDENTSTATE 0x87 : <state u8><challenge u32 LE>             nSize == 5 estricto
OP_SIGNATURE     0x86 : <siglen u8><sig[siglen]>                 V1 : siglen == nSize-1
                        <siglen u8><sig[siglen]><kind u8>        V2 : siglen == nSize-2
```

- `state`: 0=no disponible, 1=`IS_SIGNATURENEEDED` (clave ya conocida → firma sola), 2=`IS_KEYANDSIGNEEDED` (enviar pubkey y luego firma).
- `kind` (`byChaIPKind`): `10=CRYPT_CIP_REMOTECLIENT` (IP del par, caso LowID), `20=CRYPT_CIP_LOCALCLIENT` (nuestro client ID), `30=CRYPT_CIP_NONECLIENT` (IP=0).
- V2 utilizado únicamente si el par no anuncia V1 (`m_bySupportSecIdent & 1`). Capacidad anunciada = valor 3 en los info-packets.

### 2.5 Vinculación IP y antiabuso

- `LOCALCLIENT`: `ChallengeIP` = IP del firmante vista por el verificador; `REMOTECLIENT`: IP pública del verificador (alternativa `ED2KID` si LowID).
- `Verified(dwForIP)` fija `m_dwIdentIP`: un par identificado que cambia de IP → `IS_IDBADGUY` hasta reverificación.
- Antiflood: una sola firma aceptada por IP (`m_dwLastSignatureIP`).

### 2.6 Créditos

- `clients.met` indexado por **user_hash MD4** del par (`GetCredit(m_achUserHash)`).
- La clave pública verificada se almacena en `CreditStruct.abySecureIdent[80]` + `nKeySize` — fijación en la primera verificación, `SetSecureIdent` rechaza cualquier cambio posterior.
- Nueva fijación sobre un par ya acreditado → reinicio a 1/1 de los contadores (`Verified()`).

---

## 3. Arquitectura objetivo

### 3.1 Principio rector

Dos criptografías separadas, nunca las mismas claves:

| Necesidad | Mecanismo | Rol |
|---|---|---|
| Compatibilidad eMule/aMule | **RSA-384 + PKCS1v15/SHA-1** (legacy SUI) | Demostrar la posesión del `user_hash`, preservar los créditos |
| Identidad moderna eMuleRust | **Ed25519** | Firmas de las extensiones propietarias |
| Intercambio de claves / canales | **X25519 + HKDF + ChaCha20-Poly1305** | Sesiones efímeras cifradas |
| Integridad moderna | BLAKE3 / SHA-256 | Complemento a MD4/AICH legacy |

RSA-384 es criptográficamente débil: **solo** sirve para la interoperabilidad SUI, nunca para el cifrado de archivos/mensajes ni para la protección de las claves modernas.

### 3.2 Reparto por crate

```
mule-crypto/                  # nuevo: primitivas únicamente
  legacy_sui.rs               # RSA-384 eMule: claves, sign, verify
  modern_identity.rs          # Ed25519 (clave separada, sin conversión Ed→X)
  kdf.rs                      # HKDF, separación de dominios

mule-format/
  cryptkey_dat.rs             # parsea/escribe Base64+DER eMule, cuarentena

mule-protocol/
  ed2k/secure_ident.rs        # opcodes (corregidos), payloads V1/V2, state wire

mule-daemon/
  services/identity.rs        # IdentityBundle { legacy_sui, ed25519, user_hash }
  services/peer_session + inbound_listener
                            # máquina de estados SUI compartida
```

Reglas: los decodificadores permanecen puros (solo formato); `mule-crypto` es la única frontera criptográfica; el despachador decide el "cuándo".

### 3.3 Máquina de estados SUI (compartida inbound/outbound)

`SecureIdentState`: `Unsupported / Idle / AwaitingPeerPublicKey / ChallengeSent / ChallengeReceived / AwaitingSignature / Verified / Failed`. Dos niveles estrictos: byte wire (`state u8 + challenge u32`) ≠ semántica de negocio — `interpret_secure_ident_state()` hace la traducción, `todo!()` mientras la especificación no esté validada.

### 3.4 Política IP

`SecureIdentIpBinding`: `Required / DisabledForCgnat / DisabledForUnknownAddress / CompatibilityFallback` — decisión mediante `mule-network-policy` (`is_cgnat_ip` existente), sin comparación ingenua de direcciones en el código SUI.

### 3.5 Canal moderno (Ed25519)

Nunca sobrecarga silenciosa de `OP_SIGNATURE`: negociación de capacidad eMuleRust explícita + subprotocolo versionado separado. Los pares de claves Ed25519 y X25519 son **distintos** (la conversión directa de semilla es incorrecta).

---

## 4. Contexto: enrutamiento en cebolla (análisis anterior)

Objetivo más amplio: cifrar las transmisiones y anonimizar mediante enrutamiento multisalto inspirado en Tor/I2P.

**Retenido**:
- KAD permanece en claro para el descubrimiento; la cebolla se aplica al transporte.
- Doble segmentación: partes de 9,28 MB lógicas / chunks de red (≤ 64 KB), relleno a tamaño fijo.
- Streaming mediante `AsyncRead`/`AsyncWrite` (Tokio) — base existente en `ObfuscatedStream`.

**Correcciones aportadas al análisis inicial**:
1. Nonce ChaCha20-Poly1305: nunca un nonce constante por clave (contador o nonce aleatorio por paquete).
2. Sin conversión ingenua Ed25519→X25519: dos pares de claves separados.
3. Los relés deben ser clientes eMuleRust — los 280 contactos KAD (eMule oficiales) ignorarán cualquier opcode propietario. Problema de descubrimiento de capacidades = prerrequisito mayor sin resolver.
4. Cebolla UDP adaptada a las peticiones KAD/chat (< MTU); transferencia de bloques = túneles TCP multisalto.
5. Arti/Tor descartado para la transferencia de archivos (saturación, celdas 512 B, exit nodes bloqueando el P2P).

---

## 5. Hitos v0.1.9

| Fase | Nombre | Entregable | Cripto activa |
|---|---|---|---|
| A0 | SUI Wire Specification | `docs/ed2k/SUI_WIRE_FORMAT.md` ✓ + fixtures + capturas Tier 3 | No |
| A1 | SUI Legacy Implementation | parser `cryptkey.dat`, sign/verify real, máquina de estados, créditos | Sí, tras A0 |
| A2 | SUI Interop Qualification | Tier 1/2/3 eMule 0.50a + aMule bidireccional | Validada |

**Cerrojo de seguridad (antes de A1)**: reemplazar `(48..=96).contains(&sig.len())` por un fallo explícito — ningún `Verified`, ningún `credit_multiplier` sin verificación RSA real.

**Pendiente por hacer en A0**: vectores hex Tier 3, fixture `cryptkey.dat` real parseada byte a byte, documentación exacta del tag `m_bySupportSecIdent`, comportamiento `state > 2`, cotas `nSize` eMule.

**Spike técnico previo a A1**: verificar que el crate `rsa` acepta RSA-384 (generación y `from_components`) — si no, implementación específica estrecha en `mule-crypto::legacy_sui` (operación histórica únicamente, ninguna API RSA genérica).

---

## 6. Reglas defensivas

- Desafío CSPRNG (`OsRng`), consumido una sola vez, expiración corta (2-5 min), intentos limitados por par/sesión.
- Ninguna reserva dictada por una longitud de red no acotada; sin `unwrap` en ruta de red; fuzzing de los decodificadores.
- Nunca clave privada en SQLite, en registros o mediante RPC/MCP — huellas truncadas únicamente.
- Escritura `cryptkey.dat` atómica (temp + `sync_all` + rename), permisos de usuario restrictivos.
- `mule-network-policy` consultada antes de cualquier respuesta — ni oráculo ni amplificador.

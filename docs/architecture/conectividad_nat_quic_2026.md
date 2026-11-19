# Conectividad, NAT Traversal, IPv6, QUIC y Hole Punching (Estándar 2026)

Estado: **Especificación de arquitectura implementada en `mule-daemon`**

Este documento detalla la pila de conectividad moderna diseñada para redes domésticas de 2026 con CGNAT, routers sin UPnP clásico y despliegue Dual-Stack IPv6.

---

## 1. Cascada de apertura de puertos: UPnP IGD → PCP → NAT-PMP e IPv6 Pinhole

El servicio `UpnpService` (`crates/mule-daemon/src/services/upnp`) amplía el mapeo de puertos con una cascada automática:

1. **UPnP IGD (IPv4)**: mantiene la inspección segura e idempotente (`ExactMatch`, `StaleLease`, `ForeignConflict`) y detección de CGNAT (`100.64.0.0/10`, RFC 6598).
2. **PCP (Port Control Protocol, RFC 6887) sobre UDP `:5351`**:
   - Si UPnP IGD falla o el router no responde SSDP, se consulta la puerta de enlace LAN predeterminada mediante paquetes PCP v2 `MAP` (60 bytes) con `nonce` aleatorio de 96 bits y dirección IPv4 mapeada en formato IPv6 (`::ffff:a.b.c.d`).
   - Se valida que la puerta de enlace sea una dirección LAN/loopback válida (`is_valid_gateway_addr`) y se aplica *timeout* de `2 s` por intento.
3. **NAT-PMP (RFC 6886) sobre UDP `:5351`**:
   - Si el router responde `UNSUPP_VERSION` (`result_code = 1`) o no soporta PCP, la cascada degrada automáticamente a NAT-PMP v0 (`PUBLIC_ADDR` + `MAP_TCP` / `MAP_UDP`).
4. **UPnP IGDv2 IPv6 Firewall Pinhole (`WANIPv6FirewallControl:1`)**:
   - Cuando se detecta una dirección IPv6 global no-link-local (`2000::/3`) y una puerta de enlace LAN válida con servicio `WANIPv6FirewallControl:1`, se solicita la apertura de un *pinhole* (`AddPinhole`) y se registra su `UniqueID` para renovación y cierre (`DeletePinhole`).
   - Todas las URLs de control SOAP se validan contra `is_valid_gateway_addr` para impedir cualquier desvío SSRF.

---

## 2. Dual-Stack IPv6 y Happy Eyeballs v2 (RFC 8305)

Cuando un par anuncia tanto un punto de enlace IPv6 (`"pc26_ipv6"`) como IPv4:

1. La lista de candidatos se ordena intercalando familias de direcciones con **preferencia inicial por IPv6** seguida de IPv4.
2. Si solo hay una dirección candidata, se ejecuta una conexión TCP directa sin temporizadores adicionales.
3. Si hay múltiples familias, se inicia el primer intento (`IPv6`) y se espera `HAPPY_EYEBALLS_DELAY = 250 ms` antes de lanzar el siguiente (`IPv4`) en paralelo.
4. La primera conexión que complete el saludo TCP gana inmediatamente y cancela los intentos restantes, evitando los bloqueos de segundos ante IPv6 mal enrutado.
5. El oyente entrante soporta enlace Dual-Stack mediante `socket2` (`only_v6(false)` cuando se vincula a `[::]`).

---

## 3. Transporte QUIC P2P (RFC 9000)

El módulo `quic_transport.rs` levanta un extremo `quinn::Endpoint` sobre UDP (bajo permiso explícito de `NetworkPolicyManager`):

- **ALPN**: `"pulse-ed2k-2026"`.
- **Autenticación mutua sin CA externa (P2P)**:
  - Cada nodo genera en memoria un certificado X.509 autofirmado a partir de su clave persistente Ed25519 (`ModernIdentityKey`) mediante `rcgen`.
  - El verificador personalizado de `rustls` comprueba la firma TLS 1.3 del apretón de manos y verifica que la clave pública `SubjectPublicKeyInfo` del certificado remoto coincide exactamente con los 32 bytes Ed25519 anunciados por el par (`"pc26_ed25519"`), eliminando cualquier dependencia de autoridades certificadoras y neutralizando ataques *MitM*.
- **Multiplexación**: permite abrir múltiples flujos bidireccionales independientes por conexión sin *Head-of-Line blocking*, conservando intacto el entramado eD2K/PC26 y el *kill switch* atómico.

---

## 4. Coordinación de perforación simultánea UDP (*UDP Hole Punching*)

Para conectar pares tras NAT simétrico ligero o NAT de cono completo donde el callback TCP es imposible:

1. El módulo `hole_punch.rs` define un microprotocolo binario compacto de `13` bytes (`HOLE_PUNCH_FRAME_LEN`) con prefijo mágico `HOLE_PUNCH_MAGIC = *b"PCHP"` (`0x50 0x43 0x48 0x50`), distinguible de Kad/eD2K UDP:
   - Disposición en cable: `[magic "PCHP" (4B)][kind u8][session_id u64 BE]`
   - Tipos (`PunchMsgKind`): `Sync = 0x01`, `Ping = 0x02`, `Ack = 0x03`.
2. Tras recibir el punto de enlace observado por el nodo coordinador (*rendezvous* / compañero Kad), ambos extremos emiten ráfagas sincronizadas de hasta `6` paquetes `Ping` (`DEFAULT_PUNCH_ATTEMPTS = 6`) espaciados `80 ms` (`PUNCH_INTERVAL = 80 ms`) hacia el par remoto hasta recibir `Ack` con el mismo `session_id` (límite `PUNCH_ACK_TIMEOUT = 1200 ms`), validando el camino UDP directo para establecer QUIC.

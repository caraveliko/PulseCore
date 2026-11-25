# Negociación y Perímetro de Seguridad en `mule-torrent` (Estándar 2026)

Estado: **Implementado sobre `librqbit 9.0.1`**

Este documento especifica cómo el motor BitTorrent integrado (`crates/mule-torrent`) adopta el mismo perímetro de seguridad, red y privacidad que el núcleo eD2K/Kad, documentando de forma transparente las capacidades nativas de `librqbit 9.0.1` y los puntos de extensión futuros.

---

## 1. Integración de políticas de seguridad (`mule-filter` y `mule-network-policy`)

1. **Perfiles de red**:
   - Cuando `NetworkPolicyManager::profile().is_loopback_only()` es verdadero (`LocalOnly` / laboratorio), `TorrentEngine::new_with_policy` fuerza el modo estricto sin conexión externa (`dht: None`, `disable_trackers: true`, `listen: None`, sin LSD ni UPnP).
   - En perfil público, solo habilita red tras validar la política activa.
2. **Interruptor de corte (*Kill Switch*) atómico**:
   - El token `network_policy.cancellation_token()` se enlaza al `tokio_util::sync::CancellationToken` de `librqbit::SessionOptions`: al activarse el *kill switch*, la sesión BitTorrent se cancela de forma inmediata.
3. **Filtro IP unificado (`ipfilter.dat`)**:
   - Los rangos cargados en `mule_filter::IpFilter` se exportan atómicamente en formato `nombre:inicio-fin` y se inyectan mediante `SessionOptions.blocklist_url` (`file://...`) para que los pares bloqueados en eD2K queden también excluidos en BitTorrent.
4. **Protección anti-SSRF en `add_url`**:
   - `validate_torrent_url` restringe explícitamente los esquemas admitidos a `magnet:`, `http://` y `https://`, rechazando cualquier otro esquema (como `file://`).
   - Las URLs `http://` y `https://` pasan por validación de host/IP de `NetworkPolicyManager` antes de delegar en `librqbit`, rechazando direcciones loopback, privadas o de metadatos cloud (`169.254.169.254`) cuando el perfil no las autoriza. Los enlaces `magnet:` continúan su flujo habitual.
5. **Vigilancia «Solo Vídeo» en `fastresume`**:
   - Al restaurar torrents incompletos desde `bt_session`, `TorrentEngine::new_with_policy` reactiva automáticamente `spawn_video_header_watchdog` para verificar las cabeceras binarias de los archivos de vídeo en curso.

---

## 2. Transporte y capacidades modernas en `librqbit 9.0.1`

| Capacidad | Estado en `mule-torrent` (2026) | Mecanismo en `librqbit 9.0.1` |
| :--- | :--- | :--- |
| **uTP (BEP 29)** | **Activo por defecto** | `ListenerOptions { mode: ListenerMode::TcpAndUtp, .. }` |
| **Dual-Stack IPv6** | **Activo por defecto** | `ipv4_only: false` y escucha dual-stack `[::]` |
| **PEX / DHT** | **Activo en perfil público** | DHT persistente y extensiones de pares estándar |
| **FastResume** | **Activo por defecto** | `SessionOptions.fastresume = true` con persistencia en `data_dir/bt_session` |
| **UPnP coordinado** | **Sincronizado con DaemonConfig/BD** | `ListenerOptions.enable_upnp_port_forwarding` ligado al ajuste `upnp_enabled` |
| **Filtro IP (`ipfilter.dat`)** | **Activo** | Exportación atómica → `blocklist_url` |
| **Kill Switch y Anti-SSRF** | **Activo** | Enlazado a `NetworkPolicyManager` |
| **MSE/PE (cifrado RC4 BT)** | *No soportado por `librqbit 9.0.1`* | Expuesto en `TorrentCapabilitiesSnapshot.mse_pe_supported` como `false` |
| **BitTorrent v2 (BEP 52)** | *No soportado por `librqbit 9.0.1`* | Árbol Merkle BLAKE3/SHA-256 ya disponible en `mule-crypto::merkle_blake3`; expuesto en `TorrentCapabilitiesSnapshot.bt_v2_supported` como `false` |
| **Holepunching (BEP 55)** | *No soportado por `librqbit 9.0.1`* | Expuesto en `TorrentCapabilitiesSnapshot.holepunch_supported` como `false` |

> [!NOTE]
> `TorrentCapabilitiesSnapshot` reporta de manera honesta qué capacidades están activas en tiempo de ejecución y cuáles esperan una versión posterior del motor subyacente, evitando ocultar limitaciones de la librería.

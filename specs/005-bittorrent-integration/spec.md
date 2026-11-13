# Especificación de funcionalidad: Integración de BitTorrent y Magnet (motor dual de protocolos)

**ID de funcionalidad**: `005-bittorrent-integration`  
**Estado**: Implementado  
**Versión objetivo**: `0.5.0`  
**Versión objetivo**: `0.5.0`  
**Autor**: Antigravity & User  
**Creado**: 2026-10-29  

---

## 1. Resumen ejecutivo

`eMule-Rust` es un cliente eDonkey2000/Kad moderno y eficiente en memoria. Sin embargo, los ecosistemas modernos de servidores domésticos dependen en gran medida tanto de eD2k (para contenido clásico, regional y poco común) como de BitTorrent (para estrenos recientes y descargas rápidas). Ejecutar clientes separados (p. ej., qBittorrent y aMule) consume una cantidad significativa de CPU y RAM en dispositivos con recursos limitados como la Raspberry Pi.

Esta especificación introduce un **motor BitTorrent** nativo y asíncrono (`crates/mule-torrent`) integrado de forma transparente en `mule-daemon`. Para Sonarr, Radarr, Prowlarr y la interfaz web, el demonio actúa como un cliente P2P universal que gestiona tanto enlaces `ed2k://` como transferencias `magnet:` / `.torrent` en un único proceso que consume menos de 55 MB de RAM.

---

## 2. Requisitos e historias de usuario

### Historias de usuario
1. **Ingesta universal desde Sonarr/Radarr**: El usuario configura un único cliente Transmission en Sonarr. Cuando Sonarr envía un lanzamiento torrent (URI magnet o `.torrent`), el demonio lo encola y descarga mediante el protocolo BitTorrent. Cuando se envía un enlace eD2k, lo descarga mediante eD2k.
2. **Supervisión unificada en la interfaz web**: En la interfaz web, todas las transferencias aparecen en una lista unificada con insignias claras: `[eD2k]` (verde) o `[Torrent]` (azul). Los usuarios pueden pausar, reanudar, eliminar e inspeccionar el progreso con independencia del protocolo.
3. **Bajo consumo de recursos**: El demonio combinado con eD2k/Kad y BitTorrent activos no debe superar los 60 MB RSS en un uso doméstico típico.

---

## 3. Arquitectura de protocolos

```
                                  +-----------------------+
                                  |     Sonarr / Radarr   |
                                  +-----------+-----------+
                                              | HTTP Transmission RPC
                                              v
                                  +-----------------------+
                                  |      mule-daemon      |
                                  |    (Axum / WebUI)     |
                                  +-----+-----------+-----+
                                        |           |
            magnet: / .torrent          |           |  ed2k://
       +--------------------------------+           +-----------------------------+
       |                                                                          |
       v                                                                          v
+-------------------------------+                         +-------------------------------+
|         mule-torrent          |                         |           mule-core           |
| (BitTorrent DHT, Peers, uTP)  |                         |  (eD2k, Kad DHT, AICH, Obf)   |
+-------------------------------+                         +-------------------------------+
```

### Enrutamiento del protocolo Transmission RPC
- `torrent-add`:
  - Si `filename` comienza por `magnet:?` o contiene una URL `.torrent` / base64: enrutar a `mule-torrent`.
  - Si `filename` comienza por `ed2k://`: enrutar a `mule-core`.
- `torrent-get`:
  - Combina las descargas activas tanto de `mule-core` (eD2k) como de `mule-torrent` (BitTorrent) en el array `torrents` de la respuesta.
- `torrent-start` / `torrent-stop` / `torrent-remove`:
  - Despacha las acciones al motor correspondiente en función del ID o hash del torrent.

# Despliegue de Pulse Core con Docker

Pulse Core (`mule-daemon`) incluye la WebUI embebida, SQLite integrado y `tikv-jemallocator` configurado para mantener bajo el consumo de RAM. La imagen oficial se basa en `debian:bookworm-slim` (`glibc`) e incluye `mule-daemon` y `mule-cli`.

## Inicio rápido

```bash
docker compose up -d --build
```

La interfaz web quedará disponible en `http://127.0.0.1:4660` (o en la IP de tu servidor/NAS). Si no defines `MULE_AUTH_TOKEN`, el daemon generará uno automáticamente en `/config/auth.token`.

## Estructura de volúmenes (*hardlinks*)

El organizador multimedia de Pulse Core utiliza enlaces duros (`hard_link`) para mover o clasificar películas y series sin duplicar espacio en disco ni generar escrituras extra. Para que funcionen en Docker, las descargas y la biblioteca deben residir **dentro del mismo volumen**:

- `/config` → base de datos `mule.db`, registros (`logs/`), `auth.token` y ajustes.
- `/data` → volumen único con `/data/downloads`, `/data/media/Movies` y `/data/media/Series`.

> [!IMPORTANT]
> Evita montar `/downloads` y `/media` como dos volúmenes Docker separados; si pertenecen a puntos de montaje distintos, el kernel devolverá `EXDEV` y el organizador tendrá que copiar los archivos completos.

## Permisos (`PUID` / `PGID`)

Por defecto el contenedor ejecuta `mule-daemon` con UID/GID `1000:1000`. En NAS (Synology, TrueNAS, Unraid) o servidores Linux puedes ajustarlos en `docker-compose.yml`:

```yaml
environment:
  PUID: "1000"
  PGID: "1000"
```

## Puertos y modos de red

| Puerto | Protocolo | Uso |
| :--- | :--- | :--- |
| `4660` | TCP | WebUI, API JSON-RPC/WebSocket, Torznab y RPC compatible con Transmission |
| `39735` | TCP | Tráfico P2P entrante eD2K |
| `39737` | UDP | Red DHT Kademlia (Kad) |

- **Modo `bridge` (predeterminado)**: abre solo los puertos necesarios y funciona directamente detrás de un proxy inverso o en paralelo con contenedores de Sonarr/Radarr/Jellyfin en la misma red Docker.
- **Modo `host`**: descomenta `network_mode: host` y fija `MULE_UPNP: "true"` si deseas que el daemon abra los puertos automáticamente en tu router mediante UPnP.
- **VPN (Gluetun)**: usa `network_mode: "service:gluetun"`, elimina la sección `ports:` del servicio `pulsecore` y expón `4660/tcp`, `39735/tcp` y `39737/udp` en el contenedor `gluetun`.

## Uso de `mule-cli` dentro del contenedor

```bash
docker exec -it pulsecore mule-cli --workdir /config status
```

## Compilación manual de la imagen

```bash
docker build -t pulsecore:local .
```

# ⚙️ Configuración y Ajustes del Sistema

Pulse Core cuenta con un sistema de configuración unificado y persistente en **SQLite**, gestionado a través de su interfaz web o mediante la API JSON-RPC. La gran mayoría de los parámetros se aplican **en caliente**, sin requerir reinicios del daemon.

---

## 1. Almacenamiento y Rutas de Datos

### Ubicación predeterminada de los datos
- **En Linux / macOS**: `~/.local/share/eMuleRust/` (o según `$XDG_DATA_HOME`).
- **En Windows**: `%LOCALAPPDATA%\eMuleRust\`.
- **Modo Portable**: Si existe un archivo `portable.flag` junto al ejecutable, todos los datos, bases de datos y configuraciones se almacenan en una carpeta local `data/` en el mismo directorio del binario.

### Estructura de archivos internos
- `settings.sqlite3`: Base de datos que almacena pares clave-valor de configuración, el historial de transferencias y la biblioteca multimedia.
- `ipfilter.dat`: Lista de rangos de direcciones IP bloqueadas para el cortafuegos a nivel de socket.
- `server.met`: Lista de servidores eD2K conocidos.
- `nodes.dat`: Tabla de contactos para el bootstrap de la red DHT Kademlia.

---

## 2. Puertos de Red y Reglas de Cortafuegos

Para obtener **ID Alta (HighID)** en redes eD2K y una conectividad óptima en Kademlia y Torrent, es recomendable mapear los siguientes puertos en el cortafuegos de tu sistema operativo o en tu router doméstico (si no utilizas UPnP):

| Puerto por defecto | Protocolo | Servicio | Función y Notas |
| :---: | :---: | :--- | :--- |
| **`4660`** | TCP | WebUI & RPC | Interfaz Web en navegador y APIs locales. Por seguridad, enlaza a `127.0.0.1` salvo configuración remota autorizada. |
| **`39735`** | TCP | Red eD2K | Conexión entrante para transferencia de fragmentos entre clientes eDonkey/eMule. |
| **`39737`** | UDP | Kademlia DHT | Mensajes de enrutamiento Kademlia, peticiones de fuentes y comprobación de cortafuegos. |
| **`6881`** | TCP / UDP | Red Torrent | Negociación y transferencia de piezas del enjambre Torrent. |

> [!TIP]
> **Soporte UPnP / NAT-PMP integrado:** Pulse Core incluye negociación automática de puertos mediante el protocolo UPnP/IGD. Si tu router lo admite, abrirá y cerrará los puertos necesarios automáticamente al arrancar y detener el servicio.

---

## 3. Parámetros de Configuración Clave

A continuación se resumen los parámetros operativos más relevantes, modificables en cualquier momento desde la pestaña **Ajustes** de la interfaz web o mediante `settings.set`:

### Categoría General
| Clave | Tipo | Valor por defecto | Descripción |
| :--- | :---: | :---: | :--- |
| `nickname` | String | `"PulseCore"` | Apodo anunciado a los servidores eD2K y a los pares remotos. |
| `defaultDownloadDir` | Path | `~/Descargas/PulseCore` | Carpeta de destino donde se mueven las transferencias completadas (`Incoming`). |
| `tempDir` | Path | `~/.local/share/eMuleRust/temp` | Carpeta donde se escriben los fragmentos temporales (`.part` y `.part.met`). |
| `shareDownloadDir` | Bool | `true` | Si está activo, comparte automáticamente los archivos descargados en la carpeta `Incoming`. |
| `historyRetentionDays` | U32 | `30` | Días de persistencia del historial de descargas finalizadas en la base de datos. |
| `ichEnabled` | Bool | `true` | Recuperación inteligente de partes corruptas (*Intelligent Corruption Handling*). |

### Categoría Conexión y Redes
| Clave | Tipo | Valor por defecto | Descripción |
| :--- | :---: | :---: | :--- |
| `networkProfile` | String | `"public-ed2k-kad-preview"` | Perfil de red activo: `local-lab` (aislado en loopback), `public-ed2k-preview` o `public-ed2k-kad-preview`. |
| `maxConnections` | U32 | `500` | Número máximo de sockets TCP concurrentes permitidos. |
| `maxSourcesPerFile` | U32 | `300` | Límite máximo de fuentes activas asignadas a una misma descarga. |
| `upnpEnabled` | Bool | `true` | Apertura automática de puertos en el router mediante UPnP. |
| `killSwitchEnabled` | Bool | `false` | Interruptor atómico de corte: bloquea instantáneamente todo el tráfico P2P si se detecta una anomalía de red. |

### Categoría Seguridad y Ancho de Banda
| Clave | Tipo | Valor por defecto | Descripción |
| :--- | :---: | :---: | :--- |
| `ipFilterEnabled` | Bool | `true` | Activa el filtrado rápido en memoria contra rangos de `ipfilter.dat`. |
| `maxUploadKbs` | U32 | `0` (ilimitado) | Límite global de tasa de subida en KiB/s (`0` = sin límite). |
| `maxDownloadKbs` | U32 | `0` (ilimitado) | Límite global de tasa de descarga en KiB/s (`0` = sin límite). |

---

## 4. Modificación de Ajustes vía JSON-RPC

Para entornos de automatización, los ajustes pueden consultarse y actualizarse mediante llamadas WebSocket o HTTP JSON-RPC:

### Consultar configuración actual (`settings.get`)
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "settings.get",
  "params": {}
}
```

### Modificar un ajuste en caliente (`settings.set`)
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "settings.set",
  "params": {
    "nickname": "Servidor-Pulse",
    "maxUploadKbs": 2048
  }
}
```
El daemon aplicará el nuevo límite de subida y el nuevo apodo inmediatamente, sin interrumpir ninguna descarga en curso.

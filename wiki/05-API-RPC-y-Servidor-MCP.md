# 🤖 API RPC y Servidor MCP para Inteligencia Artificial

Pulse Core está diseñado para ser completamente automatizable y programable. Dispone de una interfaz **JSON-RPC 2.0 sobre WebSockets** y un servidor nativo **MCP (Model Context Protocol)** que expone más de 85 herramientas para agentes de Inteligencia Artificial.

---

## 1. Protocolo JSON-RPC 2.0 sobre WebSocket

Tanto la interfaz web integrada como cualquier herramienta externa se comunican con `mule-daemon` a través del endpoint WebSocket:

- **Endpoint**: `ws://127.0.0.1:4660/rpc`
- **Comprobación de salud (*Health check*)**: `GET http://127.0.0.1:4660/health`
- **Framing**: Un mensaje de texto WebSocket = una petición, una respuesta o una notificación JSON en UTF-8.

### Estructura de Mensajes
```jsonc
// Petición (Request)
{"jsonrpc": "2.0", "id": 1, "method": "downloads.list", "params": {}}

// Respuesta exitosa (Response)
{"jsonrpc": "2.0", "id": 1, "result": {"downloads": [...]}}

// Respuesta de error (Error)
{"jsonrpc": "2.0", "id": 1, "error": {"code": -32010, "message": "Enlace no válido", "data": {}}}

// Notificación de evento en vivo (sin id)
{"jsonrpc": "2.0", "method": "download.completed", "params": {"fileHash": "..."}}
```

---

## 2. Catálogo de Métodos RPC Principales

Los métodos están organizados por dominios funcionales:

### Dominio `downloads` (Transferencias)
| Método | Parámetros | Descripción |
| :--- | :--- | :--- |
| `downloads.list` | `{}` | Devuelve la lista completa de transferencias activas, estado, velocidad y fuentes. |
| `downloads.add` | `{"link": "..."}` | Añade una descarga a partir de un enlace `ed2k://`, `magnet:?` o archivo local. |
| `downloads.pause` | `{"fileHash": "..."}` | Pausa temporalmente la descarga de un archivo. |
| `downloads.resume` | `{"fileHash": "..."}` | Reanuda una descarga previamente pausada. |
| `downloads.cancel` | `{"fileHash": "..."}` | Cancela la transferencia y elimina los fragmentos temporales en disco. |
| `downloads.setPriority` | `{"fileHash": "...", "priority": "high"}` | Ajusta la prioridad de asignación de ancho de banda (`low`, `normal`, `high`). |

### Dominio `servers` y `kad` (Redes P2P)
| Método | Parámetros | Descripción |
| :--- | :--- | :--- |
| `servers.list` | `{}` | Lista todos los servidores eD2K conocidos y su estado de conexión. |
| `servers.connect` | `{"ip": "...", "port": ...}` | Fuerza la conexión a un servidor eD2K específico. |
| `kad.status` | `{}` | Muestra el estado del enrutamiento DHT, contactos activos e ID de cortafuegos. |
| `kad.bootstrap` | `{"ip": "...", "port": ...}` | Envía una sonda de inicio a un nodo conocido para unirse a la red Kad. |

### Dominio `search` y `settings`
| Método | Parámetros | Descripción |
| :--- | :--- | :--- |
| `search.start` | `{"query": "...", "target": "global"}` | Inicia una búsqueda descentralizada en Kad y servidores eD2K. |
| `search.results` | `{"searchId": ...}` | Obtiene los resultados agregados y ordenados por número de fuentes. |
| `settings.get` | `{}` | Lee la configuración operativa completa del daemon. |
| `settings.set` | `{"nickname": "...", ...}` | Modifica parámetros operativos en caliente sin reiniciar. |
| `stats.get` | `{}` | Devuelve telemetría en tiempo real: tasas de subida/bajada, uso de RAM y contadores de sockets. |

---

## 3. Servidor MCP (Model Context Protocol)

Pulse Core incluye un ejecutable dedicado (`mule-mcp`) que implementa la especificación **Model Context Protocol** de Anthropic / Linux Foundation.

Permite a agentes de IA (como Antigravity, Claude Code o asistentes locales) interactuar de forma segura con el daemon en lenguaje natural:

```
[Usuario] "Detén las descargas de baja prioridad y dime cuántos contactos hay en Kad"
   │
   ▼
[Agente IA] -> [mule-mcp] -> JSON-RPC -> [mule-daemon]
   │
   ▼
"Se han pausado 2 descargas de baja prioridad. La red Kad cuenta actualmente con 682 contactos activos con ID Alta."
```

### Configuración de `mule-mcp` en clientes de IA
Añade el servidor a tu archivo de configuración de MCP (ej. `mcp.json` o `claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "pulse-core": {
      "command": "/ruta/a/target/release/mule-mcp",
      "args": ["--daemon-url", "ws://127.0.0.1:4660/rpc"]
    }
  }
}
```

---

## 4. Ejemplo Práctico en Python (Cliente WebSocket)

Puedes interactuar con Pulse Core desde cualquier lenguaje con apenas unas líneas de código:

```python
import asyncio
import json
import websockets

async def consultar_estado():
    uri = "ws://127.0.0.1:4660/rpc"
    async with websockets.connect(uri) as ws:
        # Petición JSON-RPC para obtener estadísticas
        peticion = {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "stats.get",
            "params": {}
        }
        await ws.send(json.dumps(peticion))
        
        respuesta = await ws.recv()
        datos = json.loads(respuesta)
        
        stats = datos["result"]
        print(f"Bajada actual: {stats['downloadSpeedKbs']} KiB/s")
        print(f"Subida actual: {stats['uploadSpeedKbs']} KiB/s")
        print(f"Pares eD2K activos: {stats['activeEd2kPeers']}")

asyncio.run(consultar_estado())
```

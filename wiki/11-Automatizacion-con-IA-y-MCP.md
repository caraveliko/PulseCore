# 🤖 11. Automatización con IA y Servidor MCP

**Pulse Core** es el primer motor P2P del mundo diseñado nativamente para la era de la Inteligencia Artificial. A través de su ejecutable dedicado **`mule-mcp`**, implementa el estándar abierto **Model Context Protocol (MCP)**, permitiendo a asistentes de IA como **Antigravity**, **Claude Code**, **Gemini CLI** o **Cursor** controlar el daemon de forma autónoma y segura mediante lenguaje natural.

---

## 1. ¿Cómo funciona la Integración con IA?

```text
┌─────────────────┐       Llamada MCP        ┌──────────────┐     WebSocket RPC    ┌─────────────┐
│  Agente de IA   │ ───────────────────────> │   mule-mcp   │ ───────────────────> │ mule-daemon │
│  (Antigravity)  │ <─────────────────────── │ (85+ tools)  │ <─────────────────── │ (Motor P2P) │
└─────────────────┘      JSON Estructurado   └──────────────┘      Eventos y Datos └─────────────┘
```

1. El usuario le pide algo a la IA en lenguaje natural (ej. *«Comprueba si hay descargas atascadas y aumenta su prioridad»*).
2. El modelo de IA consulta el catálogo de herramientas de `mule-mcp`.
3. `mule-mcp` ejecuta la llamada sobre `mule-daemon` y le devuelve los datos estructurados a la IA.
4. La IA interpreta la respuesta y responde de forma comprensible al usuario.

---

## 2. Configuración en Clientes de IA

### En Claude Desktop / Claude Code (`claude_desktop_config.json`)
Añade el servidor en tu archivo de configuración de Claude:

```json
{
  "mcpServers": {
    "pulse-core": {
      "command": "/ruta/absoluta/a/PulseCore/target/release/mule-mcp",
      "args": ["--daemon-url", "ws://127.0.0.1:4660/rpc"]
    }
  }
}
```

### En Antigravity / Gemini CLI
Si utilizas Antigravity, el servidor MCP se autodescubre a través de las extensiones del workspace de Cargo.

---

## 3. Catálogo de Herramientas MCP Disponibles (~87 herramientas)

Algunas de las capacidades que la IA puede ejecutar:

| Herramienta MCP | Acción que realiza la Inteligencia Artificial |
| :--- | :--- |
| `downloads_list` | Lee todas las transferencias, su velocidad y su porcentaje. |
| `downloads_add` | Añade nuevos enlaces `ed2k://`, `magnet:?` o `.torrent`. |
| `downloads_pause` / `resume` | Pausa o reanuda descargas específicas. |
| `downloads_set_priority` | Asigna prioridad Alta, Normal o Baja a un archivo. |
| `kad_status` | Analiza el estado del enrutamiento Kad y el número de contactos. |
| `servers_list` | Consulta la lista de servidores eD2K y el estado de la ID. |
| `media_search` | Busca películas y series en la base de datos de metadatos. |
| `media_organize` | Dispara el proceso de organización y creación de enlaces duros. |
| `subtitles_download` | Busca y descarga subtítulos sincronizados con OpenSubtitles. |
| `stats_get` | Proporciona un informe completo de telemetría de red y consumo de RAM. |

---

## 4. Ejemplos de Uso en Lenguaje Natural

Una vez conectado, puedes interactuar con tu servidor Pulse Core de formas increíbles:

- 💬 *«¿Cuál es el estado de mis descargas hoy?»*  
  👉 La IA listará las transferencias activas, el tiempo restante estimado y la velocidad global.

- 💬 *«Busca en la red eD2K documentales en español sobre naturaleza y descárgame el que tenga más fuentes completas.»*  
  👉 La IA iniciará la búsqueda, filtrará los resultados con más disponibilidad y añadirá el mejor enlace a la cola.

- 💬 *«El servidor me reporta ID Baja. Diagnostica por qué y dime qué puertos debo abrir.»*  
  👉 La IA comprobará la configuración de puertos de tu daemon y te proporcionará las instrucciones exactas para tu modelo de router.

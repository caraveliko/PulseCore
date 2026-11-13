# Guía de uso — Asistente de IA local (`mule-assistant`)

`mule-assistant` permite que un **modelo de lenguaje local** (Ollama, o cualquier backend compatible con `/api/chat`) controle eMuleRust a través de las herramientas MCP existentes. El demonio sigue siendo la única autoridad: no se duplica ninguna lógica de negocio y el modelo solo ve un subconjunto seleccionado y validado de las 87 herramientas.

```
Ollama (127.0.0.1:11434)          Panel de control
        │ chat + tool_calls               │ WS JSON-RPC
        ▼                                 ▼
            ┌─────── mule-assistant ───────┐
            │  policy : allowlist          │
            │         ∩ perfil MCP         │
            │         ∩ capacidades daemon │
            └───────────┬──────────────────┘
                        │ DaemonClient (mismo token que el RPC)
                        ▼
                   mule-daemon  ← autoridad de seguridad
```

---

## 1. Requisitos previos

| Componente | Requisito |
| :--- | :--- |
| **Ollama** | Instalado y en ejecución en `http://127.0.0.1:11434` (por defecto) |
| **Modelo** | Compatible con *tool calling* en Ollama — p. ej. `qwen3:8b` (por defecto), `llama3.1`, `mistral-nemo`… |
| **Daemon** | `mule-daemon` en ejecución, sesión activa (archivo `auth.token` presente en el workdir) |

```powershell
# Comprobar que Ollama responde y que el modelo está presente
ollama list
ollama pull qwen3:8b        # si falta
```

> **Nota**: un modelo sin soporte de tool calling funciona en modo
> «chat simple», pero no podrá ejecutar acciones — el asistente
> responderá únicamente con texto.

---

## 2. Inicio rápido

### 2.1 Modo interactivo (REPL) — el más sencillo

```powershell
cargo run -p mule-assistant -- chat
```

```
session prête — 15 outils visibles (profil full)
assistant> ¿cuál es el estado del cliente?
```

El asistente invoca las herramientas necesarias (mostradas como `[outil →]`),
muestra los resultados (`[résultat ←]`) y, a continuación, la respuesta final en
streaming. `Ctrl+C` o línea vacía para salir.

### 2.2 Modo one-shot

```powershell
cargo run -p mule-assistant -- prompt "lista mis descargas activas"
```

Sin terminal interactiva, las herramientas que requieren confirmación se **rechazan por
defecto** (comportamiento seguro) — el modelo recibe «cancelado por el
usuario» y adapta su respuesta.

### 2.3 Modo servidor — para interfaz remota

```powershell
cargo run -p mule-assistant -- serve
# ws://127.0.0.1:4680/assistant
```

Deja este proceso en ejecución y conecta desde el panel de la sección
**«Asistente de IA»**. El panel se conecta
automáticamente: el token de sesión se lee desde
`<workdir>/auth.token` — el mismo secreto que el RPC del demonio, sin ninguna
contraseña adicional.

---

## 3. Referencia de opciones

| Opción | Por defecto | Función |
| :--- | :--- | :--- |
| `--url <ws://…>` | detección automática | URL RPC del demonio (`ws://127.0.0.1:4660/rpc`) |
| `--token <hex64>` | `auth.token` del workdir | Token de sesión del demonio |
| `--workdir <ruta>` | resolución estándar | Workdir que contiene `session.json` / `auth.token` |
| `--app <ruta>` | — | Raíz del paquete de la aplicación |
| `--endpoint <http://…>` | `http://127.0.0.1:11434` | URL del backend LLM — **se requiere IP literal loopback** |
| `--model <nombre>` | `qwen3:8b` | Modelo en el backend |
| `--allow-remote-endpoint` | desactivado | Autoriza explícitamente un endpoint LLM no loopback |
| `--tool-profile <p>` | `full` | Límite de riesgo: `read` · `safe` · `full` |
| `serve --port <n>` | `4680` | Puerto loopback del servidor WebSocket |

### Perfiles de herramientas (`--tool-profile`)

El perfil limita lo que el modelo puede ver **e** invocar — el
filtrado se vuelve a comprobar en tiempo de ejecución, no solo al mostrarlo.

| Perfil | Contenido | Uso |
| :--- | :--- | :--- |
| `read` | Solo consulta (`status`, `list_*`, `get_*`, `search`…) | Exploración, demostración, uso prudente |
| `safe` | `read` + mutaciones reversibles (`pause`, `resume`, `connect`…) — **sin** las destructivas | Uso diario |
| `full` | Todas las herramientas de la allowlist | Operación completa (confirmaciones siempre activas) |

El mismo parámetro existe en el servidor MCP externo: `mule-mcp --tool-profile
safe` protege también a los clientes MCP (Claude Desktop, Devin…) — es el
mismo código, un único propietario del riesgo.

### Herramientas visibles para el modelo

El conjunto efectivo es la intersección de tres filtros, **comprobada de nuevo
antes de cada ejecución**:

```
herramientas MCP del perfil  ∩  allowlist curada (15 herramientas)  ∩  capacidades del demonio
```

La allowlist cubre las operaciones habituales:

- **Estado**: `emule_status`, `emule_get_settings`
- **Búsqueda/red**: `emule_search`, `emule_list_servers`,
  `emule_connect_server`, `emule_disconnect_server`, `emule_kad_status`
- **Descargas**: `emule_list_downloads`,
  `emule_get_download_details`, `emule_add_link`, `emule_pause_download`,
  `emule_resume_download`
- **Compartidos/fuentes**: `emule_list_shares`, `emule_swarm_status`,
  `emule_list_sources`

Si una capacidad falta en el demonio (p. ej. `server-search` desactivado),
la herramienta correspondiente se oculta al modelo.

### Confirmaciones del usuario

Algunas herramientas exigen validación humana **sobre los argumentos
exactos** propuestos por el modelo:

- `emule_add_link` (añade una descarga — efecto externo),
- `emule_connect_server` / `emule_disconnect_server`,
- **cualquier herramienta destructiva** (última barrera, incluso en perfil `full`).

En REPL: `o/n` interactivo. En la interfaz: banner «El asistente
solicita confirmación» con los argumentos mostrados y botones
**Permitir/Rechazar**. Un rechazo se convierte en un mensaje de error de herramienta
para el modelo — la conversación continúa con normalidad.

---

## 4. El panel de interfaz

Al abrir **«Asistente de IA»**, la vista muestra:

- **Cabecera**: título + indicador de estado (`Conectado · N herramientas`,
  `Conectando…`, `Error de conexión`, `Desconectado`);
- **Conversación**: burbujas de usuario (derecha) / asistente (izquierda),
  tarjetas de herramientas (llamada + resultado, contenido en monoespaciado, truncado en
  pantalla si es muy largo);
- **Streaming**: la respuesta se redacta token a token; las llamadas
  a herramientas aparecen en cuanto se validan;
- **Banner de confirmación** cuando se propone una herramienta protegida;
- **Botón detener** durante un turno para cancelar (`assistant.cancel`);
- **Estado sin conexión**: explicación + botón «Reintentar» si el
  servidor del asistente no está iniciado o si no se encuentra el token.

> La conversación se comparte por servidor: una reconexión recupera el
> mismo historial (elección monousuario intencionada).

---

## 5. Seguridad — lo que está garantizado

| Garantía | Mecanismo |
| :--- | :--- |
| El modelo no sale de la máquina | Endpoint **IP literal loopback** obligatorio (`127.0.0.1`/`::1`), nombres de host rechazados, redirecciones HTTP desactivadas, proxies de entorno ignorados |
| Sin acciones fuera de alcance | Allowlist fija ∩ perfil ∩ capacidades, recomprobada en ejecución — el modelo no puede invocar una herramienta no listada |
| Sin acciones irreversibles sin consentimiento | Confirmaciones sobre argumentos exactos; `Destructive` siempre confirmado |
| Misma autenticación que el RPC | `assistant.hello` exige el token de sesión del demonio; el WS solo escucha en loopback |
| Bucle acotado | `MAX_TOOL_ROUNDS` = 4 encadenamientos de herramientas máx. por mensaje |
| Contexto acotado | Resultados de herramientas truncados a 4000 caracteres para el modelo (la UI recibe el texto íntegro) |
| Salida «thinking» oculta | Los fragmentos de razonamiento del modelo nunca se muestran |

**Límites conocidos**: los resultados P2P voluminosos pasan por los
eventos existentes del demonio, no mediante reformulación del modelo; la
fiabilidad del tool calling depende del modelo — mídela con tus modelos
objetivo (`RUST_LOG=mule_assistant=debug` expone `load_duration`,
`prompt_eval_duration`, `eval_duration` de Ollama).

---

## 6. Protocolo WebSocket (desarrolladores)

Endpoint: `ws://127.0.0.1:4680/assistant` — un objeto JSON por trama.

```jsonc
// → autenticación (primera llamada obligatoria)
{"id":1,"method":"assistant.hello","params":{"token":"<hex64>"}}
// ← {"id":1,"result":{"server":"mule-assistant","version":"…","tools":15}}

// → mensaje del usuario
{"id":2,"method":"assistant.chat","params":{"text":"¿estado del cliente?"}}
// ← {"id":2,"result":{"accepted":true}}
// ← notificaciones posteriores:
//    assistant.event           {type:"token"|"toolStart"|"toolResult"|"final"|"notice"|"error", …}
//    assistant.confirmRequired {name, args}     → responder con:
//    assistant.turnEnd         {error: null|string}

{"id":3,"method":"assistant.confirm","params":{"allow":true}}
{"id":4,"method":"assistant.cancel","params":{}}
```

Códigos de error: `-32001` token inválido · `-32002` autenticación requerida ·
`-32003` turno ya en curso · `-32004` ninguna confirmación pendiente ·
`-32005` ningún turno en curso.

---

## 7. Resolución de problemas

| Síntoma | Causa probable | Solución |
| :--- | :--- | :--- |
| «Asistente no disponible» | `mule-assistant serve` no iniciado | Iniciar el servidor y pulsar «Reintentar» |
| «Token de sesión no encontrado» | Demonio externo o web | El asistente exige un demonio local supervisado (workdir legible) |
| `endpoint … non loopback` | URL LLM remota o nombre de host | Usar `http://127.0.0.1:11434`; `--allow-remote-endpoint` si es realmente remoto |
| El modelo nunca invoca herramientas | Modelo sin tool calling | `ollama pull qwen3:8b` u otro modelo compatible |
| `outil bloqué par le profil` | `--tool-profile read|safe` | Pasar `--tool-profile full` o elegir otra herramienta |
| Respuestas muy lentas | Generación en CPU | Modelo más pequeño; timeout LLM = 300 s (config) |
| `un tour est déjà en cours` | Doble envío | Esperar a `turnEnd` o `assistant.cancel` |

---

*Véase también: `docs/CHANGELOG.md` (entrada Unreleased),
`crates/mule-assistant/src/ws.rs` (protocolo),
`crates/mule-assistant/src/policy.rs` (allowlist y confirmaciones).*

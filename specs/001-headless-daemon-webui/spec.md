# Especificación de funcionalidad: Servidor multiplataforma (Linux / Windows) e interfaz web integrada

**ID de funcionalidad**: `001-headless-daemon-webui`  
**Estado**: Implementado  
**Versión objetivo**: `0.3.0`  
**Autor**: Antigravity & User  
**Creado**: 2026-10-29  

---

## 1. Resumen ejecutivo

**Pulse Core** (`eMule-Rust`) cuenta con un motor principal en Tokio de alto rendimiento. Esta especificación define los requisitos para consolidar `mule-daemon` como un **servidor multiplataforma sin interfaz gráfica de escritorio** pensado para Windows, laboratorios domésticos 24/7, NAS y SBC (como la Raspberry Pi 5), equipado con una **interfaz web integrada y adaptable** servida directamente desde el demonio mediante Axum y WebSockets JSON-RPC.

---

## 2. Planteamiento del problema y valor para el usuario

- **El problema**: El `aMule` tradicional consume más de 2,6 GB de RAM en una Raspberry Pi, utiliza E/S bloqueante, se bloquea o cuelga con frecuencia y depende de un servidor `amuleweb` arcaico. Por otro lado, `eMule-Rust` funciona con ~40 MB de RAM gracias a códecs de red acotados, pero carece de una interfaz web accesible para máquinas headless.
- **El valor**:
  - Reduce el consumo de memoria de 2,6 GB a menos de 60 MB de RAM.
  - Cero dependencias gráficas (GUI); se ejecuta de forma nativa como unidad systemd en x86_64 y ARM64.
  - Accesible desde cualquier navegador web (`http://amule.lan:4660` o mediante proxy inverso) en teléfonos móviles, tabletas y PC.
  - Actualizaciones en tiempo real mediante WebSockets sin recargas manuales de página.

---

## 3. Escenarios de usuario y criterios de aceptación

### Escenario 1: Arranque en servidor headless
- **Dado** un sistema Linux headless (Ubuntu / Debian / Arch / Raspberry Pi OS) o Windows en segundo plano,
- **Cuando** se lanza `mule-daemon --workdir ~/.mule --downloads-dir ~/amule/incoming --bind 0.0.0.0:4660 --ed2k-port 4662 --kad-port 4672` (como servicio `systemd` o ejecutando `mule-daemon.exe`),
- **Entonces** arranca en < 200 ms, crea un bloqueo de instancia exclusivo, inicializa SQLite en modo WAL, se vincula a la interfaz de red configurada (p. ej. `0.0.0.0:4660` o `127.0.0.1:4660`) y no requiere ninguna dependencia gráfica externa.

### Escenario 2: Acceso al panel web
- **Dado** que el demonio está en ejecución,
- **Cuando** el usuario accede a `http://<server-ip>:4660` desde un navegador,
- **Entonces** el demonio sirve una aplicación de página única (SPA) integrada,
- **Y** autentica mediante token de sesión o contraseña configurada,
- **Y** muestra el estado de conexión en tiempo real (servidor eD2k HighID, estado de Kad, velocidades de descarga/subida, cola activa).

### Escenario 3: Gestión de transferencias en tiempo real
- **Dado** que la interfaz web está abierta,
- **Cuando** el usuario pega un enlace `ed2k://|file|...|/`,
- **Entonces** el enlace se envía mediante JSON-RPC, se analiza, se valida y se añade a la lista de transferencias,
- **Y** la barra de progreso, el recuento de fuentes y el mapa de partes se actualizan en vivo a través de la conexión WebSocket.

### Escenario 4: Icono en la bandeja del sistema en Windows
- **Dado** que `mule-daemon.exe` se ejecuta en Windows (salvo que se desactive con `--no-tray` o `MULE_NO_TRAY=1`),
- **Cuando** el servidor arranca,
- **Entonces** muestra únicamente un icono ligero en la barra de tareas / bandeja del sistema (`crates/mule-daemon/src/tray.rs`), sin ventanas gráficas de escritorio,
- **Y** al hacer clic izquierdo o doble clic sobre el icono se abre directamente la página web del servidor (`http://127.0.0.1:4660/`) en el navegador predeterminado,
- **Y** el menú contextual (clic derecho) ofrece las funciones esenciales: indicar el estado y la URL de escucha, **Abrir WebUI**, **Abrir carpeta de descargas**, **Abrir carpeta de datos** y **Detener servidor** (apagado ordenado).

### Escenario 5: Selección de carpetas Incoming y Temp mediante texto o explorador de archivos
- **Dado** que el usuario está en la pestaña de directorios de la interfaz web (`Opciones → Directorios`),
- **Cuando** configura las rutas de **Carpeta de Descargas Finales (Incoming)**, **Carpeta de Fragmentos Temporales (Temp)** o bibliotecas multimedia,
- **Entonces** puede escribir la ruta manualmente en el campo de texto como hasta ahora,
- **Y** también dispone de un botón con icono (`📂`) junto a cada campo para abrir el explorador de archivos/carpetas (diálogo nativo en Windows local o explorador visual del servidor integrado en la WebUI) y seleccionar la ruta deseada.

---

## 4. Arquitectura técnica y restricciones

### 4.1 Configuración del demonio (CLI + SQLite en caliente)
- **Argumentos de arranque (`crates/mule-daemon/src/main.rs`)**: `--workdir`, `--downloads-dir`, `--bind` (por defecto `127.0.0.1:4660`), `--ed2k-port`, `--kad-port`, `--upnp`, `--web-dir`, `--token`, `--mode` (`portable` / `installed`) y `--no-tray`.
- **Modo portable**: la presencia de `portable.flag` junto al ejecutable mantiene los datos y la configuración junto al binario en lugar de `%LOCALAPPDATA%` / `~/.local/share`.
- **Ajustes persistidos en SQLite (`KNOWN_SETTINGS_KEYS` en `crates/mule-daemon/src/rpc/handlers/settings.rs`)**: todos los parámetros operativos (directorios `Incoming` y `Temp`, puertos, límites de velocidad, sensor USS, perfiles de red, automatización multimedia y MCP) se leen y actualizan en caliente desde la WebUI mediante `settings.get` y `settings.set`.

### 4.2 Estrategia de la interfaz web integrada
- Construida como SPA ligera en **HTML5 + CSS + JavaScript puro** (`web/index.html`) sin frameworks externos ni pasos de transpilación.
- Integrada en el binario final de `mule-daemon` mediante `rust-embed` para que un único ejecutable lo contenga todo (con soporte opcional de `--web-dir` para desarrollo).
- Axum sirve los recursos estáticos con cabeceras de caché (`Cache-Control`) y gestiona la comunicación JSON-RPC 2.0 mediante WebSocket en `/rpc` y HTTP POST en `/api/rpc`.

### 4.3 Icono de bandeja del sistema en Windows (System Tray)
- Implementado en `crates/mule-daemon/src/tray.rs` directamente sobre la API nativa Win32 (`Shell_NotifyIconW`, `ShellExecuteW`), sin ventanas de escritorio ni dependencias pesadas.
- Proporciona acceso directo a la página web (`Abrir WebUI` o clic/doble clic en el icono), acceso rápido a las carpetas de descargas y de datos, y parada ordenada del demonio (`Detener servidor`).

---

## 5. Invariantes de seguridad y aislamiento

- **Protección por token**: Los endpoints WebSocket y JSON-RPC requieren autenticación mediante token bearer o cookie de sesión.
- **Protección contra path traversal**: El servicio de recursos estáticos restringe estrictamente las rutas a los recursos integrados.
- **Política anti-SSRF e IP segura**: Hereda la defensa en 7 capas de `mule-network-policy` y la validación de loopback.

---

## 6. Métricas de éxito y verificación

1. **Presupuesto de memoria**: Memoria en estado estable < 60 MB RSS con más de 100 elementos activos en cola.
2. **Arranque en frío**: El demonio alcanza el estado operativo en < 250 ms.
3. **Latencia web**: Tiempo de respuesta inferior a 10 ms en red local (LAN) para consultas de estadísticas y renderizado de interfaz.
4. **Resiliencia**: Cero procesos zombi, cierre limpio ante SIGINT/SIGTERM sin corrupción del WAL de SQLite.

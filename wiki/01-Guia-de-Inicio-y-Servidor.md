# 🚀 Guía de Inicio y Despliegue en Servidores

Esta guía describe cómo compilar, configurar y ejecutar **Pulse Core** tanto en entornos de escritorio como en servidores dedicados, microservidores domésticos, NAS y placas monoplaca tipo Raspberry Pi.

---

## 1. Requisitos del Sistema

### Entorno de compilación
- **Rust Toolchain**: versión `stable` 1.80 o superior (instalable mediante [rustup.rs](https://rustup.rs/)).
- **Compilador C/C++ del sistema**:
  - En Linux: `build-essential`, `gcc` o `clang`, y cabeceras de desarrollo de OpenSSL (`libssl-dev` o `openssl-devel`).
  - En Windows: Visual Studio C++ Build Tools (MSVC x64) o clang.
  - En macOS: Xcode Command Line Tools (`xcode-select --install`).

### Consumo de recursos estimado
- **Memoria RAM**: ~35-50 MB en reposo; ~70-120 MB bajo transferencias intensivas concurrentes eD2K y Torrent.
- **CPU**: Mínimo impacto gracias a la arquitectura Tokio asíncrona y al bucle de eventos sin bloqueo.

---

## 2. Compilación Nativa Universal con Cargo

La forma recomendada y canónica de compilar Pulse Core en cualquier sistema operativo es utilizando directamente `cargo`:

```bash
# 1. Clonar el repositorio oficial
git clone https://github.com/caraveliko/PulseCore.git
cd PulseCore

# 2. Compilar el servicio daemon en modo Release
cargo build --release -p mule-daemon

# 3. (Opcional) Compilar la herramienta CLI de inspección local
cargo build --release -p mule-cli
```

Los binarios optimizados se generan en el directorio `target/release/`:
- `mule-daemon` (en Linux / macOS) o `mule-daemon.exe` (en Windows): Servicio en segundo plano con motor P2P, WebUI y APIs RPC.
- `mule-cli` o `mule-cli.exe`: Herramienta de línea de comandos para inspección de enlaces eD2K, archivos `.part.met` y sumas de verificación.

---

## 3. Despliegue en Servidores Linux (*Headless*) con systemd

Pulse Core está diseñado para ejecutarse de forma desatendida y continua en servidores sin monitor ni entorno de escritorio gráfico.

### Paso 1: Instalar el binario en el sistema
```bash
mkdir -p ~/.local/bin
cp target/release/mule-daemon ~/.local/bin/
```

### Paso 2: Crear el servicio de usuario de systemd
Crea el archivo `~/.config/systemd/user/pulse-core.service`:

```ini
[Unit]
Description=Pulse Core P2P Daemon (eD2K + Kad + Torrent)
After=network.target

[Service]
Type=simple
ExecStart=%h/.local/bin/mule-daemon
Restart=always
RestartSec=5
Environment=RUST_LOG=mule_daemon=info,mule_protocol=info,mule_core=info
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
```

### Paso 3: Activar y arrancar el servicio
```bash
# Recargar la configuración de servicios de usuario
systemctl --user daemon-reload

# Habilitar y arrancar inmediatamente
systemctl --user enable --now pulse-core

# Comprobar el estado del servicio
systemctl --user status pulse-core
```

### Paso 4: Persistencia tras cerrar sesión (*Linger*)
En distribuciones como Debian, Ubuntu o Raspberry Pi OS, los servicios de usuario de `systemd` se detienen al cerrar la sesión SSH a menos que se active el modo *linger*:

```bash
loginctl enable-linger $USER
```

A partir de este momento, Pulse Core continuará ejecutándose ininterrumpidamente tras reiniciar el servidor.

---

## 4. Despliegue en Raspberry Pi (`aarch64`)

Pulse Core cuenta con soporte nativo de primera clase para arquitecturas ARM64 (Raspberry Pi 4, Raspberry Pi 5 y placas similares):

### Compilación directa en la Raspberry Pi
Simplemente ejecuta los pasos estándar de Cargo descritos en la sección 2.

### Compilación cruzada desde un PC x86_64
Si dispones de una máquina más potente y deseas compilar para tu Raspberry Pi en segundos:

```bash
# Instalar el objetivo aarch64 en rustup
rustup target add aarch64-unknown-linux-gnu

# Compilar para ARM64
cargo build --release -p mule-daemon --target aarch64-unknown-linux-gnu

# Transferir el binario compilado a la Raspberry Pi vía SCP
scp target/aarch64-unknown-linux-gnu/release/mule-daemon usuario@192.168.1.xxx:~/.local/bin/
```

---

## 5. Ejecución en Windows 10/11 con Bandeja del Sistema

En Windows, `mule-daemon.exe` incluye integración nativa con la bandeja del sistema (*system tray* de Windows):

1. Ejecuta `.\target\release\mule-daemon.exe`.
2. El daemon creará automáticamente un icono en la barra de tareas junto al reloj.
3. Haciendo clic derecho sobre el icono podrás:
   - Abrir el panel de control web en tu navegador predeterminado.
   - Consultar el estado de subida y bajada.
   - Detener el servicio de forma limpia.

*(Si deseas empaquetar una release portable en `.zip` con scripts `.bat` y manifiestos SBOM, puedes ejecutar `powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\build_windows_server.ps1`).*

---

## 6. Primer Acceso y Verificación

Una vez iniciado el daemon, abre tu navegador y accede a:
> 🌐 **`http://127.0.0.1:4660`** (o sustituye `127.0.0.1` por la IP local de tu servidor en la red doméstica).

Desde esta interfaz podrás:
- Supervisar transferencias en tiempo real mediante WebSockets.
- Añadir enlaces `ed2k://`, archivos `.torrent` o enlaces `magnet:?`.
- Configurar tus carpetas de descargas entrantes y temporales en la pestaña **Ajustes**.

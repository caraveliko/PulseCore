# 🔰 01. Instalación y Primeros Pasos

Esta guía te guiará paso a paso desde la obtención del código fuente hasta la puesta en marcha de **Pulse Core** y tu primer acceso al panel de control web.

---

## 1. Requisitos Previos

Antes de comenzar, asegúrate de contar con:
- **Rust Toolchain**: versión estable 1.80 o más reciente. Si no lo tienes instalado, abre tu terminal y ejecuta:
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
  *(En Windows puedes descargar el instalador oficial desde [rustup.rs](https://rustup.rs/)).*
- **Compilador C/C++ del sistema**:
  - **Linux (Debian / Ubuntu / Raspberry Pi OS)**:
    ```bash
    sudo apt update && sudo apt install -y build-essential libssl-dev pkg-config
    ```
  - **Linux (Arch / Manjaro)**:
    ```bash
    sudo pacman -S --needed base-devel openssl
    ```
  - **Windows 10 / 11**: Microsoft Visual Studio C++ Build Tools (MSVC x64).

---

## 2. Clonar y Compilar Pulse Core

### Paso 1: Obtener el código fuente
Abre una terminal o consola y clona el repositorio oficial de GitHub:
```bash
git clone https://github.com/caraveliko/PulseCore.git
cd PulseCore
```

### Paso 2: Compilación nativa con Cargo
Compila el ejecutable del daemon en modo optimizado (*Release*):
```bash
cargo build --release -p mule-daemon
```
*Tiempo estimado: entre 1 y 3 minutos según la potencia de tu procesador.*

Una vez finalizado, encontrarás el ejecutable en:
- En Linux: `target/release/mule-daemon`
- En Windows: `target\release\mule-daemon.exe`

---

## 3. Primer Arranque del Servicio

### En Linux / macOS / Raspberry Pi
Ejecuta el daemon directamente en la terminal:
```bash
./target/release/mule-daemon
```

Verás una salida de inicio similar a esta:
```text
INFO mule_daemon: Iniciando Pulse Core v0.3.0 (Motor P2P eD2K + Kad + Torrent)
INFO mule_daemon::inbound_listener: Escuchando WebUI y WebSocket RPC en 127.0.0.1:4660
INFO mule_daemon::kad: Tabla de enrutamiento Kad inicializada
INFO mule_daemon::services::organizer: Servicio de catalogación multimedia activo
```

### En Windows
Haz doble clic sobre `target\release\mule-daemon.exe` o ejecútalo desde PowerShell / CMD:
```powershell
.\target\release\mule-daemon.exe
```
El daemon arrancará e insertará automáticamente un icono en la **bandeja del sistema de Windows** (junto al reloj).

---

## 4. Acceso al Panel de Control Web (WebUI)

Con el daemon en marcha, abre tu navegador web preferido (Chrome, Firefox, Edge, Safari, Brave) y navega a:

> 🌐 **`http://127.0.0.1:4660`**

*(Si has instalado Pulse Core en un servidor doméstico o NAS, sustituye `127.0.0.1` por la dirección IP local de tu servidor en la red doméstica, por ejemplo `http://192.168.1.100:4660`).*

---

## 5. Configuración Inicial Recomendada

Al acceder por primera vez, te recomendamos revisar estos tres ajustes en la pestaña **Ajustes** (⚙️):

1. **Apodo (*Nickname*)**: Cambia el apodo predeterminado a tu alias favorito.
2. **Carpeta de Descargas (*Incoming*)**: Define la ruta de disco donde quieres que se depositen tus archivos completados (ej. `/media/disco/descargas` o `D:\Descargas`).
3. **Carpeta Temporal (*Temp*)**: Ubicación donde se almacenarán las partes a medias mientras descargan. Te recomendamos que esté en un disco con suficiente espacio libre.

¡Listo! Ya tienes Pulse Core preparado para funcionar. Pasa al siguiente manual para aprender a dominar cada pantalla de la interfaz web.

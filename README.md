<div align="center">

<img src="Asset/pulse-core-logo.svg" alt="Pulse Core logo" width="260"/>

# Pulse Core

**Reinvención moderna y asíncrona en Rust de las redes eMule (eD2K + Kad DHT) y Torrent, con automatización multimedia nativa e interfaz web integrada.**

[![Licencia: GPL-2.0-or-later](https://img.shields.io/badge/Licencia-GPL--2.0--or--later-blue.svg)](LICENSE)
[![Plataforma](https://img.shields.io/badge/Plataforma-Windows%20%7C%20Linux%20%7C%20aarch64-lightgrey)](README.md)
[![Rust: 1.80+](https://img.shields.io/badge/Rust-1.80%2B-orange.svg)](Cargo.toml)
[![Idioma](https://img.shields.io/badge/Idioma-Espa%C3%B1ol%20(ES--ES)-red)](README.md)

<br/>

[Características](#-qué-hace-diferente-a-pulse-core) • [Interfaz Web](#-interfaz-web-integrada) • [Puertos y Red](#-configuración-de-red-y-puertos) • [Arquitectura](#-arquitectura-del-workspace) • [Inicio Rápido](#-inicio-rápido) • [Reportar un error](#-antes-de-reportar-un-error)

</div>

---

> [![Aviso Importante](https://img.shields.io/badge/AVISO-IMPORTANTE-8a2be2.svg)](README.md)
>
> **Desarrollo técnico riguroso:** Pulse Core es una reimplementación limpia creada en simbiosis entre ingeniería humana experta y agentes de IA avanzados. La totalidad del código, los comentarios y la documentación están redactados en **español de España**, siguiendo estándares inflexibles de seguridad de memoria en Rust: sin pánicos en producción, sin advertencias del compilador (`-D warnings`), con fidelidad milimétrica a las fuentes de eMule 0.72a y suites de prueba que se ejecutan 100 % sin conexión a Internet.

El cliente original de eMule nació en 2002 sobre C++ y MFC para Windows de 32 bits, sufriendo los bloqueos de interfaz y las fugas de memoria típicas de la época. **Pulse Core** traslada toda esa herencia a la era moderna: un único binario ligero y asíncrono (`mule-daemon`) basado en **Tokio** y **Axum** que unifica los protocolos **eD2K**, la red DHT **Kademlia de 128 bits** y enjambres **Torrent** en un mismo bucle de eventos sin bloqueo.

Está especialmente diseñado para **servidores domésticos, microservidores, NAS (Synology, QNAP, TrueNAS, Unraid) y Raspberry Pi**: opera de manera 100 % *headless* (sin necesidad de monitor ni interfaz gráfica instalada en el sistema) con un consumo mínimo de apenas ~45 MB de RAM en reposo, administrable de forma remota y segura a través de su panel web embebido. En equipos de sobremesa con Windows 11, se integra de forma transparente en la bandeja del sistema.

> [![Advertencia](https://img.shields.io/badge/ADVERTENCIA-DESARROLLO%20ACTIVO-orange.svg)](README.md)
>
> **Proyecto en desarrollo activo y pruebas de enjambre:** El núcleo fundamental es 100 % operativo: el cálculo de identificadores eD2K (MD4 + AICH), la canalización anti-SSRF, el filtrado IP (`ipfilter.dat`), la interfaz web y la organización mediante enlaces duros están completamente probados y son estables. Actualmente estamos afinando la contención de ancho de banda en descargas concurrentes masivas y la convergencia de fuentes en enjambres híbridos.
>
> **Entornos validados:** Probado con éxito en estaciones de trabajo AMD Ryzen / Intel Core bajo Windows 10/11 x64, Linux x86_64 (Arch, Debian, Ubuntu) y procesadores ARM64 (Raspberry Pi 4 y 5 bajo Raspberry Pi OS, con consumos inferiores a ~45 MB de RAM en reposo).

> [![Consejo](https://img.shields.io/badge/CONSEJO-ACCESO%20R%C3%81PIDO-brightgreen.svg)](README.md)
>
> **Panel Web instantáneo sin dependencias:** No necesitas Node.js, Docker ni servidores HTTP externos para controlar Pulse Core. Inicia el daemon y abre de inmediato en tu navegador **`http://127.0.0.1:4660`** para supervisar transferencias en tiempo real mediante WebSockets, añadir enlaces `ed2k://` o magnet al vuelo y gestionar el catálogo multimedia sin latencia.
>
> Para colaborar o inspeccionar las decisiones de diseño, consulta [`docs/INDEX.md`](docs/INDEX.md) y [`AGENTS.md`](AGENTS.md). Recuerda que cualquier aportación debe superar `cargo test --workspace --all-features` y el linteado estricto sin advertencias.

> [![Nota](https://img.shields.io/badge/NOTA-EXENCI%C3%93N%20LEGAL-blue.svg)](README.md)
>
> **Neutralidad tecnológica absoluta:** Pulse Core **no incluye, no aloja, no indexa ni proporciona acceso a ningún archivo, película, serie o material con derechos de autor**. No suministra listas de servidores de terceros ni rastreadores preconfigurados. Es un motor de transporte de datos agnóstico y descentralizado; el uso y la legalidad del material descargado son responsabilidad exclusiva del usuario final.

---

## 🖥️ Interfaz Web integrada

El servicio incorpora un panel de control web reactivo y moderno servido directamente en el puerto `4660`, sin dependencias externas:

<div align="center">
  <img src="docs/screenshots/downloads.jpg" alt="Panel de control Web de Pulse Core" width="85%" style="border-radius: 8px; box-shadow: 0 4px 20px rgba(0,0,0,0.3);"/>
  <p><em>Panel de control reactivo en tiempo real servido nativamente por <code>mule-daemon</code> (Axum + WebSockets).</em></p>
</div>

---

## ⚡ Qué hace diferente a Pulse Core

### 1. Dos redes en un solo motor asíncrono
Descarga y comparte piezas simultáneamente entre clientes clásicos de eMule y enjambres modernos de Torrent. El asignador de almacenamiento disperso asigna espacio dinámicamente y verifica la integridad de cada fragmento mediante sumas de verificación jerárquicas eD2K y árboles AICH.

### 2. Organización y catalogación multimedia propia
Pulse Core incorpora su propio motor de gestión de medios sin depender de programas de terceros ni servicios externos:
- **Actualización y mejora progresiva de calidades**: Supervisa activamente el catálogo de medios local. Cuando se detecta en la red una versión con mayor resolución, mejor códec o calidad superior para una película o serie ya descargada, el motor la captura automáticamente, la descarga y actualiza la biblioteca sustituyendo el enlace de forma transparente.
- **Enlaces duros sin duplicar espacio (`hard_link`)**: Organiza y clasifica automáticamente las descargas finalizadas en la estructura requerida por gestores como Plex o Jellyfin (`Películas/Título (Año)/...`) sin consumir ni un solo megabyte extra en disco.
- **Sincronización de subtítulos**: Cliente REST integrado con OpenSubtitles (VLSub) que calcula el hash exacto de 64 bits (`primeros 64KB + últimos 64KB + tamaño`) para descargar y descomprimir el subtítulo `.es.srt` exacto y sincronizado al milisegundo.
- **Filtrado y puntuación inteligente**: Algoritmos propios de descarte de falsos positivos y asignación de prioridad por calidad e idioma.

### 3. Control por Inteligencia Artificial (Servidor MCP integrado)
Incorpora un servidor **MCP** (*Model Context Protocol*) con más de 85 herramientas expuestas. Puedes conectar asistentes de IA (como Antigravity, Claude Code o Gemini) para pausar transferencias, analizar la salud de las conexiones, optimizar colas o inspeccionar los registros del daemon mediante lenguaje natural.

### 4. Seguridad de red sin concesiones
- **Aislamiento Loopback y Anti-SSRF**: Ningún paquete malicioso puede forzar al daemon a escanear tu red local o comunicarse con servicios internos protegidos.
- **Filtrado dinámico de IPs (`ipfilter.dat`)**: Descarte inmediato a nivel de socket de rangos de direcciones sospechosas o maliciosas.
- **Interruptor de corte atómico (*kill switch*)**: Cierre y desconexión fulminante de todo el tráfico de red ante cualquier anomalía de seguridad.

---

## 🌐 Configuración de red y puertos

Para despliegues domésticos, servidores dedicados o contenedores NAS, estos son los puertos estándar utilizados por el motor:

| Puerto por defecto | Protocolo | Servicio | Función |
| :---: | :---: | :--- | :--- |
| **`4660`** | TCP | WebUI & JSON-RPC | Panel de control web en navegador y APIs de control local. |
| **`39735`** | TCP | Transferencias eD2K | Conexión con clientes eDonkey/eMule para descarga y subida. |
| **`39737`** | UDP | Kademlia (Kad DHT) | Búsqueda descentralizada de fuentes y enrutamiento global. |
| **`6881`** | TCP / UDP | Transferencias Torrent | Tráfico del enjambre Torrent y negociación de piezas. |

*(Todos los puertos son totalmente configurables en caliente o mediante variables de entorno).*

---

## 🏗️ Arquitectura del Workspace

El proyecto está organizado en un workspace de Cargo modular, donde cada componente tiene una única responsabilidad bien definida:

| Crate | Rol principal |
| :--- | :--- |
| **`crates/mule-daemon`** | Servidor principal Tokio/Axum, WebUI embebida, APIs RPC/WebSocket y bandeja de Windows (`src/tray.rs`). |
| **`crates/mule-protocol`** | Códecs de cable acotados eD2K/Kad con vectores de prueba estrictos basados en eMule 0.72a. |
| **`crates/mule-core`** | Dominio de negocio, gestor de descargas y motor de almacenamiento disperso. |
| **`crates/mule-torrent`** | Motor Torrent integrado y asignador de enjambre híbrido. |
| **`crates/mule-crypto`** | Primitivas criptográficas: MD4, árboles de hashing AICH e identificadores P2P. |
| **`crates/mule-mcp`** | Servidor MCP con más de 85 herramientas para interacción con asistentes y agentes de IA. |
| **`crates/mule-network-policy`** | Canalización de seguridad anti-SSRF, mitigación de ataques y aislamiento en bucle local. |
| **`crates/mule-filter`** | Filtrado de rangos IP maliciosos (`ipfilter.dat`) de alta velocidad. |
| **`crates/mule-format`** | Decodificación y serialización de formatos de archivo clásicos (`.met`, `.dat`). |

---

## 🚀 Inicio Rápido

### Requisitos previos
- **Rust** (`stable`, 1.80 o superior con `cargo`).
- Compilador C/C++ del sistema (MSVC en Windows, GCC o Clang en Linux).

---

### Compilación nativa con Cargo (Universal)

Puedes compilar el servicio en cualquier sistema operativo directamente con el gestor de paquetes y compilador de Rust:

```bash
# 1. Clonar el repositorio
git clone https://github.com/caraveliko/PulseCore.git
cd PulseCore

# 2. Compilar el daemon principal en modo Release
cargo build --release -p mule-daemon

# (Opcional) Compilar tanto el daemon como la CLI de control:
cargo build --release --bin mule-daemon --bin mule-cli
```

Una vez finalizada la compilación, ejecuta el binario generado:

- **En Linux / Raspberry Pi (`x86_64` / `aarch64`):**
  ```bash
  ./target/release/mule-daemon
  ```
- **En Windows (CMD o PowerShell):**
  ```powershell
  .\target\release\mule-daemon.exe
  ```

Una vez iniciado el servicio, accede a la interfaz desde cualquier navegador:
> 🌐 **`http://127.0.0.1:4660`**

---

### Empaquetado para distribución en Windows (Opcional)

Si deseas empaquetar una versión completa comprimida en `.zip` con scripts `.bat` de arranque automático y manifiestos SBOM:
```powershell
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\build_windows_server.ps1
```

---

## 🛠️ Antes de reportar un error

Para resolver cualquier problema con la mayor rapidez posible, comprueba lo siguiente antes de abrir una incidencia (*issue*):

1. **Reproducción en la rama principal**: Asegúrate de que el fallo persista en la última versión de la rama `main`.
2. **Captura de registros detallados**: Arranca el daemon activando los registros de depuración de Tokio y tracing:
   ```bash
   RUST_LOG=mule_daemon=debug,mule_protocol=debug,mule_core=debug ./target/release/mule-daemon
   ```
3. **Detalles del entorno**: Especifica tu sistema operativo (p. ej. *Windows 11 23H2* o *Ubuntu 24.04*), arquitectura de procesador (`x86_64` o `aarch64`), y el subsistema afectado (eD2K, Kad, Torrent, WebUI o Catalogación multimedia).
4. **Pruebas en bucle cerrado**: Si sospechas de un fallo en los códecs de cable, comprueba si pasa las pruebas locales:
   ```bash
   cargo test -p mule-protocol --all-features
   ```

---

## 📄 Licencia y Atribución

Este proyecto se distribuye bajo la licencia **GPL-2.0-or-later**. 
Consulta los archivos [`LICENSE`](LICENSE) y [`NOTICE`](NOTICE) para información sobre autoría original, licencias de terceros y referencias protocolarias.

---

## ⚖️ Exención de Responsabilidad Legal

1. **Finalidad técnica y de preservación**: **Pulse Core** es un proyecto de software libre con fines didácticos, de investigación y de preservación tecnológica de protocolos P2P históricos.
2. **Ausencia de catálogo**: El proyecto no gestiona, no promueve ni mantiene índices de contenido protegido ni servidores de intercambio de archivos.
3. **Responsabilidad exclusiva del usuario**: Corresponde al usuario asegurarse de que su utilización del software se ajusta a las leyes y regulaciones vigentes en su territorio en materia de telecomunicaciones y propiedad intelectual.
4. **Garantía**: Conforme a los términos de la licencia GPL-2.0-or-later, este software se suministra «TAL CUAL», sin ninguna garantía explícita ni implícita de rendimiento o adecuación a un fin particular.

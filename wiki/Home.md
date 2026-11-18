# Bienvenido a la Wiki de Pulse Core

<div align="center">
  <img src="https://raw.githubusercontent.com/caraveliko/PulseCore/master/Asset/pulse-core-logo.svg" alt="Pulse Core logo" width="220"/>
  <br/>
  <strong>Documentación técnica oficial de Pulse Core — Motor P2P asíncrono en Rust</strong>
</div>

---

> [![Aviso Importante](https://img.shields.io/badge/AVISO-DOCUMENTACI%C3%93N%20OFICIAL-8a2be2.svg)](Home)
>
> Toda la documentación técnica de **Pulse Core** se redacta y mantiene exclusivamente en **español de España**. Esta Wiki proporciona una referencia exhaustiva sobre arquitectura, configuración avanzada, especificaciones de cable e integración de automatizaciones para administradores de sistemas y desarrolladores.

---

## 🗺️ Mapa de Navegación de la Wiki

Explora las guías oficiales organizadas por ámbito técnico:

### 🚀 1. [Guía de Inicio y Despliegue en Servidores](01-Guia-de-Inicio-y-Servidor)
- Requisitos e instalación de la cadena de herramientas Rust.
- Compilación nativa universal con Cargo (`mule-daemon` y `mule-cli`).
- Despliegue *headless* en servidores Linux, microservidores y Raspberry Pi (`aarch64`).
- Configuración de servicios en segundo plano con `systemd` y persistencia con `loginctl enable-linger`.
- Integración en Windows 11 con bandeja del sistema nativa.

### ⚙️ 2. [Configuración y Ajustes del Sistema](02-Configuracion-y-Ajustes)
- Persistencia en base de datos SQLite y modificación en caliente vía JSON-RPC.
- Rutas del sistema: carpetas de descargas entrantes (`Incoming`), archivos temporales (`.part`) e importaciones.
- Presupuestos de red, límites de velocidad globales y por par, y gestión de cuotas.
- Tabla completa de parámetros de configuración y equivalencias con eMule clásico.

### 🌐 3. [Protocolos P2P: eD2K, Kad DHT y Torrent](03-Protocolos-P2P)
- Arquitectura del bucle de eventos unificado sin bloqueo.
- Especificación en cable eD2K: entramado acotado, compresión zlib (`OP_COMPRESSEDPART`) e intercambio de fuentes (SX1/SX2).
- Red Kademlia DHT de 128 bits: enrutamiento XOR, contactos, árbol de búsqueda y bootstrap descentralizado.
- Motor Torrent integrado: negociación de enjambres, árboles AICH y hashing de dos niveles.

### 🎬 4. [Organización y Catalogación Multimedia](04-Organizacion-Multimedia)
- Organización automática para centros multimedia (Plex y Jellyfin) mediante enlaces duros (`hard_link`) sin consumo de disco adicional (*Zero-Copy*).
- Detección y actualización progresiva de calidades (*Quality Upgrades*) de forma desatendida.
- Sincronización de subtítulos con OpenSubtitles mediante cálculo de hash de 64 bits (`first 64KB + last 64KB + size`).
- Filtrado inteligente de nombres de archivo y algoritmos de puntuación de lanzamientos.

### 🤖 5. [API RPC y Servidor MCP para Inteligencia Artificial](05-API-RPC-y-Servidor-MCP)
- Especificación del protocolo JSON-RPC 2.0 sobre WebSockets (`ws://127.0.0.1:4660/rpc`).
- Catálogo de métodos RPC: descargas, compartidos, servidores, búsqueda e historial.
- Servidor MCP (*Model Context Protocol*) con más de 85 herramientas expuestas para asistentes de IA y agentes autónomos.
- Ejemplos prácticos de automatización en Python y llamadas CLI.

### 🛡️ 6. [Seguridad de Red y Mitigación de Abusos](06-Seguridad-y-Red)
- Canalización de validación estricta y aislamiento anti-SSRF.
- Filtrado dinámico de IPs maliciosas mediante listas de bloqueo (`ipfilter.dat`).
- Interruptor de corte atómico (*kill switch*) para desconexión instantánea ante anomalías.
- Aislamiento estricto de bucle local (*loopback isolation*).

---

## 📌 Enlaces Rápidos y Recursos

- **Repositorio oficial**: [https://github.com/caraveliko/PulseCore](https://github.com/caraveliko/PulseCore)
- **Panel Web del Daemon**: `http://127.0.0.1:4660` (cuando el servicio está en ejecución)
- **Licencia**: [GPL-2.0-or-later](../LICENSE)

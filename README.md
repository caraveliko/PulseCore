<div align="center">

<img src="Asset/pulse-core-logo.jpg" alt="Pulse Core logo" width="160"/>

# Pulse Core

**Cliente P2P híbrido en Rust que combina las redes eMule (eD2K + Kad DHT) y BitTorrent, con interfaz web integrada y herramientas de organización multimedia.**

[![License: GPL-2.0-or-later](https://img.shields.io/badge/License-GPL--2.0--or--later-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Plataforma-Windows%20%7C%20Linux-lightgrey)](README.md)
[![Language](https://img.shields.io/badge/Idioma-Espa%C3%B1ol%20(ES--ES)-red)](README.md)

</div>

---

## ¿Qué es Pulse Core?

**Pulse Core** es un cliente P2P moderno y asíncrono (Tokio) escrito en **Rust** que unifica los protocolos **eD2K**, **Kademlia DHT (Kad)** y **BitTorrent** en un único servicio (`mule-daemon`).

Está diseñado tanto para su uso en estaciones de trabajo como en servidores domésticos (*headless*), incorporando una interfaz web ligera servida directamente por el ejecutable y utilidades locales para catalogar y organizar descargas.

---

## Características

- **Soporte multiprotocolo**: Descarga simultáneamente desde redes eD2K / Kad y enjambres BitTorrent mediante un único proceso.
- **Fidelidad protocolaria eD2K/Kad**: Soporte de compresión zlib (`OP_COMPRESSEDPART`), intercambio de fuentes (SX1/SX2) y DHT Kademlia v2 de 128 bits.
- **Interfaz Web integrada**: Panel de control accesible en `http://127.0.0.1:4660` sin necesidad de servidores web externos, con control de velocidad (Modo Tortuga).
- **Organización multimedia**: Clasificación automática para gestores como Plex o Jellyfin (`Películas/Título (Año)/...`) mediante enlaces duros (`hard_link`) para ahorrar espacio en disco, con descarga opcional de subtítulos vía OpenSubtitles.
- **Filtrado y puntuación configurable**: Reglas de descarte de falsos positivos y prioridad por calidad o idioma.
- **Seguridad de red**: Canalización con validación anti-SSRF, filtrado de IPs maliciosas (`ipfilter.dat`) y aislamiento en bucle local.
- **Control e interoperabilidad**: Servidor MCP (*Model Context Protocol*) integrado para automatización e integración con asistentes.

---

## Inicio Rápido

### Requisitos
- **Rust** (`stable`, 1.80 o superior)

### Compilación y ejecución

**En Linux:**
```bash
cargo build --release -p mule-daemon
./target/release/mule-daemon
```

**En Windows:**
```powershell
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\build_windows_server.ps1
```
*(O compilar con `cargo build --release -p mule-daemon`).*

Una vez iniciado el daemon, abre en el navegador:
> **`http://127.0.0.1:4660`**

---

## Licencia y Atribución

Distribuido bajo licencia **GPL-2.0-or-later** — consulta los archivos [`LICENSE`](LICENSE) y [`NOTICE`](NOTICE) para detalles de atribución y referencias protocolarias.

---

## ⚖️ Aviso Legal / Legal Disclaimer

> **EXENCIÓN DE RESPONSABILIDAD**
>
> **Pulse Core** es un proyecto de software libre de carácter técnico y educativo que implementa un cliente para protocolos estándar de comunicación entre pares (*peer-to-peer*: eD2K, Kademlia y BitTorrent), junto con herramientas locales de organización de archivos.
>
> 1. **Ausencia de contenido**: Este software **no aloja, almacena, indexa, distribuye, enlaza ni proporciona acceso** a ningún archivo, película, serie, obra multimedia o material protegido por derechos de autor. No incluye servidores propios, rastreadores (*trackers*) ni catálogos de contenido de terceros.
> 2. **Neutralidad tecnológica**: Las funciones de búsqueda y organización operan exclusivamente sobre redes descentralizadas públicas o indexadores externos configurados libremente por el propio usuario. Los autores y colaboradores del proyecto no supervisan ni tienen conocimiento alguno sobre los archivos gestionados por los usuarios.
> 3. **Responsabilidad del usuario**: El uso que se haga de esta herramienta recae **única y exclusivamente sobre el usuario final**, quien debe asegurarse de contar con la debida autorización y cumplir la legislación aplicable en materia de propiedad intelectual.
> 4. **Sin garantías**: Este programa se distribuye «TAL CUAL» (*AS IS*), sin garantías de ningún tipo, conforme a los términos de la licencia GPL-2.0-or-later.

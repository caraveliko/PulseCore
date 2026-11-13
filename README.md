<div align="center">

<img src="Asset/pulse-core-logo.jpg" alt="Pulse Core logo" width="160"/>

# Pulse Core

**Cliente P2P híbrido único que combina las redes BitTorrent y eMule (eD2K + Kad DHT) en un único motor de alto rendimiento escrito en Rust, con automatización multimedia y especialización en Español de España.**

[![License: GPL-2.0-or-later](https://img.shields.io/badge/License-GPL--2.0--or--later-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Plataforma-Windows%20%7C%20Linux-lightgrey)](README.md)
[![Language](https://img.shields.io/badge/Idioma-Espa%C3%B1ol%20(ES--ES)-red)](README.md)

</div>

---

## ¿Qué es Pulse Core?

**Pulse Core** es un cliente y servidor P2P híbrido moderno, seguro y unificado que integra los protocolos **eD2K (eDonkey2000)**, **Kademlia DHT (Kad)** y **BitTorrent** en un único motor desarrollado en **Rust** con arquitectura asíncrona (Tokio).

A diferencia de los clientes tradicionales, nace como un **cliente híbrido integral** pensado para funcionar tanto en escritorio como en modo servidor/headless, con una **interfaz web embebida moderna** y herramientas nativas de organización multimedia **especializadas en el idioma español de España (Castellano)**.

---

## Características Principales

### 🔀 Cliente P2P Híbrido Único (eMule + BitTorrent)
- **Motor unificado en Rust**: Gestiona descargas simultáneas de las redes **eD2K / Kad DHT** y **BitTorrent** desde un único daemon (`mule-daemon`) con gestión segura de memoria y almacenamiento disperso eficiente.
- **Compatibilidad protocolaria eD2K/Kad**: Implementación fiel de tramas, compresión zlib al vuelo (`OP_COMPRESSEDPART`), intercambio de fuentes (SX1/SX2) y tabla de enrutamiento Kademlia v2 de 128 bits.

### 🇪🇸 Especialización en Español de España (Castellano)
- **Localización y filtrado estricto**: Motor de búsqueda y puntuación (*Scorer*) diseñado específicamente para detectar, priorizar y filtrar lanzamientos en **Español de España**, descartando automáticamente falsos positivos, idiomas no deseados o coincidencias erróneas.
- **Interfaz 100% en Castellano**: Tanto la interfaz web embebida como la aplicación de escritorio están adaptadas al español.

### 🎬 Automatización Multimedia Integrada (Estilo Servarr)
- **Gestión de Películas y Series**: Catálogo multimedia persistente en SQLite para organizar y monitorizar títulos completos o episodios pendientes.
- **AutoGrabber inteligente**: Rastreo y captura automática mediante indexadores compatibles con **Torznab / Prowlarr** y búsquedas en la red **Kad/eD2K**.
- **Organizador compatible con Plex y Jellyfin**: Clasificación automática (`Movies/Título (Año)/...`) mediante enlaces duros (`hard_link`) instantáneos sin duplicar espacio en disco.
- **Subtítulos automáticos**: Cliente REST integrado para OpenSubtitles con cálculo de hash de 64 bits, descompresión y normalización UTF-8 (`.es.srt`).

### 🖥️ Interfaz Web Embebida, Modo Tortuga y Control IA
- **WebUI embebida sin dependencias**: Al arrancar `mule-daemon`, sirve automáticamente su panel de control web completo en `http://127.0.0.1:4660`.
- **Modo Tortuga**: Limitación rápida de velocidad de subida/bajada con un solo clic para no saturar tu red doméstica cuando lo necesites.
- **Servidor MCP nativo**: Expone más de 87 herramientas mediante *Model Context Protocol* para automatización y control mediante asistentes de IA.
- **Seguridad reforzada**: Pipeline anti-SSRF, aislamiento loopback por defecto, soporte de listas de bloqueo IP (`ipfilter.dat`) y utilidades de migración desde aMule/eMule.

---

## Inicio Rápido

### Compilar y ejecutar el Daemon + WebUI (Windows o Linux)

#### Requisitos
- **Rust** (`stable`, 1.80+)

#### Compilación e inicio en Servidor Windows
```powershell
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\build_windows_server.ps1
```
*(Genera la carpeta `dist\PulseCore-Server-Windows\` con los ejecutables y lanzadores para iniciar en primer o segundo plano).*

Al ejecutar `mule-daemon` (o `Iniciar Servidor.bat` en Windows), abre tu navegador en:
> **`http://127.0.0.1:4660`**

---

## Licencia y Atribución

Distribuido bajo licencia **GPL-2.0-or-later** — consulta los archivos [`LICENSE`](LICENSE) y [`NOTICE`](NOTICE) para ver los detalles de atribución del código base original y las referencias protocolarias.

---

## ⚖️ Aviso Legal / Legal Disclaimer

> **EXENCIÓN DE RESPONSABILIDAD**
>
> **Pulse Core** es un proyecto de software libre de carácter estrictamente técnico y educativo que implementa un cliente neutral para protocolos estándar de comunicación entre pares (*peer-to-peer*: eD2K, Kademlia y BitTorrent), junto con herramientas locales de organización de archivos.
>
> 1. **Ausencia de contenido**: Este software **no aloja, almacena, indexa, distribuye, enlaza ni proporciona acceso** a ningún archivo, película, serie, obra multimedia o material protegido por derechos de autor. No incluye servidores propios, rastreadores (*trackers*) ni catálogos de contenido de terceros.
> 2. **Neutralidad tecnológica**: Las funciones de búsqueda, filtrado lingüístico y automatización operan exclusivamente sobre redes descentralizadas públicas o indexadores externos configurados libremente por el propio usuario. Los autores y colaboradores del proyecto no tienen control, supervisión ni conocimiento alguno sobre los archivos que cada usuario busca, descarga o comparte.
> 3. **Responsabilidad exclusiva del usuario**: El uso que se haga de esta herramienta recae **única y exclusivamente sobre el usuario final**, quien es el único responsable de asegurarse de que los contenidos gestionados cuentan con la debida autorización o pertenecen al dominio público, así como de cumplir con la legislación vigente en materia de propiedad intelectual y derechos de autor aplicable en su país o jurisdicción.
> 4. **Sin garantías**: Este programa se distribuye «TAL CUAL» (*AS IS*), sin garantías de ningún tipo, expresas o implícitas, conforme a los términos de la licencia GPL-2.0-or-later. Los desarrolladores no asumen responsabilidad alguna por cualquier daño directo, indirecto o consecuencia legal derivada del uso o mal uso de este software.

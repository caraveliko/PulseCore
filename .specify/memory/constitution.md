# Constitución de Pulse Core (`eMule-Rust`)
<!-- Gobernanza de desarrollo dirigido por especificaciones para Pulse Core (eMule-Rust) -->

## Principios fundamentales

### I. Desarrollo dirigido por especificaciones (NO NEGOCIABLE)
Toda funcionalidad, cambio arquitectónico o extensión de protocolo DEBE comenzar con una especificación formal (`.specify/specs/` o `specs/`) antes de escribir código de implementación.
El ciclo de vida es estrictamente:
1. **Especificar**: clarificar recorridos de usuario, límites de protocolo, modelos de datos y criterios de aceptación.
2. **Planificar**: diseño de arquitectura, validación de dependencias y análisis de riesgos.
3. **Tareas**: unidades de implementación granulares y verificables.
4. **Implementar**: ejecución de código ciñéndose estrictamente al contrato.
5. **Converger**: verificación de integración frente a las especificaciones y pruebas de regresión.

### II. Servidor multiplataforma sin sobrecarga (Linux *Headless* / SBC y Windows en segundo plano)
- El daemon principal (`mule-daemon`) debe funcionar sin fallos en Linux *headless* (`x86_64` y `ARM64` / Raspberry Pi) como servicio `systemd`, así como en Windows 10/11 (`x64`) en segundo plano sin ventana de consola (`windows_subsystem = "windows"` en compilaciones *release*) acompañado únicamente de su icono nativo en la bandeja del sistema (`crates/mule-daemon/src/tray.rs`).
- El uso de RAM debe mantenerse estrictamente acotado (< 100 MB en estado estacionario frente a los 2,6 GB de aMule).
- Sin dependencias de interfaz gráfica de escritorio en el daemon backend. Toda interacción se realiza mediante WebSockets asíncronos, API REST o CLI.

### III. Interoperabilidad del ecosistema (*arr y *homelabs* modernos)
- Exponer APIs modernas HTTP/REST y WebSocket.
- Proporcionar una capa de compatibilidad que emule **Transmission RPC** o **qBittorrent WebAPI**, permitiendo que Sonarr, Radarr y Prowlarr gestionen descargas eD2K/Kad de forma fluida sin scripts externos frágiles.

### IV. Interfaz web embebida (Moderna y responsiva)
- Servir una interfaz web ligera y rápida directamente desde `mule-daemon` (mediante `axum` + WebSockets).
- Debe funcionar sin problemas en navegadores móviles, tabletas y escritorio sin requerir entornos de ejecución de escritorio pesados.
- En Windows, la única interfaz local complementaria permitida es un icono nativo en la barra de tareas (`crates/mule-daemon/src/tray.rs`) con las funciones esenciales (abrir la página web de la WebUI, abrir carpetas de descargas/datos y detener el servidor).

### V. Inteligencia de enjambre y colas equitativas
- Mantener conexiones con colas de pares remotos sin pausas por fuerza bruta.
- Implementar asignación ponderada *max-min fair share* y graduación automática de prioridad (los archivos cercanos al 100 % o con fuentes raras reciben las solicitudes de fragmentos prioritarias).
- Descubrimiento autónomo de fuentes en Kademlia v2 DHT con comprobaciones periódicas de salud.

### VI. Pruebas primero y conformidad protocolaria
- Análisis de cable acotado: cada trama de protocolo debe tener límites estrictos de asignación para evitar OOM / DoS.
- Pruebas exhaustivas de integración que comparen el comportamiento en cable frente a la referencia oficial eD2K / eMule 0.72a.
- Pruebas basadas en propiedades (`proptest`) para codificadores/decodificadores binarios de cable.

### VII. Idioma único (Español de España)
- Todas las especificaciones (`specs/`), documentos Markdown (`.md`) e *Inner Doc Comments* (`//!`) de los archivos que componen el programa deben estar redactados exclusivamente en **español de España**.

## Pila tecnológica y estándares
- **Lenguaje**: Rust (Edición 2021+, estrictamente seguro en memoria, `#![deny(unsafe_code)]` donde proceda).
- **Entorno de ejecución asíncrono**: Tokio + Axum (HTTP/WebSockets) + Rusqlite (SQLite WAL ACID).
- **Plataformas objetivo**: Linux (`x86_64`, `aarch64` / Raspberry Pi) y Windows 10/11 (`x64`).
- **Formatos de API**: JSON-RPC 2.0 (protocolo nativo del daemon), REST (emulación Transmission / Torznab), MCP (herramientas para agentes de IA).

## Gobernanza y puertas de calidad
- Todas las incorporaciones de funcionalidades requieren una especificación y una lista de comprobación aceptadas.
- CI / `cargo test --workspace` debe pasar sin advertencias de Clippy.
- Los cambios en el esquema de estado deben incluir migraciones SQLite retrocompatibles.

**Versión**: 1.0.0 | **Ratificada**: 2026-10-29 | **Estado**: Activa

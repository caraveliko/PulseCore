# Notas de la Versión — Pulse Core v0.4.5

**Fecha de publicación:** 21 de noviembre de 2026  
**Estado:** Versión `0.4.5` — **Fusión de Filmoteca y Descargas**, **compatibilidad Media Center completa (Jellyfin, Emby, Plex, Kodi e Infuse) + Modo Simple**, **deduplicación entre carpetas con vista previa** y **temas lisos en la WebUI**.

---

## 1. Síntesis de la Versión

La versión **v0.4.5** da un salto cualitativo en experiencia de usuario y compatibilidad multimedia. Unifica **Descargas** y **Filmoteca** en una sola pantalla con tres sub-vistas (**Cartelera**, **Cola** y **Calendario**), incorpora temas de colores lisos (incluido **Negro Puro**), rediseña la sección de **Compartidos** con telemetría y vista por carpetas, y añade control completo en **Opciones** tanto para usuarios de **Media Center** (Jellyfin, Plex, Kodi, Infuse) como para quienes prefieren descargas directas en **Modo Simple**.

---

## 2. Novedades y Mejoras Principales

### Interfaz Web (`web/index.html`, `web/assets/theme.css`, `web/assets/atmosphere.js`)
- **Fusión Filmoteca + Descargas**: barra superior simplificada a 4 pestañas (`Filmoteca`, `Red P2P`, `Compartidos`, `Opciones`). En **Cartelera**, una banda dinámica muestra las transferencias activas en curso y permite saltar a la **Cola** técnica e Inspector con un clic.
- **Temas con colores lisos y marca renovada**: selector renombrado a **Temas** con 4 nuevos modos sólidos (*Negro Puro*, *Grafito*, *Azul Noche*, *Esmeralda Oscuro*), logotipo **Cristal + Pulso Vivo** adaptado al color del tema y botón CTA destacado **Añadir Título**.
- **Compartidos rediseñado**: 4 tarjetas KPI en vivo, buscador instantáneo, filtros por tipo/estado, ordenación por columnas y conmutador de vista **Lista / Carpetas**.
- **Guía local integrada y limpieza visual**: el panel de ayuda funciona 100 % en local sin conectarse a modelos LLM externos (derivando la automatización con IA al servidor `mule-mcp`), se eliminan columnas redundantes y se refuerza la contención de texto en *tooltips* y tablas.

### Motor y Compatibilidad Media Center (`crates/mule-daemon`)
- **Modo Simple vs Modo Media Center (`organizeEnabled`)**: permite desactivar la organización automática para conservar los vídeos con su nombre original en la carpeta de descargas.
- **Perfiles Jellyfin / Plex / Kodi (`mediaServerProfile`)**: formatea carpetas y etiquetas según el servidor (`[tmdbid-...]`, `{tmdb-...}` o limpio para Kodi/Infuse), con generación opcional de `movie.nfo` / `tvshow.nfo`, `poster.jpg` / `fanart.jpg` y refresco de biblioteca en Jellyfin/Emby, Plex y Kodi.
- **Deduplicación avanzada y Upgrade de calidad**: detección de películas duplicadas entre carpetas distintas, limpieza de entradas huérfanas en `shared_files`, ajustes `qualityUpgradeEnabled` / `qualityMaxResolution` y **Detector de duplicados con vista previa (`dryRun`)** en **Opciones**.

---

## 3. Verificación e Integridad

Comprobación de sintaxis JavaScript (`node --check`), `cargo clippy -p mule-daemon --all-targets --all-features -- -D warnings`, pruebas unitarias de `cleaner` y `organizer`, pruebas `transmission_rpc_tests` y despliegue previo validado en Raspberry Pi (`aarch64-unknown-linux-gnu`) antes de publicar la etiqueta `v0.4.5` para **Windows x64**, **Linux x64** y **Linux ARM64 (`aarch64`)**.

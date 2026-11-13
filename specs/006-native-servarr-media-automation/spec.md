# Especificación de funcionalidad: Automatización nativa de Servarr y medios (`mule-daemon`)

**Rama de funcionalidad**: `006-native-servarr-media-automation`
**Creado**: 2026-11-03
**Estado**: Implementado

## 1. Descripción general y propósito

`mule-daemon` integra de forma nativa capacidades de automatización multimedia equivalentes a **Prowlarr** (agregación de indexadores), **Radarr** (catálogo y capturador de películas), **Sonarr** (catálogo y capturador de series/episodios) y **Bazarr** (hash de 64 bits de OpenSubtitles y automatización de subtítulos), unificadas con organización del sistema de archivos sin copias para **Jellyfin** / **Plex**.

Todas las rutas del sistema de archivos y los endpoints o tokens de servicios externos son estrictamente configurables mediante `DaemonConfig` (`crates/mule-daemon/src/config.rs`), `settings` en SQLite (`KNOWN_SETTINGS_KEYS` en `crates/mule-daemon/src/rpc/handlers/settings.rs`) y variables de entorno `EMULERUST_*`, sin ninguna ruta ni credencial fija en el código fuente.

## 2. Componentes principales

1. **Biblioteca multimedia (`crates/mule-daemon/src/services/media.rs`)**:
   - Catálogo respaldado por SQLite (`media_library`, `media_episodes`, `indexers`) que almacena películas, series, episodios, perfiles de calidad y estado de supervisión.
2. **Puntuador de lanzamientos y autocapturador (`crates/mule-daemon/src/services/scorer.rs`, `auto_grabber.rs`, `indexer_aggregator.rs`)**:
   - Agregación de indexadores Torznab y públicos (`search.all`, `indexer.*`).
   - Coincidencia estricta de títulos (`matches_release`) con tolerancia de año $\pm 1$, puntuación por perfil de calidad y preferencia de audio en español/multiaudio.
   - Respeta el interruptor de emergencia de `NetworkPolicyEngine` (`NetworkState::Disabled`).
3. **Organizador multimedia sin copias (`crates/mule-daemon/src/services/organizer.rs`, `disk_scanner.rs`)**:
   - Organiza las descargas completadas en `<movies_dir>/<Title (Year)>/...` y `<series_dir>/<Title>/Season XX/...` usando `std::fs::hard_link` con respaldo a `std::fs::copy` en movimientos entre dispositivos (`EXDEV`).
   - Notificaciones opcionales de actualización de biblioteca para Jellyfin y Plex cuando están configurados `jellyfin_url`/`jellyfin_api_key` o `plex_url`.
4. **Servicio complementario OpenSubtitles (`crates/mule-daemon/src/services/subtitles.rs`)**:
   - Calcula el hash de 64 bits de OpenSubtitles (`first 64 KiB + last 64 KiB + file size`) y consulta la API REST de OpenSubtitles (`subtitles.search`, `subtitles.download`, `subtitles.autoDownload`, `subtitles.check`), guardando archivos `.srt` normalizados en UTF-8.

## 3. Configuración y variables de entorno

| Clave de ajuste (`settings.set`) | Variable de entorno | Valor por defecto |
| :--- | :--- | :--- |
| `nasRootDir` | `EMULERUST_NAS_PATH` | `None` |
| `moviesDir` | `EMULERUST_MOVIES_DIR` | `<default_download_dir>/media/Movies` |
| `seriesDir` | `EMULERUST_SERIES_DIR` | `<default_download_dir>/media/Series` |
| `sonarrUrl` / `sonarrApiKey` | `EMULERUST_SONARR_URL` / `EMULERUST_SONARR_API_KEY` | `None` |
| `radarrUrl` / `radarrApiKey` | `EMULERUST_RADARR_URL` / `EMULERUST_RADARR_API_KEY` | `None` |
| `jellyfinUrl` / `jellyfinApiKey` | `EMULERUST_JELLYFIN_URL` / `EMULERUST_JELLYFIN_API_KEY` | `None` |
| `plexUrl` | `EMULERUST_PLEX_URL` | `None` |
| `nasSshTarget` / `nasSshPort` | `EMULERUST_NAS_SSH_TARGET` / `EMULERUST_NAS_SSH_PORT` | `None` / `22` |

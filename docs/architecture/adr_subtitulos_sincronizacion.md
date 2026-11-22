# ADR: Subtítulos precisos, validados y sincronizados

- **Estado**: Aceptado
- **Fecha**: 2026-11-22
- **Ámbito**: `crates/mule-daemon/src/services/subtitles/`, `crates/mule-daemon/src/services/media_probe.rs`, `crates/mule-daemon/src/services/organizer.rs`, `crates/mule-core/src/persistence/sqlite/`

---

## 1. Contexto y problema

La implementación inicial del motor de subtítulos en `mule-daemon` presentaba cinco fallos estructurales que provocaban la descarga de subtítulos incorrectos o desincronizados:

1. **Búsqueda por IMDb ID sin temporada ni episodio**: en series, la consulta `imdbid-{id}` omitía `season-{s}/episode-{e}`, devolviendo subtítulos de cualquier capítulo.
2. **Hash incompleto**: aunque se calculaba el tamaño del vídeo, `moviebytesize-{size}` no se incluía en la URL de OpenSubtitles.
3. **Selección por popularidad bruta**: sin coincidencia de hash, se elegía el candidato con mayor número de descargas sin evaluar grupo de lanzamiento, fuente (`WEB-DL`, `BluRay`, `HDTV`), FPS ni variante de español (`es-ES` frente a `es-LA`).
4. **Pérdida del nombre original de *release***: al organizar el archivo en `Movies/...` o `TV Shows/...`, se perdía el nombre original del torrent o descarga eD2K, impidiendo emparejar la *release* exacta en búsquedas posteriores.
5. **Ausencia de validación y sincronización post-descarga**: no se comprobaba que el archivo descargado fuese un `.srt` válido ni que su duración encajara con el vídeo, ni existía ajuste de FPS, alineación contra pistas embebidas ni lista negra de subtítulos defectuosos.

---

## 2. Decisión arquitectónica

### 2.1 Preservación de `original_release_name`
- `MediaItem` y `EpisodeItem` (`mule-core::model`) incorporan el campo opcional `original_release_name`.
- Al organizar una descarga completada en `MediaOrganizer`, el nombre del archivo de origen se persiste en SQLite (`media_library` / `media_episodes`) antes de ejecutar la búsqueda de subtítulos.

### 2.2 Inspección multimedia ampliada (`MediaProbeInfo`)
`ffprobe` (`-show_streams -show_format`) extrae en una sola pasada:
- `fps_milli: Option<u32>`: frecuencia de fotogramas en milésimas (p. ej., `23976`, `24000`, `25000`, `29970`).
- `duration_secs: Option<u64>`: duración total del contenedor en segundos.
- `embedded_text_sub_stream: Option<usize>`: índice relativo (`0:s:<idx>`) de una pista de subtítulos textual extraíble (`subrip`, `ass`, `ssa`, `mov_text`, `webvtt`) para usarla como referencia de sincronización.

### 2.3 Búsqueda multi-estrategia y extensible
La búsqueda combina resultados de:
1. **Hash exacto**: `moviebytesize-{size}/moviehash-{hash}/sublanguageid-{lang}`.
2. **IMDb ID**: `imdbid-{id}[/season-{s}/episode-{e}]/sublanguageid-{lang}`.
3. **Consulta textual**: `query-{title}[/season-{s}/episode-{e}]/sublanguageid-{lang}`.
4. **OpenSubtitles.com REST API v1** (opcional, si `opensubtitlesApiKey` está configurada).

Los resultados se deduplican por identificador de subtítulo conservando la mejor estrategia de coincidencia.

### 2.4 Motor de puntuación (*scoring*) y descartes duros
Cada candidato recibe una puntuación determinista evaluada por `subtitles::scorer`:

| Criterio | Efecto |
|---|---|
| `SubBad == "1"` o formato no textual | **Descarte duro** |
| Temporada/episodio explícitos distintos al objetivo | **Descarte duro** |
| `is_hi == true` con `subtitlesAllowHi == false` | **Descarte duro** |
| Variante explícita `es-LA`/`latino` con `subtitlesVariant == "es-ES"` (o viceversa) | **Descarte duro** |
| Coincidencia exacta `moviehash` + `moviebytesize` | `+100` |
| Coincidencia de grupo de *release* (`FLUX`, `NTb`, `SPARKS`, etc.) | `+40` |
| Coincidencia de fuente (`WEB-DL`, `WEBRip`, `BluRay`, `REMUX`, `HDTV`) | `+30` |
| Coincidencia de servicio de *streaming* (`AMZN`, `NF`, `DSNP`, `ATVP`, `HMAX`) | `+15` |
| Coincidencia de edición (`Extended`, `Director's Cut`, `Unrated`, `REPACK`) | `+25` (penalización `-50` si hay conflicto) |
| Coincidencia de FPS (`|sub_fps - video_fps| <= 100` milésimas) | `+20` (penalización `-25` si difieren) |
| Coincidencia de variante de idioma (`es-ES` / `es-LA`) | `+20` |
| Subtítulo marcado como de confianza (`SubFromTrusted`) | `+10` |
| Traducción automática (`SubAutoTranslation`) | `-30` |
| Popularidad (logarítmica / acotada) | hasta `+10` |

En descarga automática (`auto_fetch_for_video`), ningún candidato por debajo de `subtitlesMinScore` (por defecto `40`) se descarga.

### 2.5 Validación sintáctica, saneamiento y lista negra
Antes de guardar un `.srt` en disco (`subtitles::srt`):
1. Descompresión gzip y normalización Windows-1252 / ISO-8859-1 a UTF-8 sin BOM.
2. Análisis sintáctico SRT exigiendo al menos `MIN_VALID_SRT_CUES` (`10`) bloques válidos y rechazando HTML/JSON.
3. Comprobación temporal frente a `duration_secs` del vídeo (si se conoce): se rechaza si el último *cue* supera la duración del vídeo + margen (`300 s`) o es inferior al `15 %` en vídeos de más de 10 minutos.
4. Heurística ligera de idioma para evitar subtítulos mal etiquetados en otro idioma.
5. Saneamiento opcional de marcas HI y anuncios de portales.

La tabla SQLite `subtitle_blacklist` persiste los pares `(media_key, provider, provider_sub_id)` rechazados manualmente o mediante `subtitles.blacklistAndRetry` para no volver a descargarlos.

### 2.6 Cascada de sincronización post-descarga (`subtitles::sync`)
Cuando el candidato elegido no proviene de una coincidencia exacta por `moviehash` y `subtitlesAutoSync != "off"`:
1. **Reescalado determinista de FPS**: si el subtítulo declara un FPS distinto al `fps_milli` real del vídeo (p. ej. `25000` frente a `23976`), se reescalan todos los *timestamps* (`t' = t * sub_fps / video_fps`).
2. **Alineación sub-a-sub contra pista embebida** (si `subtitlesAutoSync == "embedded_sub"` y `embedded_text_sub_stream` está disponible):
   - Se extrae la pista textual embebida mediante `ffmpeg` bajo un semáforo global de concurrencia `1` (protección de CPU/RAM en Raspberry Pi `aarch64`).
   - Se estima el desplazamiento óptimo (`offset_ms`) mediante histograma de diferencias de inicios de *cues* + refinamiento fino, y solo se aplica si la mejora de solapamiento supera el umbral mínimo configurado.
3. **Ajuste manual**: el método RPC `subtitles.sync` permite aplicar desplazamientos puntuales (`offsetMs`) o conversiones de FPS sobre el archivo `.srt` existente desde la WebUI.

---

## 3. Restricciones operativas y de pruebas
- Todas las pruebas unitarias e de integración deben ejecutarse **100 % sin conexión**, sin emitir peticiones HTTP reales.
- Todos los umbrales, pesos y límites viven en constantes documentadas al inicio de cada módulo o en `settings` SQLite.
- El espacio de nombres `subtitles.*` respeta las reglas de seguridad anti-SSRF y la política «Solo Vídeo».

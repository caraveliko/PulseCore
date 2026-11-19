# ADR 0006: Política de Contenido Estricta «Solo Vídeo» (Opción E1)

## Estado
Aceptado

## Fecha
2026-11-19

## Contexto
**Pulse Core** (`eMule-Rust`) está diseñado específicamente para gestionar, descargar, indexar y organizar contenido multimedia audiovisual (películas y series). Permitir la descarga, compartición o indexación indiscriminada de ejecutables (`.exe`, `.msi`, `.sh`, `.ps1`, `.apk`), archivos comprimidos, imágenes de disco (`.iso`, `.img`), documentos o archivos con doble extensión engañosa (`pelicula.exe.mkv`) introduce dos riesgos innecesarios:

1. **Desviación del propósito de la aplicación**: uso del nodo para distribuir software arbitrario o archivos no multimedia.
2. **Superficie de riesgo e inseguridad**: riesgo de envenenamiento o malware camuflado con extensión de vídeo o dentro de torrents multifichero acompañados de binarios ejecutables o texto basura.

Se evaluaron distintas alternativas de aplicación, priorizando la solución con **menor consumo de CPU, RAM y operaciones de E/S en disco**, sin dependencias de procesos externos (`ffprobe`/`libmagic`) y con garantía de defensa en profundidad imposible de desactivar en tiempo de ejecución.

## Decisión
Adoptar de forma fija en tiempo de compilación la **Política de Contenido «Solo Vídeo» (Opción E1)** mediante una única fuente de verdad y **cuatro puertas defensivas** en todos los motores del daemon:

### 1. Fuente única de verdad (`mule_network_policy::content_policy`)
Toda la lógica normativa, listas blancas y comprobaciones binarias viven centralizadas y sin duplicación en [crates/mule-network-policy/src/content_policy.rs](../../../crates/mule-network-policy/src/content_policy.rs):
- **Lista blanca de vídeo (`ALLOWED_VIDEO_EXTENSIONS`)**: `mkv`, `mp4`, `avi`, `m4v`, `mov`, `wmv`, `webm`, `ts`, `m2ts`, `mts`, `mpg`, `mpeg`, `m2v`, `vob`, `flv`, `ogv`, `ogm`, `asf`, `divx`, `rm`, `rmvb`, `3gp`, `3g2`, `f4v`.
- **Subtítulos complementarios dentro de torrents (`COMPANION_SUBTITLE_EXTENSIONS`)**: `srt`, `vtt`, `ass`, `ssa`, `sub`, `idx`, `sup` (siempre condicionados a la presencia de al menos un archivo de vídeo en el mismo torrent).
- **Bloqueo de doble extensión (`FORBIDDEN_EMBEDDED_EXTENSIONS`)**: rechazo de nombres cuya penúltima extensión pertenezca a ejecutables, scripts, comprimidos, imágenes de disco o documentos (por ejemplo, `pelicula.exe.mkv`, `capitulo.zip.mp4`, `sub.exe.srt`).
- **Inspección binaria pura en Rust (`validate_video_header` / `validate_video_file`)**: reconocimiento positivo en memoria de cabeceras reales de contenedor (EBML Matroska/WebM con `DocType`, ISO BMFF MP4/M4V/MOV/3GP/F4V rechazando marcas de audio/imagen `M4A `/`heic`/`avif`, RIFF AVI/AVIX, MPEG-TS/M2TS con guardia frente a texto ASCII puro, MPEG-PS/VOB, ASF/WMV, FLV con bit de vídeo activo, Ogg Video con códec de vídeo, RealMedia `.RMF`) y rechazo explícito de firmas no vídeo (`MZ`, `\x7FELF`, Mach-O, `PK`, `Rar!`, `7z`, `%PDF-`, `GZIP`/`BZIP2`/`XZ`, JPEG/PNG/GIF/WebP, ID3/FLAC/WAV, `#!`, HTML/XML, ISO 9660 `CD001`). Sin subprocesos externos (`ffprobe`) ni asignaciones innecesarias.

### 2. Cuatro puertas defensivas (eD2K, Kad, Torznab, BitTorrent y Compartidos)
1. **Puerta 1 — Búsqueda y descubrimiento**:
   - eD2K servidor: `SearchQuery::file_type` envía `FT_FILETYPE = "Video"` en cable (`OP_SEARCHREQUEST`); `servers.search` rechaza filtros `extension` no pertenecientes a vídeo y `emit_search_results` filtra resultados entrantes no vídeo.
   - Kad y Torznab: `search_keywords` y `/torznab/api` filtran respuestas cuyo nombre no supere `is_allowed_video_filename`.
2. **Puerta 2 — Admisión previa a la asignación en disco**:
   - eD2K: `add_ed2k_link`, `resume_download`, `import_part_met` y `recover_orphan_part_mets` exigen `validate_video_filename`. En `.part.met` con parte 0 ya verificada se inspecciona además la cabecera binaria antes de adoptar el archivo.
   - BitTorrent (`mule-torrent`): admisión en dos pasos mediante `librqbit` (`list_only: true` → `select_torrent_video_files` → reingreso con `only_files` solo sobre índices de vídeo y subtítulos y conservación de `seen_peers`). Rechazo inmediato con `TorrentError::ContentRejected` si no existe ningún archivo de vídeo.
3. **Puerta 3 — Inspección en vuelo de coste mínimo**:
   - eD2K: en `inject_range_impl`, **únicamente tras superar la verificación MD4 de la parte 0** (evitando así falsos positivos cuando un par malicioso envía basura corrupta), se inspecciona `HEADER_INSPECT_BYTES` (4 KiB) directamente desde `staged_parts` en RAM (`O(1)`, cero lecturas extra de disco; respaldo en `PartFile` si la parte se reanudó desde disco). Ante cabecera no válida se borran `.part` y `.part.met`, se registra el hash en `cancelled_files`, se marca `Failed` y se anota `"content_policy_rejected"` en el historial.
   - BitTorrent: `spawn_video_header_watchdog` vigila de forma asíncrona y finita cada índice de vídeo usando `tokio::fs::File` (esperando si el archivo disperso aún tiene prefijo de ceros antes de recibir la pieza 0) y ejecuta `session.delete(..., true)` en cuanto detecta una cabecera falsificada o finaliza un archivo no válido; una vez verificados todos los vídeos del torrent, la tarea finaliza con consumo cero de CPU/RAM.
4. **Puerta 4 — Prefinalización, compartidos y renombrado**:
   - Prefinalización (`prepare_finalize`): si `content_header_verified` es falso (por ejemplo, cargas recuperadas o sin MD4 en vuelo de parte 0), `validate_video_file` valida los primeros `FILE_HEADER_INSPECT_BYTES` (64 KiB) antes de mover a `Incoming`/organizador Plex-Jellyfin.
   - Mutaciones y escaneos: `downloads.rename`, `shares.rename`, `shares.add`, `incoming_share::is_shareable_file` (compartida con `known_met_import`), `disk_scanner`, `cleaner` y `organizer` delegan exclusivamente en `mule_network_policy::content_policy`.

### 3. Invariantes no negociables
- **Fija en tiempo de compilación**: no existe opción RPC, ajuste en `DaemonConfig` ni variable de entorno para desactivar o relajar la política «Solo Vídeo».
- **Propiedad única**: queda terminantemente prohibido duplicar listas de extensiones o validadores ad hoc fuera de `mule_network_policy::content_policy`.
- **Resiliencia ante falsos negativos/positivos**: ningún par corrupto puede forzar la cancelación permanente (`cancelled_files`) de una descarga legítima de vídeo enviando un bloque `MZ` en parte 0 porque la inspección en vuelo solo actúa después de verificar el hash MD4 esperado de la parte 0.

## Consecuencias
- **Positivas**: cumplimiento estricto de descarga y compartición exclusiva de vídeo en todos los protocolos (eD2K, Kad, Torznab, BitTorrent y Transmission RPC); cero procesos externos; cero lecturas adicionales en disco en la ruta caliente normal de descarga eD2K; mantenimiento centralizado al existir una única lista blanca en `mule-network-policy`.
- **Negativas**: cualquier archivo legítimo comprimido, ISO, audio independiente o binario general queda rechazado por diseño en admisión, compartición, renombrado y búsqueda; las suites de prueba deben emplear nombres de archivo de vídeo (`.mkv`, `.mp4`, etc.) y cabeceras binarias válidas (`MINIMAL_VALID_MKV_HEADER`) cuando ejercitan rutas de verificación MD4 de parte 0 o finalización.

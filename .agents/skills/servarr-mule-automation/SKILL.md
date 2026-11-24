---
name: servarr-mule-automation
description: >-
  Operar, compilar de forma cruzada, depurar y ampliar la automatización nativa de Servarr
  (Prowlarr, Radarr, Sonarr, Bazarr) integrada en mule-daemon (eMule-Rust), incluyendo la
  integración REST con OpenSubtitles, sincronización de hashes de verificación de 64 bits,
  compilación cruzada aarch64 y organización multimedia sin sobrecarga.
---

# Automatización nativa de Servarr y Bazarr en eMule-Rust

Esta habilidad (*skill*) proporciona reglas operativas, protocolos, particularidades de API y guías de ejecución para el ecosistema multimedia Servarr integrado de forma nativa en `mule-daemon`.

---

## 1. Compilación cruzada y despliegue en destino

### Reglas de compilación
La cadena de herramientas de compilación cruzada para `aarch64-unknown-linux-gnu` está configurada en `.cargo/config.toml` del repositorio `eMule-Rust`.

```bash
# Cambiar SIEMPRE de directorio a la raíz del workspace antes de ejecutar comandos cargo:
cd <ruta-del-repositorio>

# Compilar binario de lanzamiento para Raspberry Pi (aarch64)
cargo build -p mule-daemon --release --target aarch64-unknown-linux-gnu
```

### Despliegue remoto y gestión del servicio
- **Host de destino**: Raspberry Pi (`<ip-raspberry>`, usuario `<usuario>`)
- **Ruta de destino remota**: `~/.local/bin/mule-daemon`
- **Script de despliegue**:
```bash
scp target/aarch64-unknown-linux-gnu/release/mule-daemon <usuario>@<ip-raspberry>:~/.local/bin/
ssh <usuario>@<ip-raspberry> "systemctl --user restart mule-daemon.service"
```
- **Persistencia del servicio**:
  Asegurarse de que la persistencia de usuario (*linger*) esté habilitada para que `mule-daemon` siga ejecutándose tras cerrar la sesión SSH:
  `loginctl enable-linger <usuario>`

---

## 2. Integración con la API REST de OpenSubtitles (Motor Bazarr)

### Protocolo de búsqueda y cabeceras
- **URL base**: `https://rest.opensubtitles.org/search/`
- **Cabecera obligatoria**: `User-Agent: VLSub 0.10.2`
  - Evita claves de API, cuotas comerciales y bloqueos por captcha.
- **Reglas de parámetros de consulta**:
  - `sublanguageid`: debe ser un único código de 3 letras (p. ej., `sublanguageid-spa`). Múltiples códigos separados por comas devuelven HTTP 400 (`multiple values per parameter are not supported`).
  - Consultas de búsqueda: sustituir los espacios por `+` (p. ej., `query-The+Matrix`).

### Algoritmo de hash de 64 bits de OpenSubtitles
Se utiliza para la sincronización exacta 1:1 entre subtítulos y audio sin desfase temporal:
```rust
pub fn compute_opensubtitles_hash(path: &Path) -> Result<(u64, u64)> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    if size < 131072 {
        return Err(anyhow!("File too small for 64k dual-window hash"));
    }
    let mut hash = size;
    // First 64KB (8192 little-endian u64 words)
    let mut buf = [0u8; 65536];
    file.read_exact(&mut buf)?;
    for chunk in buf.chunks_exact(8) {
        hash = hash.wrapping_add(u64::from_le_bytes(chunk.try_into().unwrap()));
    }
    // Last 64KB (8192 little-endian u64 words)
    file.seek(SeekFrom::Start(size - 65536))?;
    file.read_exact(&mut buf)?;
    for chunk in buf.chunks_exact(8) {
        hash = hash.wrapping_add(u64::from_le_bytes(chunk.try_into().unwrap()));
    }
    Ok((size, hash))
}
```
Formato de ruta de consulta: `moviebytesize-{size}/moviehash-{hash:016x}/sublanguageid-{lang}`.
Para búsquedas por IMDb ID de series, incluir siempre `season-{s}/episode-{e}` además de `imdbid-{id}`.

### Puntuación, validación y sincronización (ADR `adr_subtitulos_sincronizacion.md`)
- Los candidatos de OpenSubtitles (y opcionalmente OpenSubtitles.com REST v1 si existe `opensubtitlesApiKey`) se evalúan mediante `subtitles::scorer` combinando hash, grupo de *release* (`original_release_name`), fuente, servicio, edición, FPS, variante (`es-ES`/`es-LA`) y confianza.
- Toda descarga se valida sintácticamente en `subtitles::srt` (mínimo 10 bloques, duración compatible con `MediaProbeInfo.duration_secs` y heurística de idioma) antes de escribir `<video>.es.srt`.
- Si no hubo coincidencia exacta por `moviehash` y `subtitlesAutoSync != "off"`, `subtitles::sync` reescala por FPS si difieren y alinea el `.srt` contra la pista textual embebida extraída con `ffmpeg`.
- Los subtítulos rechazados se registran en `subtitle_blacklist` (SQLite) para evitar redescargas.

### Decodificación del flujo de subtítulos
- Las descargas están comprimidas en `.gz`: descomprimir al vuelo usando `flate2::read::GzDecoder`.
- Los archivos de subtítulos pueden estar codificados en CP1252 o ISO-8859-1:
  1. Eliminar el BOM UTF-8 si está presente (`\u{feff}`).
  2. Si `std::str::from_utf8` falla, decodificar mediante `encoding_rs::WINDOWS_1252` y recodificar a UTF-8 puro.
- Guardar junto al vídeo como archivo acompañante: `<video_base_path>.es.srt`.

---

## 3. Organizador multimedia y enlaces duros (*Hardlinks*)

### Ubicación de archivos y comportamiento de enlace duro
- **Esquema de destino**:
  - Películas: `Movies/<Title> (<Year>)/<Title> (<Year>) [<Quality>].<ext>`
  - Series: `TV Shows/<Series Title> (<Year>)/Season <SS>/<Series Title> - S<SS>E<EE>.<ext>`
- **Estrategia de enlace duro**:
  - Principal: `std::fs::hard_link(&source, &dest)` (cero sobrecarga, atómico, sin consumo adicional de disco).
  - Respaldo entre dispositivos: si el código de error es `EXDEV` (particiones o sistemas de archivos distintos), recurrir a `std::fs::copy(&source, &dest)`.

---

## 4. Validación y coincidencia de lanzamientos (antifalsos positivos)

Antes de capturar un lanzamiento torrent/eD2K mediante indexadores Torznab:
1. **Coincidencia de tokens del título**: calcular la superposición frente a tokens alfanuméricos limpios (ignorando grupos de lanzamiento habituales y palabras vacías). Exigir una coincidencia > 75 % o inclusión completa de tokens.
2. **Tolerancia de año**: si el lanzamiento contiene un año detectado de 4 dígitos, debe coincidir con `target_year ± 1`. Si no coincide, rechazar de inmediato.
3. **Límite de episodio**: para series, asegurar que el patrón `SxxExx` detectado coincida exactamente con el episodio objetivo.

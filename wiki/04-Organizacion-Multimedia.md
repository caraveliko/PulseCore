# 🎬 Organización y Catalogación Multimedia

Pulse Core incorpora de forma nativa su propio motor de gestión de medios (`mule-daemon::services::media` y `organizer`), diseñado para transformar las descargas completadas en una biblioteca ordenada y lista para reproducir en **Plex**, **Jellyfin** o **Kodi**, sin requerir herramientas externas ni servicios pesados.

---

## 1. Enlaces Duros sin Duplicar Espacio (*Zero-Copy Hardlinks*)

Uno de los mayores problemas al utilizar clientes P2P tradicionales junto con gestores de medios es la duplicación de espacio: o bien se mueve el archivo (rompiendo la compartición y el ratio de subida en la red), o bien se duplica (gastando el doble de almacenamiento).

Pulse Core resuelve esto mediante **enlaces duros del sistema de archivos (`std::fs::hard_link`)**:

```text
Disco Duro / Almacenamiento (Inodo #48291)
  ├── Entrada 1: ~/Descargas/PulseCore/Incoming/Pelicula.2024.1080p.mkv   <-- Compartiendo en eD2K/Torrent
  └── Entrada 2: /media/Peliculas/Pelicula (2024)/Pelicula (2024).mkv    <-- Leído por Plex / Jellyfin
```

### Ventajas de este enfoque
- **Consumo de disco adicional = 0 bytes**: Ambas rutas apuntan al mismo bloque físico de almacenamiento.
- **Siembra continua ininterrumpida**: El archivo puede seguir compartiéndose en la red eD2K y en el enjambre Torrent indefinidamente.
- **Estructura estricta para servidores multimedia**: Las películas y series se renombran y organizan automáticamente según las convenciones estándar:
  - Películas: `Películas/Título (Año)/Título (Año).ext`
  - Series de TV: `Series/Nombre de la Serie/Temporada 01/Nombre S01E01.ext`

*(Nota técnica: Si la carpeta de destino reside en un sistema de archivos o disco físico distinto, el motor detecta el error de sistema `EXDEV` y realiza una copia segura con verificación de integridad).*

---

## 2. Actualización Progresiva de Calidades (*Quality Upgrades*)

El motor de catalogación evalúa continuamente las calidades disponibles en la red para los elementos supervisados en tu biblioteca:

1. **Puntuación de Lanzamientos**: Cada versión descubierta en la red se califica numéricamente considerando:
   - **Resolución**: 720p < 1080p < 2160p (4K UHD).
   - **Códec de Vídeo**: MPEG-4 < H.264/AVC < H.265/HEVC < AV1.
   - **Pista de Audio y Rango Dinámico**: Detección de HDR10, Dolby Vision y pistas multicanal (5.1, Atmos).
2. **Reemplazo Transparente y Atómico**:
   - Si se detecta un lanzamiento con mayor puntuación que la versión actualmente almacenada, Pulse Core programa su descarga en segundo plano.
   - Una vez finalizada y verificada la nueva versión, el enlace duro en la biblioteca multimedia se actualiza de forma atómica. Tu servidor de medios (Plex o Jellyfin) detecta inmediatamente la versión mejorada sin que el usuario tenga que borrar archivos ni renombrar carpetas a mano.

---

## 3. Descarga Automática de Subtítulos (OpenSubtitles / VLSub)

Para garantizar que los subtítulos estén perfectamente sincronizados con la pista de audio y la velocidad de fotogramas (*framerate*) del archivo descargado, Pulse Core implementa el cliente REST de OpenSubtitles con **algoritmo de suma de verificación de 64 bits**:

### Cálculo del Hash de Vídeo
En lugar de basarse en el nombre del archivo (que puede ser engañoso), el motor calcula una suma de verificación directa sobre el flujo binario:
$$\text{Hash}_{64} = \text{Tamaño en bytes} + \sum \text{primeros 64 KB} + \sum \text{últimos 64 KB}$$

### Flujo de trabajo automatizado
1. Al completarse la descarga del vídeo, el servicio de subtítulos calcula el hash de 64 bits y realiza una consulta a la API de OpenSubtitles.
2. Si existe un subtítulo exacto en el idioma preferido (por defecto español de España, `.es.srt`), se descarga el paquete comprimido `.gz`.
3. El motor descomprime el flujo en memoria, normaliza la codificación a **UTF-8 estricto** y lo deposita junto al archivo de vídeo con el mismo nombre base:
   - `Película (2024).mkv`
   - `Película (2024).es.srt`

---

## 4. Filtrado Inteligente de Falsos Positivos

Las redes P2P abiertas pueden contener archivos con títulos engañosos, grabaciones de cine de baja calidad (*CAM/TS*) o archivos corruptos.

Pulse Core incluye algoritmos heurísticos de filtrado que analizan los metadatos antes de autorizar la captura:
- **Tolerancia de año estricta**: Rechaza coincidencias con desviaciones de año superiores a $\pm 1$ para evitar descargar adaptaciones clásicas o secuelas homónimas.
- **Descarte de etiquetas de baja fidelidad**: Reglas de exclusión para términos como `CAM`, `TELESYNC`, `HDCAM`, `SAMPLE` o resoluciones anómalas.
- **Validación de tamaño mínimo**: Comprobación de que el tamaño del archivo se corresponda con la duración esperada según el códec reportado en los metadatos.

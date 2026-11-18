# 🖥️ 02. Manual Completo de la Interfaz Web (WebUI)

La interfaz web de **Pulse Core** es servida directamente por el binario `mule-daemon` en el puerto `4660`. Es una aplicación reactiva en tiempo real que se comunica mediante **WebSockets bidireccionales**, lo que significa que el progreso de descarga, las tasas de transferencia y el estado de las fuentes se actualizan al milisegundo sin tener que recargar la página.

---

## 1. Anatomía de la Barra de Navegación Superior

En la parte superior de la interfaz encontrarás la barra de control fija:

- **Logotipo y Estado Global**: Muestra el icono de Pulse Core y el estado de la conexión en vivo (punto verde = WebSocket conectado; punto rojo = reconectando).
- **Indicadores de Velocidad en Tiempo Real**:
  - 🔻 **Bajada**: Velocidad global agregada en KiB/s o MiB/s.
  - 🔺 **Subida**: Velocidad de subida compartida hacia otros pares.
- **Botón de Añadir Descarga (➕)**: Abre el modal rápido para pegar enlaces.
- **Pestañas de Navegación**:
  1. 📥 **Descargas**: Vista principal de transferencias en curso.
  2. 🔍 **Buscar**: Motor de búsqueda global en Kad y eD2K.
  3. 🌐 **Servidores**: Lista de servidores eD2K y estado de conexión.
  4. 🕸️ **Red Kad**: Diagnóstico y tabla de contactos de Kademlia DHT.
  5. 📁 **Compartidos**: Archivos que estás sembrando en la red.
  6. 🎬 **Cine & Series**: Catálogo y biblioteca multimedia propia.
  7. 📜 **Registro en Vivo**: Trazas de auditoría y eventos en tiempo real.
  8. ⚙️ **Ajustes**: Panel completo de configuración del sistema.

---

## 2. Pestaña de Descargas (📥)

Es el centro neurálgico del programa.

### Cómo añadir una nueva descarga
Pulsa el botón **«Añadir Enlace»** (o el icono `➕`) en la esquina superior derecha:
- Puedes pegar:
  - Enlaces eD2K: `ed2k://|file|nombre.ext|tamaño|hash_md4|/`
  - Enlaces con hash AICH: `ed2k://|file|nombre.ext|tamaño|hash_md4|h=hash_aich|/`
  - Enlaces Magnet: `magnet:?xt=urn:btih:...`
  - Enlaces HTTP directos a archivos `.torrent` o `.met`.
- Pulsa **«Descargar»** y la transferencia aparecerá de inmediato en la tabla.

### Columnas de la tabla de transferencias
| Columna | Significado |
| :--- | :--- |
| **Nombre** | Nombre del archivo con icono de protocolo (`[eD2K]` en azul, `[Torrent]` en naranja). |
| **Tamaño** | Tamaño total formateado (MB / GB). |
| **Completado** | Porcentaje numérico y bytes descargados hasta el momento. |
| **Velocidad** | Tasa de transferencia instantánea dedicada a este archivo. |
| **Progreso** | Barra gráfica segmentada de franjas de colores (véase [Manual 03](03-Gestion-de-Descargas-y-Fuentes)). |
| **Fuentes** | Número de fuentes descubiertas / fuentes útiles conectadas (ej. `45/120 (3)`). |
| **Prioridad** | Nivel de preferencia de ancho de banda (`Baja`, `Normal`, `Alta`, `Auto`). |
| **Estado** | Estado operativo: `Descargando`, `En pausa`, `En cola`, `Verificando`, `Completado`. |

### Menú de Acciones (clic en cualquier descarga)
Al hacer clic sobre una fila se despliega el menú contextual con opciones:
- ⏸️ **Pausar / Reanudar**: Detiene temporalmente la descarga conservando las fuentes en memoria.
- ⏹️ **Detener**: Desconecta las fuentes y congela el archivo `.part`.
- 🗑️ **Cancelar y Borrar**: Elimina la descarga y purga los fragmentos temporales en disco.
- ℹ️ **Detalles del Archivo**: Abre una ventana con el árbol de partes AICH, metadatos técnicos y la lista de todos los pares IP conectados y su porcentaje de archivo.

---

## 3. Pestaña de Búsqueda (🔍)

Permite buscar contenido en millones de fuentes descentralizadas:

### Modos de Búsqueda
1. **Red Kad (Recomendado)**: Búsqueda 100 % descentralizada en el árbol de contactos Kademlia sin intermediarios.
2. **Servidor Global**: Consulta a todos los servidores eD2K conocidos.
3. **Servidor Local**: Consulta rápida únicamente al servidor eD2K al que estás conectado.

### Filtros de Búsqueda
Puedes acotar los resultados mediante los desplegables:
- **Tipo**: *Cualquiera*, *Vídeo*, *Audio*, *Archivo comprimido (.zip, .rar, .7z)*, *Imagen de disco (.iso)*, *Documentos*.
- **Tamaño Mínimo y Máximo**: Útil para filtrar muestras (*samples*) o archivos falsos muy pequeños.
- **Extensión**: Especificar p. ej. `mkv`, `flac`, etc.

### Interpretación de Resultados
Los resultados se ordenan por defecto por **Disponibilidad** (número de fuentes completas que tienen el archivo al 100 %):
- **Barra verde / azul intenso**: Alta disponibilidad (decenas o cientos de fuentes). La descarga comenzará rápidamente.
- **Barra roja / pocas fuentes**: Archivo raro o con poca disponibilidad. Puede tardar más tiempo en encontrar a alguien en línea con las partes necesarias.

---

## 4. Pestaña de Servidores (🌐)

Supervisa tu conexión a la red de servidores clásicos eD2K:
- Muestra la lista de servidores indexadores con su dirección IP, puerto, ping y número de usuarios y archivos compartidos.
- **Conectar / Desconectar**: Pulsa sobre cualquier servidor para conectar de inmediato.
- **Información de Conexión en el panel inferior**:
  - **Tu ID de Cliente**: Si tu ID es mayor de `16777216`, tienes **ID Alta (*HighID*)** (conectividad perfecta). Si es menor, tienes **ID Baja (*LowID*)** (puerto bloqueado por cortafuegos o NAT).
  - Consulta el [Manual 05](05-Red-eD2K-Servidores-e-ID-Alta) para solucionar la ID Baja.

---

## 5. Pestaña de Red Kad (🕸️)

Permite supervisar la salud de la red Kademlia DHT:
- **Estado**: `Conectado` (verde), `Conectando` (amarillo) o `Desconectado` (rojo).
- **Estado de Cortafuegos**:
  - `Abierto`: Tu puerto UDP `39737` es accesible desde el exterior. Máxima velocidad de búsqueda.
  - `Tras cortafuegos (*Firewalled*)`: Tráfico UDP bloqueado. Pulse Core activará automáticamente un cliente compañero (*Buddy*) para permitir transferencias.
- **Contactos Activos**: Número de nodos conocidos en tus K-buckets (lo óptimo es entre 200 y 800 contactos).
- **Botón Bootstrap**: Si la red Kad no conecta, puedes introducir la IP y puerto de un nodo activo o cargar un archivo `nodes.dat`.

---

## 6. Pestaña de Archivos Compartidos (📁)

Muestra los archivos que estás compartiendo en la red:
- **Carpeta Incoming**: Archivos ya completados que se están sembrando.
- **Carpetas Adicionales**: Cualquier directorio que hayas añadido en los Ajustes.
- **Estadísticas por archivo**: Total de peticiones recibidas, megabytes subidos y partes más populares.
- **Copiar Enlace eD2K**: Haz clic derecho sobre cualquier archivo compartido para copiar su enlace oficial `ed2k://` y compartirlo con amigos o foros comunitarios.

---

## 7. Pestaña de Cine & Series (🎬)

Es el módulo de biblioteca multimedia propio de Pulse Core:
- **Vista en Cuadrícula de Portadas**: Muestra los pósteres oficiales de películas y series catalogadas en tu disco.
- **Ficha Técnica**: Al pulsar sobre cualquier título, verás su sinopsis, año, director, pistas de audio, resolución y si cuenta con subtítulo sincronizado `.es.srt`.
- **Estatus de Enlace Duro**: Muestra si el archivo está enlazado (*Zero-Copy*) con tu carpeta de Plex o Jellyfin.

---

## 8. Pestaña de Registro en Vivo (*Logs*) (📜)

Muestra las trazas de depuración de Rust generadas en tiempo real mediante `tracing`:
- Permite filtrar por severidad: `Info`, `Warning`, `Error` o `Debug`.
- Muy útil para comprobar por qué un servidor no responde, qué IPs han sido descartadas por el filtro anti-SSRF o por `ipfilter.dat`, o cuándo se ha completado un fragmento.

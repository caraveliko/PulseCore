# 🎬 04. Biblioteca Multimedia, Enlaces Duros y Subtítulos

Uno de los aspectos más potentes y diferenciadores de **Pulse Core** es su motor de gestión y organización multimedia propio (`organizer` y `media_library`). Esta funcionalidad convierte el daemon en el compañero perfecto para servidores de medios como **Plex**, **Jellyfin** o **Kodi**.

---

## 1. El Problema del Espacio y los Enlaces Duros (*Zero-Copy*)

Cuando descargas una película en un cliente tradicional, te encuentras ante un dilema:
- Si **mueves** el archivo a tu carpeta de películas para que lo lea Plex, rompes la compartición en la red P2P (dejas de sembrar y pierdes ratio).
- Si **copias** el archivo, gastas el doble de disco (una película de 15 GB pasa a ocupar 30 GB).

Pulse Core soluciona esto de raíz utilizando **enlaces duros del sistema de archivos (`hard_link`)**:

### ¿Qué es un enlace duro?
A nivel de sistema de archivos (NTFS en Windows, ext4/btrfs/ZFS en Linux):
- Un archivo físico es un bloque de datos en disco identificado por un número de inodo.
- Un enlace duro es simplemente una **segunda entrada en el directorio que apunta al mismo bloque físico**.
- Puedes tener el archivo en dos rutas a la vez:
  ```text
  /media/datos/Descargas/Incoming/Gladiator.2000.1080p.mkv     (Ruta de compartición P2P)
  /media/datos/Peliculas/Gladiator (2000)/Gladiator (2000).mkv  (Ruta de Plex / Jellyfin)
  ```
- **Espacio ocupado**: exactamente 15 GB, ¡cero bytes adicionales duplicados!
- Si borras una de las dos rutas, la otra sigue existiendo intacta. El espacio solo se libera cuando se eliminan ambas.

> [!IMPORTANT]
> **Requisito para Enlaces Duros**: La carpeta de descargas entrantes (`Incoming`) y la carpeta de biblioteca multimedia deben residir en la misma partición o disco físico. Si están en discos distintos, Pulse Core detectará la frontera de sistema de archivos (`EXDEV`) y realizará una copia ordinaria.

---

## 2. Estructura de Carpetas Estandarizada

Cuando una descarga multimedia finaliza, Pulse Core analiza sus metadatos (título, año, temporada, episodio) y crea la jerarquía recomendada por Plex y Jellyfin:

### Estructura para Películas
```text
Películas/
└── Nombre de la Película (Año)/
    ├── Nombre de la Película (Año).mkv
    └── Nombre de la Película (Año).es.srt
```

### Estructura para Series de Televisión
```text
Series/
└── Nombre de la Serie (Año)/
    └── Temporada 01/
        ├── Nombre S01E01.mkv
        ├── Nombre S01E01.es.srt
        └── Nombre S01E02.mkv
```

---

## 3. Actualización Progresiva de Calidades (*Quality Upgrades*)

Pulse Core no solo descarga un archivo y se olvida: **supervisa activamente la calidad de tu biblioteca**.

### ¿Cómo funciona la mejora automática?
1. Supongamos que agregas una película clásica a tu catálogo de la que actualmente solo existe una versión `720p HDTV`. Pulse Core la descarga y la enlaza a tu biblioteca.
2. La película queda registrada en estado *Monitoreada*.
3. Días o semanas después, un usuario de la red comparte una versión remasterizada en `1080p BluRay` o `2160p 4K HDR`.
4. El algoritmo de puntuación de Pulse Core detecta la nueva versión, verifica que supera la puntuación de calidad de la copia actual y programa automáticamente su descarga en segundo plano.
5. Al completarse, **sustituye atómicamente el enlace duro** en tu carpeta de Plex. Tu servidor multimedia detecta la nueva resolución de inmediato sin que hayas tenido que hacer nada manualmente.

---

## 4. Subtítulos Sincronizados al Milisegundo con OpenSubtitles

Nada arruina más una película que un subtítulo desfasado por dos segundos debido a una diferencia de fotogramas por segundo (*23.976 fps* frente a *25.000 fps*).

Para evitar esto, Pulse Core integra el cliente REST de **OpenSubtitles (VLSub)** con cálculo de hash de vídeo de 64 bits:

### El algoritmo de suma de verificación binaria
En lugar de buscar el subtítulo por el nombre del archivo (que suele ser genérico), el daemon lee los primeros 64 KB y los últimos 64 KB del archivo de vídeo y suma su longitud en bytes. Esta combinación genera una **huella digital única**:

- Solo los subtítulos creados y sincronizados específicamente para ese mismo ripeo de vídeo compartirán dicho hash.
- Pulse Core descarga el archivo `.gz`, lo descomprime en memoria y valida que su codificación sea **UTF-8 universal** (evitando caracteres corruptos o problemas con tildes y eñes).
- Guarda el subtítulo junto al vídeo con la extensión `.es.srt` (español de España) para que Plex y Jellyfin lo activen automáticamente.

# 📦 08. Migración desde el eMule Clásico

Si eres un usuario veterano de **eMule 0.50a**, **eMule 0.72a** o mods comunitarios (MorphXT, Xtreme, ScarAngel), probablemente tengas descargas incompletas de varios gigabytes que no quieres perder.

**Pulse Core** incluye compatibilidad binaria nativa con el formato de metadatos de eMule (`.part.met`) y permite importar tus transferencias a medias para continuarlas sin perder ni un solo byte ya descargado.

---

## 1. Dónde Guarda el eMule Clásico las Descargas a Medias

En Windows, el eMule original suele almacenar las partes temporales en una de las siguientes ubicaciones:

- `C:\Program Files (x86)\eMule\Temp\`
- `%LOCALAPPDATA%\eMule\Temp\` (escribe esto en la barra de direcciones del Explorador de Windows)
- O en la ruta que hubieras configurado en *Preferencias ➔ Directorios ➔ Archivos temporales*.

Para cada descarga en curso encontrarás tres archivos:
1. `001.part`: El archivo de datos con los fragmentos binarios descargados.
2. `001.part.met`: Los metadatos de la descarga (nombre real, tamaño total, hash eD2K, árbol AICH y lista de partes completadas).
3. `001.part.met.bak`: Copia de seguridad automática de los metadatos.

---

## 2. Paso a Paso: Importar Descargas a Pulse Core

Sigue estos sencillos pasos para migrar tus descargas:

### Paso 1: Cerrar el eMule clásico
Asegúrate de cerrar completamente tu cliente eMule antiguo para que libere los bloqueos de lectura/escritura sobre los archivos `.part`.

### Paso 2: Copiar los archivos a la carpeta Temp de Pulse Core
Copia los pares de archivos (`xxx.part` y `xxx.part.met`) a la carpeta temporal de Pulse Core:
- En Linux: `~/.local/share/eMuleRust/temp/`
- En Windows: `%LOCALAPPDATA%\eMuleRust\temp\` (o tu carpeta personalizada si usas modo portable).

### Paso 3: Arrancar Pulse Core o Usar la Función de Importación
- Al arrancar `mule-daemon`, el servicio escaneará automáticamente la carpeta temporal.
- Leerá el formato `.part.met` clásico, verificará los hashes de las partes ya completadas mediante el algoritmo ICH/AICH y añadirá la descarga a la lista de transferencias activas.
- Verás que la barra de progreso refleja exactamente el mismo porcentaje y las mismas franjas negras/azules que tenías en eMule.
- La descarga continuará de inmediato buscando fuentes en Kad y eD2K.

---

## 3. Verificación Previa con `mule-cli` (Opcional)

Si quieres inspeccionar el estado de un archivo `.part.met` antes de importarlo para asegurarte de que no está dañado, puedes utilizar la herramienta de consola de Pulse Core:

```bash
./target/release/mule-cli inspect-part /ruta/a/001.part.met
```

La consola te mostrará una ficha técnica detallada:
```text
=== Ficha de Metadatos .part.met ===
Nombre original:  Documental_Historia_P2P.mkv
Tamaño total:     4.521.890.120 bytes (4,21 GiB)
Hash eD2K (MD4):  d41d8cd98f00b204e9800998ecf8427e
Árbol AICH:       Presente y validado (hashes SHA-1 correctos)
Partes totales:   488 partes
Partes completas: 340 partes (69,6 % completado)
Estado:           Listo para reanudar sin pérdida
```

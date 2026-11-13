# Especificación de funcionalidad: Interfaz web integrada y búsqueda global

**ID de funcionalidad**: `003-embedded-webui-and-search`  
**Estado**: Implementado  
**Versión objetivo**: `0.4.0`  
**Versión objetivo**: `0.4.0`  

## 1. Resumen ejecutivo
Para ofrecer un verdadero despliegue headless sin dependencias, `mule-daemon` debe compilar la interfaz web directamente dentro de su binario mediante `rust-embed`. Si no se especifica un `--web-dir` externo, la interfaz web integrada se sirve de forma transparente.
Además, los usuarios deben poder buscar contenido en los servidores eD2k conectados y en la DHT Kad directamente desde la interfaz web, ver resultados en vivo con indicadores de disponibilidad y encolar descargas con un solo clic.

## 2. Requisitos
1. **Recursos integrados**:
   - Recursos de `web/` compilados en el binario mediante `rust-embed`.
   - Respaldo automático cuando `--web-dir` está ausente.
   - Respaldo de rutas SPA (`index.html` ante 404).
2. **Búsqueda global en la interfaz web**:
   - Pestaña «Búsqueda» dedicada en `web/index.html`.
   - Campo para palabra clave o término, filtros de tamaño opcionales (mín./máx.) y filtro de categoría de archivo.
   - La ejecución de la búsqueda envía `kad.searchKeywords` y `servers.search`.
   - Los resultados llegan en flujo continuo y se deduplican por hash eD2k.
   - Acción «Descargar» en un clic por resultado que encola mediante `downloads.addEd2kLink`.
3. **Selector de directorios (`Incoming` y `Temp`)**:
   - En los campos de rutas de directorios (**Carpeta de Descargas Finales (Incoming)**, **Carpeta de Fragmentos Temporales (Temp)** y bibliotecas multimedia), se mantiene el campo de texto para introducir la ruta manualmente.
   - Cada campo incorpora un botón con icono de carpeta (`📂`) que abre el explorador de archivos/carpetas: en Windows local invoca el selector nativo (`settings.pickFolder`) y, en entornos Linux / Raspberry Pi o acceso remoto, abre el explorador visual integrado en la WebUI respaldado por `settings.listDirs`.

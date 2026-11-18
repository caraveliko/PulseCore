# 💻 10. Manual de la CLI (`mule-cli`)

Junto con el daemon principal, el proyecto incluye **`mule-cli`**, una herramienta ligera de consola para administradores y desarrolladores que permite inspeccionar archivos locales, calcular sumas de verificación eD2K y generar enlaces sin necesidad de abrir el navegador.

---

## 1. Compilación e Instalación

Para compilar la CLI:
```bash
cargo build --release -p mule-cli
```
El binario resultante se encontrará en `target/release/mule-cli` (o `.exe` en Windows). Puedes moverlo a tu ruta de ejecutables (ej. `/usr/local/bin` o `~/.local/bin`).

---

## 2. Comandos Principales

### 1. Generar enlace eD2K oficial (`link`)
Calcula el hash del archivo y genera el enlace estándar compatible con cualquier cliente de eMule:

```bash
mule-cli link /ruta/a/mi_archivo.iso
```
Salida en consola:
```text
ed2k://|file|mi_archivo.iso|4294967296|31d6cfe0d16ae931b73c59d7e0c089c0|h=4k7h2j9d8f7g6h5j4k3l2m1n0b9v8c7x|/
```
*(Incluye automáticamente el hash raíz del árbol AICH `h=...` para garantizar la recuperación de partes dañadas).*

---

### 2. Inspección criptográfica de un archivo (`hash`)
Muestra el desglose de identificadores criptográficos:

```bash
mule-cli hash /ruta/a/video.mkv
```
Salida:
```text
=== Desglose de Hashes ===
Tamaño en bytes: 1.572.864.000 (1,46 GiB)
Partes eD2K:     170 partes de 9,28 MB
Hash eD2K (MD4): a1b2c3d4e5f60718293a4b5c6d7e8f90
Raíz AICH:       e3b0c44298fc1c149afbf4c8996fb92427ae41e4
Hash Torrent:    b4c5d6e7f8a90123456789abcdef0123456789ab
```

---

### 3. Inspeccionar descargas a medias (`inspect-part`)
Analiza la integridad de un archivo temporal `.part.met` procedente de eMule o de Pulse Core:

```bash
mule-cli inspect-part ~/.local/share/eMuleRust/temp/002.part.met
```
Muestra:
- Nombre original del archivo y tamaño total esperado.
- Número de partes descargadas, pendientes y corruptas.
- Versión del formato `.part.met` y compatibilidad.

---

### 4. Salida en formato JSON para Scripts (`--json`)
Todos los comandos admiten el modificador `--json`, ideal para integrar Pulse Core en scripts de bash o flujos de trabajo automatizados:

```bash
mule-cli hash video.mkv --json
```
```json
{
  "fileName": "video.mkv",
  "sizeBytes": 1572864000,
  "ed2kHash": "a1b2c3d4e5f60718293a4b5c6d7e8f90",
  "aichRoot": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4",
  "partsCount": 170
}
```

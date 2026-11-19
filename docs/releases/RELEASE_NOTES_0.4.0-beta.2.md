# Notas de la Versión — Pulse Core v0.4.0-beta.2

**Fecha de publicación:** 20 de noviembre de 2026  
**Estado:** Versión beta — Política estricta de contenido **«Solo Vídeo»** (ADR 0006) y **sistema híbrido de actualización para Linux / `systemd`**, rollback O(1), WebUI y CLI.

---

## 1. Síntesis de la Versión

La versión **v0.4.0-beta.2** refuerza la especialización multimedia del cliente con la política fija de compilación **«Solo Vídeo»** y resuelve el ciclo completo de actualización en servidores y equipos **Linux (`x86_64` y `aarch64` / Raspberry Pi)**, además de mantener soporte íntegro en Windows.

---

## 2. Novedades y Mejoras Principales

### Política de Contenido «Solo Vídeo» (ADR 0006)
- **Fuente única de verdad (`mule_network_policy::content_policy`)**: catálogo inmutable de extensiones multimedia permitidas y rechazo nominal (incluidas extensiones dobles y compuestas).
- **Cuatro puertas defensivas obligatorias**:
  1. Admisión (`downloads.addEd2kLink`, resultados Kad/eD2K, Torznab y descargas manuales).
  2. Inspección binaria temprana en vuelo al completarse la parte 0 (MD4 verificada), bloqueando payloads ejecutables, comprimidos o ajenos antes de consolidar la descarga.
  3. Prefinalización, recuperación `.part.met`, archivos huérfanos y compartidos (`shares.*`).
  4. Selección automática en torrents BitTorrent (`mule-torrent`): solo admite contenedores de vídeo válidos y subtítulos acompañantes cuando el torrent contiene al menos un archivo de vídeo.

### Sistema Híbrido de Actualización Linux / `systemd`, Rollback O(1) y WebUI
- **Actualizador oficial endurecido**: integración directa con `NetworkPolicyManager` (interruptor de corte, aislamiento `LocalLab` y redirecciones 302 validadas hop-a-hop contra SSRF), descarga en *streaming* con progreso real, comparación SemVer de prelanzamientos (`0.4.0-beta.2 > 0.4.0-beta.1`) y mensajes claros ante HTTP 404.
- **Actualización desde binario local compilado (`POST /api/system/upload-update`)**: subida autenticada mediante `Authorization: Bearer <token>` (límite 64 MiB), verificación estricta de cabecera ELF64 (`EM_X86_64` / `EM_AARCH64`) o PE (`MZ`), suma SHA-256 opcional y sustitución atómica sin riesgo de instalar una arquitectura incorrecta.
- **Respaldo O(1) `.old` y rollback inmediato (`system.rollbackUpdate`)**: enlace duro `hard_link` previo a cada sustitución en Unix conservado tras reiniciar, validado antes de restaurar.
- **Integración nativa con `systemd` (`scripts/mule-daemon.service`)**: `%h/.local/bin` añadido a `ReadWritePaths`, `RestartSec=2`, liberación explícita de `DirectoryLock` y detección de `INVOCATION_ID`/`NOTIFY_SOCKET` para salida ordenada sin condición de carrera.
- **WebUI moderna (`web/index.html`)**: píldora inteligente de actualización en la barra superior con tarjeta emergente informativa, comprobación silenciosa al conectar, modal Drag & Drop con progreso real, rollback condicional integrado y reconexión automática sobre `/health`.
- **Herramientas de consola**: nuevo subcomando nativo `mule-cli update` (`status`, `check`, `upload`, `rollback`) y script [`scripts/update_linux.sh`](../../scripts/update_linux.sh) (`--local`, `--rpi`, `--install-service`, `--ssh`, `--rollback`).

---

## 3. Verificación e Integridad

Los binarios y paquetes para **Windows (x64)**, **Linux (x64)** y **Raspberry Pi (ARM64 / aarch64)** se compilan mediante GitHub Actions acompañados de sus sumas SHA-256 y del manifiesto firmado Ed25519 (`manifest.json`) cuando la clave `RELEASE_ED25519_KEY` está configurada.

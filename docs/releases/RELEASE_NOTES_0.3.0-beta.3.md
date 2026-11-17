# Notas de la Versión — Pulse Core v0.3.0-beta.3

**Fecha de publicación:** 17 de noviembre de 2026  
**Estado:** Versión beta — sistema de auto-actualización atómica integrada con verificación criptográfica Ed25519 e interfaz WebUI.

---

## 1. Síntesis de la Versión

La versión **v0.3.0-beta.3** introduce el sistema de actualización automática nativo y seguro de **Pulse Core**. Esta funcionalidad permite al demonio comprobar, descargar y aplicar nuevas versiones en caliente sin interrumpir descargas ni corromper transacciones SQLite en Windows, Linux x86_64 y Raspberry Pi ARM64.

---

## 2. Novedades y Mejoras Principales

### Sistema de Auto-Actualización Integrado
- **Sustitución atómica en caliente**: mecanismo multiplataforma que gestiona el bloqueo de archivos en ejecución en Windows (*move-aside* `.old`) y sustitución atómica de inodos en sistemas POSIX/Linux.
- **Verificación criptográfica asimétrica**: validación obligatoria de firmas Ed25519 con clave pública incrustada e integridad SHA-256 previa a cualquier manipulación en disco, previniendo ataques MITM y suplantaciones.
- **Apagado ordenado y relanzamiento**: preservación íntegra de fragmentos dispersos (`.part`), volcado de metadatos `.met`, cierre de mapeos UPnP y relanzamiento desacoplado del nuevo proceso.
- **API JSON-RPC de gestión**: nuevos métodos `system.checkUpdate`, `system.getUpdateStatus` y `system.applyUpdate`.

### Interfaz Web (WebUI)
- **Sub-pestaña de Actualizaciones**: nuevo panel en las Opciones del Sistema P2P con comprobación bajo demanda, visualización de notas de versión, barra de progreso y actualización en un solo clic.
- **Sincronización de versión**: detección dinámica de la versión del demonio en el arranque e indicación de estado visual del sistema.

### Automatización de Entrega y Firma en CI/CD
- **Flujo de empaquetado y firma en GitHub Actions**: generación automática del archivo de manifiesto firmado (`manifest.json`) en cada publicación de versión para Windows x64, Linux x86_64 y Raspberry Pi aarch64.
- **Script de firma criptográfica**: utilidad `scripts/generate_update_manifest.py` para generación de claves y firma de paquetes.

---

## 3. Verificación e Integridad

Todas las compilaciones para **Windows (x64)**, **Linux (x64)** y **Raspberry Pi (ARM64 / aarch64)** se encuentran firmadas criptográficamente y acompañadas por su correspondiente suma SHA-256 en el manifiesto oficial de la release.

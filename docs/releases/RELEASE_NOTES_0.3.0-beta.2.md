# Notas de la Versión — Pulse Core v0.3.0-beta.2

**Fecha de publicación:** 17 de noviembre de 2026  
**Estado:** Versión beta — mejoras de compatibilidad multiplataforma, seguridad CodeQL y sincronización WebUI.

---

## 1. Síntesis de la Versión

La versión **v0.3.0-beta.2** consolida la arquitectura del servidor unificado **Pulse Core** (anteriormente eMule-Rust), incorporando correcciones críticas de compatibilidad para entornos Windows, actualizaciones de seguridad auditadas mediante análisis estático CodeQL y optimizaciones en la interfaz web embebida.

---

## 2. Novedades y Mejoras Principales

### Multiplataforma y Compatibilidad Windows
- **Normalización de rutas canónicas en `mule-cli`**: corrección en el escaneo de directorios temporales de migración aMule (`import_amule`) para resolver de forma unívoca rutas en sistemas de archivos no sensibles a mayúsculas/minúsculas (NTFS en Windows), evitando entradas duplicadas de descargas incompletas.
- **Soporte cross-device en el organizador multimedia**: manejo robusto de enlaces y transferencias entre diferentes volúmenes y dispositivos de almacenamiento.

### Seguridad y Análisis Estático (CodeQL)
- **Aislamiento de permisos en flujos de CI**: restricción de permisos en GitHub Actions para cumplir con las directrices de seguridad de la cadena de suministro.
- **Validación SHA-256 en WebUI y cabeceras de API**: refuerzo en la verificación de integridad de activos estáticos y sanitización de cabeceras de autorización.
- **Refactorización de cifrado de cable**: desacoplamiento semántico del cifrado de ofuscación protocolaria (`WireObfuscationCipher`) para evitar falsos positivos heurísticos.

### Interfaz Web (WebUI) y Asistente
- **Aviso legal obligatorio**: incorporación de pantalla de conformidad inicial y términos de uso en el primer arranque de la interfaz web.
- **Generación dinámica de contraseñas de prueba**: sustitución de contraseñas fijas por identificadores dinámicos UUID en la suite de pruebas de autenticación WebUI.
- **Sincronización de autoría y contribución**: actualización de la información de autores y contribuyentes del proyecto Pulse Core.

---

## 3. Verificación e Integridad

Todas las compilaciones para **Windows (x64)**, **Linux (x64)** y **Raspberry Pi (ARM64 / aarch64)** se generan y empaquetan de forma automatizada mediante GitHub Actions con sus correspondientes sumas criptográficas SHA-256.

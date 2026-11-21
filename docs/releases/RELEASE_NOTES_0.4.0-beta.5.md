# Notas de la Versión — Pulse Core v0.4.0-beta.5

**Fecha de publicación:** 21 de noviembre de 2026  
**Estado:** Versión beta — **Rediseño integral de Opciones y barra de estado inferior**, **navegación P2P unificada**, **cinta continua de partes en Descargas**, **responsividad elástica** y **entrega inmediata de estilos/scripts sin caché heredada**.

---

## 1. Síntesis de la Versión

La versión **v0.4.0-beta.5** consolida la evolución visual y funcional de la WebUI embebida de Pulse Core. Reorganiza por completo el panel de **Opciones** para hacerlo intuitivo tanto en instalaciones locales/USB como en servidores NAS, renueva la barra inferior con telemetría limpia, simplifica la cabecera principal a 5 pestañas unificando *Servidores eD2k* y *Red Kademlia DHT* bajo **Red P2P**, e incorpora mejoras de densidad, cinta de partes en descargas y escalones responsivos para ventanas estrechas.

---

## 2. Novedades y Mejoras Principales

### Interfaz Web y Experiencia de Usuario (`web/index.html`, `web/assets/theme.css`)
- **Panel de Opciones (`#tab-settings`)**: nueva distribución con navegación lateral por categorías, buscador instantáneo, filtro `Esencial` / `Avanzado`, interruptores `.opt-switch` y selector claro entre almacenamiento **Local / USB** y **NAS / Red**.
- **Barra de estado inferior (`.bottom-statusbar`)**: celdas separadas mediante divisores verticales, micromedidores de `CPU`, `RAM` y almacenamiento con etiquetas dinámicas reales, y corrección del indicador de estado P2P (`#p2p-net-dot`).
- **Cabecera y navegación unificada**: reducción de 7 a 5 pestañas principales unificando *Servidores eD2k* y *Red Kademlia* en **Red P2P** (`.net-subview-bar` con `switchNetworkView`) y trasladando **Registro** al icono de consola `#btn-header-logs`.
- **Descargas, inspector y Cine & Series**: cinta continua multicolor `.part-ribbon` para descargas activas, ocultación del hash crudo en tabla, iconos SVG en el botón de modo compacto, inspector inferior compacto con pestañas en línea en `.inspector-header`, puntos de ruptura responsivos (`900px–1320px`) y pulido visual de **Cine & Series**.

### Servidor Estático Embebido (`crates/mule-daemon/src/rpc/server.rs`)
- **Actualización inmediata de interfaz**: `embedded_asset_handler` sirve ahora los archivos `.css` y `.js` con `NO_CACHE_CONTROL` (sin caché de 24 h), evitando que los navegadores muestren hojas de estilo o scripts antiguos tras actualizar el binario.

---

## 3. Verificación e Integridad

Suite rápida (`python3 scripts/smart_test.py --mode fast`) y despliegue en Raspberry Pi (`aarch64-unknown-linux-gnu`) validados en vivo antes de generar los paquetes distribuibles de **Windows x64**, **Linux x64** y **Linux ARM64 (`aarch64`)**.

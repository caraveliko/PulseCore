# Notas de la Versión — Pulse Core v0.4.0-beta.3

**Fecha de publicación:** 20 de noviembre de 2026  
**Estado:** Versión beta — **Sistema integral de notificaciones**, **endurecimiento Kad DHT para WAN (incluido LowID Buddy)**, **herramientas MCP/Asistente** y **arneses de pruebas consolidados**.

---

## 1. Síntesis de la Versión

La versión **v0.4.0-beta.3** introduce un sistema completo de notificaciones persistentes con canales push externos, eleva la fiabilidad y resistencia Sybil de **Kademlia (Kad)** en redes WAN reales —incorporando el protocolo completo de *LowID Buddy*—, expone la gestión de avisos a agentes de IA mediante **MCP** y consolida los arneses de compilación y pruebas para acelerar drásticamente el ciclo de desarrollo y validación.

---

## 2. Novedades y Mejoras Principales

### Sistema Integral de Notificaciones (`notifications.*`)
- **Almacenamiento persistente y tiempo real**: nueva tabla en SQLite (`notifications`) con centro de avisos en la WebUI (`web/index.html`), indicador de no leídos, toasts en vivo vía evento WebSocket `event.notificationCreated` y métodos RPC (`notifications.list`, `markRead`, `clear`, `getConfig`, `updateConfig`, `testChannel`).
- **Monitorización de eventos clave**: avisos automáticos al completarse descargas, detectar errores críticos, recuperar archivos huérfanos, encontrar actualizaciones disponibles o automatizar capturas multimedia.
- **Canales externos protegidos**: soporte nativo para Webhook genérico, Telegram, Discord, Gotify, ntfy y Pushover bajo las reglas anti-SSRF de `NetworkPolicyManager`, con redacción automática de secretos en todas las respuestas RPC.
- **Integración MCP y Asistente de IA**: seis nuevas herramientas (`emule_list_notifications`, `emule_mark_notifications_read`, `emule_clear_notifications`, `emule_get_notifications_config`, `emule_update_notifications_config`, `emule_test_notification_channel`) gobernadas por `ToolProfile` (`read|safe|full`) y la lista de capacidades de `mule-assistant`.

### Endurecimiento Kad DHT en Redes WAN
- **Cliente LowID Buddy completo (`mule-protocol` + `services/kad/buddy_client.rs`)**: codificación `KADEMLIA_FINDBUDDY_REQ` (`0x51`), apertura de enlace TCP hacia el buddy seleccionado (con ofuscación RC4 opcional), latidos periódicos `OP_BUDDYPING`/`OP_BUDDYPONG` y apertura de conexión saliente inmediata al recibir `OP_CALLBACK`.
- **Defensa anti-Sybil y estabilidad de enrutamiento (`mule-core`)**: límites globales de contactos en `KBucketTable` por dirección IP (`MAX_GLOBAL_CONTACTS_PER_IP = 1`) y por subred `/24` (`MAX_GLOBAL_CONTACTS_PER_SUBNET_24 = 10`, con exención de loopback), junto con revalidación periódica LRU de los contactos más antiguos por bucket.
- **Resistencia del plano UDP (`services/kad_udp`)**: limitador de caudal diferenciado por opcode e IP, validación estricta de IDs de transacción UDP (IP + puerto exacto con respaldo por IP para NAT simétrico) y semántica estándar de carga/rechazo en `PublishRes` (`<= 100` aceptado, `u8::MAX` en rechazo local).
- **Publicación y mantenimiento eficientes**: agrupación por lotes de palabras clave (`publish_keywords_batch`: hasta 150 archivos por término y 50 entradas por paquete con concurrencia y cadencia acotadas), *self-lookup* cada 4 h (`NODECOMPLETE`) y *jitter* de republicación (`<= 300 s`).

### Rendimiento de Pruebas y Limpieza del Repositorio
- **Arneses consolidados y corredor inteligente (`scripts/smart_test.py`)**: sustitución de decenas de binarios individuales por suites consolidadas por crate (`*_suite`), perfil `dev` ligero y modos `--mode smart|fast|full` con verificación de sintaxis JS y detección de clones B1.
- **Documentación y scripts en español de España**: traducción completa de los scripts de auditoría/bancos QA, sincronización del mapa en [`docs/INDEX.md`](../INDEX.md), especificación RPC en [`docs/architecture/rpc_protocol.md`](../architecture/rpc_protocol.md) y guías de la Wiki.

---

## 3. Verificación e Integridad

Todas las comprobaciones (`smart_test.py --mode full`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, guardia antiduplicación B1 y pruebas en loopback eD2K/Kad/MCP/Notificaciones) se ejecutan al 100 % sin conexión externa antes del despliegue en **Linux x64**, **Raspberry Pi (`aarch64`)** y **Windows x64**.

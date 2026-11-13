# Notas de la Versión — eMuleRust v0.1.8

**Fecha de publicación:** 19 de septiembre de 2026  
**Estado:** Versión de Rendimiento, Estabilidad de Enjambre y Observabilidad

---

## 1. Síntesis de la Versión

La versión **v0.1.8** de eMuleRust aporta optimizaciones determinantes para la descarga multipar en enjambre real frente a clientes eMule oficiales, el respeto estricto de los mecanismos de colas de espera remotas (*Queue Ranking*), la equidad en las subidas y una observabilidad de nivel industrial:

1. **Agregación Multipar del Caudal (`sync_transfer_rates`)**:
   - Fin de las caídas bruscas de caudal a 0 B/s: la velocidad de descarga por fichero se calcula dinámicamente como la suma instantánea de todos los pares activos que transfieren bloques para ese hash.
   - `RateResetGuard`: al desconectarse o marcharse un par, el caudal residual se recalcula de inmediato sin puesta a cero intempestiva del fichero.

2. **Gestión Conforme de las Colas de Espera Remotas (Queue Ranking eMule)**:
   - Detección del estado `Disconnected in queue`: cuando un cliente eMule remoto cierra el socket TCP tras habernos asignado un rango (`OP_QUEUERANKING`), nuestro cliente mantiene la fuente en estado `Allowed` en lugar de clasificarla como error.
   - Respeto escrupuloso del intervalo de reconsulta canónico de eMule (10 a 15 minutos) en lugar de reconexiones agresivas a 60 segundos, evitando cualquier baneo por flood en los pares.

3. **Protección de las Cuotas de Enjambre (Water-Filling)**:
   - Los pares puestos en espera en las colas remotas se integran en el recuento de fuentes viables, manteniendo estable el techo de asignación water-filling.

4. **Gestor de Subida (Upload Queue)**:
   - Soporte completo de las peticiones entrantes `OP_STARTUPLOADREQ` y `OP_ACCEPTUPLOADREQ`.
   - Asignación equitativa de las ranuras de envío según los créditos eMule acumulados (multiplicador hasta 10x).

5. **Sistema de Registros Estratégicos Normalizado en 5 Categorías**:
   - `[PART_COMPLETED]`: Notificación al completarse una parte de 9,28 MB.
   - `[DL_PROGRESS]`: Informe sintético cada 10 segundos por fichero activo.
   - `[KAD_SEARCH]`: Telemetría de búsqueda de fuentes y palabras clave DHT.
   - `[UPLOAD_...]`: Ciclo de vida de las ranuras de envío (`REQUEST`, `GRANTED`, `QUEUED`, `RELEASED`, `PROMOTED`).
   - `[SYSTEM_HEARTBEAT]`: Síntesis consolidada del estado del daemon emitida cada 60 segundos.

6. **Cartografía Exhaustiva del Repositorio**:
   - Fichero [`docs/INDEX.md`](file:///d:/Projet/eMuleRust/docs/INDEX.md) que inventaría y documenta cada fichero versionado del proyecto.

---

## 2. Comandos de Validación

```powershell
# Verificación completa de compilación y lints
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Ejecución de la suite de pruebas Rust (204+ pruebas)
cargo test --workspace --all-features

# Validación de la aplicación Flutter Desktop
cd apps/desktop
flutter analyze
flutter test
```

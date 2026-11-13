# Plan de implementación: Utilidad de migración desde aMule

**ID de funcionalidad**: `004-amule-migration-utility`  

## 1. Plan técnico
- Añadir el subcomando `ImportAmule` a `crates/mule-cli`.
- Conectar `mule-cli` a los módulos de servicio de `mule-daemon` o a la lógica directa de importación de BD/archivos.
- Procesar secuencialmente todos los archivos `.met` y `.dat`, informando del estado (recuento importado, omitidos, errores).
- Copiar o mover de forma segura los archivos `.part` y `.part.met` incompletos sin corromper los archivos originales.
- Imprimir una tabla resumen de migración formateada con colores en terminal.

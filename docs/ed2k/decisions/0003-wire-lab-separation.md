# ADR 0003: Separación Estanca entre el Protocolo de Laboratorio y el Protocolo Histórico eD2K

## Estado
Aceptado

## Fecha
2026-09-13

## Contexto
El proyecto desarrolló en la Fase 4 un banco de pruebas completo de transferencia de archivos en bucle local (`mule-protocol::lab`, magic `0xEE`, `LabMessage`) para demostrar la continuidad entre encuadre, planificación, almacenamiento temporal `.part`, cálculo de hash eD2K e integridad SQLite.

Sería peligroso y confuso intentar mezclar o extender directamente este protocolo de laboratorio para absorber las particularidades del protocolo eD2K histórico (`0xE3`, `0xC5`, `0xD4`).

## Decisión
1. `mule-protocol::lab` y `mule-protocol::ed2k` se mantienen en dos directorios de código completamente separados.
2. No se comparte ningún tipo de mensaje entre `lab` y `ed2k`. Los únicos elementos comunes son las primitivas de streaming de bajo nivel (`RawPacket`, `PacketCodec`, constantes de longitud de cabecera).
3. Los límites de trama (tamaño de paquete, tamaño de cadena, número de tags) son independientes entre ambos módulos.

## Consecuencias
- **Positivas**: El laboratorio sigue siendo una suite de pruebas robusta y aislada protegida por la feature `lab-loopback`, sin regresiones ni interferencias con la arqueología eD2K.
- **Negativas**: Ligera duplicación de estructuras de mensajes similares (p. ej.: `Hello` vs. `Ed2kHello`), pero garantizando un desacoplamiento total y una independencia semántica.

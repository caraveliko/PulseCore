# ADR 0002: Prohibición Absoluta de Conectividad de Red eD2K Externa

## Estado
Reemplazado por [ADR 0004](0004-public-ed2k-kad-production.md) (fase de desarrollo terminada — paso a producción)

## Fecha
2026-09-13

## Contexto
La red pública eD2K incluye servidores públicos heterogéneos, pares no autenticados y vectores de ataque potenciales (tramas malformadas, bombas de descompresión zlib, inundaciones de peticiones). Una exposición prematura de un nuevo cliente Rust sin validación exhaustiva de los parsers y de los límites de recursos expondría el proyecto a fallos graves.

## Decisión
1. El crate `mule-protocol::ed2k` tiene prohibición formal de crear estructuras de sockets de red (`TcpStream`, `UdpSocket`), tipos de direcciones de red activas (`SocketAddr`) o tareas de escucha/conexión asíncronas.
2. El binario `mule-daemon` y la interfaz Flutter no deben incluir ningún indicador, opción ni ruta que permita una conexión de red a un servidor o par público externo.
3. Toda la suite de pruebas eD2K debe ser ejecutable sin conexión y sin acceso a Internet.

## Consecuencias
- **Positivas**: Entorno de pruebas 100 % determinista, cero fugas de información, seguridad absoluta para el entorno de desarrollo.
- **Negativas**: Ninguna interacción directa con los pares de la red pública en esta etapa.

## Histórico
Esta prohibición cubría la fase de construcción de la aplicación. El punto 1 (pureza de `mule-protocol` — cero I/O de red en el crate) **sigue vigente**: es una propiedad arquitectónica permanente, independiente del estado de red del daemon. Los puntos 2 y 3 quedan enmendados por el ADR 0004 que autoriza la conectividad pública en producción, bajo perfil explícito.

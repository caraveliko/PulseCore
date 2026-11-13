# ADR 0001: Decodificadores eD2K de Solo Lectura sin Codificador ni Sesión Conectada

## Estado
Aceptado

## Fecha
2026-09-13

## Contexto
La integración del protocolo histórico eDonkey2000 / eMule representa un riesgo arquitectónico y de seguridad importante si se aborda mediante la escritura simultánea de clientes de red activos, máquinas de estados de sesión y codificadores. Las especificaciones históricas están dispersas en bases de código C++ de más de veinte años (eMule MFC, aMule).

## Decisión
Para la Fase 5:
1. En `mule-protocol::ed2k` solo se implementan decodificadores de solo lectura (`Ed2kDecoder`) y estructuras de datos pasivas de observación (`Ed2kHello`, `Ed2kHelloAnswer`, `Ed2kFileStatus`).
2. No se produce ningún codificador de paquetes eD2K activo.
3. No se asocia a estas estructuras ninguna máquina de estados de sesión activa (gestor de envío/recepción, temporizador de ping, negociación de subida/descarga).
4. Toda validación se efectúa exclusivamente frente a un corpus de fixtures binarias estáticas y reproducibles.

## Consecuencias
- **Positivas**: Riesgo nulo de error o efecto colateral de red, superficie de ataque mínima, auditabilidad total de los parsers antes de cualquier intento de emisión.
- **Negativas**: El cliente todavía no transfiere datos con pares eMule reales (que es el objetivo buscado de esta fase pasiva).

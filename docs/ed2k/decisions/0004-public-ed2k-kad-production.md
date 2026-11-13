# ADR 0004: Conectividad eD2K/Kad Pública en Producción

## Estado
Aceptado

## Fecha
2026-03-03

## Contexto
El [ADR 0002](0002-no-public-network.md) prohibía toda conectividad de red externa durante la fase de construcción: parsers sin probar, riesgo de exposición precoz. Esta fase ha terminado: los codecs están cubiertos por los vectores Tier 1, los pares emulados Tier 2 y, sobre todo, los **binarios de referencia reales Tier 3** (eMule 0.50a, eMule 0.72a, aMule 3.1.0 en loopback en espacio aislado). La aplicación pasa a versión de producción — un cliente eD2K sin acceso a la red real no tiene valor funcional.

## Decisión
1. **El perfil `public-ed2k-kad-preview` del daemon está autorizado en producción**: se permiten conexiones salientes hacia servidores eD2K públicos, pares eD2K y bootstrap Kad cuando este perfil está activo.
2. **El valor por defecto permanece confinado**: el flavor `standard` arranca sin conexión automática; la conectividad pública requiere una acción explícita del usuario (perfil de red / conexión a servidor). Ninguna compilación pública expone los métodos `lab.*` ni compila `lab-loopback`/`lab-tools`.
3. **Se mantiene la pureza de `mule-protocol`**: cero sockets, cero I/O de red en el crate — la conectividad vive en `mule-daemon` (inbound listener, peer sessions, `kad_udp`) bajo el control de `mule-network-policy` (anti-SSRF, presupuestos, kill switch).
4. **Suite de pruebas sin conexión por defecto**: `cargo test` no debe emitir nunca tráfico saliente. Las pruebas de red real permanecen `#[ignore]`d detrás de variables de entorno explícitas (p. ej. `EMULE_REFERENCE_BIN` para el Tier 3 externo) — nunca activadas en CI estándar.
5. **Pruebas de red real (Fase D)**: ejecutadas manualmente, supervisadas, sobre datos desechables, con recogida de métricas anonimizadas; por naturaleza no son reproducibles en CI.

## Consecuencias
- **Positivas**: el producto pasa a ser funcional como cliente eD2K/Kad; validación real de la interoperabilidad; comportamiento de red observado y probado sobre el protocolo real.
- **Negativas**: exposición a los pares no autenticados de la red pública — mitigada por las cotas de `limits.rs`, el pipeline anti-SSRF, el kill switch atómico, los presupuestos de recursos y el fuzzing de los parsers.
- **Deuda asumida**: el perfil `public-ed2k-kad-preview` conserva su sufijo «preview» hasta la validación completa del plan de pruebas `docs/qa/PLAN_TEST_COMPLET.md` (Fases A–D).

# 🛡️ Seguridad de Red y Mitigación de Abusos

La seguridad en redes P2P descentralizadas es un pilar fundamental de **Pulse Core**. Todas las conexiones entrantes y salientes atraviesan una canalización de políticas de red estricta (`crates/mule-network-policy` y `crates/mule-filter`) diseñada para mitigar ataques de red comunes, filtrado de agentes maliciosos y protección contra fugas de datos locales.

---

## 1. Canalización Anti-SSRF (*Server-Side Request Forgery*)

Los clientes P2P tradicionales solían ser vulnerables a vectores de ataque SSRF: un atacante podía distribuir un enlace `ed2k://` o un `.torrent` cuyas fuentes apuntaran a IPs de la red local del usuario (p. ej., `192.168.1.1`), al bucle local (`127.0.0.1`) o a endpoints de metadatos en la nube (`169.254.169.254`), forzando al cliente a escanear puertos internos o enviar peticiones a servicios protegidos.

Pulse Core implementa una **canalización anti-SSRF no negociable**:
- **Validación previa a la conexión**: Toda dirección IP de par o servidor descubierta en la red se valida contra un árbol de exclusión estricto antes de solicitar el socket del sistema operativo.
- **Rangos bloqueados por defecto**:
  - `127.0.0.0/8` y `::1` (bucle local).
  - `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` (redes privadas RFC 1918).
  - `169.254.0.0/16` y `fe80::/10` (*link-local* y metadatos de proveedores cloud).
  - `0.0.0.0/8`, `224.0.0.0/4` (*multicast* e IPs reservadas).
- **Aislamiento de pruebas**: Los entornos de prueba y laboratorio utilizan perfiles cerrados (`local-lab`) que permiten únicamente la comunicación entre instancias locales de prueba, impidiendo que el tráfico de desarrollo escape a Internet.

---

## 2. Filtrado de IPs Maliciosas (`ipfilter.dat`)

Pulse Core integra un motor de filtrado IP de ultra alta velocidad en memoria (`crates/mule-filter`):

- **Estructuras de Árboles de Intervalos**: Los rangos de direcciones prohibidas se cargan desde el archivo estándar `ipfilter.dat` y se compilan en memoria en una estructura de búsqueda binaria acotada.
- **Descarte a nivel de socket**: Cuando un cliente remoto inicia un handshake TCP o envía un datagrama UDP, su dirección IP se evalúa en microsegundos. Si está incluida en la lista negra, la conexión se cierra inmediatamente antes de asignar buffers de protocolo o desencriptar paquetes.
- **Actualización dinámica**: Puedes actualizar el archivo `ipfilter.dat` en cualquier momento en el directorio de datos; el daemon detectará el cambio y recargará la tabla en caliente.

---

## 3. Interruptor de Corte Atómico (*Kill Switch*)

Para usuarios que ejecutan Pulse Core a través de túneles VPN o redes protegidas, el daemon incluye un interruptor de corte atómico implementado mediante primitivas de sincronización sin bloqueo (`std::sync::atomic::AtomicBool`):

- **Activación inmediata**: Si se pierde la interfaz de red protegida o se detecta una anomalía grave en el enrutamiento, el *Kill Switch* se activa en tiempo constante $\mathcal{O}(1)$.
- **Efecto de corte**:
  - Se abortan todas las conexiones TCP activas de subida y bajada.
  - Se detiene el procesamiento de datagramas de la red Kademlia.
  - La interfaz Web y las APIs locales permanecen accesibles para diagnóstico, pero todo el tráfico exterior queda completamente sellado.

---

## 4. Protección contra DNS Rebinding en la WebUI

La interfaz web y el servidor RPC de Pulse Core escuchan por defecto en la dirección de bucle local `127.0.0.1:4660`.

Para impedir que sitios web maliciosos visitados en el navegador del usuario puedan interactuar con el daemon mediante técnicas de **DNS Rebinding** o peticiones cruzadas no autorizadas:
1. **Validación estricta de la cabecera `Host`**: El servidor HTTP Axum solo acepta peticiones cuya cabecera `Host` coincida explícitamente con `localhost:4660`, `127.0.0.1:4660` o nombres de dominio locales autorizados. Cualquier petición con un host externo (ej. `evil-rebind.attacker.com`) es rechazada con un código HTTP `403 Forbidden`.
2. **Control de `Origin` en WebSockets**: Las conexiones entrantes al endpoint `/rpc` verifican que el origen de la llamada proceda de la propia interfaz embebida o de orígenes locales explícitamente incluidos en la lista de permitidos.

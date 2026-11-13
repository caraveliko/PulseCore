# Especificación eD2K - Alcance y Fronteras Pasivas (Fase 5)

## 1. Objetivo de la Fase 5
La Fase 5 de eMuleRust está dedicada a **la arqueología de software, la especificación formal y la implementación de decodificadores de solo lectura** para el protocolo histórico eDonkey2000 (eD2K) y sus extensiones eMule.

Esta fase tiene como finalidad establecer una comprensión rigurosa, versionada y probada de las tramas binarias históricas, basándose exclusivamente en especificaciones documentadas, fixtures binarias fijadas e implementaciones de referencia de código abierto.

---

## 2. Reglas Fundamentales y Prohibiciones Formales

### 2.1 Cero Conexión de Red Externa (100 % Sin Conexión)
- **Prohibición absoluta de comunicación de red**: El submódulo `mule-protocol::ed2k` no debe **nunca** abrir ningún socket de red (`TcpStream`, `UdpSocket`), ni resolver nombres de host o direcciones IP (`SocketAddr`), ni escuchar en una interfaz de red local o pública.
- **Ninguna conexión pública**: El cliente no debe bajo ningún concepto intentar contactar con servidores eD2K públicos, pares en Internet o la red distribuida Kademlia (Kad).
- **Sin emisión de paquetes eD2K**: Durante esta fase no se crea ningún codificador activo destinado a enviarse por una red eD2K real. Solo se autorizan estructuras pasivas de observación y funciones de decodificación.

### 2.2 Desacoplamiento Total respecto al Banco de Pruebas de Laboratorio
- El protocolo de laboratorio `PR_LAB` (`0xEE`) utilizado en las fases 4.1 a 4.4 permanece confinado en `mule-protocol::lab`.
- Ningún tipo ni mensaje de `mule-protocol::ed2k` depende del módulo `lab`.
- Los límites de tramas, la gestión de identificadores y los decodificadores eD2K son completamente autónomos.

### 2.3 Ausencia de Estado de Sesión Conectada
- Las estructuras de datos eD2K producidas por los decodificadores (`Ed2kHello`, `Ed2kHelloAnswer`, `Ed2kFileStatus`) son puras estructuras de observación pasivas (`value objects`).
- Durante esta fase no se instancia ninguna máquina de estados de sesión ni ningún gestor de envío o recepción conectado para eD2K.

---

## 3. Estrategia de Validación y Criterios de Aceptación
- **Corpus fijado de fixtures**: La conformidad de los decodificadores se valida exclusivamente frente a archivos binarios versionados en `crates/mule-protocol/tests/fixtures/ed2k/`, referenciados en un manifiesto con hash SHA-256.
- **Seguridad antidenegación de servicio**: Toda decodificación verifica estrictamente los topes de tamaño de trama, longitud de cadena y cantidad de tags antes de cualquier reserva de memoria.
- **Cero pánico**: No se tolera ningún `unwrap()` ni `expect()` en la ruta de decodificación.

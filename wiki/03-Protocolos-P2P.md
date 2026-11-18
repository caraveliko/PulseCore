# 🌐 Protocolos P2P: eD2K, Kad DHT y Torrent

**Pulse Core** destaca por unificar en un único bucle de eventos asíncrono (**Tokio**) tres de los ecosistemas descentralizados más consolidados del mundo P2P: el protocolo cliente-servidor **eD2K**, la red sin servidores **Kademlia DHT** de 128 bits y los enjambres de distribución de **Torrent**.

---

## 1. Arquitectura del Motor Asíncrono

En los clientes tradicionales de eMule (desarrollados sobre C++ y MFC a principios de los 2000), las operaciones de red y el cálculo criptográfico competían con el hilo de interfaz, causando congelaciones frecuentes bajo alta concurrencia.

Pulse Core solventa esta limitación mediante:
- **E/S Asíncrona Pura**: Tokio gestiona miles de conexiones TCP y datagramas UDP mediante multiplexación `epoll` (Linux) e `IOCP` (Windows).
- **Códecs de Cable Acotados**: Todos los analizadores sintácticos de tramas (`crates/mule-protocol`) imponen límites estrictos de asignación de memoria. Cualquier paquete malicioso o sobredimensionado es descartado a nivel de códec antes de saturar el montículo (*heap*).
- **Almacenamiento Disperso Unificado**: El motor de persistencia (`crates/mule-core`) gestiona la escritura en disco de forma no secuencial mediante archivos dispersos (*sparse files*), permitiendo ensamblar piezas provenientes tanto de pares eD2K como de enjambres Torrent sobre el mismo archivo en progreso.

---

## 2. Red eD2K (eDonkey2000)

Pulse Core mantiene una fidelidad protocolaria milimétrica con el código de referencia de **eMule 0.72a**:

### Entramado y Cabeceras en Cable
Los paquetes eD2K se identifican mediante un byte de protocolo de sincronización:
- `0xE3` (`PR_ED2K`): Protocolo eD2K estándar.
- `0xC5` (`PR_EMULE`): Extensiones propietarias de eMule.
- `0xD4` (`PR_ZLIB`): Paquetes comprimidos en tránsito con zlib.

### Características implementadas
- **Compresión en cable (`OP_COMPRESSEDPART`)**: Los bloques de datos se transfieren comprimidos mediante zlib, reduciendo el consumo de ancho de banda hasta en un 30 % en archivos compresibles.
- **Intercambio de Fuentes (SX1 y SX2)**: Negociación automatizada de fuentes entre clientes remotos (`OP_REQUESTSOURCES2` y `OP_ANSWERSOURCES2`), permitiendo descubrir pares sin sobrecargar a los servidores indexadores.
- **Identificación de dos niveles (MD4 + AICH)**:
  - Cada archivo se divide en partes de **9.280.000 bytes** (~9,28 MB). Cada parte genera un resumen MD4; el hash final del archivo es el MD4 del conjunto de hashes de sus partes.
  - Para evitar descartar 9,28 MB completos ante un fallo de transmisión, se utiliza el árbol **AICH** (*Advanced Intelligent Corruption Handling*), que divide cada bloque en microfragmentos de **180 KB**, verificando y descartando únicamente el bloque exacto dañado.

---

## 3. Red Kademlia DHT (Kad v2)

La red Kademlia de eMule es un sistema distribuido que opera enteramente sobre **UDP** y no requiere ningún servidor central para la indexación ni para la búsqueda de fuentes.

### Espacio Métrico XOR de 128 bits
- Cada nodo, contacto y archivo tiene asignada una clave de 128 bits (`KadId`).
- La distancia entre dos identificadores $x$ e $y$ se calcula mediante la operación matemática bit a bit:
  $$\Delta(x, y) = x \oplus y$$
- La tabla de contactos se organiza en **K-buckets** jerárquicos basados en la distancia métrica XOR, con desalojo ordenado por antigüedad (LRU).

### Funcionalidades de Kad en Pulse Core
1. **Bootstrap Descentralizado**: Inicialización de la tabla de enrutamiento a partir de archivos de contactos conocidos (`nodes.dat`) o mediante contactos descubiertos a través de clientes eD2K conectados.
2. **Búsqueda Distribuida**: Búsqueda de fuentes de descarga y metadatos realizando consultas iterativas $\alpha$-concurrentes hacia los $k$ contactos más próximos al hash del archivo.
3. **Comprobación de Cortafuegos y Buddy System**:
   - Detección de estado tras NAT / cortafuegos mediante paquetes de prueba UDP cruzados.
   - Si el cliente no puede recibir conexiones entrantes (estado *Firewalled*), se asocia con un cliente compañero (*Buddy*) con IP pública para actuar como relé de peticiones de conexión.

---

## 4. Motor Torrent Integrado

Pulse Core integra un subsistema Torrent de alto rendimiento que opera de forma concurrente con las redes eD2K:

- **Soporte de Metadatos y Enlaces Magnet**: Resolución instantánea de hashes de información (*info-hash*) tanto para enlaces clásicos como para especificaciones híbridas.
- **Validación de Piezas**: Hashing criptográfico SHA-1 / SHA-256 a nivel de bloque, integrado en la misma cola de verificación del daemon.
- **Orquestación de Ancho de Banda**: El asignador de cuotas equilibra dinámicamente la capacidad de subida y bajada entre los enjambres de Torrent y las conexiones de eD2K para evitar la inanición (*starvation*) de cualquiera de los protocolos.

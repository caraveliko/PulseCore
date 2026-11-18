# 🧲 07. Motor Torrent y Enjambres Híbridos

**Pulse Core** integra un motor nativo de **Torrent** que convive armónicamente con las redes eD2K y Kad en el mismo servicio `mule-daemon`. En este manual aprenderás a utilizarlo y entenderás cómo gestiona las transferencias concurrentes.

---

## 1. Cómo Añadir Torrents y Enlaces Magnet

Puedes añadir transferencias de Torrent a Pulse Core de tres formas muy sencillas:

1. **Pegando un enlace Magnet**:
   - Pulsa el botón `➕` en la WebUI y pega el enlace `magnet:?xt=urn:btih:...`.
   - Pulse Core contactará con los rastreadores (*trackers*) y con la red DHT de BitTorrent para descargar los metadatos en segundo plano y comenzar la transferencia.
2. **Arrastrando un archivo `.torrent`**:
   - Arrastra directamente cualquier archivo `.torrent` desde tu explorador de archivos sobre la ventana de la WebUI de Pulse Core.
3. **A través de la API JSON-RPC / MCP**:
   - Mediante el método `downloads.add` o pidiéndoselo a tu asistente de IA conectado.

---

## 2. El Almacenamiento Disperso Unificado (*Sparse Storage*)

El gran avance arquitectónico de Pulse Core frente a tener dos programas separados (por ejemplo un eMule por un lado y un cliente Torrent por otro) reside en su **motor de almacenamiento disperso compartido**:

- Cuando se añade una descarga, el motor crea un archivo disperso (*sparse file*) que reserva el tamaño lógico sin ocupar espacio real en disco hasta que se escriben los datos.
- Las piezas verificadas se escriben directamente en su posición exacta (*offset* en bytes).
- Los algoritmos de validación aseguran que las sumas de verificación de cada pieza se calculen en hilos de trabajo independientes (`rayon` / Tokio blocking tasks), sin ralentizar el bucle de eventos de red.

---

## 3. Orquestación Dinámica de Ancho de Banda

Uno de los problemas habituales al ejecutar clientes Torrent junto con redes tradicionales es que los enjambres masivos de Torrent tienden a abrir cientos de conexiones agresivas, agotando la tabla de sockets del router y ahogando el tráfico de eD2K.

Pulse Core implementa un **asignador de presupuestos de red (`ResourceBudgets`)**:
- **Equilibrio de cuotas**: Asigna franjas de ancho de banda protegidas para asegurar que las conexiones eD2K y los paquetes de Kad DHT no sufran de inanición (*starvation*).
- **Control de conexiones máximas**: Un límite global estricto (por defecto 500 conexiones simultáneas) evita saturar la tabla NAT de routers domésticos.
- **Límites globales configurables**: Puedes fijar una tasa máxima de subida y bajada en la pestaña **Ajustes** que se aplicará conjuntamente a todo el tráfico saliente del daemon.

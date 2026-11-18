# 🕸️ 06. Red Kad DHT: Búsqueda y Transferencia sin Servidores

La red **Kad (Kademlia DHT)** es la joya de la corona del ecosistema de eMule: una red completamente descentralizada que funciona **sin un solo servidor central**. Aunque todos los servidores eD2K del mundo cayeran simultáneamente, Pulse Core seguiría buscando, conectando y descargando archivos a través de Kad.

---

## 1. ¿Cómo funciona la Red Kademlia de eMule?

A diferencia de las redes tradicionales con servidores indexadores, en Kad **cada usuario conectado actúa como un pequeño servidor**:

- **Espacio Métrico XOR de 128 bits**: Todos los nodos y todos los archivos compartidos tienen una clave única de 128 bits.
- **Distancia matemática**: La proximidad entre dos entidades no depende de su ubicación geográfica, sino de la distancia lógica calculada mediante la operación binaria XOR ($\oplus$).
- **Tablas de enrutamiento (K-Buckets)**: Tu cliente no necesita conocer a todos los millones de usuarios de la red; solo guarda en memoria una lista estructurada de contactos (organizados en cubetas según su distancia).
- **Búsqueda por saltos**: Cuando buscas un archivo, Pulse Core le pregunta a los nodos de su lista que están numéricamente más cerca del hash del archivo. Esos nodos le responden con otros aún más cercanos, encontrando las fuentes en cuestión de 3 o 4 saltos.

---

## 2. Estados de la Red Kad en la Interfaz

En la pestaña **Red Kad** de la WebUI verás un indicador de estado:

| Estado | Significado | Qué debes hacer |
| :---: | :--- | :--- |
| 🟢 **Conectado** | Todo funciona correctamente. Cientos de contactos activos en memoria. | Nada, la red está en óptimo rendimiento. |
| 🟡 **Conectando** | El cliente está buscando contactos iniciales para incorporarse al enjambre. | Espera 1 o 2 minutos. |
| 🔴 **Desconectado** | No hay comunicación con ningún nodo de la red. | Necesitas hacer **Bootstrap** (véase la sección siguiente). |

---

## 3. Cómo solucionar «Kad Desconectado» (Bootstrap)

Si acabas de instalar Pulse Core por primera vez, tu lista de contactos estará vacía y el cliente no sabrá a quién enviar el primer paquete de saludo. Para unirte a la red necesitas hacer **Bootstrap** (*arranque en frío*):

### Método 1: Bootstrap automático desde un servidor eD2K (El más fácil)
1. Conéctate a cualquier servidor en la pestaña **Servidores**.
2. Pon a descargar cualquier archivo popular o con muchas fuentes.
3. En cuanto te conectes con los primeros pares, Pulse Core extraerá sus contactos de Kad e iniciará el árbol automáticamente.

### Método 2: Cargar un archivo `nodes.dat`
1. Descarga una lista de nodos Kad de confianza (el archivo `nodes.dat` de la comunidad).
2. En la pestaña **Red Kad**, haz clic en **«Cargar nodes.dat»** y selecciona el archivo descargado.
3. El cliente leerá los contactos y se conectará en pocos segundos.

### Método 3: Bootstrap manual por IP y Puerto
Si conoces a alguien que esté ejecutando eMule o Pulse Core con IP pública:
1. Pídele su IP pública y su puerto UDP de Kad (ej. `82.120.45.10:39737`).
2. En el recuadro **Bootstrap manual**, introduce la dirección y pulsa **Conectar**.

---

## 4. Estado «Tras Cortafuegos» (*Firewalled*) y el Sistema de Compañero (*Buddy*)

Al igual que en eD2K existe la ID Baja, en Kad puedes encontrarte con el estado:
> **Estado de Cortafuegos: Tras cortafuegos (*Firewalled*)**

Esto significa que tu puerto UDP `39737` no puede recibir datagramas directos desde el exterior.

### ¿Cómo soluciona Pulse Core este problema? (El protocolo *Buddy Client*)
Si Pulse Core detecta que estás tras un cortafuegos estricto:
1. Busca automáticamente en la red a otro usuario con IP pública y estado abierto (*Open*).
2. Le solicita establecer una relación de **Compañero (*Buddy*)**.
3. El cliente compañero actúa como una «antena de relevo»: cuando otro usuario quiera contactar contigo para enviarte un archivo, enviará la petición a tu compañero, y este te avisará inmediatamente a través de la conexión establecida.
4. Gracias a esta arquitectura, **puedes seguir descargando por Kad incluso si tu router bloquea el tráfico entrante**.

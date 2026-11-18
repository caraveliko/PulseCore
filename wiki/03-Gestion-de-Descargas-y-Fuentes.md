# 📥 03. Gestión de Descargas, Fuentes y Enjambres

En esta guía aprenderás cómo funciona internamente el motor de transferencias de **Pulse Core**, cómo interpretar con precisión la barra de progreso y cómo se gestionan las fuentes y la integridad de los datos.

---

## 1. Código de Colores de la Barra de Progreso

La barra gráfica de progreso de Pulse Core está dividida en segmentos verticales que representan las distintas partes del archivo:

| Color de la franja | Significado operativo |
| :---: | :--- |
| ⬛ **Negro / Gris oscuro** | **Parte completada**: Este fragmento ya ha sido descargado, verificado criptográficamente y escrito en disco. |
| 🟦 **Azul oscuro intenso** | **Alta disponibilidad**: Esta parte aún no la tienes, pero muchas de las fuentes conectadas la tienen disponible para compartir. |
| 🔷 **Azul claro / Celeste** | **Disponibilidad media**: Pocas fuentes tienen esta parte. |
| 🟥 **Rojo** | **Parte no disponible**: Ninguna de las fuentes que tienes actualmente conectadas posee este fragmento. La descarga esperará a que aparezca una nueva fuente que lo tenga. |
| 🟨 **Amarillo** | **Descarga activa**: Esta parte exacta se está transfiriendo en este instante desde uno de los pares remotos. |
| 🟩 **Verde continuo** | **Descarga 100 % finalizada**: Todos los fragmentos han sido completados y ensamblados en la carpeta `Incoming`. |

---

## 2. Anatomía de un Archivo eD2K: Partes y Bloques

Para entender por qué una descarga puede pausar unos segundos o avanzar a saltos:

1. **Partes de 9,28 MB (9.280.000 bytes)**:
   - Todo archivo se divide en partes completas de ~9,28 MB.
   - Cada parte tiene su propio hash MD4 individual.
2. **Bloques de Transferencia de 180 KB**:
   - Cada parte de 9,28 MB se descarga a su vez en pequeños bloques de **180 KB** (el tamaño estándar de solicitud en red).
   - Puedes estar descargando bloques de diferentes partes de forma simultánea desde múltiples fuentes a la vez.

---

## 3. El Sistema de Créditos y Colas Remotas

A diferencia de los protocolos de descarga directa por HTTP, las redes eD2K funcionan mediante un **sistema de colas descentralizado y meritocrático**:

### Cómo funciona una cola eD2K
1. Cuando encuentras una fuente, Pulse Core se conecta a ella y solicita una posición en su cola de subida (`OP_QUEUERANKING`).
2. Recibirás un número de turno en cola (ej. puesto 350).
3. Conforme el usuario remoto va despachando transferencias, tu número de turno va bajando (350 ➔ 200 ➔ 50 ➔ 1).
4. Al llegar al puesto 1, el par remoto abre la conexión de transferencia y comienza a enviarte datos.

### El multiplicador de créditos
Pulse Core almacena criptográficamente los créditos que te otorgan otros usuarios cuando tú les subes datos.
- **Cuanto más subes a la red, más créditos ganas**.
- Cuando solicitas un archivo a alguien a quien previamente le has subido datos, tu posición en su cola avanza hasta **10 veces más rápido**.
- El sistema de créditos premia a los usuarios que comparten y penaliza a quienes cierran la subida (*leechers*).

---

## 4. Algoritmo de Partes Raras (*Rarest First*)

Para asegurar que los archivos poco comunes no desaparezcan de la red:
- Pulse Core analiza la disponibilidad de cada fragmento entre todos los pares descubiertos.
- En lugar de descargar el archivo estrictamente de principio a fin, el motor prioriza **solicitar primero aquellas partes que tienen menos fuentes**.
- Esto garantiza que en cuanto obtengas esa parte rara, tu cliente la compartirá inmediatamente con el resto del enjambre, salvando el archivo de la extinción.

---

## 5. Recuperación de Partes Corruptas: ICH y AICH

En las redes P2P masivas, un paquete de red puede dañarse por pérdidas de conexión o clientes defectuosos.

Pulse Core cuenta con un sistema de doble barrera de recuperación:

### 1. ICH (*Intelligent Corruption Handling*)
Cuando se completa una parte de 9,28 MB, el daemon calcula su hash MD4 y lo compara con el metadato original:
- Si el hash coincide, la parte se marca como completada (negro).
- Si no coincide (parte corrupta), en un cliente antiguo tendrías que volver a descargar los 9,28 MB completos. Aquí entra en juego AICH.

### 2. AICH (*Advanced Intelligent Corruption Handling*)
AICH construye un árbol binario de hashes SHA-1 a nivel de microbloques de 180 KB:
- Al detectar que una parte está corrupta, Pulse Core pide a las fuentes el hash de cada microbloque de 180 KB.
- Descarta **únicamente el bloque exacto de 180 KB defectuoso** y conserva los otros 9,1 MB sanos.
- ¡Ahorra hasta un 98 % de ancho de banda y evita frustraciones en archivos grandes!

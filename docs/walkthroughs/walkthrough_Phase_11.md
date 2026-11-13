# Guía de Ejecución y Validación — Fase 11: Enjambre Multifuente, Asignador Equitativo y Arranque de Baja Latencia

Este documento constituye la guía técnica de referencia para la **Fase 11** (versiones `v0.1.4` y `v0.1.5`) del proyecto **eMuleRust**, dedicada a la orquestación dinámica de enjambres de pares y a la aceleración del arranque de las descargas:
* **SwarmCoordinator Autónomo**: Gestión de enjambres multifuente, autodescubrimiento cadenciado híbrido (Servidores eD2K + Kad DHT) y supervisión del ciclo de vida de los pares.
* **Asignador Dinámico Water-Filling**: Algoritmo **Weighted Max-Min Fair-Share** para el reparto óptimo y equitativo de las conexiones de red globales entre ficheros concurrentes.
* **Optimización de la Latencia de Arranque**: Detección acelerada de fuentes (30s / 60s / 300s), tolerancia suave ante tiempos de espera agotados de estado (`Tried` con reintento a los 45s) y reconsulta rápida de la cola de espera (3 min).
* **Fidelidad Wire y Retrocompatibilidad**: Soporte de etiquetas eD2K de tipo fijo `0x11..0x7F` (`TAGTYPE_STR1..n`), decodificación Kad heredada `0x00` y paralelización del bootstrap.

---

## 1. Arquitectura del Enjambre Multifuente

```mermaid
flowchart TB
    subgraph Engine ["Daemon eMuleRust (Tokio Runtime)"]
        SC["SwarmCoordinator (Bucle autónomo 3s)"]
        SA["SwarmAllocator (Weighted Max-Min Fair-Share Water-filling)"]
        PM["PeerSessionManager (Conexiones TCP Paralelas)"]
        KadSvc["KadService (Descubrimiento DHT)"]
        ServerSvc["ServerSessionManager (Descubrimiento Servidor)"]
    end

    subgraph Storage ["Persistencia y Datos"]
        DB[(SQLite WAL : downloads, peer_sources)]
        PartFiles["data/temp/*.part (Sparse Files)"]
    end

    subgraph External ["Pares Remotos (Red eD2K y Kad)"]
        PeerA["Par A (eMule 0.50a - HighID)"]
        PeerB["Par B (aMule 2.3.3)"]
        PeerC["Par C (eMule Community)"]
    end

    SC -->|1. Evalúa la demanda por fichero| SA
    SA -->|2. Asigna los cupos de ranuras| SC
    SC -->|3. Selecciona las fuentes viables| DB
    SC -->|4. Inicia las sesiones necesarias| PM
    PM <==>|TCP eD2K / Ofuscación RC4| PeerA & PeerB & PeerC
    PM -->|5. Escribe los bloques recibidos (180 KB)| PartFiles

    SC -.->|Autodescubrimiento cadenciado 30s/60s/300s| KadSvc & ServerSvc
    KadSvc & ServerSvc -.->|Nuevas fuentes descubiertas| DB
```

---

## 2. Algoritmo de Asignación: Weighted Max-Min Fair-Share Water-Filling

El módulo [`crates/mule-daemon/src/services/swarm_allocator.rs`](file:///d:/Projet/eMuleRust/crates/mule-daemon/src/services/swarm_allocator.rs) resuelve el problema clásico de contención de las conexiones de red globales cuando varias descargas corren simultáneamente.

### 2.1 Modelo de Demanda (`FileDemand`)
Cada descarga activa declara su demanda:
* **Ponderación por Prioridad (`priority_weight`)**:
  * `Low`: peso 1
  * `Normal`: peso 2
  * `High`: peso 4
  * `Release`: peso 8
* **Techo de viabilidad (`effective_cap`)**:  
  $$\text{effective\_cap} = \min(\text{max\_peers\_per\_file},\ \text{viable\_sources\_count},\ \text{remaining\_parts} \times 3)$$
  * Si a un fichero solo le quedan 2 partes de 9,28 MB por terminar, no monopoliza inútilmente 40 ranuras: su asignación queda acotada a $2 \times 3 = 6$ pares (protección *endgame anti-hoarding*).

### 2.2 Procedimiento de Rellenado (Water-Filling)
1. **Asignación del suelo garantizado**: Cada fichero que disponga de fuentes viables recibe de inmediato un suelo de $\min(\text{min\_peers\_per\_file}, \text{effective\_cap})$.
2. **Distribución iterativa del excedente**: La cuota global residual ($\text{budget\_restant} = \text{max\_global\_peers} - \sum \text{planchers}$) se distribuye de forma continua y proporcional al peso de prioridad de los ficheros aún no saturados.
3. En cuanto un fichero alcanza su `effective_cap`, queda congelado y el excedente de agua (ranuras) se vierte sobre las demás descargas necesitadas.

---

## 3. Aceleración del Arranque y Gestión Resiliente de Tiempos de Espera

### 3.1 Descubrimiento Dinámico de Fuentes
Anteriormente, el coordinador consultaba las fuentes cada 5 minutos fijos. En un fichero recién añadido con 0 fuentes, la descarga podía permanecer inactiva varios minutos. El nuevo planificador dinámico adapta su frecuencia:
* **0 fuentes conocidas**: reconsulta inmediata cada **30 segundos**.
* **1 a 14 fuentes**: reconsulta cada **60 segundos**.
* **15+ fuentes** (enjambre bien nutrido): reconsulta de crucero cada **300 segundos** (5 minutos).

### 3.2 Gestión Suave de los Timeouts de Estado
Cuando un par remoto no responde de inmediato a la petición de disponibilidad de partes (`OP_FILESTATUSREQ`):
* Ya no se le banea agresivamente con una penalización de una hora.
* Se le degrada al estado flexible `Tried` con un plazo de reintento corto de **45 segundos**.

### 3.3 Tratamiento Directo de la Cola de Espera eMule
Cuando un cliente remoto recibe nuestra petición pero sus ranuras de subida están llenas, responde de inmediato con `OP_QUEUERANKING` (nuestro rango en su cola) o `OP_ACCEPTUPLOADREQ`.
Estos paquetes se interceptan ahora directamente desde la fase de handshake, eliminando los falsos timeouts de 20 segundos que provocaban desconexiones intempestivas.

---

## 4. Fidelidad Wire y Retrocompatibilidad

### 4.1 Decodificación de Cadenas Fijas `TAGTYPE_STR1..n` ([tags.rs](file:///d:/Projet/eMuleRust/crates/mule-protocol/src/ed2k/tags.rs))
* **Anomalía histórica resuelta**: Los servidores eD2K Lugdunum emiten paquetes `OP_SERVERIDENT` que contienen etiquetas de tipo `0x11` a `0x7F`. Estos tipos representan cadenas de tamaño fijo sin prefijo de longitud `u16`.
* **Regla wire**:  
  $$\text{Tamaño de la cadena} = \text{TagType} - 0x10$$
  * Ejemplo: `0x19` corresponde a una cadena fija de $0x19 - 0x10 = 9$ bytes.

### 4.2 Arranque Kad Legacy ([codec.rs](file:///d:/Projet/eMuleRust/crates/mule-protocol/src/kad/codec.rs))
* Soporte del paquete histórico `KADEMLIA_BOOTSTRAP_REQ_OLD` (`0x00`) garantizando la compatibilidad con los clientes eMule ancestrales.
* Paralelización del bootstrap (`bootstrap_active_contacts`): consulta concurrente mediante `futures_util::future::join_all` que reduce el tiempo de arranque de 50 segundos a **menos de 5 segundos**.

---

## 5. Comandos de Validación (100 % Sin Conexión)

```powershell
# 1. Pruebas unitarias del asignador de enjambre y del algoritmo water-filling
cargo test -p mule-daemon --lib services::swarm_allocator

# 2. Pruebas de descubrimiento de fuentes y de ofuscación eD2K
cargo test -p mule-daemon --test ed2k_source_discovery --test ed2k_obfuscation_tests

# 3. Pruebas de decodificación de etiquetas fijas y codecs Kad heredados
cargo test -p mule-protocol --test server_codec_tests --test kad_codec_tests

# 4. Pruebas de integración del coordinador y de las sesiones de pares
cargo test -p mule-daemon --test peer_connection_tests --test swarm_dual_peer_tests
```

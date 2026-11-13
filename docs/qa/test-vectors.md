# Documentación de los Vectores de Prueba y Procedencia

Este documento recopila la procedencia, los parámetros y los hashes de referencia de los vectores de prueba utilizados para validar `mule-format`.

---

## 1. Generador Determinista (`PatternReader`)

Con el fin de evitar almacenar ficheros voluminosos (varias decenas de megabytes) en el historial de Git, las pruebas aprovechan el generador cíclico estándar:

* **Patrón base**: Secuencia inmutable de bytes `0x00, 0x01, 0x02, ..., 0xFF` (256 bytes).
* **Repetición**: Cíclica hasta alcanzar la longitud objetivo exacta.
* **Garantía**: Determinista, reproducible de forma independiente en cualquier plataforma, sin asignación excesiva de memoria RAM ($O(1)$ RAM).

---

## 2. Tabla de Fixtures de Referencia

| Identificador | Longitud (bytes) | Hash eD2K Esperado (hex) | Partes Esperadas | Regla / Caso Límite Validado |
| :--- | :--- | :--- | :--- | :--- |
| `pattern_1_byte` | `1` | `47c61a0fa8738ba77308a8a600f88e4b` | 1 | Un único byte no nulo (`0x00`). |
| `pattern_9727999` | `9 727 999` | `4f2dc92b1fd24bde92dd01d85154b06c` | 1 | Tamaño límite justo por debajo de una parte eD2K completa ($n < 9\,728\,000$). |
| `pattern_9728000` | `9 728 000` | `d815a0409aaa0b4279dd86e580bb043f` | 2 | Exactamente una parte eD2K ($n = 9\,728\,000$). Se añade `MD4(empty)`. |
| `pattern_9728001` | `9 728 001` | `695b81b1727666818988524bd7fc823c` | 2 | Una parte completa + 1 byte residual. |
| `pattern_19456000` | `19 456 000` | `7fdd19736858b9f24c65da0e0e0adba3` | 3 | Exactamente 2 partes eD2K ($2 \times 9\,728\,000$). Se añade `MD4(empty)`. |

---

## 3. Regla del Hashset eMule en Tamaños Múltiplos

El algoritmo de hash de eMule presenta una particularidad histórica documentada:
* Cuando un fichero mide **exactamente un múltiplo** de `9 728 000` bytes ($k \times 9\,728\,000$ con $k \ge 1$), se inserta un bloque adicional de tamaño cero en la lista de hashes de parte antes de calcular el hash raíz final.
* El hash de este bloque vacío es el resumen MD4 estándar: `31d6cfe0d16ae931b73c59d7e0c089c0`.
* Para un fichero de exactamente $9\,728\,000$ bytes, la lista de partes incluye por tanto:
  1. `part_0`: MD4 de los primeros $9\,728\,000$ bytes.
  2. `part_1`: `31d6cfe0d16ae931b73c59d7e0c089c0` (bloque vacío).
  El hash raíz es $\text{MD4}(\text{part}_0 \mathbin{\Vert} \text{part}_1)$.

---

## 4. Árbol de Merkle AICH (180 KiB)

* **Tamaño de un bloque base**: `184 320` bytes ($180\text{ KiB}$).
* **Parte completa**: 53 hojas ($52 \times 184\,320 + 143\,360 = 9\,728\,000$).
* **Verificación de Prueba (`AichProof`)**:
  * Validación estructural: validación del número de hojas (`leaf_count >= 1`), índice (`leaf_index < leaf_count`), profundidad exacta y direcciones de los hermanos (`SiblingPosition::Left` / `Right`) antes del cálculo criptográfico.
  * Validación criptográfica: ascenso SHA-1 desde la hoja hacia la raíz de Merkle.

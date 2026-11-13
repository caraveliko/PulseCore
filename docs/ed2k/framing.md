# Especificación de Encuadre (Framing) del Protocolo eD2K

## 1. Estructura General de una Trama de Red

Toda comunicación eD2K (TCP) se divide en tramas binarias prefijadas por una cabecera de 5 bytes:

```text
+---------------+-------------------------------+---------------+-----------------------+
| Protocol (u8) |    Packet Length (u32 LE)     |  Opcode (u8)  |    Payload Bytes      |
|    1 byte     |           4 bytes             |    1 byte     |  (Length - 1) bytes   |
+---------------+-------------------------------+---------------+-----------------------+
```

### 1.1 Campos de la Cabecera (Header)
1. **Protocol Byte (1 byte)**:
   - `0xE3`: Protocolo estándar eDonkey2000 (`PR_ED2K` / `OP_EDONKEYPROT`).
   - `0xC5`: Extensiones eMule (`PR_EMULE` / `OP_EMULEPROT`).
   - `0xD4`: Tramas comprimidas mediante zlib/deflate (`PR_EMULE_COMPRESSED` / `OP_PACKEDPROT`).
2. **Packet Length (4 bytes, entero sin signo Little-Endian)**:
   - Define el número total de bytes que siguen al campo de longitud en la trama.
   - **Fórmula esencial**:
     $$\text{Packet Length} = 1 \text{ (Opcode)} + \text{longitud del Payload}$$
     $$\text{Tamaño total en el cable (Wire Length)} = 5 \text{ (Header)} + \text{Packet Length} = 6 + \text{longitud del Payload}$$
   - **Restricción mínima**: Todo paquete válido debe contener al menos 1 byte para el opcode. Un valor de `Packet Length < 1` se considera inmediatamente una violación de protocolo (`FrameTooSmall`).
3. **Opcode (1 byte)**:
   - Código de operación que designa la semántica del mensaje.
4. **Payload (Tamaño variable)**:
   - Datos de aplicación serializados, de longitud $\text{Packet Length} - 1$.

---

## 2. Endianness (Orden de Bytes)

El protocolo eDonkey2000 y sus derivados eMule utilizan **exclusivamente el orden de bytes little-endian** para todos los enteros numéricos transmitidos por la red:
- `u16`: 2 bytes little-endian.
- `u32`: 4 bytes little-endian (en particular, las direcciones IPv4 en los Client IDs y Server IPs, así como las longitudes de cadenas y contadores de tags).
- `u64`: 8 bytes little-endian (tamaños de archivos grandes > 4 GB en las extensiones de tags).
- `f32`: Flotantes IEEE 754 de 32 bits en formato little-endian.

Las huellas criptográficas (hashes MD4 de 16 bytes para usuarios y archivos) se almacenan y transmiten como arrays brutos de bytes sin reordenamiento (`[u8; 16]`).

---

## 3. Separación de Límites: eD2K vs. Laboratorio

El banco de pruebas de laboratorio (`mule-protocol::lab`) utiliza un protocolo simplificado `0xEE` acotado para transferencias de bloques de 180 KiB.
El protocolo eD2K histórico (`0xE3`, `0xC5`, `0xD4`) posee límites distintos:

| Parámetro | Protocolo Laboratorio (`lab`) | Protocolo eD2K Pasivo (`ed2k`) | Justificación eD2K |
| :--- | :--- | :--- | :--- |
| **Trama máxima por defecto** | 1 048 576 bytes (1 MiB) | 262 144 bytes (256 KiB) | Suficiente para bloques de 180 KiB + overhead |
| **Trama Hello máxima** | 65 536 bytes (64 KiB) | 65 536 bytes (64 KiB) | Prevención de ataque por denegación de servicio (tags abusivos) |
| **Tamaño máximo de cadena** | 16 384 bytes | 1 024 bytes | Nombres de archivo y seudónimos habituales ≤ 512 o 1024 B |
| **Número máximo de tags** | 256 | 256 | Límite histórico suficiente para el apretón de manos |
| **Tramas comprimidas (0xD4)** | Rechazo explícito | Rechazo explícito | Prohibido hasta la fase de auditoría hostil de descompresión |

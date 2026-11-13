# Especificación de Límites Estrictos eD2K y Protección Anti-OOM

Este documento formaliza los topes de tamaño y cardinalidad aplicados por los decodificadores de `mule-protocol::ed2k` para protegerse frente a entradas maliciosas, desbordamientos de memoria (OOM) y ataques por denegación de servicio.

---

## 1. Principios Rectores

1. **Verificación Previa (Pre-allocation Check)**:
   No se reserva ningún vector dinámico ni cadena de caracteres basándose en un entero leído de la red sin haber sido previamente acotado por una constante estricta.
2. **Cotas Específicas por Opcode**:
   Cada mensaje dispone de un tope correspondiente a su semántica de negocio en lugar de un límite global demasiado generoso.
3. **Ausencia de Ambigüedad sobre Tipos Desconocidos**:
   Los datos desconocidos se limitan en tamaño máximo antes de encapsularse en un contenedor bruto (`RawTag`).

---

## 2. Tabla de Topes Aplicados

| Parámetro | Tope Estricto | Contexto / Opcode | Consecuencia en caso de superación |
| :--- | :---: | :--- | :--- |
| **Tamaño de Trama Hello** | 65 536 bytes (64 KiB) | `OP_HELLO`, `OP_HELLOANSWER` | `Ed2kParseError::FrameTooLarge` |
| **Tamaño de Trama Part / Data** | 262 144 bytes (256 KiB) | `OP_SENDINGPART` | `Ed2kParseError::FrameTooLarge` |
| **Tamaño de Trama Metadatos** | 65 536 bytes (64 KiB) | `OP_FILESTATUS`, `OP_HASHSETANSWER` | `Ed2kParseError::FrameTooLarge` |
| **Número Máximo de Tags** | 256 tags | Todos los mensajes que contienen tags | `Ed2kParseError::TooManyTags` |
| **Longitud de Cadena Textual** | 1 024 bytes | Nombres de usuario, software, archivos | `Ed2kParseError::OversizedString` |
| **Número Máximo de Parts** | 65 535 parts | `OP_FILESTATUS` | `Ed2kParseError::MalformedField` |
| **Tamaño Máximo de Blob de Tag**| 65 536 bytes | Tags de tipo Blob (`0x07`) | `Ed2kParseError::FrameTooLarge` |

---

## 3. Detalle de las Verificaciones durante el Parsing de Tags

Cada tag TLV (Type-Length-Value) se decodifica según los siguientes pasos:
1. **Lectura del Tipo de Tag (1 byte)**:
   - Rechazo si fin de flujo inesperado (`UnexpectedEof`).
2. **Lectura del Nombre de Tag**:
   - Longitud del nombre de tag (2 bytes LE).
   - Si $\text{longitud} == 1$: lectura de un identificador numérico especial (1 byte).
   - Si $\text{longitud} > 1$: verificación de que $\text{longitud} \le 1024$ y, a continuación, extracción de la cadena UTF-8 con tolerancia.
3. **Lectura del Valor según el Tipo**:
   - `0x01` (Hash): 16 bytes fijos.
   - `0x02` (String): lectura de la longitud (2 bytes LE) $\le 1024$ y, después, de los bytes.
   - `0x03` (UInt32): 4 bytes LE.
   - `0x04` (Float32): 4 bytes LE.
   - `0x06` (Bool): 1 byte.
   - `0x07` (Blob): lectura de la longitud (4 bytes LE) $\le 65536$ y, después, de los bytes.
   - `0x08` (UInt16): 2 bytes LE.
   - `0x09` (UInt8): 1 byte.
   - `0x0A` (UInt64): 8 bytes LE.
   - Otro / Desconocido: el tag se conserva como `RawTag` si la longitud puede delimitarse, o provoca un error `InvalidTagType`.

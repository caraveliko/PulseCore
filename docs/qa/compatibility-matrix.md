# Matriz de Compatibilidad y Especificaciones Protocolarias

Este documento formaliza las decisiones de interoperabilidad y las garantías del motor eMule moderno.

---

## 1. Algoritmos de Hash e Integridad

| Componente | Especificación eMule / aMule | Soporte en `mule-format` | Notas / Casos Límite |
| :--- | :--- | :--- | :--- |
| **Tamaño de una parte eD2K** | Exactamente `9 728 000` bytes (9 500 KiB) | **Conforme** (`ED2K_PART_SIZE`) | Constante entera estricta. |
| **Hash eD2K ($0 < n < 9\,728\,000$)** | `MD4(file)` | **Conforme** | Un único resumen MD4 para todo el fichero. |
| **Hash eD2K ($n = 9\,728\,000$)** | `MD4(MD4(part_0) \|\| MD4(empty))` | **Conforme** | Tratado como un hashset añadiendo `MD4("")`. |
| **Hash eD2K ($n = k \times 9\,728\,000$)** | `MD4(part_0 \|\| ... \|\| MD4(empty))` | **Conforme** | Se conserva el comportamiento oficial de eMule. |
| **Fichero vacío ($n = 0$)** | Rechazado por eMule | **Rechazado** (`FormatError::EmptyInput`) | Considerado como descarga no válida. |
| **Bloque base AICH** | `184 320` bytes (180 KiB) | **Conforme** (`AICH_BLOCK_SIZE`) | Árbol de Merkle binario basado en SHA-1. |
| **Parte eD2K completa AICH** | 53 hojas (52 de 180 KiB + 1 de 143 360 B) | **Conforme** | Segmentación conforme a la implementación de referencia. |
| **Prueba de auditoría Merkle AICH** | Validación de un bloque de 180 KiB corrupto | **Conforme** (`AichProof`) | Permite la recuperación selectiva de un bloque corrupto. |

---

## 2. Formatos de intercambio y almacenamiento

| Formato | Estado de lectura | Estado de escritura | Rol en la arquitectura |
| :--- | :--- | :--- | :--- |
| **Enlaces `ed2k://`** | Completo (estricto + etiquetas desconocidas) | Canónico (`to_link_string`) | Entrada de usuario, importaciones, compartidos. |
| **Ficheros `.part.met`** | Soportado (`0xe0`, `0xe1`, `0xe2`, gaps con nombre eMule) | **Soportado** (`write_to`/`save`, atómico `.tmp`+renombrado) | Importación directa de descargas existentes; escritura compatible con `CPartFile::SavePartFile`. |
| **Árbol AICH (`.aich`)** | Previsto | Previsto | Caché local de los árboles de Merkle. |
| **Estado interno `mule-core`** | Versionado (SQLite transaccional) | Versionado (SQLite transaccional) | Persistencia moderna desacoplada de los formatos heredados. |

---

## 3. Desglose Arquitectónico

```
┌─────────────────────────────────────────────────────────────┐
│                 apps/desktop (Flutter)                      │
│          Interfaz gráfica de usuario moderna                │
└──────────────────────────────┬──────────────────────────────┘
                               │ WebSocket / JSON-RPC local
┌──────────────────────────────▼──────────────────────────────┐
│                    crates/mule-daemon                       │
│     Proceso en segundo plano, control de API local          │
├─────────────────────────────────────────────────────────────┤
│                    crates/mule-core                         │
│     Gestor de descargas, piezas, colas, créditos            │
├─────────────────────────────────────────────────────────────┤
│                   crates/mule-format                        │
│     Hash determinista, decodificación binaria, parseo       │
└─────────────────────────────────────────────────────────────┘
```

# Etapa 6 — Contención: instantánea + ordenación fuera de bloqueo

> **Dependencias**: ninguna. Etapa pequeña y quirúrgica — intercalar cuando se quiera.
> **Prerrequisito**: **medir primero** — añadir un registro de duración de retención del mutex antes de optimizar más allá.

## Objetivo

`find_closest*` = `collect_all()` + ordenación completa **bajo** el `Mutex` global (`routing_table.rs:229-251`). Con ~1 300 contactos y búsquedas/publicaciones/bootstrap concurrentes, la sección crítica es innecesariamente larga.

## Cambios

### 6a. `mule-core/src/kad/routing_table.rs`

Hacer accesible `collect_all`: `pub fn snapshot_all(&self) -> Vec<KadContact>` (o `pub` directo si es coherente con la API). Copia `Vec` bajo bloqueo = O(n) memcpy — mucho más corta que recogida **+** ordenación.

### 6b. Callers — ordenar fuera de bloqueo

| Sitio | Antes | Después |
|---|---|---|
| `kad.rs` `bootstrap_active_contacts` (l.472-475) | `find_closest` bajo bloqueo | `snapshot_all` bajo bloqueo → `sort_by_key(\|c\| local_id.distance_to(c.kad_id))` + `take(n)` fuera de bloqueo |
| `lookup.rs` seeds (`converge_closest_contacts`) | ídem | ídem |
| `spread_verified` / selecciones internas | ídem | ídem si es posible salir del bloqueo |

**Conservar las variantes `find_verified_closest`** para las rutas donde el filtro `verified` debe ser coherente con la mutación (`learn_contact` bajo el mismo lock) — decidir caso por caso, documentar.

### 6c. Instrumentación

`tracing::debug!` de la duración de retención del lock en los sitios calientes (antes/después) — la métrica decide si una etapa posterior (`RwLock`, bloqueos por bucket) está justificada. **No** implementar ninguna estructura concurrente fina sin métrica que la demande: los invariantes LRU/replacement cache valen más que una ganancia especulativa.

## Pruebas

- Mismas pruebas que la tabla (orden de clasificación idéntico antes/después — `sort_by_key` estable sobre la misma clave).
- Prueba de contención: N tareas en `snapshot_all` concurrente + inserciones — sin interbloqueo, resultados coherentes.

## Criterios de finalización

- [ ] Duración de retención del mutex medida y reducida en las rutas calientes.
- [ ] Orden de los contactos devueltos idéntico a las pruebas existentes.
- [ ] Sin `RwLock`/bucketing fino sin métrica que lo justifique.

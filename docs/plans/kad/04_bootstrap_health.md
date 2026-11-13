# Etapa 4 — Salud del bootstrap (`BootstrapHealth`)

> **Dependencias**: etapas 1–3 — el probing produce las métricas (edad `last_seen`, tasa de fallos) de las que se alimenta la salud.
> **Naturaleza**: mejora **más allá** de eMule, puramente local — la puerta actual ya es fiel a eMule (`Kademlia.cpp:287`: `!IsConnected()` = `HasHadContact()`, cadencias 2 s/15 s) y el mantenimiento (30 s, `kad.rs:677-688`) ya amplía mientras `0 < contacts < 400`. **No es una corrección de error, es un refinamiento de cadencia.**

## Objetivo

Reemplazar el booleano `has_verified` (`kad.rs:586-595`) por un estado de salud: un solo contacto verificado no debe bastar para pasar a cadencia «healthy» — una tabla con 3 contactos mal distribuidos es funcionalmente frágil aunque un par haya respondido una vez.

## Cambios

### 4a. Nuevo archivo `crates/mule-daemon/src/services/kad/bootstrap_health.rs`

```rust
pub enum BootstrapHealth { Empty, Fragile, Building, Healthy }

pub struct BootstrapHealthSnapshot {
    pub health: BootstrapHealth,
    pub verified: usize,
    pub nonempty_buckets: usize,
    pub distinct_subnet24: usize,
    pub recently_seen: usize,   // last_seen < RECENT_WINDOW
}

pub fn assess(table: &KBucketTable, local_id: &KadId) -> BootstrapHealthSnapshot
```

Criterios indicativos (constantes al principio del archivo — ajustar según medición):

| Estado | Criterio | Cadencia drenaje |
|---|---|---|
| `Empty` | `verified == 0` | `BOOTSTRAP_DRAIN_EMPTY = 2 s` |
| `Fragile` | `1..15` verificados **o** `nonempty_buckets < 8` **o** `/24 < 4` | `BOOTSTRAP_DRAIN_FRAGILE = 5 s` |
| `Building` | `15..50` verificados | `BOOTSTRAP_DRAIN_BUILDING = 15 s` (eMule) |
| `Healthy` | `≥ 50` verificados, diversidad suficiente | skip (`continue`, comportamiento actual) |

### 4b. `kad.rs`

- **Drenaje** (l.586-625): `let snap = assess(&table, &local_id)`; `Healthy` → `continue`; si no intervalo por estado en lugar del `min_interval` binario.
- **Mantenimiento** (l.677): umbral de expansión modulado — `Fragile` → `bootstrap_active_contacts(BOOTSTRAP_SAMPLE_FRAGILE)` más amplio que el `15` actual.
- Registro `kad.bootstrap.health = "fragile"` en cada transición de estado.

### 4c. `Degraded` (estado opcional — tras etapa 5)

Una vez disponible la taxonomía de errores: `Degraded` = caída reciente de contactos verificados O tasa de timeout elevada → retorno temporal a cadencia `Fragile`. No implementarlo hasta que la etapa 5 esté hecha.

## Constantes (`bootstrap_health.rs`)

`HEALTH_FRAGILE_MAX_VERIFIED`, `HEALTH_MIN_BUCKETS`, `HEALTH_MIN_SUBNETS`, `RECENT_WINDOW`, `BOOTSTRAP_DRAIN_*`, `BOOTSTRAP_SAMPLE_FRAGILE`.

## Pruebas

- `assess`: tablas sintéticas → cada estado (verificados=0; 3 verificados/1 bucket → Fragile pese a >0; 60 verificados/30 buckets → Healthy).
- Drenaje: cola de candidatos no vacía + estado Fragile → el drenaje continúa (hoy `continue` desde el primer verificado).

## Criterios de finalización

- [ ] Una tabla con 1 verificado sigue drenando.
- [ ] `Healthy` alcanzado = puerta actual sin cambios (comportamiento eMule preservado).
- [ ] Transición de estado registrada una vez (no en cada tick).

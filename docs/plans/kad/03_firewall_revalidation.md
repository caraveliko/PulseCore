# Etapa 3 — Revalidación cortafuegos UDP

> **Dependencias**: ninguna. Etapa pequeña, gran ganancia UX — entregable como quick win desde el principio.
> **Riesgo interop**: nulo — el ciclo `FWCHECKUDPREQ → FIREWALLUDP` ya existe; solo cambiamos **cuándo** se relanza.

## Objetivo

Hoy el mantenimiento solo ejecuta la prueba mientras `!fw.verified` (`kad.rs:694-705`): una vez concluido `Open` o `Firewalled`, el estado queda **congelado hasta el reinicio** — nunca se detecta un cambio de NAT/IP.

## Estado verificado

`FirewallTestTracker` (`types.rs:161-180`) ya tiene: `status`, `verified`, `extern_port`, `use_extern`, `observed_ip`, `last_check`, `pending`.

## Cambios

### 3a. `kad_udp/types.rs`

- `FirewallTestTracker::mark_stale()` — restablece `verified = false`, `status = Unverified`, conserva `observed_ip`/`extern_port` (sirven para detectar el cambio).
- Campo `last_observed_ip: Option<Ipv4Addr>` separado de `observed_ip` si es necesario para comparar «IP observada en la última prueba» frente a «IP observada en el último `FIREWALLED_RES` recibido».

### 3b. Disparadores de revalidación (`kad.rs` mantenimiento ~l.694)

```rust
let needs_fw_check = {
    let fw = tracker.lock().await;
    (!fw.verified
        || fw.last_check.map(|t| t.elapsed() > FW_RECHECK_INTERVAL).unwrap_or(false)
        || fw.observed_ip_changed())
        && fw.status != UdpFirewallStatus::Testing
};
```

| Disparador | Fuente |
|---|---|
| `verified=false` | comportamiento actual |
| `last_check` expirado | `FW_RECHECK_INTERVAL` (const `4 h` al principio de `firewall.rs`) |
| IP observada diferente | `handlers.rs` — al recibir `FIREWALLED_RES`/`FIREWALLUDP` con IP ≠ `last_observed_ip` → `mark_stale()` |
| Cambio de bind/puerto | hook `KadUdpService::start`/`restart` → `mark_stale()` |

### 3c. `kad/firewall.rs`

`run_udp_firewall_check` sin cambios funcionales; documentar que ahora puede relanzarse sobre un tracker `verified` pero `stale`.

## Pruebas

- Tracker: `mark_stale` preserva `observed_ip`, restablece `verified`.
- Simular un `FIREWALLED_RES` con IP diferente → `needs_fw_check` verdadero.
- `last_check` > intervalo → `needs_fw_check` verdadero sin otro cambio.

## Criterios de finalización

- [ ] El estado converge tras un cambio de IP observada **sin reinicio**.
- [ ] Reprueba periódica únicamente tras `FW_RECHECK_INTERVAL` — sin bucle.
- [ ] `Testing` nunca es interrumpido por un disparador concurrente.

## Referencia eMule

`CUDPFirewallTester` — eMule vuelve a probar ante cambios de condiciones de red (reconexión, nueva IP pública).

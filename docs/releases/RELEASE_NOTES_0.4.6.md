# Notas de la Versión — Pulse Core v0.4.6

**Fecha de publicación:** 22 de noviembre de 2026  
**Estado:** Versión `0.4.6` — **Control Inteligente de Subida** (USS/LEDBAT++ AIMD, *Smart Slot Focus*, presupuesto unificado eD2K + BitTorrent, modo eco por cuota diaria y conciencia de streaming Jellyfin/Plex).

---

## 1. Síntesis de la Versión

La versión **v0.4.6** introduce un sistema integral de **Control Inteligente de Subida** diseñado para conexiones de fibra óptica, NAS domésticos y servidores multimedia. Evita la saturación de la línea y la fragmentación de ranuras, coordina el ancho de banda entre **eD2K** y **BitTorrent** y adapta dinámicamente la subida cuando no hay descargas en curso, cuando se agota la cuota diaria o cuando hay reproducciones activas en **Jellyfin** o **Plex**.

---

## 2. Novedades y Mejoras Principales

### Control Adaptativo y Gestión de Ranuras (`mule-daemon`)
- **LEDBAT++ con ley AIMD proporcional al ancho de banda (`uss_ledbat.rs`)**: reemplaza el paso fijo anterior por subida aditiva proporcional (`+5 %` por ciclo sin congestión) y recorte multiplicativo rápido (hasta `-25 %`) ante *bufferbloat*, escalando con fluidez en enlaces de fibra.
- **Techo automático en reposo (`uss.rs`)**: mediante `idleUploadCapKbs`, cuando el control inteligente está activo y la bajada agregada es inferior a `1 KiB/s`, limita la subida al techo de reposo configurado.
- ***Smart Slot Focus* (`upload_queue/scoring.rs`)**: mediante `smartSlotTargetKbs` (`0` = modo clásico eMule 0.72a), concentra el caudal en menos ranuras más rápidas para entregar piezas completas antes y mantener una cola donde actúen los créditos y prioridades.

### Presupuesto Unificado, Cuota Eco y Streaming (`mule-torrent`, `mule-daemon`)
- **Reparto dinámico eD2K + BitTorrent (`upload_budget.rs`, `mule-torrent`)**: aplica el límite efectivo de subida en caliente también sobre `librqbit::Limits`, repartiendo el presupuesto según demanda real (con suelo mínimo del `20 %` cuando ambos compiten).
- **Modo eco por cuota diaria (`quotaEcoUploadKbs`)**: al alcanzar `dailyUploadQuotaGb`, permite mantener la cola abierta reduciendo la subida a una velocidad eco para conservar créditos eD2K en lugar de cortar las ranuras.
- **Conciencia de streaming (`streaming_watcher.rs`)**: cuando `streamingUploadCapKbs > 0`, consulta periódicamente `/Sessions` en Jellyfin o `/status/sessions` en Plex y recorta temporalmente la subida durante reproducciones activas no pausadas.

### Monitor de Ancho de Banda y Preferencias (`web/`)
- **Controles rápidos y telemetría en vivo**: botón **⚡ Control Inteligente**, estado y motivo de regulación, RTT frente a línea base (`+ms` de cola), reparto eD2K/BT, inputs rápidos (*Techo ↑*, *Reposo ↑*, *Streaming ↑*, *Objetivo/ranura*) y línea `━ Límite ↑` superpuesta en la gráfica SVG.

---

## 3. Verificación e Integridad

Validación completa con `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `python3 scripts/smart_test.py --mode smart` (sintaxis JavaScript, guardia antiduplicados B1 y suites de pruebas afectadas) y compilación cruzada `release` para Raspberry Pi (`aarch64-unknown-linux-gnu`) antes de publicar la etiqueta `v0.4.6` para **Windows x64**, **Linux x64** y **Linux ARM64 (`aarch64`)**.

# Notas de la Versión — eMuleRust v0.2.0-RC1

**Fecha de publicación:** 23 de septiembre de 2026  
**Estado:** Release Candidate — alineación de comportamiento eMule 0.72a, UX y resiliencia Kad

---

## 1. Síntesis de la Versión

La versión **v0.2.0-RC1** concluye un ciclo de auditoría completo del daemon y de
la interfaz: eliminación de limitaciones internas heredadas de las primeras
versiones de prueba, arranque Kad hecho resiliente al estilo de eMule y
consolidación de la experiencia de usuario (medidas de caudal reales,
pestaña Compartidos enriquecida, localización francesa exhaustiva).

1. **Limitaciones internas eliminadas — alineación con eMule 0.72a**:
   - Ranuras de subida simultáneas ampliadas de 10 a **100**
     (`MAX_UP_CLIENTS_ALLOWED`, `Opcodes.h`) — el techo anterior
     rompía el escalado en conexiones rápidas.
   - Las **4 primeras ranuras** se abren sin condición de caudal
     (`AcceptNewClient`: `max(MIN_UP_CLIENTS_ALLOWED, 4)`).
   - `promote_waiting` comparte ahora la elegibilidad
     `can_open_slot` (suelo de caudal + veto USS).
   - Ritmo de conexiones de fuentes: `MAX_CONNECT_PER_TICK` 5 → 12
     (~4 conexiones/s, ritmo `MaxConnPer5Sec`=20 de eMule); el presupuesto
     token-bucket `rate_limit_per_min` (240/min) sigue siendo la salvaguarda.
   - Tope entrante: la constante muerta `MAX_INBOUND_CONNECTIONS` (500)
     se elimina — el listener sigue el presupuesto `maxConnections` en vivo;
     el tope anti-flood por IP (5) se conserva.
   - Soft-cap de Source Exchange: el ajuste persistido
     `maxPeersPerDownload` sustituye al respaldo fijo en 400
     (`GetMaxSourcePerFile` → `thePrefs.maxsourceperfile`).

2. **Arranque Kad resiliente — `s_liBootstrapList`**:
   - Un tiempo de espera agotado en el primer bootstrap dejaba Kad muerto con una
     tabla vacía — el mantenimiento solo corría si
     `total_contacts > 0`.
   - Nueva cola `bootstrap_candidates` (tope 50, deduplicada,
     `kad_version >= 2`) alimentada por cada handshake eD2K.
   - Drenaje en tarea en segundo plano: 1 `BOOTSTRAP_REQ` cada **2 s** mientras
     la tabla esté vacía, **15 s** en caso contrario — `CKademlia::Process`
     (Kademlia.cpp:287). Compuerta fiel `HasHadContact`: el drenaje corre
     mientras ningún contacto *verificado* haya respondido.
   - `nodes.dat` (`<data_dir>/nodes.dat`) autocargado al arrancar.
   - `kad.statusChanged` emitido en cuanto se aprenden los primeros contactos.

3. **USS alineado con eMule**: *Last Common Route Finder* mediante
   traceroute ICMP en Windows (`uss_icmp.rs`) y selección del último
   router común — medición del margen de subida fiel a
   `UploadBandwidthThrottler`.

4. **Caudales de subida realmente medidos**: la velocidad de compartición se
   calcula sobre los bytes efectivamente emitidos (por ranura y agregada),
   refrescada continuamente y decreciendo hacia cero en ausencia
   de actividad — reutilizada por la interfaz, las instantáneas RPC, las estadísticas del
   daemon, el USS y la cola de goteo.

5. **Pestaña Compartidos enriquecida**:
   - **Categoría por fichero compartido** (`shared_files.category_id`,
     migración idempotente, RPC `shares.setCategory`) — un compartido procedente
     de una descarga sigue la categoría de esta.
   - Agrupación por categoría (orden `sortOrder`), «Sin categoría» al
     final, ordenación interna por **total subido descendente**.
   - Motor de búsqueda (nombre + hash eD2K), estadísticas `↑ caudal` / `Σ total`
     por fichero, prioridad de subida eMule (`verylow`…`powershare`).

6. **UI descargas**:
   - Chip de prioridad modernizado: píldora tintada, iconos semánticos,
     3 colores (ámbar = High, azul cielo = Normal, pizarra = Low).
   - Barra de progreso desduplicada: `PartAvailabilityBar` muestra
     por sí sola el avance (borde verde eMule + chunks), ampliada a 14 px.

7. **Auditoría de los ajustes**: inventario de las 36 claves RPC — cada campo
   está cableado, acotado (`maxConnections` ≤ 65 000, `maxUploadQueue`
   acotado antes de persistir) y sin duplicados entre secciones.

8. **Lista de fuentes alineada con `srclist`/deadlist de eMule**:
   enfriamientos, rechazos (FNF) y filtrados por IP ocultos (deadlist invisible);
   `sourceCount` solo cuenta las fuentes visibles.

9. **Localización francesa exhaustiva**: repaso fichero por fichero,
   terminología eMule natural.

---

## 2. Comandos de Validación

```powershell
# Verificación completa de compilación y lints
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Suite de pruebas Rust (100 % sin conexión)
cargo test --workspace --all-features

# Validación de la aplicación Flutter Desktop
cd apps/desktop
flutter analyze
flutter test
```

---

## 3. Migraciones

- **`shared_files.category_id`**: columna añadida mediante `ALTER TABLE`
  idempotente al arrancar — no se requiere ninguna acción manual, las bases
  existentes se migran in situ.
- **`nodes.dat`**: basta con dejar el fichero en `<data_dir>` — se
  carga automáticamente al inicio como candidatos de bootstrap.

---

## 4. Notas de Seguridad

- Ningún método `lab.*` en la variante `standard` — el paquete empaquetado
  se verifica mediante `test_packaged_bundle.ps1`.
- Pipeline anti-SSRF, aislamiento loopback, kill switch atómico y
  seudonimización de pares (`mule-network-policy`) sin cambios.

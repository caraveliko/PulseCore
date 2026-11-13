# Notas de la Versión — eMuleRust v0.2.0-RC3

**Fecha de publicación:** Octubre de 2026
**Estado:** Release Candidate — cambio de marca completo a **eMuleRust**, campaña de
validación A–D totalmente en verde, endurecimiento anti-SSRF, primer repositorio
público en GitHub.

---

## 1. Síntesis de la Versión

**v0.2.0-RC3** es la primera versión publicada bajo el nombre definitivo
**eMuleRust** en GitHub (`laurentgeynet-ux/eMule-Rust`). Se apoya en
RC2 (auditoría exhaustiva en línea de eMule 0.72a + interoperabilidad en vivo) y
añade una campaña completa de validación en 4 compuertas — todas en verde.

1. **Cambio de marca a eMuleRust** — nombre mostrado, ejecutables
   (`eMuleRust.exe`), paquetes, variables de entorno `EMULERUST_*`
   (se conservan los alias obsoletos `EMULE_SION_*`), identificadores Dart/Rust,
   documentación bilingüe (README en inglés + `README.fr.md`).

2. **Campaña de validación A–D**:
   - **Fase A**: `verify_all.ps1` 100 % en verde, paquete estándar
     recompilado y verificado mediante smoke test (sin ningún método `lab.*` expuesto).
   - **Fase B**: banco RPC funcional **134 PASS / 0 FAIL** —
     cobertura completa de la superficie pública (identidad, descargas,
     formatos `.met`/`.dat`, compartidos, fuentes, servidores, Kad, filtros,
     ajustes, historial) + rutas de error.
   - **Fase C**: interoperabilidad en vivo **7/7 × 3 referencias** — eMule 0.50a,
     eMule 0.72a y aMule 3.1.0 (binarios fijados por SHA-256,
     sandbox hermético, loopback estricto).
   - **Fase D**: banco en red real supervisado **21/21** — importación
     real de `server.met` (16 servidores), sesión eD2K establecida,
     95 contactos Kad, búsquedas reales eD2K + Kad, kill switch,
     UPnP mapeado y luego desmapeado limpiamente (< 500 ms).

3. **Endurecimiento anti-SSRF**: `servers.updateFromUrl` y
   `kad.bootstrapFromUrl` validan ahora el esquema (solo http/https) y
   las direcciones resueltas — rechazando objetivos link-local, LAN privada y
   prohibidos por la política.

4. **Correcciones de robustez**:
   - Apagado acotado del daemon: desmapeo UPnP desacoplado (`std::thread` +
     oneshot de 500 ms) — sin más bloqueos de > 5 s en puertas de enlace lentas.
   - Smoke test empaquetado: drenaje continuo de la tubería del daemon.
   - E2E loopback: fallo/reanudación determinista mediante `downloads.pause`.
   - Fixtures de prueba consolidados en `mule-test-support`.

5. **CI/CD de GitHub**: flujo de trabajo `CI` (Rust check/clippy/test +
   Flutter analyze/test, daemon compilado para pruebas de supervisor/E2E),
   flujo de trabajo `Packages` (paquetes Windows + Linux como artefactos),
   `release.yml` (compilación + smoke + GitHub Release en etiquetas `v*`),
   Dependabot activado.

## 2. Comandos de Validación

```powershell
# Verificación completa
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\verify_all.ps1

# Banco RPC funcional (loopback, sin conexión)
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\test_phase_b_functional_bench.ps1

# Banco en red real (supervisado — emite tráfico eD2K/Kad público)
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\test_phase_d_public_network.ps1

# Interoperabilidad en vivo frente a binarios de referencia fijados
cargo test -p mule-protocol --test tier3_loopback --all-features -- --include-ignored
```

## 3. Migraciones

- **Variables de entorno**: `EMULERUST_WORKDIR`,
  `EMULERUST_DOWNLOADS_DIR`, `EMULERUST_DEPLOY_DIR` tienen prioridad;
  las heredadas `EMULE_SION_*` / `EMULE_DEPLOY_DIR` siguen leyéndose como
  respaldos obsoletos (sin acción necesaria).
- **UPnP**: las concesiones creadas bajo el prefijo heredado `eMule-Sion` siguen
  reconociéndose y desmapeándose al apagar.
- **Workdir**: se utiliza `%LOCALAPPDATA%\eMuleRust`; sin migración
  automática desde `eMule-Sion` — reimportar mediante el asistente de migración
  de eMule si es necesario.

## 4. Notas de Seguridad

- Anti-SSRF aplicado a las descargas por URL (`server.met` / `nodes.dat`) —
  esquema + resolución DNS comprobados frente a la política de red.
- Ningún método `lab.*` en la variante `standard` — verificado por
  `test_packaged_bundle.ps1` en CI.
- Rama `master` protegida: force-push y eliminación bloqueados.
- Compilaciones sin firmar (Authenticode opcional mediante los secretos
  `WINDOWS_CERT_BASE64` / `WINDOWS_CERT_PASSWORD`).

## 5. Limitaciones Conocidas

- **Solo Windows x64** para la interfaz empaquetada (el paquete Linux de CI es
  experimental — primera compilación automatizada añadida en esta RC).
- Sesión eD2K observada como **LowID** en el banco de Fase D a pesar de un
  mapeo UPnP confirmado — comportamiento aceptable (degradado), en
  investigación.
- Sin descargas en segundo plano en Android (restricción del SO) — el objetivo
  Android no está mantenido.

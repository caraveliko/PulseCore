## Resumen

<!-- Qué y por qué — referenciar la incidencia relacionada (Fixes #123) -->

## Tipo de cambio

- [ ] Corrección de error
- [ ] Nueva funcionalidad
- [ ] Refactorización / mantenimiento
- [ ] Documentación
- [ ] Cambio protocolario en cable (eD2K/Kad) ⚠️

## Lista de comprobación

- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` pasa
- [ ] `cargo test --workspace --all-features` pasa
- [ ] Ninguna prueba emite tráfico de red saliente (solo loopback)
- [ ] Código, documentación, *Inner Doc Comments* (`//!`) y archivos Markdown en español de España
- [ ] Si afecta al protocolo de cable: comportamiento verificado contra eMule 0.72a
      (`srchybrid`) + especificación `docs/ed2k/` + vector de prueba añadido
- [ ] Si afecta a la seguridad: canalización anti-SSRF / aislamiento loopback /
      interruptor de corte / seudonimización sin debilitar
- [ ] Ninguna funcionalidad `lab.*` expuesta en el sabor `standard`
- [ ] Nueva dependencia: compatible con `deny.toml` y preferiblemente
      declarada en `[workspace.dependencies]`

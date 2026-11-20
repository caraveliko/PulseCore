# GEMINI.md — Reglas para agentes de IA

La fuente única de verdad para el mapa del repositorio, comandos de validación, constantes, antiduplicación y convenciones está en [`AGENTS.md`](AGENTS.md).

Recordatorios esenciales:
- **Fidelidad protocolaria**: verificar cualquier cambio de protocolo frente a las fuentes de **eMule 0.72a** y los ADR en `docs/ed2k/`.
- **Pruebas 100 % sin conexión**: ejecutar `python3 scripts/smart_test.py --mode smart` (o `cargo test --workspace --all-features --lib --tests`) sin tráfico de red saliente.
- **Seguridad y política «Solo Vídeo»**: no debilitar `mule-network-policy`, no exponer `lab.*` en compilaciones estándar y delegar cualquier comprobación de contenido en `mule_network_policy::content_policy`.
- **Español de España obligatorio**: todo código documentado, comentarios (`//`, `///`, `//!`, `/* ... */`) y archivos `.md` deben redactarse exclusivamente en español de España.

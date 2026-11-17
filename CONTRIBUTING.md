# Contribuir a eMuleRust

Gracias por tu interés en el proyecto. Este documento describe el flujo de contribución y los requisitos de calidad.

## Flujo de ramas

| Rama | Rol |
| :--- | :--- |
| `master` | Producción — únicamente fusiones validadas desde `Beta`, etiquetadas como `v*` |
| `Beta` | Prelanzamiento / QA — candidatos a lanzamiento (*release candidates*) |
| `Dev` | Integración continua — destino por defecto de las *pull requests* |
| `feature/*`, `Android`, `iptv` | Ramas de trabajo — fusionadas en `Dev` mediante PR |

Flujo: `feature/*` → `Dev` → `Beta` (RC) → `master` (lanzamiento etiquetado).

## Antes de enviar una PR

Ejecuta el conjunto completo de validación (100 % sin conexión):

```powershell
# Rust — lint y pruebas
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features

# O todo en uno:
powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\verify_all.ps1
```

El workspace compila con `-D warnings`: no se tolera ninguna advertencia.

## Reglas no negociables

1. **Fidelidad protocolaria**: la referencia de verdad es el código fuente de **eMule 0.72a**. Cualquier cambio en opcodes, entramado o comportamiento en cable debe verificarse contra esas fuentes y `docs/ed2k/`, con un vector de prueba. Consulta los ADRs antes de modificar `mule-protocol`.
2. **Pruebas exclusivamente sin conexión**: ninguna prueba debe emitir tráfico de red saliente. Usa loopback (`test_qa_loopback_e2e.ps1` como referencia).
3. **Seguridad**: nunca debilitar la canalización anti-SSRF, el aislamiento loopback por defecto, el interruptor de corte atómico ni la seudonimización de pares (`mule-network-policy`). Nunca publicar versiones con `-LabMode` / `--features lab-tools`.
4. **Dependencias**: revisa `deny.toml` antes de añadir una crate; prefiere las dependencias del workspace (`[workspace.dependencies]`).

## Convenciones

- Código, documentación, comentarios en el código (`//`, `///`, `//!`, `/* ... */`) y archivos Markdown exclusivamente en **español de España** (no se admite ningún otro idioma).
- Errores en Rust: `thiserror`, prohibido usar `unwrap()` fuera de las pruebas.
- Registro (*Logging*): `tracing` (nunca `println!` en los crates).
- Asincronía: Tokio; sin bloqueos síncronos en tareas asíncronas.
- Arquitectura de interfaz: WebUI embebida servida por `mule-daemon` y bandeja del sistema de Windows en `crates/mule-daemon/src/tray.rs`.
- Mensajes de *commit*: estilo *conventional commits* en español (`feat(x):`, `fix(y):`, `chore:`, `docs:`…).

## Informar de un error

Abre una *issue* en GitHub con: versión (`version.json` del paquete), sistema, pasos de reproducción y registros (*logs*) relevantes. Para vulnerabilidades de seguridad, sigue [`SECURITY.md`](SECURITY.md).

## Licencia

Toda contribución se licencia bajo **GPL-2.0-or-later** (consulta [`LICENSE`](LICENSE)).

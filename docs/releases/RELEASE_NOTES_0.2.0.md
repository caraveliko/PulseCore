# Notas de la Versión — eMuleRust v0.2.0

**Fecha de publicación:** 23 de septiembre de 2026
**Estado:** Versión final — alineación con eMule 0.72a, interfaz web sin configuración, publicación de código abierto

---

## 1. Síntesis de la Versión

La versión **v0.2.0** es la versión final de la línea 0.2.x. Recoge
todo el contenido de `v0.2.0-RC1` (véase
[RELEASE_NOTES_0.2.0-RC1.md](RELEASE_NOTES_0.2.0-RC1.md)) y añade
la interfaz web servida por el daemon, así como la preparación del repositorio
para su publicación pública.

Contenido principal heredado de la RC1:

- Limitaciones internas eliminadas y alineación de comportamiento con **eMule 0.72a**
  (100 ranuras de subida, ritmo de conexiones, soft-cap de Source Exchange).
- **Arranque Kad resiliente**: cola `bootstrap_candidates`, drenaje
  fiel a `CKademlia::Process`, carga automática de `nodes.dat`.
- **USS** mediante traceroute ICMP (*Last Common Route Finder*).
- Caudales de subida **realmente medidos** (por ranura y agregados).
- Pestaña Compartidos enriquecida (categorías, búsqueda, estadísticas de subida),
  auditoría de las 36 claves de ajustes, localización francesa exhaustiva.

## 2. Novedades desde la RC1

### Interfaz web servida por el daemon — conexión sin configuración

- **`ServeDir` integrado** (`tower-http`): el daemon sirve la compilación
  web de Flutter en su puerto HTTP. Resolución de la carpeta:
  `--web-dir` / `MULE_WEB_DIR` > autodetección `<exe>/../web`
  (paquete `bin/` → `web/`), `<exe>/web`, `../../apps/web/build/web`
  (desarrollo cargo). `index.html` como respaldo SPA.
- **Autenticación transparente**: `GET /session-config.js` emite
  `window.MULE_SESSION` (endpoint + token) — respuesta limitada a enlaces
  **loopback** para no exponer nunca `auth_token` en una interfaz de
  red.
- **Aplicación web**: conexión sin entrada manual cuando la sirve el daemon,
  parámetro `?token=`, campo de token opcional, `autoReconnect` en el
  cliente RPC.
- **Escritorio**: botón «Interfaz web» en la barra lateral que abre el
  navegador en el endpoint de sesión.
- **Paquete**: `build_windows_release.ps1` compila `apps/web`
  (`-SkipWeb`, `-WebBaseHref`), copia los activos bajo `web/`, genera
  `Interface Web.bat` (inicia el daemon si no está presente y luego abre
  `http://127.0.0.1:4660/`) y añade `includesWebUi` a
  `version.json`.

### Paleta suavizada de la barra de progreso

Se conserva la semántica `DrawStatusBar` de eMule con tonos
Tailwind ya utilizados por `AppTheme`; las constantes de colores de
partes son públicas y se comparten con el mapa part-map.

### Preparación para la publicación de código abierto

- `LICENSE` (GPL-2.0-or-later), `CONTRIBUTING.md`, `SECURITY.md`.
- `.gitignore` generalizado a todas las aplicaciones Flutter
  (`**/build/`, `**/.dart_tool/`, artefactos Android, keystores).
- Configuraciones `.agents/` despersonalizadas (`*.example.json`).
- `DeployDir` de `build_windows_release.ps1` convertido en opt-in.

---

## 3. Comandos de Validación

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

## 4. Migraciones

- **`shared_files.category_id`**: `ALTER TABLE` idempotente al
  arrancar — no se requiere ninguna acción manual.
- **`nodes.dat`**: basta con dejar el fichero en `<data_dir>` — se
  carga automáticamente al inicio como candidatos de bootstrap.

---

## 5. Notas de Seguridad

- Ningún método `lab.*` en la variante `standard` — el paquete empaquetado
  se verifica mediante `test_packaged_bundle.ps1`.
- `session-config.js` (token de sesión web) solo se emite en los
  enlaces **loopback**.
- Pipeline anti-SSRF, aislamiento loopback, kill switch atómico y
  seudonimización de pares (`mule-network-policy`) sin cambios.
- Integridad: `SHA256SUMS.txt` + `version.json` + SBOM CycloneDX
  generados en cada compilación, verificados por la compuerta de coherencia 4-way.

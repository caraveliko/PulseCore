# Informe de análisis de duplicación — workspace Rust

Fecha: 2026-09-24. Analizador: `scripts/find_duplicates.py` (símbolos homónimos,
cuerpos de funciones normalizados, bloques clonados entre ficheros ≥ 8 líneas limpias).
Alcance: `crates/**` src + tests — 330 ficheros, ~106 200 líneas.

## Clasificación

- **D1** clon exacto → fusionar en el crate propietario
- **D2** cuasi-clon → generalizar / parametrizar
- **D3** duplicación semántica → unificar o documentar la distinción
- **D4** código muerto → eliminar
- **D5** falso positivo / repetición legítima → conservar (documentado)

## D1 — Clones exactos (código de producción)

| Duplicado | Ubicaciones | Acción |
| :--- | :--- | :--- |
| `send_msg` + `read_msg` (~30 l. cada uno, idénticos) | `mule-daemon/src/lab/downloader.rs`, `lab/source_service.rs` | Extraer a `lab/io.rs` (pub(crate)) |
| Lista `capabilities` (~24 l., literal) | `rpc/handlers/settings.rs` + `rpc/methods.rs` | `const CAPABILITIES` único en `methods.rs` |
| `name()` (búsqueda `TAG_SPECIAL_NAME` en tags) | `ed2k/server/search.rs` + `ed2k/server/types.rs` (mule-protocol) | Helper compartido en `tags.rs` |
| `open()` (File::open + read_from) | `met/part_met.rs` + `met/server_met.rs` (mule-format) | Helper `open_read` en `met/` |
| `count_kad_*` ×4, `purge_expired_*` ×3, `record_credit_*` ×2 | `mule-core/src/persistence/sqlite/kad.rs`, `credits.rs` | Helpers privados `count_rows` / `purge_expired` / `record_credit` |
| Accesores `misc_options1/2` ×8 | `mule-protocol/src/ed2k/peer_info.rs` | Helpers privados `misc1_nibble` / `misc2_flag` |

## D1 — Clones exactos (pruebas)

| Duplicado | Ocurrencias | Acción |
| :--- | :--- | :--- |
| `make_req` | ×11 ficheros `mule-daemon/tests/` | `tests/common/` |
| `setup_test_state` (3 variantes) | ×12 ficheros `mule-daemon/tests/` | `tests/common/` (variantes parametrizadas) |
| `spawn_test_daemon` (3 variantes) | ×7 ficheros `mule-daemon/tests/` + `mule-mcp/tests/` | `tests/common/` |
| `RunningDaemon` (+ `start`, `Drop`) | ×3 ficheros `mule-daemon/tests/` | `tests/common/` |
| `send_json` / `recv_response_by_id` | ×2-3 ficheros | `tests/common/` |
| `sample_client_tags` ×3, `sample_file_hash` ×2, `hash16/hash20`, `read_msg` | `mule-protocol/tests/` | Ampliar `tests/common/` existente |

## D3 — Duplicación semántica

| Caso | Diagnóstico | Decisión |
| :--- | :--- | :--- |
| `TagValue` ×2 | `mule-format::met` (ficheros .met: `Bsob`, `Hash16`, `Uint64`) frente a `mule-protocol::ed2k` (en línea: `UInt8/16/32/64`, `Hash`). Dominios distintos, variantes distintas — **no unificable** sin perder la discriminación en línea. | Conservar ambos; renombrar `met::TagValue` → `MetTagValue` para eliminar la homonimia |
| `PeerSource` ×2 | `mule-format::ed2k` (parseo de enlace `host:port[:hash]`) frente a `mule-core::model` (registro persistente). Roles diferentes. | Renombrar el tipo de formato → `LinkSource` |
| `JsonRpcError` ×2 | `mule-mcp::protocol` (código i64, `#[allow(dead_code)]`) duplica `mule-daemon::error` (código i32, completo). `mule-mcp` ya depende de `mule-daemon`. | Completar `error.rs` (`parse_error`, `invalid_request`), eliminar el tipo local mcp |
| `AppState` ×2 | `mule-daemon::state` frente a `mule-mcp::main` — roles distintos | Renombrar el de mcp → `McpState` |
| Esquemas JSON de las herramientas MCP | Fragmentos `json!({...})` casi idénticos repetidos en `tools/kad.rs`, `servers.rs`, `system.rs`, `downloads/schemas.rs`, `shares/schemas.rs` | Fragmentos compartidos (`tools/schema_fragments.rs`) |
| `emit_event` (peer frente a server session) | Mismo esqueleto, variante de evento diferente | D5 — cuerpo de 4 líneas, generalización no rentable |
| `handle_*_status` ×4 | Wrappers `snapshot().await → to_value` | D5 — handlers RPC explícitos, una línea de lógica |

## D4 — Código muerto

- `#[allow(dead_code)]` en `mule-mcp::protocol::JsonRpcError`: parcialmente sin usar → absorbido por la desduplicación anterior.
- Revisión `clippy` `dead_code` tras la consolidación.

## D5 — Repeticiones legítimas conservadas

- `test_fixture_*` (mule-format / mule-protocol): vectores de prueba explícitos — prima la legibilidad.
- `internal_error`/`invalid_request`/`invalid_params` (mcp): constructores triviales — absorbidos mediante `JsonRpcError`.
- Homónimos de métodos en tipos diferentes (`update_status`, `set_enabled`, `stop`, `snapshot`…): APIs paralelas, sin código duplicado.
- `mod tests`, `mod types`: idiomático en Rust.
- `to_hex` ×4: métodos inherentes en 4 tipos de hash distintos; solo `kad_id.rs` reimplementa el bucle en lugar de `hex::encode` → alinear.

## Orden de ejecución de los lotes

1. **Lote 1** — `mule-daemon/tests/common/` (mayor foco, ~30 duplicados)
2. **Lote 2** — tipos: `JsonRpcError` (mcp→daemon), `PeerSource`→`LinkSource`, `TagValue`→`MetTagValue`, `AppState`→`McpState`
3. **Lote 3** — utilidades src: `lab/io.rs`, `CAPABILITIES`, helpers `tags.rs`/`met`, helpers sqlite, helpers `peer_info`, `to_hex`, fragmentos de esquemas MCP
4. **Lote 4** — ampliación `mule-protocol/tests/common/`, remanentes, verificación `clippy`/`dead_code`
5. **Salvaguarda** — `scripts/test_duplicates.ps1` + conexión `verify_all.ps1` + nota en AGENTS.md

Verificación tras cada lote: `cargo check --workspace --all-targets --all-features`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings`,
`cargo test --workspace --all-features` (sin conexión).

## Balance tras la refactorización

Métricas: **338 ficheros / ~104 700 líneas** (antes 330 / ~106 200 — es decir, ~1 500
líneas netas eliminadas, además de la creación de `mule-test-support`, `capabilities.rs`,
`lab/io.rs`, `tests/common/fixtures.rs`).

Realizado:

- **Lote 1** — crate `mule-test-support` (dev-dependency de `mule-daemon` y
  `mule-mcp`): `spawn_test_daemon*` ×3 variantes, `RunningDaemon`, `rpc_state*`,
  `make_req`, `send_json`/`recv_response_by_id` centralizados (~30 duplicados
  de infraestructura de pruebas eliminados).
- **Lote 2** — tipos: `JsonRpcError` unificado bajo `mule-daemon::error`
  (+ `parse_error`/`invalid_request`), `PeerSource`→`LinkSource`,
  `met::TagValue`→`MetTagValue`, `AppState`→`McpState`.
- **Lote 3** — utilidades src: `rpc/capabilities.rs` (fuente única de la
  lista `capabilities`), `lab/io.rs`, helpers sqlite (`count_rows`,
  `purge_expired`, `record_credit_delta`), accesores de campos de bits `peer_info`,
  `downloads::db_err` (~60 sitios `map_err`), `handlers::json_value` (~41 sitios),
  `kad_buf` codificador Kad (~25 sitios), `met::reader::open_met_file`,
  esquemas MCP: `obj_schema`/`prop` aplicados a las **87 herramientas** mediante
  `scripts/migrate_mcp_schemas.py`.
- **Lote 4** — `mule-protocol/tests/common/fixtures.rs` (`sample_user_hash`,
  `sample_client_tags`, `seeded_hash`, `resume_file_hash`, `nominal_client`,
  `connect_nominal_client`…) + `roundtrip_fixture`/`decode_malformed` en las
  suites afectadas; fixtures `.met` compartidos en `mule-format/tests`.
- **Salvaguarda** — `scripts/test_duplicates.ps1` + `scripts/dup_allowlist.txt`,
  conectado en el paso 2/5 de `scripts/verify_all.ps1`; tabla de propiedad de
  utilidades añadida a `AGENTS.md`.

Remanentes tolerados (D5, véase `dup_allowlist.txt`): `read_msg` daemon↔test
(compartición imposible entre crates), `open` inherente por tipo `.met`, esqueletos
de pruebas parametrizadas. Las secciones A/B2 del informe siguen siendo informativas:
homónimos de métodos en tipos distintos, esqueletos de codificadores en línea,
accesores — repeticiones de forma, no de sustancia.

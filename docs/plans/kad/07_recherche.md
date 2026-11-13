# Etapa 7 — Búsqueda funcional

> **Dependencias**: etapa 5 (paginación y recogida deslizante suponen el transporte fiable).
> **7a toca `mule-protocol`** → reglas estrictas: spec `docs/ed2k/` + fuentes `KademliaUDPListener.cpp:883` (`CreateSearchExpressionTree`) + round-trip obligatorio.

## 7a — Codificador de expresión restrictiva (`mule-protocol`)

**Estado**: `search_expr.rs` es **solo decodificador** (`MAX_SEARCH_EXPR_DEPTH = 24`). El indicador restrictivo existe en el cable: `start_pos & 0x8000` + árbol serializado en sufijo (`decoder/search.rs:719-728`).

**Por hacer**:

1. `encode_search_expr(expr: &KadSearchExpression) -> Vec<u8>` en `search_expr.rs` — espejo **exacto** del decodificador (mismos opcodes en el cable, mismo orden, `utf8_string`, ancho numérico).
2. `encoder.rs`: `encode_search_keyword_req_restrictive(keyword_hash, start_pos | 0x8000, expr_bytes)` o parámetro opcional sobre el existente.
3. **⚠️ Verificación obligatoria** antes de escribir: diseño exacto de `CreateSearchExpressionTree` (`KademliaUDPListener.cpp:883`+) — orden de los operandos, marcadores `0x00/0x01/0x02`, tamaños de los números (2/4/8 bytes según valor).
4. Pruebas: round-trip `decode(encode(x)) == x` sobre todas las formas de `KadSearchExpression` + vector binario eMule si está disponible en `docs/ed2k/`.

## 7b — Multitérmino (`search_keywords.rs`)

**Estado**: `extract_kad_keywords(term).into_iter().next()` — una sola palabra, el árbol restrictivo solo se usa en entrada.

**Por hacer**:

- Construir el `KadSearchExpression` completo: `AND` de los términos + nodos `>=`/`<=` para `min_size`/`max_size`/`extension` si están presentes.
- Semántica eMule: **un `SEARCH_KEY_REQ` por término clave del árbol** (no un pivote único), llevando cada uno la expresión completa — hacia los mismos indexadores convergidos.
- Fusión de los resultados: dedup por `file_hash` sobre la marcha, unión de las entradas.

## 7c — Paginación

**Estado**: `send_search_keyword` ya acepta `start_pos`; `send_search_sources` fija en duro `0` (`search_client.rs:85`).

**Por hacer**:

1. `send_search_sources(..., start_pos: u16)` — añadir el parámetro, pasarlo a `encode_search_source_req` en lugar del `0`.
2. `search_sources_iterative` / `search_keywords_iterative`: por indexador, si la página anterior estaba llena (`>= SEARCH_PAGE_FULL = 300` o el máx. en el cable de un lote), petición `start_pos += returned` hasta página no llena o cota `SEARCH_MAX_PAGES`/`SEARCH_MAX_RESULTS`.

Constantes en el archivo de búsqueda correspondiente: `SEARCH_PAGE_FULL`, `SEARCH_MAX_PAGES`, `SEARCH_MAX_RESULTS`.

## 7d — Coalescing + `source_count`

**Estado**: `in_flight_searches: HashSet<String>` (`kad.rs:45`) — el duplicado recibe `Ok(vec![])` en lugar del resultado real (`search_sources.rs:24-54`).

**Por hacer**:

1. `HashSet<String>` → `HashMap<String, tokio::sync::broadcast::Sender<Result<Arc<Vec<..>>, KadTxError>>>`: primer llamador = líder, los duplicados se suscriben y reciben **el mismo resultado**.
2. `publish_keyword` (`search_keywords.rs:230`): `source_count` = número real de fuentes conocidas del archivo compartido (consultar el índice/`DownloadService`), nunca el `1` constante.

## Pruebas

- `mule-protocol`: round-trip expresión + vector binario.
- Multitérmino: petición «a b» → 2 `SEARCH_KEY_REQ` emitidos con árbol AND completo.
- Paginación: indexador ficticio que devuelve 300 y luego 50 → dos peticiones (`start_pos` 0 y luego 300), parada tras la página no llena.
- Coalescing: dos llamadas simultáneas misma clave → **mismo** `Vec` recibido por ambas.

## Criterios de finalización

- [ ] Búsqueda «a b c» devuelve resultados AND multitérmino.
- [ ] Resultados más allá de 300 por indexador.
- [ ] Duplicado de búsqueda recibe el resultado real.
- [ ] `cargo test -p mule-protocol --all-features` en verde.

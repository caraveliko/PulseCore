// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use super::*;

/// Intervalo mínimo (en milisegundos) entre refrescos de fechas de estreno para películas pendientes (12 horas).
const UNRELEASED_METADATA_REFRESH_MS: i64 = 12 * 60 * 60 * 1000;

impl AutoGrabber {
    /// Ejecuta la búsqueda, el filtrado, el diagnóstico y la descarga automática para una película dada.
    pub(super) async fn process_movie_item(
        state: &AppState,
        aggregator: &IndexerAggregator,
        item: &MediaItem,
        ag_state: &mut AutoGrabberState,
        force: bool,
    ) -> Option<Value> {
        let now = now_ms();
        let mut effective_item = item.clone();

        // Comprobación inteligente de estreno: evitar buscar películas aún no estrenadas o sin lanzamiento disponible
        if effective_item.status == "wanted" || effective_item.status == "unreleased" {
            let today = time::OffsetDateTime::now_utc().date();
            let today_str = format!("{today}");
            let current_year = today.year() as u32;

            let needs_refresh = effective_item.tmdb_id.is_some()
                && ((effective_item.status == "unreleased"
                    && now.saturating_sub(effective_item.updated_at) >= UNRELEASED_METADATA_REFRESH_MS)
                    || (effective_item.release_date.is_none()
                        && effective_item.digital_release_date.is_none()
                        && effective_item.year.is_none_or(|y| y >= current_year)));

            let mut metadata_updated = false;
            if needs_refresh {
                if let Some(tid) = effective_item.tmdb_id {
                    if let Ok(meta) = state.media_metadata.get_movie(tid).await {
                        if effective_item.year.is_none() {
                            effective_item.year = meta.year;
                        }
                        if meta.release_date.is_some() {
                            effective_item.release_date = meta.release_date;
                        }
                        if meta.digital_release_date.is_some() {
                            effective_item.digital_release_date = meta.digital_release_date;
                        }
                        if meta.release_status.is_some() {
                            effective_item.release_status = meta.release_status;
                        }
                        metadata_updated = true;
                    }
                }
            }

            let avail = crate::services::media_metadata::evaluate_movie_availability(
                effective_item.year,
                effective_item.release_date.as_deref(),
                effective_item.digital_release_date.as_deref(),
                effective_item.release_status.as_deref(),
                &today_str,
            );

            if !avail.is_released {
                if effective_item.status != "unreleased" || metadata_updated {
                    effective_item.status = "unreleased".to_string();
                    let dl_svc = state.downloads.lock().await;
                    let _ = dl_svc.db().update_media_release_info(
                        &effective_item.id,
                        effective_item.year,
                        effective_item.release_date.as_deref(),
                        effective_item.digital_release_date.as_deref(),
                        effective_item.release_status.as_deref(),
                        Some("unreleased"),
                        now,
                    );
                }

                let diag = ag_state
                    .diagnostics
                    .entry(effective_item.id.clone())
                    .or_default();
                diag.last_search_ms = now;
                diag.last_status = "unreleased".to_string();
                diag.summary_message = match avail.available_from {
                    Some(ref dt) => format!("{} · Disponible aprox.: {dt}", avail.reason),
                    None => avail.reason,
                };
                let reason_log = diag.summary_message.clone();
                Self::save_state(state, ag_state).await;
                tracing::debug!(
                    title = %effective_item.title,
                    reason = %reason_log,
                    "AutoGrabber omite película por estrenar"
                );
                return None;
            } else if effective_item.status == "unreleased" {
                effective_item.status = "wanted".to_string();
                let dl_svc = state.downloads.lock().await;
                let _ = dl_svc.db().update_media_release_info(
                    &effective_item.id,
                    effective_item.year,
                    effective_item.release_date.as_deref(),
                    effective_item.digital_release_date.as_deref(),
                    effective_item.release_status.as_deref(),
                    Some("wanted"),
                    now,
                );
                tracing::info!(
                    title = %effective_item.title,
                    "Película estrenada: promocionada automáticamente de 'unreleased' a 'wanted'"
                );
            }
        }
        let item = &effective_item;
        let (upgrade_enabled, max_res) = {
            let dl_svc = state.downloads.lock().await;
            let upg = dl_svc
                .get_setting("quality_upgrade_enabled")
                .ok()
                .flatten()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true);
            let res = dl_svc
                .get_setting("quality_max_resolution")
                .ok()
                .flatten()
                .unwrap_or_else(|| "1080p".to_string());
            (upg, res)
        };
        let upgrade_ctx = if upgrade_enabled || force {
            Self::check_movie_upgrade_eligibility(item, ag_state, now, force).await
        } else {
            None
        };
        let is_upgrade = upgrade_ctx.is_some();

        if item.status != "wanted" && !is_upgrade {
            return None;
        }

        // Comprobar el retroceso inteligente si no está forzado
        if !force && item.status == "wanted" {
            if let Some(diag) = ag_state.diagnostics.get(&item.id) {
                if now < diag.next_eligible_search_ms {
                    return None;
                }
            }
        }

        let mut queries = Vec::new();
        let clean_title = item.title.replace([':', '-', '\''], " ");
        if let Some(year) = item.year {
            queries.push(format!("{} {year}", item.title));
            if clean_title != item.title {
                queries.push(format!("{clean_title} {year}"));
            }
        } else {
            queries.push(item.title.clone());
            if clean_title != item.title {
                queries.push(clean_title.clone());
            }
        }
        if let Some(ref orig) = item.original_title {
            if !orig.is_empty() && !orig.eq_ignore_ascii_case(&item.title) {
                let clean_orig = orig.replace([':', '-', '\''], " ");
                if let Some(year) = item.year {
                    queries.push(format!("{orig} {year}"));
                    if clean_orig != *orig {
                        queries.push(format!("{clean_orig} {year}"));
                    }
                } else {
                    queries.push(orig.clone());
                    if clean_orig != *orig {
                        queries.push(clean_orig);
                    }
                }
            }
        }

        let es_variants =
            spanish_title_search_variants(&item.title, item.original_title.as_deref());
        for es_t in &es_variants {
            if let Some(year) = item.year {
                queries.push(format!("{es_t} {year}"));
            }
            queries.push(es_t.clone());
        }

        // Títulos alternativos y traducciones oficiales proporcionados por el servicio de metadatos
        let ext_details = state.media_metadata.get_extended_details(item).await;
        for alt_title in &ext_details.alternative_titles {
            let clean_alt = alt_title.replace([':', '-', '\''], " ");
            if let Some(year) = item.year {
                queries.push(format!("{alt_title} {year}"));
                if clean_alt != *alt_title {
                    queries.push(format!("{clean_alt} {year}"));
                }
            } else {
                queries.push(alt_title.clone());
                if clean_alt != *alt_title {
                    queries.push(clean_alt);
                }
            }
        }

        // Dividir por ':' (ej. "Ali G: Who Iz I?" -> "Ali G", "Who Iz I")
        if item.title.contains(':') {
            let parts: Vec<&str> = item.title.split(':').collect();
            if parts.len() >= 2 {
                let prefix = parts[0].trim();
                let suffix = parts[1].trim();
                if !prefix.is_empty() {
                    if let Some(year) = item.year {
                        queries.push(format!("{prefix} {year}"));
                    }
                    queries.push(prefix.to_string());
                }
                if !suffix.is_empty() {
                    if let Some(year) = item.year {
                        queries.push(format!("{suffix} {year}"));
                    }
                    queries.push(suffix.to_string());
                }
            }
        }

        // Variantes en español y números para secuelas (ej. "Part Two" -> "Parte Dos", "Parte 2", "2")
        let title_lower = item.title.to_lowercase();
        if title_lower.contains("part two") || title_lower.contains("part 2") {
            let base = item.title.split(':').next().unwrap_or(&item.title).trim();
            if let Some(year) = item.year {
                queries.push(format!("{base} Parte Dos {year}"));
                queries.push(format!("{base} Parte 2 {year}"));
                queries.push(format!("{base} 2 {year}"));
            } else {
                queries.push(format!("{base} Parte Dos"));
                queries.push(format!("{base} Parte 2"));
                queries.push(format!("{base} 2"));
            }
        } else if title_lower.contains("part three") || title_lower.contains("part 3") {
            let base = item.title.split(':').next().unwrap_or(&item.title).trim();
            if let Some(year) = item.year {
                queries.push(format!("{base} Parte Tres {year}"));
                queries.push(format!("{base} Parte 3 {year}"));
                queries.push(format!("{base} 3 {year}"));
            } else {
                queries.push(format!("{base} Parte Tres"));
                queries.push(format!("{base} Parte 3"));
                queries.push(format!("{base} 3"));
            }
        }

        let effective_profile = match max_res.as_str() {
            "2160p" if !item.quality_profile.contains("2160p") && !item.quality_profile.contains("4k") => {
                format!("{}-2160p", item.quality_profile)
            }
            "720p" => "720p-es".to_string(),
            _ => item.quality_profile.clone(),
        };

        let search_futs = queries
            .iter()
            .map(|q| aggregator.search_all(state, q, true, &effective_profile));
        let query_batches = join_all(search_futs).await;

        let mut all_releases = Vec::new();
        let mut seen_links = HashSet::new();
        for rels in query_batches {
            for r in rels {
                if seen_links.insert(r.link.clone()) {
                    all_releases.push(r);
                }
            }
        }

        let total_candidates = all_releases.len();
        let mut rejection_counts: HashMap<String, usize> = HashMap::new();
        let mut valid_releases = Vec::new();

        for r in all_releases {
            if ag_state.is_release_blacklisted(&item.id, &r.hash, &r.title) {
                *rejection_counts
                    .entry("En lista negra (descartada)".to_string())
                    .or_insert(0) += 1;
                continue;
            }
            if r.is_rejected {
                let reason = r
                    .rejection_reason
                    .clone()
                    .unwrap_or_else(|| "Calidad/idioma no válido".to_string());
                *rejection_counts.entry(reason).or_insert(0) += 1;
                continue;
            }
            let alt_refs: Vec<&str> = ext_details.alternative_titles.iter().map(|s| s.as_str()).collect();
            if !matches_release_with_alts(
                &r.title,
                &item.title,
                item.original_title.as_deref(),
                &alt_refs,
                item.year,
                true,
                None,
            ) {
                *rejection_counts
                    .entry("No coincide título/año exacto o es secuela".to_string())
                    .or_insert(0) += 1;
                continue;
            }
            if r.score < 20 {
                *rejection_counts
                    .entry("Puntuación insuficiente".to_string())
                    .or_insert(0) += 1;
                continue;
            }
            if let Some((_, _, current_score)) = upgrade_ctx {
                let is_better_lang = r.language == "Castellano" || r.language == "Dual";
                let wants_4k = item.quality_profile.to_lowercase().contains("4k")
                    || item.quality_profile.to_lowercase().contains("2160p");
                let is_acceptable_hd = r.quality == "1080p" || (wants_4k && r.quality == "2160p");
                if !is_better_lang || !is_acceptable_hd || r.score < current_score + MIN_UPGRADE_SCORE_DELTA {
                    *rejection_counts
                        .entry("No supera la versión actual para Upgrade".to_string())
                        .or_insert(0) += 1;
                    continue;
                }
            }
            valid_releases.push(r);
        }

        valid_releases.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| b.sources.cmp(&a.sources))
        });

        let valid_count = valid_releases.len();
        let mut top_reasons: Vec<(String, usize)> = rejection_counts.into_iter().collect();
        top_reasons.sort_by_key(|a| std::cmp::Reverse(a.1));
        let formatted_reasons: Vec<String> = top_reasons
            .iter()
            .take(3)
            .map(|(r, c)| format!("{c}× {r}"))
            .collect();

        if let Some(best) = valid_releases.into_iter().next() {
            // Guardar temporalmente el estado antes de `grab_release`, que lo recarga
            Self::save_state(state, ag_state).await;
            if let Ok(dl_id) = Self::grab_release(state, &item.id, &best).await {
                *ag_state = Self::load_state(state).await;
                if let Some(diag) = ag_state.diagnostics.get_mut(&item.id) {
                    diag.total_candidates = total_candidates;
                    diag.valid_candidates = valid_count;
                    diag.top_rejection_reasons = formatted_reasons;
                    diag.summary_message = format!(
                        "Analizadas {} releases ({} válidas) → Seleccionada '{}' [{} · {} · {} pts]",
                        total_candidates, valid_count, best.title, best.quality, best.language, best.score
                    );
                }
                Self::save_state(state, ag_state).await;
                return Some(json!({
                    "title": item.title,
                    "release": best.title,
                    "protocol": best.protocol,
                    "quality": best.quality,
                    "language": best.language,
                    "score": best.score,
                    "isUpgrade": is_upgrade,
                    "downloadId": dl_id
                }));
            }
        }

        // Ninguna versión válida encontrada -> actualizar el diagnóstico y el retroceso inteligente
        let diag = ag_state.diagnostics.entry(item.id.clone()).or_default();
        diag.last_search_ms = now;
        diag.total_candidates = total_candidates;
        diag.valid_candidates = 0;
        diag.consecutive_empty_searches = diag.consecutive_empty_searches.saturating_add(1);
        diag.next_eligible_search_ms =
            Self::compute_next_backoff_ms(now, diag.consecutive_empty_searches);
        diag.top_rejection_reasons = formatted_reasons.clone();

        if is_upgrade {
            diag.last_status = "completed_monitoring_upgrade".to_string();
            diag.summary_message = format!(
                "Búsqueda de Upgrade ({total_candidates} analizadas): aún no hay versión superior en Castellano/Dual 1080p+."
            );
        } else if total_candidates == 0 {
            diag.last_status = "no_results".to_string();
            diag.summary_message =
                "0 resultados encontrados en la red Kademlia/Torznab. Reintento programado con Smart Backoff.".to_string();
        } else {
            diag.last_status = "rejected_all".to_string();
            diag.summary_message = format!(
                "Analizadas {} releases, ninguna superó el filtro estricto ({})",
                total_candidates,
                formatted_reasons.join(" · ")
            );
        }
        Self::save_state(state, ag_state).await;
        None
    }
}

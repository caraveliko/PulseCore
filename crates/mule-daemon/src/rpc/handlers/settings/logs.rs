// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde_json::{json, Value};

use crate::error::JsonRpcError;
use crate::state::AppState;

const LOG_READ_CHUNK_BYTES: usize = 64 * 1024;
const LOG_MAX_SCAN_BYTES: u64 = 16 * 1024 * 1024;
const LOG_MAX_LINE_BYTES: usize = 256 * 1024;

/// Devuelve entradas de registro del daemon procesadas a partir de archivos JSON rotativos
pub async fn handle_logs(state: &AppState, params: Option<Value>) -> Result<Value, JsonRpcError> {
    let limit = params
        .as_ref()
        .and_then(|p| p.get("limit"))
        .and_then(|v| v.as_u64())
        .unwrap_or(200)
        .min(1000) as usize;

    let filter_level = params
        .as_ref()
        .and_then(|p| p.get("level"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_uppercase());

    let search_query = params
        .as_ref()
        .and_then(|p| p.get("search"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase());

    let logs_dir = state.config.data_dir.join("logs");

    tokio::task::spawn_blocking(move || {
        let entries = read_recent_log_entries(
            &logs_dir,
            limit,
            filter_level.as_deref(),
            search_query.as_deref(),
            LOG_MAX_SCAN_BYTES,
        );

        Ok(json!({
            "logs": entries,
            "total": entries.len()
        }))
    })
    .await
    .map_err(|e| JsonRpcError::internal(format!("Logs task error: {e}")))?
}

fn read_recent_log_entries(
    logs_dir: &std::path::Path,
    limit: usize,
    filter_level: Option<&str>,
    search_query: Option<&str>,
    max_scan_bytes: u64,
) -> Vec<Value> {
    let mut entries = Vec::new();
    if limit == 0 || !logs_dir.is_dir() {
        return entries;
    }

    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(logs_dir)
        .ok()
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("daemon") && n.ends_with(".log"))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();

    files.sort();
    files.reverse(); // los más recientes primero

    let mut remaining_budget = max_scan_bytes;
    for file_path in files {
        if entries.len() >= limit || remaining_budget == 0 {
            break;
        }
        if let Ok(mut file) = std::fs::File::open(&file_path) {
            scan_log_file_reverse(
                &mut file,
                limit,
                filter_level,
                search_query,
                &mut remaining_budget,
                &mut entries,
            );
        }
    }

    entries.reverse();
    entries
}

fn scan_log_file_reverse(
    file: &mut std::fs::File,
    limit: usize,
    filter_level: Option<&str>,
    search_query: Option<&str>,
    remaining_budget: &mut u64,
    entries: &mut Vec<Value>,
) {
    use std::io::{Read, Seek, SeekFrom};

    let Ok(meta) = file.metadata() else {
        return;
    };
    let mut pos = meta.len();
    let mut buf = vec![0u8; LOG_READ_CHUNK_BYTES];
    let mut carry: Vec<u8> = Vec::new();
    let mut skip_oversized = false;

    while pos > 0 && *remaining_budget > 0 && entries.len() < limit {
        let to_read = pos
            .min(*remaining_budget)
            .min(LOG_READ_CHUNK_BYTES as u64) as usize;
        if to_read == 0 {
            break;
        }
        pos -= to_read as u64;
        *remaining_budget -= to_read as u64;

        if file.seek(SeekFrom::Start(pos)).is_err() {
            break;
        }
        if file.read_exact(&mut buf[..to_read]).is_err() {
            break;
        }

        let chunk = &buf[..to_read];
        let Some(last_nl) = chunk.iter().rposition(|&b| b == b'\n') else {
            if skip_oversized {
                continue;
            }
            if chunk.len() + carry.len() > LOG_MAX_LINE_BYTES {
                carry.clear();
                skip_oversized = true;
            } else if carry.is_empty() {
                carry.extend_from_slice(chunk);
            } else {
                let mut combined = Vec::with_capacity(chunk.len() + carry.len());
                combined.extend_from_slice(chunk);
                combined.extend_from_slice(&carry);
                carry = combined;
            }
            continue;
        };

        // El tramo posterior al último '\n' completa la línea acumulada en `carry`.
        let tail = &chunk[last_nl + 1..];
        if !skip_oversized {
            if carry.is_empty() {
                if tail.len() <= LOG_MAX_LINE_BYTES {
                    if let Some(entry) = try_parse_log_line(tail, filter_level, search_query) {
                        entries.push(entry);
                        if entries.len() >= limit {
                            return;
                        }
                    }
                }
            } else if tail.len() + carry.len() <= LOG_MAX_LINE_BYTES {
                let mut line = Vec::with_capacity(tail.len() + carry.len());
                line.extend_from_slice(tail);
                line.extend_from_slice(&carry);
                if let Some(entry) = try_parse_log_line(&line, filter_level, search_query) {
                    entries.push(entry);
                    if entries.len() >= limit {
                        return;
                    }
                }
            }
        }
        carry.clear();
        skip_oversized = false;

        let mut seg_end = last_nl;
        while let Some(prev_nl) = chunk[..seg_end].iter().rposition(|&b| b == b'\n') {
            let line = &chunk[prev_nl + 1..seg_end];
            if line.len() <= LOG_MAX_LINE_BYTES {
                if let Some(entry) = try_parse_log_line(line, filter_level, search_query) {
                    entries.push(entry);
                    if entries.len() >= limit {
                        return;
                    }
                }
            }
            seg_end = prev_nl;
        }

        let prefix = &chunk[..seg_end];
        if prefix.len() > LOG_MAX_LINE_BYTES {
            skip_oversized = true;
        } else {
            carry.extend_from_slice(prefix);
        }
    }

    if pos == 0 && entries.len() < limit && !skip_oversized && !carry.is_empty() {
        if let Some(entry) = try_parse_log_line(&carry, filter_level, search_query) {
            entries.push(entry);
        }
    }
}

fn try_parse_log_line(
    raw: &[u8],
    filter_level: Option<&str>,
    search_query: Option<&str>,
) -> Option<Value> {
    let trimmed = raw.strip_suffix(b"\r").unwrap_or(raw);
    if trimmed.is_empty() {
        return None;
    }
    let line_str = std::str::from_utf8(trimmed).ok()?;
    let v = serde_json::from_str::<Value>(line_str).ok()?;
    let level = v.get("level").and_then(|l| l.as_str()).unwrap_or("INFO");
    if let Some(fl) = filter_level {
        if fl != "ALL" && level != fl {
            return None;
        }
    }

    let target = v.get("target").and_then(|t| t.as_str()).unwrap_or("");
    let msg = v
        .get("fields")
        .and_then(|f| f.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("");

    if let Some(sq) = search_query {
        if !msg.to_lowercase().contains(sq) && !target.to_lowercase().contains(sq) {
            return None;
        }
    }

    let ts = v.get("timestamp").and_then(|t| t.as_str()).unwrap_or("");
    Some(json!({
        "timestamp": ts,
        "level": level,
        "target": target,
        "message": msg
    }))
}

/// Lista subdirectorios existentes en el servidor para el explorador de carpetas de la WebUI.
#[cfg(test)]
mod logs_tests {
    use super::*;
    use std::io::Write;

    fn make_log_line(idx: usize, level: &str, target: &str, msg: &str) -> String {
        json!({
            "timestamp": format!("2026-11-21T12:00:{:02}.000Z", idx % 60),
            "level": level,
            "target": target,
            "fields": { "message": msg }
        })
        .to_string()
    }

    #[test]
    fn multi_chunk_file_returns_last_entries_in_chronological_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.2026-11-21.log");
        let mut f = std::fs::File::create(&path).unwrap();
        let total_lines = 1500;
        for i in 0..total_lines {
            let line = make_log_line(i, "INFO", "mule_daemon::test", &format!("entry-{i:04}-payload-padding-0123456789abcdef"));
            writeln!(f, "{line}").unwrap();
        }
        assert!(f.metadata().unwrap().len() > 150 * 1024);

        let entries = read_recent_log_entries(dir.path(), 5, None, None, LOG_MAX_SCAN_BYTES);
        assert_eq!(entries.len(), 5);
        for (offset, entry) in entries.iter().enumerate() {
            let expected = format!("entry-{:04}-payload-padding-0123456789abcdef", total_lines - 5 + offset);
            assert_eq!(entry["message"].as_str().unwrap(), expected);
        }
    }

    #[test]
    fn multibyte_utf8_across_64k_boundary_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.2026-11-21.log");
        let mut content = String::new();
        // Llenar hasta rodear la frontera exacta de 64 KiB con una línea que contiene acentos y emojis.
        let filler = make_log_line(0, "INFO", "mule_daemon::utf8", &"a".repeat(200));
        while content.len() + filler.len() + 1 < LOG_READ_CHUNK_BYTES - 40 {
            content.push_str(&filler);
            content.push('\n');
        }
        let boundary_msg = "Señal crítica 🚀 café árbol ñandú ".repeat(4);
        let boundary_line = make_log_line(1, "WARN", "mule_daemon::utf8", &boundary_msg);
        content.push_str(&boundary_line);
        content.push('\n');
        content.push_str(&make_log_line(2, "INFO", "mule_daemon::utf8", "final-line"));
        content.push('\n');

        std::fs::write(&path, content.as_bytes()).unwrap();

        let entries = read_recent_log_entries(dir.path(), 3, None, None, LOG_MAX_SCAN_BYTES);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[1]["message"].as_str().unwrap(), boundary_msg);
        assert_eq!(entries[2]["message"].as_str().unwrap(), "final-line");
    }

    #[test]
    fn rollover_across_multiple_log_files() {
        let dir = tempfile::tempdir().unwrap();
        let old_path = dir.path().join("daemon.2026-11-20.log");
        let new_path = dir.path().join("daemon.2026-11-21.log");

        std::fs::write(
            &old_path,
            format!(
                "{}\n{}\n{}\n",
                make_log_line(0, "INFO", "t", "old-1"),
                make_log_line(1, "INFO", "t", "old-2"),
                make_log_line(2, "INFO", "t", "old-3"),
            ),
        )
        .unwrap();
        std::fs::write(
            &new_path,
            format!(
                "{}\n{}\n",
                make_log_line(3, "INFO", "t", "new-1"),
                make_log_line(4, "INFO", "t", "new-2"),
            ),
        )
        .unwrap();

        let entries = read_recent_log_entries(dir.path(), 4, None, None, LOG_MAX_SCAN_BYTES);
        let msgs: Vec<&str> = entries.iter().map(|e| e["message"].as_str().unwrap()).collect();
        assert_eq!(msgs, vec!["old-2", "old-3", "new-1", "new-2"]);
    }

    #[test]
    fn filters_crlf_and_missing_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.2026-11-21.log");
        let raw = format!(
            "{}\r\n{}\r\n{}\r\n{}",
            make_log_line(0, "INFO", "mod::a", "hola mundo"),
            make_log_line(1, "WARN", "mod::net", "Timeout crítico"),
            make_log_line(2, "ERROR", "mod::disk", "Otra cosa"),
            make_log_line(3, "WARN", "mod::Net", "Último sin salto"),
        );
        std::fs::write(&path, raw.as_bytes()).unwrap();

        let entries = read_recent_log_entries(
            dir.path(),
            10,
            Some("WARN"),
            Some("net"),
            LOG_MAX_SCAN_BYTES,
        );
        let msgs: Vec<&str> = entries.iter().map(|e| e["message"].as_str().unwrap()).collect();
        assert_eq!(msgs, vec!["Timeout crítico", "Último sin salto"]);

        let all_entries = read_recent_log_entries(dir.path(), 10, Some("ALL"), None, LOG_MAX_SCAN_BYTES);
        assert_eq!(all_entries.len(), 4);
    }

    #[test]
    fn scan_budget_stops_cleanly_without_partial_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.2026-11-21.log");
        let mut raw = String::new();
        for i in 0..100 {
            raw.push_str(&make_log_line(i, "INFO", "mod::budget", &format!("msg-{i:03}")));
            raw.push('\n');
        }
        std::fs::write(&path, raw.as_bytes()).unwrap();

        // Presupuesto menor que el fichero completo: debe parar limpio y devolver solo líneas completas finales.
        let budget = 500u64;
        let entries = read_recent_log_entries(dir.path(), 100, None, None, budget);
        assert!(!entries.is_empty());
        assert!(entries.len() < 10);
        assert_eq!(entries.last().unwrap()["message"].as_str().unwrap(), "msg-099");
        // La línea 0 no pudo leerse dentro de 500 bytes y no debe aparecer ni truncada.
        assert_ne!(entries.first().unwrap()["message"].as_str().unwrap(), "msg-000");
    }
}

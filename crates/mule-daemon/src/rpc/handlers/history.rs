// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde_json::{json, Value};

use crate::error::JsonRpcError;
use crate::state::AppState;

/// Extrae explícitamente los campos opcionales de `HistoryFilter` desde un objeto JSON.
fn parse_history_filter(params: &Value) -> mule_core::HistoryFilter {
    let Some(obj) = params.as_object() else {
        return mule_core::HistoryFilter::default();
    };
    mule_core::HistoryFilter {
        event_type: obj
            .get("eventType")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        status: obj
            .get("status")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        from_timestamp_ms: obj.get("fromTimestampMs").and_then(Value::as_i64),
        to_timestamp_ms: obj.get("toTimestampMs").and_then(Value::as_i64),
        limit: obj
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok()),
        offset: obj
            .get("offset")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok()),
    }
}

pub async fn handle_history_list(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let filter = params
        .as_ref()
        .map(parse_history_filter)
        .unwrap_or_default();

    let entries = state.downloads.lock().await.query_history(&filter)?;
    Ok(crate::rpc::handlers::json_value(&entries))
}

pub async fn handle_history_stats(state: &AppState) -> Result<Value, JsonRpcError> {
    let stats = state.downloads.lock().await.get_history_stats()?;
    Ok(crate::rpc::handlers::json_value(&stats))
}

pub async fn handle_history_clear(state: &AppState) -> Result<Value, JsonRpcError> {
    let cleared_count = state.downloads.lock().await.clear_history()?;
    Ok(json!({
        "success": true,
        "clearedCount": cleared_count
    }))
}

pub async fn handle_history_export(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let format = params["format"].as_str().unwrap_or("json");

    let filter = parse_history_filter(&params);

    let dls = state.downloads.lock().await;
    let content = dls.export_history(format, &filter)?;
    let entries = dls.query_history(&filter)?;

    Ok(json!({
        "format": format.to_ascii_lowercase(),
        "content": content,
        "count": entries.len()
    }))
}

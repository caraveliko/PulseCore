// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use crate::client::DaemonClient;
use crate::protocol::ToolDefinition;
use super::{obj_schema, prop};
use serde_json::{json, Value};

pub fn list_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "emule_status".to_string(),
            description: "Get overall eMuleRust daemon status, uptime, transfer speeds, active downloads count, server connection, and Kad status.".to_string(),
            input_schema: obj_schema(json!({}), &[]),
        },
        ToolDefinition {
            name: "emule_get_settings".to_string(),
            description: "Get eMuleRust daemon configuration, default paths, limits, and retention policy.".to_string(),
            input_schema: obj_schema(json!({}), &[]),
        },
        ToolDefinition {
            name: "emule_update_settings".to_string(),
            description: "Update daemon settings: default download folder, history retention, peer connection quotas.".to_string(),
            input_schema: obj_schema(json!({"default_download_dir": prop("string", "Absolute path to the new default completed downloads folder"),
                    "history_retention_days": prop("integer", "Number of days to keep download history records (0 = forever)"),
                    "max_global_download_peers": prop("integer", "Maximum number of simultaneous peer connections across all downloads"),
                    "max_peers_per_download": prop("integer", "Maximum active peer connections allowed for a single download"),
                    "min_peers_per_download": prop("integer", "Target minimum peers to maintain per active download")}), &[]),
        },
        ToolDefinition {
            name: "emule_reset_download_dir".to_string(),
            description: "Reset the default download directory to the default data folder.".to_string(),
            input_schema: obj_schema(json!({}), &[]),
        },
        ToolDefinition {
            name: "emule_set_limits".to_string(),
            description: "Configure global download and upload rate limits in KB/s (0 = unlimited).".to_string(),
            input_schema: obj_schema(json!({"download_limit_kbps": prop("integer", "Max download rate in KB/s (0 for unlimited)"),
                    "upload_limit_kbps": prop("integer", "Max upload rate in KB/s (0 for unlimited)")}), &[]),
        },
        ToolDefinition {
            name: "emule_shutdown".to_string(),
            description: "Gracefully shut down the eMuleRust background daemon.".to_string(),
            input_schema: obj_schema(json!({}), &[]),
        },
        ToolDefinition {
            name: "emule_get_network_policy".to_string(),
            description: "Get active network security policy, profile ('standard', 'vpn_only', 'tor_proxy', 'paranoid', 'local_lab'), budgets, and active connection counts.".to_string(),
            input_schema: obj_schema(json!({}), &[]),
        },
        ToolDefinition {
            name: "emule_set_network_profile".to_string(),
            description: "Switch the network isolation profile ('standard', 'vpn_only', 'tor_proxy', 'paranoid', 'local_lab').".to_string(),
            input_schema: obj_schema(json!({"profile": {
                        "type": "string",
                        "enum": ["standard", "vpn_only", "tor_proxy", "paranoid", "local_lab"],
                        "description": "Target network profile"
                    }}), &["profile"]),
        },
        ToolDefinition {
            name: "emule_network_disconnect_all".to_string(),
            description: "Emergency network killswitch: immediately disconnects all server, peer, and listener sockets.".to_string(),
            input_schema: obj_schema(json!({}), &[]),
        },
        ToolDefinition {
            name: "emule_network_enable".to_string(),
            description: "Re-enable network communications after a killswitch disconnect.".to_string(),
            input_schema: obj_schema(json!({}), &[]),
        },
        ToolDefinition {
            name: "emule_get_audit_log".to_string(),
            description: "Inspect network security audit log events (blocked connections, profile violations, rate limiting).".to_string(),
            input_schema: obj_schema(json!({"limit": prop("integer", "Max entries to retrieve (default: 50)")}), &[]),
        },
        ToolDefinition {
            name: "emule_set_network_budgets".to_string(),
            description: "Set network bandwidth and connection budgets dynamically.".to_string(),
            input_schema: obj_schema(json!({"budgets": prop("object", "Budget definitions for rate limits and concurrent socket limits")}), &["budgets"]),
        },
        ToolDefinition {
            name: "emule_get_filter_status".to_string(),
            description: "Get IP filter security status, loaded rules count, and blocked connection statistics.".to_string(),
            input_schema: obj_schema(json!({}), &[]),
        },
        ToolDefinition {
            name: "emule_import_ip_filter".to_string(),
            description: "Import IP blocklist file (ipfilter.dat, guarding.p2p, or peerguardian format) to block malicious peers.".to_string(),
            input_schema: obj_schema(json!({"file_path": prop("string", "Path to ipfilter.dat file")}), &["file_path"]),
        },
        ToolDefinition {
            name: "emule_set_filter_enabled".to_string(),
            description: "Enable or disable IP blocklist filtering dynamically.".to_string(),
            input_schema: obj_schema(json!({"enabled": prop("boolean", "Whether the IP filter is active")}), &["enabled"]),
        },
        ToolDefinition {
            name: "emule_clear_ip_filter".to_string(),
            description: "Clear all IP filter rules from memory.".to_string(),
            input_schema: obj_schema(json!({}), &[]),
        },
        ToolDefinition {
            name: "emule_disk_space".to_string(),
            description: "Query local and NAS disk space usage (total, used, free bytes).".to_string(),
            input_schema: obj_schema(json!({"path": prop("string", "Optional directory path to inspect")}), &[]),
        },
        ToolDefinition {
            name: "emule_nas_info".to_string(),
            description: "Query NAS mount status and media directory file counts.".to_string(),
            input_schema: obj_schema(json!({}), &[]),
        },
        ToolDefinition {
            name: "emule_system_logs".to_string(),
            description: "Read recent daemon log lines with optional substring filter.".to_string(),
            input_schema: obj_schema(json!({
                "lines": prop("integer", "Number of recent log lines to return (default: 150)"),
                "filter": prop("string", "Optional case-insensitive filter substring")
            }), &[]),
        },
        ToolDefinition {
            name: "emule_media_list".to_string(),
            description: "List items in the native Servarr media library (movies and series).".to_string(),
            input_schema: obj_schema(json!({
                "media_type": prop("string", "Optional filter ('movie' or 'series')"),
                "status": prop("string", "Optional filter ('wanted', 'downloading', 'available')")
            }), &[]),
        },
        ToolDefinition {
            name: "emule_search_all".to_string(),
            description: "Unified multi-protocol search across eD2K servers, Kad DHT, and Torznab indexers.".to_string(),
            input_schema: obj_schema(json!({
                "term": prop("string", "Search query keywords"),
                "min_size": prop("integer", "Optional minimum file size in bytes"),
                "max_size": prop("integer", "Optional maximum file size in bytes"),
                "extension": prop("string", "Optional file extension filter")
            }), &["term"]),
        },
    ]
}

pub async fn handle_tool(
    client: &DaemonClient,
    tool_name: &str,
    args: &Value,
) -> Option<crate::protocol::ToolCallResult> {
    use crate::protocol::ToolCallResult;
    match tool_name {
        "emule_status" => match client.call("system.status", json!({})).await {
            Ok(res) => Some(ToolCallResult::json(&res)),
            Err(e) => Some(ToolCallResult::error(format!(
                "Failed to retrieve status: {e}"
            ))),
        },
        "emule_get_settings" => match client.call("settings.get", json!({})).await {
            Ok(res) => Some(ToolCallResult::json(&res)),
            Err(e) => Some(ToolCallResult::error(format!(
                "Failed to get settings: {e}"
            ))),
        },
        "emule_update_settings" => {
            let mut params = json!({});
            if let Some(d) = args.get("default_download_dir").and_then(|s| s.as_str()) {
                params["defaultDownloadDir"] = json!(d);
            }
            if let Some(r) = args.get("history_retention_days").and_then(|s| s.as_u64()) {
                params["historyRetentionDays"] = json!(r);
            }
            if let Some(g) = args
                .get("max_global_download_peers")
                .and_then(|s| s.as_u64())
            {
                params["maxGlobalDownloadPeers"] = json!(g);
            }
            if let Some(p) = args.get("max_peers_per_download").and_then(|s| s.as_u64()) {
                params["maxPeersPerDownload"] = json!(p);
            }
            if let Some(m) = args.get("min_peers_per_download").and_then(|s| s.as_u64()) {
                params["minPeersPerDownload"] = json!(m);
            }

            match client.call("settings.set", params).await {
                Ok(res) => Some(ToolCallResult::json(&res)),
                Err(e) => Some(ToolCallResult::error(format!(
                    "Failed to update settings: {e}"
                ))),
            }
        }
        "emule_reset_download_dir" => {
            match client.call("settings.resetDownloadDir", json!({})).await {
                Ok(res) => Some(ToolCallResult::json(&res)),
                Err(e) => Some(ToolCallResult::error(format!(
                    "Failed to reset download directory: {e}"
                ))),
            }
        }
        "emule_set_limits" => {
            let mut params = json!({});
            if let Some(dl) = args.get("download_limit_kbps").and_then(|d| d.as_u64()) {
                params["downloadLimit"] = json!(dl);
            }
            if let Some(ul) = args.get("upload_limit_kbps").and_then(|u| u.as_u64()) {
                params["uploadLimit"] = json!(ul);
            }

            match client.call("limits.set", params).await {
                Ok(res) => Some(ToolCallResult::json(&res)),
                Err(e) => Some(ToolCallResult::error(format!(
                    "Failed to set rate limits: {e}"
                ))),
            }
        }
        "emule_shutdown" => match client.call("system.stop", json!({})).await {
            Ok(res) => Some(ToolCallResult::json(&res)),
            Err(e) => Some(ToolCallResult::error(format!(
                "Failed to shut down daemon: {e}"
            ))),
        },
        "emule_get_network_policy" => match client.call("network.getPolicy", json!({})).await {
            Ok(res) => Some(ToolCallResult::json(&res)),
            Err(e) => Some(ToolCallResult::error(format!(
                "Failed to get network policy: {e}"
            ))),
        },
        "emule_set_network_profile" => {
            let profile = match args.get("profile").and_then(|p| p.as_str()) {
                Some(p) => p,
                None => {
                    return Some(ToolCallResult::error(
                        "Missing required parameter: 'profile'",
                    ))
                }
            };

            match client
                .call("network.setProfile", json!({ "profile": profile }))
                .await
            {
                Ok(res) => Some(ToolCallResult::json(&res)),
                Err(e) => Some(ToolCallResult::error(format!(
                    "Failed to set network profile: {e}"
                ))),
            }
        }
        "emule_network_disconnect_all" => {
            match client.call("network.disconnectAll", json!({})).await {
                Ok(res) => Some(ToolCallResult::json(&res)),
                Err(e) => Some(ToolCallResult::error(format!(
                    "Failed to trigger network disconnect: {e}"
                ))),
            }
        }
        "emule_network_enable" => match client.call("network.enable", json!({})).await {
            Ok(res) => Some(ToolCallResult::json(&res)),
            Err(e) => Some(ToolCallResult::error(format!(
                "Failed to enable network: {e}"
            ))),
        },
        "emule_get_audit_log" => {
            let mut params = json!({});
            if let Some(l) = args.get("limit").and_then(|s| s.as_u64()) {
                params["limit"] = json!(l);
            }

            match client.call("network.getAuditLog", params).await {
                Ok(res) => Some(ToolCallResult::json(&res)),
                Err(e) => Some(ToolCallResult::error(format!(
                    "Failed to get network audit log: {e}"
                ))),
            }
        }
        "emule_set_network_budgets" => {
            let budgets = match args.get("budgets") {
                Some(b) => b,
                None => {
                    return Some(ToolCallResult::error(
                        "Missing required parameter: 'budgets'",
                    ))
                }
            };

            match client
                .call("network.setBudgets", json!({ "budgets": budgets }))
                .await
            {
                Ok(res) => Some(ToolCallResult::json(&res)),
                Err(e) => Some(ToolCallResult::error(format!(
                    "Failed to set network budgets: {e}"
                ))),
            }
        }
        "emule_get_filter_status" => match client.call("filter.getStatus", json!({})).await {
            Ok(res) => Some(ToolCallResult::json(&res)),
            Err(e) => Some(ToolCallResult::error(format!(
                "Failed to get IP filter status: {e}"
            ))),
        },
        "emule_import_ip_filter" => {
            let file_path = match args.get("file_path").and_then(|f| f.as_str()) {
                Some(f) => f,
                None => {
                    return Some(ToolCallResult::error(
                        "Missing required parameter: 'file_path'",
                    ))
                }
            };

            match client
                .call("filter.importFile", json!({ "filePath": file_path }))
                .await
            {
                Ok(res) => Some(ToolCallResult::json(&res)),
                Err(e) => Some(ToolCallResult::error(format!(
                    "Failed to import IP filter: {e}"
                ))),
            }
        }
        "emule_set_filter_enabled" => {
            let enabled = match args.get("enabled").and_then(|e| e.as_bool()) {
                Some(e) => e,
                None => {
                    return Some(ToolCallResult::error(
                        "Missing required parameter: 'enabled'",
                    ))
                }
            };

            match client
                .call("filter.setEnabled", json!({ "enabled": enabled }))
                .await
            {
                Ok(res) => Some(ToolCallResult::json(&res)),
                Err(e) => Some(ToolCallResult::error(format!(
                    "Failed to set filter enabled state: {e}"
                ))),
            }
        }
        "emule_clear_ip_filter" => match client.call("filter.clear", json!({})).await {
            Ok(res) => Some(ToolCallResult::json(&res)),
            Err(e) => Some(ToolCallResult::error(format!(
                "Failed to clear IP filter: {e}"
            ))),
        },
        "emule_disk_space" => {
            let mut params = json!({});
            if let Some(p) = args.get("path").and_then(|s| s.as_str()) {
                params["path"] = json!(p);
            }
            match client.call("system.diskSpace", params).await {
                Ok(res) => Some(ToolCallResult::json(&res)),
                Err(e) => Some(ToolCallResult::error(format!(
                    "Failed to query disk space: {e}"
                ))),
            }
        }
        "emule_nas_info" => match client.call("system.nasInfo", json!({})).await {
            Ok(res) => Some(ToolCallResult::json(&res)),
            Err(e) => Some(ToolCallResult::error(format!(
                "Failed to query NAS info: {e}"
            ))),
        },
        "emule_system_logs" => {
            let mut params = json!({});
            if let Some(l) = args.get("lines").and_then(|n| n.as_u64()) {
                params["lines"] = json!(l);
            }
            if let Some(f) = args.get("filter").and_then(|s| s.as_str()) {
                params["filter"] = json!(f);
            }
            match client.call("system.logs", params).await {
                Ok(res) => Some(ToolCallResult::json(&res)),
                Err(e) => Some(ToolCallResult::error(format!(
                    "Failed to read system logs: {e}"
                ))),
            }
        }
        "emule_media_list" => {
            let mut params = json!({});
            if let Some(mt) = args.get("media_type").and_then(|s| s.as_str()) {
                params["mediaType"] = json!(mt);
            }
            if let Some(st) = args.get("status").and_then(|s| s.as_str()) {
                params["status"] = json!(st);
            }
            match client.call("media.list", params).await {
                Ok(res) => Some(ToolCallResult::json(&res)),
                Err(e) => Some(ToolCallResult::error(format!(
                    "Failed to list media library: {e}"
                ))),
            }
        }
        "emule_search_all" => {
            let Some(term) = args.get("term").and_then(|s| s.as_str()) else {
                return Some(ToolCallResult::error(
                    "Missing required parameter: 'term'",
                ));
            };
            let mut params = json!({ "term": term });
            if let Some(min_s) = args.get("min_size").and_then(|n| n.as_u64()) {
                params["minSize"] = json!(min_s);
            }
            if let Some(max_s) = args.get("max_size").and_then(|n| n.as_u64()) {
                params["maxSize"] = json!(max_s);
            }
            if let Some(ext) = args.get("extension").and_then(|s| s.as_str()) {
                params["extension"] = json!(ext);
            }
            match client.call("search.all", params).await {
                Ok(res) => Some(ToolCallResult::json(&res)),
                Err(e) => Some(ToolCallResult::error(format!(
                    "Failed to execute unified search: {e}"
                ))),
            }
        }
        _ => None,
    }
}

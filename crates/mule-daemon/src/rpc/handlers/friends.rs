// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde_json::{json, Value};

use crate::error::JsonRpcError;
use crate::state::AppState;

/// `friends.list` — lista los amigos configurados (`friends.met` de eMule).
pub async fn handle_friends_list(state: &AppState) -> Result<Value, JsonRpcError> {
    let friends = state.downloads.lock().await.list_friends()?;
    let list: Vec<Value> = friends
        .iter()
        .map(|f| {
            json!({
                "userHash": f.user_hash,
                "name": f.name,
                "addedAtUnixMs": f.added_at_unix_ms,
            })
        })
        .collect();
    Ok(json!({ "friends": list, "total": list.len() }))
}

/// `friends.add` — concede al hash de usuario una ranura de subida dedicada.
/// Parámetros: `userHash` (hexadecimal, obligatorio), `name` (opcional).
pub async fn handle_friends_add(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let user_hash = params
        .as_ref()
        .and_then(|p| p["userHash"].as_str())
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'userHash'"))?
        .to_lowercase();
    let name = params
        .as_ref()
        .and_then(|p| p["name"].as_str())
        .unwrap_or("")
        .to_string();
    state.downloads.lock().await.add_friend(&user_hash, &name)?;
    state
        .inbound_listener
        .upload_manager()
        .set_friend(&format!("uh:{user_hash}"), true);
    Ok(json!({ "ok": true, "userHash": user_hash }))
}

/// `friends.remove` — revoca la ranura de amigo.
/// Parámetros: `userHash` (hexadecimal, obligatorio).
pub async fn handle_friends_remove(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let user_hash = params
        .as_ref()
        .and_then(|p| p["userHash"].as_str())
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'userHash'"))?
        .to_lowercase();
    state.downloads.lock().await.remove_friend(&user_hash)?;
    state
        .inbound_listener
        .upload_manager()
        .set_friend(&format!("uh:{user_hash}"), false);
    Ok(json!({ "ok": true, "userHash": user_hash }))
}

/// `friends.importEmfriendsMet` — `CClientList::LoadFriendsList`:
/// importa un archivo `emfriends.met`. Los registros sin hash de usuario se
/// notifican pero se omiten (nuestra ranura de amigo está indexada por hash, como la
/// búsqueda `IsFriend` de eMule).
/// Parámetros: `metPath` (obligatorio).
pub async fn handle_friends_import_emfriends_met(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let met_path = params
        .as_ref()
        .and_then(|p| p["metPath"].as_str())
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'metPath'"))?;
    let report = state
        .downloads
        .lock()
        .await
        .import_emfriends_met(std::path::Path::new(met_path))?;
    let upload_manager = state.inbound_listener.upload_manager();
    for user_hash in &report.user_hashes {
        upload_manager.set_friend(&format!("uh:{user_hash}"), true);
    }
    Ok(crate::rpc::handlers::json_value(&report))
}

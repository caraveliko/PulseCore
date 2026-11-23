// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde_json::{json, Value};

use crate::error::JsonRpcError;
use crate::state::AppState;

pub async fn handle_settings_list_dirs(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let requested = params
        .as_ref()
        .and_then(|p| p.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from);

    let fallback = state.config.default_download_dir.clone();

    tokio::task::spawn_blocking(move || {
        let current = resolve_existing_directory(requested, &fallback);
        let parent_path = current
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().into_owned());

        let mut directories = Vec::new();
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&current) {
            for entry in entries.flatten() {
                let Ok(ft) = entry.file_type() else {
                    continue;
                };
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = entry.path().to_string_lossy().into_owned();
                if ft.is_dir() {
                    directories.push((name, path));
                } else if ft.is_file() {
                    let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    files.push((name, path, size_bytes));
                }
            }
        }
        directories.sort_by_key(|a| a.0.to_lowercase());
        files.sort_by_key(|a| a.0.to_lowercase());

        let dirs_json: Vec<Value> = directories
            .into_iter()
            .map(|(name, path)| json!({ "name": name, "path": path }))
            .collect();
        let files_json: Vec<Value> = files
            .into_iter()
            .map(|(name, path, size_bytes)| {
                json!({ "name": name, "path": path, "sizeBytes": size_bytes })
            })
            .collect();

        Ok(json!({
            "currentPath": current.to_string_lossy(),
            "parentPath": parent_path,
            "directories": dirs_json,
            "files": files_json,
            "roots": list_available_roots(),
        }))
    })
    .await
    .map_err(|e| JsonRpcError::internal(format!("List dirs task error: {e}")))?
}

fn resolve_existing_directory(
    requested: Option<std::path::PathBuf>,
    fallback: &std::path::Path,
) -> std::path::PathBuf {
    let mut candidate = requested.unwrap_or_else(|| fallback.to_path_buf());
    loop {
        if candidate.is_dir() {
            return candidate;
        }
        match candidate.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => {
                candidate = parent.to_path_buf();
            }
            _ => break,
        }
    }
    if fallback.is_dir() {
        return fallback.to_path_buf();
    }
    #[cfg(windows)]
    {
        std::path::PathBuf::from("C:\\")
    }
    #[cfg(not(windows))]
    {
        std::path::PathBuf::from("/")
    }
}

fn list_available_roots() -> Vec<String> {
    #[cfg(windows)]
    {
        (b'A'..=b'Z')
            .map(|drive| format!("{}:\\", drive as char))
            .filter(|root| std::path::Path::new(root).is_dir())
            .collect()
    }
    #[cfg(not(windows))]
    {
        ["/", "/home", "/mnt", "/media"]
            .into_iter()
            .filter(|root| std::path::Path::new(root).is_dir())
            .map(str::to_string)
            .collect()
    }
}

/// Abre el selector nativo de carpetas en Windows; en otros sistemas indica `supported: false`.
pub async fn handle_settings_pick_folder(params: Option<Value>) -> Result<Value, JsonRpcError> {
    #[cfg(windows)]
    {
        let initial_path = params
            .as_ref()
            .and_then(|p| p.get("initialPath"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        tokio::task::spawn_blocking(move || pick_folder_windows(initial_path))
            .await
            .map_err(|e| JsonRpcError::internal(format!("Pick folder task error: {e}")))?
    }
    #[cfg(not(windows))]
    {
        let _ = params;
        Ok(json!({ "supported": false }))
    }
}

#[cfg(windows)]
fn pick_folder_windows(initial_path: Option<String>) -> Result<Value, JsonRpcError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let script = concat!(
        "Add-Type -AssemblyName System.Windows.Forms;",
        "$dlg = New-Object System.Windows.Forms.FolderBrowserDialog;",
        "$dlg.Description = 'Seleccionar carpeta';",
        "$dlg.ShowNewFolderButton = $true;",
        "if ($env:MULE_PICK_INITIAL_PATH -and (Test-Path -LiteralPath $env:MULE_PICK_INITIAL_PATH)) { $dlg.SelectedPath = $env:MULE_PICK_INITIAL_PATH };",
        "$top = New-Object System.Windows.Forms.Form;",
        "$top.TopMost = $true;",
        "if ($dlg.ShowDialog($top) -eq [System.Windows.Forms.DialogResult]::OK) {",
        " [Console]::OutputEncoding = [System.Text.Encoding]::UTF8;",
        " Write-Output $dlg.SelectedPath",
        "}"
    );

    let mut cmd = std::process::Command::new("powershell");
    cmd.creation_flags(CREATE_NO_WINDOW)
        .args(["-NoProfile", "-STA", "-Command", script]);
    if let Some(ref initial) = initial_path {
        cmd.env("MULE_PICK_INITIAL_PATH", initial);
    }

    let output = cmd
        .output()
        .map_err(|e| JsonRpcError::internal(format!("No se pudo abrir el selector de carpetas: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(JsonRpcError::internal(format!(
            "El selector de carpetas falló: {}",
            stderr.trim()
        )));
    }

    let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if selected.is_empty() {
        Ok(json!({
            "supported": true,
            "cancelled": true,
            "path": Value::Null,
        }))
    } else {
        Ok(json!({
            "supported": true,
            "cancelled": false,
            "path": selected,
        }))
    }
}

/// Resuelve la carpeta de destino de una descarga, película o serie y opcionalmente la abre en el entorno local.
pub async fn handle_settings_open_folder(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let raw_path = params
        .as_ref()
        .and_then(|p| p.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from);
    let media_id = params
        .as_ref()
        .and_then(|p| p.get("mediaId"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let download_id = params
        .as_ref()
        .and_then(|p| p.get("downloadId"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let open_native = params
        .as_ref()
        .and_then(|p| p.get("openNative"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut target_path = raw_path;
    let mut fallback_dir = state.effective_download_dir().await;

    if target_path.is_none() {
        if let Some(ref mid) = media_id {
            let (item_opt, eps) = {
                let dl_svc = state.downloads.lock().await;
                let item = dl_svc.db().get_media_item(mid).ok().flatten();
                let eps = dl_svc.db().get_episodes_for_media(mid).unwrap_or_default();
                (item, eps)
            };
            if let Some(item) = item_opt {
                if item.media_type == "tv" {
                    fallback_dir = state.effective_series_dir().await;
                } else {
                    fallback_dir = state.effective_movies_dir().await;
                }
                if let Some(fp) = item.file_path {
                    target_path = Some(std::path::PathBuf::from(fp));
                } else if let Some(ep_fp) = eps.into_iter().find_map(|e| e.file_path) {
                    target_path = Some(std::path::PathBuf::from(ep_fp));
                } else if !item.root_folder.trim().is_empty() {
                    let root_p = std::path::PathBuf::from(&item.root_folder);
                    if root_p.is_dir() {
                        fallback_dir = root_p;
                    }
                }
            }
        } else if let Some(ref did) = download_id {
            let dl_svc = state.downloads.lock().await;
            if let Ok(uuid) = uuid::Uuid::parse_str(did) {
                if let Some(detail) = dl_svc.get_download_detail(&uuid) {
                    target_path = Some(std::path::PathBuf::from(detail.target_path));
                }
            }
        }
    }

    tokio::task::spawn_blocking(move || {
        let mut highlighted_file: Option<String> = None;
        let candidate = target_path.unwrap_or_else(|| fallback_dir.clone());
        let exists_exact = candidate.exists();

        let dir_to_inspect = if candidate.is_file() {
            highlighted_file = candidate
                .file_name()
                .map(|n| n.to_string_lossy().into_owned());
            candidate
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| fallback_dir.clone())
        } else if candidate.is_dir() {
            candidate.clone()
        } else {
            // Si el archivo aún no existe en esa ruta, intentar inspeccionar su carpeta padre
            highlighted_file = candidate
                .file_name()
                .map(|n| n.to_string_lossy().into_owned());
            resolve_existing_directory(candidate.parent().map(std::path::Path::to_path_buf), &fallback_dir)
        };

        let resolved_dir = resolve_existing_directory(Some(dir_to_inspect), &fallback_dir);

        if open_native {
            try_open_folder_native(&resolved_dir);
        }

        let parent_path = resolved_dir
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().into_owned());

        let mut directories = Vec::new();
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&resolved_dir) {
            for entry in entries.flatten() {
                let Ok(ft) = entry.file_type() else {
                    continue;
                };
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = entry.path().to_string_lossy().into_owned();
                if ft.is_dir() {
                    directories.push((name, path));
                } else if ft.is_file() {
                    let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    files.push((name, path, size_bytes));
                }
            }
        }
        directories.sort_by_key(|a| a.0.to_lowercase());
        files.sort_by_key(|a| a.0.to_lowercase());

        Ok(json!({
            "resolvedDir": resolved_dir.to_string_lossy(),
            "parentPath": parent_path,
            "highlightedFile": highlighted_file,
            "exists": exists_exact,
            "directories": directories.into_iter().map(|(name, path)| json!({ "name": name, "path": path })).collect::<Vec<_>>(),
            "files": files.into_iter().map(|(name, path, size_bytes)| json!({ "name": name, "path": path, "sizeBytes": size_bytes })).collect::<Vec<_>>(),
            "roots": list_available_roots(),
        }))
    })
    .await
    .map_err(|e| JsonRpcError::internal(format!("Open folder task error: {e}")))?
}

fn try_open_folder_native(dir: &std::path::Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("explorer")
            .creation_flags(CREATE_NO_WINDOW)
            .arg(dir)
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(dir).spawn();
    }
}


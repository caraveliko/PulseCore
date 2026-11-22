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
        if let Ok(entries) = std::fs::read_dir(&current) {
            for entry in entries.flatten() {
                let Ok(ft) = entry.file_type() else {
                    continue;
                };
                if !ft.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = entry.path().to_string_lossy().into_owned();
                directories.push((name, path));
            }
        }
        directories.sort_by_key(|a| a.0.to_lowercase());

        let dirs_json: Vec<Value> = directories
            .into_iter()
            .map(|(name, path)| json!({ "name": name, "path": path }))
            .collect();

        Ok(json!({
            "currentPath": current.to_string_lossy(),
            "parentPath": parent_path,
            "directories": dirs_json,
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


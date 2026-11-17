// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Mecanismo de sustitución atómica y gestión de archivos ejecutables en disco.

use std::fs;
use std::path::{Path, PathBuf};

use super::constants::{UPDATE_BACKUP_SUFFIX, UPDATE_STAGING_SUFFIX};
use super::error::UpdaterError;

/// Obtiene la ruta canónica del ejecutable del demonio en ejecución.
pub fn get_current_executable_path() -> Result<PathBuf, UpdaterError> {
    std::env::current_exe().map_err(|_| UpdaterError::ExecutablePathUnknown)
}

/// Genera la ruta del archivo temporal en fase de preparación (*staging*).
pub fn get_staging_executable_path(current_exe: &Path) -> PathBuf {
    let mut file_name = current_exe
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "mule-daemon".to_string());
    file_name.push_str(UPDATE_STAGING_SUFFIX);
    current_exe.with_file_name(file_name)
}

/// Genera la ruta de la copia de respaldo previa en Windows.
pub fn get_backup_executable_path(current_exe: &Path) -> PathBuf {
    let mut file_name = current_exe
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "mule-daemon".to_string());
    file_name.push_str(UPDATE_BACKUP_SUFFIX);
    current_exe.with_file_name(file_name)
}

/// Escribe el contenido binario descargado en el archivo de preparación (*staging*).
pub fn write_staging_binary(staging_path: &Path, data: &[u8]) -> Result<(), UpdaterError> {
    fs::write(staging_path, data)?;

    // En plataformas Unix aseguramos permisos de ejecución (0o755)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        let _ = fs::set_permissions(staging_path, perms);
    }

    Ok(())
}

/// Sustituye el ejecutable actual por el nuevo binario validado de forma atómica y segura.
///
/// - En Windows: Aplica la técnica de renombramiento *move-aside* (`.old`) para sortear
///   el bloqueo de archivos en ejecución de NTFS.
/// - En Unix: Ejecuta `fs::rename`, que es atómico sobre el inodo del sistema de archivos.
pub fn replace_current_executable(staging_path: &Path, target_exe: &Path) -> Result<(), UpdaterError> {
    if !staging_path.exists() {
        return Err(UpdaterError::NoUpdateReady);
    }

    #[cfg(windows)]
    {
        let backup_path = get_backup_executable_path(target_exe);
        if backup_path.exists() {
            let _ = fs::remove_file(&backup_path);
        }

        // Renombrado del ejecutable en curso hacia .old (Windows lo permite en el mismo volumen)
        fs::rename(target_exe, &backup_path)?;

        // Desplazamiento del nuevo binario hacia la ruta de destino original
        if let Err(e) = fs::rename(staging_path, target_exe) {
            // Intento de reversión en caso de error
            let _ = fs::rename(&backup_path, target_exe);
            return Err(UpdaterError::Io(e));
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        let _ = fs::set_permissions(staging_path, perms);
        fs::rename(staging_path, target_exe)?;
    }

    Ok(())
}

/// Elimina copias de seguridad residuales (`.old`) procedentes de actualizaciones anteriores.
pub fn cleanup_old_backup_if_present(target_exe: &Path) {
    let backup_path = get_backup_executable_path(target_exe);
    if backup_path.exists() {
        let _ = fs::remove_file(&backup_path);
    }
}

/// Relanza el binario actual como un nuevo proceso independiente desacoplado.
pub fn respawn_current_process() -> Result<(), UpdaterError> {
    let current_exe = get_current_executable_path()?;
    let args: Vec<String> = std::env::args().skip(1).collect();

    tracing::info!(
        exe = %current_exe.display(),
        args = ?args,
        "Relanzando proceso eMuleRust tras la sustitución de la actualización..."
    );

    let mut cmd = std::process::Command::new(&current_exe);
    cmd.args(&args);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        cmd.creation_flags(DETACHED_PROCESS);
    }

    cmd.spawn().map_err(UpdaterError::Io)?;
    Ok(())
}

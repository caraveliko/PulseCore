// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Mecanismo de sustitución atómica y gestión de archivos ejecutables en disco.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use super::constants::{
    MIN_EXECUTABLE_HEADER_BYTES, UPDATE_BACKUP_SUFFIX, UPDATE_STAGING_SUFFIX, WRITE_PROBE_SUFFIX,
};
use super::error::UpdaterError;
use super::verifier::validate_executable_for_triple;

/// Convierte un error de sistema de archivos en un error tipado más explicativo si falta permiso.
fn map_fs_error(err: std::io::Error, path: &Path) -> UpdaterError {
    match err.kind() {
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => {
            UpdaterError::PermissionDenied(format!(
                "No se puede escribir en {} ({err}). En Linux/systemd comprueba que ReadWritePaths incluya el directorio del binario.",
                path.display()
            ))
        }
        _ => UpdaterError::Io(err),
    }
}

/// Indica si el proceso actual se está ejecutando bajo supervisión de `systemd` en Linux.
pub fn is_managed_by_systemd() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("INVOCATION_ID").is_some() || std::env::var_os("NOTIFY_SOCKET").is_some()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Comprueba mediante una sonda efímera si el directorio del ejecutable permite escritura.
pub fn is_executable_dir_writable(target_exe: &Path) -> bool {
    let Some(parent) = target_exe.parent() else {
        return false;
    };
    if !parent.is_dir() {
        return false;
    }
    let probe_name = format!(".mule-daemon-{}{}", std::process::id(), WRITE_PROBE_SUFFIX);
    let probe_path = parent.join(probe_name);
    match OpenOptions::new().write(true).create_new(true).open(&probe_path) {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe_path);
            true
        }
        Err(_) => false,
    }
}

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
    fs::write(staging_path, data).map_err(|e| map_fs_error(e, staging_path))?;

    // En plataformas Unix aseguramos permisos de ejecución (0o755)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(staging_path, perms).map_err(|e| map_fs_error(e, staging_path))?;
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
        fs::rename(target_exe, &backup_path).map_err(|e| map_fs_error(e, &backup_path))?;

        // Desplazamiento del nuevo binario hacia la ruta de destino original
        if let Err(e) = fs::rename(staging_path, target_exe) {
            // Intento de reversión en caso de error
            let _ = fs::rename(&backup_path, target_exe);
            return Err(map_fs_error(e, target_exe));
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if target_exe.exists() {
            let backup_path = get_backup_executable_path(target_exe);
            if backup_path.exists() {
                let _ = fs::remove_file(&backup_path);
            }
            // Enlace duro O(1) para preservar el binario previo sin duplicar bloques en disco
            if let Err(err) = fs::hard_link(target_exe, &backup_path) {
                if matches!(
                    err.kind(),
                    ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem
                ) {
                    return Err(map_fs_error(err, &backup_path));
                }
                fs::copy(target_exe, &backup_path).map_err(|e| map_fs_error(e, &backup_path))?;
            }
            let backup_perms = fs::Permissions::from_mode(0o755);
            fs::set_permissions(&backup_path, backup_perms)
                .map_err(|e| map_fs_error(e, &backup_path))?;
        }

        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(staging_path, perms).map_err(|e| map_fs_error(e, staging_path))?;
        fs::rename(staging_path, target_exe).map_err(|e| map_fs_error(e, target_exe))?;
    }

    Ok(())
}

/// Comprueba si existe una copia de respaldo `.old` lista para revertir el binario.
pub fn has_rollback_backup(target_exe: &Path) -> bool {
    get_backup_executable_path(target_exe).is_file()
}

/// Elimina binarios temporales huérfanos en fase `.update`, conservando `.old` para rollback.
pub fn cleanup_stale_staging_if_present(target_exe: &Path) {
    let staging_path = get_staging_executable_path(target_exe);
    if staging_path.exists() {
        let _ = fs::remove_file(&staging_path);
    }
}

/// Elimina copias de seguridad residuales (`.old`) procedentes de actualizaciones anteriores.
pub fn cleanup_old_backup_if_present(target_exe: &Path) {
    let backup_path = get_backup_executable_path(target_exe);
    if backup_path.exists() {
        let _ = fs::remove_file(&backup_path);
    }
}

/// Revierte el ejecutable actual restaurando de forma atómica la copia `.old` previa.
pub fn rollback_to_backup(target_exe: &Path, expected_triple: &str) -> Result<PathBuf, UpdaterError> {
    let backup_path = get_backup_executable_path(target_exe);
    if !backup_path.is_file() {
        return Err(UpdaterError::NoRollbackAvailable);
    }

    // Valida la cabecera binaria antes de restaurar para evitar revertir hacia un archivo corrupto
    let mut file = fs::File::open(&backup_path).map_err(|e| map_fs_error(e, &backup_path))?;
    let mut header_buf = [0u8; MIN_EXECUTABLE_HEADER_BYTES];
    let mut filled = 0;
    while filled < header_buf.len() {
        let n = file
            .read(&mut header_buf[filled..])
            .map_err(|e| map_fs_error(e, &backup_path))?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    drop(file);
    validate_executable_for_triple(&header_buf[..filled], expected_triple)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&backup_path, perms).map_err(|e| map_fs_error(e, &backup_path))?;
        fs::rename(&backup_path, target_exe).map_err(|e| map_fs_error(e, target_exe))?;
    }

    #[cfg(windows)]
    {
        let staging_path = get_staging_executable_path(target_exe);
        if staging_path.exists() {
            let _ = fs::remove_file(&staging_path);
        }
        if target_exe.exists() {
            fs::rename(target_exe, &staging_path).map_err(|e| map_fs_error(e, &staging_path))?;
        }
        if let Err(e) = fs::rename(&backup_path, target_exe) {
            if staging_path.exists() {
                let _ = fs::rename(&staging_path, target_exe);
            }
            return Err(map_fs_error(e, target_exe));
        }
        let _ = fs::remove_file(&staging_path);
    }

    Ok(target_exe.to_path_buf())
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

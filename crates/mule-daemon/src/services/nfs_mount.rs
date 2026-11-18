// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::services::nas_transfer::FilesystemKind;

/// Puerto TCP estándar para el protocolo NFSv3/NFSv4.
pub const NFS_DEFAULT_PORT: u16 = 2049;

/// Tiempo máximo de espera (ms) para comprobar la accesibilidad TCP del servidor NFS.
pub const NFS_TCP_PROBE_TIMEOUT_MS: u64 = 1500;

/// Profundidad máxima de resolución de enlaces simbólicos al normalizar rutas locales.
const MAX_SYMLINK_DEPTH: usize = 16;

/// Nombres candidatos habituales para la carpeta de películas en un volumen NAS (orden de prioridad).
pub const MOVIES_FOLDER_CANDIDATES: &[&str] =
    &["Movies", "Películas", "Peliculas", "Films", "Cine"];

/// Nombres candidatos habituales para la carpeta de series en un volumen NAS (orden de prioridad).
pub const SERIES_FOLDER_CANDIDATES: &[&str] = &["Series", "TV Shows", "Series TV", "TV"];

/// Montaje de red descubierto en `/proc/mounts` o declarado en `/etc/fstab`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredNetworkMount {
    pub mount_point: String,
    pub remote_export: String,
    pub fs_type: String,
    pub server_host: Option<String>,
    pub mounted: bool,
}

/// Estado detallado del punto de montaje y subcarpetas resueltas para una ruta de destino NAS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NfsDirectoryStatus {
    pub target_path: String,
    pub fs_kind: String,
    pub is_network_mount: bool,
    pub mount_point: Option<String>,
    pub remote_export: Option<String>,
    pub server_host: Option<String>,
    pub unmounted_fstab_trap: bool,
    pub nfs_port_reachable: Option<bool>,
    pub resolved_movies_dir: String,
    pub resolved_series_dir: String,
}

#[derive(Debug, Clone)]
struct MountEntry {
    remote_export: String,
    mount_point: String,
    fs_type: String,
    fs_kind: FilesystemKind,
}

/// Decodifica los escapes octales habituales en `/proc/mounts` y `/etc/fstab` (p. ej. `\040` -> espacio).
fn unescape_mount_field(raw: &str) -> String {
    raw.replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

/// Clasifica un tipo de sistema de archivos Linux en [`FilesystemKind`].
pub fn classify_fs_type(fstype: &str) -> FilesystemKind {
    let lower = fstype.trim().to_ascii_lowercase();
    if matches!(lower.as_str(), "nfs" | "nfs4") {
        FilesystemKind::Nfs
    } else if matches!(lower.as_str(), "cifs" | "smbfs" | "smb3") {
        FilesystemKind::Smb
    } else if matches!(
        lower.as_str(),
        "sshfs" | "fuse.sshfs" | "glusterfs" | "ceph" | "fuse.rclone"
    ) {
        FilesystemKind::Remote
    } else {
        FilesystemKind::Local
    }
}

/// Extrae el nombre o IP del servidor remoto a partir del origen de montaje (`host:/ruta` o `//host/recurso`).
pub fn extract_server_host(remote_export: &str, fs_kind: FilesystemKind) -> Option<String> {
    let trimmed = remote_export.trim();
    if trimmed.is_empty() {
        return None;
    }

    match fs_kind {
        FilesystemKind::Nfs => {
            if let Some(rest) = trimmed.strip_prefix('[') {
                return rest
                    .split_once(']')
                    .map(|(h, _)| h.trim().to_string())
                    .filter(|h| !h.is_empty());
            }
            trimmed
                .split_once(':')
                .map(|(h, _)| h.trim().to_string())
                .filter(|h| !h.is_empty())
        }
        FilesystemKind::Smb => {
            let without_prefix = trimmed.trim_start_matches(['/', '\\']);
            let host = without_prefix
                .split(['/', '\\'])
                .next()
                .unwrap_or("")
                .trim();
            let host = host.rsplit('@').next().unwrap_or(host).trim();
            (!host.is_empty()).then(|| host.to_string())
        }
        FilesystemKind::Remote => {
            let before_colon = trimmed.split(':').next().unwrap_or(trimmed);
            let host = before_colon
                .rsplit('@')
                .next()
                .unwrap_or(before_colon)
                .trim();
            (!host.is_empty()).then(|| host.to_string())
        }
        FilesystemKind::Local => None,
    }
}

fn parse_mount_lines(raw: &str, skip_comments: bool) -> Vec<MountEntry> {
    let mut entries = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || (skip_comments && trimmed.starts_with('#')) {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        let remote_export = unescape_mount_field(parts[0]);
        let mount_point = unescape_mount_field(parts[1]);
        let fs_type = parts[2].to_string();
        let fs_kind = classify_fs_type(&fs_type);
        entries.push(MountEntry {
            remote_export,
            mount_point,
            fs_type,
            fs_kind,
        });
    }
    entries
}

/// Analiza el contenido de `/proc/mounts` y `/etc/fstab` devolviendo todos los montajes de red descubiertos.
pub fn parse_mounts_and_fstab(proc_mounts: &str, etc_fstab: &str) -> Vec<DiscoveredNetworkMount> {
    let active = parse_mount_lines(proc_mounts, false);
    let fstab = parse_mount_lines(etc_fstab, true);

    let mut discovered = Vec::new();
    let mut active_network_points: Vec<String> = Vec::new();

    for entry in active {
        if !entry.fs_kind.is_network() {
            continue;
        }
        if discovered
            .iter()
            .any(|d: &DiscoveredNetworkMount| d.mount_point == entry.mount_point)
        {
            continue;
        }
        active_network_points.push(entry.mount_point.clone());
        let server_host = extract_server_host(&entry.remote_export, entry.fs_kind);
        discovered.push(DiscoveredNetworkMount {
            mount_point: entry.mount_point,
            remote_export: entry.remote_export,
            fs_type: entry.fs_type,
            server_host,
            mounted: true,
        });
    }

    for entry in fstab {
        if !entry.fs_kind.is_network() {
            continue;
        }
        if active_network_points
            .iter()
            .any(|p| p == &entry.mount_point)
            || discovered
                .iter()
                .any(|d| d.mount_point == entry.mount_point)
        {
            continue;
        }
        let server_host = extract_server_host(&entry.remote_export, entry.fs_kind);
        discovered.push(DiscoveredNetworkMount {
            mount_point: entry.mount_point,
            remote_export: entry.remote_export,
            fs_type: entry.fs_type,
            server_host,
            mounted: false,
        });
    }

    discovered
}

/// Descubre todos los montajes de red (`nfs`, `nfs4`, `cifs`, etc.) configurados o montados en el sistema.
#[cfg(target_os = "linux")]
pub fn discover_system_network_mounts() -> Vec<DiscoveredNetworkMount> {
    let proc_mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let etc_fstab = std::fs::read_to_string("/etc/fstab").unwrap_or_default();
    parse_mounts_and_fstab(&proc_mounts, &etc_fstab)
}

/// Descubre montajes de red del sistema (sin `/proc/mounts` fuera de Linux).
#[cfg(not(target_os = "linux"))]
pub fn discover_system_network_mounts() -> Vec<DiscoveredNetworkMount> {
    Vec::new()
}

/// Resuelve la subcarpeta de biblioteca (`Movies` o `Series`) dentro de `nas_root` buscando nombres existentes
/// sin distinguir mayúsculas ni minúsculas; si no existe ninguno, devuelve `nas_root.join(canonical_default)`.
pub fn resolve_media_subfolder(
    nas_root: &Path,
    candidates: &[&str],
    canonical_default: &str,
) -> PathBuf {
    if let Ok(entries) = std::fs::read_dir(nas_root) {
        let mut dir_names: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            let is_dir = match entry.file_type() {
                Ok(ft) if ft.is_dir() => true,
                Ok(ft) if ft.is_symlink() => entry.path().is_dir(),
                Ok(ft) if ft.is_file() => false,
                _ => entry.path().is_dir(),
            };
            if is_dir {
                dir_names.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        for candidate in candidates {
            if let Some(found) = dir_names
                .iter()
                .find(|name| name.eq_ignore_ascii_case(candidate))
            {
                return nas_root.join(found);
            }
        }
    }
    nas_root.join(canonical_default)
}

/// Normaliza léxicamente `.` y `..` sin realizar llamadas de sistema al VFS.
fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => normalized.push(p.as_os_str()),
            Component::RootDir => normalized.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let can_pop = matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                );
                if can_pop {
                    normalized.pop();
                }
            }
            Component::Normal(s) => normalized.push(s),
        }
    }
    normalized
}

/// Resuelve enlaces simbólicos en los componentes locales anteriores a cualquier punto de montaje de red,
/// evitando tocar rutas NFS si el montaje pudiera estar colgado.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn resolve_path_safely(path: &Path, known_network_points: &[&str]) -> PathBuf {
    let mut current = normalize_lexical_path(path);
    for _ in 0..MAX_SYMLINK_DEPTH {
        let mut rebuilt = PathBuf::new();
        let mut restarted = false;
        let comps: Vec<Component<'_>> = current.components().collect();
        for (idx, comp) in comps.iter().enumerate() {
            rebuilt.push(comp.as_os_str());
            if known_network_points
                .iter()
                .any(|mnt| rebuilt == Path::new(mnt))
            {
                for rem in &comps[idx + 1..] {
                    rebuilt.push(rem.as_os_str());
                }
                return rebuilt;
            }
            if matches!(comp, Component::Normal(_)) {
                if let Ok(meta) = std::fs::symlink_metadata(&rebuilt) {
                    if meta.file_type().is_symlink() {
                        if let Ok(link_target) = std::fs::read_link(&rebuilt) {
                            let base = rebuilt.parent().unwrap_or_else(|| Path::new("/"));
                            let mut next = if link_target.is_absolute() {
                                link_target
                            } else {
                                base.join(link_target)
                            };
                            for rem in &comps[idx + 1..] {
                                next.push(rem.as_os_str());
                            }
                            current = normalize_lexical_path(&next);
                            restarted = true;
                            break;
                        }
                    }
                }
            }
        }
        if !restarted {
            return rebuilt;
        }
    }
    current
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn find_best_mount_match<'a>(target: &Path, entries: &'a [MountEntry]) -> Option<&'a MountEntry> {
    let mut best: Option<(usize, &'a MountEntry)> = None;
    for entry in entries {
        let mnt_path = Path::new(&entry.mount_point);
        if target.starts_with(mnt_path) {
            let len = entry.mount_point.len();
            match best {
                None => best = Some((len, entry)),
                Some((best_len, _)) if len > best_len => {
                    best = Some((len, entry));
                }
                Some((best_len, best_entry))
                    if len == best_len
                        && best_entry.fs_type.eq_ignore_ascii_case("autofs")
                        && !entry.fs_type.eq_ignore_ascii_case("autofs") =>
                {
                    best = Some((len, entry));
                }
                _ => {}
            }
        }
    }
    best.map(|(_, entry)| entry)
}

/// Comprueba mediante una conexión TCP rápida si el puerto `2049` de un servidor NFS responde.
pub fn probe_nfs_tcp_port(host: &str, timeout_ms: u64) -> bool {
    probe_tcp_port(host, NFS_DEFAULT_PORT, timeout_ms)
}

fn probe_tcp_port(host: &str, port: u16, timeout_ms: u64) -> bool {
    let timeout = Duration::from_millis(timeout_ms.max(50));
    let Ok(addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    let addrs: Vec<SocketAddr> = addrs.collect();
    for addr in addrs {
        if TcpStream::connect_timeout(&addr, timeout).is_ok() {
            return true;
        }
    }
    false
}

/// Detecta rápidamente el [`FilesystemKind`] de una ruta en Linux usando el analizador unificado de `/proc/mounts`.
#[cfg(target_os = "linux")]
pub fn detect_filesystem_kind_linux(path: &Path) -> FilesystemKind {
    let proc_mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let entries = parse_mount_lines(&proc_mounts, false);
    let net_points: Vec<&str> = entries
        .iter()
        .filter(|e| e.fs_kind.is_network())
        .map(|e| e.mount_point.as_str())
        .collect();
    let resolved = resolve_path_safely(path, &net_points);
    find_best_mount_match(&resolved, &entries)
        .map(|e| e.fs_kind)
        .unwrap_or(FilesystemKind::Local)
}

/// Inspecciona el estado de montaje de `target_path` utilizando textos explícitos de `/proc/mounts` y `/etc/fstab`.
pub fn inspect_directory_status_with_sources(
    target_path: &Path,
    proc_mounts: &str,
    etc_fstab: &str,
    check_tcp: bool,
) -> NfsDirectoryStatus {
    let active_entries = parse_mount_lines(proc_mounts, false);
    let fstab_entries: Vec<MountEntry> = parse_mount_lines(etc_fstab, true)
        .into_iter()
        .filter(|e| e.fs_kind.is_network())
        .collect();

    let mut known_net_points: Vec<&str> = active_entries
        .iter()
        .filter(|e| e.fs_kind.is_network())
        .map(|e| e.mount_point.as_str())
        .collect();
    for f in &fstab_entries {
        if !known_net_points.contains(&f.mount_point.as_str()) {
            known_net_points.push(f.mount_point.as_str());
        }
    }

    let resolved_target = resolve_path_safely(target_path, &known_net_points);
    let active_match = find_best_mount_match(&resolved_target, &active_entries);
    let fstab_match = find_best_mount_match(&resolved_target, &fstab_entries);

    let active_is_network = active_match.is_some_and(|e| e.fs_kind.is_network());
    let unmounted_fstab_trap = match (active_match, fstab_match) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(act), Some(fst)) => {
            !act.fs_kind.is_network() || act.mount_point.len() < fst.mount_point.len()
        }
    };

    let (fs_kind, is_network_mount, mount_point, remote_export, server_host, probe_kind) =
        if unmounted_fstab_trap {
            let fst = fstab_match.expect("fstab_match garantizado por unmounted_fstab_trap");
            let host = extract_server_host(&fst.remote_export, fst.fs_kind);
            (
                active_match
                    .map(|a| a.fs_kind.as_str())
                    .unwrap_or("local")
                    .to_string(),
                false,
                Some(fst.mount_point.clone()),
                Some(fst.remote_export.clone()),
                host,
                fst.fs_kind,
            )
        } else if let Some(act) = active_match {
            let host = extract_server_host(&act.remote_export, act.fs_kind);
            (
                act.fs_kind.as_str().to_string(),
                active_is_network,
                Some(act.mount_point.clone()),
                Some(act.remote_export.clone()),
                host,
                act.fs_kind,
            )
        } else {
            (
                "local".to_string(),
                false,
                None,
                None,
                None,
                FilesystemKind::Local,
            )
        };

    let nfs_port_reachable = if check_tcp && probe_kind == FilesystemKind::Nfs {
        server_host
            .as_deref()
            .map(|h| probe_nfs_tcp_port(h, NFS_TCP_PROBE_TIMEOUT_MS))
    } else {
        None
    };

    let safe_to_scan_subdirs = !unmounted_fstab_trap && nfs_port_reachable != Some(false);
    let (resolved_movies_dir, resolved_series_dir) = if safe_to_scan_subdirs {
        (
            resolve_media_subfolder(target_path, MOVIES_FOLDER_CANDIDATES, "Movies")
                .to_string_lossy()
                .into_owned(),
            resolve_media_subfolder(target_path, SERIES_FOLDER_CANDIDATES, "Series")
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        (
            target_path.join("Movies").to_string_lossy().into_owned(),
            target_path.join("Series").to_string_lossy().into_owned(),
        )
    };

    NfsDirectoryStatus {
        target_path: target_path.to_string_lossy().into_owned(),
        fs_kind,
        is_network_mount,
        mount_point,
        remote_export,
        server_host,
        unmounted_fstab_trap,
        nfs_port_reachable,
        resolved_movies_dir,
        resolved_series_dir,
    }
}

/// Inspecciona el estado de montaje de `target_path` en el sistema actual.
#[cfg(target_os = "linux")]
pub fn inspect_directory_status(target_path: &Path, check_tcp: bool) -> NfsDirectoryStatus {
    let proc_mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let etc_fstab = std::fs::read_to_string("/etc/fstab").unwrap_or_default();
    inspect_directory_status_with_sources(target_path, &proc_mounts, &etc_fstab, check_tcp)
}

/// Inspecciona el estado de montaje de `target_path` fuera de Linux (UNC/SMB o almacenamiento local).
#[cfg(not(target_os = "linux"))]
pub fn inspect_directory_status(target_path: &Path, _check_tcp: bool) -> NfsDirectoryStatus {
    let fs_kind = crate::services::nas_transfer::detect_filesystem_kind(target_path);
    let target_str = target_path.to_string_lossy().into_owned();
    let server_host = extract_server_host(&target_str, fs_kind);
    let movies = resolve_media_subfolder(target_path, MOVIES_FOLDER_CANDIDATES, "Movies");
    let series = resolve_media_subfolder(target_path, SERIES_FOLDER_CANDIDATES, "Series");

    NfsDirectoryStatus {
        target_path: target_str,
        fs_kind: fs_kind.as_str().to_string(),
        is_network_mount: fs_kind.is_network(),
        mount_point: None,
        remote_export: None,
        server_host,
        unmounted_fstab_trap: false,
        nfs_port_reachable: None,
        resolved_movies_dir: movies.to_string_lossy().into_owned(),
        resolved_series_dir: series.to_string_lossy().into_owned(),
    }
}

/// Verifica que la ruta destino esté lista y montada antes de crear directorios o escribir archivos.
///
/// Aborta inmediatamente con un error de E/S si:
/// - La ruta cae bajo un punto de montaje de red declarado en `/etc/fstab` que no está montado (`unmounted_fstab_trap`).
/// - `require_network_mount` está activo y el destino reside en disco local.
/// - El destino es un montaje NFS cuyo servidor no responde en el puerto TCP `2049`.
pub fn verify_destination_mount_ready(
    target_path: &Path,
    require_network_mount: bool,
) -> std::io::Result<()> {
    let status = inspect_directory_status(target_path, true);
    validate_mount_status(target_path, &status, require_network_mount)
}

fn validate_mount_status(
    target_path: &Path,
    status: &NfsDirectoryStatus,
    require_network_mount: bool,
) -> std::io::Result<()> {
    if status.unmounted_fstab_trap {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            format!(
                "El punto de montaje NFS/red para '{}' está declarado en /etc/fstab pero no está montado; escritura abortada para proteger el disco local",
                target_path.display()
            ),
        ));
    }

    if require_network_mount && !status.is_network_mount {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            format!(
                "La ruta destino '{}' reside en el disco local y la protección 'Exigir montaje NFS/red' está activa",
                target_path.display()
            ),
        ));
    }

    if status.fs_kind == "nfs" && status.nfs_port_reachable == Some(false) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!(
                "El servidor NFS '{}' no responde en el puerto TCP {}",
                status.server_host.as_deref().unwrap_or(""),
                NFS_DEFAULT_PORT
            ),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use tempfile::tempdir;

    #[test]
    fn test_parse_mounts_and_fstab_detects_mounted_and_unmounted() {
        let proc_mounts = "\
/dev/mmcblk0p2 / ext4 rw,noatime 0 0
192.168.1.50:/volume1/video /mnt/nas nfs4 rw,relatime,vers=4.1 0 0
";
        let etc_fstab = "\
# comentario de prueba
UUID=1234 / ext4 defaults 0 1
192.168.1.50:/volume1/video /mnt/nas nfs4 defaults 0 0
//192.168.1.60/backup /mnt/smb cifs credentials=/root/.smb,uid=1000 0 0
";

        let list = parse_mounts_and_fstab(proc_mounts, etc_fstab);
        assert_eq!(list.len(), 2);

        assert_eq!(list[0].mount_point, "/mnt/nas");
        assert_eq!(list[0].remote_export, "192.168.1.50:/volume1/video");
        assert_eq!(list[0].fs_type, "nfs4");
        assert_eq!(list[0].server_host.as_deref(), Some("192.168.1.50"));
        assert!(list[0].mounted);

        assert_eq!(list[1].mount_point, "/mnt/smb");
        assert_eq!(list[1].remote_export, "//192.168.1.60/backup");
        assert_eq!(list[1].fs_type, "cifs");
        assert_eq!(list[1].server_host.as_deref(), Some("192.168.1.60"));
        assert!(!list[1].mounted);
    }

    #[test]
    fn test_inspect_detects_unmounted_fstab_trap_and_rejects_write() {
        let proc_mounts = "/dev/mmcblk0p2 / ext4 rw,noatime 0 0\n";
        let etc_fstab = "192.168.1.50:/volume1/media /mnt/nas nfs defaults 0 0\n";

        let target = Path::new("/mnt/nas/Movies/Matrix (1999)/Matrix.mkv");
        let status = inspect_directory_status_with_sources(target, proc_mounts, etc_fstab, false);

        assert!(status.unmounted_fstab_trap);
        assert!(!status.is_network_mount);
        assert_eq!(status.mount_point.as_deref(), Some("/mnt/nas"));
        assert_eq!(status.server_host.as_deref(), Some("192.168.1.50"));

        let err = validate_mount_status(target, &status, false).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotConnected);
    }

    #[test]
    fn test_inspect_allows_mounted_nfs_and_respects_prefix_boundary() {
        let proc_mounts = "\
/dev/mmcblk0p2 / ext4 rw,noatime 0 0
192.168.1.50:/volume1/media /mnt/nas nfs4 rw,relatime 0 0
";
        let etc_fstab = "192.168.1.50:/volume1/media /mnt/nas nfs4 defaults 0 0\n";

        let nas_target = Path::new("/mnt/nas/Series/Silo");
        let nas_status =
            inspect_directory_status_with_sources(nas_target, proc_mounts, etc_fstab, false);
        assert!(nas_status.is_network_mount);
        assert_eq!(nas_status.fs_kind, "nfs");
        assert!(!nas_status.unmounted_fstab_trap);
        assert!(validate_mount_status(nas_target, &nas_status, true).is_ok());

        // `/mnt/nas_local` no debe confundirse con `/mnt/nas`
        let local_target = Path::new("/mnt/nas_local/Pelicula.mkv");
        let local_status =
            inspect_directory_status_with_sources(local_target, proc_mounts, etc_fstab, false);
        assert!(!local_status.is_network_mount);
        assert_eq!(local_status.fs_kind, "local");
        assert!(!local_status.unmounted_fstab_trap);
        let err = validate_mount_status(local_target, &local_status, true).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotConnected);
        assert!(validate_mount_status(local_target, &local_status, false).is_ok());
    }

    #[test]
    fn test_resolve_media_subfolder_detects_localized_names() {
        let root = tempdir().unwrap();
        std::fs::create_dir(root.path().join("Películas")).unwrap();
        std::fs::create_dir(root.path().join("TV Shows")).unwrap();

        let movies = resolve_media_subfolder(root.path(), MOVIES_FOLDER_CANDIDATES, "Movies");
        let series = resolve_media_subfolder(root.path(), SERIES_FOLDER_CANDIDATES, "Series");
        assert_eq!(movies, root.path().join("Películas"));
        assert_eq!(series, root.path().join("TV Shows"));

        let empty_root = tempdir().unwrap();
        assert_eq!(
            resolve_media_subfolder(empty_root.path(), MOVIES_FOLDER_CANDIDATES, "Movies"),
            empty_root.path().join("Movies")
        );
        assert_eq!(
            resolve_media_subfolder(empty_root.path(), SERIES_FOLDER_CANDIDATES, "Series"),
            empty_root.path().join("Series")
        );
    }

    #[test]
    fn test_probe_tcp_port_loopback() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(probe_tcp_port("127.0.0.1", port, 300));
        drop(listener);
        assert!(!probe_tcp_port("127.0.0.1", port, 150));
    }
}

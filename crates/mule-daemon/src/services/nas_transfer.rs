// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::path::Path;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Taille des tampons de transfert pour montages réseau NFS (128 Kio = 131 072 octets),
/// alignée exactement sur la valeur wsize négociée par NFSv3/NFSv4.
pub const NFS_CHUNK_SIZE: usize = 128 * 1024;

/// Taille des tampons pour copies locales entre disques physiques distincts (1 Mio).
pub const LOCAL_CHUNK_SIZE: usize = 1024 * 1024;

/// Débit cible par défaut vers le NAS embarqué (Synology DS212 à 256 Mo RAM) en Mo/s.
/// Évite le buffer bloat, la saturation mémoire du NAS et les retransmissions TCP.
pub const DEFAULT_NAS_TRANSFER_RATE_MBPS: u32 = 20;

/// Type de système de fichiers du point de montage cible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemKind {
    /// Disque bloc local (NVMe, SSD SATA, HDD, ext4, NTFS, XFS, tmpfs).
    Local,
    /// Montage réseau NFS (Network File System).
    Nfs,
    /// Montage réseau Windows / Samba (SMB / CIFS).
    Smb,
    /// Autre montage réseau ou distant (SSHFS, Gluster, Ceph).
    Remote,
}

impl FilesystemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Nfs => "nfs",
            Self::Smb => "smb",
            Self::Remote => "remote",
        }
    }

    pub fn is_network(&self) -> bool {
        matches!(self, Self::Nfs | Self::Smb | Self::Remote)
    }
}

/// Détecte le type de système de fichiers du chemin cible.
/// Sur Linux, résout le point de montage le plus spécifique dans `/proc/mounts`.
#[cfg(target_os = "linux")]
pub fn detect_filesystem_kind(path: &Path) -> FilesystemKind {
    let mut current = path.to_path_buf();
    while !current.exists() {
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            break;
        }
    }
    let target_canonical = current.canonicalize().unwrap_or(current);
    let target_str = target_canonical.to_string_lossy();

    if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
        let mut best_match: Option<(usize, &str)> = None; // (prefix_len, fstype)
        for line in mounts.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                let mnt_pt = parts[1];
                let fstype = parts[2];
                if target_str.starts_with(mnt_pt) {
                    let len = mnt_pt.len();
                    if let Some((best_len, _)) = best_match {
                        if len > best_len {
                            best_match = Some((len, fstype));
                        }
                    } else {
                        best_match = Some((len, fstype));
                    }
                }
            }
        }

        if let Some((_, fstype)) = best_match {
            if fstype == "nfs" || fstype == "nfs4" {
                return FilesystemKind::Nfs;
            } else if fstype == "cifs" || fstype == "smbfs" {
                return FilesystemKind::Smb;
            } else if matches!(fstype, "sshfs" | "fuse.sshfs" | "glusterfs" | "ceph") {
                return FilesystemKind::Remote;
            }
        }
    }

    FilesystemKind::Local
}

#[cfg(not(target_os = "linux"))]
pub fn detect_filesystem_kind(path: &Path) -> FilesystemKind {
    let s = path.to_string_lossy();
    if s.starts_with("\\\\") || s.starts_with("//") {
        FilesystemKind::Smb
    } else {
        FilesystemKind::Local
    }
}

/// Instantané d'avancement d'un transfert réseau ou local vers la destination finale.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NasTransferProgress {
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub speed_bytes_per_sec: u64,
    pub percent: f32,
    pub fs_kind: String,
}

#[cfg(target_os = "linux")]
mod fadvise {
    use std::os::unix::io::RawFd;
    pub const POSIX_FADV_SEQUENTIAL: i32 = 2;
    pub const POSIX_FADV_DONTNEED: i32 = 4;

    extern "C" {
        pub fn posix_fadvise(fd: RawFd, offset: i64, len: i64, advice: i32) -> i32;
    }
}

/// Transfère un fichier vers sa destination finale avec calibrage protocolaire :
/// - Blocs de 128 Kio calés sur `wsize` pour NFS (évite la fragmentation RPC).
/// - Régulation de débit (*pacing*) pour ménager la mémoire vive du NAS (ex. Synology DS212 256 Mo).
/// - Préchauffage et éviction séquentielle de cache (`posix_fadvise`).
/// - Appel `fdatasync` final unique pour garantir la persistance physique avant validation.
pub fn transfer_to_destination<F>(
    src: &Path,
    tgt: &Path,
    target_rate_mbps: u32,
    mut progress_cb: Option<F>,
) -> std::io::Result<u64>
where
    F: FnMut(&NasTransferProgress),
{
    let fs_kind = detect_filesystem_kind(tgt);
    let total_bytes = std::fs::metadata(src)?.len();

    let mut src_file = std::fs::File::open(src)?;
    if let Some(parent) = tgt.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tgt_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(tgt)?;

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        unsafe {
            let _ = fadvise::posix_fadvise(
                src_file.as_raw_fd(),
                0,
                0,
                fadvise::POSIX_FADV_SEQUENTIAL,
            );
        }
    }

    let chunk_size = if fs_kind.is_network() {
        NFS_CHUNK_SIZE
    } else {
        LOCAL_CHUNK_SIZE
    };

    let target_rate_bps = if fs_kind.is_network() && target_rate_mbps > 0 {
        (target_rate_mbps as u64) * 1024 * 1024
    } else {
        0
    };

    info!(
        "[NAS_TRANSFER] Starting transfer: '{}' -> '{}' (type: {}, size: {} MB, target_rate: {} MB/s)",
        src.display(),
        tgt.display(),
        fs_kind.as_str(),
        total_bytes / (1024 * 1024),
        if target_rate_bps > 0 { target_rate_mbps } else { 0 }
    );

    let mut buf = vec![0u8; chunk_size];
    let mut transferred = 0u64;
    let t_start = std::time::Instant::now();
    let mut last_progress_time = std::time::Instant::now();

    loop {
        let n = std::io::Read::read(&mut src_file, &mut buf)?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut tgt_file, &buf[..n])?;
        transferred += n as u64;

        // Régulation de débit (pacing)
        if target_rate_bps > 0 {
            let expected_elapsed = std::time::Duration::from_secs_f64(transferred as f64 / target_rate_bps as f64);
            let actual_elapsed = t_start.elapsed();
            if expected_elapsed > actual_elapsed {
                let sleep_duration = expected_elapsed - actual_elapsed;
                if sleep_duration.as_millis() >= 1 {
                    std::thread::sleep(sleep_duration);
                }
            }
        }

        // Notification d'avancement toutes les 500 ms
        if last_progress_time.elapsed().as_millis() >= 500 {
            if let Some(ref mut cb) = progress_cb {
                let elapsed_secs = t_start.elapsed().as_secs_f64();
                let speed = if elapsed_secs > 0.0 {
                    (transferred as f64 / elapsed_secs) as u64
                } else {
                    0
                };
                let percent = if total_bytes > 0 {
                    (transferred as f64 / total_bytes as f64) * 100.0
                } else {
                    100.0
                };
                cb(&NasTransferProgress {
                    bytes_transferred: transferred,
                    total_bytes,
                    speed_bytes_per_sec: speed,
                    percent: percent as f32,
                    fs_kind: fs_kind.as_str().to_string(),
                });
            }
            last_progress_time = std::time::Instant::now();
        }
    }

    // Persistance physique sur les disques distants ou locaux
    tgt_file.sync_data()?;

    if let Some(ref mut cb) = progress_cb {
        let elapsed_secs = t_start.elapsed().as_secs_f64();
        let speed = if elapsed_secs > 0.0 {
            (transferred as f64 / elapsed_secs) as u64
        } else {
            0
        };
        cb(&NasTransferProgress {
            bytes_transferred: transferred,
            total_bytes,
            speed_bytes_per_sec: speed,
            percent: 100.0,
            fs_kind: fs_kind.as_str().to_string(),
        });
    }

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        unsafe {
            let _ = fadvise::posix_fadvise(
                src_file.as_raw_fd(),
                0,
                0,
                fadvise::POSIX_FADV_DONTNEED,
            );
        }
    }

    debug!(
        "[NAS_TRANSFER] Completed transfer of {} bytes in {:.2}s",
        transferred,
        t_start.elapsed().as_secs_f64()
    );

    Ok(transferred)
}

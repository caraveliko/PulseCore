// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::path::Path;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Tamaño de los búferes de transferencia para montajes de red NFS (128 KiB = 131 072 bytes),
/// alineado exactamente con el valor wsize negociado por NFSv3/NFSv4.
pub const NFS_CHUNK_SIZE: usize = 128 * 1024;

/// Tamaño de los búferes para copias locales entre discos físicos distintos (1 MiB).
pub const LOCAL_CHUNK_SIZE: usize = 1024 * 1024;

/// Tasa de transferencia objetivo por defecto hacia el NAS embebido (Synology DS212 con 256 MB RAM) en MB/s.
/// Evita la sobrecarga de búfer (buffer bloat), la saturación de memoria del NAS y las retransmisiones TCP.
pub const DEFAULT_NAS_TRANSFER_RATE_MBPS: u32 = 20;

/// Tipo de sistema de archivos del punto de montaje destino.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemKind {
    /// Disco de bloques local (NVMe, SSD SATA, HDD, ext4, NTFS, XFS, tmpfs).
    Local,
    /// Montaje de red NFS (Network File System).
    Nfs,
    /// Montaje de red Windows / Samba (SMB / CIFS).
    Smb,
    /// Otro montaje de red o remoto (SSHFS, Gluster, Ceph).
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

/// Detecta el tipo de sistema de archivos de la ruta objetivo.
/// En Linux, resuelve el punto de montaje más específico en `/proc/mounts` a través de `nfs_mount`.
#[cfg(target_os = "linux")]
pub fn detect_filesystem_kind(path: &Path) -> FilesystemKind {
    crate::services::nfs_mount::detect_filesystem_kind_linux(path)
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

/// Instantánea de progreso de una transferencia de red o local hacia el destino final.
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

/// Transfiere un archivo hacia su destino final con calibración protocolaria:
/// - Bloques de 128 KiB ajustados a `wsize` para NFS (evita la fragmentación RPC).
/// - Regulación de tasa (*pacing*) para moderar el uso de memoria RAM del NAS (ej. Synology DS212 256 MB).
/// - Precarga y expulsión secuencial de caché (`posix_fadvise`).
/// - Llamada final única a `fdatasync` para garantizar la persistencia física antes de la validación.
pub fn transfer_to_destination<F>(
    src: &Path,
    tgt: &Path,
    target_rate_mbps: u32,
    mut progress_cb: Option<F>,
) -> std::io::Result<u64>
where
    F: FnMut(&NasTransferProgress),
{
    crate::services::nfs_mount::verify_destination_mount_ready(tgt, false)?;
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

        // Regulación de tasa (pacing)
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

        // Notificación de progreso cada 500 ms
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

    // Persistencia física en los discos remotos o locales
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

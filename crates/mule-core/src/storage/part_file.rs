// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;

use crate::error::CoreError;

/// Política de asignación de almacenamiento para archivos de descarga temporales.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AllocationPolicy {
    /// Deja que el archivo crezca con la escritura (crecimiento dinámico —
    /// el nombre «sparse» describe la intención: en NTFS los rangos no
    /// escritos no consumen clústeres mientras el archivo no se
    /// extienda mediante `set_len`; no se realiza ningún marcado
    /// `FSCTL_SET_SPARSE`).
    #[default]
    SparseIfSupported,
    Preallocate,
    None,
}

/// Contadores de E/S posicionadas de un [`PartFile`] — instrumentación de la
/// canalización de disco (invariante objetivo: `pread_bytes ≈ 0` en fase de
/// descarga, `pwrite_bytes == total_size`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PartFileIoStats {
    /// Número de llamadas de lectura posicionada (`pread`/`seek_read`).
    pub read_ops: u64,
    /// Bytes releídos desde el disco.
    pub read_bytes: u64,
    /// Número de llamadas de escritura posicionada (`pwrite`/`seek_write`).
    pub write_ops: u64,
    /// Bytes escritos.
    pub write_bytes: u64,
    /// Número de llamadas reales a `sync_data` (fsync por lotes).
    pub sync_ops: u64,
}

/// Descriptor de un archivo físico de descarga temporal (`.part`).
pub struct PartFile {
    file: File,
    path: PathBuf,
    total_size: u64,
    io_stats: PartFileIoStats,
}

impl PartFile {
    /// Abre o crea un archivo `.part` con la política de asignación especificada.
    pub fn open_or_create<P: AsRef<Path>>(
        path: P,
        total_size: u64,
        policy: AllocationPolicy,
    ) -> Result<Self, CoreError> {
        let path_buf = path.as_ref().to_path_buf();
        if let Some(parent) = path_buf.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path_buf)?;

        let metadata = file.metadata()?;
        if metadata.len() > total_size {
            return Err(CoreError::InvalidRange {
                start: 0,
                end: metadata.len(),
                reason: "part file on disk exceeds total file size boundary",
            });
        }

        let part = Self {
            file,
            path: path_buf,
            total_size,
            io_stats: PartFileIoStats::default(),
        };

        match policy {
            AllocationPolicy::Preallocate | AllocationPolicy::SparseIfSupported => {
                let _ = part.file.set_len(total_size);
            }
            AllocationPolicy::None => {
                // Mantener el tamaño del archivo tal cual o crecer dinámicamente al escribir
            }
        }

        Ok(part)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn total_size(&self) -> u64 {
        self.total_size
    }

    /// Contadores de E/S acumulados desde la apertura del descriptor.
    pub fn io_stats(&self) -> PartFileIoStats {
        self.io_stats
    }

    /// Escritura posicionada (`pwrite`/`seek_write`, una sola llamada al sistema)
    /// — sin `seek` mutable sobre el descriptor compartido.
    pub fn write_segment(&mut self, offset: u64, data: &[u8]) -> Result<(), CoreError> {
        if offset + data.len() as u64 > self.total_size {
            return Err(CoreError::InvalidRange {
                start: offset,
                end: offset + data.len() as u64,
                reason: "segment exceeds file total size boundary",
            });
        }

        positioned_write(&self.file, offset, data)?;
        self.io_stats.write_ops += 1;
        self.io_stats.write_bytes += data.len() as u64;
        Ok(())
    }

    /// Vuelca los búferes no escritos al disco físico.
    pub fn flush(&mut self) -> Result<(), CoreError> {
        self.file.sync_data()?;
        super::fadvise::advise_dontneed_all(&self.file);
        self.io_stats.sync_ops += 1;
        Ok(())
    }

    /// Lectura posicionada (`pread`/`seek_read`) de un rango (verificación
    /// de hash, recuperación AICH) — reservada para rutas frías: la
    /// verificación de partes en línea utiliza el búfer preparado en RAM.
    pub fn read_range(&mut self, offset: u64, len: usize) -> Result<Vec<u8>, CoreError> {
        let mut buf = vec![0u8; len];
        positioned_read(&self.file, offset, &mut buf)?;
        self.io_stats.read_ops += 1;
        self.io_stats.read_bytes += len as u64;
        Ok(buf)
    }
}

/// Escritura posicionada completa — las API `seek_write`/`write_at` tienen
/// permitido escribir menos de lo solicitado: se itera en bucle (equivalente a
/// `write_all`) sin desplazar el cursor del descriptor.
#[cfg(windows)]
fn positioned_write(file: &File, offset: u64, data: &[u8]) -> Result<(), CoreError> {
    let mut written = 0usize;
    while written < data.len() {
        let n = file.seek_write(&data[written..], offset + written as u64)?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "seek_write").into());
        }
        written += n;
    }
    Ok(())
}

#[cfg(unix)]
fn positioned_write(file: &File, offset: u64, data: &[u8]) -> Result<(), CoreError> {
    let mut written = 0usize;
    while written < data.len() {
        let n = file.write_at(&data[written..], offset + written as u64)?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "write_at").into());
        }
        written += n;
    }
    Ok(())
}

/// Lectura posicionada completa (`read_exact`) — fallo explícito si el
/// archivo no contiene suficientes datos (evita un búfer rellenado con ceros
/// silencioso que falsearía una verificación de hash).
#[cfg(windows)]
fn positioned_read(file: &File, offset: u64, buf: &mut [u8]) -> Result<(), CoreError> {
    let mut read = 0usize;
    while read < buf.len() {
        let n = file.seek_read(&mut buf[read..], offset + read as u64)?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "seek_read").into());
        }
        read += n;
    }
    Ok(())
}

#[cfg(unix)]
fn positioned_read(file: &File, offset: u64, buf: &mut [u8]) -> Result<(), CoreError> {
    file.read_exact_at(buf, offset)?;
    Ok(())
}

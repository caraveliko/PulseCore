// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::fs::File;
use std::io::{self, Read};

/// Tamaño de ventana por defecto para expulsar páginas de caché ya leídas o escritas (64 MiB).
pub const FADVISE_WINDOW_BYTES: u64 = 64 * 1024 * 1024;

#[cfg(target_os = "linux")]
mod sys {
    use std::os::unix::io::RawFd;

    /// Indica al núcleo que el patrón de acceso será secuencial (`POSIX_FADV_SEQUENTIAL`).
    pub const POSIX_FADV_SEQUENTIAL: i32 = 2;
    /// Indica al núcleo que las páginas del rango ya no son necesarias (`POSIX_FADV_DONTNEED`).
    pub const POSIX_FADV_DONTNEED: i32 = 4;

    extern "C" {
        pub fn posix_fadvise(fd: RawFd, offset: i64, len: i64, advice: i32) -> i32;
    }
}

/// Aconseja al núcleo acceso secuencial sobre todo el descriptor de archivo.
///
/// En sistemas distintos de Linux es una operación nula (*no-op*).
pub fn advise_sequential(file: &File) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        // SAFETY: `file` es un descriptor abierto válido durante la llamada.
        unsafe {
            let _ = sys::posix_fadvise(file.as_raw_fd(), 0, 0, sys::POSIX_FADV_SEQUENTIAL);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = file;
}

/// Solicita al núcleo expulsar de la caché de página todas las páginas asociadas al archivo.
///
/// En sistemas distintos de Linux es una operación nula (*no-op*).
pub fn advise_dontneed_all(file: &File) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        // SAFETY: `file` es un descriptor abierto válido durante la llamada.
        unsafe {
            let _ = sys::posix_fadvise(file.as_raw_fd(), 0, 0, sys::POSIX_FADV_DONTNEED);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = file;
}

/// Solicita al núcleo expulsar de la caché de página el rango `[offset, offset + len)`.
///
/// Si `len == 0` o alguno de los valores supera `i64::MAX`, la llamada se omite de forma segura.
/// En sistemas distintos de Linux es una operación nula (*no-op*).
pub fn advise_dontneed_range(file: &File, offset: u64, len: u64) {
    if len == 0 {
        return;
    }
    let (Ok(off), Ok(length)) = (i64::try_from(offset), i64::try_from(len)) else {
        return;
    };
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        // SAFETY: `file` es un descriptor abierto válido y `off`/`length` están acotados a `i64`.
        unsafe {
            let _ = sys::posix_fadvise(file.as_raw_fd(), off, length, sys::POSIX_FADV_DONTNEED);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (file, off, length);
}

/// Lector secuencial que expulsa periódicamente de la caché de página los bloques ya consumidos.
///
/// Evita que el cálculo de hashes o la lectura secuencial de archivos grandes sature la memoria
/// de caché de archivos asignada al cgroup del proceso.
pub struct EvictingFileReader {
    file: File,
    bytes_read: u64,
    last_evicted: u64,
    window_bytes: u64,
}

impl EvictingFileReader {
    /// Crea un lector con la ventana de expulsión por defecto ([`FADVISE_WINDOW_BYTES`]).
    pub fn new(file: File) -> Self {
        Self::with_window(file, FADVISE_WINDOW_BYTES)
    }

    /// Crea un lector con una ventana de expulsión personalizada en bytes.
    ///
    /// Si `window_bytes` es `0`, se utiliza [`FADVISE_WINDOW_BYTES`].
    pub fn with_window(file: File, window_bytes: u64) -> Self {
        advise_sequential(&file);
        let effective_window = if window_bytes == 0 {
            FADVISE_WINDOW_BYTES
        } else {
            window_bytes
        };
        Self {
            file,
            bytes_read: 0,
            last_evicted: 0,
            window_bytes: effective_window,
        }
    }
}

impl Read for EvictingFileReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.file.read(buf)?;
        if n > 0 {
            self.bytes_read = self.bytes_read.saturating_add(n as u64);
            let delta = self.bytes_read.saturating_sub(self.last_evicted);
            if delta >= self.window_bytes {
                advise_dontneed_range(&self.file, self.last_evicted, delta);
                self.last_evicted = self.bytes_read;
            }
        }
        Ok(n)
    }
}

impl Drop for EvictingFileReader {
    fn drop(&mut self) {
        advise_dontneed_all(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn advise_helpers_do_not_fail_on_valid_file_and_extreme_ranges() {
        let mut tmp = NamedTempFile::new().expect("crear archivo temporal");
        tmp.write_all(&[0xABu8; 4096]).expect("escribir relleno");
        tmp.flush().expect("volcar archivo temporal");

        let file = tmp.reopen().expect("reabrir archivo temporal");
        advise_sequential(&file);
        advise_dontneed_range(&file, 0, 0);
        advise_dontneed_range(&file, 0, 4096);
        advise_dontneed_range(&file, u64::MAX, 1024);
        advise_dontneed_range(&file, 0, u64::MAX);
        advise_dontneed_all(&file);
    }

    #[test]
    fn evicting_file_reader_reads_full_content_and_triggers_window_eviction() {
        let mut tmp = NamedTempFile::new().expect("crear archivo temporal");
        let payload: Vec<u8> = (0..(16 * 1024)).map(|i| (i % 251) as u8).collect();
        tmp.write_all(&payload).expect("escribir carga de prueba");
        tmp.flush().expect("volcar archivo temporal");

        let file = tmp.reopen().expect("reabrir archivo temporal");
        let mut reader = EvictingFileReader::with_window(file, 4096);
        let mut read_back = Vec::new();
        reader
            .read_to_end(&mut read_back)
            .expect("leer contenido completo");

        assert_eq!(read_back, payload);
        assert_eq!(reader.bytes_read, payload.len() as u64);
        assert_eq!(reader.last_evicted, payload.len() as u64);
    }
}

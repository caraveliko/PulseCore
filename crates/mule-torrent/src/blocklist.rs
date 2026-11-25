// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

//! Exportación atómica de `mule-filter` al formato P2P admitido por `librqbit`.

use std::io::Write;
use std::path::{Path, PathBuf};

use mule_filter::IpFilter;

/// Nombre del archivo de lista de bloqueo P2P exportado para `librqbit`.
const LIBRQBIT_BLOCKLIST_FILENAME: &str = "librqbit_ipfilter.p2p";

/// Exporta los rangos estáticos y baneos dinámicos activos de `filter` a un archivo
/// legible por `librqbit::ip_ranges::IpRanges`.
///
/// Devuelve `Ok(None)` cuando el filtro está deshabilitado o vacío (evitando un archivo
/// vacío que `IpRanges` rechazaría por ser menor de 2 bytes).
pub fn export_ipfilter_to_librqbit_file(
    filter: &IpFilter,
    dir: &Path,
) -> std::io::Result<Option<(PathBuf, usize)>> {
    let ranges = filter.export_ipv4_ranges();
    if ranges.is_empty() {
        return Ok(None);
    }

    std::fs::create_dir_all(dir)?;
    let abs_dir = if dir.is_relative() {
        std::env::current_dir()?.join(dir)
    } else {
        dir.to_path_buf()
    };
    let final_path = abs_dir.join(LIBRQBIT_BLOCKLIST_FILENAME);
    let tmp_path = abs_dir.join(format!("{LIBRQBIT_BLOCKLIST_FILENAME}.tmp"));

    {
        let f = std::fs::File::create(&tmp_path)?;
        let mut writer = std::io::BufWriter::new(f);
        for (start, end) in &ranges {
            writeln!(writer, "mule-filter:{start}-{end}")?;
        }
        writer.flush()?;
        writer.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    }
    std::fs::rename(&tmp_path, &final_path)?;

    Ok(Some((final_path, ranges.len())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn exports_ranges_in_librqbit_p2p_format_and_skips_empty() {
        let dir = tempfile::tempdir().unwrap();
        let filter = IpFilter::new();

        assert!(export_ipfilter_to_librqbit_file(&filter, dir.path())
            .unwrap()
            .is_none());

        filter
            .load_from_str("203.0.113.1 - 203.0.113.50 , 0 , TestRange\n")
            .unwrap();
        let _ = filter.record_connection_attempt(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)));

        let (path, count) = export_ipfilter_to_librqbit_file(&filter, dir.path())
            .unwrap()
            .expect("debe exportarse al menos un rango");
        assert_eq!(count, 1);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("mule-filter:203.0.113.1-203.0.113.50"));
    }
}

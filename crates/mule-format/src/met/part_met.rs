// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use crate::aich::hash_tree::AichHash;
use crate::ed2k::hash::Ed2kHash;
use crate::met::error::MetError;
use crate::met::reader::CountingReader;
use crate::met::tags::*;

pub const PARTFILE_VERSION: u8 = 0xe0;
pub const PARTFILE_SPLITTEDVERSION: u8 = 0xe1;
pub const PARTFILE_VERSION_LARGEFILE: u8 = 0xe2;

pub const MAX_PART_COUNT: usize = 65_536;
pub const MAX_TAG_COUNT: usize = 10_000;

/// Archivo `.part.met` analizado de eMule que rastrea una descarga parcial.
#[derive(Debug, Clone, PartialEq)]
pub struct PartMetFile {
    pub version: u8,
    pub mod_date: u32,
    pub file_hash: Ed2kHash,
    pub part_hashes: Vec<[u8; 16]>,
    pub file_name: Option<String>,
    pub file_size: Option<u64>,
    pub transferred: u64,
    pub aich_hash: Option<AichHash>,
    pub gaps: Vec<(u64, u64)>,
    pub tags: Vec<Tag>,
}

impl PartMetFile {
    /// Lee y valida un archivo `.part.met` desde el disco.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, MetError> {
        let mut file = crate::met::reader::open_met_file(path)?;
        Self::read_from(&mut file)
    }

    /// Deserializa una estructura `.part.met` desde cualquier lector con seguimiento de desplazamiento a nivel de bytes.
    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self, MetError> {
        let mut cr = CountingReader::new(reader);

        let header_offset = cr.offset();
        let version = cr.read_u8("version header")?;
        if version != PARTFILE_VERSION
            && version != PARTFILE_SPLITTEDVERSION
            && version != PARTFILE_VERSION_LARGEFILE
        {
            return Err(MetError::UnsupportedHeader {
                offset: header_offset,
                value: version,
            });
        }

        let mod_date = cr.read_u32_le("mod date")?;

        let hash_bytes = cr.read_exact_bytes(16, "file hash")?;
        let mut hash_buf = [0u8; 16];
        hash_buf.copy_from_slice(&hash_bytes);
        let file_hash = Ed2kHash(hash_buf);

        let count_offset = cr.offset();
        let hash_count = cr.read_u16_le("part hash count")? as usize;
        if hash_count > MAX_PART_COUNT {
            return Err(MetError::InvalidLength {
                offset: count_offset,
                declared: hash_count as u64,
                maximum: MAX_PART_COUNT as u64,
                context: "part hash count",
            });
        }

        let mut part_hashes = Vec::with_capacity(hash_count);
        for _ in 0..hash_count {
            let part_bytes = cr.read_exact_bytes(16, "part hash")?;
            let mut part_buf = [0u8; 16];
            part_buf.copy_from_slice(&part_bytes);
            part_hashes.push(part_buf);
        }

        let tag_count_offset = cr.offset();
        let tag_count = cr.read_u32_le("tag count")? as usize;
        if tag_count > MAX_TAG_COUNT {
            return Err(MetError::InvalidLength {
                offset: tag_count_offset,
                declared: tag_count as u64,
                maximum: MAX_TAG_COUNT as u64,
                context: "tag count",
            });
        }

        let mut tags = Vec::with_capacity(tag_count);
        let mut file_name = None;
        let mut file_size: Option<u64> = None;
        let mut file_size_hi = 0u64;
        let mut transferred = 0u64;
        let mut aich_hash = None;
        let mut gap_starts = Vec::new();
        let mut gap_ends = Vec::new();

        for _ in 0..tag_count {
            let tag = cr.read_tag()?;

            // eMule escribe los límites de los huecos (gaps) como etiquetas enteras *con nombre* cuyo nombre
            // comienza con el byte FT_GAPSTART/FT_GAPEND seguido de un índice decimal
            // (ruta de PartFile.cpp SavePartFile / carga). Resuelve el
            // id FT_* efectivo para esas etiquetas con nombre.
            let effective_id = if tag.name_id != 0 {
                tag.name_id
            } else {
                match tag.name.as_bytes().first().copied().unwrap_or_default() {
                    b @ (FT_GAPSTART | FT_GAPEND) => b,
                    _ => 0,
                }
            };
            match effective_id {
                FT_FILENAME => {
                    if let Some(name) = tag.as_str() {
                        file_name = Some(name.to_string());
                    }
                }
                FT_FILESIZE => {
                    if let Some(sz) = tag.as_u64() {
                        file_size = Some(sz);
                    }
                }
                FT_FILESIZE_HI => {
                    if let Some(hi) = tag.as_u64() {
                        file_size_hi = hi << 32;
                    }
                }
                FT_TRANSFERRED => {
                    if let Some(tr) = tag.as_u64() {
                        transferred = tr;
                    }
                }
                FT_AICH_HASH => {
                    if let Some(s) = tag.as_str() {
                        if let Ok(h) = AichHash::from_base32(s) {
                            aich_hash = Some(h);
                        }
                    }
                }
                FT_GAPSTART => {
                    if let Some(start) = tag.as_u64() {
                        gap_starts.push(start);
                    }
                }
                FT_GAPEND => {
                    if let Some(end) = tag.as_u64() {
                        gap_ends.push(end);
                    }
                }
                _ => {}
            }

            tags.push(tag);
        }

        if let Some(sz) = file_size.as_mut() {
            *sz += file_size_hi;
        }

        let mut gaps = Vec::new();
        for (start, end) in gap_starts.into_iter().zip(gap_ends) {
            gaps.push((start, end));
        }

        Ok(Self {
            version,
            mod_date,
            file_hash,
            part_hashes,
            file_name,
            file_size,
            transferred,
            aich_hash,
            gaps,
            tags,
        })
    }

    /// Devuelve el número total esperado de partes de 9 728 000 bytes para este archivo.
    pub fn part_count(&self) -> usize {
        match self.file_size {
            Some(0) | None => 0,
            Some(sz) => {
                let part_sz = crate::ed2k::hash::ED2K_PART_SIZE as u64;
                sz.div_ceil(part_sz) as usize
            }
        }
    }

    /// Calcula los rangos de bytes completados (adquiridos) invirtiendo los huecos (gaps) faltantes.
    ///
    /// Valida que todos los huecos cumplan `0 <= start <= end <= total_size`.
    /// Devuelve intervalos de bytes semiabiertos ordenados y disjuntos `[start, end)`.
    pub fn compute_completed_ranges(&self) -> Result<Vec<(u64, u64)>, MetError> {
        let total_size = match self.file_size {
            Some(sz) => sz,
            None => return Ok(Vec::new()),
        };

        if total_size == 0 {
            return Ok(Vec::new());
        }

        // Si no hay huecos registrados:
        // Cuando eMule no ha comenzado o descargado nada, o bien escribe un hueco para todo el archivo
        // o tiene transferred == 0. Si transferred == 0 y gaps está vacío, no se ha adquirido nada.
        // Si transferred > 0 y gaps está vacío, el archivo completo está descargado.
        if self.gaps.is_empty() {
            if self.transferred == 0 {
                return Ok(Vec::new());
            } else {
                return Ok(vec![(0, total_size)]);
            }
        }

        // Validar huecos
        let mut normalized_gaps = Vec::with_capacity(self.gaps.len());
        for &(start, end) in &self.gaps {
            if start > end || end > total_size {
                return Err(MetError::InconsistentMetadata {
                    field: "gaps",
                    reason: format!(
                        "gap interval [{}, {}] exceeds file total size {}",
                        start, end, total_size
                    ),
                });
            }
            if start < end {
                normalized_gaps.push((start, end));
            }
        }

        // Ordenar y fusionar huecos superpuestos/contiguos
        normalized_gaps.sort_by_key(|g| g.0);
        let mut merged_gaps: Vec<(u64, u64)> = Vec::new();
        for (s, e) in normalized_gaps {
            if let Some(last) = merged_gaps.last_mut() {
                if s <= last.1 {
                    last.1 = last.1.max(e);
                    continue;
                }
            }
            merged_gaps.push((s, e));
        }

        // Invertir huecos en [0, total_size)
        let mut completed = Vec::new();
        let mut cursor = 0u64;

        for (g_start, g_end) in merged_gaps {
            if g_start > cursor {
                completed.push((cursor, g_start));
            }
            cursor = cursor.max(g_end);
        }

        if cursor < total_size {
            completed.push((cursor, total_size));
        }

        Ok(completed)
    }

    /// Serializa este `.part.met` en `writer` en el formato de cable
    /// de `CPartFile::SavePartFile` de eMule.
    ///
    /// Disposición: `u8 version`, `u32 mod_date`, `hash16`, `u16 part_count`,
    /// hashes de partes, `u32 tag_count`, luego etiquetas (nombre de archivo, tamaño, transferido,
    /// hash AICH y etiquetas de hueco con nombre `\x09<idx>` / `\x0A<idx>`).
    /// Las etiquetas enteras de 64 bits solo se emiten para archivos grandes (> 4 GiB),
    /// coincidiendo con la semántica de `PARTFILE_VERSION_LARGEFILE`.
    pub fn write_to<W: Write>(&self, w: &mut W) -> Result<(), MetError> {
        let large_file = self.file_size.unwrap_or(0) > u32::MAX as u64;
        let version = if large_file {
            PARTFILE_VERSION_LARGEFILE
        } else {
            PARTFILE_VERSION
        };

        w.write_all(&[version]).map_err(|e| met_io_err(0, e))?;
        w.write_all(&self.mod_date.to_le_bytes())
            .map_err(|e| met_io_err(0, e))?;
        w.write_all(&self.file_hash.0)
            .map_err(|e| met_io_err(0, e))?;
        w.write_all(&(self.part_hashes.len() as u16).to_le_bytes())
            .map_err(|e| met_io_err(0, e))?;
        for ph in &self.part_hashes {
            w.write_all(ph).map_err(|e| met_io_err(0, e))?;
        }

        // Construir la lista de etiquetas: nombre de archivo, tamaño, transferido, AICH, huecos.
        let mut tags: Vec<Tag> = Vec::new();
        if let Some(name) = &self.file_name {
            tags.push(Tag::new_id(FT_FILENAME, MetTagValue::String(name.clone())));
        }
        if let Some(size) = self.file_size {
            tags.push(Tag::new_id(FT_FILESIZE, MetTagValue::Uint64(size)));
        }
        if self.transferred > 0 {
            tags.push(Tag::new_id(
                FT_TRANSFERRED,
                MetTagValue::Uint64(self.transferred),
            ));
        }
        if let Some(aich) = &self.aich_hash {
            tags.push(Tag::new_id(
                FT_AICH_HASH,
                MetTagValue::String(aich.to_base32()),
            ));
        }
        // Etiquetas adicionales suministradas por el llamador (solo str/int, reflejando m_taglist de eMule).
        for t in &self.tags {
            if matches!(t.value, MetTagValue::String(_) | MetTagValue::Uint64(_)) {
                tags.push(t.clone());
            }
        }
        // Los huecos se escriben como etiquetas con nombre "<FT_GAPSTART><idx>" / "<FT_GAPEND><idx>"
        // (SavePartFile de eMule escribe `gap.end + 1` — límite exclusivo real).
        for (i, (start, end)) in self.gaps.iter().enumerate() {
            let mut sname = vec![FT_GAPSTART];
            sname.extend_from_slice(i.to_string().as_bytes());
            tags.push(Tag {
                name_id: 0,
                name: String::from_utf8_lossy(&sname).into_owned(),
                value: MetTagValue::Uint64(*start),
            });
            let mut ename = vec![FT_GAPEND];
            ename.extend_from_slice(i.to_string().as_bytes());
            tags.push(Tag {
                name_id: 0,
                name: String::from_utf8_lossy(&ename).into_owned(),
                value: MetTagValue::Uint64(*end),
            });
        }

        w.write_all(&(tags.len() as u32).to_le_bytes())
            .map_err(|e| met_io_err(0, e))?;
        for tag in &tags {
            write_tag(w, tag, large_file)?;
        }
        Ok(())
    }

    /// Escribe atómicamente este `.part.met` en `path`: los datos van a
    /// `path.tmp` primero y luego se renombran sobre el destino
    /// (refleja la semántica tmp+move de `SavePartFile` de eMule).
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), MetError> {
        let path = path.as_ref();
        let mut tmp_name = path.as_os_str().to_os_string();
        tmp_name.push(".tmp");
        let tmp_path = Path::new(&tmp_name);

        let write_res = (|| -> Result<(), MetError> {
            let mut file = File::create(tmp_path).map_err(|e| MetError::Io {
                offset: 0,
                message: e.to_string(),
            })?;
            self.write_to(&mut file)?;
            file.flush().map_err(|e| MetError::Io {
                offset: 0,
                message: e.to_string(),
            })?;
            Ok(())
        })();

        if let Err(e) = write_res {
            let _ = std::fs::remove_file(tmp_path);
            return Err(e);
        }

        std::fs::rename(tmp_path, path).map_err(|e| MetError::Io {
            offset: 0,
            message: e.to_string(),
        })
    }
}

fn met_io_err(offset: u64, e: std::io::Error) -> MetError {
    MetError::Io {
        offset,
        message: e.to_string(),
    }
}

/// Escribe una sola etiqueta en la forma binaria `.met` de eMule:
/// `u8 type`, luego `u16 name_len=1 + u8 name_id` para etiquetas con nombre por id o
/// `u16 name_len + name bytes` para etiquetas con nombre, luego el valor tipado.
/// Las etiquetas enteras se emiten como `TAGTYPE_UINT32`, o `TAGTYPE_UINT64` cuando
/// `large_file` está activo (convención `IsLargeFile()` de eMule).
fn write_tag<W: Write>(w: &mut W, tag: &Tag, large_file: bool) -> Result<(), MetError> {
    let tag_type: u8 = match &tag.value {
        MetTagValue::String(_) => TAGTYPE_STRING,
        MetTagValue::Uint64(v) if large_file || *v > u32::MAX as u64 => TAGTYPE_UINT64,
        MetTagValue::Uint64(_) => TAGTYPE_UINT32,
        MetTagValue::Float32(_) => TAGTYPE_FLOAT32,
        MetTagValue::Hash16(_) => TAGTYPE_HASH16,
        MetTagValue::Blob(_) => TAGTYPE_BLOB,
        MetTagValue::Bsob(_) => TAGTYPE_BSOB,
        MetTagValue::Bool(_) => TAGTYPE_BOOL,
    };
    w.write_all(&[tag_type]).map_err(|e| met_io_err(0, e))?;

    if tag.name_id != 0 {
        w.write_all(&1u16.to_le_bytes())
            .map_err(|e| met_io_err(0, e))?;
        w.write_all(&[tag.name_id]).map_err(|e| met_io_err(0, e))?;
    } else {
        w.write_all(&(tag.name.len() as u16).to_le_bytes())
            .map_err(|e| met_io_err(0, e))?;
        w.write_all(tag.name.as_bytes())
            .map_err(|e| met_io_err(0, e))?;
    }

    match &tag.value {
        MetTagValue::String(s) => {
            w.write_all(&(s.len() as u16).to_le_bytes())
                .map_err(|e| met_io_err(0, e))?;
            w.write_all(s.as_bytes()).map_err(|e| met_io_err(0, e))?;
        }
        MetTagValue::Uint64(v) => {
            if tag_type == TAGTYPE_UINT64 {
                w.write_all(&v.to_le_bytes())
                    .map_err(|e| met_io_err(0, e))?;
            } else {
                w.write_all(&(*v as u32).to_le_bytes())
                    .map_err(|e| met_io_err(0, e))?;
            }
        }
        MetTagValue::Float32(f) => {
            w.write_all(&f.to_le_bytes())
                .map_err(|e| met_io_err(0, e))?;
        }
        MetTagValue::Hash16(h) => {
            w.write_all(&h.0).map_err(|e| met_io_err(0, e))?;
        }
        MetTagValue::Blob(b) => {
            w.write_all(&(b.len() as u32).to_le_bytes())
                .map_err(|e| met_io_err(0, e))?;
            w.write_all(b).map_err(|e| met_io_err(0, e))?;
        }
        MetTagValue::Bsob(b) => {
            w.write_all(&[b.len() as u8])
                .map_err(|e| met_io_err(0, e))?;
            w.write_all(b).map_err(|e| met_io_err(0, e))?;
        }
        MetTagValue::Bool(v) => {
            w.write_all(&[*v as u8]).map_err(|e| met_io_err(0, e))?;
        }
    }
    Ok(())
}

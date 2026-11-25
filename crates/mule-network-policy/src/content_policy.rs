// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Política de contenido «Solo Vídeo» para Pulse Core.
//!
//! Fuente única de verdad para la admisión por nombre/extensión, validación de
//! cabeceras binarias en bruto (sin procesos externos) y selección de archivos
//! en torrents multifichero. Diseñada para cero asignaciones en ruta caliente y
//! consumo mínimo de CPU y E/S.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Extensiones de archivos de vídeo permitidas por la política de contenido.
pub const ALLOWED_VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "avi", "m4v", "mov", "wmv", "webm", "ts", "m2ts", "mts", "mpg", "mpeg", "m2v",
    "vob", "flv", "ogv", "ogm", "asf", "divx", "rm", "rmvb", "3gp", "3g2", "f4v",
];

/// Extensiones de subtítulos complementarios permitidas dentro de torrents con vídeo.
pub const COMPANION_SUBTITLE_EXTENSIONS: &[&str] =
    &["srt", "vtt", "ass", "ssa", "sub", "idx", "sup"];

/// Extensiones peligrosas o no multimedia prohibidas en posición final o intermedia (doble extensión).
pub const FORBIDDEN_EMBEDDED_EXTENSIONS: &[&str] = &[
    "exe", "msi", "bat", "cmd", "scr", "pif", "vbs", "vbe", "js", "jse", "wsf", "ps1", "sh",
    "apk", "dll", "sys", "lnk", "reg", "jar", "app", "dmg", "deb", "rpm", "zip", "rar", "7z",
    "tar", "gz", "bz2", "xz", "iso", "img", "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx",
    "html", "htm", "url",
];

/// Tamaño mínimo de cabecera requerido cuando ninguna firma binaria ha coincidido aún.
pub const HEADER_INSPECT_MIN_BYTES: usize = 16;

/// Ventana óptima de cabecera inspeccionada en memoria RAM durante la transferencia (4 KiB).
pub const HEADER_INSPECT_BYTES: usize = 4096;

/// Ventana máxima leída desde disco antes de finalizar un archivo (64 KiB).
pub const FILE_HEADER_INSPECT_BYTES: usize = 65_536;

/// Cabecera mínima válida de Matroska (EBML + DocType `matroska`) reutilizable en pruebas.
pub const MINIMAL_VALID_MKV_HEADER: &[u8] = b"\x1A\x45\xDF\xA3\x93\x42\x82\x88matroska";

/// Firma mágica EBML de Matroska y WebM.
const EBML_MAGIC: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];
/// Identificador de elemento EBML `DocType` (`0x42 0x82`).
const EBML_DOCTYPE_TAG: [u8; 2] = [0x42, 0x82];
/// Límite de barrido para localizar el `DocType` dentro de la cabecera EBML.
const EBML_SCAN_LIMIT: usize = 128;

/// Tamaño mínimo de cabecera de caja ISO BMFF (`size` + `type`).
const BOX_HEADER_SIZE: usize = 8;
/// Desplazamiento del tipo de caja `ftyp` en ISO BMFF.
const FTYP_OFFSET: usize = 4;
/// Desplazamiento de la marca principal (`major_brand`) en la caja `ftyp`.
const FTYP_MAJOR_OFFSET: usize = 8;
/// Tamaño mínimo válido de una caja `ftyp` (tamaño + `ftyp` + `major_brand`).
const FTYP_MIN_SIZE: usize = 12;
/// Desplazamiento donde comienzan las marcas compatibles en ISO BMFF.
const FTYP_COMPAT_OFFSET: usize = 16;
/// Longitud en bytes de cada identificador de marca o átomo ISO BMFF.
const BRAND_LEN: usize = 4;

/// Marcas principales ISO BMFF correspondientes exclusivamente a audio o imagen fija.
const AUDIO_OR_IMAGE_BMFF_BRANDS: &[&[u8; 4]] = &[
    b"M4A ", b"M4B ", b"M4P ", b"heic", b"heix", b"mif1", b"msf1", b"avif", b"avis",
];

/// Marcas compatibles específicas de vídeo capaces de validar contenedores ISO BMFF de forma inequívoca.
const STRICT_VIDEO_BMFF_BRANDS: &[&[u8; 4]] = &[
    b"M4V ", b"m4v ", b"qt  ", b"avc1", b"hev1", b"hvc1", b"av01", b"vp09", b"3gp4", b"3gp5",
    b"3gp6", b"3gp7", b"3g2a", b"3g2b", b"f4v ", b"MSNV", b"NDXC", b"XAVC", b"dby1",
];

/// Marcas ISO BMFF admitidas cuando la marca principal no es de audio/imagen puro.
const VIDEO_BMFF_BRANDS: &[&[u8; 4]] = &[
    b"isom", b"iso2", b"iso3", b"iso4", b"iso5", b"iso6", b"mp41", b"mp42", b"M4V ", b"m4v ",
    b"qt  ", b"avc1", b"hev1", b"hvc1", b"av01", b"vp09", b"3gp4", b"3gp5", b"3gp6", b"3gp7",
    b"3g2a", b"3g2b", b"f4v ", b"MSNV", b"NDXC", b"dash", b"cmfc", b"mp71", b"kddi", b"mmp4",
    b"XAVC", b"dby1",
];

/// Átomos iniciales admitidos en QuickTime clásico anterior a `ftyp`.
const CLASSIC_QT_ATOMS: &[&[u8; 4]] = &[b"moov", b"mdat", b"wide", b"free", b"skip", b"pnot"];

/// Desplazamiento de la subetiqueta de formato en contenedores RIFF (`WAVE`, `WEBP`, `AVI `).
const RIFF_TYPE_OFFSET: usize = 8;
/// Tamaño mínimo de cabecera RIFF con tipo de contenedor.
const RIFF_MIN_HEADER: usize = 12;

/// Byte de sincronización de paquetes de flujo de transporte MPEG (`0x47`).
const MPEG_TS_SYNC_BYTE: u8 = 0x47;
/// Longitud estándar de un paquete MPEG-TS en bytes.
const MPEG_TS_PACKET_LEN: usize = 188;
/// Longitud estándar de un paquete M2TS (Blu-ray BDAV) en bytes.
const M2TS_PACKET_LEN: usize = 192;
/// Desplazamiento del byte de sincronización tras la marca temporal de 4 bytes en M2TS.
const M2TS_SYNC_OFFSET: usize = 4;
/// Longitud mínima para verificar dos paquetes M2TS consecutivos.
const M2TS_TWO_PACKETS_LEN: usize = M2TS_PACKET_LEN + M2TS_SYNC_OFFSET + 1;
/// Longitud mínima para verificar tres paquetes M2TS consecutivos.
const M2TS_THREE_PACKETS_LEN: usize = 2 * M2TS_PACKET_LEN + M2TS_SYNC_OFFSET + 1;
/// Longitud mínima de la ventana inspeccionada para descartar texto plano con 'G' repetidas en MPEG-TS.
const ASCII_TEXT_GUARD_LIMIT: usize = 512;

/// GUID canónico de cabecera ASF / WMV (`30 26 B2 75 8E 66 CF 11 A6 D9 00 AA 00 62 CE 6C`).
const ASF_HEADER_GUID: [u8; 16] = [
    0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C,
];

/// Tamaño mínimo de cabecera FLV (`"FLV\x01"` + byte de indicadores).
const FLV_MIN_HEADER: usize = 5;
/// Máscara del bit que indica presencia de pista de vídeo en la cabecera FLV.
const FLV_VIDEO_FLAG_MASK: u8 = 0x01;

/// Límite de barrido en bytes para identificar códecs de vídeo dentro de una página Ogg.
const OGG_SCAN_LIMIT: usize = 512;
/// Firmas binarias de códecs/cabeceras de vídeo dentro de contenedores Ogg.
const OGG_VIDEO_SIGNATURES: &[&[u8]] = &[
    b"\x80theora",
    b"OVP80",
    b"BBCD",
    b"fishead",
    b"\x01video",
    b"video\0",
];

/// Desplazamiento de la firma `CD001` en descriptores de volumen ISO 9660.
const ISO9660_VD_OFFSET: usize = 0x8001;
/// Límite final de la firma `CD001` en ISO 9660.
const ISO9660_VD_END: usize = 0x8006;

/// Longitud mínima de cabecera para reconocer texto HTML/XML o scripts.
const TEXT_MAGIC_MIN_LEN: usize = 5;

/// Tipo de contenedor de vídeo reconocido mediante inspección binaria.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoContainerKind {
    /// Contenedor Matroska (`.mkv`).
    Matroska,
    /// Contenedor WebM (`.webm`).
    WebM,
    /// Contenedor ISO BMFF de vídeo (`.mp4`, `.m4v`, `.mov`, `.3gp`, `.3g2`, `.f4v`).
    IsoBmffVideo,
    /// Contenedor Audio Video Interleave (`.avi`, `.divx`).
    Avi,
    /// Flujo de transporte MPEG (`.ts`, `.m2ts`, `.mts`).
    MpegTs,
    /// Flujo de programa MPEG (`.mpg`, `.mpeg`, `.m2v`, `.vob`).
    MpegPs,
    /// Advanced Systems Format / Windows Media Video (`.wmv`, `.asf`).
    AsfWmv,
    /// Flash Video con pista de vídeo declarada (`.flv`).
    Flv,
    /// Contenedor Ogg con firma de vídeo (`.ogv`, `.ogm`).
    OggVideo,
    /// Contenedor RealMedia (`.rm`, `.rmvb`).
    RealMedia,
}

/// Errores tipados emitidos al aplicar la política de contenido «Solo Vídeo».
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContentPolicyError {
    /// El nombre de archivo está vacío tras limpiar espacios.
    #[error("el nombre de archivo está vacío y no cumple la política de solo vídeo")]
    EmptyFileName,
    /// El archivo carece de extensión explícita.
    #[error("el archivo '{file_name}' carece de extensión de vídeo permitida")]
    MissingExtension { file_name: String },
    /// La extensión final no pertenece a la lista blanca de formatos de vídeo.
    #[error("extensión '.{extension}' no permitida en '{file_name}': solo se admiten archivos de vídeo")]
    DisallowedExtension {
        file_name: String,
        extension: String,
    },
    /// El nombre oculta una extensión peligrosa o no multimedia delante de la extensión final.
    #[error("extensión incrustada peligrosa '.{embedded}' detectada en '{file_name}'")]
    DangerousDoubleExtension { file_name: String, embedded: String },
    /// Los bytes iniciales del archivo no corresponden a ningún contenedor de vídeo válido.
    #[error("la cabecera binaria de '{file_name}' no corresponde a un vídeo válido ({detected})")]
    InvalidVideoHeader { file_name: String, detected: String },
    /// Un torrent no contiene ningún archivo de vídeo seleccionable.
    #[error("el torrent no contiene ningún archivo de vídeo permitido")]
    NoVideoFilesInTorrent,
    /// Error de lectura al inspeccionar la cabecera del archivo en disco.
    #[error("no se pudo leer la cabecera de '{file_name}': {reason}")]
    HeaderReadError { file_name: String, reason: String },
}

/// Resultado de filtrar los archivos declarados por un torrent según la política de vídeo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentFileSelection {
    /// Índices ordenados y sin duplicados seleccionados para descargar (vídeos + subtítulos).
    pub selected_indices: Vec<usize>,
    /// Índices correspondientes exclusivamente a archivos de vídeo.
    pub video_indices: Vec<usize>,
    /// Índices correspondientes a subtítulos complementarios incluidos junto al vídeo.
    pub subtitle_indices: Vec<usize>,
    /// Número de archivos omitidos por no ser vídeo ni subtítulo complementario.
    pub skipped_count: usize,
}

/// Extrae el componente final del nombre de archivo ignorando separadores Unix y Windows.
fn base_filename(file_name: &str) -> &str {
    let trimmed = file_name.trim();
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(trimmed)
        .trim()
}

/// Devuelve la extensión final normalizada en minúsculas (sin punto), si existe y no está vacía.
pub fn normalized_extension(file_name: &str) -> Option<String> {
    let base = base_filename(file_name);
    let (_, ext) = base.rsplit_once('.')?;
    let clean = ext.trim();
    if clean.is_empty() {
        None
    } else {
        Some(clean.to_ascii_lowercase())
    }
}

/// Comprueba si una extensión (con o sin punto inicial) está en la lista blanca de vídeo.
pub fn is_allowed_video_extension(ext: &str) -> bool {
    let clean = ext.trim().trim_start_matches('.');
    if clean.is_empty() {
        return false;
    }
    ALLOWED_VIDEO_EXTENSIONS
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(clean))
}

/// Comprueba si un nombre de archivo supera la validación nominal de vídeo.
pub fn is_allowed_video_filename(file_name: &str) -> bool {
    validate_video_filename(file_name).is_ok()
}

/// Comprueba si un nombre corresponde a un subtítulo complementario sin doble extensión peligrosa.
pub fn is_companion_subtitle_filename(file_name: &str) -> bool {
    let base = base_filename(file_name);
    let Some((stem, ext)) = base.rsplit_once('.') else {
        return false;
    };
    let clean_ext = ext.trim();
    if clean_ext.is_empty()
        || !COMPANION_SUBTITLE_EXTENSIONS
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(clean_ext))
    {
        return false;
    }
    find_embedded_forbidden_extension(stem).is_none()
}

/// Busca si el segmento anterior a la extensión final termina en una extensión peligrosa incrustada.
fn find_embedded_forbidden_extension(stem: &str) -> Option<String> {
    let (_, prev_ext) = stem.rsplit_once('.')?;
    let clean = prev_ext.trim();
    if clean.is_empty() {
        return None;
    }
    if FORBIDDEN_EMBEDDED_EXTENSIONS
        .iter()
        .any(|bad| bad.eq_ignore_ascii_case(clean))
    {
        Some(clean.to_ascii_lowercase())
    } else {
        None
    }
}

/// Valida que el nombre de archivo tenga una extensión de vídeo permitida y carezca de doble extensión peligrosa.
pub fn validate_video_filename(file_name: &str) -> Result<(), ContentPolicyError> {
    let base = base_filename(file_name);
    if base.is_empty() {
        return Err(ContentPolicyError::EmptyFileName);
    }

    let Some((stem, ext)) = base.rsplit_once('.') else {
        return Err(ContentPolicyError::MissingExtension {
            file_name: base.to_string(),
        });
    };

    let clean_ext = ext.trim();
    if clean_ext.is_empty() {
        return Err(ContentPolicyError::MissingExtension {
            file_name: base.to_string(),
        });
    }

    if !is_allowed_video_extension(clean_ext) {
        return Err(ContentPolicyError::DisallowedExtension {
            file_name: base.to_string(),
            extension: clean_ext.to_ascii_lowercase(),
        });
    }

    if let Some(embedded) = find_embedded_forbidden_extension(stem) {
        return Err(ContentPolicyError::DangerousDoubleExtension {
            file_name: base.to_string(),
            embedded,
        });
    }

    Ok(())
}

/// Busca una subsecuencia de bytes dentro de una ventana acotada sin realizar asignaciones.
fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Detecta de forma descriptiva firmas conocidas de archivos ejecutables, comprimidos, documentos, audio o imagen.
fn detect_known_non_video_signature(header: &[u8]) -> Option<&'static str> {
    if header.starts_with(b"MZ") {
        return Some("ejecutable Windows PE/MZ");
    }
    if header.starts_with(b"\x7FELF") {
        return Some("binario ejecutable ELF");
    }
    if header.len() >= 4
        && (header.starts_with(&[0xFE, 0xED, 0xFA, 0xCE])
            || header.starts_with(&[0xFE, 0xED, 0xFA, 0xCF])
            || header.starts_with(&[0xCE, 0xFA, 0xED, 0xFE])
            || header.starts_with(&[0xCF, 0xFA, 0xED, 0xFE])
            || header.starts_with(&[0xCA, 0xFE, 0xBA, 0xBE]))
    {
        return Some("binario ejecutable Mach-O");
    }
    if header.starts_with(b"PK\x03\x04") || header.starts_with(b"PK\x05\x06") {
        return Some("archivo comprimido ZIP/Office");
    }
    if header.starts_with(b"Rar!\x1A\x07") {
        return Some("archivo comprimido RAR");
    }
    if header.starts_with(b"7z\xBC\xAF\x27\x1C") {
        return Some("archivo comprimido 7z");
    }
    if header.starts_with(b"%PDF-") {
        return Some("documento PDF");
    }
    if header.starts_with(&[0x1F, 0x8B]) {
        return Some("archivo comprimido GZIP");
    }
    if header.starts_with(b"BZh") {
        return Some("archivo comprimido BZIP2");
    }
    if header.starts_with(&[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00]) {
        return Some("archivo comprimido XZ");
    }
    if header.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("imagen JPEG");
    }
    if header.starts_with(b"\x89PNG\r\n\x1A\n") {
        return Some("imagen PNG");
    }
    if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") {
        return Some("imagen GIF");
    }
    if header.starts_with(b"ID3") {
        return Some("audio MP3/ID3");
    }
    if header.starts_with(b"fLaC") {
        return Some("audio FLAC");
    }
    if header.len() >= RIFF_MIN_HEADER && &header[0..4] == b"RIFF" {
        if &header[RIFF_TYPE_OFFSET..RIFF_MIN_HEADER] == b"WAVE" {
            return Some("audio WAV");
        }
        if &header[RIFF_TYPE_OFFSET..RIFF_MIN_HEADER] == b"WEBP" {
            return Some("imagen WebP");
        }
    }
    if header.starts_with(b"#!") {
        return Some("script ejecutable");
    }
    if header.len() >= TEXT_MAGIC_MIN_LEN {
        let trimmed = trim_ascii_whitespace_prefix(header);
        if starts_with_ignore_ascii_case(trimmed, b"<!doctype")
            || starts_with_ignore_ascii_case(trimmed, b"<html")
            || starts_with_ignore_ascii_case(trimmed, b"<?xml")
        {
            return Some("documento HTML/XML");
        }
    }
    if header.len() >= ISO9660_VD_END && &header[ISO9660_VD_OFFSET..ISO9660_VD_END] == b"CD001" {
        return Some("imagen de disco ISO 9660");
    }
    None
}

fn trim_ascii_whitespace_prefix(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    &bytes[start..]
}

fn starts_with_ignore_ascii_case(haystack: &[u8], prefix: &[u8]) -> bool {
    haystack.len() >= prefix.len()
        && haystack[..prefix.len()]
            .iter()
            .zip(prefix.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// Valida que una cabecera binaria corresponda positivamente a un contenedor de vídeo permitido.
pub fn validate_video_header(
    header: &[u8],
    file_name: &str,
) -> Result<VideoContainerKind, ContentPolicyError> {
    if let Some(non_video) = detect_known_non_video_signature(header) {
        return Err(ContentPolicyError::InvalidVideoHeader {
            file_name: file_name.to_string(),
            detected: non_video.to_string(),
        });
    }

    // 1. Matroska / WebM (EBML)
    if header.starts_with(&EBML_MAGIC) {
        let scan_end = header.len().min(EBML_SCAN_LIMIT);
        let window = &header[..scan_end];
        if contains_subslice(window, b"webm") {
            return Ok(VideoContainerKind::WebM);
        }
        if contains_subslice(window, b"matroska") {
            return Ok(VideoContainerKind::Matroska);
        }
        let detected = if contains_subslice(window, &EBML_DOCTYPE_TAG) {
            "contenedor EBML con DocType distinto de matroska/webm"
        } else {
            "cabecera EBML sin DocType matroska/webm"
        };
        return Err(ContentPolicyError::InvalidVideoHeader {
            file_name: file_name.to_string(),
            detected: detected.to_string(),
        });
    }

    // 2. ISO BMFF (MP4 / M4V / MOV / 3GP / F4V)
    if header.len() >= FTYP_MIN_SIZE && &header[FTYP_OFFSET..FTYP_MAJOR_OFFSET] == b"ftyp" {
        let box_size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        if box_size >= FTYP_MIN_SIZE {
            let major: [u8; 4] = [
                header[FTYP_MAJOR_OFFSET],
                header[FTYP_MAJOR_OFFSET + 1],
                header[FTYP_MAJOR_OFFSET + 2],
                header[FTYP_MAJOR_OFFSET + 3],
            ];
            let compat_end = box_size.min(header.len());
            let is_audio_or_image_major = AUDIO_OR_IMAGE_BMFF_BRANDS.contains(&&major);

            let has_video_brand = if is_audio_or_image_major {
                compat_end >= FTYP_COMPAT_OFFSET + BRAND_LEN
                    && header[FTYP_COMPAT_OFFSET..compat_end]
                        .as_chunks::<BRAND_LEN>()
                        .0
                        .iter()
                        .any(|chunk| STRICT_VIDEO_BMFF_BRANDS.contains(&chunk))
            } else {
                VIDEO_BMFF_BRANDS.contains(&&major)
                    || (compat_end >= FTYP_COMPAT_OFFSET + BRAND_LEN
                        && header[FTYP_COMPAT_OFFSET..compat_end]
                            .as_chunks::<BRAND_LEN>()
                            .0
                            .iter()
                            .any(|chunk| VIDEO_BMFF_BRANDS.contains(&chunk)))
            };

            if has_video_brand {
                return Ok(VideoContainerKind::IsoBmffVideo);
            }

            let brand_str = String::from_utf8_lossy(&major).into_owned();
            let detected = if is_audio_or_image_major {
                format!("contenedor ISO BMFF de audio/imagen (marca '{brand_str}')")
            } else {
                format!("contenedor ISO BMFF sin marca de vídeo reconocida (marca '{brand_str}')")
            };
            return Err(ContentPolicyError::InvalidVideoHeader {
                file_name: file_name.to_string(),
                detected,
            });
        }
    }

    // QuickTime clásico sin caja `ftyp`
    if header.len() >= BOX_HEADER_SIZE {
        let atom_size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        if atom_size >= BOX_HEADER_SIZE
            && CLASSIC_QT_ATOMS
                .iter()
                .any(|atom| atom.as_slice() == &header[FTYP_OFFSET..BOX_HEADER_SIZE])
        {
            return Ok(VideoContainerKind::IsoBmffVideo);
        }
    }

    // 3. AVI
    if header.len() >= RIFF_MIN_HEADER
        && &header[0..4] == b"RIFF"
        && (&header[RIFF_TYPE_OFFSET..RIFF_MIN_HEADER] == b"AVI "
            || &header[RIFF_TYPE_OFFSET..RIFF_MIN_HEADER] == b"AVIX")
    {
        return Ok(VideoContainerKind::Avi);
    }

    // 4. MPEG-TS (188 B) y M2TS (192 B), evitando falsos positivos sobre texto ASCII repetitivo
    let is_pure_ascii_text = header.len() >= MPEG_TS_PACKET_LEN
        && header[..header.len().min(ASCII_TEXT_GUARD_LIMIT)]
            .iter()
            .all(|b| b.is_ascii_graphic() || b.is_ascii_whitespace());

    if !is_pure_ascii_text {
        let m2ts_match = if header.len() >= M2TS_THREE_PACKETS_LEN {
            header[M2TS_SYNC_OFFSET] == MPEG_TS_SYNC_BYTE
                && header[M2TS_SYNC_OFFSET + M2TS_PACKET_LEN] == MPEG_TS_SYNC_BYTE
                && header[M2TS_SYNC_OFFSET + 2 * M2TS_PACKET_LEN] == MPEG_TS_SYNC_BYTE
        } else if header.len() >= M2TS_TWO_PACKETS_LEN {
            header[M2TS_SYNC_OFFSET] == MPEG_TS_SYNC_BYTE
                && header[M2TS_SYNC_OFFSET + M2TS_PACKET_LEN] == MPEG_TS_SYNC_BYTE
        } else {
            false
        };
        if m2ts_match {
            return Ok(VideoContainerKind::MpegTs);
        }

        let max_offset = MPEG_TS_PACKET_LEN.min(header.len());
        for start in 0..max_offset {
            if header[start] == MPEG_TS_SYNC_BYTE
                && start + MPEG_TS_PACKET_LEN < header.len()
                && header[start + MPEG_TS_PACKET_LEN] == MPEG_TS_SYNC_BYTE
                && (start + 2 * MPEG_TS_PACKET_LEN >= header.len()
                    || header[start + 2 * MPEG_TS_PACKET_LEN] == MPEG_TS_SYNC_BYTE)
            {
                return Ok(VideoContainerKind::MpegTs);
            }
        }
    }

    // 5. MPEG-PS / VOB
    if header.starts_with(&[0x00, 0x00, 0x01, 0xBA])
        || header.starts_with(&[0x00, 0x00, 0x01, 0xB3])
    {
        return Ok(VideoContainerKind::MpegPs);
    }

    // 6. ASF / WMV
    if header.starts_with(&ASF_HEADER_GUID) {
        return Ok(VideoContainerKind::AsfWmv);
    }

    // 7. FLV con bandera de vídeo activa
    if header.len() >= FLV_MIN_HEADER && header.starts_with(b"FLV\x01") {
        if (header[4] & FLV_VIDEO_FLAG_MASK) != 0 {
            return Ok(VideoContainerKind::Flv);
        }
        return Err(ContentPolicyError::InvalidVideoHeader {
            file_name: file_name.to_string(),
            detected: "contenedor FLV sin pista de vídeo (solo audio)".to_string(),
        });
    }

    // 8. Ogg Video
    if header.starts_with(b"OggS") {
        let scan_end = header.len().min(OGG_SCAN_LIMIT);
        let window = &header[..scan_end];
        if OGG_VIDEO_SIGNATURES
            .iter()
            .any(|sig| contains_subslice(window, sig))
        {
            return Ok(VideoContainerKind::OggVideo);
        }
        return Err(ContentPolicyError::InvalidVideoHeader {
            file_name: file_name.to_string(),
            detected: "contenedor Ogg sin firma de códec de vídeo (posible audio Vorbis/Opus)"
                .to_string(),
        });
    }

    // 9. RealMedia
    if header.starts_with(b".RMF") {
        return Ok(VideoContainerKind::RealMedia);
    }

    let detected = if header.len() < HEADER_INSPECT_MIN_BYTES {
        format!(
            "cabecera insuficiente ({} bytes < {HEADER_INSPECT_MIN_BYTES} bytes mínimos)",
            header.len()
        )
    } else {
        "sin firma de contenedor de vídeo reconocida".to_string()
    };
    Err(ContentPolicyError::InvalidVideoHeader {
        file_name: file_name.to_string(),
        detected,
    })
}

/// Lee los primeros [`FILE_HEADER_INSPECT_BYTES`] de un archivo en disco y valida tanto su nombre como su cabecera binaria.
pub fn validate_video_file(
    path: &Path,
    display_name: &str,
) -> Result<VideoContainerKind, ContentPolicyError> {
    validate_video_filename(display_name)?;

    let mut file = File::open(path).map_err(|e| ContentPolicyError::HeaderReadError {
        file_name: display_name.to_string(),
        reason: e.to_string(),
    })?;

    let mut buf = vec![0u8; FILE_HEADER_INSPECT_BYTES];
    let mut n = 0usize;
    while n < buf.len() {
        match file.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(read_bytes) => n += read_bytes,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(ContentPolicyError::HeaderReadError {
                    file_name: display_name.to_string(),
                    reason: e.to_string(),
                });
            }
        }
    }

    validate_video_header(&buf[..n], display_name)
}

/// Filtra los archivos de un torrent permitiendo descargar únicamente vídeos válidos y sus subtítulos complementarios.
///
/// Rechaza el torrent completo si no contiene al menos un archivo de vídeo permitido.
pub fn select_torrent_video_files<'a, I>(files: I) -> Result<TorrentFileSelection, ContentPolicyError>
where
    I: IntoIterator<Item = (usize, &'a str)>,
{
    let mut video_indices = Vec::new();
    let mut subtitle_indices = Vec::new();
    let mut skipped_count = 0usize;

    for (idx, path_str) in files {
        if is_allowed_video_filename(path_str) {
            video_indices.push(idx);
        } else if is_companion_subtitle_filename(path_str) {
            subtitle_indices.push(idx);
        } else {
            skipped_count += 1;
        }
    }

    if video_indices.is_empty() {
        return Err(ContentPolicyError::NoVideoFilesInTorrent);
    }

    let selected_set: BTreeSet<usize> = video_indices
        .iter()
        .copied()
        .chain(subtitle_indices.iter().copied())
        .collect();
    let selected_indices: Vec<usize> = selected_set.into_iter().collect();

    Ok(TorrentFileSelection {
        selected_indices,
        video_indices,
        subtitle_indices,
        skipped_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filename_whitelist_and_double_extensions() {
        for ext in ALLOWED_VIDEO_EXTENSIONS {
            let name = format!("Pelicula.Ejemplo.2026.1080p.{ext}");
            assert!(
                validate_video_filename(&name).is_ok(),
                "debe aceptarse extensión '{ext}'"
            );
            let upper = format!("Serie.S01E01.{}", ext.to_ascii_uppercase());
            assert!(validate_video_filename(&upper).is_ok());
        }

        // Rutas con dominios legítimos en escena P2P no deben bloquearse
        assert!(validate_video_filename("Pelicula.www.sitio.com.1080p.mkv").is_ok());
        assert!(validate_video_filename("/downloads/Series/Capitulo.S01E02.mp4").is_ok());
        assert!(validate_video_filename("C:\\incoming\\Documental.avi").is_ok());

        // Rechazos esperados
        assert!(matches!(
            validate_video_filename("   "),
            Err(ContentPolicyError::EmptyFileName)
        ));
        assert!(matches!(
            validate_video_filename("sin_extension"),
            Err(ContentPolicyError::MissingExtension { .. })
        ));
        for bad in [
            "instalador.exe",
            "archivo.zip",
            "coleccion.rar",
            "imagen.iso",
            "cancion.mp3",
            "libro.pdf",
            "subtitulo.srt",
            "caratula.jpg",
            "falsa.exe.mkv",
            "payload.bat.mp4",
            "archivo.zip.avi",
            "documento.pdf.webm",
        ] {
            assert!(
                validate_video_filename(bad).is_err(),
                "debía rechazarse '{bad}'"
            );
        }
    }

    #[test]
    fn test_companion_subtitles() {
        assert!(is_companion_subtitle_filename("Subs/Pelicula.es.srt"));
        assert!(is_companion_subtitle_filename("Pelicula.VTT"));
        assert!(is_companion_subtitle_filename("Episode.ass"));
        assert!(!is_companion_subtitle_filename("Subs/malicioso.exe.srt"));
        assert!(!is_companion_subtitle_filename("readme.txt"));
        assert!(!is_companion_subtitle_filename("Pelicula.mkv"));
    }

    #[test]
    fn test_valid_video_containers_headers() {
        assert_eq!(
            validate_video_header(MINIMAL_VALID_MKV_HEADER, "peli.mkv"),
            Ok(VideoContainerKind::Matroska)
        );

        let webm = b"\x1A\x45\xDF\xA3\x93\x42\x82\x84webm\x00\x00\x00\x00";
        assert_eq!(
            validate_video_header(webm, "clip.webm"),
            Ok(VideoContainerKind::WebM)
        );

        let mut mp4 = Vec::new();
        mp4.extend_from_slice(&24u32.to_be_bytes());
        mp4.extend_from_slice(b"ftypmp42");
        mp4.extend_from_slice(&0u32.to_be_bytes());
        mp4.extend_from_slice(b"isommp42");
        assert_eq!(
            validate_video_header(&mp4, "serie.mp4"),
            Ok(VideoContainerKind::IsoBmffVideo)
        );

        let qt_classic = b"\x00\x00\x00\x20moov\x00\x00\x00\x00\x00\x00\x00\x00";
        assert_eq!(
            validate_video_header(qt_classic, "viejo.mov"),
            Ok(VideoContainerKind::IsoBmffVideo)
        );

        let avi = b"RIFF\x24\x00\x00\x00AVI LIST\x00\x00\x00\x00";
        assert_eq!(
            validate_video_header(avi, "peli.avi"),
            Ok(VideoContainerKind::Avi)
        );

        let mut ts = vec![0u8; 512];
        ts[0] = 0x47;
        ts[188] = 0x47;
        ts[376] = 0x47;
        assert_eq!(
            validate_video_header(&ts, "emision.ts"),
            Ok(VideoContainerKind::MpegTs)
        );

        let mut m2ts = vec![0u8; 512];
        m2ts[4] = 0x47;
        m2ts[196] = 0x47;
        m2ts[388] = 0x47;
        assert_eq!(
            validate_video_header(&m2ts, "bluray.m2ts"),
            Ok(VideoContainerKind::MpegTs)
        );

        let mpeg_ps = b"\x00\x00\x01\xBA\x44\x00\x04\x00\x04\x01\x01\x89\xC3\xF8\x00\x00";
        assert_eq!(
            validate_video_header(mpeg_ps, "dvd.vob"),
            Ok(VideoContainerKind::MpegPs)
        );

        assert_eq!(
            validate_video_header(&ASF_HEADER_GUID, "video.wmv"),
            Ok(VideoContainerKind::AsfWmv)
        );

        let flv_video = b"FLV\x01\x05\x00\x00\x00\x09\x00\x00\x00\x00\x00\x00\x00";
        assert_eq!(
            validate_video_header(flv_video, "stream.flv"),
            Ok(VideoContainerKind::Flv)
        );

        let mut ogg_theora = b"OggS\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
        ogg_theora.extend_from_slice(b"\x80theora\x03\x02\x01");
        assert_eq!(
            validate_video_header(&ogg_theora, "clip.ogv"),
            Ok(VideoContainerKind::OggVideo)
        );

        let rmvb = b".RMF\x00\x00\x00\x12\x00\x00\x00\x00\x00\x00\x00\x04";
        assert_eq!(
            validate_video_header(rmvb, "archivo.rmvb"),
            Ok(VideoContainerKind::RealMedia)
        );
    }

    #[test]
    fn test_rejects_disguised_non_video_headers() {
        let exe = b"MZ\x90\x00\x03\x00\x00\x00\x04\x00\x00\x00\xFF\xFF\x00\x00";
        assert!(validate_video_header(exe, "falsa.mkv").is_err());

        let elf = b"\x7FELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        assert!(validate_video_header(elf, "falsa.mp4").is_err());

        let zip = b"PK\x03\x04\x14\x00\x00\x00\x08\x00\x00\x00\x00\x00\x00\x00";
        assert!(validate_video_header(zip, "falsa.mkv").is_err());

        let rar = b"Rar!\x1A\x07\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        assert!(validate_video_header(rar, "falsa.avi").is_err());

        let pdf = b"%PDF-1.7\n1 0 obj\n";
        assert!(validate_video_header(pdf, "libro.mp4").is_err());

        let jpg = b"\xFF\xD8\xFF\xE0\x00\x10JFIF\x00\x01\x01\x00\x00\x01";
        assert!(validate_video_header(jpg, "foto.mkv").is_err());

        let png = b"\x89PNG\r\n\x1A\n\x00\x00\x00\rIHDR";
        assert!(validate_video_header(png, "foto.mp4").is_err());

        let mp3 = b"ID3\x03\x00\x00\x00\x00\x00\x00TPE1\x00\x00";
        assert!(validate_video_header(mp3, "audio.mp4").is_err());

        let wav = b"RIFF\x24\x00\x00\x00WAVEfmt ";
        assert!(validate_video_header(wav, "audio.avi").is_err());

        // Audio M4A disfrazado como .mp4 con marcas compatibles genéricas
        let mut m4a = Vec::new();
        m4a.extend_from_slice(&28u32.to_be_bytes());
        m4a.extend_from_slice(b"ftypM4A ");
        m4a.extend_from_slice(&0u32.to_be_bytes());
        m4a.extend_from_slice(b"M4A mp42isom");
        assert!(validate_video_header(&m4a, "musica.mp4").is_err());

        // Imagen HEIC/AVIF disfrazada como .mp4
        let mut avif = Vec::new();
        avif.extend_from_slice(&24u32.to_be_bytes());
        avif.extend_from_slice(b"ftypavif");
        avif.extend_from_slice(&0u32.to_be_bytes());
        avif.extend_from_slice(b"avifmif1");
        assert!(validate_video_header(&avif, "imagen.mp4").is_err());

        // FLV solo audio (sin bit de vídeo 0x01)
        let flv_audio = b"FLV\x01\x04\x00\x00\x00\x09\x00\x00\x00\x00\x00\x00\x00";
        assert!(validate_video_header(flv_audio, "audio.flv").is_err());

        // Ogg Vorbis solo audio
        let ogg_vorbis = b"OggS\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00\x01vorbis";
        assert!(validate_video_header(ogg_vorbis, "audio.ogv").is_err());

        // EBML con DocType distinto
        let wrong_ebml = b"\x1A\x45\xDF\xA3\x93\x42\x82\x87unknown\x00";
        assert!(validate_video_header(wrong_ebml, "raro.mkv").is_err());

        // Texto ASCII con 'G' (0x47) repetidas no debe simular MPEG-TS
        let ascii_gs = vec![b'G'; 512];
        assert!(validate_video_header(&ascii_gs, "texto.ts").is_err());
    }

    #[test]
    fn test_torrent_file_selection() {
        let mixed = [
            (0usize, "Serie.S01/leeme.txt"),
            (1, "Serie.S01/setup.exe"),
            (2, "Serie.S01/S01E01.mkv"),
            (3, "Serie.S01/S01E01.es.srt"),
            (4, "Serie.S01/S01E02.mp4"),
            (5, "Serie.S01/poster.jpg"),
        ];
        let sel = select_torrent_video_files(mixed).expect("debe seleccionar vídeos y subtítulos");
        assert_eq!(sel.video_indices, vec![2, 4]);
        assert_eq!(sel.subtitle_indices, vec![3]);
        assert_eq!(sel.selected_indices, vec![2, 3, 4]);
        assert_eq!(sel.skipped_count, 3);

        let no_video = [(0usize, "software/app.exe"), (1, "software/manual.pdf")];
        assert_eq!(
            select_torrent_video_files(no_video),
            Err(ContentPolicyError::NoVideoFilesInTorrent)
        );
    }
}

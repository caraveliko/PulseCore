// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Representación y fusión de intervalos IPv4 para el filtro IP en memoria.

use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;
use std::sync::Arc;

/// Rango IPv4 compacto optimizado para consumo mínimo de RAM.
/// Ocupa únicamente 8 bytes por intervalo en memoria contigua (`start` + `end`),
/// permitiendo albergar cientos de miles de rangos con un consumo < 3 MiB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactRange {
    pub start: u32,
    pub end: u32,
}

impl CompactRange {
    #[inline]
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    #[inline]
    pub const fn contains(&self, ip: u32) -> bool {
        ip >= self.start && ip <= self.end
    }

    #[inline]
    pub fn start_ip(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.start)
    }

    #[inline]
    pub fn end_ip(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.end)
    }
}

/// Una dirección IPv4 representada como un rango inclusivo `[start, end]`.
/// Los valores IP se almacenan como `u32` en big-endian (orden de red).
///
/// La descripción utiliza `Arc<str>` para compartir cadenas idénticas entre
/// miles de reglas y evitar alocaciones repetidas en el heap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterRange {
    pub start: u32,
    pub end: u32,
    pub level: u32,
    pub description: Arc<str>,
}

impl FilterRange {
    pub fn new(start: Ipv4Addr, end: Ipv4Addr, level: u32, description: impl AsRef<str>) -> Self {
        let start_u32 = u32::from(start);
        let end_u32 = u32::from(end);
        let (s, e) = if start_u32 <= end_u32 {
            (start_u32, end_u32)
        } else {
            (end_u32, start_u32)
        };
        Self {
            start: s,
            end: e,
            level,
            description: Arc::from(description.as_ref()),
        }
    }

    #[inline]
    pub fn contains(&self, ip: u32) -> bool {
        ip >= self.start && ip <= self.end
    }

    #[inline]
    pub fn start_ip(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.start)
    }

    #[inline]
    pub fn end_ip(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.end)
    }
}

/// Fusiona intervalos ordenados donde sea posible.
/// Si dos rangos se solapan o son adyacentes, se unen en un único rango compacto.
pub fn coalesce_compact_ranges(mut ranges: Vec<CompactRange>) -> (Vec<CompactRange>, usize) {
    if ranges.len() <= 1 {
        return (ranges, 0);
    }

    ranges.sort_unstable_by(|a, b| a.start.cmp(&b.start).then_with(|| a.end.cmp(&b.end)));

    let mut merged: Vec<CompactRange> = Vec::with_capacity(ranges.len());
    let mut merged_count = 0usize;

    for r in ranges {
        if let Some(last) = merged.last_mut() {
            let is_adjacent = last.end < u32::MAX && (last.end + 1 >= r.start);
            let is_overlapping = r.start <= last.end;

            if is_adjacent || is_overlapping {
                if r.end > last.end {
                    last.end = r.end;
                }
                merged_count += 1;
            } else {
                merged.push(r);
            }
        } else {
            merged.push(r);
        }
    }

    merged.shrink_to_fit();
    (merged, merged_count)
}

/// Fusiona intervalos ordenados donde sea posible.
/// Si dos rangos se solapan o son adyacentes Y tienen el mismo nivel, se fusionan.
pub fn coalesce_ranges(mut ranges: Vec<FilterRange>) -> (Vec<FilterRange>, usize) {
    if ranges.len() <= 1 {
        return (ranges, 0);
    }

    // Ordenar por inicio ascendente, luego por fin ascendente
    ranges.sort_unstable_by(|a, b| a.start.cmp(&b.start).then_with(|| a.end.cmp(&b.end)));

    let mut merged: Vec<FilterRange> = Vec::with_capacity(ranges.len());
    let mut merged_count = 0usize;

    for r in ranges {
        if let Some(last) = merged.last_mut() {
            // Comprobar si el rango actual se solapa o es inmediatamente adyacente (last.end + 1 == r.start)
            let is_adjacent = last.end < u32::MAX && (last.end + 1 >= r.start);
            let is_overlapping = r.start <= last.end;

            if (is_adjacent || is_overlapping) && last.level == r.level {
                if r.end > last.end {
                    last.end = r.end;
                }
                if !r.description.is_empty() && !last.description.contains(r.description.as_ref()) {
                    let mut combined = String::with_capacity(last.description.len() + r.description.len() + 3);
                    if last.description.is_empty() {
                        combined.push_str(r.description.as_ref());
                    } else {
                        combined.push_str(last.description.as_ref());
                        combined.push_str(" | ");
                        combined.push_str(r.description.as_ref());
                    }
                    last.description = Arc::from(combined);
                }
                merged_count += 1;
            } else {
                merged.push(r);
            }
        } else {
            merged.push(r);
        }
    }

    merged.shrink_to_fit();
    (merged, merged_count)
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use std::collections::BTreeMap;
use std::ops::Range;

use crate::error::CoreError;

/// Conjunto canónico de intervalos semiabiertos `[start, end)`.
///
/// Invariantes:
/// - Cada rango no está vacío: `start < end`.
/// - Los rangos están ordenados por `start`.
/// - Ningún par de rangos se solapa o toca (los rangos adyacentes se fusionan automáticamente).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RangeSet {
    ranges: BTreeMap<u64, u64>,
}

impl RangeSet {
    pub fn new() -> Self {
        Self {
            ranges: BTreeMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    pub fn ranges(&self) -> Vec<Range<u64>> {
        self.ranges.iter().map(|(&s, &e)| s..e).collect()
    }

    /// Número total de bytes cubiertos en todos los intervalos.
    pub fn covered_len(&self) -> u64 {
        self.ranges.iter().map(|(&s, &e)| e - s).sum()
    }

    /// Inserta un rango semiabierto `[start, end)` en el conjunto, fusionando intervalos solapados y adyacentes.
    pub fn insert(&mut self, range: Range<u64>) -> Result<(), CoreError> {
        let (mut new_start, mut new_end) = (range.start, range.end);
        if new_start >= new_end {
            return Err(CoreError::InvalidRange {
                start: new_start,
                end: new_end,
                reason: "start must be strictly less than end",
            });
        }

        // Buscar rangos existentes que se solapen o toquen [new_start, new_end].
        // Un rango existente [s, e) se solapa o toca con [new_start, new_end] si:
        // s <= new_end y e >= new_start.
        let mut to_remove = Vec::new();

        // Comprobar el rango predecesor (si existe) que comienza antes de new_start
        if let Some((&s, &e)) = self.ranges.range(..=new_start).next_back() {
            if e >= new_start {
                new_start = s;
                new_end = new_end.max(e);
                to_remove.push(s);
            }
        }

        // Comprobar todos los rangos que comienzan entre new_start y new_end
        for (&s, &e) in self.ranges.range(new_start..=new_end) {
            new_end = new_end.max(e);
            to_remove.push(s);
        }

        for s in to_remove {
            self.ranges.remove(&s);
        }

        self.ranges.insert(new_start, new_end);
        Ok(())
    }

    /// Comprueba si el intervalo `[start, end)` completo está cubierto.
    pub fn contains(&self, range: Range<u64>) -> bool {
        if range.start >= range.end {
            return false;
        }

        if let Some((&s, &e)) = self.ranges.range(..=range.start).next_back() {
            s <= range.start && e >= range.end
        } else {
            false
        }
    }

    /// Comprueba si alguna parte de `[start, end)` interseca con el conjunto.
    pub fn intersects(&self, range: Range<u64>) -> bool {
        if range.start >= range.end {
            return false;
        }

        // Comprobar predecesor
        if let Some((_, &e)) = self.ranges.range(..=range.start).next_back() {
            if e > range.start {
                return true;
            }
        }

        // Comprobar cualquier rango que comience dentro del intervalo de consulta
        if let Some((&s, _)) = self.ranges.range(range.start..range.end).next() {
            if s < range.end {
                return true;
            }
        }

        false
    }

    /// Devuelve los huecos faltantes dentro de un intervalo delimitador `limit` dado.
    pub fn missing_within(&self, limit: Range<u64>) -> Vec<Range<u64>> {
        let (limit_start, limit_end) = (limit.start, limit.end);
        if limit_start >= limit_end {
            return Vec::new();
        }

        let mut gaps = Vec::new();
        let mut cursor = limit_start;

        for (&s, &e) in &self.ranges {
            if e <= cursor {
                continue;
            }
            if s >= limit_end {
                break;
            }

            if s > cursor {
                gaps.push(cursor..s.min(limit_end));
            }
            cursor = cursor.max(e);
            if cursor >= limit_end {
                break;
            }
        }

        if cursor < limit_end {
            gaps.push(cursor..limit_end);
        }

        gaps
    }

    /// Desaloja un rango `[start, end)` del conjunto (p. ej. al detectar corrupción).
    pub fn remove(&mut self, range: Range<u64>) -> Result<(), CoreError> {
        let (rem_start, rem_end) = (range.start, range.end);
        if rem_start >= rem_end {
            return Err(CoreError::InvalidRange {
                start: rem_start,
                end: rem_end,
                reason: "start must be strictly less than end",
            });
        }

        let mut to_remove = Vec::new();
        let mut to_insert = Vec::new();

        // Comprobar predecesor
        if let Some((&s, &e)) = self.ranges.range(..=rem_start).next_back() {
            if e > rem_start {
                to_remove.push(s);
                if s < rem_start {
                    to_insert.push((s, rem_start));
                }
                if e > rem_end {
                    to_insert.push((rem_end, e));
                }
            }
        }

        // Comprobar los rangos que comienzan dentro del rango eliminado
        for (&s, &e) in self.ranges.range(rem_start..rem_end) {
            to_remove.push(s);
            if e > rem_end {
                to_insert.push((rem_end, e));
            }
        }

        for s in to_remove {
            self.ranges.remove(&s);
        }
        for (s, e) in to_insert {
            self.ranges.insert(s, e);
        }

        Ok(())
    }
}

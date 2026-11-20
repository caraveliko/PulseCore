// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_core::{CoreError, RangeSet};

#[test]
fn test_single_insertion() {
    let mut set = RangeSet::new();
    set.insert(100..200).unwrap();

    assert_eq!(set.len(), 1);
    assert_eq!(set.covered_len(), 100);
    assert_eq!(set.ranges(), vec![100..200]);
    assert!(set.contains(100..200));
    assert!(set.contains(120..180));
    assert!(!set.contains(50..150));
}

#[test]
fn test_adjacent_merges() {
    let mut set = RangeSet::new();

    // 1. Insertar [100, 200) y [300, 400)
    set.insert(100..200).unwrap();
    set.insert(300..400).unwrap();
    assert_eq!(set.len(), 2);
    assert_eq!(set.ranges(), vec![100..200, 300..400]);

    // 2. Insertar [200, 300) -> fusión simultánea izquierda + derecha en [100, 400)
    set.insert(200..300).unwrap();
    assert_eq!(set.len(), 1);
    assert_eq!(set.ranges(), vec![100..400]);
    assert_eq!(set.covered_len(), 300);
}

#[test]
fn test_partial_and_total_overlaps() {
    let mut set = RangeSet::new();

    set.insert(100..250).unwrap();
    // Solapamiento parcial con el borde derecho
    set.insert(200..350).unwrap();
    assert_eq!(set.ranges(), vec![100..350]);

    // Solapamiento total interior
    set.insert(150..250).unwrap();
    assert_eq!(set.ranges(), vec![100..350]);

    // Solapamiento total envolvente
    set.insert(50..400).unwrap();
    assert_eq!(set.ranges(), vec![50..400]);
    assert_eq!(set.covered_len(), 350);
}

#[test]
fn test_out_of_order_insertions() {
    let mut set = RangeSet::new();

    set.insert(500..600).unwrap();
    set.insert(100..200).unwrap();
    set.insert(300..400).unwrap();

    assert_eq!(set.ranges(), vec![100..200, 300..400, 500..600]);
    assert_eq!(set.covered_len(), 300);
}

#[test]
#[allow(clippy::reversed_empty_ranges)]
fn test_invalid_ranges_rejected() {
    let mut set = RangeSet::new();

    // Rango vacío [x, x)
    assert!(matches!(
        set.insert(100..100).unwrap_err(),
        CoreError::InvalidRange { .. }
    ));

    // Rango invertido [start > end)
    assert!(matches!(
        set.insert(200..100).unwrap_err(),
        CoreError::InvalidRange { .. }
    ));
}

#[test]
fn test_missing_within_gaps() {
    let mut set = RangeSet::new();
    set.insert(100..200).unwrap();
    set.insert(300..400).unwrap();

    // Límite: 0..500
    let gaps = set.missing_within(0..500);
    assert_eq!(gaps, vec![0..100, 200..300, 400..500]);

    // Límite completamente dentro del rango cubierto: sin huecos
    assert!(set.missing_within(120..180).is_empty());

    // Límite completamente dentro de un hueco faltante
    assert_eq!(set.missing_within(220..280), vec![220..280]);
}

#[test]
fn test_range_removal() {
    let mut set = RangeSet::new();
    set.insert(100..500).unwrap();

    // Eliminar el centro [200, 300) -> se divide en [100, 200) y [300, 500)
    set.remove(200..300).unwrap();
    assert_eq!(set.ranges(), vec![100..200, 300..500]);
    assert_eq!(set.covered_len(), 300);

    // Eliminar el límite izquierdo [50, 150) -> [150, 200) y [300, 500)
    set.remove(50..150).unwrap();
    assert_eq!(set.ranges(), vec![150..200, 300..500]);

    // Eliminar todo [100, 600) -> conjunto vacío
    set.remove(100..600).unwrap();
    assert!(set.is_empty());
    assert_eq!(set.covered_len(), 0);
}

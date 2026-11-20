// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_core::RangeSet;
use proptest::prelude::*;
use rand::seq::SliceRandom;
use rand::SeedableRng;

proptest! {
    #[test]
    fn prop_range_set_permutation_invariance(
        // Generar entre 1 y 25 intervalos válidos disjuntos
        raw_intervals in prop::collection::vec((1u64..10_000, 1u64..5_000), 1..25),
        seed in any::<u64>(),
    ) {
        // Construir intervalos no solapados avanzando hacia adelante
        let mut intervals = Vec::new();
        let mut cursor = 0u64;
        for (gap, len) in raw_intervals {
            cursor += gap;
            let start = cursor;
            let end = cursor + len;
            cursor = end + 10; // asegurar hueco >= 10 para que no se toquen
            intervals.push(start..end);
        }

        // Conjunto A: insertar en el orden clasificado original
        let mut set_a = RangeSet::new();
        for r in &intervals {
            prop_assert!(set_a.insert(r.clone()).is_ok());
        }

        // Conjunto B: insertar en permutación barajada aleatoriamente
        let mut shuffled = intervals.clone();
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        shuffled.shuffle(&mut rng);

        let mut set_b = RangeSet::new();
        for r in shuffled {
            prop_assert!(set_b.insert(r).is_ok());
        }

        // Invariantes:
        // 1. Equivalencia canónica exacta de rangos
        prop_assert_eq!(set_a.ranges(), set_b.ranges());
        // 2. Equivalencia exacta de longitud cubierta
        prop_assert_eq!(set_a.covered_len(), set_b.covered_len());
        // 3. Cada intervalo insertado está contenido
        for r in &intervals {
            prop_assert!(set_a.contains(r.clone()));
            prop_assert!(set_b.contains(r.clone()));
        }
        // 4. Huecos faltantes idénticos
        let max_end = cursor + 1000;
        prop_assert_eq!(set_a.missing_within(0..max_end), set_b.missing_within(0..max_end));
    }

    #[test]
    fn prop_range_set_overlapping_insertions_grow_monotonically(
        base_start in 0u64..100_000,
        len1 in 10u64..1000,
        offset_shift in 0u64..500,
        len2 in 10u64..1000,
    ) {
        let mut set = RangeSet::new();
        let r1 = base_start..(base_start + len1);
        prop_assert!(set.insert(r1.clone()).is_ok());
        let covered_before = set.covered_len();

        let r2_start = base_start + offset_shift;
        let r2 = r2_start..(r2_start + len2);
        prop_assert!(set.insert(r2.clone()).is_ok());
        let covered_after = set.covered_len();

        // Añadir más rangos nunca debe disminuir la longitud cubierta
        prop_assert!(covered_after >= covered_before);
        prop_assert!(set.contains(r1));
        prop_assert!(set.contains(r2));
    }
}

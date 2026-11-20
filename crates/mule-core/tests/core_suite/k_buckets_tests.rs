// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_core::kad::{KBucketTable, KadContact, KadId, K_BUCKET_SIZE};
use mule_core::persistence::Database;
use std::net::Ipv4Addr;

#[test]
fn test_kad_id_xor_metric_and_distance() {
    let id_a = KadId([0u8; 16]);
    let mut bytes_b = [0u8; 16];
    bytes_b[3] = 0x80; // Difiere en el MSB (bit 31 del DWORD 0)
    let id_b = KadId(bytes_b);

    assert_eq!(id_a.xor_distance(&id_b), id_b);
    assert_eq!(id_b.leading_zeros(), 0);
    assert_eq!(id_a.bucket_index_for(&id_b), Some(127));

    let mut bytes_c = [0u8; 16];
    bytes_c[12] = 0x01; // Difiere en el LSB (bit 0 del DWORD 3)
    let id_c = KadId(bytes_c);
    assert_eq!(id_c.leading_zeros(), 127);
    assert_eq!(id_a.bucket_index_for(&id_c), Some(0));

    assert_eq!(id_a.bucket_index_for(&id_a), None); // Distancia a sí mismo es 0
}

#[test]
fn test_routing_table_insertion_and_capacity() {
    let local_id = KadId([0u8; 16]);
    let mut table = KBucketTable::new(local_id);

    // Insertar 20 contactos en la cubeta 127 (usando distintas subredes /24)
    for i in 0..K_BUCKET_SIZE {
        let mut b = [0u8; 16];
        b[3] = 0x80;
        b[12] = (i + 1) as u8;
        let c = KadContact::new(
            KadId(b),
            Ipv4Addr::new(10, 0, (i + 1) as u8, 1),
            39737,
            None,
            2,
            1000,
        );
        let evict_candidate = table.insert_or_update(c);
        assert!(evict_candidate.is_none());
    }

    assert_eq!(table.total_contacts(), K_BUCKET_SIZE);
    assert_eq!(table.total_replacements(), 0);

    // El contacto 21 en la cubeta 127 -> va a la caché de reemplazo, devuelve el candidato de cabeza
    let mut b21 = [0u8; 16];
    b21[3] = 0x80;
    b21[12] = 255;
    let c21 = KadContact::new(
        KadId(b21),
        Ipv4Addr::new(10, 0, 21, 1),
        39737,
        None,
        2,
        1000,
    );
    let evict_candidate = table.insert_or_update(c21);
    assert!(evict_candidate.is_some());
    assert_eq!(table.total_contacts(), K_BUCKET_SIZE);
    assert_eq!(table.total_replacements(), 1);

    // Eliminar la cabeza de la cubeta debería promocionar el reemplazo
    let head_id = evict_candidate.unwrap().kad_id;
    let removed = table.remove(&head_id);
    assert!(removed.is_some());
    assert_eq!(table.total_contacts(), K_BUCKET_SIZE);
    assert_eq!(table.total_replacements(), 0);
    assert!(table.get(&KadId(b21)).is_some());
}

#[test]
fn test_routing_table_subnet_sybil_protection() {
    let local_id = KadId([0u8; 16]);
    let mut table = KBucketTable::new(local_id);

    // Insertar 2 contactos de la misma subred /24 (192.168.1.0/24)
    for i in 0..2 {
        let mut b = [0u8; 16];
        b[3] = 0x80;
        b[12] = (i + 1) as u8;
        let c = KadContact::new(
            KadId(b),
            Ipv4Addr::new(192, 168, 1, (i + 1) as u8),
            39737,
            None,
            2,
            1000,
        );
        let evict = table.insert_or_update(c);
        assert!(evict.is_none());
    }
    assert_eq!(table.total_contacts(), 2);

    // El 3.er contacto de la misma subred /24 debe ser rechazado por la protección anti-Sybil
    let mut b3 = [0u8; 16];
    b3[3] = 0x80;
    b3[12] = 3;
    let c3 = KadContact::new(
        KadId(b3),
        Ipv4Addr::new(192, 168, 1, 3),
        39737,
        None,
        2,
        1000,
    );
    let evict3 = table.insert_or_update(c3);
    assert!(evict3.is_none());
    assert_eq!(
        table.total_contacts(),
        2,
        "3rd contact from same /24 subnet must be rejected"
    );
}

#[test]
fn test_sqlite_kad_contacts_persistence() {
    let mut db = Database::open_in_memory().unwrap();

    let c1 = KadContact::new(
        KadId([1u8; 16]),
        Ipv4Addr::new(1, 2, 3, 4),
        39737,
        Some(39735),
        2,
        1000,
    );
    let c2 = KadContact::new(
        KadId([2u8; 16]),
        Ipv4Addr::new(5, 6, 7, 8),
        1234,
        None,
        1,
        2000,
    );

    db.batch_upsert_kad_contacts(&[c1.clone(), c2.clone()])
        .unwrap();
    assert_eq!(db.count_kad_contacts().unwrap(), 2);

    let list = db.list_kad_contacts(10).unwrap();
    assert_eq!(list.len(), 2);
    // Ordenado por last_seen DESC -> c2 primero
    assert_eq!(list[0].kad_id, KadId([2u8; 16]));
    assert_eq!(list[1].kad_id, KadId([1u8; 16]));
    assert_eq!(list[1].tcp_port, Some(39735));

    db.clear_kad_contacts().unwrap();
    assert_eq!(db.count_kad_contacts().unwrap(), 0);
}

#[test]
fn test_find_closest_verified_balances_far_verified_and_close_unverified() {
    let local_id = KadId([0u8; 16]);
    let target = KadId([0x10; 16]);
    let mut table = KBucketTable::new(local_id);

    // Insertar 8 contactos verificados pero lejanos respecto a `target`
    for i in 0..8u8 {
        let mut id = [0xFF; 16];
        id[15] = i;
        let mut c = KadContact::new(KadId(id), Ipv4Addr::new(10, 1, i, 1), 4672, None, 2, 1000);
        c.verified = true;
        table.insert_or_update(c);
    }

    // Insertar 4 contactos no verificados muy cercanos a `target`
    let mut close_ids = Vec::new();
    for i in 1..=4u8 {
        let mut id = [0x10; 16];
        id[15] = 0x10 ^ i;
        close_ids.push(KadId(id));
        let c = KadContact::new(KadId(id), Ipv4Addr::new(10, 2, i, 1), 4672, None, 2, 1000);
        assert!(!c.verified);
        table.insert_or_update(c);
    }

    // Con count = 8: hasta 4 verificados lejanos + los 4 contactos más cercanos sin verificar
    let picked = table.find_closest_verified(&target, 8);
    assert_eq!(picked.len(), 8);
    for close in &close_ids {
        assert!(
            picked.iter().any(|c| &c.kad_id == close),
            "el contacto cercano no verificado debe incluirse como semilla"
        );
    }
    // El resultado debe quedar ordenado estrictamente por distancia XOR a `target`
    for win in picked.windows(2) {
        assert!(win[0].kad_id.xor_distance(&target) <= win[1].kad_id.xor_distance(&target));
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Pruebas para la votación de confianza de raíz AICH y la recuperación a nivel de bloque.

use mule_core::AllocationPolicy;
use mule_daemon::services::downloads::operations::AichVoteOutcome;
use mule_daemon::services::downloads::DownloadService;
use tempfile::tempdir;

const BLOCK: usize = 184_320;

fn build_file(blocks: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(blocks * BLOCK);
    for i in 0..blocks {
        for j in 0..BLOCK {
            data.push((i * 31 + j % 11) as u8);
        }
    }
    data
}

#[test]
fn aich_root_reaches_consensus_with_ten_unique_ips() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let link = format!(
        "ed2k://|file|aich.mkv|{}|{}|/",
        500_000,
        hex::encode([0xAAu8; 16])
    );
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();

    let root = [0x42u8; 20];
    for ip in 1..=9u32 {
        let outcome = svc.record_aich_root_vote(&snap.id, root, ip << 16);
        assert_eq!(outcome, AichVoteOutcome::Untrusted);
        assert_eq!(svc.trusted_aich_root(&snap.id), None);
    }
    // La décima IP enmascarada única alcanza el consenso.
    let outcome = svc.record_aich_root_vote(&snap.id, root, 10 << 16);
    assert_eq!(outcome, AichVoteOutcome::Trusted);
    assert_eq!(svc.trusted_aich_root(&snap.id), Some(root));

    // Un informe conflictivo se rechaza una vez que se confía en la raíz.
    let outcome = svc.record_aich_root_vote(&snap.id, [0xEE; 20], 11 << 16);
    assert_eq!(outcome, AichVoteOutcome::Mismatch);
}

#[test]
fn aich_root_from_link_is_immediately_trusted() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let root = [0x77u8; 20];
    let link = format!(
        "ed2k://|file|aich.mkv|{}|{}|h={}|/",
        500_000,
        hex::encode([0xAAu8; 16]),
        data_encoding::BASE32_NOPAD.encode(&root)
    );
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();
    assert_eq!(svc.trusted_aich_root(&snap.id), Some(root));
}

#[test]
fn aich_recovery_evicts_only_corrupted_blocks() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    // 3 bloques AICH (553 KB) en una parte eD2K.
    let good = build_file(3);
    let tree = mule_format::build_aich_tree(&good).unwrap();
    let root = tree.root_hash().unwrap();

    let link = format!(
        "ed2k://|file|aich.mkv|{}|{}|h={}|/",
        good.len(),
        hex::encode([0xAAu8; 16]),
        data_encoding::BASE32_NOPAD.encode(&root.0)
    );
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();

    // Inyectar la parte completa con el bloque 1 dañado.
    let mut received = good.clone();
    received[BLOCK + 100] ^= 0xFF;
    svc.inject_range(&snap.id, 0, &received, None).unwrap();
    assert_eq!(svc.get_covered_len(&snap.id), Some(good.len() as u64));

    // Construir datos de recuperación como lo haría quien sube, luego aplicarlos.
    let entries = mule_format::create_part_recovery_data(&tree, good.len() as u64, 0).unwrap();
    let report = svc
        .apply_aich_recovery(&snap.id, 0, root.0, entries)
        .unwrap();

    assert_eq!(report.corrupted_blocks, 1);
    assert_eq!(
        report.evicted_ranges,
        vec![(BLOCK as u64, (2 * BLOCK) as u64)]
    );
    // Los bloques 0 y 2 permanecen cubiertos.
    assert_eq!(
        svc.get_covered_len(&snap.id),
        Some(good.len() as u64 - BLOCK as u64)
    );
}

#[test]
fn aich_recovery_marks_clean_part_verified() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let good = build_file(3);
    let tree = mule_format::build_aich_tree(&good).unwrap();
    let root = tree.root_hash().unwrap();

    let link = format!(
        "ed2k://|file|aich.mkv|{}|{}|h={}|/",
        good.len(),
        hex::encode([0xAAu8; 16]),
        data_encoding::BASE32_NOPAD.encode(&root.0)
    );
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();

    svc.inject_range(&snap.id, 0, &good, None).unwrap();

    let entries = mule_format::create_part_recovery_data(&tree, good.len() as u64, 0).unwrap();
    let report = svc
        .apply_aich_recovery(&snap.id, 0, root.0, entries)
        .unwrap();

    assert_eq!(report.corrupted_blocks, 0);
    assert_eq!(svc.db_ref().get_verified_parts(&snap.id).unwrap(), vec![0]);
}

#[test]
fn aich_recovery_rejects_untrusted_master_hash() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let good = build_file(3);
    let tree = mule_format::build_aich_tree(&good).unwrap();
    let root = tree.root_hash().unwrap();

    let link = format!(
        "ed2k://|file|aich.mkv|{}|{}|h={}|/",
        good.len(),
        hex::encode([0xAAu8; 16]),
        data_encoding::BASE32_NOPAD.encode(&root.0)
    );
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();
    svc.inject_range(&snap.id, 0, &good, None).unwrap();

    // Los datos de recuperación con un hash maestro diferente no deben sobrescribir la
    // raíz de confianza ni alterar los rangos.
    let entries = mule_format::create_part_recovery_data(&tree, good.len() as u64, 0).unwrap();
    assert!(svc
        .apply_aich_recovery(&snap.id, 0, [0xEE; 20], entries)
        .is_err());
    assert_eq!(svc.get_covered_len(&snap.id), Some(good.len() as u64));
    assert_eq!(svc.trusted_aich_root(&snap.id), Some(root.0));
}

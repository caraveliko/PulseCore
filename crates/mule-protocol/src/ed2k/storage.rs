// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Almacenamiento persistente, ficheros .part dispersos y estado de reanudación resiliente ante fallos para transferencias eD2K.
//!
//! El nivel 3.5 (Tier 3.5) formaliza la persistencia física de ficheros y los invariantes de reanudación:
//! - Preasignación de tamaño exacto / representación dispersa en disco del fichero `.part`.
//! - Registro de estado complementario `.resume.json` que guarda los intervalos de bytes completados y el total de bytes recibidos.
//! - Persistencia atómica del estado mediante fichero temporal (`.tmp`) y renombrado para garantizar consistencia ante fallos.
//! - Reanudación desde estado parcial en disco: recarga `.resume.json` y verifica la coherencia del hash y del tamaño del fichero.
//! - Escrituras incrementales de fragmentos en el fichero `.part` con protección estricta contra duplicados y solapamientos.
//! - Verificación de integridad de partes en disco: transmite los bytes de la parte completada directamente al calculador MD4 local
//!   para comprobar la suma de verificación de la parte frente al valor esperado sin necesidad de mantener toda la parte en RAM.

pub mod sink;
pub mod types;

pub use sink::Ed2kPartFileSink;
pub use types::{merge_intervals, Ed2kBlockWriter, Ed2kPartDownloadStatus, Ed2kPartResumeState};

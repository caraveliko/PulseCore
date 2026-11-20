// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Módulo de orquestación de referencia externa de nivel 3 (Tier 3).
//!
//! Expone la gestión del entorno, la validación de binarios, la protección de procesos y la generación de informes de divergencias
//! para ejecutar binarios externos fijados frente a eMuleRust.

pub mod ed2k_reference_interop_tests;
pub mod environment;
pub mod harness;
pub mod report;

#[allow(unused_imports)]
pub use environment::{ReferenceKind, Tier3Environment};
#[allow(unused_imports)]
pub use harness::{Tier3BinaryValidator, Tier3Harness, Tier3HarnessError, Tier3ProcessGuard};
#[allow(unused_imports)]
pub use report::{DivergenceEntry, ObservedTagReport, Tier3DivergenceReport, WireTraceRecord};

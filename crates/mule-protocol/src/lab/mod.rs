// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Protocolo entre pares en bucle local (loopback) de laboratorio para una validación controlada y segura de transferencias.
//!
//! Nota: este módulo está estrictamente separado de los formatos de red eD2K históricos para evitar
//! afirmaciones prematuras de interoperación con la red externa.

pub mod in_flight;
pub mod messages;
pub mod source_engine;
pub mod state_machine;

pub use in_flight::{InFlightError, InFlightTracker};
pub use messages::{LabMessage, LabRejectReason, LAB_OP_HELLO, LAB_OP_HELLO_ACK};
pub use source_engine::{HostedFile, LabSourceEngine};
pub use state_machine::{PeerAction, PeerFailure, PeerState, PeerStateMachine, ProtocolViolation};

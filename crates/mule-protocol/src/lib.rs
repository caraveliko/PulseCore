// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! `mule-protocol` proporciona entramado, codificación, decodificación,
//! constantes y rasgos de planificación desacoplados, deterministas y sin `unsafe`, para el protocolo de red eD2K en eMuleRust.

pub mod ed2k;
pub mod error;
pub mod framing;
pub mod kad;
#[cfg(feature = "lab-loopback")]
pub mod lab;
pub mod limits;
pub mod traits;
pub mod types;

pub use error::{FramingError, ProtocolError};
pub use framing::{PacketCodec, RawPacket};
pub use kad::*;
#[cfg(feature = "lab-loopback")]
pub use lab::*;
pub use limits::*;
pub use traits::*;
pub use types::*;

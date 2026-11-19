// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Primitivas criptográficas para eMuleRust.
//!
//! Reglas de límites:
//! - Este crate posee toda la lógica de generación de claves, firma y verificación.
//! - No realiza ninguna E/S de red ni sabe nada sobre el estado de sesión eD2K.
//! - La SUI heredada de eMule (RSA-384, RSASSA-PKCS1v15-SHA1) es únicamente un
//!   mecanismo de compatibilidad. Nunca debe usarse para cifrar cargas útiles ni
//!   proteger identidades modernas. Véase `docs/ed2k/SUI_WIRE_FORMAT.md`.

pub mod cryptkey;
pub mod legacy_sui;
pub mod merkle_blake3;
pub mod modern_identity;
pub mod pc26_handshake;

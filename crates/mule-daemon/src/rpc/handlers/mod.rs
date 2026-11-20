// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

pub mod categories;
pub mod downloads;
pub mod friends;
pub mod history;
pub mod kad;
pub mod lab;
pub mod media;
pub mod network;
pub mod notifications;
pub mod servers;
pub mod settings;
pub mod shares;
pub mod sources;
pub mod upnp;
pub mod updater;
pub mod updater_upload;

/// Serializa la carga útil de un manejador en `serde_json::Value`.
/// Las cargas útiles de RPC son `Serialize` por construcción; en caso de fallo
/// (inalcanzable en la práctica), se devuelve un objeto vacío en lugar de un error.
pub(crate) fn json_value<T: serde::Serialize + ?Sized>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or_default()
}

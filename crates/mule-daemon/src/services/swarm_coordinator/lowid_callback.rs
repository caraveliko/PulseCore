// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Alcanzabilidad de devolución de llamada LowID — `CemuleApp::CanDoCallback` de eMule
//! (`Emule.cpp:1067`) y `CServerConnect::IsLocalServer`
//! (`ServerConnect.cpp:519`).
//!
//! Los LowID solo son únicos por servidor: `OP_CALLBACKREQUEST` llega a una
//! fuente `lowid:<id>` únicamente cuando reside en nuestro servidor conectado.
//! Cuando no podemos recibir ninguna devolución de conexión, la fuente permanece
//! aparcada (`DS_LOWTOLOWIP` de eMule) hasta que cambie nuestro estado de ID;
//! cuando sí podemos pero no aplica ninguna opción de callback, eMule la descarta
//! (`Disconnected("LowID: No Callback Option available")`).

use mule_core::model::{PeerSource, PeerSourceOrigin};

/// Contexto de alcanzabilidad de devolución de llamada para un ciclo de planificación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CallbackCtx {
    /// La sesión de servidor eD2K está conectada (`client_id != 0` asignado).
    pub server_connected: bool,
    /// El servidor nos asignó un HighID — TCP entrante nos alcanza
    /// (`IsLowID() == false` de eMule).
    pub server_high_id: bool,
    /// IPv4 del servidor conectado (LE u32) — `cur_server->GetIP()`.
    pub our_server_ip: u32,
    /// Puerto TCP del servidor conectado — `cur_server->GetPort()`.
    pub our_server_port: u16,
    /// UDP de Kad está en ejecución *y* verificado como alcanzable
    /// (`CKademlia::IsConnected() && !IsFirewalled()`): el par puede entonces
    /// conectarse de vuelta a nuestro puerto TCP incluso cuando nuestro ID eD2K es LowID.
    pub kad_open: bool,
}

/// `CServerConnect::IsLocalServer` de eMule — la fuente reside en nuestro
/// servidor conectado, por lo que `OP_CALLBACKREQUEST` apunta al espacio de
/// nombres LowID correcto.
///
/// `server_ip == 0` en la fuente indica que el contexto del servidor se desconoce:
/// solo las fuentes con origen servidor cuentan como locales en ese caso (fueron devueltas por
/// el `OP_FOUNDSOURCES` de nuestro propio servidor). `server_port == 0` = puerto desconocido
/// (respondedores UDP `OP_GLOBFOUNDSOURCES`) → coincidencia solo por IP.
pub(crate) fn is_local_server(src: &PeerSource, ctx: &CallbackCtx) -> bool {
    if !ctx.server_connected {
        return false;
    }
    if src.server_ip == 0 {
        return src.origin == PeerSourceOrigin::Server;
    }
    src.server_ip == ctx.our_server_ip
        && (src.server_port == 0 || src.server_port == ctx.our_server_port)
}

/// `CemuleApp::CanDoCallback` de eMule — ¿se le puede pedir a la fuente LowID que
/// devuelva la conexión hacia nosotros?
///
/// - Sin Kad utilizable → solo una conexión eD2K HighID nos hace alcanzables.
/// - Kad conectado y sin cortafuegos → las devoluciones de llamada son posibles; pero cuando
///   *nosotros* somos LowID en el mismo servidor que la fuente, la devolución de llamada
///   se retransmitiría hacia nosotros mismos — eMule lo rechaza (violación de protocolo, riesgo de baneo).
pub(crate) fn can_do_callback(src: &PeerSource, ctx: &CallbackCtx) -> bool {
    if !ctx.kad_open {
        return ctx.server_high_id;
    }
    let we_lowid = ctx.server_connected && !ctx.server_high_id;
    if we_lowid {
        !is_local_server(src, ctx)
    } else {
        true
    }
}

/// Indica si una fuente `lowid:` tiene una ruta viable de callback de servidor — opción
/// `IsLocalServer` de `TryToConnect` de eMule (`BaseClient.cpp:1429`). Las
/// opciones restantes de eMule (devolución directa por UDP, compañero Kad) no las
/// portan las fuentes `lowid:` (las fuentes `kadcb:` portan el compañero en su lugar).
pub(crate) fn has_server_callback_option(src: &PeerSource, ctx: &CallbackCtx) -> bool {
    is_local_server(src, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mule_core::model::PeerSourceStatus;

    fn ctx(server_connected: bool, high: bool, kad_open: bool) -> CallbackCtx {
        CallbackCtx {
            server_connected,
            server_high_id: high,
            our_server_ip: u32::from_le_bytes([1, 2, 3, 4]),
            our_server_port: 49661,
            kad_open,
        }
    }

    fn lowid_src(server_ip: u32, server_port: u16, origin: PeerSourceOrigin) -> PeerSource {
        PeerSource {
            file_hash: "00".repeat(16),
            address: "lowid:42".to_string(),
            ip: 42,
            port: 0,
            origin,
            discovered_at_unix_ms: 0,
            last_seen_unix_ms: 0,
            status: PeerSourceStatus::Candidate,
            fail_count: 0,
            last_error: None,
            cooldown_until_unix_ms: None,
            user_hash: None,
            crypt_options: 0,
            buddy_id: None,
            software: None,
            availability: None,
            server_ip,
            server_port,
            udp_port: 0,
        }
    }

    const OUR_IP: u32 = u32::from_le_bytes([1, 2, 3, 4]);

    #[test]
    fn local_server_matching() {
        let c = ctx(true, true, false);
        // Las fuentes de origen servidor sin campos de servidor explícitos son locales.
        assert!(is_local_server(
            &lowid_src(0, 0, PeerSourceOrigin::Server),
            &c
        ));
        // Fuentes SX: coincidencia exacta de extremo o solo por IP (puerto desconocido).
        let sx = PeerSourceOrigin::SourceExchange;
        assert!(is_local_server(&lowid_src(OUR_IP, 49661, sx), &c));
        assert!(is_local_server(&lowid_src(OUR_IP, 0, sx), &c));
        assert!(!is_local_server(&lowid_src(OUR_IP, 49662, sx), &c));
        assert!(!is_local_server(&lowid_src(0x0A0B0C0D, 0, sx), &c));
        // No conectado → nada es local.
        assert!(!is_local_server(
            &lowid_src(0, 0, PeerSourceOrigin::Server),
            &ctx(false, false, false)
        ));
        // SX sin campos de servidor NO es local.
        assert!(!is_local_server(&lowid_src(0, 0, sx), &c));
    }

    #[test]
    fn can_do_callback_matrix() {
        let sx = PeerSourceOrigin::SourceExchange;
        let local = lowid_src(OUR_IP, 49661, sx);
        let foreign = lowid_src(0x0A0B0C0D, 49661, sx);

        // HighID → devolución de llamada posible para cualquier LowID.
        let high = ctx(true, true, false);
        assert!(can_do_callback(&local, &high));
        assert!(can_do_callback(&foreign, &high));

        // LowID + sin Kad → nadie puede alcanzarnos.
        let low_nokad = ctx(true, false, false);
        assert!(!can_do_callback(&local, &low_nokad));
        assert!(!can_do_callback(&foreign, &low_nokad));

        // LowID + Kad abierto → solo lowids foráneas (mismo servidor está baneado).
        let low_kad = ctx(true, false, true);
        assert!(!can_do_callback(&local, &low_kad));
        assert!(can_do_callback(&foreign, &low_kad));

        // Sin servidor + Kad abierto → todo es foráneo → permitido.
        let kad_only = ctx(false, false, true);
        assert!(can_do_callback(&local, &kad_only));
        assert!(can_do_callback(&foreign, &kad_only));
    }
}

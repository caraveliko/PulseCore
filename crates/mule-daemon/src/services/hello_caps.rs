//! Construcción de la palabra de capacidades `CT_EMULE_MISCOPTIONS2` — fuente única
//! para los hellos eD2K (entrante, saliente, devolución de conexión).
//!
//! Disposición en eMule `BaseClient.cpp:1011`:
//! `FileIdentifiers<<13 | DirectUDPCallback<<12 | SupportsCaptcha<<11 |
//!  SourceEx2<<10 | RequiresCrypt<<9 | RequestsCrypt<<8 | SupportsCrypt<<7 |
//!  reserved<<6 | ExtMultiPacket<<5 | LargeFiles<<4 | KadVersion<<0`.
//!
//! El bit 11 (CAPTCHA de chat) permanece a 0: la respuesta CAPTCHA no está implementada.

/// Construye la palabra `CT_EMULE_MISCOPTIONS2` de un hello de eMule.
///
/// - `tcp_low_id`: verdadero cuando la identidad eD2K actual es un LowID
///   (`client_id < 0x0100_0000`), proxy local de `theApp.IsFirewalled()` /
///   `CKademlia::IsFirewalled()` del lado TCP.
/// - `udp_verified_open`: verdadero cuando el comprobador de cortafuegos UDP Kad ha verificado
///   que el UDP entrante es accesible (`verified && status == Open` —
///   equivalente a `!IsFirewalledUDP(true) && IsVerified()`, NAT full-cone).
///   Combinado con LowID, arma el bit 12 «DirectUDPCallback» como
///   `GetMyConnectOptions(true, true)`.
pub(crate) fn misc_options2(
    tcp_low_id: bool,
    udp_verified_open: bool,
    obfuscation_enabled: bool,
    obfuscation_required: bool,
) -> u32 {
    let u_direct_udp_cb = (tcp_low_id && udp_verified_open) as u32;
    (1 << 13) // FileIdentifiers / MultipacketExt2 — codecs implémentés
        | (u_direct_udp_cb << 12)
        | (1 << 10) // SourceExchange2
        | ((obfuscation_required as u32) << 9)
        | ((obfuscation_enabled as u32) << 8)
        | ((obfuscation_enabled as u32) << 7)
        | (1 << 5) // ExtMultiPacket
        | (1 << 4) // LargeFiles / tags 64 bits
        | crate::services::kad::firewall::OUR_KAD_VERSION as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_bits_always_present() {
        let m = misc_options2(false, false, false, false);
        assert_eq!(m & (1 << 13), 1 << 13, "FileIdent");
        assert_eq!(m & (1 << 10), 1 << 10, "SrcExch2");
        assert_eq!(m & (1 << 5), 1 << 5, "ExtMultiPacket");
        assert_eq!(m & (1 << 4), 1 << 4, "LargeFiles");
        assert_eq!(
            m & 0x0F,
            crate::services::kad::firewall::OUR_KAD_VERSION as u32,
            "KadVersion"
        );
        assert_eq!(m & (1 << 11), 0, "Captcha non implémenté");
        assert_eq!(m & (1 << 12), 0, "pas de callback direct en HighID");
    }

    #[test]
    fn direct_callback_requires_lowid_and_verified_udp() {
        // TCP inaccesible + UDP verificado abierto → bit 12 (GetMyConnectOptions de eMule).
        assert_ne!(
            misc_options2(true, true, false, false) & (1 << 12),
            0
        );
        // UDP no verificado o cerrado → sin bit 12.
        assert_eq!(misc_options2(true, false, false, false) & (1 << 12), 0);
        // UDP abierto pero TCP accesible → sin bit 12.
        assert_eq!(misc_options2(false, true, false, false) & (1 << 12), 0);
    }

    #[test]
    fn crypt_bits_follow_obfuscation_flags() {
        let m = misc_options2(false, false, true, true);
        assert_ne!(m & (1 << 7), 0, "SupportsCryptLayer");
        assert_ne!(m & (1 << 8), 0, "RequestsCryptLayer");
        assert_ne!(m & (1 << 9), 0, "RequiresCryptLayer");
    }
}

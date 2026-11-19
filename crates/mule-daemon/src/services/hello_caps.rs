// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Construcción unificada de etiquetas y capacidades para `OP_HELLO` / `OP_HELLO_ANSWER`.
//!
//! Actúa como fuente única de verdad para saludos entrantes, salientes y callbacks,
//! unificando la versión anunciada (eMule 0.72a) y las capacidades modernas `PC26`.

use mule_protocol::ed2k::pc26::{
    build_pc26_hello_tags, Pc26PeerCaps, PC26_CAP_BLAKE3_MERKLE, PC26_CAP_ED25519_SUI_V3,
    PC26_CAP_HYBRID_MLKEM768, PC26_CAP_IPV6, PC26_CAP_QUIC, PC26_CAP_UDP_HOLE_PUNCH,
    PC26_CAP_X25519_CHACHA, PC26_CAP_ZSTD,
};
use mule_protocol::ed2k::tags::{
    Ed2kTag, TagName, TagValue, TAG_SPECIAL_CT_EMULE_VERSION, TAG_SPECIAL_EMULE_MISCOPTIONS1,
    TAG_SPECIAL_EMULE_MISCOPTIONS2, TAG_SPECIAL_EMULE_UDPPORTS, TAG_SPECIAL_NAME,
    TAG_SPECIAL_VERSION,
};

/// Versión base del protocolo eD2K anunciada en `CT_VERSION` (`0x3C`, eMule estándar).
pub(crate) const ED2K_CLIENT_VERSION: u32 = 0x3C;
/// Versión de eMule anunciada en `CT_EMULE_VERSION` — `(major<<17)|(minor<<10)|(update<<7)` = eMule 0.72a.
pub(crate) const EMULE_VERSION_TAG: u32 = 72 << 10;
/// `m_uCurVersionShort` para `OP_EMULEINFO`/`OP_EMULEINFOANSWER` (0.72a → 72).
pub(crate) const EMULE_VERSION_SHORT: u8 = 72;

/// Parámetros de entrada para construir un conjunto completo de etiquetas de saludo eD2K.
pub(crate) struct HelloBuildParams<'a> {
    pub nickname: &'a str,
    pub tcp_port: u16,
    pub kad_port: u16,
    pub sec_ident_cap: u8,
    pub tcp_low_id: bool,
    pub udp_verified_open: bool,
    pub obfuscation_enabled: bool,
    pub obfuscation_required: bool,
    pub pc26_caps: Option<&'a Pc26PeerCaps>,
}

/// Construye la palabra `CT_EMULE_MISCOPTIONS2` de un hello de eMule.
///
/// Disposición en eMule `BaseClient.cpp:1011`:
/// `FileIdentifiers<<13 | DirectUDPCallback<<12 | SupportsCaptcha<<11 |
///  SourceEx2<<10 | RequiresCrypt<<9 | RequestsCrypt<<8 | SupportsCrypt<<7 |
///  reserved<<6 | ExtMultiPacket<<5 | LargeFiles<<4 | KadVersion<<0`.
pub(crate) fn misc_options2(
    tcp_low_id: bool,
    udp_verified_open: bool,
    obfuscation_enabled: bool,
    obfuscation_required: bool,
) -> u32 {
    let u_direct_udp_cb = (tcp_low_id && udp_verified_open) as u32;
    (1 << 13)
        | (u_direct_udp_cb << 12)
        | (1 << 10)
        | ((obfuscation_required as u32) << 9)
        | ((obfuscation_enabled as u32) << 8)
        | ((obfuscation_enabled as u32) << 7)
        | (1 << 5)
        | (1 << 4)
        | crate::services::kad::firewall::OUR_KAD_VERSION as u32
}

/// Construye la palabra `CT_EMULE_MISCOPTIONS1` estándar de eMule 0.72a.
pub(crate) fn misc_options1(sec_ident_cap: u8) -> u32 {
    (1 << 29)
        | (1 << 28)
        | (4 << 24)
        | (1 << 20)
        | ((u32::from(sec_ident_cap) & 0x0F) << 16)
        | (4 << 12)
        | (2 << 8)
        | (1 << 4)
        | (1 << 1)
}

/// Construye las etiquetas completas de `OP_HELLO` / `OP_HELLO_ANSWER`
/// (6 etiquetas estándar de eMule 0.72a más las etiquetas TLV `pc26_*` cuando están activas).
pub(crate) fn build_hello_tags(params: &HelloBuildParams<'_>) -> Vec<Ed2kTag> {
    let mut tags = Vec::with_capacity(10);
    let nickname = if params.nickname.trim().is_empty() {
        "eMuleRust"
    } else {
        params.nickname
    };

    tags.push(Ed2kTag {
        name: TagName::Id(TAG_SPECIAL_NAME),
        value: TagValue::String(nickname.to_string()),
    });
    tags.push(Ed2kTag {
        name: TagName::Id(TAG_SPECIAL_VERSION),
        value: TagValue::UInt32(ED2K_CLIENT_VERSION),
    });
    let udp_ports = (u32::from(params.kad_port) << 16) | u32::from(params.tcp_port);
    tags.push(Ed2kTag {
        name: TagName::Id(TAG_SPECIAL_EMULE_UDPPORTS),
        value: TagValue::UInt32(udp_ports),
    });
    tags.push(Ed2kTag {
        name: TagName::Id(TAG_SPECIAL_CT_EMULE_VERSION),
        value: TagValue::UInt32(EMULE_VERSION_TAG),
    });
    tags.push(Ed2kTag {
        name: TagName::Id(TAG_SPECIAL_EMULE_MISCOPTIONS1),
        value: TagValue::UInt32(misc_options1(params.sec_ident_cap)),
    });
    let misc2 = misc_options2(
        params.tcp_low_id,
        params.udp_verified_open,
        params.obfuscation_enabled,
        params.obfuscation_required,
    );
    tags.push(Ed2kTag {
        name: TagName::Id(TAG_SPECIAL_EMULE_MISCOPTIONS2),
        value: TagValue::UInt32(misc2),
    });

    if let Some(pc26) = params.pc26_caps {
        tags.extend(build_pc26_hello_tags(pc26));
    }

    tags
}

/// Construye la máscara local predeterminada de capacidades `PC26`.
pub(crate) fn local_pc26_caps(
    ed25519_pubkey: Option<[u8; 32]>,
    ipv6_endpoint: Option<([u8; 16], u16)>,
    quic_port: Option<u16>,
    allow_pq: bool,
) -> Pc26PeerCaps {
    let mut flags = PC26_CAP_X25519_CHACHA
        | PC26_CAP_ZSTD
        | PC26_CAP_BLAKE3_MERKLE
        | PC26_CAP_UDP_HOLE_PUNCH;
    if allow_pq {
        flags |= PC26_CAP_HYBRID_MLKEM768;
    }
    if ed25519_pubkey.is_some() {
        flags |= PC26_CAP_ED25519_SUI_V3;
    }
    if ipv6_endpoint.is_some() {
        flags |= PC26_CAP_IPV6;
    }
    if quic_port.is_some_and(|p| p > 0) {
        flags |= PC26_CAP_QUIC;
    }
    Pc26PeerCaps {
        flags,
        ed25519_pubkey,
        ipv6_endpoint,
        quic_port: quic_port.filter(|&p| p > 0),
    }
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
        assert_eq!(m & (1 << 11), 0, "Captcha no implementado");
        assert_eq!(m & (1 << 12), 0, "sin callback directo en HighID");
    }

    #[test]
    fn direct_callback_requires_lowid_and_verified_udp() {
        assert_ne!(misc_options2(true, true, false, false) & (1 << 12), 0);
        assert_eq!(misc_options2(true, false, false, false) & (1 << 12), 0);
        assert_eq!(misc_options2(false, true, false, false) & (1 << 12), 0);
    }

    #[test]
    fn crypt_bits_follow_obfuscation_flags() {
        let m = misc_options2(false, false, true, true);
        assert_ne!(m & (1 << 7), 0, "SupportsCryptLayer");
        assert_ne!(m & (1 << 8), 0, "RequestsCryptLayer");
        assert_ne!(m & (1 << 9), 0, "RequiresCryptLayer");
    }

    #[test]
    fn build_hello_tags_unifies_version_and_appends_pc26() {
        let pc26 = local_pc26_caps(Some([0x33u8; 32]), None, Some(39738), true);
        let tags = build_hello_tags(&HelloBuildParams {
            nickname: "PulseNode",
            tcp_port: 39735,
            kad_port: 39737,
            sec_ident_cap: 3,
            tcp_low_id: false,
            udp_verified_open: false,
            obfuscation_enabled: true,
            obfuscation_required: false,
            pc26_caps: Some(&pc26),
        });
        assert!(tags.iter().any(|t| matches!(
            (&t.name, &t.value),
            (TagName::Id(TAG_SPECIAL_CT_EMULE_VERSION), TagValue::UInt32(v)) if *v == EMULE_VERSION_TAG
        )));
        let parsed = mule_protocol::ed2k::pc26::parse_pc26_tags(&tags).unwrap();
        assert!(parsed.supports_hybrid_mlkem768());
        assert!(parsed.supports_ed25519_sui_v3());
        assert_eq!(parsed.quic_port, Some(39738));
    }
}

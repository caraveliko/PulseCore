// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Metadatos de pares y representación de capacidades negociadas durante el saludo (handshake) eD2K.

use super::packet::{Ed2kEmuleInfo, Ed2kHello, Ed2kHelloAnswer};
use super::tags::{
    Ed2kTag, RawTag, TagName, TagValue, TAG_SPECIAL_CT_EMULE_VERSION,
    TAG_SPECIAL_EMULE_MISCOPTIONS1, TAG_SPECIAL_EMULE_MISCOPTIONS2, TAG_SPECIAL_EMULE_UDPPORTS,
    TAG_SPECIAL_EMULE_VERSION, TAG_SPECIAL_MISCOPTIONS1, TAG_SPECIAL_MISCOPTIONS2,
    TAG_SPECIAL_NAME, TAG_SPECIAL_VERSION,
};

/// Información consolidada y capacidades para un par remoto eD2K conectado.
#[derive(Debug, Clone, PartialEq)]
pub struct Ed2kPeerInfo {
    /// Hash de usuario MD4 de 16 bytes del par remoto.
    pub user_hash: [u8; 16],
    /// ID de cliente eD2K (HighID o LowID).
    pub client_id: u32,
    /// Puerto TCP de escucha del par remoto.
    pub tcp_port: u16,
    /// Nombre de cliente legible, si se declara en las etiquetas.
    pub name: Option<String>,
    /// Versión del protocolo / software cliente.
    pub version: Option<u32>,
    /// Código de versión específico de eMule, si está presente.
    pub emule_version: Option<u32>,
    /// Mapa de bits de opciones misceláneas 1 (extensiones de protocolo eMule).
    pub misc_options1: Option<u32>,
    /// Mapa de bits de opciones misceláneas 2.
    pub misc_options2: Option<u32>,
    /// Puerto UDP extendido de eMule del par remoto, si se anuncia en las etiquetas.
    pub emule_udp_port: Option<u16>,
    /// Dirección IPv4 del servidor eD2K al que está conectado el par (o 0 si no hay ninguno).
    pub server_ip: u32,
    /// Puerto del servidor eD2K al que está conectado el par (o 0 si no hay ninguno).
    pub server_port: u16,
    /// Todas las etiquetas de metadatos reconocidas intercambiadas en el saludo inicial.
    pub tags: Vec<Ed2kTag>,
    /// Etiquetas conservadas sin procesar o sin analizar.
    pub unknown_tags: Vec<RawTag>,
    /// Anuncio de protocolo extendido de eMule (`OP_EMULEINFO`), si se recibió.
    pub emule_info: Option<Ed2kEmuleInfo>,
}

impl Ed2kPeerInfo {
    /// Construye información de par a partir de un mensaje `Ed2kHello` entrante.
    pub fn from_hello(hello: &Ed2kHello) -> Self {
        Self::from_parts(
            hello.user_hash,
            hello.client_id,
            hello.tcp_port,
            hello.server_ip,
            hello.server_port,
            hello.tags.clone(),
            hello.unknown_tags.clone(),
        )
    }

    /// Construye información de par a partir de un mensaje `Ed2kHelloAnswer` entrante.
    pub fn from_hello_answer(answer: &Ed2kHelloAnswer) -> Self {
        Self::from_parts(
            answer.user_hash,
            answer.client_id,
            answer.tcp_port,
            answer.server_ip,
            answer.server_port,
            answer.tags.clone(),
            answer.unknown_tags.clone(),
        )
    }

    fn from_parts(
        user_hash: [u8; 16],
        client_id: u32,
        tcp_port: u16,
        server_ip: u32,
        server_port: u16,
        tags: Vec<Ed2kTag>,
        unknown_tags: Vec<RawTag>,
    ) -> Self {
        let mut name = None;
        let mut version = None;
        let mut emule_version = None;
        let mut misc_options1 = None;
        let mut misc_options2 = None;
        let mut emule_udp_port = None;

        for tag in &tags {
            match &tag.name {
                TagName::Id(TAG_SPECIAL_NAME) => {
                    if let TagValue::String(s) = &tag.value {
                        name = Some(s.clone());
                    }
                }
                TagName::Id(TAG_SPECIAL_VERSION) => {
                    if let TagValue::UInt32(v) = &tag.value {
                        version = Some(*v);
                    }
                }
                // Los clientes eMule anuncian su versión bajo CT_EMULE_VERSION
                // (0xFB); el identificador heredado 0x5A (SO_EMULEVERSION) se conserva como
                // respaldo para dialectos más antiguos.
                TagName::Id(TAG_SPECIAL_EMULE_VERSION | TAG_SPECIAL_CT_EMULE_VERSION) => {
                    if let TagValue::UInt32(v) = &tag.value {
                        emule_version = Some(*v);
                    }
                }
                TagName::Id(TAG_SPECIAL_MISCOPTIONS1 | TAG_SPECIAL_EMULE_MISCOPTIONS1) => {
                    if let TagValue::UInt32(v) = &tag.value {
                        misc_options1 = Some(*v);
                    }
                }
                TagName::Id(TAG_SPECIAL_MISCOPTIONS2 | TAG_SPECIAL_EMULE_MISCOPTIONS2) => {
                    if let TagValue::UInt32(v) = &tag.value {
                        misc_options2 = Some(*v);
                    }
                }
                TagName::Id(TAG_SPECIAL_EMULE_UDPPORTS) => {
                    if let TagValue::UInt32(v) = &tag.value {
                        emule_udp_port = Some(*v as u16);
                    } else if let TagValue::UInt16(v) = &tag.value {
                        emule_udp_port = Some(*v);
                    }
                }
                TagName::Text(t) if t.eq_ignore_ascii_case("name") => {
                    if let TagValue::String(s) = &tag.value {
                        name = Some(s.clone());
                    }
                }
                _ => {}
            }
        }

        Self {
            user_hash,
            client_id,
            tcp_port,
            name,
            version,
            emule_version,
            misc_options1,
            misc_options2,
            emule_udp_port,
            server_ip,
            server_port,
            tags,
            unknown_tags,
            emule_info: None,
        }
    }

    /// Busca una etiqueta por su nombre de visualización.
    pub fn find_tag(&self, display_name: &str) -> Option<&Ed2kTag> {
        crate::ed2k::tags::find_tag_by_name(&self.tags, display_name)
    }

    /// Devuelve true si este par se identificó como un cliente eMule.
    pub fn is_emule(&self) -> bool {
        self.emule_version.is_some()
    }

    /// Identificador de software cliente a partir del byte alto de `CT_EMULE_VERSION`
    /// (`CompatibleClientID` de eMule: 0 = eMule, 1 = cDonkey, 2 = xMule/lxMule,
    /// 3 = aMule, 4 = Shareaza, 5 = eMule Plus, 6 = HydraNode).
    pub fn software_id(&self) -> Option<u8> {
        self.emule_version.map(|v| (v >> 24) as u8)
    }

    /// Nombre legible del software cliente derivado de `software_id`.
    pub fn software_name(&self) -> &'static str {
        match self.software_id() {
            Some(0) => "eMule",
            Some(1) => "cDonkey",
            Some(2) => "xMule",
            Some(3) => "aMule",
            Some(4) => "Shareaza",
            Some(5) => "eMule Plus",
            Some(6) => "HydraNode",
            Some(0xAA) => "eMule-compat",
            Some(_) => "unknown",
            None => "ed2k",
        }
    }

    /// Cadena de versión a escala eMule decodificada de los 24 bits bajos de
    /// `CT_EMULE_VERSION` (`(mjr<<17)|(min<<10)|(upd<<7)`), p. ej. `"0.72a"`.
    pub fn emule_version_string(&self) -> Option<String> {
        self.emule_version.map(|v| {
            let v = v & 0x00FF_FFFF;
            let mjr = (v >> 17) & 0x7F;
            let min = (v >> 10) & 0x7F;
            let upd = (v >> 7) & 0x07;
            format!("{mjr}.{min}{}", (b'a' + upd as u8) as char)
        })
    }

    /// `m_byExtendedRequestsVer` de eMule — bits 8-11 de CT_EMULE_MISCOPTIONS1
    /// (0 = ninguno, 1 = peticiones extendidas, 2 = peticiones de ficheros grandes de 64 bits).
    pub fn extended_requests_version(&self) -> u8 {
        misc_field(self.misc_options1, 8)
    }

    /// Versión del protocolo Kad anunciada en los bits 0-4 de CT_EMULE_MISCOPTIONS2.
    pub fn kad_version(&self) -> u8 {
        misc_field(self.misc_options2, 0)
    }

    /// Devuelve true si este par anunció soporte para ficheros >4 GiB
    /// (bit 4 de misc_options2, `m_fLargeFiles` de eMule).
    pub fn supports_large_files(&self) -> bool {
        misc_bit(self.misc_options2, 4)
    }

    /// Devuelve true si se anunció la capacidad de compresión en OP_EMULEINFO
    /// o mediante los bits 20-23 de `CT_EMULE_MISCOPTIONS1` (`nDataCompVer`, `ProcessHelloTypePacket`
    /// de eMule). Los pares que lo anuncian aceptan
    /// bloques zlib `OP_COMPRESSEDPART` en las subidas.
    pub fn has_compression_capability(&self) -> bool {
        self.emule_info
            .as_ref()
            .is_some_and(|info| info.has_compression_capability())
            || misc_field(self.misc_options1, 20) > 0
    }

    /// `m_bySourceExchange1Ver` de eMule — versión SX1 anunciada en los
    /// bits 12-15 de CT_EMULE_MISCOPTIONS1 (`BaseClient.cpp:479`). `0` significa que
    /// el par no anunció intercambio de fuentes heredado.
    pub fn source_exchange1_version(&self) -> u8 {
        misc_field(self.misc_options1, 12)
    }

    /// Devuelve true si la capacidad de intercambio de fuentes se anunció en OP_EMULEINFO o CT_EMULE_MISCOPTIONS1.
    pub fn has_source_exchange_capability(&self) -> bool {
        self.emule_info
            .as_ref()
            .is_some_and(|info| info.has_source_exchange_capability())
            || misc_field(self.misc_options1, 12) > 0
    }

    /// Devuelve true si este par admite Source Exchange 2 (bit 10 de misc_options2).
    pub fn supports_source_exchange2(&self) -> bool {
        misc_bit(self.misc_options2, 10)
    }

    /// `m_bMultiPacket` de eMule — bit 1 de CT_EMULE_MISCOPTIONS1
    /// (`BaseClient.cpp:484`). El par acepta peticiones de ficheros
    /// `OP_MULTIPACKET` (hash + subcódigos incrustados en un paquete `PR_EMULE`).
    pub fn supports_multi_packet(&self) -> bool {
        misc_bit(self.misc_options1, 1)
    }

    /// `m_fExtMultiPacket` de eMule — bit 5 de CT_EMULE_MISCOPTIONS2
    /// (`BaseClient.cpp:518`). El par acepta `OP_MULTIPACKET_EXT`
    /// (hash + tamaño de fichero u64 + subcódigos de operación).
    pub fn supports_ext_multi_packet(&self) -> bool {
        misc_bit(self.misc_options2, 5)
    }

    /// `m_fSupportsFileIdent` de eMule — bit 13 de CT_EMULE_MISCOPTIONS2
    /// (`BaseClient.cpp:510`). El par acepta peticiones `OP_MULTIPACKET_EXT2`
    /// que portan un `FileIdentifier` (hash + raíz AICH + tamaño).
    pub fn supports_file_identifiers(&self) -> bool {
        misc_bit(self.misc_options2, 13)
    }

    /// `m_fDirectUDPCallback` de eMule — bit 12 de CT_EMULE_MISCOPTIONS2
    /// (`BaseClient.cpp:511`). El par está detrás de cortafuegos TCP pero abierto por UDP
    /// (NAT full-cone) y acepta datagramas `OP_DIRECTCALLBACKREQ` pidiéndole
    /// que marque a nuestro puerto TCP (`BaseClient.cpp:1403`).
    pub fn supports_direct_udp_callback(&self) -> bool {
        misc_bit(self.misc_options2, 12)
    }

    /// Devuelve true si este par admite SX1 (vía OP_EMULEINFO) o SX2 (vía misc_options2).
    pub fn supports_source_exchange(&self) -> bool {
        self.supports_source_exchange2() || self.has_source_exchange_capability()
    }

    /// Devuelve true si este par admite AICH (bit 29 de misc_options1,
    /// `m_bySupportAICH` de eMule en CT_EMULE_MISCOPTIONS1).
    pub fn supports_aich(&self) -> bool {
        self.misc_options1
            .map(|m| ((m >> 29) & 0x07) > 0)
            .unwrap_or(false)
    }

    /// `m_byUDPVer` de eMule — versión del protocolo UDP de cliente anunciada en los
    /// bits 24-27 de CT_EMULE_MISCOPTIONS1 (0 = sin soporte UDP).
    pub fn udp_version(&self) -> u8 {
        misc_field(self.misc_options1, 24)
    }

    /// `m_bySupportSecIdent` de eMule — nivel de soporte de Secure Ident anunciado en los
    /// bits 16-19 de CT_EMULE_MISCOPTIONS1 (0 = ninguno, bit0 = v1, bit1 = v2).
    pub fn secure_ident_support(&self) -> u8 {
        misc_field(self.misc_options1, 16)
    }

    /// Devuelve true si este par admite ofuscación de protocolo (bit 7 de misc_options2).
    pub fn supports_crypt_layer(&self) -> bool {
        misc_bit(self.misc_options2, 7)
    }

    /// Devuelve true si este par solicita ofuscación de protocolo (bit 8 de misc_options2).
    pub fn requests_crypt_layer(&self) -> bool {
        misc_bit(self.misc_options2, 8)
    }

    /// Devuelve true si este par requiere estrictamente ofuscación de protocolo (bit 9 de misc_options2).
    pub fn requires_crypt_layer(&self) -> bool {
        misc_bit(self.misc_options2, 9)
    }

    /// Devuelve las capacidades modernas `PC26` anunciadas por el par, si existen.
    pub fn pc26_caps(&self) -> Option<super::pc26::Pc26PeerCaps> {
        super::pc26::parse_pc26_tags(&self.tags)
    }
}

/// Extrae un campo de cuarteto (nibble) de 4 bits en `shift` de una palabra `misc_options*`
/// (devuelve `0` cuando la etiqueta de opción estaba ausente).
fn misc_field(options: Option<u32>, shift: u32) -> u8 {
    options.map(|m| ((m >> shift) & 0x0F) as u8).unwrap_or(0)
}

/// Comprueba el bit `bit` de una palabra `misc_options*` (`false` cuando la etiqueta de opción estaba ausente).
fn misc_bit(options: Option<u32>, bit: u32) -> bool {
    options.map(|m| (m & (1 << bit)) != 0).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sx1_capability_detected_from_misc_options1() {
        let mut info = Ed2kPeerInfo {
            user_hash: [0u8; 16],
            client_id: 123456789,
            tcp_port: 49662,
            name: None,
            version: None,
            emule_version: None,
            misc_options1: Some(4 << 12), // SX1 version = 4
            misc_options2: None,
            emule_udp_port: None,
            server_ip: 0,
            server_port: 0,
            tags: vec![],
            unknown_tags: vec![],
            emule_info: None,
        };

        assert!(info.has_source_exchange_capability());
        assert!(info.supports_source_exchange());
        assert!(!info.supports_source_exchange2());

        // Ahora se activa el bit 10 de SX2 en misc_options2
        info.misc_options2 = Some(1 << 10);
        assert!(info.supports_source_exchange2());
        assert!(info.supports_source_exchange());
    }

    #[test]
    fn test_direct_udp_callback_detected_from_misc_options2() {
        // `m_fDirectUDPCallback` de eMule = bit 12 de CT_EMULE_MISCOPTIONS2
        // (`BaseClient.cpp:511`) — anunciado por pares detrás de cortafuegos TCP pero con UDP abierto
        // (NAT full-cone) que aceptan `OP_DIRECTCALLBACKREQ`.
        let mut info = Ed2kPeerInfo {
            user_hash: [0u8; 16],
            client_id: 123456789,
            tcp_port: 49662,
            name: None,
            version: None,
            emule_version: None,
            misc_options1: None,
            misc_options2: None,
            emule_udp_port: None,
            server_ip: 0,
            server_port: 0,
            tags: vec![],
            unknown_tags: vec![],
            emule_info: None,
        };
        assert!(!info.supports_direct_udp_callback());

        info.misc_options2 = Some(1 << 12);
        assert!(info.supports_direct_udp_callback());

        // Los bits vecinos no deben filtrarse (bit 11 = captcha, bit 13 =
        // FileIdent).
        info.misc_options2 = Some((1 << 11) | (1 << 13));
        assert!(!info.supports_direct_udp_callback());
    }
}

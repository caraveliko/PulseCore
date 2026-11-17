// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Mensajes UDP de cliente a cliente (`ClientUDPSocket.cpp`, eMule 0.72a).
//!
//! Estos datagramas se envían por el socket UDP compartido bajo el byte de protocolo
//! de eMule (`0xC5`), junto con el tráfico de Kademlia — exactamente igual que
//! `CClientUDPSocket::ProcessPacket`, que despacha según el primer byte.
//!
//! Códigos de operación admitidos:
//! - `OP_REASKFILEPING` (`0x90`) — mantenimiento de conexión hacia una fuente en cola.
//! - `OP_REASKACK` (`0x91`) — transporta el rango en la cola de subida.
//! - `OP_FILENOTFOUND` (`0x92`) — el fichero solicitado no está compartido.
//! - `OP_QUEUEFULL` (`0x93`) — la cola de subida remota está llena.
//! - `OP_REASKCALLBACKUDP` (`0x94`) — retransmisión mediante buddy hacia una fuente LowID.
//! - `OP_DIRECTCALLBACKREQ` (`0x95`) — petición de conexión saliente hacia un
//!   par tras cortafuegos en Kad.
//! - `OP_PORTTEST` (`0xFE`) — prueba de conectividad UDP (un único byte `0x12`).

use super::opcodes::{
    OP_DIRECTCALLBACKREQ, OP_FILENOTFOUND, OP_PORTTEST, OP_QUEUEFULL, OP_REASKACK,
    OP_REASKCALLBACKUDP, OP_REASKFILEPING,
};
use super::protocol::PR_EMULE;

/// Mensaje UDP de cliente a cliente decodificado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientUdpMessage {
    /// `OP_REASKFILEPING` — `hash16` + información extendida opcional
    /// (`u16` fuentes completas, precedido por un bloque de estado de partes
    /// para pares cuya versión UDP es > 3).
    ReaskFilePing {
        /// Hash MD4 del fichero solicitado.
        file_hash: [u8; 16],
        /// Cantidad anunciada de fuentes completas, cuando está presente.
        complete_sources: Option<u16>,
    },
    /// `OP_REASKACK` — rango en la cola (último `u16` de la carga útil; un
    /// mapa de bits opcional con el estado de partes puede precederlo para versión UDP > 3).
    ReaskAck {
        /// Posición basada en 1 en la cola de subida remota.
        queue_rank: u16,
        /// Bloque de estado de partes cuando el par envió uno (versión UDP > 3).
        part_status: Option<UdpPartStatus>,
    },
    /// `OP_FILENOTFOUND` — el par no comparte el fichero solicitado.
    FileNotFound,
    /// `OP_QUEUEFULL` — la cola de subida del par está llena.
    QueueFull,
    /// `OP_REASKCALLBACKUDP` — un solicitante nos pide (como buddy de Kad) retransmitir
    /// una repetición de petición de fichero hacia nuestro cliente buddy mediante TCP.
    /// Carga útil: `[buddy_id:16][file_hash:16][part_status..][complete_src:2]`.
    ReaskCallback {
        /// ID de buddy al que se dirige el paquete (debe coincidir con el ID de nuestro buddy).
        buddy_id: [u8; 16],
        /// Resto de la carga útil tras el ID de buddy: `file_hash` + información extendida
        /// de repetición de petición, reenviada tal cual tras la cabecera
        /// IP/puerto del solicitante (empaquetado `memcpy(packet + 10)` de eMule).
        rest: Vec<u8>,
    },
    /// `OP_DIRECTCALLBACKREQ` — el remitente nos pide (como par tras cortafuegos
    /// en Kad) abrir una conexión TCP saliente hacia él
    /// (`[tcp_port:2][userhash:16][connect_options:1]`).
    DirectCallbackReq {
        /// Puerto TCP en el que escucha el solicitante.
        tcp_port: u16,
        /// Hash de usuario del solicitante (identidad + clave de capa criptográfica).
        user_hash: [u8; 16],
        /// Byte de opciones de conexión de eMule (soporte/petición/requisito de cifrado).
        conn_opts: u8,
    },
    /// `OP_PORTTEST` — sondeo de conectividad UDP. eMule responde únicamente un
    /// byte individual `0x12`, y lo hace a través de su socket de prueba TCP dedicado
    /// en lugar de en este canal UDP.
    PortTest {
        /// Byte individual de petición (eMule espera `0x12`).
        request: u8,
    },
    /// Cualquier otro código de operación recibido en el canal UDP de cliente.
    Unknown {
        /// Byte sin procesar del código de operación.
        opcode: u8,
    },
}

/// Bloque de estado de partes transportado por `OP_REASKACK` para pares con versión UDP > 3:
/// `u16 part_count` seguido de un mapa de bits de `ceil(part_count/8)` bytes
/// (`CUpDownClient::ProcessFileStatus` de eMule — las partes que el cliente de
/// subida remoto posee del fichero solicitado).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpPartStatus {
    /// Cantidad total de partes que describe el mapa de bits.
    pub part_count: u16,
    /// Mapa de bits de disponibilidad, `ceil(part_count/8)` bytes, bit *i* activo =
    /// el par posee la parte *i*.
    pub bitfield: Vec<u8>,
}

/// Codifica `OP_REASKFILEPING` en el formato de cable para versión UDP > 3:
/// `hash16 + u16 part_status_count + u16 complete_sources`
/// (`DownloadClient.cpp: UDPReaskForDownload` — el bloque de estado de partes es
/// `u16(0)` vacío para ficheros que completamos, lo cual también es inofensivo para
/// pares de versión ≤ 3: leen el último `u16`).
pub fn encode_reask_file_ping(file_hash: &[u8; 16], complete_sources: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(22);
    out.push(PR_EMULE);
    out.push(OP_REASKFILEPING);
    out.extend_from_slice(file_hash);
    out.extend_from_slice(&0u16.to_le_bytes()); // estado de partes vacío
    out.extend_from_slice(&complete_sources.to_le_bytes());
    out
}

/// Codifica `OP_REASKACK`.
///
/// `include_part_status` refleja `sender->GetUDPVersion() > 3` de eMule: cuando
/// la versión UDP del solicitante es > 3, la respuesta comienza con un bloque de estado
/// de partes vacío (`u16(0)` — los ficheros compartidos siempre están completos), seguido
/// por el rango en la cola `u16`. Los pares más antiguos solo reciben el rango.
pub fn encode_reask_ack(queue_rank: u16, include_part_status: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(6);
    out.push(PR_EMULE);
    out.push(OP_REASKACK);
    if include_part_status {
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out.extend_from_slice(&queue_rank.to_le_bytes());
    out
}

/// Codifica `OP_FILENOTFOUND` (carga útil vacía).
pub fn encode_file_not_found() -> Vec<u8> {
    vec![PR_EMULE, OP_FILENOTFOUND]
}

/// Bit 3 del byte de opciones de conexión (`GetMyConnectOptions`): el par está
/// tras un cortafuegos TCP pero con UDP abierto y acepta `OP_DIRECTCALLBACKREQ`
/// (`OtherFunctions.cpp:4049-4052`). Transportado en el bit 12 de MISCOPTIONS2 del
/// hello de eD2K y en `TAG_ENCRYPTION` de las fuentes de Kad publicadas.
pub const CONNECT_OPT_DIRECT_CALLBACK: u8 = 0x08;

/// Codifica `OP_DIRECTCALLBACKREQ` (`0x95`) — `BaseClient.cpp:1411-1415` de eMule
/// (paso 4 de `TryToConnect`): solicitamos a un par inalcanzable por TCP (anunciado mediante
/// `SupportsDirectUDPCallback`, bit 12 de MISCOPTIONS2) que devuelva la llamada a nuestro
/// puerto TCP. Carga útil: `<nuestro puerto TCP u16><nuestro userhash 16><opciones de conexión u8>`
/// — el byte de opciones es `GetMyConnectOptions(true, false)` (solo indicadores de cifrado,
/// bit de callback directo a cero).
pub fn encode_direct_callback_req(tcp_port: u16, user_hash: &[u8; 16], conn_opts: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + 19);
    out.push(PR_EMULE);
    out.push(OP_DIRECTCALLBACKREQ);
    out.extend_from_slice(&tcp_port.to_le_bytes());
    out.extend_from_slice(user_hash);
    out.push(conn_opts);
    out
}

/// Codifica `OP_QUEUEFULL` (carga útil vacía).
pub fn encode_queue_full() -> Vec<u8> {
    vec![PR_EMULE, OP_QUEUEFULL]
}

/// Decodifica un datagrama UDP de cliente a cliente (el primer byte debe ser `0xC5` o
/// `0xE3`; se rechazan los bytes del protocolo Kademlia para que el llamante pueda enrutarlos).
pub fn decode_client_udp(data: &[u8]) -> Result<ClientUdpMessage, String> {
    if data.len() < 2 {
        return Err("datagrama UDP de cliente demasiado corto".into());
    }
    let payload = &data[2..];
    match data[1] {
        OP_REASKFILEPING => {
            if payload.len() < 16 {
                return Err("OP_REASKFILEPING truncado".into());
            }
            let mut file_hash = [0u8; 16];
            file_hash.copy_from_slice(&payload[..16]);
            // Información extendida (versión UDP > 2): u16 final = fuentes completas.
            let complete_sources = if payload.len() >= 18 {
                Some(u16::from_le_bytes([
                    payload[payload.len() - 2],
                    payload[payload.len() - 1],
                ]))
            } else {
                None
            };
            Ok(ClientUdpMessage::ReaskFilePing {
                file_hash,
                complete_sources,
            })
        }
        OP_REASKACK => {
            if payload.len() < 2 {
                return Err("OP_REASKACK truncado".into());
            }
            let rank_off = payload.len() - 2;
            let queue_rank = u16::from_le_bytes([payload[rank_off], payload[rank_off + 1]]);
            // Formato de versión UDP > 3: conteo de mapa de bits u16 + ceil(count/8) bytes.
            let part_status = if rank_off >= 2 {
                let count = u16::from_le_bytes([payload[0], payload[1]]);
                let bitfield_len = (count as usize).div_ceil(8);
                if 2 + bitfield_len == rank_off {
                    Some(UdpPartStatus {
                        part_count: count,
                        bitfield: payload[2..rank_off].to_vec(),
                    })
                } else {
                    None
                }
            } else {
                None
            };
            Ok(ClientUdpMessage::ReaskAck {
                queue_rank,
                part_status,
            })
        }
        OP_FILENOTFOUND => Ok(ClientUdpMessage::FileNotFound),
        OP_QUEUEFULL => Ok(ClientUdpMessage::QueueFull),
        OP_DIRECTCALLBACKREQ => {
            if payload.len() < 19 {
                return Err("OP_DIRECTCALLBACKREQ truncado".into());
            }
            let mut user_hash = [0u8; 16];
            user_hash.copy_from_slice(&payload[2..18]);
            Ok(ClientUdpMessage::DirectCallbackReq {
                tcp_port: u16::from_le_bytes([payload[0], payload[1]]),
                user_hash,
                conn_opts: payload[18],
            })
        }
        OP_PORTTEST => {
            // eMule solo atiende un sondeo de un solo byte `0x12`.
            if payload.len() != 1 {
                return Err("OP_PORTTEST malformado".into());
            }
            Ok(ClientUdpMessage::PortTest {
                request: payload[0],
            })
        }
        OP_REASKCALLBACKUDP => {
            if payload.len() < 17 {
                return Err("OP_REASKCALLBACKUDP truncado".into());
            }
            let mut buddy_id = [0u8; 16];
            buddy_id.copy_from_slice(&payload[..16]);
            Ok(ClientUdpMessage::ReaskCallback {
                buddy_id,
                rest: payload[16..].to_vec(),
            })
        }
        other => Ok(ClientUdpMessage::Unknown { opcode: other }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_wire_format() {
        let hash = [0xAB; 16];
        let pkt = encode_reask_file_ping(&hash, 7);
        assert_eq!(pkt.len(), 22);
        assert_eq!(pkt[0], 0xC5);
        assert_eq!(pkt[1], 0x90);
        assert_eq!(&pkt[2..18], &hash);
        assert_eq!(&pkt[18..20], &[0, 0]); // estado de partes vacío
        assert_eq!(&pkt[20..22], &7u16.to_le_bytes());
    }

    #[test]
    fn decode_ping_v2_and_v4() {
        let hash = [1u8; 16];
        // Formato v2: solo hash
        let mut v2 = vec![PR_EMULE, OP_REASKFILEPING];
        v2.extend_from_slice(&hash);
        match decode_client_udp(&v2).unwrap() {
            ClientUdpMessage::ReaskFilePing {
                file_hash,
                complete_sources,
            } => {
                assert_eq!(file_hash, hash);
                assert_eq!(complete_sources, None);
            }
            m => panic!("unexpected {m:?}"),
        }
        // Formato v>3: hash + conteo de estado u16 + fuentes completas u16
        match decode_client_udp(&encode_reask_file_ping(&hash, 42)).unwrap() {
            ClientUdpMessage::ReaskFilePing {
                complete_sources, ..
            } => assert_eq!(complete_sources, Some(42)),
            m => panic!("unexpected {m:?}"),
        }
    }

    #[test]
    fn ack_with_and_without_part_status() {
        // Rango simple
        match decode_client_udp(&encode_reask_ack(5, false)).unwrap() {
            ClientUdpMessage::ReaskAck {
                queue_rank,
                part_status,
            } => {
                assert_eq!(queue_rank, 5);
                assert!(part_status.is_none());
            }
            m => panic!("unexpected {m:?}"),
        }
        // Con estado de partes vacío
        match decode_client_udp(&encode_reask_ack(9, true)).unwrap() {
            ClientUdpMessage::ReaskAck {
                queue_rank,
                part_status,
            } => {
                assert_eq!(queue_rank, 9);
                assert_eq!(
                    part_status,
                    Some(UdpPartStatus {
                        part_count: 0,
                        bitfield: vec![]
                    })
                );
            }
            m => panic!("unexpected {m:?}"),
        }
        // Con un mapa de bits real: count=3 + mapa de bits de 1 byte + rango
        let raw = vec![PR_EMULE, OP_REASKACK, 3, 0, 0b101, 17, 0];
        match decode_client_udp(&raw).unwrap() {
            ClientUdpMessage::ReaskAck {
                queue_rank,
                part_status,
            } => {
                assert_eq!(queue_rank, 17);
                assert_eq!(
                    part_status,
                    Some(UdpPartStatus {
                        part_count: 3,
                        bitfield: vec![0b101]
                    })
                );
            }
            m => panic!("unexpected {m:?}"),
        }
    }

    #[test]
    fn queuefull_and_filenotfound() {
        assert_eq!(
            decode_client_udp(&encode_queue_full()).unwrap(),
            ClientUdpMessage::QueueFull
        );
        assert_eq!(
            decode_client_udp(&encode_file_not_found()).unwrap(),
            ClientUdpMessage::FileNotFound
        );
        assert_eq!(
            decode_client_udp(&[PR_EMULE, 0x55, 1]).unwrap(),
            ClientUdpMessage::Unknown { opcode: 0x55 }
        );
    }

    #[test]
    fn direct_callback_req_wire_format() {
        // eMule Opcodes.h: <TCPPort 2><Userhash 16><ConnectionOptions 1>
        let mut raw = vec![PR_EMULE, OP_DIRECTCALLBACKREQ];
        raw.extend_from_slice(&49662u16.to_le_bytes());
        raw.extend_from_slice(&[0xAB; 16]);
        raw.push(0x07);
        match decode_client_udp(&raw).unwrap() {
            ClientUdpMessage::DirectCallbackReq {
                tcp_port,
                user_hash,
                conn_opts,
            } => {
                assert_eq!(tcp_port, 49662);
                assert_eq!(user_hash, [0xAB; 16]);
                assert_eq!(conn_opts, 0x07);
            }
            m => panic!("unexpected {m:?}"),
        }
        // Se rechaza si está truncado (carga útil < 19 bytes).
        assert!(decode_client_udp(&raw[..18]).is_err());

        // El codificador saliente (`BaseClient.cpp:1411-1415`) debe producir los
        // mismos bytes exactos en cable — la prueba de ida y vuelta cubre entramado y orden de campos.
        let encoded = encode_direct_callback_req(49662, &[0xAB; 16], 0x07);
        assert_eq!(encoded, raw);
        match decode_client_udp(&encoded).unwrap() {
            ClientUdpMessage::DirectCallbackReq {
                tcp_port,
                user_hash,
                conn_opts,
            } => {
                assert_eq!(tcp_port, 49662);
                assert_eq!(user_hash, [0xAB; 16]);
                assert_eq!(conn_opts, 0x07);
            }
            m => panic!("unexpected {m:?}"),
        }
    }

    #[test]
    fn port_test_wire_format() {
        match decode_client_udp(&[PR_EMULE, OP_PORTTEST, 0x12]).unwrap() {
            ClientUdpMessage::PortTest { request } => assert_eq!(request, 0x12),
            m => panic!("unexpected {m:?}"),
        }
        // eMule ignora cualquier cosa que no sea un único byte de petición.
        assert!(decode_client_udp(&[PR_EMULE, OP_PORTTEST]).is_err());
        assert!(decode_client_udp(&[PR_EMULE, OP_PORTTEST, 0x12, 0x34]).is_err());
    }
}

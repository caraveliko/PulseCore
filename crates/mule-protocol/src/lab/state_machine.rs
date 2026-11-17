// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Máquina de estados determinista pura para sesiones entre pares de laboratorio.

use thiserror::Error;

use super::messages::{LabMessage, LAB_OP_HELLO, LAB_OP_HELLO_ACK};
use crate::framing::RawPacket;
use crate::traits::{ReceivedBlock, SourceStateUpdate};
use crate::types::{PeerId, PR_EMULE_COMPRESSED, PR_LAB};

/// Errores de violación de protocolo que provocan la terminación inmediata de la sesión.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum ProtocolViolation {
    /// El mensaje llegó en un estado no permitido.
    #[error("Unexpected message opcode 0x{opcode:02X} in state {state}")]
    UnexpectedMessageInState {
        /// Nombre del estado actual.
        state: &'static str,
        /// Código de operación del mensaje recibido.
        opcode: u8,
    },

    /// La versión de protocolo negociada es incompatible.
    #[error("Incompatible protocol version: client={client}, server={server}")]
    IncompatibleVersion {
        /// Versión del cliente.
        client: u32,
        /// Versión del servidor.
        server: u32,
    },

    /// El nonce devuelto en HelloAck no coincidió con el nonce de desafío original.
    #[error("Handshake nonce mismatch")]
    NonceMismatch,

    /// El mensaje Hello se envió más de una vez.
    #[error("Double Hello rejected: session already initialized")]
    DoubleHello,

    /// Intento de transferencia de bloque antes de que la sesión estuviera en estado Ready.
    #[error("Transfer message received before Ready state")]
    PrematureTransferMessage,

    /// La estructura de la carga útil estaba mal formada.
    #[error("Malformed message payload: {0}")]
    Malformed(String),

    /// Trama comprimida (0xD4) rechazada a la espera de la etapa de descompresión acotada.
    #[error("Compressed frame 0xD4 (PR_EMULE_COMPRESSED) is not supported prior to bounded decompression stage")]
    CompressedProtocolUnsupported,

    /// Byte de protocolo inesperado o no compatible.
    #[error("Unexpected protocol byte 0x{protocol:02X} (expected 0xEE PR_LAB)")]
    UnexpectedProtocol {
        /// Byte de protocolo rechazado.
        protocol: u8,
    },
}

/// Causa de fallo terminal de la sesión.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum PeerFailure {
    /// Error de violación de protocolo.
    #[error(transparent)]
    Violation(#[from] ProtocolViolation),

    /// Expiró el tiempo de espera del protocolo de enlace o de inactividad.
    #[error("Timeout: {0}")]
    Timeout(String),

    /// Fallo de E/S de socket.
    #[error("I/O error: {0}")]
    Io(String),
}

/// Estados del ciclo de vida de una conexión entre pares de laboratorio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerState {
    /// Estado inicial desconectado.
    Disconnected,
    /// Estableciendo activamente la conexión de transporte.
    Connecting,
    /// Oyente a la espera del Hello del iniciador.
    AwaitingHello,
    /// Iniciador a la espera del HelloAck del receptor.
    AwaitingHelloAck {
        /// Nonce de desafío local a la espera de confirmación.
        local_nonce: [u8; 16],
    },
    /// Sesión completamente autenticada y negociada lista para la transferencia de datos.
    Ready {
        /// Identificador del par remoto.
        remote_peer_id: PeerId,
        /// Versión del protocolo de laboratorio acordada mutuamente.
        negotiated_version: u32,
    },
    /// Terminando la conexión de forma ordenada.
    Closing,
    /// La sesión ha terminado limpiamente.
    Closed,
    /// Estado de fallo terminal irreversible.
    Failed {
        /// Motivo del fallo.
        reason: PeerFailure,
    },
}

impl PeerState {
    /// Devuelve el nombre legible para humanos del estado actual.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Connecting => "Connecting",
            Self::AwaitingHello => "AwaitingHello",
            Self::AwaitingHelloAck { .. } => "AwaitingHelloAck",
            Self::Ready { .. } => "Ready",
            Self::Closing => "Closing",
            Self::Closed => "Closed",
            Self::Failed { .. } => "Failed",
        }
    }

    /// Verdadero si la sesión está lista para transferencias.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    /// Verdadero si la sesión se encuentra en un estado terminal (Closed o Failed).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Closed | Self::Failed { .. })
    }
}

/// Acciones producidas por la máquina de estados para ser ejecutadas por la capa de transporte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerAction {
    /// Transmite un mensaje al par remoto.
    SendMessage(LabMessage),
    /// Reenvía un bloque descargado al sumidero de almacenamiento.
    NotifySink(ReceivedBlock),
    /// Reenvía los metadatos del par remoto al planificador de descargas.
    NotifyPlanner(SourceStateUpdate),
    /// Cierra el transporte de red físico.
    CloseConnection,
}

/// Controlador de máquina de estados que gestiona la progresión de la sesión.
pub struct PeerStateMachine {
    state: PeerState,
    local_peer_id: PeerId,
    supported_version: u32,
}

impl PeerStateMachine {
    /// Crea una nueva máquina de estados para un oyente que espera conexiones entrantes.
    pub fn new_listener(local_peer_id: PeerId, supported_version: u32) -> Self {
        Self {
            state: PeerState::AwaitingHello,
            local_peer_id,
            supported_version,
        }
    }

    /// Crea una nueva máquina de estados para un iniciador que comienza una conexión.
    pub fn new_initiator(
        local_peer_id: PeerId,
        supported_version: u32,
        challenge_nonce: [u8; 16],
    ) -> (Self, LabMessage) {
        let sm = Self {
            state: PeerState::AwaitingHelloAck {
                local_nonce: challenge_nonce,
            },
            local_peer_id,
            supported_version,
        };
        let hello = LabMessage::Hello {
            client_version: supported_version,
            nonce: challenge_nonce,
            peer_id: local_peer_id,
        };
        (sm, hello)
    }

    /// Devuelve una referencia al estado actual.
    pub fn state(&self) -> &PeerState {
        &self.state
    }

    /// Fuerza la máquina de estados al estado Failed con el motivo proporcionado.
    pub fn fail(&mut self, failure: PeerFailure) {
        self.state = PeerState::Failed { reason: failure };
    }

    /// Paso de transición pura que gestiona un mensaje entrante.
    pub fn on_message(&mut self, msg: LabMessage) -> Result<Vec<PeerAction>, ProtocolViolation> {
        let (next_state, actions) = Self::step(
            self.state.clone(),
            msg,
            self.local_peer_id,
            self.supported_version,
        )?;
        self.state = next_state;
        Ok(actions)
    }

    /// Transición pura que gestiona un paquete sin procesar entrante antes de decodificarlo.
    ///
    /// Valida el byte de protocolo (rechazando PR_EMULE_COMPRESSED o protocolos inesperados)
    /// y decodifica en un [`LabMessage`].
    pub fn on_raw_packet(
        &mut self,
        packet: &RawPacket,
    ) -> Result<Vec<PeerAction>, ProtocolViolation> {
        if packet.protocol == PR_EMULE_COMPRESSED {
            self.state = PeerState::Failed {
                reason: PeerFailure::Violation(ProtocolViolation::CompressedProtocolUnsupported),
            };
            return Err(ProtocolViolation::CompressedProtocolUnsupported);
        }

        if packet.protocol != PR_LAB {
            let violation = ProtocolViolation::UnexpectedProtocol {
                protocol: packet.protocol,
            };
            self.state = PeerState::Failed {
                reason: PeerFailure::Violation(violation.clone()),
            };
            return Err(violation);
        }

        let msg = match LabMessage::from_raw_packet(packet) {
            Ok(m) => m,
            Err(e) => {
                let violation = ProtocolViolation::Malformed(e.to_string());
                self.state = PeerState::Failed {
                    reason: PeerFailure::Violation(violation.clone()),
                };
                return Err(violation);
            }
        };

        self.on_message(msg)
    }

    /// Función de transición pura comprobable sin red ni efectos secundarios.
    pub fn step(
        state: PeerState,
        msg: LabMessage,
        local_peer_id: PeerId,
        supported_version: u32,
    ) -> Result<(PeerState, Vec<PeerAction>), ProtocolViolation> {
        // Controlador universal de Goodbye: puede llegar en cualquier estado no terminal.
        if let LabMessage::Goodbye { .. } = &msg {
            return match state {
                PeerState::Closed | PeerState::Failed { .. } => {
                    Err(ProtocolViolation::UnexpectedMessageInState {
                        state: state.name(),
                        opcode: 0x07,
                    })
                }
                _ => Ok((PeerState::Closed, vec![PeerAction::CloseConnection])),
            };
        }

        // Comprobación universal de transferencia prematura: RequestBlock y BlockData están prohibidos antes de Ready.
        if !state.is_ready()
            && matches!(
                msg,
                LabMessage::RequestBlock { .. } | LabMessage::BlockData { .. }
            )
        {
            return Err(ProtocolViolation::PrematureTransferMessage);
        }

        match state {
            PeerState::AwaitingHello => match msg {
                LabMessage::Hello {
                    client_version,
                    nonce,
                    peer_id,
                } => {
                    if client_version != supported_version {
                        return Err(ProtocolViolation::IncompatibleVersion {
                            client: client_version,
                            server: supported_version,
                        });
                    }
                    let ack = LabMessage::HelloAck {
                        server_version: supported_version,
                        nonce_ack: nonce,
                        peer_id: local_peer_id,
                    };
                    let next = PeerState::Ready {
                        remote_peer_id: peer_id,
                        negotiated_version: supported_version,
                    };
                    Ok((next, vec![PeerAction::SendMessage(ack)]))
                }
                _ => Err(ProtocolViolation::UnexpectedMessageInState {
                    state: "AwaitingHello",
                    opcode: LAB_OP_HELLO_ACK,
                }),
            },

            PeerState::AwaitingHelloAck { local_nonce } => match msg {
                LabMessage::HelloAck {
                    server_version,
                    nonce_ack,
                    peer_id,
                } => {
                    if server_version != supported_version {
                        return Err(ProtocolViolation::IncompatibleVersion {
                            client: supported_version,
                            server: server_version,
                        });
                    }
                    if nonce_ack != local_nonce {
                        return Err(ProtocolViolation::NonceMismatch);
                    }
                    let next = PeerState::Ready {
                        remote_peer_id: peer_id,
                        negotiated_version: supported_version,
                    };
                    Ok((next, vec![]))
                }
                _ => Err(ProtocolViolation::UnexpectedMessageInState {
                    state: "AwaitingHelloAck",
                    opcode: LAB_OP_HELLO,
                }),
            },

            PeerState::Ready {
                remote_peer_id,
                negotiated_version,
            } => match msg {
                LabMessage::Hello { .. } => Err(ProtocolViolation::DoubleHello),
                LabMessage::HelloAck { .. } => Err(ProtocolViolation::UnexpectedMessageInState {
                    state: "Ready",
                    opcode: LAB_OP_HELLO_ACK,
                }),
                LabMessage::FileOffer {
                    file_hash,
                    file_size: _,
                    name: _,
                } => {
                    let update = SourceStateUpdate {
                        peer_id: remote_peer_id,
                        file_hash,
                        available_parts: vec![true],
                        queue_rank: None,
                    };
                    let next = PeerState::Ready {
                        remote_peer_id,
                        negotiated_version,
                    };
                    Ok((next, vec![PeerAction::NotifyPlanner(update)]))
                }
                LabMessage::RequestBlock { .. } => {
                    // RequestBlock lo gestiona el motor de fuentes
                    let next = PeerState::Ready {
                        remote_peer_id,
                        negotiated_version,
                    };
                    Ok((next, vec![]))
                }
                LabMessage::BlockData {
                    file_hash,
                    start_offset,
                    data,
                } => {
                    let block = ReceivedBlock::new(file_hash, start_offset, data);
                    let next = PeerState::Ready {
                        remote_peer_id,
                        negotiated_version,
                    };
                    Ok((next, vec![PeerAction::NotifySink(block)]))
                }
                LabMessage::BlockRejected { .. } => {
                    let next = PeerState::Ready {
                        remote_peer_id,
                        negotiated_version,
                    };
                    Ok((next, vec![]))
                }
                LabMessage::ProtocolError { .. } => {
                    let next = PeerState::Closed;
                    Ok((next, vec![PeerAction::CloseConnection]))
                }
                LabMessage::Goodbye { .. } => unreachable!(),
            },

            other => match msg {
                LabMessage::RequestBlock { .. } | LabMessage::BlockData { .. } => {
                    Err(ProtocolViolation::PrematureTransferMessage)
                }
                _ => Err(ProtocolViolation::UnexpectedMessageInState {
                    state: other.name(),
                    opcode: 0,
                }),
            },
        }
    }
}

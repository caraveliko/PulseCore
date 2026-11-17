// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Utilidades WebSocket: envío de JSON y espera de la respuesta con el `id`
//! esperado. Las notificaciones de eventos broadcast se multiplexan en el
//! socket y se descartan hasta recibir la respuesta.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

/// Número máximo de tramas leídas en busca de una respuesta (los eventos de difusión
/// pueden preceder a la respuesta RPC en el mismo socket).
const MAX_RESPONSE_FRAMES: usize = 64;

/// Tiempo de espera por trama leída — limita la paciencia total sin bloquear
/// indefinidamente una prueba en un socket silencioso.
const FRAME_TIMEOUT: Duration = Duration::from_secs(10);

/// Envía `val` serializado en JSON en el flujo de escritura WS.
pub async fn send_json(
    write: &mut (impl SinkExt<Message, Error = WsError> + Unpin),
    val: Value,
) {
    write
        .send(Message::Text(val.to_string().into()))
        .await
        .unwrap();
}

/// Lee el flujo WS hasta la respuesta JSON-RPC que contiene `id == expected_id`.
/// Entra en pánico si no se produce ninguna respuesta en [`MAX_RESPONSE_FRAMES`] tramas
/// o si una trama supera [`FRAME_TIMEOUT`].
pub async fn recv_response_by_id(
    read: &mut (impl StreamExt<Item = Result<Message, WsError>> + Unpin),
    expected_id: u64,
) -> Value {
    for _ in 0..MAX_RESPONSE_FRAMES {
        let msg = tokio::time::timeout(FRAME_TIMEOUT, read.next())
            .await
            .expect("timeout waiting for RPC response")
            .expect("ws stream ended")
            .expect("ws frame error");
        if let Ok(text) = msg.to_text() {
            if let Ok(val) = serde_json::from_str::<Value>(text) {
                if val.get("id").and_then(|v| v.as_u64()) == Some(expected_id) {
                    return val;
                }
            }
        }
    }
    panic!("Did not receive response for request id {expected_id}");
}

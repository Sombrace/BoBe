use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use tracing::{debug, error};

use crate::speech::protocol::{ServerMessage, VoicePhase};
use crate::voice::output::VoiceOutput;

pub(crate) async fn close_with_error(mut socket: WebSocket, code: &str, message: &str) {
    let payload = ServerMessage::Error {
        code: code.to_string(),
        message: message.to_string(),
    };
    if let Ok(text) = serde_json::to_string(&payload) {
        if let Err(send_error) = socket.send(Message::Text(Utf8Bytes::from(text))).await {
            debug!(error = %send_error, "voice.handshake_error_send_failed");
        }
    } else {
        error!("voice.handshake_error_encode_failed");
    }
}

pub(crate) async fn send_state(output: &VoiceOutput, phase: VoicePhase, turn_id: &str) {
    send_json(
        output,
        &ServerMessage::State {
            phase,
            turn_id: turn_id.to_string(),
        },
    )
    .await;
}

pub(crate) async fn send_error(output: &VoiceOutput, code: &str, message: &str) {
    send_json(
        output,
        &ServerMessage::Error {
            code: code.to_string(),
            message: message.to_string(),
        },
    )
    .await;
}

pub(crate) async fn send_json(output: &VoiceOutput, msg: &ServerMessage) {
    if !output.control(msg.clone()).await {
        debug!("voice.control_send_closed");
    }
}

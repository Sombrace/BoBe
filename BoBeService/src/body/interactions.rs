use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::capabilities::{BodyCapabilities, BodyLimits};
use super::gateway::BodyGateway;
use super::outbound::{OutboundSender, send_json};
use super::protocol::PROTOCOL_MAJOR;
use crate::error::AppError;
use crate::runtime::message_requests::{RequestDelivery, RequestEvent};

const MAX_SENSORS: usize = 16;
const SENSOR_TTL: Duration = Duration::from_mins(5);
const TEXT_MAX_BYTES: usize = 1024;
const TEXT_CHUNK_BYTES: usize = 512;

#[derive(Default)]
pub(super) struct InteractionState {
    active: Option<(Uuid, CancellationToken)>,
}

#[derive(Default)]
pub(super) struct SensorState {
    pub(super) generation: u64,
    sensors: BTreeMap<String, (SensorSample, Instant, u64)>,
    last_sensor: Option<Instant>,
    sensor_tokens: f64,
}

pub(super) struct Connection {
    pub(super) generation: u64,
    pub(super) sender: OutboundSender,
    pub(super) disconnect: CancellationToken,
    pub(super) capabilities: BodyCapabilities,
    pub(super) limits: BodyLimits,
    pub(super) state: Arc<Mutex<InteractionState>>,
    pub(super) sensors: Arc<Mutex<SensorState>>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SensorSample {
    pub(crate) name: String,
    pub(crate) sequence: u32,
    pub(crate) value: Value,
    pub(crate) unit: String,
    pub(crate) observed_at: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextInput {
    r#type: String,
    protocol_major: u8,
    request_id: Uuid,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionInput {
    r#type: String,
    protocol_major: u8,
    request_id: Uuid,
    action: String,
    label: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelInput {
    r#type: String,
    protocol_major: u8,
    request_id: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SensorInput {
    r#type: String,
    protocol_major: u8,
    name: String,
    sequence: u32,
    value: Value,
    #[serde(default)]
    unit: String,
}

fn valid_name(name: &str, maximum: usize) -> bool {
    !name.is_empty()
        && name.len() <= maximum
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn scoped_request(device_id: &str, request_id: Uuid) -> Uuid {
    let hash = Sha256::digest(format!("bobe-device:{device_id}:{request_id}"));
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

async fn input_result(
    connection: &Connection,
    request_id: Uuid,
    status: &str,
    code: &str,
    replayed: bool,
) -> bool {
    send_json(
        &connection.sender,
        &json!({
            "type":"input.result", "protocol_major":PROTOCOL_MAJOR,
            "request_id":request_id, "status":status, "code":code, "replayed":replayed,
        }),
    )
    .await
}

async fn display_text(
    connection: &Connection,
    request_id: Uuid,
    sequence: &mut u32,
    text: &str,
    final_chunk: bool,
) -> bool {
    let maximum = ((connection.limits.max_control_bytes as usize).saturating_sub(256) / 6)
        .clamp(1, TEXT_CHUNK_BYTES);
    let normalized = text.replace('\0', "\u{fffd}");
    let mut remaining = normalized.as_str();
    while !remaining.is_empty() {
        let end = remaining.floor_char_boundary(maximum.min(remaining.len()));
        if end == 0 {
            return false;
        }
        if !send_json(
            &connection.sender,
            &json!({
                "type":"display.text", "protocol_major":PROTOCOL_MAJOR,
                "request_id":request_id, "sequence":*sequence,
                "clear":*sequence == 0, "final":false, "text":&remaining[..end],
            }),
        )
        .await
        {
            return false;
        }
        *sequence = sequence.saturating_add(1);
        remaining = &remaining[end..];
    }
    if final_chunk {
        return send_json(
            &connection.sender,
            &json!({
                "type":"display.text", "protocol_major":PROTOCOL_MAJOR,
                "request_id":request_id, "sequence":*sequence,
                "clear":*sequence == 0, "final":true, "text":"",
            }),
        )
        .await;
    }
    true
}

impl BodyGateway {
    pub(crate) async fn handle_device_input(
        self: &Arc<Self>,
        device_id: &str,
        generation: u64,
        kind: &str,
        value: &str,
    ) -> bool {
        let Some(connection) = self.interaction_connection(device_id, Some(generation)) else {
            return false;
        };
        match kind {
            "input.text" => {
                let Ok(input) = serde_json::from_str::<TextInput>(value) else {
                    return false;
                };
                if input.r#type != kind || input.protocol_major != PROTOCOL_MAJOR {
                    return false;
                }
                if !connection.capabilities.text_input {
                    return input_result(
                        &connection,
                        input.request_id,
                        "rejected",
                        "not_supported",
                        false,
                    )
                    .await;
                }
                if input.text.trim().is_empty()
                    || input.text.len() > TEXT_MAX_BYTES
                    || input.text.contains('\0')
                {
                    return input_result(
                        &connection,
                        input.request_id,
                        "rejected",
                        "invalid_text",
                        false,
                    )
                    .await;
                }
                self.submit_device_input(connection, device_id, input.request_id, input.text)
                    .await
            }
            "input.action" => {
                let Ok(input) = serde_json::from_str::<ActionInput>(value) else {
                    return false;
                };
                if input.r#type != kind || input.protocol_major != PROTOCOL_MAJOR {
                    return false;
                }
                if !connection.capabilities.actions {
                    return input_result(
                        &connection,
                        input.request_id,
                        "rejected",
                        "not_supported",
                        false,
                    )
                    .await;
                }
                if !valid_name(&input.action, 64)
                    || input.label.len() > 128
                    || input.label.contains('\0')
                {
                    return input_result(
                        &connection,
                        input.request_id,
                        "rejected",
                        "invalid_action",
                        false,
                    )
                    .await;
                }
                let content = format!(
                    "I pressed the device action '{}' ({}).",
                    input.action, input.label
                );
                self.submit_device_input(connection, device_id, input.request_id, content)
                    .await
            }
            "input.cancel" => {
                let Ok(input) = serde_json::from_str::<CancelInput>(value) else {
                    return false;
                };
                if input.r#type != kind || input.protocol_major != PROTOCOL_MAJOR {
                    return false;
                }
                let cancellation = connection
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .active
                    .as_ref()
                    .filter(|(id, _)| *id == input.request_id)
                    .map(|(_, cancel)| cancel.clone());
                if let Some(cancel) = cancellation {
                    cancel.cancel();
                    input_result(
                        &connection,
                        input.request_id,
                        "accepted",
                        "cancellation_requested",
                        false,
                    )
                    .await
                } else {
                    input_result(
                        &connection,
                        input.request_id,
                        "rejected",
                        "not_active",
                        false,
                    )
                    .await
                }
            }
            "sensor.sample" => self.record_device_sensor(&connection, value).await,
            _ => false,
        }
    }

    async fn submit_device_input(
        self: &Arc<Self>,
        connection: Connection,
        device_id: &str,
        request_id: Uuid,
        content: String,
    ) -> bool {
        let active = connection
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .active
            .as_ref()
            .map(|(id, _)| *id);
        if active.is_some_and(|id| id != request_id) {
            return input_result(&connection, request_id, "rejected", "busy", false).await;
        }
        let sensors = self.device_sensor_snapshot(device_id).unwrap_or_default();
        let context = json!({"device_id":device_id,"text_display":connection.capabilities.text_display,"sensors":sensors});
        let cancellation = connection.disconnect.child_token();
        let (sender, mut receiver) = mpsc::channel(8);
        let delivery = RequestDelivery {
            sender,
            cancellation: cancellation.clone(),
            device_context: context.to_string(),
        };
        let scoped = scoped_request(device_id, request_id);
        let accepted = match self
            .runtime
            .requests
            .submit(
                scoped,
                content,
                Some(delivery),
                crate::models::ids::ConversationChannelId::for_endpoint(device_id),
            )
            .await
        {
            Ok(value) => value,
            Err(error) => {
                let code = match error {
                    AppError::RequestReplayUnsafe(_) => "unsafe_retry",
                    AppError::Conflict(_) => "busy_or_conflict",
                    AppError::Validation(_) => "invalid_request",
                    _ => "request_failed",
                };
                return input_result(&connection, request_id, "rejected", code, false).await;
            }
        };
        if !accepted.started {
            if active.is_some() || matches!(accepted.request_status, "queued" | "running") {
                return input_result(&connection, request_id, "running", "", true).await;
            }
            if connection.capabilities.text_output {
                match self.runtime.requests.completed_response(scoped).await {
                    Ok(Some(response)) => {
                        if !display_text(&connection, request_id, &mut 0, &response, true).await {
                            return false;
                        }
                    }
                    _ => {
                        return input_result(
                            &connection,
                            request_id,
                            "rejected",
                            "response_unavailable",
                            true,
                        )
                        .await;
                    }
                }
            }
            return input_result(&connection, request_id, "completed", "", true).await;
        }
        connection
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .active = Some((request_id, cancellation.clone()));
        if !input_result(&connection, request_id, "accepted", "", accepted.replayed).await {
            cancellation.cancel();
            return false;
        }
        tokio::spawn(async move {
            let mut sequence = 0;
            while let Some(event) = receiver.recv().await {
                match event {
                    RequestEvent::Text(text) => {
                        if connection.capabilities.text_output
                            && !display_text(&connection, request_id, &mut sequence, &text, false)
                                .await
                        {
                            cancellation.cancel();
                            break;
                        }
                    }
                    RequestEvent::Finished { status } => {
                        if connection.capabilities.text_output {
                            let _delivered =
                                display_text(&connection, request_id, &mut sequence, "", true)
                                    .await;
                        }
                        let _delivered =
                            input_result(&connection, request_id, status, "", false).await;
                        break;
                    }
                }
            }
            let mut state = connection
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if state
                .active
                .as_ref()
                .is_some_and(|(id, _)| *id == request_id)
            {
                state.active = None;
            }
        });
        true
    }

    async fn record_device_sensor(&self, connection: &Connection, value: &str) -> bool {
        let Ok(input) = serde_json::from_str::<SensorInput>(value) else {
            return false;
        };
        if input.r#type != "sensor.sample"
            || input.protocol_major != PROTOCOL_MAJOR
            || !connection.capabilities.sensors
            || !valid_name(&input.name, 32)
            || input.unit.len() > 16
            || input.unit.bytes().any(|byte| byte.is_ascii_control())
            || !(input.value.is_boolean() || input.value.as_f64().is_some_and(f64::is_finite))
        {
            return false;
        }
        let now = Instant::now();
        let error = {
            let mut state = connection
                .sensors
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if state.generation != connection.generation || connection.disconnect.is_cancelled() {
                return false;
            }
            state
                .sensors
                .retain(|_, (_, time, _)| now.duration_since(*time) < SENSOR_TTL);
            state.sensor_tokens = state.last_sensor.map_or(10.0, |last| {
                (state.sensor_tokens + now.duration_since(last).as_secs_f64() * 10.0).min(10.0)
            });
            state.last_sensor = Some(now);
            let existing = state.sensors.get(&input.name);
            if existing.is_some_and(|(sample, _, recorded_generation)| {
                *recorded_generation == connection.generation && input.sequence <= sample.sequence
            }) {
                let same = existing.is_some_and(|(sample, _, _)| {
                    input.sequence == sample.sequence
                        && input.value == sample.value
                        && input.unit == sample.unit
                });
                (!same).then_some("stale_sensor_sequence")
            } else if state.sensor_tokens < 1.0 {
                Some("sensor_rate_limited")
            } else if !state.sensors.contains_key(&input.name) && state.sensors.len() >= MAX_SENSORS
            {
                Some("sensor_limit")
            } else {
                state.sensor_tokens -= 1.0;
                state.sensors.insert(
                    input.name.clone(),
                    (
                        SensorSample {
                            name: input.name.clone(),
                            sequence: input.sequence,
                            value: input.value,
                            unit: input.unit,
                            observed_at: Utc::now(),
                        },
                        now,
                        connection.generation,
                    ),
                );
                None
            }
        };
        if let Some(code) = error {
            return send_json(
                &connection.sender,
                &json!({"type":"error","protocol_major":PROTOCOL_MAJOR,
                "code":code,"message":"Sensor sample was not accepted"}),
            )
            .await;
        }
        send_json(
            &connection.sender,
            &json!({"type":"sensor.ack","protocol_major":PROTOCOL_MAJOR,
            "name":input.name,"sequence":input.sequence}),
        )
        .await
    }

    pub(crate) fn device_sensor_snapshot(&self, device_id: &str) -> Option<Vec<SensorSample>> {
        let sensors = self
            .sensor_states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(device_id)
            .cloned()?;
        let state = sensors.lock().unwrap_or_else(PoisonError::into_inner);
        Some(
            state
                .sensors
                .values()
                .filter(|(_, when, _)| when.elapsed() < SENSOR_TTL)
                .map(|(sample, _, _)| sample.clone())
                .collect(),
        )
    }

    pub(crate) fn clear_sensor_samples(&self) {
        for sensors in self
            .sensor_states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
        {
            sensors
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .sensors
                .clear();
        }
    }

    pub(crate) async fn send_device_text(
        &self,
        device_id: &str,
        text: &str,
    ) -> Result<Uuid, AppError> {
        if text.is_empty() || text.len() > 16 * 1024 || text.contains('\0') {
            return Err(AppError::Validation(
                "display text must contain 1 to 16384 UTF-8 bytes without NUL".into(),
            ));
        }
        let connection = self
            .interaction_connection(device_id, None)
            .ok_or_else(|| AppError::Conflict("device is not connected".into()))?;
        if !connection.capabilities.text_output {
            return Err(AppError::Validation(
                "device has no text output capability".into(),
            ));
        }
        let request_id = Uuid::new_v4();
        if !display_text(&connection, request_id, &mut 0, text, true).await {
            return Err(AppError::Conflict(
                "device disconnected during delivery".into(),
            ));
        }
        Ok(request_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_identity_is_scoped_to_the_device() {
        let request = Uuid::new_v4();
        assert_ne!(
            scoped_request("keyboard-a", request),
            scoped_request("keyboard-b", request)
        );
        assert_ne!(scoped_request("keyboard-a", request), request);
        assert_eq!(
            scoped_request("keyboard-a", request),
            scoped_request("keyboard-a", request)
        );
    }

    #[test]
    fn action_and_sensor_names_are_bounded_data() {
        assert!(valid_name("menu.next", 64));
        assert!(valid_name("temperature_c", 32));
        assert!(!valid_name("../../exec", 64));
        assert!(!valid_name("line\nbreak", 32));
        assert!(!valid_name(&"x".repeat(33), 32));
    }
}

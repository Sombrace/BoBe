use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::gateway::{AdapterInputDisposition, BodyGateway};
use super::identity::BodyPeerIdentity;
use super::outbound::{OutboundReceiver, OutboundSender, send_json, send_message};
use super::protocol::{
    ADAPTER_PROTOCOL_MINOR, ADAPTER_SUBPROTOCOL_V1, AUDIO_FRAME_DURATION_MS, AdapterAudioFormat,
    AdapterInbound, AdapterWelcome, BODY_SUBPROTOCOL_V1, BodyBatteryState, BodyHealthReport,
    BodyHello, BodyWelcome, CAPTURE_CHANNELS, CAPTURE_CODEC, CAPTURE_SAMPLE_RATE_HZ, CaptureCancel,
    CaptureDecision, CaptureRequest, MAX_CONTROL_BYTES, MediaHeader, OfferDecision, PROTOCOL_MAJOR,
    PlaybackCredit, RouteDescriptor, RoutedStreamControl, UpdateStateReport, control_type,
    negotiate_protocol_minor,
};

const CHANNEL_CAPACITY: usize = 64;
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const BODY_STALE_TIMEOUT: Duration = Duration::from_secs(35);
const SEND_TIMEOUT: Duration = Duration::from_secs(2);
const HEARTBEAT_INTERVAL_MS: u32 = 10_000;

fn spawn_writer(
    mut sink: futures::stream::SplitSink<WebSocket, Message>,
    mut receiver: OutboundReceiver,
    disconnect: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(outbound) = receiver.recv().await {
            let delivered = matches!(
                tokio::time::timeout(SEND_TIMEOUT, sink.send(outbound.message)).await,
                Ok(Ok(()))
            );
            if let Some(completion) = outbound.completion {
                let _ignored = completion.send(delivered);
            }
            if !delivered {
                break;
            }
        }
        drop(tokio::time::timeout(SEND_TIMEOUT, sink.close()).await);
        disconnect.cancel();
    })
}

async fn first_text(stream: &mut futures::stream::SplitStream<WebSocket>) -> Option<String> {
    let message = tokio::time::timeout(HELLO_TIMEOUT, stream.next())
        .await
        .ok()??;
    match message.ok()? {
        Message::Text(text) if text.len() <= MAX_CONTROL_BYTES => Some(text.to_string()),
        _ => None,
    }
}

fn select_subprotocol(mut upgrade: WebSocketUpgrade, expected: &str) -> Option<WebSocketUpgrade> {
    upgrade = upgrade
        .max_message_size(MAX_CONTROL_BYTES)
        .max_frame_size(MAX_CONTROL_BYTES);
    let selected = upgrade
        .requested_protocols()
        .find(|protocol| protocol.as_bytes() == expected.as_bytes())
        .cloned()?;
    upgrade.set_selected_protocol(selected);
    Some(upgrade)
}

pub(crate) async fn body_stream(
    upgrade: WebSocketUpgrade,
    State(gateway): State<Arc<BodyGateway>>,
    Extension(identity): Extension<BodyPeerIdentity>,
    Extension(endpoints): Extension<Arc<crate::db::endpoint_repo::EndpointRepo>>,
    Extension(registry_lock): Extension<Arc<tokio::sync::Mutex<()>>>,
) -> Response {
    let Some(upgrade) = select_subprotocol(upgrade, BODY_SUBPROTOCOL_V1) else {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    };
    upgrade
        .on_upgrade(move |socket| handle_body(socket, gateway, identity, endpoints, registry_lock))
        .into_response()
}

async fn handle_body(
    socket: WebSocket,
    gateway: Arc<BodyGateway>,
    identity: BodyPeerIdentity,
    endpoints: Arc<crate::db::endpoint_repo::EndpointRepo>,
    registry_lock: Arc<tokio::sync::Mutex<()>>,
) {
    let (sink, mut stream) = socket.split();
    let Some(hello_text) = first_text(&mut stream).await else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&hello_text) else {
        return;
    };
    if control_type(&value) != Some("hello") {
        return;
    }
    let Ok(hello) = serde_json::from_str::<BodyHello>(&hello_text) else {
        return;
    };
    let Some(negotiated_minor) = negotiate_protocol_minor(&hello) else {
        return;
    };
    let registry_guard = registry_lock.lock().await;
    let (capabilities, limits) = (hello.capabilities.clone(), hello.limits.clone());
    let Ok(Some(endpoint)) = endpoints.active_by_id(&hello.device_id).await else {
        return;
    };
    let Ok(approved) = serde_json::from_str::<serde_json::Value>(&endpoint.capabilities_json)
    else {
        return;
    };
    if !valid_hello(&hello)
        || identity.endpoint_id != hello.device_id
        || endpoint.kind != "physical_body"
        || endpoint.certificate_sha256.as_deref() != Some(identity.certificate_sha256.as_slice())
        || !capabilities.validates_registration(negotiated_minor, &limits)
        || !capabilities.authorized_by(negotiated_minor, &approved)
    {
        return;
    }
    if !(endpoint.protocol_min..=endpoint.protocol_max).contains(&i64::from(negotiated_minor)) {
        return;
    }
    let Ok(capabilities_json) = serde_json::to_string(&serde_json::json!({
        "capabilities": capabilities,
        "limits": limits,
    })) else {
        return;
    };
    match endpoints
        .update_observed_capabilities(&hello.device_id, &capabilities_json)
        .await
    {
        Ok(true) => {}
        Ok(false) | Err(_) => return,
    }

    let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
    let disconnect = CancellationToken::new();
    let registration = match gateway.register_body(
        hello.device_id.clone(),
        hello.boot_id.clone(),
        sender.clone(),
        disconnect.clone(),
        capabilities.clone(),
        limits,
    ) {
        Ok(registration) => registration,
        Err(reason) => {
            warn!(device_id = %hello.device_id, reason, "body.registration_rejected");
            return;
        }
    };
    drop(registry_guard);
    if let Some(route) = registration.superseded_route.as_ref() {
        gateway.release_route(route, "body_rebooted", true).await;
    }
    let writer = spawn_writer(sink, receiver, disconnect.clone());
    let session_id = registration.session_id.to_string();
    if !send_json(
        &sender,
        &BodyWelcome {
            r#type: "welcome",
            protocol_major: PROTOCOL_MAJOR,
            protocol_minor: negotiated_minor,
            session_id: &session_id,
            connection_generation: registration.generation,
            heartbeat_interval_ms: HEARTBEAT_INTERVAL_MS,
            controller_epoch: gateway.controller_epoch(),
        },
    )
    .await
    {
        gateway
            .unregister_body(&hello.device_id, registration.generation)
            .await;
        writer.abort();
        return;
    }
    if capabilities.display.is_some()
        && !gateway
            .send_current_theme_to_body(&hello.device_id, registration.generation)
            .await
    {
        gateway
            .unregister_body(&hello.device_id, registration.generation)
            .await;
        writer.abort();
        return;
    }
    info!(
        device_id = %hello.device_id,
        connection_generation = registration.generation,
        firmware = %hello.firmware,
        hardware = %hello.hardware,
        "body.connected"
    );

    loop {
        let next = tokio::select! {
            () = disconnect.cancelled() => break,
            next = tokio::time::timeout(BODY_STALE_TIMEOUT, stream.next()) => next,
        };
        let Ok(Some(Ok(message))) = next else {
            break;
        };
        if !gateway.touch_body(&hello.device_id, registration.generation) {
            break;
        }
        match message {
            Message::Text(text) => {
                if text.len() > MAX_CONTROL_BYTES
                    || !handle_body_control(
                        &gateway,
                        &hello.device_id,
                        registration.generation,
                        &sender,
                        text.as_str(),
                    )
                    .await
                {
                    break;
                }
            }
            Message::Binary(bytes) => {
                let Some(header) = MediaHeader::validate_microphone_frame(&bytes) else {
                    break;
                };
                let forwarded = match gateway
                    .active_route_for_body(&hello.device_id, registration.generation)
                {
                    Some(route) if route.capture_stream_id == header.stream_id => {
                        gateway
                            .forward_microphone(&route, header.sequence, header.flags, &bytes)
                            .await
                    }
                    _ => false,
                };
                if !forwarded
                    && !gateway.is_preempted_capture_stream(
                        &hello.device_id,
                        registration.generation,
                        header.stream_id,
                    )
                {
                    break;
                }
            }
            Message::Close(_) => break,
            Message::Ping(payload) => {
                if !send_message(&sender, Message::Pong(payload)).await {
                    break;
                }
            }
            Message::Pong(_) => {}
        }
    }

    gateway
        .unregister_body(&hello.device_id, registration.generation)
        .await;
    disconnect.cancel();
    writer.abort();
    drop(writer.await);
    info!(
        device_id = %hello.device_id,
        connection_generation = registration.generation,
        "body.disconnected"
    );
}

fn valid_hello(hello: &BodyHello) -> bool {
    !hello.device_id.is_empty()
        && hello.device_id.len() <= 64
        && hello.boot_id.len() == 32
        && hello.firmware.len() <= 64
        && hello.hardware.len() <= 64
}

async fn handle_body_control(
    gateway: &Arc<BodyGateway>,
    device_id: &str,
    generation: u64,
    sender: &OutboundSender,
    text: &str,
) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    if value
        .get("protocol_major")
        .and_then(serde_json::Value::as_u64)
        != Some(u64::from(PROTOCOL_MAJOR))
    {
        return false;
    }
    let Some(kind) = control_type(&value).map(str::to_owned) else {
        return false;
    };
    match kind.as_str() {
        "input.text" | "input.action" | "input.cancel" | "sensor.sample" => {
            gateway
                .handle_device_input(device_id, generation, &kind, text)
                .await
        }
        "capture.request" => {
            if !gateway
                .interaction_connection(device_id, Some(generation))
                .is_some_and(|connection| {
                    connection
                        .capabilities
                        .validates_audio_turn(&connection.limits)
                })
            {
                return false;
            }
            let Ok(request) = serde_json::from_value::<CaptureRequest>(value) else {
                return false;
            };
            if request.codec != CAPTURE_CODEC
                || request.sample_rate_hz != CAPTURE_SAMPLE_RATE_HZ
                || request.channels != CAPTURE_CHANNELS
                || request.frame_duration_ms != AUDIO_FRAME_DURATION_MS
                || !matches!(request.trigger.as_str(), "action_button" | "wake_word")
            {
                return false;
            }
            if request.trigger == "wake_word" {
                match gateway
                    .arbitrate_wake(device_id, &request.request_id, request.wake.as_ref())
                    .await
                {
                    crate::body::wake_arbitrator::WakeDecision::Winner => {}
                    crate::body::wake_arbitrator::WakeDecision::Lost => {
                        return send_json(
                            sender,
                            &CaptureDecision {
                                r#type: "capture.denied",
                                protocol_major: PROTOCOL_MAJOR,
                                request_id: &request.request_id,
                                stream_id: request.stream_id,
                                lease_id: None,
                                turn_id: None,
                                lease_ttl_ms: None,
                                reason: Some("wake_arbitration_lost"),
                            },
                        )
                        .await;
                    }
                    crate::body::wake_arbitrator::WakeDecision::ExplicitCapturePreempted => {
                        return send_json(
                            sender,
                            &CaptureDecision {
                                r#type: "capture.denied",
                                protocol_major: PROTOCOL_MAJOR,
                                request_id: &request.request_id,
                                stream_id: request.stream_id,
                                lease_id: None,
                                turn_id: None,
                                lease_ttl_ms: None,
                                reason: Some("explicit_capture_preempted"),
                            },
                        )
                        .await;
                    }
                }
            } else {
                gateway.preempt_pending_wake().await;
            }
            gateway
                .request_capture(device_id, generation, sender, &request)
                .await
        }
        "capture.cancel" => {
            let Ok(cancel) = serde_json::from_value::<CaptureCancel>(value) else {
                return false;
            };
            gateway
                .cancel_capture_request(device_id, generation, &cancel.request_id, cancel.stream_id)
                .await
        }
        "audio.capture.open" | "audio.capture.close" => {
            let Ok(control) = serde_json::from_value::<RoutedStreamControl>(value) else {
                return false;
            };
            let opening = kind == "audio.capture.open";
            let Some(route) = gateway.active_route_for_body(device_id, generation) else {
                return gateway
                    .refuse_preempted_capture(device_id, generation, &control, opening)
                    .await;
            };
            if !matches_route_control(&route, &control)
                || control.stream_id != route.capture_stream_id
            {
                return false;
            }
            if opening {
                gateway.open_capture(&route).await.is_ok()
                    || gateway
                        .refuse_preempted_capture(device_id, generation, &control, true)
                        .await
            } else if control
                .reason
                .as_deref()
                .is_some_and(|reason| !matches!(reason, "button_released" | "vad_end"))
            {
                gateway
                    .release_route(&route, "capture_failed_on_body", true)
                    .await;
                true
            } else {
                gateway.close_capture(&route).await.is_ok()
            }
        }
        "audio.playback.ready" | "audio.playback.ack" => {
            let Some(route) = gateway.active_route_for_body(device_id, generation) else {
                return false;
            };
            let Ok(credit) = serde_json::from_value::<PlaybackCredit>(value) else {
                return false;
            };
            if (kind == "audio.playback.ready" && credit.played_samples.is_some())
                || (kind == "audio.playback.ack" && credit.played_samples.is_none())
            {
                return false;
            }
            gateway.apply_playback_credit(&route, &credit).await
        }
        "audio.playback.drained" => {
            let Some(route) = gateway.active_route_for_body(device_id, generation) else {
                return false;
            };
            let Ok(control) = serde_json::from_value::<RoutedStreamControl>(value) else {
                return false;
            };
            matches_route_control(&route, &control)
                && control.reason.is_none()
                && gateway.playback_drained(&route, control.stream_id).await
        }
        "audio.playback.failed" => {
            let Some(route) = gateway.active_route_for_body(device_id, generation) else {
                return false;
            };
            let Ok(control) = serde_json::from_value::<RoutedStreamControl>(value) else {
                return false;
            };
            let Some(reason) = control
                .reason
                .as_deref()
                .filter(|reason| !reason.is_empty())
            else {
                return false;
            };
            matches_route_control(&route, &control)
                && gateway
                    .playback_failed(&route, control.stream_id, reason)
                    .await
        }
        "health.report" => {
            let Ok(health) = serde_json::from_value::<BodyHealthReport>(value) else {
                return false;
            };
            match gateway.classify_offer(device_id, generation, &health) {
                OfferDecision::Send => gateway.send_update_offer(device_id, generation).await,
                OfferDecision::AlreadyOffered
                | OfferDecision::RetryLater
                | OfferDecision::Suppressed
                | OfferDecision::NotNewer
                | OfferDecision::NotConfigured
                | OfferDecision::NoHealthYet
                | OfferDecision::StaleConnection => true,
            }
        }
        "battery.state" => {
            let Ok(battery) = serde_json::from_value::<BodyBatteryState>(value) else {
                return false;
            };
            if battery.percent > 100 {
                return false;
            }
            gateway.record_battery(device_id, generation, &battery);
            true
        }
        "update.state" => {
            let Ok(report) = serde_json::from_value::<UpdateStateReport>(value) else {
                return false;
            };
            if !report.fields_are_valid() {
                return false;
            }
            if !report.has_known_phase() {
                tracing::warn!(
                    device_id,
                    connection_generation = generation,
                    phase = %report.phase,
                    "body.update_state_unknown_phase"
                );
                return true;
            }
            gateway.record_update_state(device_id, generation, &report);
            true
        }
        "button.event" => true,
        _ => false,
    }
}

fn matches_route_control(route: &RouteDescriptor, control: &RoutedStreamControl) -> bool {
    control.lease_id == route.lease_id && control.turn_id == route.turn_id
}

pub(crate) async fn adapter_stream(
    upgrade: WebSocketUpgrade,
    State(gateway): State<Arc<BodyGateway>>,
) -> Response {
    let Some(upgrade) = select_subprotocol(upgrade, ADAPTER_SUBPROTOCOL_V1) else {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    };
    upgrade
        .on_upgrade(move |socket| handle_adapter(socket, gateway))
        .into_response()
}

async fn handle_adapter(socket: WebSocket, gateway: Arc<BodyGateway>) {
    let (sink, mut stream) = socket.split();
    let Some(hello_text) = first_text(&mut stream).await else {
        return;
    };
    let Ok(AdapterInbound::Hello {
        protocol_major,
        protocol_minor,
        capture_format,
    }) = serde_json::from_str::<AdapterInbound>(&hello_text)
    else {
        return;
    };
    if !valid_adapter_contract(protocol_major, protocol_minor, &capture_format) {
        return;
    }
    let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
    let disconnect = CancellationToken::new();
    let send_gate = Arc::new(AsyncMutex::new(()));
    let welcome_guard = send_gate.lock().await;
    let generation = match gateway.register_adapter(
        sender.clone(),
        Arc::clone(&send_gate),
        disconnect.clone(),
    ) {
        Ok(generation) => generation,
        Err(reason) => {
            warn!(reason, "body.adapter_registration_rejected");
            return;
        }
    };
    let writer = spawn_writer(sink, receiver, disconnect.clone());
    let welcome_sent = send_json(
        &sender,
        &AdapterWelcome {
            r#type: "adapter.welcome",
            protocol_major: PROTOCOL_MAJOR,
            protocol_minor: ADAPTER_PROTOCOL_MINOR,
            adapter_generation: generation,
        },
    )
    .await;
    drop(welcome_guard);
    if !welcome_sent {
        gateway.unregister_adapter(generation).await;
        writer.abort();
        return;
    }
    info!(adapter_generation = generation, "body.adapter_connected");

    loop {
        let message = tokio::select! {
            () = disconnect.cancelled() => break,
            message = stream.next() => message,
        };
        let Some(message) = message else {
            break;
        };
        let Ok(message) = message else {
            break;
        };
        let valid = match message {
            Message::Text(text) if text.len() <= MAX_CONTROL_BYTES => {
                match handle_adapter_control(&gateway, text.as_str()).await {
                    AdapterInputDisposition::Accepted | AdapterInputDisposition::Stale => true,
                    AdapterInputDisposition::Invalid => false,
                }
            }
            Message::Ping(payload) => send_message(&sender, Message::Pong(payload)).await,
            Message::Pong(_) => true,
            Message::Binary(_) | Message::Close(_) | Message::Text(_) => false,
        };
        if !valid {
            break;
        }
    }

    gateway.unregister_adapter(generation).await;
    disconnect.cancel();
    writer.abort();
    drop(writer.await);
    info!(adapter_generation = generation, "body.adapter_disconnected");
}

fn valid_adapter_contract(
    protocol_major: u8,
    protocol_minor: u8,
    capture: &AdapterAudioFormat,
) -> bool {
    protocol_major == PROTOCOL_MAJOR
        && protocol_minor == ADAPTER_PROTOCOL_MINOR
        && capture.codec == CAPTURE_CODEC
        && capture.sample_rate_hz == CAPTURE_SAMPLE_RATE_HZ
        && capture.channels == CAPTURE_CHANNELS
        && capture.frame_duration_ms == AUDIO_FRAME_DURATION_MS
}

async fn handle_adapter_control(gateway: &Arc<BodyGateway>, text: &str) -> AdapterInputDisposition {
    let Ok(message) = serde_json::from_str::<AdapterInbound>(text) else {
        return AdapterInputDisposition::Invalid;
    };
    match message {
        AdapterInbound::TranscriptPartial {
            connection_generation,
            lease_id,
            text,
        } => {
            let disposition = gateway
                .transcript_partial(connection_generation, lease_id, &text)
                .await;
            if disposition == AdapterInputDisposition::Stale {
                tracing::debug!("body.stale_adapter_partial_dropped");
            }
            disposition
        }
        AdapterInbound::TranscriptFinal {
            connection_generation,
            lease_id,
            text,
        } => {
            let disposition = gateway
                .transcript_final(connection_generation, lease_id, text)
                .await;
            if disposition == AdapterInputDisposition::Stale {
                tracing::debug!("body.stale_adapter_final_dropped");
            }
            disposition
        }
        AdapterInbound::TranscriptEmpty {
            connection_generation,
            lease_id,
        } => {
            let disposition = gateway
                .transcript_empty(connection_generation, lease_id)
                .await;
            if disposition == AdapterInputDisposition::Stale {
                tracing::debug!("body.stale_adapter_empty_dropped");
            }
            disposition
        }
        AdapterInbound::Hello { .. } => AdapterInputDisposition::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(codec: &str, sample_rate_hz: u32) -> AdapterAudioFormat {
        AdapterAudioFormat {
            codec: codec.to_owned(),
            sample_rate_hz,
            channels: 1,
            frame_duration_ms: 20,
        }
    }

    #[test]
    fn adapter_contract_rejects_version_and_media_drift() {
        let capture = format("pcm_s16le", 16_000);
        assert!(valid_adapter_contract(1, 1, &capture));
        assert!(!valid_adapter_contract(1, 0, &capture));
        assert!(!valid_adapter_contract(2, 1, &capture));
        assert!(!valid_adapter_contract(1, 1, &format("pcm_s16le", 24_000)));
        assert!(!valid_adapter_contract(1, 1, &format("opus", 16_000)));
    }

    fn deliver(mut outbound: OutboundReceiver) -> mpsc::UnboundedReceiver<serde_json::Value> {
        let (forward, delivered) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(message) = outbound.recv().await {
                if let Message::Text(text) = &message.message {
                    drop(forward.send(serde_json::from_str(text.as_str()).expect("daemon JSON")));
                }
                if let Some(completion) = message.completion {
                    let _delivered = completion.send(true);
                }
            }
        });
        delivered
    }

    async fn next(messages: &mut mpsc::UnboundedReceiver<serde_json::Value>) -> serde_json::Value {
        tokio::time::timeout(Duration::from_secs(5), messages.recv())
            .await
            .expect("the daemon answers within five seconds")
            .expect("the connection stays open")
    }

    fn connect_body(
        gateway: &BodyGateway,
        device_id: &str,
    ) -> (
        u64,
        OutboundSender,
        mpsc::UnboundedReceiver<serde_json::Value>,
    ) {
        let (sender, outbound) = mpsc::channel(CHANNEL_CAPACITY);
        let registration = gateway
            .register_body(
                device_id.to_owned(),
                "0".repeat(32),
                sender.clone(),
                CancellationToken::new(),
                super::super::capabilities::BodyCapabilities {
                    microphone: true,
                    speaker: true,
                    wake_word: true,
                    ..Default::default()
                },
                super::super::capabilities::BodyLimits {
                    max_control_bytes: MAX_CONTROL_BYTES as u32,
                    max_playback_credit_ms: super::super::protocol::MAX_PLAYBACK_CREDIT_MS,
                },
            )
            .expect("body registration");
        (registration.generation, sender, deliver(outbound))
    }

    fn capture_request(request_id: &str, stream_id: u32, trigger: &str) -> CaptureRequest {
        CaptureRequest {
            request_id: request_id.to_owned(),
            stream_id,
            trigger: trigger.to_owned(),
            wake: None,
            codec: CAPTURE_CODEC.to_owned(),
            sample_rate_hz: CAPTURE_SAMPLE_RATE_HZ,
            channels: CAPTURE_CHANNELS,
            frame_duration_ms: AUDIO_FRAME_DURATION_MS,
        }
    }

    fn capture_control(
        kind: &str,
        lease_id: &serde_json::Value,
        turn_id: &serde_json::Value,
        stream_id: u32,
    ) -> String {
        serde_json::json!({
            "type": kind,
            "protocol_major": PROTOCOL_MAJOR,
            "lease_id": lease_id,
            "turn_id": turn_id,
            "stream_id": stream_id,
            "codec": CAPTURE_CODEC,
            "sample_rate_hz": CAPTURE_SAMPLE_RATE_HZ,
            "channels": CAPTURE_CHANNELS,
            "frame_duration_ms": AUDIO_FRAME_DURATION_MS,
            "reason": if kind == "audio.capture.close" { Some("vad_end") } else { None },
        })
        .to_string()
    }

    #[tokio::test]
    async fn an_action_button_preempts_a_granted_wake_word_lease_without_dropping_either_body() {
        let root =
            std::env::temp_dir().join(format!("bobe-body-preemption-{}", uuid::Uuid::new_v4()));
        let gateway = BodyGateway::for_tests(&root).await;
        let (adapter, adapter_outbound) = mpsc::channel(CHANNEL_CAPACITY);
        gateway
            .register_adapter(
                adapter,
                Arc::new(AsyncMutex::new(())),
                CancellationToken::new(),
            )
            .expect("speech adapter");
        let mut adapter_messages = deliver(adapter_outbound);
        let (a, a_sender, mut a_messages) = connect_body(&gateway, "body-a");
        let (b, b_sender, mut b_messages) = connect_body(&gateway, "body-b");

        assert!(
            gateway
                .request_capture(
                    "body-a",
                    a,
                    &a_sender,
                    &capture_request("wake-a", 11, "wake_word")
                )
                .await
        );
        let granted_a = next(&mut a_messages).await;
        assert_eq!(granted_a["type"], "capture.granted");
        let (lease_a, turn_a) = (granted_a["lease_id"].clone(), granted_a["turn_id"].clone());

        assert!(
            gateway
                .request_capture(
                    "body-b",
                    b,
                    &b_sender,
                    &capture_request("button-b", 22, "action_button"),
                )
                .await
        );
        let cancelled = next(&mut adapter_messages).await;
        assert_eq!(cancelled["type"], "turn.cancel");
        assert_eq!(cancelled["reason"], "explicit_capture_preempted");
        assert_eq!(cancelled["route"]["lease_id"], lease_a);
        let face = next(&mut a_messages).await;
        assert_eq!(face["type"], "face.set");
        assert_eq!(face["expression"], "idle");
        let ended = next(&mut a_messages).await;
        assert_eq!(ended["type"], "turn.state");
        assert_eq!(ended["state"], "idle");
        assert_eq!((&ended["lease_id"], &ended["turn_id"]), (&lease_a, &turn_a));
        let granted_b = next(&mut b_messages).await;
        assert_eq!(granted_b["type"], "capture.granted");
        assert_eq!(
            gateway
                .active_route_for_body("body-b", b)
                .map(|route| serde_json::json!(route.lease_id)),
            Some(granted_b["lease_id"].clone())
        );
        assert!(gateway.active_route_for_body("body-a", a).is_none());

        let open = capture_control("audio.capture.open", &lease_a, &turn_a, 11);
        assert!(handle_body_control(&gateway, "body-a", a, &a_sender, &open).await);
        let refused = next(&mut a_messages).await;
        assert_eq!(refused["type"], "turn.state");
        assert_eq!(refused["state"], "aborted");
        assert_eq!(
            (&refused["lease_id"], &refused["turn_id"]),
            (&lease_a, &turn_a)
        );
        assert!(gateway.is_preempted_capture_stream("body-a", a, 11));
        let close = capture_control("audio.capture.close", &lease_a, &turn_a, 11);
        assert!(handle_body_control(&gateway, "body-a", a, &a_sender, &close).await);
        let forged = capture_control(
            "audio.capture.open",
            &serde_json::json!(uuid::Uuid::new_v4()),
            &turn_a,
            11,
        );
        assert!(!handle_body_control(&gateway, "body-a", a, &a_sender, &forged).await);

        assert!(
            gateway
                .request_capture(
                    "body-a",
                    a,
                    &a_sender,
                    &capture_request("wake-a-2", 12, "wake_word")
                )
                .await
        );
        let denied = next(&mut a_messages).await;
        assert_eq!(denied["type"], "capture.denied");
        assert_eq!(denied["reason"], "another_endpoint_owns_voice_lease");
        assert!(!gateway.is_preempted_capture_stream("body-a", a, 11));
        assert!(b_messages.try_recv().is_err());
        drop(tokio::fs::remove_dir_all(root).await);
    }
}

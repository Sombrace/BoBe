use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use axum::extract::ws::Message;
use opusic_c::{Channels as OpusChannels, Decoder as OpusDecoder, SampleRate as OpusSampleRate};
use serde::Serialize;
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::outbound::{
    OutboundSender, send_json, send_json_confirmed, send_message, send_message_confirmed,
    try_send_message,
};
use super::protocol::{
    AUDIO_FRAME_DURATION_MS, AdapterMediaHeader, AdapterMediaKind, AdapterOutbound,
    BodyBatteryState, BodyHealthReport, CAPTURE_CHANNELS, CAPTURE_CODEC, CAPTURE_SAMPLE_RATE_HZ,
    MAX_LEASE_LIFETIME_MS, MAX_PLAYBACK_CREDIT_MS, MEDIA_FLAG_DISCONTINUITY, OfferDecision,
    PROTOCOL_MAJOR, PlaybackCredit, RouteDescriptor, SPEAKER_CHANNELS, SPEAKER_CODEC,
    SPEAKER_PCM_BYTES, SPEAKER_SAMPLE_RATE_HZ, ThemeSet, UpdateStateReport,
};
use super::update::{OfferManifest, UpdateOffer};
use crate::config::{BodyTheme, Config};
use crate::models::ids::new_turn_id;
use crate::runtime::response_streamer::StreamDelivery;
use crate::runtime::session::{RuntimeSession, UserMessageGuard};
use crate::speech::protocol::{ServerMessage, VoicePhase};
use crate::voice::context::VoiceContext;
use crate::voice::engines::{VoiceEngines, VoiceEnginesSnapshot};
use crate::voice::output::{VoiceOutput, VoiceOutputFrame};
use crate::voice::run_text_turn::{VoiceTurnAdmission, run_text_turn};
use crate::voice::session::{SessionVoiceConfig, VoiceDefaults};
use crate::voice::sinks::VoiceSink;

const LEASE_TTL: Duration = Duration::from_mins(2);
const LEASE_HARD_TTL: Duration = Duration::from_millis(MAX_LEASE_LIFETIME_MS as u64);
const CAPTURE_IDLE_TTL: Duration = LEASE_TTL;
const TRANSCRIPTION_TTL: Duration = Duration::from_secs(45);
const GENERATION_PROGRESS_TTL: Duration = Duration::from_mins(2);
const PLAYBACK_PROGRESS_TTL: Duration = Duration::from_secs(30);
const MAX_PLAYBACK_CREDIT_FRAMES: usize =
    MAX_PLAYBACK_CREDIT_MS as usize / AUDIO_FRAME_DURATION_MS as usize;
const MAX_PENDING_PLAYBACK_FRAMES: usize = 1_500;

const OUTPUT_CHANNEL_CAPACITY: usize = 64;
const CAPTION_MAX_BYTES: usize = 140;
const BODY_STATUS_DISABLED: u8 = 0;
const BODY_STATUS_STARTING: u8 = 1;
const BODY_STATUS_RUNNING: u8 = 2;
const BODY_STATUS_FAILED: u8 = 3;
const BODY_STATUS_STOPPED: u8 = 4;

#[derive(Clone)]
pub(crate) struct BodyRuntime {
    pub(crate) requests: Arc<crate::runtime::message_requests::MessageRequests>,
    pub(crate) config: Arc<ArcSwap<Config>>,
    pub(crate) runtime_session: Arc<RuntimeSession>,
    pub(crate) voice_engines: Arc<ArcSwap<VoiceEnginesSnapshot>>,
    pub(crate) voice_sink: Arc<VoiceSink>,
}

#[derive(Debug, Clone)]
pub(crate) struct BodyRegistration {
    pub(crate) generation: u64,
    pub(crate) session_id: Uuid,
    pub(crate) superseded_route: Option<RouteDescriptor>,
}

#[derive(Debug, Clone)]
pub(crate) struct LeaseGrant {
    pub(crate) route: RouteDescriptor,
    pub(crate) ttl: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdapterInputDisposition {
    Accepted,
    Stale,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdapterOutputDisposition {
    Sent,
    Stale,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeasePhase {
    Granted,
    Capturing,
    Transcribing,
    Generating,
    Playing,
    Draining,
    Releasing,
}

fn classify_adapter_route(
    route: &RouteDescriptor,
    generation: u64,
    lease_id: Uuid,
    input_is_expected: bool,
) -> AdapterInputDisposition {
    if route.connection_generation != generation || route.lease_id != lease_id {
        AdapterInputDisposition::Stale
    } else if input_is_expected {
        AdapterInputDisposition::Accepted
    } else {
        AdapterInputDisposition::Invalid
    }
}

struct BodyConnection {
    generation: u64,
    session_id: Uuid,
    sender: OutboundSender,
    send_gate: Arc<AsyncMutex<()>>,
    disconnect: CancellationToken,
    last_seen: Instant,
    offered_build: Option<u32>,
    capabilities: super::capabilities::BodyCapabilities,
    limits: super::capabilities::BodyLimits,
    interactions: Arc<Mutex<super::interactions::InteractionState>>,
    sensors: Arc<Mutex<super::interactions::SensorState>>,
}

fn offer_decision(
    offer_build: u32,
    running_build: u32,
    already_offered: Option<u32>,
) -> OfferDecision {
    if running_build == 0 {
        OfferDecision::NoHealthYet
    } else if running_build >= offer_build {
        OfferDecision::NotNewer
    } else if already_offered == Some(offer_build) {
        OfferDecision::AlreadyOffered
    } else {
        OfferDecision::Send
    }
}

fn should_replace_body(existing: Option<&BodyConnection>) -> bool {
    existing.is_some()
}

#[derive(Clone)]
struct AdapterConnection {
    generation: u64,
    sender: OutboundSender,
    send_gate: Arc<AsyncMutex<()>>,
    disconnect: CancellationToken,
}

struct PlaybackState {
    stream_id: u32,
    next_body_sequence: u32,
    ready_received: bool,
    credit_frames: usize,
    frames_sent: u64,
    played_samples: u64,
    pending: VecDeque<Vec<u8>>,
    closing: bool,
    close_sent: bool,
    drained: bool,
}

fn new_playback_state() -> PlaybackState {
    PlaybackState {
        stream_id: rand::random::<u32>().max(1),
        next_body_sequence: 0,
        ready_received: false,
        credit_frames: 0,
        frames_sent: 0,
        played_samples: 0,
        pending: VecDeque::new(),
        closing: false,
        close_sent: false,
        drained: false,
    }
}

fn accept_capture_sequence(expected: &mut Option<u32>, sequence: u32, flags: u16) -> bool {
    if expected.is_some_and(|expected| expected != sequence)
        && flags & MEDIA_FLAG_DISCONTINUITY == 0
    {
        return false;
    }
    *expected = Some(sequence.wrapping_add(1));
    true
}

fn decode_opus_body_pcm(decoder: &mut OpusDecoder, packet: &[u8]) -> Result<Vec<u8>, String> {
    let mut samples = vec![0_u16; 480];
    let decoded = decoder
        .decode_to_slice(packet, &mut samples, false)
        .map_err(|error| format!("decode body Opus packet: {error:?}"))?;
    if decoded != 480 {
        return Err(format!(
            "body Opus packet decoded to {decoded} samples; expected 480"
        ));
    }
    Ok(samples
        .into_iter()
        .flat_map(|sample| (sample as i16).to_le_bytes())
        .collect())
}

struct VoiceLease {
    route: RouteDescriptor,
    request_id: String,
    trigger: String,
    phase: LeasePhase,
    deadline: Instant,
    hard_deadline: Instant,
    admission: Option<UserMessageGuard>,
    expiry_abort: Option<tokio::task::AbortHandle>,
    turn_abort: Option<tokio::task::AbortHandle>,
    turn_finished: Option<tokio::sync::watch::Receiver<bool>>,
    generation_complete: bool,
    stt_stream: Option<Box<dyn crate::speech::SttStream>>,
    next_capture_sequence: Option<u32>,
    opus_decoder: Option<OpusDecoder>,
    playback: Option<PlaybackState>,
}

impl VoiceLease {
    fn advance(&mut self, phase: LeasePhase, ttl: Duration) {
        self.phase = phase;
        self.deadline = (Instant::now() + ttl).min(self.hard_deadline);
    }

    fn touch(&mut self, ttl: Duration) {
        self.deadline = (Instant::now() + ttl).min(self.hard_deadline);
    }
}

struct LeaseReleaseContext {
    expiry_abort: Option<tokio::task::AbortHandle>,
    turn_abort: Option<tokio::task::AbortHandle>,
    turn_finished: Option<tokio::sync::watch::Receiver<bool>>,
    adapter: Option<AdapterConnection>,
}

struct GatewayState {
    next_generation: u64,
    bodies: HashMap<String, BodyConnection>,
    adapter: Option<AdapterConnection>,
    lease: Option<VoiceLease>,
}

impl GatewayState {
    fn new() -> Self {
        Self {
            next_generation: 1,
            bodies: HashMap::new(),
            adapter: None,
            lease: None,
        }
    }

    fn take_generation(&mut self) -> u64 {
        let generation = self.next_generation;
        self.next_generation = self.next_generation.strict_add(1);
        generation
    }

    fn lease_matches(lease: &VoiceLease, route: &RouteDescriptor) -> bool {
        lease.route == *route
    }

    fn lease_is_routable(lease: &VoiceLease, route: &RouteDescriptor) -> bool {
        Self::lease_matches(lease, route) && lease.phase != LeasePhase::Releasing
    }

    fn begin_lease_release(&mut self, route: &RouteDescriptor) -> Option<LeaseReleaseContext> {
        let lease = self.lease.as_mut()?;
        if !Self::lease_matches(lease, route) || lease.phase == LeasePhase::Releasing {
            return None;
        }
        lease.phase = LeasePhase::Releasing;
        Some(LeaseReleaseContext {
            expiry_abort: lease.expiry_abort.take(),
            turn_abort: lease.turn_abort.take(),
            turn_finished: lease.turn_finished.take(),
            adapter: self.adapter.clone(),
        })
    }

    fn finish_lease_release(&mut self, route: &RouteDescriptor) -> bool {
        if self.lease.as_ref().is_some_and(|lease| {
            Self::lease_matches(lease, route) && lease.phase == LeasePhase::Releasing
        }) {
            self.lease.take();
            return true;
        }
        false
    }

    fn remove_adapter(&mut self, generation: u64) -> Option<CancellationToken> {
        if self
            .adapter
            .as_ref()
            .is_some_and(|adapter| adapter.generation == generation)
        {
            return self.adapter.take().map(|adapter| adapter.disconnect);
        }
        None
    }
}

type SensorStates = HashMap<String, Arc<Mutex<super::interactions::SensorState>>>;

pub(crate) struct BodyGateway {
    state: Mutex<GatewayState>,
    pub(super) runtime: BodyRuntime,
    controller_epoch: u64,
    face_version: AtomicU64,
    theme_version: AtomicU64,
    status: AtomicU8,
    wake_arbitrator: crate::body::wake_arbitrator::WakeArbitrator,
    clock_origin: Instant,
    update_offer: Option<Arc<UpdateOffer>>,
    update_statuses: Mutex<HashMap<String, BodyUpdateStatus>>,
    pub(super) sensor_states: Mutex<SensorStates>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct BodyUpdateStatus {
    #[serde(skip)]
    generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) health: Option<BodyHealthReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) battery: Option<BodyBatteryState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) offered_manifest: Option<OfferManifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) offered_at_generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) update_state: Option<UpdateStateReport>,
}

struct LeaseReleaseGuard {
    gateway: Arc<BodyGateway>,
    route: RouteDescriptor,
    armed: bool,
}

impl LeaseReleaseGuard {
    fn new(gateway: Arc<BodyGateway>, route: RouteDescriptor) -> Self {
        Self {
            gateway,
            route,
            armed: true,
        }
    }

    fn finish(mut self) {
        self.gateway.lock_state().finish_lease_release(&self.route);
        self.armed = false;
    }
}

impl Drop for LeaseReleaseGuard {
    fn drop(&mut self) {
        if self.armed {
            self.gateway.lock_state().finish_lease_release(&self.route);
        }
    }
}

impl BodyGateway {
    pub(crate) fn new(
        controller_epoch: u64,
        enabled: bool,
        runtime: BodyRuntime,
        update_offer: Option<Arc<UpdateOffer>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(GatewayState::new()),
            runtime,
            controller_epoch,
            face_version: AtomicU64::new(1),
            theme_version: AtomicU64::new(1),
            status: AtomicU8::new(if enabled {
                BODY_STATUS_STARTING
            } else {
                BODY_STATUS_DISABLED
            }),
            wake_arbitrator: crate::body::wake_arbitrator::WakeArbitrator::default(),
            clock_origin: Instant::now(),
            update_offer,
            update_statuses: Mutex::new(HashMap::new()),
            sensor_states: Mutex::new(HashMap::new()),
        })
    }

    fn lock_update_statuses(&self) -> std::sync::MutexGuard<'_, HashMap<String, BodyUpdateStatus>> {
        self.update_statuses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn update_statuses(&self) -> HashMap<String, BodyUpdateStatus> {
        self.lock_update_statuses().clone()
    }

    pub(crate) fn configured_offer_build(&self) -> Option<u32> {
        self.update_offer.as_ref().map(|offer| offer.build_number())
    }

    fn is_current_body(&self, device_id: &str, generation: u64) -> bool {
        self.lock_state()
            .bodies
            .get(device_id)
            .is_some_and(|body| body.generation == generation)
    }

    fn with_current_update_status(
        &self,
        device_id: &str,
        generation: u64,
        update: impl FnOnce(&mut BodyUpdateStatus),
    ) -> bool {
        if !self.is_current_body(device_id, generation) {
            return false;
        }
        let mut statuses = self.lock_update_statuses();
        let Some(status) = statuses.get_mut(device_id) else {
            return false;
        };
        if status.generation != generation {
            return false;
        }
        update(status);
        true
    }

    fn remove_update_status(&self, device_id: &str, generation: u64) {
        let mut statuses = self.lock_update_statuses();
        if statuses
            .get(device_id)
            .is_some_and(|status| status.generation == generation)
        {
            statuses.remove(device_id);
        }
    }

    pub(crate) fn record_battery_state(
        &self,
        device_id: &str,
        generation: u64,
        battery: BodyBatteryState,
    ) -> bool {
        self.with_current_update_status(device_id, generation, |status| {
            status.battery = Some(battery);
        })
    }

    pub(crate) fn record_health_report(
        &self,
        device_id: &str,
        generation: u64,
        health: BodyHealthReport,
    ) -> OfferDecision {
        let build_number = health.build_number;
        if !self.with_current_update_status(device_id, generation, |status| {
            status.health = Some(health);
        }) {
            return OfferDecision::StaleConnection;
        }
        self.classify_offer(device_id, generation, build_number)
    }

    fn classify_offer(&self, device_id: &str, generation: u64, build_number: u32) -> OfferDecision {
        let Some(offer) = self.update_offer.as_ref() else {
            return OfferDecision::NotConfigured;
        };
        let offer_build = offer.build_number();
        let mut state = self.lock_state();
        let Some(body) = state
            .bodies
            .get_mut(device_id)
            .filter(|body| body.generation == generation)
        else {
            return OfferDecision::StaleConnection;
        };
        let decision = offer_decision(offer_build, build_number, body.offered_build);
        if decision == OfferDecision::Send {
            body.offered_build = Some(offer_build);
        }
        decision
    }

    pub(crate) async fn send_update_offer(&self, device_id: &str, generation: u64) -> bool {
        let Some(offer) = self.update_offer.as_ref() else {
            return true;
        };
        let connection = {
            let state = self.lock_state();
            state
                .bodies
                .get(device_id)
                .filter(|body| body.generation == generation)
                .map(|body| (body.sender.clone(), Arc::clone(&body.send_gate)))
        };
        let Some((sender, send_gate)) = connection else {
            return false;
        };
        let _send_guard = send_gate.lock().await;
        if !self.is_current_body(device_id, generation) {
            return false;
        }
        let delivered =
            send_message_confirmed(&sender, Message::Text(offer.envelope().to_owned().into()))
                .await;
        if delivered {
            self.with_current_update_status(device_id, generation, |status| {
                status.offered_manifest = Some(offer.manifest().clone());
                status.offered_at_generation = Some(generation);
                status.update_state = None;
            });
            tracing::info!(
                device_id,
                connection_generation = generation,
                offer_build = offer.build_number(),
                offer_version = %offer.manifest().version,
                key_id = %offer.manifest().key_id,
                "body.update_offer_sent"
            );
        } else {
            let mut state = self.lock_state();
            if let Some(body) = state
                .bodies
                .get_mut(device_id)
                .filter(|body| body.generation == generation)
            {
                body.offered_build = None;
            }
            tracing::warn!(
                device_id,
                connection_generation = generation,
                "body.update_offer_send_failed"
            );
        }
        delivered
    }

    pub(crate) fn record_update_state(
        &self,
        device_id: &str,
        generation: u64,
        report: UpdateStateReport,
    ) {
        let phase = report.phase.clone();
        let progress_percent = report.progress_percent;
        let reason = report.reason.clone();
        if self.with_current_update_status(device_id, generation, |status| {
            status.update_state = Some(report);
        }) {
            tracing::info!(
                device_id,
                connection_generation = generation,
                phase,
                progress_percent,
                reason = reason.as_deref().unwrap_or(""),
                "body.update_state"
            );
        } else {
            tracing::debug!(
                device_id,
                connection_generation = generation,
                "body.update_state_stale"
            );
        }
    }

    pub(crate) async fn cancel_capture_request(
        self: &Arc<Self>,
        device_id: &str,
        generation: u64,
        request_id: &str,
        stream_id: u32,
    ) -> bool {
        let route = {
            let state = self.lock_state();
            state
                .lease
                .as_ref()
                .filter(|lease| {
                    lease.route.device_id == device_id
                        && lease.route.connection_generation == generation
                        && lease.route.capture_stream_id == stream_id
                        && lease.request_id == request_id
                        && matches!(lease.phase, LeasePhase::Granted | LeasePhase::Capturing)
                })
                .map(|lease| lease.route.clone())
        };
        let Some(route) = route else {
            return true;
        };
        self.release_route(&route, "capture_canceled_before_open", true)
            .await;
        true
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, GatewayState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) const fn controller_epoch(&self) -> u64 {
        self.controller_epoch
    }

    pub(crate) fn mark_running(&self) {
        self.status.store(BODY_STATUS_RUNNING, Ordering::Release);
    }

    pub(crate) fn mark_failed(&self) {
        self.status.store(BODY_STATUS_FAILED, Ordering::Release);
    }

    pub(crate) fn mark_stopped(&self) {
        self.status.store(BODY_STATUS_STOPPED, Ordering::Release);
    }

    pub(crate) fn health_status(&self) -> &'static str {
        match self.status.load(Ordering::Acquire) {
            BODY_STATUS_DISABLED => "disabled",
            BODY_STATUS_STARTING => "starting",
            BODY_STATUS_RUNNING => "ok",
            BODY_STATUS_FAILED => "error",
            BODY_STATUS_STOPPED => "stopped",
            _ => "unknown",
        }
    }

    pub(crate) fn is_failed(&self) -> bool {
        matches!(
            self.status.load(Ordering::Acquire),
            BODY_STATUS_FAILED | BODY_STATUS_STOPPED
        )
    }

    pub(crate) fn has_connected_body(&self) -> bool {
        !self.lock_state().bodies.is_empty()
    }

    pub(crate) async fn send_current_theme_to_body(
        &self,
        device_id: &str,
        generation: u64,
    ) -> bool {
        self.send_theme_to_body(device_id, generation).await
    }

    pub(crate) async fn apply_theme(&self, _theme: BodyTheme) -> bool {
        let bodies = {
            let state = self.lock_state();
            state
                .bodies
                .iter()
                .map(|(device_id, body)| (device_id.clone(), body.generation))
                .collect::<Vec<_>>()
        };
        let mut delivered = false;
        for (device_id, generation) in bodies {
            delivered |= self.send_theme_to_body(&device_id, generation).await;
        }
        delivered
    }

    async fn send_theme_to_body(&self, device_id: &str, generation: u64) -> bool {
        let connection = {
            let state = self.lock_state();
            state
                .bodies
                .get(device_id)
                .filter(|body| body.generation == generation)
                .map(|body| (body.sender.clone(), Arc::clone(&body.send_gate)))
        };
        let Some((sender, send_gate)) = connection else {
            return false;
        };
        let _send_guard = send_gate.lock().await;
        let current = self
            .lock_state()
            .bodies
            .get(device_id)
            .is_some_and(|body| body.generation == generation);
        if !current {
            return false;
        }
        let theme = self.runtime.config.load().body.theme;
        let state_version = self.theme_version.fetch_add(1, Ordering::AcqRel);
        send_json_confirmed(
            &sender,
            &ThemeSet {
                r#type: "theme.set",
                protocol_major: PROTOCOL_MAJOR,
                state_version,
                theme_id: theme,
            },
        )
        .await
    }

    pub(crate) fn register_body(
        &self,
        device_id: String,
        _boot_id: String,
        sender: OutboundSender,
        disconnect: CancellationToken,
        capabilities: super::capabilities::BodyCapabilities,
        limits: super::capabilities::BodyLimits,
    ) -> Result<BodyRegistration, &'static str> {
        let mut state = self.lock_state();
        let replace = should_replace_body(state.bodies.get(&device_id));
        let previous = replace.then(|| state.bodies.remove(&device_id)).flatten();
        let superseded_route = previous
            .as_ref()
            .and_then(|body| {
                state.lease.as_ref().filter(|lease| {
                    lease.route.device_id == device_id
                        && lease.route.connection_generation == body.generation
                })
            })
            .map(|lease| lease.route.clone());
        let generation = state.take_generation();
        let session_id = Uuid::new_v4();
        let sensors = Arc::clone(
            self.sensor_states
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(device_id.clone())
                .or_default(),
        );
        sensors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .generation = generation;
        state.bodies.insert(
            device_id.clone(),
            BodyConnection {
                generation,
                session_id,
                sender,
                send_gate: Arc::new(AsyncMutex::new(())),
                disconnect,
                last_seen: Instant::now(),
                offered_build: None,
                capabilities,
                limits,
                interactions: Arc::new(
                    Mutex::new(super::interactions::InteractionState::default()),
                ),
                sensors,
            },
        );
        self.lock_update_statuses().insert(
            device_id.clone(),
            BodyUpdateStatus {
                generation,
                ..BodyUpdateStatus::default()
            },
        );
        drop(state);
        if let Some(previous) = previous {
            previous.disconnect.cancel();
        }
        Ok(BodyRegistration {
            generation,
            session_id,
            superseded_route,
        })
    }

    pub(super) fn interaction_connection(
        &self,
        device_id: &str,
        generation: Option<u64>,
    ) -> Option<super::interactions::Connection> {
        let state = self.lock_state();
        let body = state.bodies.get(device_id)?;
        if generation.is_some_and(|value| value != body.generation)
            || body.disconnect.is_cancelled()
        {
            return None;
        }
        Some(super::interactions::Connection {
            generation: body.generation,
            sender: body.sender.clone(),
            disconnect: body.disconnect.clone(),
            capabilities: body.capabilities.clone(),
            limits: body.limits.clone(),
            state: Arc::clone(&body.interactions),
            sensors: Arc::clone(&body.sensors),
        })
    }

    pub(crate) async fn revoke_endpoint(self: &Arc<Self>, endpoint_id: &str) {
        self.sensor_states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(endpoint_id);
        let (route, connection) = {
            let mut state = self.lock_state();
            let connection = state.bodies.remove(endpoint_id);
            if let Some(body) = connection.as_ref() {
                self.remove_update_status(endpoint_id, body.generation);
            }
            let route = state
                .lease
                .as_ref()
                .filter(|lease| lease.route.device_id == endpoint_id)
                .map(|lease| lease.route.clone());
            (route, connection)
        };
        if let Some(connection) = connection {
            connection.disconnect.cancel();
        }
        if let Some(route) = route {
            self.release_route(&route, "endpoint_revoked", true).await;
        }
    }

    pub(crate) async fn unregister_body(self: &Arc<Self>, device_id: &str, generation: u64) {
        let route = {
            let mut state = self.lock_state();
            let current = state
                .bodies
                .get(device_id)
                .is_some_and(|body| body.generation == generation);
            if !current {
                return;
            }
            state.bodies.remove(device_id);
            self.remove_update_status(device_id, generation);
            state
                .lease
                .as_ref()
                .filter(|lease| {
                    lease.route.device_id == device_id
                        && lease.route.connection_generation == generation
                })
                .map(|lease| lease.route.clone())
        };
        if let Some(route) = route {
            self.release_route(&route, "body_disconnected", true).await;
        }
    }

    pub(crate) fn touch_body(&self, device_id: &str, generation: u64) -> bool {
        let mut state = self.lock_state();
        let Some(body) = state.bodies.get_mut(device_id) else {
            return false;
        };
        if body.generation != generation {
            return false;
        }
        body.last_seen = Instant::now();
        true
    }

    pub(crate) fn register_adapter(
        &self,
        sender: OutboundSender,
        send_gate: Arc<AsyncMutex<()>>,
        disconnect: CancellationToken,
    ) -> Result<u64, &'static str> {
        let mut state = self.lock_state();
        if state.adapter.is_some() {
            return Err("adapter_already_connected");
        }
        let generation = state.take_generation();
        state.adapter = Some(AdapterConnection {
            generation,
            sender,
            send_gate,
            disconnect,
        });
        Ok(generation)
    }

    pub(crate) async fn unregister_adapter(self: &Arc<Self>, generation: u64) {
        let (route, disconnect) = {
            let mut state = self.lock_state();
            if state
                .adapter
                .as_ref()
                .is_none_or(|adapter| adapter.generation != generation)
            {
                return;
            }
            let disconnect = state.remove_adapter(generation);
            (
                state
                    .lease
                    .as_ref()
                    .filter(|lease| lease.stt_stream.is_none())
                    .map(|lease| lease.route.clone()),
                disconnect,
            )
        };
        if let Some(disconnect) = disconnect {
            disconnect.cancel();
        }
        if let Some(route) = route {
            self.release_route(&route, "speech_adapter_disconnected", false)
                .await;
        }
    }

    pub(crate) async fn arbitrate_wake(
        &self,
        endpoint_id: &str,
        request_id: &str,
        metadata: Option<&crate::body::protocol::WakeCandidateMetadata>,
    ) -> crate::body::wake_arbitrator::WakeDecision {
        if let Some(metadata) = metadata {
            tracing::trace!(
                endpoint_id,
                model_id = metadata.model_id,
                detected_monotonic_us = metadata.detected_monotonic_us,
                "body.wake_candidate"
            );
        }
        let candidate = crate::body::wake_arbitrator::WakeCandidate {
            endpoint_id: endpoint_id.to_owned(),
            request_id: request_id.to_owned(),
            confidence: metadata.map_or(0.5, |metadata| metadata.confidence),
            snr_db: metadata.map_or(0.0, |metadata| metadata.snr_db),
            energy_dbfs: metadata.map_or(-60.0, |metadata| metadata.energy_dbfs),
        };
        self.wake_arbitrator.submit(candidate).await
    }

    pub(crate) async fn preempt_pending_wake(&self) {
        self.wake_arbitrator.preempt_pending().await;
    }

    pub(crate) fn acquire_lease(
        &self,
        device_id: &str,
        generation: u64,
        request_id: &str,
        trigger: &str,
        stream_id: u32,
    ) -> Result<LeaseGrant, &'static str> {
        if request_id.is_empty() || request_id.len() > 64 || stream_id == 0 {
            return Err("invalid_capture_request");
        }
        let mut state = self.lock_state();
        let daemon_stt = self
            .runtime
            .voice_engines
            .load()
            .stt
            .as_ref()
            .filter(|engine| engine.health().is_viable())
            .map(Arc::clone);
        if state.adapter.is_none() && daemon_stt.is_none() {
            return Err("speech_engine_unavailable");
        }
        if let Some(existing) = state.lease.as_ref() {
            let can_preempt = trigger == "action_button"
                && existing.trigger == "wake_word"
                && existing.phase == LeasePhase::Granted;
            if !can_preempt {
                return Err("another_endpoint_owns_voice_lease");
            }
            if let Some(mut preempted) = state.lease.take() {
                if let Some(abort) = preempted.expiry_abort.take() {
                    abort.abort();
                }
                if let Some(abort) = preempted.turn_abort.take() {
                    abort.abort();
                }
            }
        }
        let Some(body) = state.bodies.get(device_id) else {
            return Err("body_not_connected");
        };
        if body.generation != generation {
            return Err("stale_connection_generation");
        }
        let admission = self
            .runtime
            .runtime_session
            .try_begin_user_message(crate::models::ids::ConversationChannelId::for_endpoint(
                device_id,
            ))
            .map_err(|_| "agent_turn_unavailable")?;
        let route = RouteDescriptor {
            device_id: device_id.to_owned(),
            connection_generation: generation,
            body_session_id: body.session_id,
            lease_id: Uuid::new_v4(),
            turn_id: new_turn_id(),
            capture_stream_id: stream_id,
        };
        let now = Instant::now();
        let stt_stream = daemon_stt
            .map(|engine| engine.create_stream(false))
            .transpose()
            .map_err(|_| "speech_engine_unavailable")?;
        state.lease = Some(VoiceLease {
            route: route.clone(),
            request_id: request_id.to_owned(),
            trigger: trigger.to_owned(),
            phase: LeasePhase::Granted,
            deadline: now + LEASE_TTL,
            hard_deadline: now + LEASE_HARD_TTL,
            admission: Some(admission),
            expiry_abort: None,
            turn_abort: None,
            turn_finished: None,
            generation_complete: false,
            stt_stream,
            next_capture_sequence: None,
            opus_decoder: OpusDecoder::new(OpusChannels::Mono, OpusSampleRate::Hz24000).ok(),
            playback: None,
        });
        Ok(LeaseGrant {
            route,
            ttl: LEASE_TTL,
        })
    }

    pub(crate) fn attach_expiry_abort(
        &self,
        route: &RouteDescriptor,
        abort: tokio::task::AbortHandle,
    ) -> bool {
        let mut state = self.lock_state();
        let Some(lease) = state.lease.as_mut() else {
            return false;
        };
        if !GatewayState::lease_is_routable(lease, route) {
            return false;
        }
        lease.expiry_abort = Some(abort);
        true
    }

    pub(crate) async fn open_capture(
        self: &Arc<Self>,
        route: &RouteDescriptor,
    ) -> Result<(), &'static str> {
        let adapter = {
            let mut state = self.lock_state();
            let Some(lease) = state.lease.as_mut() else {
                return Err("voice_lease_missing");
            };
            if !GatewayState::lease_is_routable(lease, route) {
                return Err("stale_voice_route");
            }
            if lease.phase != LeasePhase::Granted {
                return Err("invalid_capture_state");
            }
            lease.advance(LeasePhase::Capturing, CAPTURE_IDLE_TTL);
            if lease.stt_stream.is_some() {
                return Ok(());
            }
            state.adapter.clone().ok_or("speech_adapter_unavailable")?
        };
        let message = AdapterOutbound::CaptureOpen {
            route: route.clone(),
            codec: CAPTURE_CODEC,
            sample_rate_hz: CAPTURE_SAMPLE_RATE_HZ,
            channels: CAPTURE_CHANNELS,
            frame_duration_ms: AUDIO_FRAME_DURATION_MS,
        };
        match self
            .send_active_adapter_json(route, &adapter, &message)
            .await
        {
            AdapterOutputDisposition::Sent => Ok(()),
            AdapterOutputDisposition::Stale => Err("stale_voice_route"),
            AdapterOutputDisposition::Failed => {
                self.release_route(route, "speech_adapter_send_failed", false)
                    .await;
                Err("speech_adapter_send_failed")
            }
        }
    }

    pub(crate) async fn close_capture(
        self: &Arc<Self>,
        route: &RouteDescriptor,
    ) -> Result<(), &'static str> {
        let direct_events = {
            let mut state = self.lock_state();
            let Some(lease) = state.lease.as_mut() else {
                return Err("voice_lease_missing");
            };
            if !GatewayState::lease_is_routable(lease, route) {
                return Err("stale_voice_route");
            }
            if lease.phase != LeasePhase::Capturing {
                return Err("invalid_capture_state");
            }
            lease.advance(LeasePhase::Transcribing, TRANSCRIPTION_TTL);
            lease
                .stt_stream
                .as_mut()
                .map(|stream| stream.finish())
                .transpose()
                .map_err(|_| "speech_recognition_failed")?
        };
        if let Some(events) = direct_events {
            if events
                .iter()
                .any(|event| matches!(event, crate::speech::SttEvent::Final(_)))
            {
                return self.handle_daemon_stt_events(route, events).await;
            }
            return match self
                .transcript_empty(route.connection_generation, route.lease_id)
                .await
            {
                AdapterInputDisposition::Accepted => Ok(()),
                _ => Err("speech_recognition_failed"),
            };
        }
        let adapter = self
            .lock_state()
            .adapter
            .clone()
            .ok_or("speech_adapter_unavailable")?;
        match self
            .send_active_adapter_json(
                route,
                &adapter,
                &AdapterOutbound::CaptureClose {
                    route: route.clone(),
                },
            )
            .await
        {
            AdapterOutputDisposition::Sent => Ok(()),
            AdapterOutputDisposition::Stale => Err("stale_voice_route"),
            AdapterOutputDisposition::Failed => {
                self.release_route(route, "speech_adapter_send_failed", false)
                    .await;
                Err("speech_adapter_send_failed")
            }
        }
    }

    pub(crate) async fn forward_microphone(
        self: &Arc<Self>,
        route: &RouteDescriptor,
        sequence: u32,
        flags: u16,
        frame: &[u8],
    ) -> bool {
        let direct_events = {
            let mut state = self.lock_state();
            let Some(lease) = state.lease.as_mut() else {
                return false;
            };
            let active = GatewayState::lease_is_routable(lease, route)
                && lease.phase == LeasePhase::Capturing;
            if !active
                || !accept_capture_sequence(&mut lease.next_capture_sequence, sequence, flags)
            {
                return false;
            }
            lease.touch(CAPTURE_IDLE_TTL);
            lease
                .stt_stream
                .as_mut()
                .map(|stream| {
                    let payload = frame
                        .get(super::protocol::MEDIA_HEADER_LEN..)
                        .ok_or_else(|| "microphone frame is missing its PCM payload".to_owned())?;
                    stream.accept_pcm_s16le(payload)
                })
                .transpose()
        };
        match direct_events {
            Ok(Some(events)) => {
                return self.handle_daemon_stt_events(route, events).await.is_ok();
            }
            Err(_) => {
                self.release_route(route, "speech_recognition_failed", false)
                    .await;
                return false;
            }
            Ok(None) => {}
        }
        let adapter = self.lock_state().adapter.clone();
        let Some(adapter) = adapter else {
            self.release_route(route, "speech_adapter_unavailable", false)
                .await;
            return false;
        };
        let encoded = AdapterMediaHeader {
            kind: AdapterMediaKind::MicrophonePcm,
            connection_generation: route.connection_generation,
            lease_id: route.lease_id,
            stream_id: route.capture_stream_id,
            sequence,
        }
        .encode(frame);
        match self
            .send_active_adapter_message(route, &adapter, Message::Binary(encoded.into()))
            .await
        {
            AdapterOutputDisposition::Sent => true,
            AdapterOutputDisposition::Stale => false,
            AdapterOutputDisposition::Failed => {
                self.release_route(route, "speech_adapter_send_failed", false)
                    .await;
                false
            }
        }
    }

    async fn handle_daemon_stt_events(
        self: &Arc<Self>,
        route: &RouteDescriptor,
        events: Vec<crate::speech::SttEvent>,
    ) -> Result<(), &'static str> {
        for event in events {
            match event {
                crate::speech::SttEvent::Partial(text) => {
                    if self
                        .transcript_partial(route.connection_generation, route.lease_id, &text)
                        .await
                        != AdapterInputDisposition::Accepted
                    {
                        return Err("stale_voice_route");
                    }
                }
                crate::speech::SttEvent::Final(text) => {
                    if self
                        .transcript_final(route.connection_generation, route.lease_id, text)
                        .await
                        != AdapterInputDisposition::Accepted
                    {
                        return Err("speech_recognition_failed");
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn transcript_partial(
        &self,
        generation: u64,
        lease_id: Uuid,
        text: &str,
    ) -> AdapterInputDisposition {
        let route = {
            let mut state = self.lock_state();
            let Some(lease) = state.lease.as_mut() else {
                return AdapterInputDisposition::Stale;
            };
            let disposition = classify_adapter_route(
                &lease.route,
                generation,
                lease_id,
                matches!(
                    lease.phase,
                    LeasePhase::Capturing | LeasePhase::Transcribing
                ),
            );
            if disposition != AdapterInputDisposition::Accepted {
                return disposition;
            }
            lease.touch(TRANSCRIPTION_TTL);
            lease.route.clone()
        };
        self.send_face(&route, "listening", &bounded_caption(text), 2_000)
            .await;
        AdapterInputDisposition::Accepted
    }

    pub(crate) async fn transcript_empty(
        self: &Arc<Self>,
        generation: u64,
        lease_id: Uuid,
    ) -> AdapterInputDisposition {
        let route = {
            let state = self.lock_state();
            let Some(lease) = state.lease.as_ref() else {
                return AdapterInputDisposition::Stale;
            };
            let disposition = classify_adapter_route(
                &lease.route,
                generation,
                lease_id,
                lease.phase == LeasePhase::Transcribing,
            );
            if disposition != AdapterInputDisposition::Accepted {
                return disposition;
            }
            lease.route.clone()
        };
        self.release_route(&route, "empty_transcript", true).await;
        AdapterInputDisposition::Accepted
    }

    pub(crate) async fn transcript_final(
        self: &Arc<Self>,
        generation: u64,
        lease_id: Uuid,
        text: String,
    ) -> AdapterInputDisposition {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return self.transcript_empty(generation, lease_id).await;
        }
        if trimmed.chars().count() > 16_000 {
            return AdapterInputDisposition::Invalid;
        }
        let route = {
            let mut state = self.lock_state();
            let Some(lease) = state.lease.as_mut() else {
                return AdapterInputDisposition::Stale;
            };
            let disposition = classify_adapter_route(
                &lease.route,
                generation,
                lease_id,
                lease.phase == LeasePhase::Transcribing,
            );
            if disposition != AdapterInputDisposition::Accepted {
                return disposition;
            }
            lease.advance(LeasePhase::Generating, GENERATION_PROGRESS_TTL);
            if lease.admission.is_none() {
                return AdapterInputDisposition::Invalid;
            }
            lease.route.clone()
        };
        self.spawn_turn(route, trimmed.to_owned()).await;
        AdapterInputDisposition::Accepted
    }

    async fn spawn_turn(self: &Arc<Self>, route: RouteDescriptor, text: String) -> bool {
        let config = self.runtime.config.load_full();
        let defaults = VoiceDefaults::from_config(&config);
        let engines = VoiceEngines::from_snapshot(&self.runtime.voice_engines.load());
        if !defaults.enabled || !engines.supports_server_tts() {
            self.release_route(&route, "voice_engine_unavailable", true)
                .await;
            return false;
        }
        let speech_plan = crate::speech::SpeechPlan::body_v1(&defaults);
        let voice_cfg = SessionVoiceConfig::new(None, speech_plan, &defaults);
        let (output, mut output_rx) = VoiceOutput::channel(OUTPUT_CHANNEL_CAPACITY);
        let (completion_tx, _completion_rx) = mpsc::channel(1);
        let context = VoiceContext {
            conversation_channel: crate::models::ids::ConversationChannelId::for_endpoint(
                &route.device_id,
            ),
            networked_runtime: false,
            output: output.clone(),
            runtime_session: Arc::clone(&self.runtime.runtime_session),
            engines,
            voice_defaults: defaults,
            voice_sink: Arc::clone(&self.runtime.voice_sink),
            delivery: StreamDelivery::EndpointOnly,
            turn_completion_tx: completion_tx,
        };

        let gateway_for_output = Arc::clone(self);
        let route_for_output = route.clone();
        let output_task = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            while let Some(frame) = output_rx.recv().await {
                gateway_for_output
                    .handle_voice_output(&route_for_output, frame)
                    .await;
            }
        }));
        let turn_id = route.turn_id.clone();
        let turn_task = tokio::spawn(async move {
            run_text_turn(
                &text,
                &turn_id,
                &context,
                voice_cfg,
                None,
                Arc::default(),
                VoiceTurnAdmission::AlreadyHeld,
            )
            .await;
            drop(context);
            drop(output);
            if let Err(error) = output_task.await {
                tracing::warn!(%error, "body.output_task_join_failed");
            }
        });
        let abort = turn_task.abort_handle();
        let (finished, completion) = tokio::sync::watch::channel(false);
        {
            let mut state = self.lock_state();
            let Some(lease) = state.lease.as_mut() else {
                turn_task.abort();
                return false;
            };
            if !GatewayState::lease_is_routable(lease, &route) {
                turn_task.abort();
                return false;
            }
            lease.turn_abort = Some(abort);
            lease.turn_finished = Some(completion);
        }

        let gateway = Arc::clone(self);
        tokio::spawn(async move {
            match turn_task.await {
                Ok(()) => {}
                Err(error) if error.is_cancelled() => {
                    tracing::debug!("body.turn_task_cancelled");
                }
                Err(error) => tracing::error!(%error, "body.turn_task_failed"),
            }
            if let Ok(turn) = crate::models::ids::conversation_turn_id_from_wire_id(&route.turn_id)
            {
                gateway
                    .runtime
                    .runtime_session
                    .settle_interrupted_turn(turn)
                    .await;
            }
            finished.send_replace(true);
            gateway.turn_task_finished(&route).await;
        });
        true
    }

    async fn handle_voice_output(
        self: &Arc<Self>,
        route: &RouteDescriptor,
        frame: VoiceOutputFrame,
    ) {
        if !self.route_is_active(route) {
            return;
        }
        match frame {
            VoiceOutputFrame::Control(message) => {
                self.handle_voice_control(route, message).await;
            }
            VoiceOutputFrame::Audio(bytes) => {
                self.forward_tts_opus(route, &bytes).await;
            }
            VoiceOutputFrame::Ping => {}
        }
    }

    async fn handle_voice_control(
        self: &Arc<Self>,
        route: &RouteDescriptor,
        message: ServerMessage,
    ) {
        match message {
            ServerMessage::State { phase, .. } => match phase {
                VoicePhase::Thinking => {
                    self.send_turn_state(route, "thinking").await;
                    self.send_face(route, "thinking", "Thinking...", 0).await;
                }
                VoicePhase::Speaking => {
                    self.send_turn_state(route, "speaking").await;
                    self.send_face(route, "speaking", "", 0).await;
                }
                VoicePhase::Idle | VoicePhase::Listening => {
                    if !self.has_active_playback(route) {
                        self.send_turn_state(route, "waiting_for_runtime").await;
                    }
                }
            },
            ServerMessage::TranscriptFinal { text, .. } => {
                self.send_face(route, "thinking", &bounded_caption(&text), 2_000)
                    .await;
            }
            ServerMessage::TtsText { text, .. } => {
                self.send_face(route, "speaking", &bounded_caption(&text), 5_000)
                    .await;
            }
            ServerMessage::TtsEnd { .. } => {
                self.mark_tts_end(route).await;
            }
            ServerMessage::Error { code, message } => {
                self.fail_route(route, &code, &message, "voice_turn_error")
                    .await;
            }
            ServerMessage::Truncate { .. } => {
                self.release_route(route, "voice_turn_truncated", true)
                    .await;
            }
            ServerMessage::SpeechPlan { .. }
            | ServerMessage::CaptureCredit { .. }
            | ServerMessage::SttPartial { .. }
            | ServerMessage::SttFinal { .. } => {}
        }
    }

    async fn forward_tts_opus(self: &Arc<Self>, route: &RouteDescriptor, bytes: &[u8]) {
        let pcm = {
            let mut state = self.lock_state();
            let Some(lease) = state.lease.as_mut() else {
                return;
            };
            if !GatewayState::lease_is_routable(lease, route) {
                return;
            }
            lease
                .opus_decoder
                .as_mut()
                .ok_or_else(|| "body Opus decoder is unavailable".to_owned())
                .and_then(|decoder| {
                    let frame = crate::speech::protocol::decode_tts_frame(bytes)
                        .ok_or_else(|| "invalid body TTS frame".to_owned())?;
                    tracing::trace!(
                        chunk_id = frame.chunk_id,
                        flags = frame.flags,
                        "body.tts_frame"
                    );
                    decode_opus_body_pcm(decoder, frame.payload)
                })
        };
        match pcm {
            Ok(pcm) => self.queue_body_pcm(route, pcm).await,
            Err(error) => {
                self.fail_route(
                    route,
                    "body_audio_decode_failed",
                    &error,
                    "body_audio_decode_failed",
                )
                .await;
            }
        }
    }

    async fn queue_body_pcm(self: &Arc<Self>, route: &RouteDescriptor, pcm: Vec<u8>) {
        if pcm.len() != SPEAKER_PCM_BYTES {
            self.release_route(route, "body_audio_frame_invalid", false)
                .await;
            return;
        }
        let (stream_id, open_body, queued) = {
            let mut state = self.lock_state();
            let Some(lease) = state.lease.as_mut() else {
                return;
            };
            if !GatewayState::lease_is_routable(lease, route)
                || !matches!(lease.phase, LeasePhase::Generating | LeasePhase::Playing)
            {
                return;
            }
            lease.touch(PLAYBACK_PROGRESS_TTL);
            let open_body = lease.playback.is_none();
            if open_body {
                lease.playback = Some(new_playback_state());
                lease.advance(LeasePhase::Playing, PLAYBACK_PROGRESS_TTL);
            }
            let Some(playback) = lease.playback.as_mut() else {
                return;
            };
            let queued = playback.pending.len() < MAX_PENDING_PLAYBACK_FRAMES;
            if queued {
                playback.pending.push_back(pcm);
            }
            (playback.stream_id, open_body, queued)
        };
        if !queued {
            self.fail_route(
                route,
                "playback_backlog_overflow",
                "Response audio exceeded the bounded playback backlog",
                "generated_audio_limit_exceeded",
            )
            .await;
            return;
        }
        if open_body && !self.open_body_playback(route, stream_id).await {
            self.release_route(route, "body_control_queue_full", true)
                .await;
            return;
        }
        self.flush_playback(route).await;
    }

    async fn open_body_playback(&self, route: &RouteDescriptor, stream_id: u32) -> bool {
        self.send_active_body_json(
            route,
            serde_json::json!({
                "type": "audio.playback.open",
                "protocol_major": PROTOCOL_MAJOR,
                "lease_id": route.lease_id,
                "turn_id": route.turn_id,
                "stream_id": stream_id,
                "codec": SPEAKER_CODEC,
                "sample_rate_hz": SPEAKER_SAMPLE_RATE_HZ,
                "channels": SPEAKER_CHANNELS,
                "frame_duration_ms": AUDIO_FRAME_DURATION_MS,
            }),
        )
        .await
    }

    pub(crate) async fn apply_playback_credit(
        self: &Arc<Self>,
        route: &RouteDescriptor,
        credit: &PlaybackCredit,
    ) -> bool {
        let valid = {
            let mut state = self.lock_state();
            let Some(lease) = state.lease.as_mut() else {
                return false;
            };
            if !GatewayState::lease_is_routable(lease, route)
                || credit.lease_id != route.lease_id
                || credit.turn_id != route.turn_id
            {
                return false;
            }
            let Some(playback) = lease.playback.as_mut() else {
                return false;
            };
            if playback.stream_id != credit.stream_id || credit.credit_ms > MAX_PLAYBACK_CREDIT_MS {
                return false;
            }
            if let Some(played_samples) = credit.played_samples {
                if played_samples % 480 != 0
                    || played_samples > playback.frames_sent.saturating_mul(480)
                {
                    return false;
                }
                if played_samples <= playback.played_samples {
                    return true;
                }
                playback.played_samples = played_samples;
            } else if playback.ready_received {
                return true;
            }
            playback.ready_received = true;
            let played_frames = playback.played_samples / 480;
            let outstanding_frames = playback.frames_sent.saturating_sub(played_frames) as usize;
            let derived_credit = MAX_PLAYBACK_CREDIT_FRAMES.saturating_sub(outstanding_frames);
            playback.credit_frames = (credit.credit_ms as usize / 20)
                .min(MAX_PLAYBACK_CREDIT_FRAMES)
                .min(derived_credit);
            lease.touch(PLAYBACK_PROGRESS_TTL);
            true
        };
        if valid {
            self.flush_playback(route).await;
            self.release_if_drained(route).await;
        }
        valid
    }

    pub(crate) async fn playback_drained(
        self: &Arc<Self>,
        route: &RouteDescriptor,
        stream_id: u32,
    ) -> bool {
        let valid = {
            let mut state = self.lock_state();
            let Some(lease) = state.lease.as_mut() else {
                return false;
            };
            if !GatewayState::lease_is_routable(lease, route) || lease.phase != LeasePhase::Draining
            {
                return false;
            }
            let Some(playback) = lease.playback.as_mut() else {
                return false;
            };
            if playback.stream_id != stream_id || !playback.close_sent {
                return false;
            }
            playback.drained = true;
            true
        };
        if valid {
            self.release_if_drained(route).await;
        }
        valid
    }

    pub(crate) async fn playback_failed(
        self: &Arc<Self>,
        route: &RouteDescriptor,
        stream_id: u32,
        reason: &str,
    ) -> bool {
        let valid = {
            let state = self.lock_state();
            state.lease.as_ref().is_some_and(|lease| {
                GatewayState::lease_is_routable(lease, route)
                    && lease
                        .playback
                        .as_ref()
                        .is_some_and(|playback| playback.stream_id == stream_id)
            })
        };
        if valid {
            self.fail_route(
                route,
                "body_playback_failed",
                &format!("Physical playback failed: {}", bounded_caption(reason)),
                "body_playback_failed",
            )
            .await;
        }
        valid
    }

    async fn flush_playback(self: &Arc<Self>, route: &RouteDescriptor) {
        let connection = {
            let state = self.lock_state();
            state
                .bodies
                .get(&route.device_id)
                .filter(|body| body.generation == route.connection_generation)
                .map(|body| (body.sender.clone(), Arc::clone(&body.send_gate)))
        };
        let Some((sender, send_gate)) = connection else {
            self.release_route(route, "body_disconnected", true).await;
            return;
        };

        let send_failure = {
            let _send_guard = send_gate.lock().await;
            let (frames, close, stream_id) = {
                let mut state = self.lock_state();
                let Some(lease) = state.lease.as_mut() else {
                    return;
                };
                if !GatewayState::lease_is_routable(lease, route) {
                    return;
                }
                let Some(playback) = lease.playback.as_mut() else {
                    return;
                };
                let stream_id = playback.stream_id;
                let mut frames = Vec::new();
                while playback.credit_frames > 0 {
                    let Some(pcm) = playback.pending.pop_front() else {
                        break;
                    };
                    let sequence = playback.next_body_sequence;
                    playback.next_body_sequence = playback.next_body_sequence.wrapping_add(1);
                    playback.credit_frames -= 1;
                    playback.frames_sent = playback.frames_sent.saturating_add(1);
                    frames.push((sequence, pcm));
                }
                let close = playback.closing && playback.pending.is_empty() && !playback.close_sent;
                if close {
                    playback.close_sent = true;
                    lease.advance(LeasePhase::Draining, PLAYBACK_PROGRESS_TTL);
                }
                (frames, close, stream_id)
            };

            let mut failure = None;
            for (sequence, pcm) in frames {
                let Some(frame) = super::protocol::MediaHeader::encode_speaker_frame(
                    stream_id,
                    sequence,
                    self.clock_origin.elapsed().as_micros() as u64,
                    &pcm,
                ) else {
                    failure = Some("speaker_frame_encode_failed");
                    break;
                };
                if !send_message(&sender, Message::Binary(frame.into())).await {
                    failure = Some("body_send_failed");
                    break;
                }
            }
            if failure.is_none()
                && close
                && !send_json(
                    &sender,
                    &serde_json::json!({
                        "type": "audio.playback.close",
                        "protocol_major": PROTOCOL_MAJOR,
                        "lease_id": route.lease_id,
                        "turn_id": route.turn_id,
                        "stream_id": stream_id,
                    }),
                )
                .await
            {
                failure = Some("body_control_queue_full");
            }
            failure
        };

        if let Some(reason) = send_failure {
            self.release_route(route, reason, true).await;
        }
    }

    async fn mark_tts_end(self: &Arc<Self>, route: &RouteDescriptor) {
        let has_playback = {
            let mut state = self.lock_state();
            let Some(lease) = state.lease.as_mut() else {
                return;
            };
            if !GatewayState::lease_is_routable(lease, route) {
                return;
            }
            if let Some(playback) = lease.playback.as_mut() {
                playback.closing = true;
                true
            } else {
                false
            }
        };
        if has_playback {
            self.flush_playback(route).await;
            self.release_if_drained(route).await;
        }
    }

    async fn turn_task_finished(self: &Arc<Self>, route: &RouteDescriptor) {
        let release = {
            let mut state = self.lock_state();
            let Some(lease) = state.lease.as_mut() else {
                return;
            };
            if !GatewayState::lease_is_routable(lease, route) {
                return;
            }
            lease.generation_complete = true;
            lease.turn_abort = None;
            lease.playback.is_none()
        };
        if release {
            self.release_route(route, "turn_complete", false).await;
        } else {
            self.release_if_drained(route).await;
        }
    }

    async fn release_if_drained(self: &Arc<Self>, route: &RouteDescriptor) {
        let drained = {
            let state = self.lock_state();
            state.lease.as_ref().is_some_and(|lease| {
                GatewayState::lease_is_routable(lease, route)
                    && lease.generation_complete
                    && lease.playback.as_ref().is_some_and(|playback| {
                        playback.close_sent && playback.pending.is_empty() && playback.drained
                    })
            })
        };
        if drained {
            if let Ok(turn) = crate::models::ids::conversation_turn_id_from_wire_id(&route.turn_id)
                && let Err(error) = self
                    .runtime
                    .runtime_session
                    .acknowledge_presentation(
                        turn,
                        crate::models::ids::ConversationChannelId::for_endpoint(&route.device_id),
                    )
                    .await
            {
                tracing::warn!(%error,turn_id=%route.turn_id,"body.presentation_receipt_failed");
            }
            self.release_route(route, "playback_drained", false).await;
        }
    }

    pub(crate) async fn watch_lease_expiry(self: &Arc<Self>, route: &RouteDescriptor) {
        loop {
            let deadline = {
                let state = self.lock_state();
                let Some(lease) = state
                    .lease
                    .as_ref()
                    .filter(|lease| GatewayState::lease_is_routable(lease, route))
                else {
                    return;
                };
                lease.deadline
            };
            tokio::time::sleep_until(deadline.into()).await;
            let expired = {
                let mut state = self.lock_state();
                let Some(lease) = state.lease.as_mut() else {
                    return;
                };
                let expired = GatewayState::lease_is_routable(lease, route)
                    && Instant::now() >= lease.deadline;
                if expired {
                    lease.expiry_abort = None;
                }
                expired
            };
            if expired {
                self.release_route(route, "voice_lease_expired", true).await;
                return;
            }
        }
    }

    pub(crate) async fn release_route(
        self: &Arc<Self>,
        route: &RouteDescriptor,
        reason: &'static str,
        notify_adapter: bool,
    ) {
        let Some(release) = ({
            let mut state = self.lock_state();
            state.begin_lease_release(route)
        }) else {
            return;
        };
        let gateway = Arc::clone(self);
        let route = route.clone();
        let task = tokio::spawn(async move {
            gateway
                .finish_release(route, release, reason, notify_adapter)
                .await;
        });
        if let Err(error) = task.await {
            tracing::error!(%error,"body.release_failed");
        }
    }

    async fn finish_release(
        self: Arc<Self>,
        route: RouteDescriptor,
        release: LeaseReleaseContext,
        reason: &'static str,
        notify_adapter: bool,
    ) {
        let completion = LeaseReleaseGuard::new(Arc::clone(&self), route.clone());
        if let Some(abort) = release.expiry_abort {
            abort.abort();
        }
        if let Some(abort) = release.turn_abort {
            abort.abort();
        }
        if let Some(mut finished) = release.turn_finished {
            while !*finished.borrow_and_update() {
                if finished.changed().await.is_err() {
                    break;
                }
            }
        }
        if let Some(adapter) = release.adapter {
            let terminal = if notify_adapter {
                AdapterOutbound::TurnCancel {
                    route: route.clone(),
                    reason,
                }
            } else {
                AdapterOutbound::TurnComplete {
                    route: route.clone(),
                    reason,
                }
            };
            let _send_guard = adapter.send_gate.lock().await;
            if !send_json_confirmed(&adapter.sender, &terminal).await {
                self.invalidate_adapter(adapter.generation);
            }
        }
        if !self.send_terminal_idle(&route).await {
            self.invalidate_body(&route.device_id, route.connection_generation);
        }
        completion.finish();
        tracing::info!(
            device_id = %route.device_id,
            lease_id = %route.lease_id,
            turn_id = %route.turn_id,
            reason,
            "body.voice_lease_released"
        );
    }

    fn has_active_playback(&self, route: &RouteDescriptor) -> bool {
        let state = self.lock_state();
        state.lease.as_ref().is_some_and(|lease| {
            GatewayState::lease_is_routable(lease, route) && lease.playback.is_some()
        })
    }

    fn route_is_active(&self, route: &RouteDescriptor) -> bool {
        let state = self.lock_state();
        state
            .lease
            .as_ref()
            .is_some_and(|lease| GatewayState::lease_is_routable(lease, route))
    }

    pub(crate) fn active_route_for_body(
        &self,
        device_id: &str,
        generation: u64,
    ) -> Option<RouteDescriptor> {
        let state = self.lock_state();
        state
            .lease
            .as_ref()
            .filter(|lease| {
                lease.phase != LeasePhase::Releasing
                    && lease.route.device_id == device_id
                    && lease.route.connection_generation == generation
            })
            .map(|lease| lease.route.clone())
    }

    async fn send_turn_state(&self, route: &RouteDescriptor, state: &'static str) -> bool {
        self.send_active_body_json(
            route,
            serde_json::json!({
                "type": "turn.state",
                "protocol_major": PROTOCOL_MAJOR,
                "lease_id": route.lease_id,
                "turn_id": route.turn_id,
                "state": state,
            }),
        )
        .await
    }

    async fn fail_route(
        self: &Arc<Self>,
        route: &RouteDescriptor,
        code: &str,
        message: &str,
        reason: &'static str,
    ) {
        self.send_active_body_json(
            route,
            serde_json::json!({
                "type": "error",
                "protocol_major": PROTOCOL_MAJOR,
                "lease_id": route.lease_id,
                "turn_id": route.turn_id,
                "code": code,
                "message": message,
            }),
        )
        .await;
        self.send_turn_state(route, "aborted").await;
        self.release_route(route, reason, true).await;
    }

    async fn send_face(
        &self,
        route: &RouteDescriptor,
        expression: &'static str,
        caption: &str,
        ttl_ms: u32,
    ) -> bool {
        let version = self.face_version.fetch_add(1, Ordering::AcqRel);
        self.send_active_body_json(
            route,
            serde_json::json!({
                "type": "face.set",
                "protocol_major": PROTOCOL_MAJOR,
                "lease_id": route.lease_id,
                "turn_id": route.turn_id,
                "state_version": version,
                "expression": expression,
                "caption": caption,
                "accent_rgb": 9_403_647,
                "ttl_ms": ttl_ms,
            }),
        )
        .await
    }

    async fn send_terminal_idle(&self, route: &RouteDescriptor) -> bool {
        let version = self.face_version.fetch_add(1, Ordering::AcqRel);
        let connection = {
            let state = self.lock_state();
            state
                .bodies
                .get(&route.device_id)
                .filter(|body| body.generation == route.connection_generation)
                .map(|body| (body.sender.clone(), Arc::clone(&body.send_gate)))
        };
        let Some((sender, send_gate)) = connection else {
            return false;
        };
        let _send_guard = send_gate.lock().await;
        let _face_sent = send_json(
            &sender,
            &serde_json::json!({
                "type": "face.set",
                "protocol_major": PROTOCOL_MAJOR,
                "lease_id": route.lease_id,
                "turn_id": route.turn_id,
                "state_version": version,
                "expression": "idle",
                "caption": "",
                "accent_rgb": 9_403_647,
                "ttl_ms": 0,
            }),
        )
        .await;
        let state_delivered = send_json_confirmed(
            &sender,
            &serde_json::json!({
                "type": "turn.state",
                "protocol_major": PROTOCOL_MAJOR,
                "lease_id": route.lease_id,
                "turn_id": route.turn_id,
                "state": "idle",
            }),
        )
        .await;
        if !state_delivered {
            tracing::warn!(
                device_id = %route.device_id,
                lease_id = %route.lease_id,
                turn_id = %route.turn_id,
                "body.terminal_delivery_failed"
            );
        }
        state_delivered
    }

    async fn send_active_body_json(
        &self,
        route: &RouteDescriptor,
        value: serde_json::Value,
    ) -> bool {
        let Ok(text) = serde_json::to_string(&value) else {
            return false;
        };
        let connection = {
            let state = self.lock_state();
            state
                .bodies
                .get(&route.device_id)
                .filter(|body| body.generation == route.connection_generation)
                .map(|body| (body.sender.clone(), Arc::clone(&body.send_gate)))
        };
        let Some((sender, send_gate)) = connection else {
            return false;
        };
        let _send_guard = send_gate.lock().await;
        let active = self
            .lock_state()
            .lease
            .as_ref()
            .is_some_and(|lease| GatewayState::lease_is_routable(lease, route));
        active && try_send_message(&sender, Message::Text(text.into()))
    }

    async fn send_active_adapter_json<T: serde::Serialize>(
        &self,
        route: &RouteDescriptor,
        adapter: &AdapterConnection,
        value: &T,
    ) -> AdapterOutputDisposition {
        let Ok(text) = serde_json::to_string(value) else {
            return AdapterOutputDisposition::Failed;
        };
        self.send_active_adapter_message(route, adapter, Message::Text(text.into()))
            .await
    }

    async fn send_active_adapter_message(
        &self,
        route: &RouteDescriptor,
        adapter: &AdapterConnection,
        message: Message,
    ) -> AdapterOutputDisposition {
        let _send_guard = adapter.send_gate.lock().await;
        let active = {
            let state = self.lock_state();
            state
                .adapter
                .as_ref()
                .is_some_and(|current| current.generation == adapter.generation)
                && state
                    .lease
                    .as_ref()
                    .is_some_and(|lease| GatewayState::lease_is_routable(lease, route))
        };
        if !active {
            return AdapterOutputDisposition::Stale;
        }
        if send_message(&adapter.sender, message).await {
            AdapterOutputDisposition::Sent
        } else {
            AdapterOutputDisposition::Failed
        }
    }

    fn invalidate_adapter(&self, generation: u64) {
        let disconnect = self.lock_state().remove_adapter(generation);
        if let Some(disconnect) = disconnect {
            disconnect.cancel();
        }
    }

    fn invalidate_body(&self, device_id: &str, generation: u64) {
        let disconnect = {
            let mut state = self.lock_state();
            let current = state
                .bodies
                .get(device_id)
                .is_some_and(|body| body.generation == generation);
            current
                .then(|| state.bodies.remove(device_id))
                .flatten()
                .map(|body| body.disconnect)
        };
        if let Some(disconnect) = disconnect {
            disconnect.cancel();
        }
    }
}

fn bounded_caption(text: &str) -> String {
    let mut output = String::new();
    for character in text.trim().chars() {
        if output.len().saturating_add(character.len_utf8()) > CAPTION_MAX_BYTES {
            break;
        }
        output.push(character);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_caption_never_exceeds_device_limit() {
        let caption = bounded_caption(&"😀".repeat(500));
        assert!(caption.len() <= CAPTION_MAX_BYTES);
        assert!(caption.is_char_boundary(caption.len()));
    }

    #[test]
    fn latest_authenticated_connection_supersedes_the_stale_socket() {
        let (sender, _receiver) = mpsc::channel(1);
        let connection = BodyConnection {
            sensors: Arc::new(Mutex::new(
                super::super::interactions::SensorState::default(),
            )),
            capabilities: super::super::capabilities::BodyCapabilities {
                microphone: true,
                speaker: true,
                ..Default::default()
            },
            limits: super::super::capabilities::BodyLimits {
                max_control_bytes: super::super::protocol::MAX_CONTROL_BYTES as u32,
                max_playback_credit_ms: super::super::protocol::MAX_PLAYBACK_CREDIT_MS,
            },
            interactions: Arc::new(Mutex::new(
                super::super::interactions::InteractionState::default(),
            )),
            generation: 1,
            session_id: Uuid::new_v4(),
            sender,
            send_gate: Arc::new(AsyncMutex::new(())),
            disconnect: CancellationToken::new(),
            last_seen: Instant::now(),
            offered_build: None,
        };

        assert!(should_replace_body(Some(&connection)));
        assert!(!should_replace_body(None));
    }

    #[test]
    fn direct_opus_decode_produces_exact_body_frame() {
        let mut encoder = crate::voice::opus::make_opus_encoder(24_000).expect("encoder");
        let packets = crate::voice::opus::encode_pcm_with(&mut encoder, &vec![0.0; 480], 24_000);
        let mut decoder =
            OpusDecoder::new(OpusChannels::Mono, OpusSampleRate::Hz24000).expect("decoder");
        let framed = crate::speech::protocol::encode_tts_frame(0, 0, &packets[0]);
        let decoded = crate::speech::protocol::decode_tts_frame(&framed).expect("frame");
        let pcm = decode_opus_body_pcm(&mut decoder, decoded.payload).expect("decode");
        assert_eq!(pcm.len(), SPEAKER_PCM_BYTES);
    }

    #[test]
    fn adapter_route_classification_separates_stale_from_invalid() {
        let route = RouteDescriptor {
            device_id: "body-1".to_owned(),
            connection_generation: 7,
            body_session_id: Uuid::new_v4(),
            lease_id: Uuid::new_v4(),
            turn_id: "turn-1".to_owned(),
            capture_stream_id: 11,
        };

        assert_eq!(
            classify_adapter_route(&route, 6, route.lease_id, true),
            AdapterInputDisposition::Stale
        );
        assert_eq!(
            classify_adapter_route(&route, 7, Uuid::new_v4(), true),
            AdapterInputDisposition::Stale
        );
        assert_eq!(
            classify_adapter_route(&route, 7, route.lease_id, false),
            AdapterInputDisposition::Invalid
        );
        assert_eq!(
            classify_adapter_route(&route, 7, route.lease_id, true),
            AdapterInputDisposition::Accepted
        );
    }

    #[test]
    fn capture_sequence_requires_continuity_or_discontinuity() {
        let mut expected = None;
        assert!(accept_capture_sequence(&mut expected, 7, 0));
        assert_eq!(expected, Some(8));
        assert!(accept_capture_sequence(&mut expected, 8, 0));
        assert!(!accept_capture_sequence(&mut expected, 10, 0));
        assert_eq!(expected, Some(9));
        assert!(accept_capture_sequence(
            &mut expected,
            10,
            MEDIA_FLAG_DISCONTINUITY,
        ));
        assert_eq!(expected, Some(11));
        assert!(!accept_capture_sequence(&mut expected, 10, 0));
        assert!(accept_capture_sequence(
            &mut expected,
            u32::MAX,
            MEDIA_FLAG_DISCONTINUITY,
        ));
        assert_eq!(expected, Some(0));
        assert!(accept_capture_sequence(&mut expected, 0, 0));
    }

    #[test]
    fn lease_remains_owned_until_terminal_delivery_finishes() {
        let route = RouteDescriptor {
            device_id: "body-1".to_owned(),
            connection_generation: 7,
            body_session_id: Uuid::new_v4(),
            lease_id: Uuid::new_v4(),
            turn_id: "turn-1".to_owned(),
            capture_stream_id: 11,
        };
        let admission = crate::runtime::turn_admission::TurnAdmission::new();
        let (adapter, _receiver) = mpsc::channel(1);
        let mut state = GatewayState::new();
        state.adapter = Some(AdapterConnection {
            generation: 1,
            sender: adapter,
            send_gate: Arc::new(AsyncMutex::new(())),
            disconnect: CancellationToken::new(),
        });
        state.lease = Some(VoiceLease {
            route: route.clone(),
            request_id: "request-1".to_owned(),
            trigger: "action_button".to_owned(),
            phase: LeasePhase::Generating,
            deadline: Instant::now() + Duration::from_secs(1),
            hard_deadline: Instant::now() + Duration::from_secs(10),
            admission: Some(
                admission
                    .try_admit(crate::runtime::turn_admission::TurnSource::User)
                    .unwrap(),
            ),
            expiry_abort: None,
            turn_abort: None,
            turn_finished: None,
            generation_complete: false,
            stt_stream: None,
            next_capture_sequence: None,
            opus_decoder: None,
            playback: None,
        });

        assert!(state.begin_lease_release(&route).is_some());
        assert!(state.lease.is_some());
        assert!(
            !state
                .lease
                .as_ref()
                .is_some_and(|lease| { GatewayState::lease_is_routable(lease, &route) })
        );
        assert!(state.begin_lease_release(&route).is_none());
        assert!(!admission.is_idle());
        assert!(state.finish_lease_release(&route));
        assert!(state.lease.is_none());
        assert!(admission.is_idle());
    }
}

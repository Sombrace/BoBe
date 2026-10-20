use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use axum::extract::ws::Message;
use opusic_c::{Channels as OpusChannels, Decoder as OpusDecoder, SampleRate as OpusSampleRate};
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
    CaptureDecision, CaptureRequest, MAX_LEASE_LIFETIME_MS, MAX_PLAYBACK_CREDIT_MS,
    MEDIA_FLAG_DISCONTINUITY, OfferDecision, PROTOCOL_MAJOR, PlaybackCredit, RouteDescriptor,
    RoutedStreamControl, SPEAKER_CHANNELS, SPEAKER_CODEC, SPEAKER_PCM_BYTES,
    SPEAKER_SAMPLE_RATE_HZ, ThemeSet, UpdateStateReport,
};
use super::update::UpdateOffer;
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
const UPDATE_RETRY_FLOOR: Duration = Duration::from_secs(5);
const UPDATE_RETRY_INITIAL: Duration = Duration::from_secs(30);
const UPDATE_RETRY_MAX: Duration = Duration::from_mins(15);
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
struct LeaseGrant {
    route: RouteDescriptor,
    ttl: Duration,
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
    preempted_route: Option<RouteDescriptor>,
    capabilities: super::capabilities::BodyCapabilities,
    limits: super::capabilities::BodyLimits,
    interactions: Arc<Mutex<super::interactions::InteractionState>>,
    sensors: Arc<Mutex<super::interactions::SensorState>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateRefusal {
    Busy,
    BatteryUnknown,
    LowBattery,
}

impl UpdateRefusal {
    fn from_reason(reason: Option<&str>) -> Option<Self> {
        match reason? {
            "busy" => Some(Self::Busy),
            "battery_unknown" => Some(Self::BatteryUnknown),
            "low_battery" => Some(Self::LowBattery),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateOfferProgress {
    Offered {
        build: u32,
        generation: u64,
        refusals: u32,
    },
    Deferred {
        build: u32,
        refusals: u32,
        refusal: UpdateRefusal,
        refused_at: Instant,
        cleared: bool,
    },
    Settled {
        build: u32,
    },
}

fn update_retry_delay(refusals: u32) -> Duration {
    UPDATE_RETRY_INITIAL
        .saturating_mul(2_u32.saturating_pow(refusals.saturating_sub(1)))
        .min(UPDATE_RETRY_MAX)
}

#[derive(Default)]
struct UpdateOffers {
    devices: HashMap<String, UpdateOfferProgress>,
}

impl UpdateOffers {
    fn classify(
        &mut self,
        device_id: &str,
        generation: u64,
        offer_build: u32,
        running_build: u32,
        now: Instant,
    ) -> OfferDecision {
        if running_build == 0 {
            return OfferDecision::NoHealthYet;
        }
        if running_build >= offer_build {
            return OfferDecision::NotNewer;
        }
        let refusals = match self.devices.get(device_id).copied() {
            Some(UpdateOfferProgress::Settled { build }) if build == offer_build => {
                return OfferDecision::Suppressed;
            }
            Some(UpdateOfferProgress::Offered {
                build,
                generation: offered_on,
                ..
            }) if build == offer_build => {
                if offered_on == generation {
                    return OfferDecision::AlreadyOffered;
                }
                tracing::warn!(
                    device_id,
                    offer_build,
                    "body.update_offer_presumed_rejected"
                );
                self.devices
                    .insert(device_id.to_owned(), UpdateOfferProgress::Settled { build });
                return OfferDecision::Suppressed;
            }
            Some(UpdateOfferProgress::Deferred {
                build,
                refusals,
                refused_at,
                cleared,
                ..
            }) if build == offer_build => {
                let wait = if cleared {
                    UPDATE_RETRY_FLOOR
                } else {
                    update_retry_delay(refusals)
                };
                if now.saturating_duration_since(refused_at) < wait {
                    return OfferDecision::RetryLater;
                }
                refusals
            }
            _ => 0,
        };
        self.devices.insert(
            device_id.to_owned(),
            UpdateOfferProgress::Offered {
                build: offer_build,
                generation,
                refusals,
            },
        );
        OfferDecision::Send
    }

    fn delivery_failed(&mut self, device_id: &str, generation: u64) {
        if matches!(
            self.devices.get(device_id),
            Some(UpdateOfferProgress::Offered { generation: offered_on, .. }) if *offered_on == generation
        ) {
            self.devices.remove(device_id);
        }
    }

    fn record_state(
        &mut self,
        device_id: &str,
        generation: u64,
        report: &UpdateStateReport,
        now: Instant,
    ) {
        let Some(UpdateOfferProgress::Offered {
            build,
            generation: offered_on,
            refusals,
        }) = self.devices.get(device_id).copied()
        else {
            return;
        };
        if offered_on != generation {
            return;
        }
        let next = match (
            report.phase.as_str(),
            UpdateRefusal::from_reason(report.reason.as_deref()),
        ) {
            ("failed", Some(refusal)) => UpdateOfferProgress::Deferred {
                build,
                refusals: refusals.saturating_add(1),
                refusal,
                refused_at: now,
                cleared: false,
            },
            _ => UpdateOfferProgress::Settled { build },
        };
        self.devices.insert(device_id.to_owned(), next);
    }

    fn observe(&mut self, device_id: &str, clears: impl FnOnce(UpdateRefusal) -> bool) {
        if let Some(UpdateOfferProgress::Deferred {
            refusal, cleared, ..
        }) = self.devices.get_mut(device_id)
            && clears(*refusal)
        {
            *cleared = true;
        }
    }

    fn observe_health(&mut self, device_id: &str, health: &BodyHealthReport) {
        let idle =
            health.lifecycle.as_deref() == Some("online") && health.turn.as_deref() == Some("idle");
        self.observe(device_id, |refusal| refusal == UpdateRefusal::Busy && idle);
    }

    fn observe_battery(&mut self, device_id: &str, battery: &BodyBatteryState) {
        self.observe(device_id, |refusal| match refusal {
            UpdateRefusal::BatteryUnknown => true,
            UpdateRefusal::LowBattery => battery.charging,
            UpdateRefusal::Busy => false,
        });
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
        stream_id: rand::random::<std::num::NonZeroU32>().get(),
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
    filler_opus_decoder: Option<OpusDecoder>,
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
    update_offers: UpdateOffers,
}

impl GatewayState {
    fn new() -> Self {
        Self {
            next_generation: 1,
            bodies: HashMap::new(),
            adapter: None,
            lease: None,
            update_offers: UpdateOffers::default(),
        }
    }

    fn has_body(&self, device_id: &str, generation: u64) -> bool {
        self.bodies
            .get(device_id)
            .is_some_and(|body| body.generation == generation)
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

    fn preempt_for(
        &mut self,
        trigger: &str,
    ) -> Result<Option<(RouteDescriptor, LeaseReleaseContext)>, &'static str> {
        let Some(existing) = self.lease.as_ref() else {
            return Ok(None);
        };
        if trigger != "action_button"
            || existing.trigger != "wake_word"
            || existing.phase != LeasePhase::Granted
        {
            return Err("another_endpoint_owns_voice_lease");
        }
        let route = existing.route.clone();
        let release = self
            .begin_lease_release(&route)
            .ok_or("another_endpoint_owns_voice_lease")?;
        if let Some(displaced) = self
            .bodies
            .get_mut(&route.device_id)
            .filter(|body| body.generation == route.connection_generation)
        {
            displaced.preempted_route = Some(route.clone());
        }
        Ok(Some((route, release)))
    }

    fn preempted_route(
        &self,
        device_id: &str,
        generation: u64,
        stream_id: u32,
    ) -> Option<&RouteDescriptor> {
        self.bodies
            .get(device_id)
            .filter(|body| body.generation == generation)?
            .preempted_route
            .as_ref()
            .filter(|route| route.capture_stream_id == stream_id)
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
    lease_grants: AsyncMutex<()>,
    clock_origin: Instant,
    update_offer: Option<Arc<UpdateOffer>>,
    pub(super) sensor_states: Mutex<SensorStates>,
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
            lease_grants: AsyncMutex::new(()),
            clock_origin: Instant::now(),
            update_offer,
            sensor_states: Mutex::new(HashMap::new()),
        })
    }

    fn is_current_body(&self, device_id: &str, generation: u64) -> bool {
        self.lock_state().has_body(device_id, generation)
    }

    pub(crate) fn classify_offer(
        &self,
        device_id: &str,
        generation: u64,
        health: &BodyHealthReport,
    ) -> OfferDecision {
        let Some(offer) = self.update_offer.as_ref() else {
            return OfferDecision::NotConfigured;
        };
        let offer_build = offer.build_number();
        let mut state = self.lock_state();
        if !state.has_body(device_id, generation) {
            return OfferDecision::StaleConnection;
        }
        state.update_offers.observe_health(device_id, health);
        state.update_offers.classify(
            device_id,
            generation,
            offer_build,
            health.build_number,
            Instant::now(),
        )
    }

    pub(crate) fn record_battery(
        &self,
        device_id: &str,
        generation: u64,
        battery: &BodyBatteryState,
    ) {
        let mut state = self.lock_state();
        if state.has_body(device_id, generation) {
            state.update_offers.observe_battery(device_id, battery);
        }
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
            tracing::info!(
                device_id,
                connection_generation = generation,
                offer_build = offer.build_number(),
                offer_version = %offer.manifest().version,
                key_id = %offer.manifest().key_id,
                "body.update_offer_sent"
            );
        } else {
            self.lock_state()
                .update_offers
                .delivery_failed(device_id, generation);
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
        report: &UpdateStateReport,
    ) {
        let mut state = self.lock_state();
        if state.has_body(device_id, generation) {
            state
                .update_offers
                .record_state(device_id, generation, report, Instant::now());
            drop(state);
            tracing::info!(
                device_id,
                connection_generation = generation,
                phase = report.phase.as_str(),
                progress_percent = report.progress_percent,
                reason = report.reason.as_deref().unwrap_or(""),
                "body.update_state"
            );
        } else {
            drop(state);
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
                preempted_route: None,
                capabilities,
                limits,
                interactions: Arc::new(
                    Mutex::new(super::interactions::InteractionState::default()),
                ),
                sensors,
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
            state.update_offers.devices.remove(endpoint_id);
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

    pub(crate) async fn request_capture(
        self: &Arc<Self>,
        device_id: &str,
        generation: u64,
        sender: &OutboundSender,
        request: &CaptureRequest,
    ) -> bool {
        let _grants = self.lease_grants.lock().await;
        let grant = match self
            .acquire_lease(
                device_id,
                generation,
                &request.request_id,
                &request.trigger,
                request.stream_id,
            )
            .await
        {
            Ok(grant) => grant,
            Err(reason) => {
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
                        reason: Some(reason),
                    },
                )
                .await;
            }
        };
        let decision = CaptureDecision {
            r#type: "capture.granted",
            protocol_major: PROTOCOL_MAJOR,
            request_id: &request.request_id,
            stream_id: request.stream_id,
            lease_id: Some(grant.route.lease_id),
            turn_id: Some(&grant.route.turn_id),
            lease_ttl_ms: Some(grant.ttl.as_millis() as u64),
            reason: None,
        };
        if !send_json(sender, &decision).await {
            return false;
        }
        let gateway = Arc::clone(self);
        let route = grant.route;
        let expiry_route = route.clone();
        let expiry = tokio::spawn(async move {
            gateway.watch_lease_expiry(&expiry_route).await;
        });
        if !self.attach_expiry_abort(&route, expiry.abort_handle()) {
            expiry.abort();
        }
        true
    }

    fn viable_daemon_stt(&self) -> Option<Arc<dyn crate::speech::SttEngine>> {
        self.runtime
            .voice_engines
            .load()
            .stt
            .as_ref()
            .filter(|engine| engine.health().is_viable())
            .map(Arc::clone)
    }

    async fn acquire_lease(
        self: &Arc<Self>,
        device_id: &str,
        generation: u64,
        request_id: &str,
        trigger: &str,
        stream_id: u32,
    ) -> Result<LeaseGrant, &'static str> {
        if request_id.is_empty() || request_id.len() > 64 || stream_id == 0 {
            return Err("invalid_capture_request");
        }
        let preempted = {
            let mut state = self.lock_state();
            match state.bodies.get_mut(device_id) {
                Some(body) if body.generation == generation => body.preempted_route = None,
                Some(_) => return Err("stale_connection_generation"),
                None => return Err("body_not_connected"),
            }
            if state.adapter.is_none() && self.viable_daemon_stt().is_none() {
                return Err("speech_engine_unavailable");
            }
            state.preempt_for(trigger)?
        };
        if let Some((route, release)) = preempted {
            self.complete_release(route, release, "explicit_capture_preempted", true)
                .await;
        }
        let daemon_stt = self.viable_daemon_stt();
        let mut state = self.lock_state();
        if state.lease.is_some() {
            return Err("another_endpoint_owns_voice_lease");
        }
        let body_session_id = match state.bodies.get(device_id) {
            Some(body) if body.generation == generation => body.session_id,
            Some(_) => return Err("stale_connection_generation"),
            None => return Err("body_not_connected"),
        };
        if state.adapter.is_none() && daemon_stt.is_none() {
            return Err("speech_engine_unavailable");
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
            body_session_id,
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
            filler_opus_decoder: None,
            playback: None,
        });
        Ok(LeaseGrant {
            route,
            ttl: LEASE_TTL,
        })
    }

    fn attach_expiry_abort(
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
            if let Some(frame) = crate::speech::protocol::decode_tts_frame(bytes) {
                let filler = frame.flags & crate::speech::protocol::FLAG_FILLER != 0;
                let decoder = if filler {
                    &mut lease.filler_opus_decoder
                } else {
                    &mut lease.opus_decoder
                };
                if (filler && frame.chunk_id == 0)
                    || frame.flags & crate::speech::protocol::FLAG_FIRST_OF_TURN != 0
                {
                    *decoder = OpusDecoder::new(OpusChannels::Mono, OpusSampleRate::Hz24000).ok();
                }
                tracing::trace!(
                    chunk_id = frame.chunk_id,
                    flags = frame.flags,
                    "body.tts_frame"
                );
                decoder
                    .as_mut()
                    .ok_or_else(|| "body Opus decoder is unavailable".to_owned())
                    .and_then(|decoder| decode_opus_body_pcm(decoder, frame.payload))
            } else {
                Err("invalid body TTS frame".to_owned())
            }
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
        self.complete_release(route.clone(), release, reason, notify_adapter)
            .await;
    }

    async fn complete_release(
        self: &Arc<Self>,
        route: RouteDescriptor,
        release: LeaseReleaseContext,
        reason: &'static str,
        notify_adapter: bool,
    ) {
        let gateway = Arc::clone(self);
        let task = tokio::spawn(async move {
            gateway
                .finish_release(route, release, reason, notify_adapter)
                .await;
        });
        if let Err(error) = task.await {
            tracing::error!(%error,"body.release_failed");
        }
    }

    pub(crate) async fn refuse_preempted_capture(
        &self,
        device_id: &str,
        generation: u64,
        control: &RoutedStreamControl,
        respond: bool,
    ) -> bool {
        let refusal = {
            let state = self.lock_state();
            let Some(route) = state
                .preempted_route(device_id, generation, control.stream_id)
                .filter(|route| {
                    route.lease_id == control.lease_id && route.turn_id == control.turn_id
                })
                .cloned()
            else {
                return false;
            };
            state
                .bodies
                .get(device_id)
                .map(|body| (route, body.sender.clone(), Arc::clone(&body.send_gate)))
        };
        let Some((route, sender, send_gate)) = refusal else {
            return false;
        };
        tracing::debug!(
            device_id,
            lease_id = %route.lease_id,
            turn_id = %route.turn_id,
            "body.preempted_capture_refused"
        );
        if !respond {
            return true;
        }
        let _send_guard = send_gate.lock().await;
        send_json(
            &sender,
            &serde_json::json!({
                "type": "turn.state",
                "protocol_major": PROTOCOL_MAJOR,
                "lease_id": route.lease_id,
                "turn_id": route.turn_id,
                "state": "aborted",
            }),
        )
        .await
    }

    pub(crate) fn is_preempted_capture_stream(
        &self,
        device_id: &str,
        generation: u64,
        stream_id: u32,
    ) -> bool {
        self.lock_state()
            .preempted_route(device_id, generation, stream_id)
            .is_some()
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

#[cfg(test)]
impl BodyGateway {
    pub(super) async fn for_tests(root: &std::path::Path) -> Arc<Self> {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let session = RuntimeSession::for_tests(root, pool.clone()).await;
        let requests = crate::runtime::message_requests::MessageRequests::new(
            pool,
            Arc::clone(&session),
            Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            Arc::clone(&session.runs),
            Arc::new(tokio::sync::RwLock::new(())),
        );
        Self::new(
            1,
            true,
            BodyRuntime {
                requests,
                config: Arc::new(ArcSwap::from_pointee(Config::default())),
                runtime_session: session,
                voice_engines: Arc::new(ArcSwap::from_pointee(VoiceEnginesSnapshot::default())),
                voice_sink: Arc::new(VoiceSink::new()),
            },
            None,
        )
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
                max_playback_credit_ms: MAX_PLAYBACK_CREDIT_MS,
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
            preempted_route: None,
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

    fn report<T: serde::de::DeserializeOwned>(json: &str) -> T {
        serde_json::from_str(json).expect("firmware report")
    }

    fn health(lifecycle: &str, turn: &str) -> BodyHealthReport {
        report(&format!(
            r#"{{"type":"health.report","protocol_major":1,"uptime_s":30,"lifecycle":"{lifecycle}","turn":"{turn}","microphone_muted":false,"app_version":"0.9.0","build_number":100,"running_slot":0}}"#
        ))
    }

    fn battery(percent: u8, charging: bool) -> BodyBatteryState {
        report(&format!(
            r#"{{"type":"battery.state","protocol_major":1,"raw_adc":2100,"percent":{percent},"charging":{charging}}}"#
        ))
    }

    fn update_state(phase: &str, reason: Option<&str>) -> UpdateStateReport {
        let reason = reason.map_or_else(String::new, |reason| format!(r#","reason":"{reason}""#));
        report(&format!(
            r#"{{"type":"update.state","protocol_major":1,"phase":"{phase}","progress_percent":0{reason}}}"#
        ))
    }

    fn offer_after_health(
        offers: &mut UpdateOffers,
        generation: u64,
        health: &BodyHealthReport,
        now: Instant,
    ) -> OfferDecision {
        offers.observe_health("body-1", health);
        offers.classify("body-1", generation, 200, health.build_number, now)
    }

    #[test]
    fn a_low_battery_refusal_is_offered_again_once_the_body_charges() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let idle = health("online", "idle");
        let mut offers = UpdateOffers::default();

        assert_eq!(
            offer_after_health(&mut offers, 7, &idle, at(0)),
            OfferDecision::Send
        );
        assert_eq!(
            offer_after_health(&mut offers, 7, &idle, at(1)),
            OfferDecision::AlreadyOffered
        );
        offers.record_state(
            "body-1",
            7,
            &update_state("failed", Some("low_battery")),
            at(1),
        );
        offers.observe_battery("body-1", &battery(20, false));
        assert_eq!(
            offer_after_health(&mut offers, 7, &idle, at(11)),
            OfferDecision::RetryLater
        );
        offers.observe_battery("body-1", &battery(21, true));
        assert_eq!(
            offer_after_health(&mut offers, 7, &idle, at(21)),
            OfferDecision::Send
        );
        offers.record_state("body-1", 7, &update_state("accepted", None), at(22));
        assert_eq!(
            offer_after_health(&mut offers, 7, &idle, at(3_600)),
            OfferDecision::Suppressed
        );
    }

    #[test]
    fn a_busy_refusal_is_offered_again_once_the_body_is_idle() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut offers = UpdateOffers::default();

        assert_eq!(
            offer_after_health(&mut offers, 7, &health("online", "idle"), at(0)),
            OfferDecision::Send
        );
        offers.record_state("body-1", 7, &update_state("failed", Some("busy")), at(0));
        assert_eq!(
            offer_after_health(&mut offers, 7, &health("online", "thinking"), at(5)),
            OfferDecision::RetryLater
        );
        assert_eq!(
            offer_after_health(&mut offers, 7, &health("updating", "idle"), at(10)),
            OfferDecision::RetryLater
        );
        assert_eq!(
            offer_after_health(&mut offers, 7, &health("online", "idle"), at(15)),
            OfferDecision::Send
        );
    }

    #[test]
    fn transient_refusals_back_off_within_a_bound() {
        let start = Instant::now();
        let mut offers = UpdateOffers::default();
        let unknown = report::<BodyHealthReport>(r#"{"build_number":100}"#);
        let mut now = start;
        let mut waits = Vec::new();
        assert_eq!(
            offer_after_health(&mut offers, 7, &unknown, now),
            OfferDecision::Send
        );
        for _ in 0..8 {
            offers.record_state(
                "body-1",
                7,
                &update_state("failed", Some("battery_unknown")),
                now,
            );
            let refused_at = now;
            while offer_after_health(&mut offers, 7, &unknown, now) == OfferDecision::RetryLater {
                now += Duration::from_secs(10);
            }
            waits.push(now.duration_since(refused_at).as_secs());
        }
        assert_eq!(waits, [30, 60, 120, 240, 480, 900, 900, 900]);

        offers.record_state(
            "body-1",
            7,
            &update_state("failed", Some("battery_unknown")),
            now,
        );
        offers.observe_battery("body-1", &battery(80, false));
        assert_eq!(
            offer_after_health(&mut offers, 7, &unknown, now + Duration::from_secs(5)),
            OfferDecision::Send
        );
    }

    #[test]
    fn permanent_update_failures_stay_suppressed_across_reconnections() {
        let start = Instant::now();
        let idle = health("online", "idle");
        let mut offers = UpdateOffers::default();
        assert_eq!(
            offer_after_health(&mut offers, 7, &idle, start),
            OfferDecision::Send
        );
        offers.record_state(
            "body-1",
            7,
            &update_state("failed", Some("install_failed")),
            start,
        );
        assert_eq!(
            offer_after_health(&mut offers, 9, &idle, start),
            OfferDecision::Suppressed
        );

        let mut rejected = UpdateOffers::default();
        assert_eq!(
            offer_after_health(&mut rejected, 7, &idle, start),
            OfferDecision::Send
        );
        assert_eq!(
            offer_after_health(&mut rejected, 8, &idle, start),
            OfferDecision::Suppressed
        );
        assert_eq!(
            offer_after_health(&mut rejected, 9, &idle, start),
            OfferDecision::Suppressed
        );

        let mut undelivered = UpdateOffers::default();
        assert_eq!(
            offer_after_health(&mut undelivered, 7, &idle, start),
            OfferDecision::Send
        );
        undelivered.delivery_failed("body-1", 7);
        assert_eq!(
            offer_after_health(&mut undelivered, 7, &idle, start),
            OfferDecision::Send
        );

        let current = report::<BodyHealthReport>(r#"{"build_number":200}"#);
        assert_eq!(
            offer_after_health(&mut offers, 9, &current, start),
            OfferDecision::NotNewer
        );
        let booting = report::<BodyHealthReport>("{}");
        assert_eq!(
            offer_after_health(&mut offers, 9, &booting, start),
            OfferDecision::NoHealthYet
        );
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
            filler_opus_decoder: None,
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

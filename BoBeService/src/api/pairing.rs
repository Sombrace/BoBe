use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bobe_pairing_core::{Invitation, ServerStart};
use rand::RngExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::api::scope::ScopeProfile;
use crate::error::AppError;

const INVITATION_TTL: Duration = Duration::from_mins(5);
const EXCHANGE_TTL: Duration = Duration::from_mins(1);
const RATE_WINDOW: Duration = Duration::from_mins(1);
const MAX_ATTEMPTS: u8 = 5;
const MAX_ACTIVE_EXCHANGES: usize = 5;
const MAX_BEGINS_PER_SOURCE: usize = 5;
const MAX_BEGINS_PER_AUTHORITY: usize = 20;
const MAX_TRACKED_SOURCES: usize = 64;

pub(crate) struct PairingManager {
    state: Mutex<State>,
}

struct State {
    invitation: Option<ActiveInvitation>,
    exchanges: HashMap<Uuid, Exchange>,
    reservation: Option<ActiveReservation>,
    authority_begins: VecDeque<Instant>,
    source_begins: HashMap<IpAddr, VecDeque<Instant>>,
}

struct ActiveInvitation {
    invitation: Invitation,
    profile: ScopeProfile,
    label: String,
    expires_at: Instant,
    attempts: u8,
}

struct Exchange {
    server: ServerStart,
    expires_at: Instant,
}

struct ActiveReservation {
    id: Uuid,
}

pub(crate) struct PairingInvitationView {
    pub(crate) code: String,
    pub(crate) expires_in_seconds: u64,
    pub(crate) label: String,
}

pub(crate) struct PairingBeginResult {
    pub(crate) exchange_id: Uuid,
    pub(crate) response: Vec<u8>,
}

pub(crate) struct PairingReservation {
    id: Uuid,
    pub(crate) session_key: Vec<u8>,
    pub(crate) profile: ScopeProfile,
    pub(crate) label: String,
}

impl PairingReservation {
    pub(crate) fn id(&self) -> Uuid {
        self.id
    }
}

impl PairingManager {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                invitation: None,
                exchanges: HashMap::new(),
                reservation: None,
                authority_begins: VecDeque::new(),
                source_begins: HashMap::new(),
            }),
        })
    }

    pub(crate) async fn create_invitation(
        &self,
        profile: ScopeProfile,
        label: &str,
    ) -> Result<PairingInvitationView, AppError> {
        if label.trim().is_empty() || label.len() > 128 {
            return Err(AppError::Validation(
                "device label must contain 1-128 characters".into(),
            ));
        }
        let code = format!("{:06}", rand::rng().random_range(0..1_000_000_u32));
        let invitation = Invitation::create(code.as_bytes())
            .map_err(|error| AppError::Internal(format!("create pairing invitation: {error}")))?;
        let mut state = self.state.lock().await;
        state.exchanges.clear();
        state.reservation = None;
        state.authority_begins.clear();
        state.source_begins.clear();
        state.invitation = Some(ActiveInvitation {
            invitation,
            profile,
            label: label.to_owned(),
            expires_at: Instant::now() + INVITATION_TTL,
            attempts: 0,
        });
        Ok(PairingInvitationView {
            code,
            expires_in_seconds: INVITATION_TTL.as_secs(),
            label: label.to_owned(),
        })
    }

    pub(crate) async fn invitation_profile(&self) -> Option<ScopeProfile> {
        self.state
            .lock()
            .await
            .invitation
            .as_ref()
            .map(|invitation| invitation.profile)
    }

    pub(crate) async fn begin(
        &self,
        profile: ScopeProfile,
        request: &[u8],
        context: &[u8],
        source: IpAddr,
    ) -> Result<PairingBeginResult, AppError> {
        let mut state = self.state.lock().await;
        let now = Instant::now();
        Self::purge(&mut state, now);
        if state.reservation.is_some() {
            return Err(pairing_unavailable());
        }
        let invitation = {
            let Some(invitation) = state.invitation.as_ref() else {
                return Err(pairing_unavailable());
            };
            if invitation.expires_at <= now || invitation.attempts >= MAX_ATTEMPTS {
                state.invitation = None;
                state.exchanges.clear();
                return Err(pairing_unavailable());
            }
            if invitation.profile != profile {
                return Err(pairing_unavailable());
            }
            invitation.invitation.clone()
        };
        if state.exchanges.len() >= MAX_ACTIVE_EXCHANGES
            || state.authority_begins.len() >= MAX_BEGINS_PER_AUTHORITY
            || state
                .source_begins
                .get(&source)
                .is_some_and(|begins| begins.len() >= MAX_BEGINS_PER_SOURCE)
            || (!state.source_begins.contains_key(&source)
                && state.source_begins.len() >= MAX_TRACKED_SOURCES)
        {
            return Err(pairing_unavailable());
        }
        let server =
            ServerStart::begin(&invitation, request, context).map_err(|_| pairing_unavailable())?;
        let response = server.response().to_vec();
        let exchange_id = Uuid::new_v4();
        state.exchanges.insert(
            exchange_id,
            Exchange {
                server,
                expires_at: now + EXCHANGE_TTL,
            },
        );
        state.authority_begins.push_back(now);
        state
            .source_begins
            .entry(source)
            .or_default()
            .push_back(now);
        Ok(PairingBeginResult {
            exchange_id,
            response,
        })
    }

    pub(crate) async fn finish(
        &self,
        exchange_id: Uuid,
        profile: ScopeProfile,
        finalization: &[u8],
        context: &[u8],
    ) -> Result<PairingReservation, AppError> {
        let mut state = self.state.lock().await;
        let now = Instant::now();
        Self::purge(&mut state, now);
        if state.reservation.is_some()
            || state
                .invitation
                .as_ref()
                .is_none_or(|invitation| invitation.profile != profile)
        {
            return Err(pairing_unavailable());
        }
        let exchange = state
            .exchanges
            .remove(&exchange_id)
            .ok_or_else(pairing_unavailable)?;
        let Ok(session_key) = exchange.server.finish(finalization, context) else {
            if let Some(invitation) = state.invitation.as_mut() {
                invitation.attempts += 1;
                if invitation.attempts >= MAX_ATTEMPTS {
                    state.invitation = None;
                    state.exchanges.clear();
                }
            }
            return Err(pairing_unavailable());
        };
        let invitation = state.invitation.as_ref().ok_or_else(pairing_unavailable)?;
        let reservation = PairingReservation {
            id: Uuid::new_v4(),
            session_key,
            profile: invitation.profile,
            label: invitation.label.clone(),
        };
        state.exchanges.clear();
        state.reservation = Some(ActiveReservation { id: reservation.id });
        Ok(reservation)
    }

    pub(crate) async fn consume(&self, reservation_id: Uuid) -> bool {
        let mut state = self.state.lock().await;
        if state.reservation.as_ref().map(|value| value.id) != Some(reservation_id) {
            return false;
        }
        state.reservation = None;
        state.invitation = None;
        state.exchanges.clear();
        true
    }

    pub(crate) async fn release(&self, reservation_id: Uuid) -> bool {
        let mut state = self.state.lock().await;
        if state.reservation.as_ref().map(|value| value.id) != Some(reservation_id) {
            return false;
        }
        state.reservation = None;
        state.exchanges.clear();
        if state
            .invitation
            .as_ref()
            .is_some_and(|invitation| invitation.expires_at <= Instant::now())
        {
            state.invitation = None;
        }
        true
    }

    fn purge(state: &mut State, now: Instant) {
        state
            .exchanges
            .retain(|_, exchange| exchange.expires_at > now);
        while state
            .authority_begins
            .front()
            .is_some_and(|started| now.duration_since(*started) >= RATE_WINDOW)
        {
            state.authority_begins.pop_front();
        }
        state.source_begins.retain(|_, begins| {
            while begins
                .front()
                .is_some_and(|started| now.duration_since(*started) >= RATE_WINDOW)
            {
                begins.pop_front();
            }
            !begins.is_empty()
        });
    }

    #[cfg(test)]
    pub(crate) async fn has_active_invitation(&self) -> bool {
        self.state.lock().await.invitation.is_some()
    }
}

pub(crate) fn pairing_unavailable() -> AppError {
    AppError::Unauthorized("pairing is unavailable or expired".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bobe_pairing_core::ClientStart;

    #[tokio::test]
    async fn exchanges_from_the_same_source_coexist() {
        let manager = PairingManager::new();
        let invitation = manager
            .create_invitation(ScopeProfile::MacApp, "Test Mac")
            .await
            .unwrap();
        let first = ClientStart::begin(invitation.code.as_bytes()).unwrap();
        let first_begin = manager
            .begin(
                ScopeProfile::MacApp,
                first.request(),
                b"first",
                "192.0.2.10".parse().unwrap(),
            )
            .await
            .unwrap();
        let second = ClientStart::begin(invitation.code.as_bytes()).unwrap();
        manager
            .begin(
                ScopeProfile::MacApp,
                second.request(),
                b"second",
                "192.0.2.10".parse().unwrap(),
            )
            .await
            .unwrap();
        let first = first
            .finish(invitation.code.as_bytes(), &first_begin.response, b"first")
            .unwrap();
        assert!(
            manager
                .finish(
                    first_begin.exchange_id,
                    ScopeProfile::MacApp,
                    first.finalization(),
                    b"first",
                )
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn one_code_is_consumed_only_after_commit() {
        let manager = PairingManager::new();
        let invitation = manager
            .create_invitation(ScopeProfile::MacApp, "Test Mac")
            .await
            .unwrap();
        assert_eq!(invitation.code.len(), 6);
        let client = ClientStart::begin(invitation.code.as_bytes()).unwrap();
        let begin = manager
            .begin(
                ScopeProfile::MacApp,
                client.request(),
                b"context",
                "127.0.0.1".parse().unwrap(),
            )
            .await
            .unwrap();
        let client = client
            .finish(invitation.code.as_bytes(), &begin.response, b"context")
            .unwrap();
        let reservation = manager
            .finish(
                begin.exchange_id,
                ScopeProfile::MacApp,
                client.finalization(),
                b"context",
            )
            .await
            .unwrap();
        assert_eq!(reservation.session_key, client.session_key());
        assert_eq!(reservation.profile, ScopeProfile::MacApp);
        assert_eq!(reservation.label, "Test Mac");
        assert!(manager.has_active_invitation().await);
        assert!(manager.release(reservation.id()).await);
        assert!(manager.has_active_invitation().await);

        let client = ClientStart::begin(invitation.code.as_bytes()).unwrap();
        let begin = manager
            .begin(
                ScopeProfile::MacApp,
                client.request(),
                b"retry",
                "127.0.0.1".parse().unwrap(),
            )
            .await
            .unwrap();
        let client = client
            .finish(invitation.code.as_bytes(), &begin.response, b"retry")
            .unwrap();
        let reservation = manager
            .finish(
                begin.exchange_id,
                ScopeProfile::MacApp,
                client.finalization(),
                b"retry",
            )
            .await
            .unwrap();
        assert!(manager.consume(reservation.id()).await);
        assert!(!manager.has_active_invitation().await);
    }

    #[tokio::test]
    async fn begin_limits_are_temporal_and_non_destructive() {
        let manager = PairingManager::new();
        let invitation = manager
            .create_invitation(ScopeProfile::MacApp, "Test Mac")
            .await
            .unwrap();
        let source = "192.0.2.10".parse().unwrap();
        for index in 0..MAX_BEGINS_PER_SOURCE {
            let client = ClientStart::begin(invitation.code.as_bytes()).unwrap();
            manager
                .begin(
                    ScopeProfile::MacApp,
                    client.request(),
                    format!("context-{index}").as_bytes(),
                    source,
                )
                .await
                .unwrap();
        }
        let extra = ClientStart::begin(invitation.code.as_bytes()).unwrap();
        assert!(
            manager
                .begin(ScopeProfile::MacApp, extra.request(), b"extra", source)
                .await
                .is_err()
        );
        assert!(manager.has_active_invitation().await);
    }

    #[tokio::test]
    async fn a_phone_code_pairs_only_under_the_phone_profile() {
        let manager = PairingManager::new();
        let invitation = manager
            .create_invitation(ScopeProfile::Phone, "iPhone")
            .await
            .unwrap();
        assert_eq!(
            manager.invitation_profile().await,
            Some(ScopeProfile::Phone)
        );
        let source = "192.0.2.20".parse().unwrap();
        let client = ClientStart::begin(invitation.code.as_bytes()).unwrap();
        assert!(
            manager
                .begin(ScopeProfile::MacApp, client.request(), b"mac", source)
                .await
                .is_err()
        );
        let begin = manager
            .begin(ScopeProfile::Phone, client.request(), b"phone", source)
            .await
            .unwrap();
        let client = client
            .finish(invitation.code.as_bytes(), &begin.response, b"phone")
            .unwrap();
        assert!(
            manager
                .finish(
                    begin.exchange_id,
                    ScopeProfile::MacApp,
                    client.finalization(),
                    b"phone",
                )
                .await
                .is_err()
        );
        let reservation = manager
            .finish(
                begin.exchange_id,
                ScopeProfile::Phone,
                client.finalization(),
                b"phone",
            )
            .await
            .unwrap();
        assert_eq!(reservation.profile, ScopeProfile::Phone);
        assert_eq!(reservation.label, "iPhone");
        assert!(manager.consume(reservation.id()).await);
        assert_eq!(manager.invitation_profile().await, None);
    }
}

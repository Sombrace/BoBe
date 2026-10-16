use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use bobe_pairing_core::{Invitation, ServerStart};
use rand::RngExt;
use sha2::Digest;
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
    delivered: Option<DeliveredEnrollment>,
    authority_begins: VecDeque<Instant>,
    source_begins: HashMap<IpAddr, VecDeque<Instant>>,
}

struct ActiveInvitation {
    id: Uuid,
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
    expires_at: Instant,
}

struct DeliveredEnrollment {
    exchange_id: Uuid,
    attempt: [u8; 32],
    profile: ScopeProfile,
    sealed: SealedEnrollment,
    expires_at: Instant,
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

pub(crate) struct FinishAttempt<'a> {
    pub(crate) exchange_id: Uuid,
    pub(crate) profile: ScopeProfile,
    pub(crate) finalization: &'a [u8],
    pub(crate) context: &'a [u8],
    pub(crate) nonce: &'a [u8],
    pub(crate) enrollment: &'a [u8],
}

impl FinishAttempt<'_> {
    fn digest(&self) -> [u8; 32] {
        let mut hasher = sha2::Sha256::new();
        for part in [self.finalization, self.context, self.nonce, self.enrollment] {
            hasher.update((part.len() as u64).to_be_bytes());
            hasher.update(part);
        }
        hasher.finalize().into()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SealedEnrollment {
    pub(crate) nonce: [u8; 12],
    pub(crate) ciphertext: Vec<u8>,
}

pub(crate) enum Finished {
    Reserved(PairingReservation),
    Delivered(SealedEnrollment),
}

pub(crate) struct PairingReservation {
    manager: Arc<PairingManager>,
    id: Uuid,
    invitation_id: Uuid,
    exchange_id: Uuid,
    attempt: [u8; 32],
    pub(crate) session_key: Vec<u8>,
    pub(crate) profile: ScopeProfile,
    pub(crate) label: String,
    settled: bool,
}

impl PairingReservation {
    pub(crate) fn deliver(mut self, sealed: SealedEnrollment) {
        self.settled = true;
        let mut state = self.manager.lock();
        if state
            .reservation
            .as_ref()
            .is_some_and(|reservation| reservation.id == self.id)
        {
            state.reservation = None;
        }
        if state
            .invitation
            .as_ref()
            .is_some_and(|invitation| invitation.id == self.invitation_id)
        {
            state.invitation = None;
            state.exchanges.clear();
        }
        state.delivered = Some(DeliveredEnrollment {
            exchange_id: self.exchange_id,
            attempt: self.attempt,
            profile: self.profile,
            sealed,
            expires_at: Instant::now() + EXCHANGE_TTL,
        });
    }
}

impl Drop for PairingReservation {
    fn drop(&mut self) {
        if !self.settled {
            self.manager.release(self.id);
        }
    }
}

impl PairingManager {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                invitation: None,
                exchanges: HashMap::new(),
                reservation: None,
                delivered: None,
                authority_begins: VecDeque::new(),
                source_begins: HashMap::new(),
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn create_invitation(
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
        let mut state = self.lock();
        state.exchanges.clear();
        state.reservation = None;
        state.delivered = None;
        state.authority_begins.clear();
        state.source_begins.clear();
        state.invitation = Some(ActiveInvitation {
            id: Uuid::new_v4(),
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

    pub(crate) fn invitation_profile(&self) -> Option<ScopeProfile> {
        self.lock()
            .invitation
            .as_ref()
            .map(|invitation| invitation.profile)
    }

    pub(crate) fn completion_profile(&self, exchange_id: Uuid) -> Option<ScopeProfile> {
        let state = self.lock();
        state
            .delivered
            .as_ref()
            .filter(|delivered| delivered.exchange_id == exchange_id)
            .map(|delivered| delivered.profile)
            .or_else(|| {
                state
                    .invitation
                    .as_ref()
                    .map(|invitation| invitation.profile)
            })
    }

    pub(crate) fn begin(
        &self,
        profile: ScopeProfile,
        request: &[u8],
        context: &[u8],
        source: IpAddr,
    ) -> Result<PairingBeginResult, AppError> {
        let mut state = self.lock();
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

    pub(crate) fn finish(
        self: &Arc<Self>,
        attempt: &FinishAttempt<'_>,
    ) -> Result<Finished, AppError> {
        let digest = attempt.digest();
        let mut state = self.lock();
        let now = Instant::now();
        Self::purge(&mut state, now);
        if state
            .delivered
            .as_ref()
            .is_some_and(|delivered| delivered.exchange_id == attempt.exchange_id)
        {
            return state
                .delivered
                .take_if(|delivered| {
                    delivered.attempt == digest && delivered.profile == attempt.profile
                })
                .map(|delivered| Finished::Delivered(delivered.sealed))
                .ok_or_else(pairing_unavailable);
        }
        if state.reservation.is_some()
            || state
                .invitation
                .as_ref()
                .is_none_or(|invitation| invitation.profile != attempt.profile)
        {
            return Err(pairing_unavailable());
        }
        let exchange = state
            .exchanges
            .remove(&attempt.exchange_id)
            .ok_or_else(pairing_unavailable)?;
        let Ok(session_key) = exchange
            .server
            .finish(attempt.finalization, attempt.context)
        else {
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
            manager: Arc::clone(self),
            id: Uuid::new_v4(),
            invitation_id: invitation.id,
            exchange_id: attempt.exchange_id,
            attempt: digest,
            session_key,
            profile: invitation.profile,
            label: invitation.label.clone(),
            settled: false,
        };
        state.exchanges.clear();
        state.reservation = Some(ActiveReservation {
            id: reservation.id,
            expires_at: now + EXCHANGE_TTL,
        });
        Ok(Finished::Reserved(reservation))
    }

    fn release(&self, reservation_id: Uuid) {
        let mut state = self.lock();
        if state
            .reservation
            .as_ref()
            .is_none_or(|reservation| reservation.id != reservation_id)
        {
            return;
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
    }

    fn purge(state: &mut State, now: Instant) {
        state
            .exchanges
            .retain(|_, exchange| exchange.expires_at > now);
        if state
            .reservation
            .as_ref()
            .is_some_and(|reservation| reservation.expires_at <= now)
        {
            state.reservation = None;
        }
        if state
            .delivered
            .as_ref()
            .is_some_and(|delivered| delivered.expires_at <= now)
        {
            state.delivered = None;
        }
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
    pub(crate) fn has_active_invitation(&self) -> bool {
        self.lock().invitation.is_some()
    }
}

pub(crate) fn pairing_unavailable() -> AppError {
    AppError::Unauthorized("pairing is unavailable or expired".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bobe_pairing_core::ClientStart;

    struct Client {
        exchange_id: Uuid,
        finalization: Vec<u8>,
        session_key: Vec<u8>,
        context: Vec<u8>,
    }

    impl Client {
        fn attempt(&self, profile: ScopeProfile) -> FinishAttempt<'_> {
            FinishAttempt {
                exchange_id: self.exchange_id,
                profile,
                finalization: &self.finalization,
                context: &self.context,
                nonce: b"request-nonce",
                enrollment: b"sealed enrollment",
            }
        }
    }

    fn exchange(
        manager: &PairingManager,
        profile: ScopeProfile,
        code: &str,
        context: &[u8],
        source: &str,
    ) -> Result<Client, AppError> {
        let client = ClientStart::begin(code.as_bytes()).unwrap();
        let begin = manager.begin(profile, client.request(), context, source.parse().unwrap())?;
        let client = client
            .finish(code.as_bytes(), &begin.response, context)
            .unwrap();
        Ok(Client {
            exchange_id: begin.exchange_id,
            finalization: client.finalization().to_vec(),
            session_key: client.session_key().to_vec(),
            context: context.to_vec(),
        })
    }

    fn reserve(manager: &Arc<PairingManager>, attempt: &FinishAttempt<'_>) -> PairingReservation {
        match manager.finish(attempt) {
            Ok(Finished::Reserved(reservation)) => reservation,
            Ok(Finished::Delivered(_)) => panic!("a first finish reserves the invitation"),
            Err(error) => panic!("finish failed: {error}"),
        }
    }

    fn sealed() -> SealedEnrollment {
        SealedEnrollment {
            nonce: [7; 12],
            ciphertext: b"issued credential".to_vec(),
        }
    }

    #[test]
    fn exchanges_from_the_same_source_coexist() {
        let manager = PairingManager::new();
        let invitation = manager
            .create_invitation(ScopeProfile::MacApp, "Test Mac")
            .unwrap();
        let first = exchange(
            &manager,
            ScopeProfile::MacApp,
            &invitation.code,
            b"first",
            "192.0.2.10",
        )
        .unwrap();
        exchange(
            &manager,
            ScopeProfile::MacApp,
            &invitation.code,
            b"second",
            "192.0.2.10",
        )
        .unwrap();
        assert!(manager.finish(&first.attempt(ScopeProfile::MacApp)).is_ok());
    }

    #[test]
    fn one_code_is_consumed_only_after_delivery() {
        let manager = PairingManager::new();
        let invitation = manager
            .create_invitation(ScopeProfile::MacApp, "Test Mac")
            .unwrap();
        assert_eq!(invitation.code.len(), 6);
        let client = exchange(
            &manager,
            ScopeProfile::MacApp,
            &invitation.code,
            b"context",
            "127.0.0.1",
        )
        .unwrap();
        let reservation = reserve(&manager, &client.attempt(ScopeProfile::MacApp));
        assert_eq!(reservation.session_key, client.session_key);
        assert_eq!(reservation.profile, ScopeProfile::MacApp);
        assert_eq!(reservation.label, "Test Mac");
        assert!(manager.has_active_invitation());
        drop(reservation);
        assert!(manager.has_active_invitation());

        let client = exchange(
            &manager,
            ScopeProfile::MacApp,
            &invitation.code,
            b"retry",
            "127.0.0.1",
        )
        .unwrap();
        reserve(&manager, &client.attempt(ScopeProfile::MacApp)).deliver(sealed());
        assert!(!manager.has_active_invitation());
    }

    #[tokio::test]
    async fn a_completion_cancelled_before_enrollment_releases_the_code() {
        let manager = PairingManager::new();
        let invitation = manager
            .create_invitation(ScopeProfile::MacApp, "Test Mac")
            .unwrap();
        let client = exchange(
            &manager,
            ScopeProfile::MacApp,
            &invitation.code,
            b"cancelled",
            "127.0.0.1",
        )
        .unwrap();
        let reservation = reserve(&manager, &client.attempt(ScopeProfile::MacApp));
        let request = async move {
            let _reservation = reservation;
            std::future::pending::<()>().await;
        };
        assert!(
            tokio::time::timeout(Duration::from_millis(10), request)
                .await
                .is_err()
        );

        let retry = exchange(
            &manager,
            ScopeProfile::MacApp,
            &invitation.code,
            b"retry",
            "127.0.0.1",
        )
        .unwrap();
        reserve(&manager, &retry.attempt(ScopeProfile::MacApp)).deliver(sealed());
        assert!(!manager.has_active_invitation());
    }

    #[test]
    fn a_completion_lost_after_enrollment_is_returned_once_to_the_same_exchange() {
        let manager = PairingManager::new();
        let invitation = manager
            .create_invitation(ScopeProfile::Phone, "iPhone")
            .unwrap();
        let client = exchange(
            &manager,
            ScopeProfile::Phone,
            &invitation.code,
            b"phone",
            "192.0.2.30",
        )
        .unwrap();
        reserve(&manager, &client.attempt(ScopeProfile::Phone)).deliver(sealed());
        assert!(!manager.has_active_invitation());
        assert_eq!(
            manager.completion_profile(client.exchange_id),
            Some(ScopeProfile::Phone)
        );
        assert_eq!(manager.completion_profile(Uuid::new_v4()), None);

        let mut altered = client.attempt(ScopeProfile::Phone);
        altered.enrollment = b"different enrollment";
        assert!(manager.finish(&altered).is_err());
        assert!(matches!(
            manager.finish(&client.attempt(ScopeProfile::Phone)),
            Ok(Finished::Delivered(redelivered)) if redelivered == sealed()
        ));
        assert!(
            manager
                .finish(&client.attempt(ScopeProfile::Phone))
                .is_err()
        );
        assert_eq!(manager.completion_profile(client.exchange_id), None);
        assert!(
            exchange(
                &manager,
                ScopeProfile::Phone,
                &invitation.code,
                b"again",
                "192.0.2.30",
            )
            .is_err()
        );
    }

    #[test]
    fn reservations_and_redeliveries_expire() {
        let manager = PairingManager::new();
        let invitation = manager
            .create_invitation(ScopeProfile::MacApp, "Test Mac")
            .unwrap();
        let client = exchange(
            &manager,
            ScopeProfile::MacApp,
            &invitation.code,
            b"stuck",
            "127.0.0.1",
        )
        .unwrap();
        let stuck = reserve(&manager, &client.attempt(ScopeProfile::MacApp));
        let later = Instant::now() + EXCHANGE_TTL;
        {
            let mut state = manager.lock();
            PairingManager::purge(&mut state, later);
            assert!(state.reservation.is_none());
            assert!(state.invitation.is_some());
        }
        let retry = exchange(
            &manager,
            ScopeProfile::MacApp,
            &invitation.code,
            b"after-expiry",
            "127.0.0.1",
        )
        .unwrap();
        let reservation = reserve(&manager, &retry.attempt(ScopeProfile::MacApp));
        drop(stuck);
        assert!(manager.lock().reservation.is_some());
        reservation.deliver(sealed());
        let mut state = manager.lock();
        assert!(state.delivered.is_some());
        PairingManager::purge(&mut state, Instant::now() + EXCHANGE_TTL);
        assert!(state.delivered.is_none());
    }

    #[test]
    fn begin_limits_are_temporal_and_non_destructive() {
        let manager = PairingManager::new();
        let invitation = manager
            .create_invitation(ScopeProfile::MacApp, "Test Mac")
            .unwrap();
        for index in 0..MAX_BEGINS_PER_SOURCE {
            exchange(
                &manager,
                ScopeProfile::MacApp,
                &invitation.code,
                format!("context-{index}").as_bytes(),
                "192.0.2.10",
            )
            .unwrap();
        }
        assert!(
            exchange(
                &manager,
                ScopeProfile::MacApp,
                &invitation.code,
                b"extra",
                "192.0.2.10",
            )
            .is_err()
        );
        assert!(manager.has_active_invitation());
    }

    #[test]
    fn a_phone_code_pairs_only_under_the_phone_profile() {
        let manager = PairingManager::new();
        let invitation = manager
            .create_invitation(ScopeProfile::Phone, "iPhone")
            .unwrap();
        assert_eq!(manager.invitation_profile(), Some(ScopeProfile::Phone));
        assert!(
            exchange(
                &manager,
                ScopeProfile::MacApp,
                &invitation.code,
                b"mac",
                "192.0.2.20",
            )
            .is_err()
        );
        let client = exchange(
            &manager,
            ScopeProfile::Phone,
            &invitation.code,
            b"phone",
            "192.0.2.20",
        )
        .unwrap();
        assert!(
            manager
                .finish(&client.attempt(ScopeProfile::MacApp))
                .is_err()
        );
        let reservation = reserve(&manager, &client.attempt(ScopeProfile::Phone));
        assert_eq!(reservation.profile, ScopeProfile::Phone);
        assert_eq!(reservation.label, "iPhone");
        reservation.deliver(sealed());
        assert_eq!(manager.invitation_profile(), None);
    }
}

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};
use uuid::Uuid;

use crate::api::principal::Principal;
use crate::api::scope::Scope;
use crate::util::sse::body_requests::{
    BodyAction, BodyActions, BodyReply, BodyRequestError, body_request_event,
};
use crate::util::sse::types::StreamBundle;

const BODY_REQUEST_QUEUE: usize = 4;

pub(crate) type BodyRequestStream = mpsc::Receiver<StreamBundle>;

pub(crate) struct SseConnectionManager {
    state: Mutex<ConnectionState>,
}

struct ConnectionState {
    connections: HashMap<String, Connection>,
    pending: HashMap<String, PendingRequest>,
}

struct Connection {
    endpoint_id: Option<String>,
    credential_id: Option<String>,
    actions: BodyActions,
    requests: Option<mpsc::Sender<StreamBundle>>,
    cancellation: CancellationToken,
    conversation_updates: bool,
    connected_at: std::time::Instant,
}

struct PendingRequest {
    connection_id: String,
    reply: oneshot::Sender<BodyReply>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BodyCompletionError {
    UnknownRequest,
}

impl SseConnectionManager {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(ConnectionState {
                connections: HashMap::new(),
                pending: HashMap::new(),
            }),
        }
    }

    pub(crate) fn connect(
        self: &Arc<Self>,
        principal: &Principal,
        actions: BodyActions,
    ) -> (ConnectionLease, BodyRequestStream) {
        let actions = if principal.scopes.contains(Scope::CaptureSubmit) {
            actions
        } else {
            BodyActions::default()
        };
        let (sender, receiver) = mpsc::channel(BODY_REQUEST_QUEUE);
        let lease = self.connect_inner(principal, actions, Some(sender), true);
        (lease, receiver)
    }

    pub(crate) fn connect_voice(self: &Arc<Self>, principal: &Principal) -> ConnectionLease {
        self.connect_inner(principal, BodyActions::default(), None, false)
    }

    fn connect_inner(
        self: &Arc<Self>,
        principal: &Principal,
        actions: BodyActions,
        requests: Option<mpsc::Sender<StreamBundle>>,
        conversation_updates: bool,
    ) -> ConnectionLease {
        let connection_id = format!("conn_{}", &Uuid::new_v4().to_string()[..8]);
        let cancellation = CancellationToken::new();
        lock_or_recover(&self.state).connections.insert(
            connection_id.clone(),
            Connection {
                endpoint_id: principal.endpoint_id.clone(),
                credential_id: principal.credential_id.clone(),
                actions,
                requests,
                cancellation: cancellation.clone(),
                conversation_updates,
                connected_at: std::time::Instant::now(),
            },
        );
        info!(
            %connection_id,
            endpoint_id = principal.endpoint_id.as_deref(),
            credential_id = principal.credential_id.as_deref(),
            screen_read = actions.contains(BodyAction::ScreenRead),
            "connection_manager.connected"
        );
        ConnectionLease {
            manager: Arc::clone(self),
            connection_id: Some(connection_id),
            cancellation,
        }
    }

    fn disconnect(&self, connection_id: &str) {
        let mut state = lock_or_recover(&self.state);
        if state.connections.remove(connection_id).is_none() {
            debug!(%connection_id, "connection_manager.ignored_stale_disconnect");
            return;
        }
        state
            .pending
            .retain(|_, pending| pending.connection_id != connection_id);
        info!(%connection_id, "connection_manager.disconnected");
    }

    pub(crate) fn serves(&self, action: BodyAction) -> bool {
        lock_or_recover(&self.state)
            .connections
            .values()
            .any(|connection| connection.can_serve(action))
    }

    pub(crate) async fn request(
        &self,
        action: BodyAction,
        deadline: Duration,
    ) -> Result<serde_json::Value, BodyRequestError> {
        let request_id = format!("breq_{}", Uuid::new_v4().simple());
        let (reply, answer) = oneshot::channel();
        {
            let mut state = lock_or_recover(&self.state);
            let (connection_id, sender) = state
                .connections
                .iter()
                .filter(|(_, connection)| connection.can_serve(action))
                .max_by_key(|(_, connection)| connection.connected_at)
                .and_then(|(id, connection)| {
                    connection
                        .requests
                        .clone()
                        .map(|sender| (id.clone(), sender))
                })
                .ok_or(BodyRequestError::NoBody)?;
            sender
                .try_send(body_request_event(&request_id, action, deadline))
                .map_err(|error| match error {
                    mpsc::error::TrySendError::Full(_) => BodyRequestError::Busy,
                    mpsc::error::TrySendError::Closed(_) => BodyRequestError::Gone,
                })?;
            state.pending.insert(
                request_id.clone(),
                PendingRequest {
                    connection_id: connection_id.clone(),
                    reply,
                },
            );
            debug!(%request_id, %connection_id, action = action.wire_name(), "body_request.sent");
        }
        let outcome = tokio::time::timeout(deadline, answer).await;
        lock_or_recover(&self.state).pending.remove(&request_id);
        match outcome {
            Err(_) => Err(BodyRequestError::TimedOut),
            Ok(Err(_)) => Err(BodyRequestError::Gone),
            Ok(Ok(reply)) => BodyRequestError::from_reply(reply),
        }
    }

    pub(crate) fn complete(
        &self,
        principal: &Principal,
        request_id: &str,
        reply: BodyReply,
    ) -> Result<(), BodyCompletionError> {
        let pending = {
            let mut state = lock_or_recover(&self.state);
            let answered_by_its_body = state
                .pending
                .get(request_id)
                .and_then(|pending| state.connections.get(&pending.connection_id))
                .is_some_and(|connection| {
                    connection.endpoint_id == principal.endpoint_id
                        && connection.credential_id == principal.credential_id
                });
            if !answered_by_its_body {
                return Err(BodyCompletionError::UnknownRequest);
            }
            state
                .pending
                .remove(request_id)
                .ok_or(BodyCompletionError::UnknownRequest)?
        };
        let _ignored = pending.reply.send(reply);
        Ok(())
    }

    pub(crate) fn has_channel(&self, channel: crate::models::ids::ConversationChannelId) -> bool {
        lock_or_recover(&self.state)
            .connections
            .values()
            .any(|connection| {
                connection.conversation_updates
                    && !connection.cancellation.is_cancelled()
                    && connection.channel() == channel
            })
    }

    pub(crate) fn offer_channel(
        &self,
        preferred: crate::models::ids::ConversationChannelId,
        screen_only: bool,
    ) -> Option<crate::models::ids::ConversationChannelId> {
        lock_or_recover(&self.state)
            .connections
            .values()
            .filter(|connection| {
                connection.conversation_updates
                    && !connection.cancellation.is_cancelled()
                    && (!screen_only || connection.actions.contains(BodyAction::ScreenRead))
            })
            .map(|connection| (connection.channel(), connection.connected_at))
            .max_by_key(|(channel, connected)| (*channel == preferred, *connected))
            .map(|(channel, _)| channel)
    }

    pub(crate) fn disconnect_endpoint(&self, endpoint_id: &str) -> usize {
        self.cancel_matching(
            |connection| connection.endpoint_id.as_deref() == Some(endpoint_id),
            "endpoint",
            endpoint_id,
        )
    }

    pub(crate) fn disconnect_credential(&self, credential_id: &str) -> usize {
        self.cancel_matching(
            |connection| connection.credential_id.as_deref() == Some(credential_id),
            "credential",
            credential_id,
        )
    }

    fn cancel_matching(
        &self,
        matches: impl Fn(&Connection) -> bool,
        identity_kind: &'static str,
        identity_id: &str,
    ) -> usize {
        let cancellations = lock_or_recover(&self.state)
            .connections
            .values()
            .filter(|connection| matches(connection))
            .map(|connection| connection.cancellation.clone())
            .collect::<Vec<_>>();
        let count = cancellations.len();
        for cancellation in cancellations {
            cancellation.cancel();
        }
        info!(
            count,
            identity_kind, identity_id, "connection_manager.revoked"
        );
        count
    }

    pub(crate) fn disconnect_all(&self) {
        let connections = {
            let mut state = lock_or_recover(&self.state);
            state.pending.clear();
            std::mem::take(&mut state.connections)
        };
        let count = connections.len();
        for connection in connections.values() {
            connection.cancellation.cancel();
        }
        info!(count, "connection_manager.disconnected_all");
    }

    #[cfg(test)]
    fn connection_count(&self) -> usize {
        lock_or_recover(&self.state).connections.len()
    }

    #[cfg(test)]
    fn pending_count(&self) -> usize {
        lock_or_recover(&self.state).pending.len()
    }
}

impl Connection {
    fn channel(&self) -> crate::models::ids::ConversationChannelId {
        self.endpoint_id.as_deref().map_or(
            crate::models::ids::ConversationChannelId::OWNER,
            crate::models::ids::ConversationChannelId::for_endpoint,
        )
    }

    fn can_serve(&self, action: BodyAction) -> bool {
        self.conversation_updates
            && self.requests.is_some()
            && !self.cancellation.is_cancelled()
            && self.actions.contains(action)
    }
}

pub(crate) struct ConnectionLease {
    manager: Arc<SseConnectionManager>,
    connection_id: Option<String>,
    cancellation: CancellationToken,
}

impl ConnectionLease {
    pub(crate) fn id(&self) -> &str {
        self.connection_id.as_deref().unwrap_or("disconnected")
    }

    pub(crate) async fn cancelled(&self) {
        self.cancellation.cancelled().await;
    }
}

impl Drop for ConnectionLease {
    fn drop(&mut self) {
        if let Some(connection_id) = self.connection_id.take() {
            self.manager.disconnect(&connection_id);
        }
    }
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::error!("connection manager mutex poisoned, recovering");
            poisoned.into_inner()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::api::scope::ScopeSet;
    use crate::util::sse::types::EventType;

    use super::*;

    const DEADLINE: Duration = Duration::from_secs(5);

    #[test]
    fn updates_choose_connected_conversation_surfaces_not_idle_voice_sockets() {
        let manager = Arc::new(SseConnectionManager::new());
        let mac = endpoint_principal("mac", "mac-key");
        let body = endpoint_principal("body", "body-key");
        let _voice = manager.connect_voice(&body);
        assert!(
            manager
                .offer_channel(body.conversation_channel(), false)
                .is_none()
        );
        let _events = manager.connect(&mac, BodyActions::screen());
        assert_eq!(
            manager.offer_channel(body.conversation_channel(), false),
            Some(mac.conversation_channel())
        );
        assert_eq!(
            manager.offer_channel(body.conversation_channel(), true),
            Some(mac.conversation_channel())
        );
        manager.disconnect_endpoint("mac");
        assert!(
            manager
                .offer_channel(mac.conversation_channel(), false)
                .is_none()
        );
    }

    #[tokio::test]
    async fn screen_requests_reach_only_a_body_that_reads_its_screen() {
        let manager = Arc::new(SseConnectionManager::new());
        let cli = Principal::owner();
        let mac = endpoint_principal("mac", "mac-key");
        let (_cli_lease, mut cli_requests) = manager.connect(&cli, BodyActions::default());
        let (_mac_lease, mut mac_requests) = manager.connect(&mac, BodyActions::screen());
        assert!(manager.serves(BodyAction::ScreenRead));

        let waiter = tokio::spawn({
            let manager = Arc::clone(&manager);
            async move { manager.request(BodyAction::ScreenRead, DEADLINE).await }
        });
        let request = mac_requests.recv().await.unwrap();
        assert_eq!(request.event_type, EventType::BodyRequest);
        assert!(cli_requests.try_recv().is_err());

        manager
            .complete(
                &mac,
                &request.message_id,
                BodyReply::Completed {
                    result: serde_json::json!({"text": "inbox"}),
                },
            )
            .unwrap();
        assert_eq!(
            waiter.await.unwrap(),
            Ok(serde_json::json!({"text": "inbox"}))
        );
        assert_eq!(manager.pending_count(), 0);
    }

    #[tokio::test]
    async fn only_the_asked_body_can_answer() {
        let manager = Arc::new(SseConnectionManager::new());
        let mac = endpoint_principal("mac", "mac-key");
        let (_lease, mut requests) = manager.connect(&mac, BodyActions::screen());
        let waiter = tokio::spawn({
            let manager = Arc::clone(&manager);
            async move { manager.request(BodyAction::ScreenRead, DEADLINE).await }
        });
        let request = requests.recv().await.unwrap();
        let declined = || BodyReply::Declined {
            reason: "screen_locked".into(),
        };

        for stranger in [
            endpoint_principal("other-mac", "other-key"),
            endpoint_principal("mac", "old-key"),
            Principal::owner(),
        ] {
            assert_eq!(
                manager.complete(&stranger, &request.message_id, declined()),
                Err(BodyCompletionError::UnknownRequest)
            );
        }
        manager
            .complete(&mac, &request.message_id, declined())
            .unwrap();
        assert_eq!(
            manager.complete(&mac, &request.message_id, declined()),
            Err(BodyCompletionError::UnknownRequest)
        );
        assert_eq!(
            waiter.await.unwrap(),
            Err(BodyRequestError::Declined("screen_locked".into()))
        );
    }

    #[tokio::test]
    async fn a_disconnect_settles_its_pending_request() {
        let manager = Arc::new(SseConnectionManager::new());
        let (lease, mut requests) =
            manager.connect(&endpoint_principal("mac", "mac-key"), BodyActions::screen());
        let waiter = tokio::spawn({
            let manager = Arc::clone(&manager);
            async move { manager.request(BodyAction::ScreenRead, DEADLINE).await }
        });
        let _request = requests.recv().await.unwrap();
        drop(lease);
        assert_eq!(waiter.await.unwrap(), Err(BodyRequestError::Gone));
        assert!(!manager.serves(BodyAction::ScreenRead));
        assert_eq!(manager.pending_count(), 0);
    }

    #[tokio::test]
    async fn an_unanswered_request_times_out_and_is_forgotten() {
        let manager = Arc::new(SseConnectionManager::new());
        let (_lease, _requests) =
            manager.connect(&endpoint_principal("mac", "mac-key"), BodyActions::screen());
        assert_eq!(
            manager
                .request(BodyAction::ScreenRead, Duration::from_millis(20))
                .await,
            Err(BodyRequestError::TimedOut)
        );
        assert_eq!(manager.pending_count(), 0);
    }

    #[tokio::test]
    async fn without_a_screen_body_nothing_is_asked() {
        let manager = Arc::new(SseConnectionManager::new());
        let mac = endpoint_principal("mac", "mac-key");
        let _voice = manager.connect_voice(&mac);
        let (_lease, _requests) = manager.connect(&mac, BodyActions::default());
        assert!(!manager.serves(BodyAction::ScreenRead));
        assert_eq!(
            manager.request(BodyAction::ScreenRead, DEADLINE).await,
            Err(BodyRequestError::NoBody)
        );
    }

    #[tokio::test]
    async fn a_body_that_cannot_answer_is_never_asked() {
        let manager = Arc::new(SseConnectionManager::new());
        let phone = Principal::endpoint(
            "phone".into(),
            "phone-key".into(),
            1,
            ScopeSet::from_static(crate::api::scope::ScopeProfile::Phone.scopes()),
        );
        let (_lease, _requests) = manager.connect(&phone, BodyActions::screen());
        assert!(!manager.serves(BodyAction::ScreenRead));
        assert_eq!(
            manager.offer_channel(phone.conversation_channel(), true),
            None
        );
        assert_eq!(
            manager.offer_channel(phone.conversation_channel(), false),
            Some(phone.conversation_channel())
        );
        assert_eq!(
            manager.request(BodyAction::ScreenRead, DEADLINE).await,
            Err(BodyRequestError::NoBody)
        );
        assert_eq!(manager.disconnect_endpoint("phone"), 1);
    }

    #[tokio::test]
    async fn a_body_that_stops_reading_requests_is_busy_not_flooded() {
        let manager = Arc::new(SseConnectionManager::new());
        let (_lease, _requests) =
            manager.connect(&endpoint_principal("mac", "mac-key"), BodyActions::screen());
        let mut waiters = Vec::new();
        for _ in 0..BODY_REQUEST_QUEUE {
            waiters.push(tokio::spawn({
                let manager = Arc::clone(&manager);
                async move { manager.request(BodyAction::ScreenRead, DEADLINE).await }
            }));
        }
        while manager.pending_count() < BODY_REQUEST_QUEUE {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            manager.request(BodyAction::ScreenRead, DEADLINE).await,
            Err(BodyRequestError::Busy)
        );
        manager.disconnect_all();
        for waiter in waiters {
            assert_eq!(waiter.await.unwrap(), Err(BodyRequestError::Gone));
        }
    }

    #[tokio::test]
    async fn endpoint_revocation_cancels_only_matching_leases() {
        let manager = Arc::new(SseConnectionManager::new());
        let (first, _first_requests) = manager.connect(
            &endpoint_principal("mac-a", "credential-a"),
            BodyActions::default(),
        );
        let (second, _second_requests) = manager.connect(
            &endpoint_principal("mac-b", "credential-b"),
            BodyActions::default(),
        );

        assert_eq!(manager.disconnect_endpoint("mac-a"), 1);
        first.cancelled().await;
        assert!(!second.cancellation.is_cancelled());
        assert_eq!(manager.connection_count(), 2);
    }

    #[tokio::test]
    async fn credential_revocation_does_not_cancel_new_generation() {
        let manager = Arc::new(SseConnectionManager::new());
        let (old, _old_requests) = manager.connect(
            &endpoint_principal("mac-a", "credential-old"),
            BodyActions::screen(),
        );
        let (current, _current_requests) = manager.connect(
            &endpoint_principal("mac-a", "credential-current"),
            BodyActions::screen(),
        );

        assert_eq!(manager.disconnect_credential("credential-old"), 1);
        old.cancelled().await;
        assert!(!current.cancellation.is_cancelled());
        assert!(manager.serves(BodyAction::ScreenRead));
    }

    #[test]
    fn disconnect_all_forgets_every_connection_and_later_drops_are_idempotent() {
        let manager = Arc::new(SseConnectionManager::new());
        let first = manager.connect(&Principal::owner(), BodyActions::screen());
        let second = manager.connect_voice(&Principal::owner());
        manager.disconnect_all();
        assert_eq!(manager.connection_count(), 0);
        assert!(!manager.serves(BodyAction::ScreenRead));
        drop((first, second));
        assert_eq!(manager.connection_count(), 0);
    }

    fn endpoint_principal(endpoint_id: &str, credential_id: &str) -> Principal {
        Principal::endpoint(
            endpoint_id.to_owned(),
            credential_id.to_owned(),
            1,
            ScopeSet::all(),
        )
    }
}

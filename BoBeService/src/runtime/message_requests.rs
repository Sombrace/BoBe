use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::ids::{ConversationChannelId, ConversationTurnId};
use crate::runtime::run_registry::RunKind;
use crate::runtime::run_registry::RunRegistry;
use crate::runtime::session::RuntimeSession;

pub(crate) struct MessageRequests {
    pool: sqlx::SqlitePool,
    session: Arc<RuntimeSession>,
    in_flight: TaskTracker,
    runs: Arc<RunRegistry>,
    personal_data: Arc<tokio::sync::RwLock<()>>,
    submission: tokio::sync::Mutex<()>,
    shutdown: CancellationToken,
}

#[derive(Debug)]
enum RequestOutcome {
    Completed,
    Failed,
    Deferred,
}

#[derive(Clone)]
pub(crate) struct RequestDelivery {
    pub(crate) sender: mpsc::Sender<RequestEvent>,
    pub(crate) cancellation: CancellationToken,
    pub(crate) device_context: String,
}

pub(crate) enum RequestEvent {
    Text(String),
    Finished { status: &'static str },
}

#[derive(Debug, Serialize)]
pub(crate) struct RequestAcceptance {
    pub(crate) message_id: String,
    pub(crate) request_status: &'static str,
    pub(crate) replayed: bool,
    #[serde(skip)]
    pub(crate) started: bool,
}

#[derive(sqlx::FromRow)]
struct RequestRecord {
    channel_id: ConversationChannelId,
    content_sha256: Vec<u8>,
    content: String,
    status: String,
}

impl MessageRequests {
    pub(crate) fn new(
        pool: sqlx::SqlitePool,
        session: Arc<RuntimeSession>,
        in_flight: TaskTracker,
        runs: Arc<RunRegistry>,
        personal_data: Arc<tokio::sync::RwLock<()>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            session,
            in_flight,
            runs,
            personal_data,
            submission: tokio::sync::Mutex::new(()),
            shutdown: CancellationToken::new(),
        })
    }

    pub(crate) fn begin_shutdown(&self) {
        self.shutdown.cancel();
    }

    pub(crate) async fn submit(
        self: &Arc<Self>,
        request_id: Uuid,
        content: String,
        delivery: Option<RequestDelivery>,
        channel: ConversationChannelId,
    ) -> Result<RequestAcceptance, AppError> {
        let received_at = chrono::Utc::now();
        let _personal = Arc::clone(&self.personal_data)
            .try_read_owned()
            .map_err(|_| AppError::Conflict("personal data maintenance is active".into()))?;
        let _submission = self.submission.lock().await;
        if self.shutdown.is_cancelled() {
            return Err(AppError::ServiceUnavailable(
                "BoBe is stopping; retry this request after restart".into(),
            ));
        }
        if content.trim().is_empty() {
            return Err(AppError::Validation("content must not be empty".into()));
        }
        let digest = Sha256::digest(content.as_bytes()).to_vec();
        let message_id =
            crate::models::ids::message_id_for_turn(ConversationTurnId::from(request_id));
        let existing = sqlx::query_as::<_, RequestRecord>(
            "SELECT channel_id, content_sha256, content, status FROM message_requests WHERE request_id = ?1",
        )
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(existing) = &existing {
            if existing.channel_id != channel
                || existing.content_sha256 != digest
                || existing.content != content
            {
                return Err(AppError::Conflict(
                    "request_id was already used for different content".into(),
                ));
            }
            match existing.status.as_str() {
                "queued" | "running" | "completed" => {
                    return Ok(RequestAcceptance {
                        message_id,
                        request_status: if existing.status == "completed" {
                            "completed"
                        } else if existing.status == "queued" {
                            "queued"
                        } else {
                            "running"
                        },
                        replayed: true,
                        started: false,
                    });
                }
                "failed" => return Err(unsafe_replay_error()),
                other => {
                    return Err(AppError::Internal(format!(
                        "Unknown message request status: {other}"
                    )));
                }
            }
        } else if request_id_is_tombstoned(&self.pool, request_id).await? {
            return Err(unsafe_replay_error());
        }
        let queued: i64 =
            sqlx::query_scalar("SELECT count(*) FROM message_requests WHERE status='queued'")
                .fetch_one(&self.pool)
                .await?;
        if queued >= 64 {
            return Err(AppError::Conflict("incoming message queue is full".into()));
        }
        let owner_input = self
            .session
            .bind_owner_answer(channel, &content, received_at)
            .await?;
        sqlx::query("INSERT INTO message_requests (request_id,channel_id,delivery_kind,content_sha256,content,status,owner_input_id) VALUES (?1,?2,?3,?4,?5,'queued',?6)")
            .bind(request_id).bind(channel).bind(if delivery.is_some() { "device" } else { "surface" })
            .bind(digest).bind(&content).bind(owner_input).execute(&self.pool).await?;
        self.runs.interrupt_automation(channel);
        if let Err(error) = self.start(request_id, content, delivery, channel, owner_input) {
            self.mark_failed(request_id, "admission_failed").await;
            return Err(error);
        }
        Ok(RequestAcceptance {
            message_id,
            request_status: "queued",
            replayed: existing.is_some(),
            started: true,
        })
    }

    fn start(
        self: &Arc<Self>,
        request_id: Uuid,
        content: String,
        delivery: Option<RequestDelivery>,
        channel: ConversationChannelId,
        owner_input: Option<Uuid>,
    ) -> Result<(), AppError> {
        let queued = self.session.queue_user_message(channel)?;
        let cancellation = delivery
            .as_ref()
            .map(|value| value.cancellation.clone())
            .unwrap_or_default();
        let registration = self.runs.register_cancellation(
            request_id,
            cancellation.clone(),
            RunKind::User,
            Some(channel),
        )?;
        let service = Arc::clone(self);
        self.in_flight.spawn(async move {
            let _registration = registration;
            let mut admission = None;
            let mut personal = None;
            let result: Result<RequestOutcome, AppError> = async {
                admission = Some(tokio::select! {
                    biased;
                    () = cancellation.cancelled() => return Ok(RequestOutcome::Failed),
                    () = service.shutdown.cancelled() => return Ok(RequestOutcome::Deferred),
                    guard = queued.admit() => guard,
                });
                personal = Some(
                    Arc::clone(&service.personal_data)
                        .try_read_owned()
                        .map_err(|_| {
                            AppError::Conflict("personal data maintenance is active".into())
                        })?,
                );
                if let Some(outcome) = prepare_execution(
                    &service.pool,
                    &service.shutdown,
                    &cancellation,
                    request_id,
                    service
                        .session
                        .answer_bound_input(owner_input, request_id, &content, channel),
                )
                .await?
                {
                    return Ok(outcome);
                }
                service.runs.started(request_id);
                let session = Arc::clone(&service.session);
                let work_delivery = delivery.clone();
                let mut work = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
                    let turn_id = ConversationTurnId::from(request_id);
                    let message_id = crate::models::ids::message_id_for_turn(turn_id);
                    if let Some(delivery) = work_delivery {
                        let content = format!(
                            "{content}\n\nDevice observations (data):\n{}",
                            delivery.device_context
                        );
                        session
                            .handle_endpoint_text_with_observer(
                                &content,
                                &message_id,
                                turn_id,
                                channel,
                                async move |text| {
                                    if !matches!(
                                        tokio::time::timeout(
                                            Duration::from_secs(2),
                                            delivery.sender.send(RequestEvent::Text(text))
                                        )
                                        .await,
                                        Ok(Ok(()))
                                    ) {
                                        delivery.cancellation.cancel();
                                    }
                                },
                            )
                            .await
                    } else {
                        session
                            .handle_user_message(&content, &message_id, turn_id, channel)
                            .await
                    }
                }));
                let outcome = tokio::select! {
                    biased;
                    () = cancellation.cancelled() => { work.abort(); work.await },
                    () = service.shutdown.cancelled() => { work.abort(); work.await },
                    outcome = &mut work => outcome,
                };
                match outcome {
                    Ok(true) => Ok(RequestOutcome::Completed),
                    Ok(false) => Ok(RequestOutcome::Failed),
                    Err(error) if error.is_cancelled() => Ok(RequestOutcome::Failed),
                    Err(error) => Err(AppError::Internal(format!(
                        "message execution failed: {error}"
                    ))),
                }
            }
            .await;
            let (status, reason) = match &result {
                Ok(RequestOutcome::Completed) => ("completed", None),
                Ok(RequestOutcome::Failed)
                    if cancellation.is_cancelled() || service.shutdown.is_cancelled() =>
                {
                    ("failed", Some("turn_cancelled"))
                }
                Ok(RequestOutcome::Failed) => ("failed", Some("turn_failed")),
                Ok(RequestOutcome::Deferred) => {
                    tracing::info!(%request_id, "message_request.deferred_until_restart");
                    return;
                }
                Err(error) => {
                    tracing::error!(%error,%request_id,"message_request.execution_failed");
                    ("failed", Some("turn_failed"))
                }
            };
            service
                .session
                .settle_interrupted_turn(ConversationTurnId::from(request_id))
                .await;
            if let Err(error) = sqlx::query(
                "UPDATE message_requests SET status = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE request_id = ?2 AND status IN ('queued','running')",
            ).bind(status).bind(request_id).execute(&service.pool).await {
                tracing::error!(%error, %request_id, "message_request.status_update_failed");
            }
            if let Some(reason) = reason {
                service
                    .session
                    .fail_message(ConversationTurnId::from(request_id), channel, reason);
            }
            if let Some(delivery) = delivery {
                let result = if reason == Some("turn_cancelled") {
                    "cancelled"
                } else {
                    status
                };
                if !matches!(
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        delivery
                            .sender
                            .send(RequestEvent::Finished { status: result }),
                    )
                    .await,
                    Ok(Ok(()))
                ) {
                    tracing::warn!(%request_id,"message_request.terminal_delivery_failed");
                }
            }
            drop(personal);
            drop(admission);
        });
        Ok(())
    }

    pub(crate) async fn completed_response(
        &self,
        request_id: Uuid,
    ) -> Result<Option<String>, AppError> {
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT content FROM conversation_turns WHERE id = ?1 AND role = 'assistant' AND is_complete = 1",
        ).bind(ConversationTurnId::from(request_id)).fetch_optional(&self.pool).await?)
    }

    pub(crate) async fn recover(self: &Arc<Self>) -> Result<(), AppError> {
        reconcile_interrupted_requests(&self.pool).await?;
        sqlx::query("UPDATE message_requests SET status='failed' WHERE status='queued' AND delivery_kind='device'")
            .execute(&self.pool).await?;
        let queued = sqlx::query_as::<_, (Uuid, Vec<u8>, String, ConversationChannelId,Option<Uuid>)>(
            "SELECT request_id, content_sha256, content, channel_id,owner_input_id FROM message_requests WHERE status = 'queued' ORDER BY rowid",
        ).fetch_all(&self.pool).await?;
        if queued.len() > 64 {
            return Err(AppError::Config(
                "retained incoming queue exceeds its capacity".into(),
            ));
        }
        for (request_id, digest, content, channel, owner_input) in queued {
            if Sha256::digest(content.as_bytes()).as_slice() != digest {
                self.mark_failed(request_id, "content_digest_mismatch")
                    .await;
                continue;
            }
            self.start(request_id, content, None, channel, owner_input)?;
        }
        Ok(())
    }

    async fn mark_failed(&self, request_id: Uuid, reason: &str) {
        tracing::warn!(%request_id, reason, "message_request.failed");
        if let Err(error) = sqlx::query(
            "UPDATE message_requests SET status = 'failed', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE request_id = ?1",
        ).bind(request_id).execute(&self.pool).await {
            tracing::error!(%error, %request_id, "message_request.status_update_failed");
        }
    }
}

async fn claim_request(pool: &sqlx::SqlitePool, request_id: Uuid) -> Result<(), AppError> {
    let claimed = sqlx::query(
        "UPDATE message_requests SET status = 'running', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE request_id = ?1 AND status = 'queued'",
    ).bind(request_id).execute(pool).await?.rows_affected();
    if claimed != 1 {
        return Err(AppError::Conflict(
            "message request is no longer queued".into(),
        ));
    }
    Ok(())
}

async fn prepare_execution(
    pool: &sqlx::SqlitePool,
    shutdown: &CancellationToken,
    cancellation: &CancellationToken,
    request_id: Uuid,
    bind_answer: impl Future<Output = Result<(), AppError>>,
) -> Result<Option<RequestOutcome>, AppError> {
    if shutdown.is_cancelled() {
        return Ok(Some(RequestOutcome::Deferred));
    }
    claim_request(pool, request_id).await?;
    if let Some(outcome) = stop_before_execution(pool, shutdown, cancellation, request_id).await {
        return Ok(Some(outcome));
    }
    bind_answer.await?;
    Ok(stop_before_execution(pool, shutdown, cancellation, request_id).await)
}

async fn stop_before_execution(
    pool: &sqlx::SqlitePool,
    shutdown: &CancellationToken,
    cancellation: &CancellationToken,
    request_id: Uuid,
) -> Option<RequestOutcome> {
    if cancellation.is_cancelled() {
        return Some(RequestOutcome::Failed);
    }
    if !shutdown.is_cancelled() {
        return None;
    }
    if let Err(error) = sqlx::query(
        "UPDATE message_requests SET status = 'queued', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE request_id = ?1 AND status = 'running'",
    )
    .bind(request_id)
    .execute(pool)
    .await
    {
        tracing::error!(%error, %request_id, "message_request.requeue_failed");
    }
    Some(RequestOutcome::Deferred)
}

pub(crate) async fn request_id_is_tombstoned(
    pool: &sqlx::SqlitePool,
    request_id: Uuid,
) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM message_request_tombstones WHERE request_id = ?1)",
    )
    .bind(request_id)
    .fetch_one(pool)
    .await?)
}

pub(crate) async fn reconcile_interrupted_requests(
    pool: &sqlx::SqlitePool,
) -> Result<(), AppError> {
    sqlx::query("UPDATE message_requests SET status = 'completed', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE status = 'running' AND EXISTS (SELECT 1 FROM conversation_turns WHERE conversation_turns.id = message_requests.request_id AND conversation_turns.is_complete = 1)")
        .execute(pool).await?;
    sqlx::query("UPDATE message_requests SET status = 'failed', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE status = 'running'")
        .execute(pool).await?;
    Ok(())
}

fn unsafe_replay_error() -> AppError {
    AppError::RequestReplayUnsafe("This message's earlier delivery was interrupted and cannot be retried safely. Check the conversation, then send it again as a new message if needed.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::assert_matches;
    use std::sync::atomic::Ordering;

    async fn log_status_changes(pool: &sqlx::SqlitePool) {
        sqlx::raw_sql(
            "CREATE TABLE status_log (status TEXT NOT NULL);
             CREATE TRIGGER log_status AFTER UPDATE OF status ON message_requests
             BEGIN INSERT INTO status_log (status) VALUES (NEW.status); END;",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn queued_request(pool: &sqlx::SqlitePool) -> Uuid {
        log_status_changes(pool).await;
        let request_id = Uuid::new_v4();
        sqlx::query("INSERT INTO message_requests (request_id,content_sha256,content) VALUES (?1,?2,'hello')")
            .bind(request_id).bind(Sha256::digest(b"hello").to_vec()).execute(pool).await.unwrap();
        request_id
    }

    async fn status_history(pool: &sqlx::SqlitePool, request_id: Uuid) -> (String, Vec<String>) {
        let status =
            sqlx::query_scalar("SELECT status FROM message_requests WHERE request_id = ?1")
                .bind(request_id)
                .fetch_one(pool)
                .await
                .unwrap();
        let history = sqlx::query_scalar("SELECT status FROM status_log ORDER BY rowid")
            .fetch_all(pool)
            .await
            .unwrap();
        (status, history)
    }

    async fn let_ready_tasks_run() {
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn an_accepted_message_stays_queued_when_shutdown_begins_before_it_runs() {
        let root = std::env::temp_dir().join(format!("bobe-message-requests-{}", Uuid::new_v4()));
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let session = RuntimeSession::for_tests(&root, pool.clone()).await;
        let in_flight = TaskTracker::new();
        let requests = MessageRequests::new(
            pool.clone(),
            Arc::clone(&session),
            in_flight.clone(),
            Arc::clone(&session.runs),
            Arc::new(tokio::sync::RwLock::new(())),
        );
        log_status_changes(&pool).await;
        let previous_turn = session
            .try_begin_user_message(ConversationChannelId::OWNER)
            .unwrap();
        let request_id = Uuid::new_v4();

        let accepted = requests
            .submit(
                request_id,
                "hello".into(),
                None,
                ConversationChannelId::OWNER,
            )
            .await
            .unwrap();
        assert_eq!(accepted.request_status, "queued");
        let_ready_tasks_run().await;
        let connection = pool.acquire().await.unwrap();
        drop(previous_turn);
        let_ready_tasks_run().await;
        requests.begin_shutdown();
        drop(connection);
        in_flight.close();
        tokio::time::timeout(Duration::from_secs(5), in_flight.wait())
            .await
            .unwrap();

        assert_eq!(
            status_history(&pool, request_id).await,
            (
                "queued".to_owned(),
                vec!["running".to_owned(), "queued".to_owned()]
            )
        );
        drop(tokio::fs::remove_dir_all(root).await);
    }

    #[tokio::test]
    async fn shutdown_during_the_claim_returns_the_request_to_the_queue() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let request_id = queued_request(&pool).await;
        let shutdown = CancellationToken::new();
        let bound = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let connection = pool.acquire().await.unwrap();
        let preparation = tokio::spawn({
            let pool = pool.clone();
            let shutdown = shutdown.clone();
            let bound = Arc::clone(&bound);
            async move {
                prepare_execution(
                    &pool,
                    &shutdown,
                    &CancellationToken::new(),
                    request_id,
                    async move {
                        bound.store(true, Ordering::Release);
                        Ok(())
                    },
                )
                .await
            }
        });
        let_ready_tasks_run().await;
        assert!(!preparation.is_finished());

        shutdown.cancel();
        drop(connection);

        let outcome = preparation.await.unwrap().unwrap();
        assert_matches!(outcome, Some(RequestOutcome::Deferred));
        assert!(!bound.load(Ordering::Acquire));
        assert_eq!(
            status_history(&pool, request_id).await,
            (
                "queued".to_owned(),
                vec!["running".to_owned(), "queued".to_owned()]
            )
        );
    }

    #[tokio::test]
    async fn shutdown_while_an_owner_answer_is_bound_returns_the_request_to_the_queue() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let request_id = queued_request(&pool).await;
        let shutdown = CancellationToken::new();

        let outcome = prepare_execution(
            &pool,
            &shutdown,
            &CancellationToken::new(),
            request_id,
            async {
                shutdown.cancel();
                Ok(())
            },
        )
        .await
        .unwrap();

        assert_matches!(outcome, Some(RequestOutcome::Deferred));
        assert_eq!(
            status_history(&pool, request_id).await,
            (
                "queued".to_owned(),
                vec!["running".to_owned(), "queued".to_owned()]
            )
        );
    }

    #[tokio::test]
    async fn an_owner_cancellation_before_execution_is_not_requeued() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let request_id = queued_request(&pool).await;
        let shutdown = CancellationToken::new();
        let cancellation = CancellationToken::new();

        let outcome = prepare_execution(&pool, &shutdown, &cancellation, request_id, async {
            cancellation.cancel();
            shutdown.cancel();
            Ok(())
        })
        .await
        .unwrap();

        assert_matches!(outcome, Some(RequestOutcome::Failed));
        assert_eq!(
            status_history(&pool, request_id).await,
            ("running".to_owned(), vec!["running".to_owned()])
        );
    }

    #[tokio::test]
    async fn a_request_claimed_before_shutdown_starts_running() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let request_id = queued_request(&pool).await;
        let shutdown = CancellationToken::new();

        let outcome = prepare_execution(
            &pool,
            &shutdown,
            &CancellationToken::new(),
            request_id,
            async { Ok(()) },
        )
        .await
        .unwrap();
        shutdown.cancel();

        assert!(outcome.is_none());
        assert_eq!(
            status_history(&pool, request_id).await,
            ("running".to_owned(), vec!["running".to_owned()])
        );
        assert_matches!(
            prepare_execution(
                &pool,
                &shutdown,
                &CancellationToken::new(),
                request_id,
                async { Ok(()) }
            )
            .await,
            Ok(Some(RequestOutcome::Deferred))
        );
    }

    #[tokio::test]
    async fn queued_requests_are_claimed_once_and_failed_claims_retain_the_accepted_intent() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        sqlx::query("CREATE TRIGGER reject_claim BEFORE UPDATE OF status ON message_requests BEGIN SELECT RAISE(ABORT, 'claim failed'); END")
            .execute(&pool).await.unwrap();
        let request_id = Uuid::new_v4();
        sqlx::query("INSERT INTO message_requests (request_id,content_sha256,content) VALUES (?1,?2,'button press')")
            .bind(request_id).bind(Sha256::digest(b"button press").to_vec()).execute(&pool).await.unwrap();
        assert!(claim_request(&pool, request_id).await.is_err());
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM message_requests")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
        sqlx::query("DROP TRIGGER reject_claim")
            .execute(&pool)
            .await
            .unwrap();
        claim_request(&pool, request_id).await.unwrap();
        let status: String =
            sqlx::query_scalar("SELECT status FROM message_requests WHERE request_id = ?1")
                .bind(request_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "running");
        assert!(claim_request(&pool, request_id).await.is_err());
    }
}

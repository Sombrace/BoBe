use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, State};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::error::AppError;
use crate::models::ids::{ConversationId, ConversationTurnId};
use crate::models::types::TurnRole;

#[derive(Debug, Deserialize)]
pub(crate) struct ConversationMessageRequest {
    pub(crate) content: String,
    pub(crate) request_id: Uuid,
}

pub(crate) use crate::runtime::message_requests::RequestAcceptance as ConversationMessageResponse;

#[derive(Debug, Serialize)]
pub(crate) struct ConversationSnapshotResponse {
    pub(crate) conversation_id: Option<ConversationId>,
    pub(crate) turns: Vec<ConversationTurnResponse>,
    pub(crate) pending_updates: Vec<ConversationTurnResponse>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ConversationTurnResponse {
    pub(crate) id: ConversationTurnId,
    pub(crate) message_id: String,
    pub(crate) role: TurnRole,
    pub(crate) content: String,
    pub(crate) is_complete: bool,
    pub(crate) created_at: chrono::DateTime<chrono::Utc>,
}

impl From<crate::models::conversation::ConversationTurn> for ConversationTurnResponse {
    fn from(turn: crate::models::conversation::ConversationTurn) -> Self {
        Self {
            id: turn.id,
            message_id: crate::models::ids::message_id_for_turn(turn.id),
            role: turn.role,
            content: turn.content,
            is_complete: turn.is_complete,
            created_at: turn.created_at,
        }
    }
}

pub(crate) async fn current_conversation(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<crate::api::principal::Principal>,
) -> Result<Json<ConversationSnapshotResponse>, AppError> {
    let pending_updates = state
        .runtime
        .runtime_session
        .unpresented_updates(principal.conversation_channel())
        .await?
        .into_iter()
        .map(ConversationTurnResponse::from)
        .collect();
    let Some((conversation, turns)) = state
        .runtime
        .runtime_session
        .current_conversation(200, principal.conversation_channel())
        .await?
    else {
        return Ok(Json(ConversationSnapshotResponse {
            conversation_id: None,
            turns: Vec::new(),
            pending_updates,
        }));
    };
    Ok(Json(ConversationSnapshotResponse {
        conversation_id: Some(conversation.id),
        turns: turns
            .into_iter()
            .map(ConversationTurnResponse::from)
            .collect(),
        pending_updates,
    }))
}

pub(crate) async fn presented(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<crate::api::principal::Principal>,
    axum::extract::Path(turn): axum::extract::Path<ConversationTurnId>,
) -> Result<axum::http::StatusCode, AppError> {
    state
        .runtime
        .runtime_session
        .acknowledge_presentation(turn, principal.conversation_channel())
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
pub(crate) async fn send_message(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<crate::api::principal::Principal>,
    Json(body): Json<ConversationMessageRequest>,
) -> Result<Json<ConversationMessageResponse>, AppError> {
    Ok(Json(
        state
            .runtime
            .message_requests
            .submit(
                body.request_id,
                body.content,
                None,
                principal.conversation_channel(),
            )
            .await?,
    ))
}

#[derive(Debug, Serialize)]
pub(crate) struct CancelRunResponse {
    pub(crate) run_id: Uuid,
    pub(crate) cancellation_requested: bool,
}

pub(crate) async fn cancel_run(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<crate::api::principal::Principal>,
    axum::extract::Path(run_id): axum::extract::Path<Uuid>,
) -> Result<Json<CancelRunResponse>, AppError> {
    let kind = state
        .runtime
        .runs
        .kind(run_id, run_channel(&principal))
        .ok_or_else(|| AppError::NotFound("active run was not found in this channel".into()))?;
    if kind == crate::runtime::run_registry::RunKind::Background {
        state.runtime.background.jobs.cancel(run_id).await?;
    }
    if !state.runtime.runs.cancel(run_id) {
        return Err(AppError::NotFound(format!(
            "active run {run_id} was not found"
        )));
    }
    tracing::info!(%run_id, "api.run_cancel_requested");
    Ok(Json(CancelRunResponse {
        run_id,
        cancellation_requested: true,
    }))
}

fn run_channel(
    principal: &crate::api::principal::Principal,
) -> Option<crate::models::ids::ConversationChannelId> {
    (principal.kind != crate::api::principal::PrincipalKind::Owner)
        .then(|| principal.conversation_channel())
}

pub(crate) async fn list_runs(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<crate::api::principal::Principal>,
) -> Json<Vec<crate::runtime::run_registry::RunStatus>> {
    Json(
        state
            .runtime
            .runs
            .list(Some(principal.conversation_channel())),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunInputRequest {
    text: String,
    #[serde(default)]
    follow_up: bool,
}

#[derive(Serialize)]
pub(crate) struct RunInputResponse {
    run_id: Uuid,
    input_id: Uuid,
    queued: bool,
    replayed: bool,
}

pub(crate) async fn submit_run_input(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<crate::api::principal::Principal>,
    axum::extract::Path((run_id, input_id)): axum::extract::Path<(Uuid, Uuid)>,
    Json(input): Json<RunInputRequest>,
) -> Result<Json<RunInputResponse>, AppError> {
    if input.text.trim().is_empty() || input.text.len() > 32768 {
        return Err(AppError::Validation(
            "run input requires 1-32768 bytes of text".into(),
        ));
    }
    state
        .runtime
        .runs
        .kind(run_id, run_channel(&principal))
        .ok_or_else(|| AppError::NotFound("active run was not found in this channel".into()))?;
    let fresh = state
        .runtime
        .runs
        .begin_input(run_id, input_id, &input.text, input.follow_up)?;
    if fresh {
        let result = state
            .runtime
            .workers
            .steer_run(run_id, &input.text, input.follow_up)
            .await;
        state
            .runtime
            .runs
            .finish_input(run_id, input_id, result.is_ok());
        result?;
    }
    Ok(Json(RunInputResponse {
        run_id,
        input_id,
        queued: true,
        replayed: !fresh,
    }))
}

pub(crate) async fn recover_message_requests(state: &Arc<AppState>) -> Result<(), AppError> {
    state.runtime.message_requests.recover().await
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::db::test_helpers::in_memory_pool;
    use crate::runtime::message_requests::{
        reconcile_interrupted_requests, request_id_is_tombstoned,
    };

    #[tokio::test]
    async fn restart_reconciles_completed_and_ambiguous_running_requests() {
        let pool = in_memory_pool().await;
        let conversation_id = ConversationId::new();
        let completed_request = Uuid::new_v4();
        let ambiguous_request = Uuid::new_v4();
        sqlx::query("INSERT INTO conversations (id, state) VALUES (?1, 'active')")
            .bind(conversation_id)
            .execute(&pool)
            .await
            .expect("insert conversation");
        sqlx::query(
            "INSERT INTO conversation_turns \
             (id, role, content, is_complete, conversation_id) \
             VALUES (?1, 'assistant', 'done', 1, ?2)",
        )
        .bind(completed_request)
        .bind(conversation_id)
        .execute(&pool)
        .await
        .expect("insert completed turn");
        for request_id in [completed_request, ambiguous_request] {
            sqlx::query(
                "INSERT INTO message_requests \
                 (request_id, content_sha256, content, status) \
                 VALUES (?1, ?2, 'hello', 'running')",
            )
            .bind(request_id)
            .bind(vec![0_u8; 32])
            .execute(&pool)
            .await
            .expect("insert running request");
        }

        reconcile_interrupted_requests(&pool)
            .await
            .expect("reconcile requests");

        let completed_status: String =
            sqlx::query_scalar("SELECT status FROM message_requests WHERE request_id = ?1")
                .bind(completed_request)
                .fetch_one(&pool)
                .await
                .expect("load completed status");
        let ambiguous_status: String =
            sqlx::query_scalar("SELECT status FROM message_requests WHERE request_id = ?1")
                .bind(ambiguous_request)
                .fetch_one(&pool)
                .await
                .expect("load ambiguous status");
        assert_eq!(completed_status, "completed");
        assert_eq!(ambiguous_status, "failed");
    }

    #[tokio::test]
    async fn retired_request_id_remains_blocked() {
        let pool = in_memory_pool().await;
        let request_id = Uuid::new_v4();
        sqlx::query("INSERT INTO message_request_tombstones (request_id) VALUES (?1)")
            .bind(request_id)
            .execute(&pool)
            .await
            .expect("insert request tombstone");

        assert!(
            request_id_is_tombstoned(&pool, request_id)
                .await
                .expect("query request tombstone")
        );
    }
}

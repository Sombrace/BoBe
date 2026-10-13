use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::WatchStream;

use crate::app_state::AppState;
use crate::error::AppError;
use crate::services::ollama::binary_manager::DownloadProgress;
use crate::services::ollama::install_service::{InstallRequest, InstallSnapshot, InstallStatus};
use crate::services::ollama::manager::PullProgress;

#[derive(Debug, Deserialize)]
pub(crate) struct InstallRequestBody {
    pub(crate) profile: String,
    pub(crate) chat_model: String,
    pub(crate) batch_model: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct InstallStartedResponse {
    pub(crate) message: String,
}

pub(crate) async fn start_install(
    State(state): State<Arc<AppState>>,
    Json(body): Json<InstallRequestBody>,
) -> Result<(axum::http::StatusCode, Json<InstallStartedResponse>), AppError> {
    let profile_id = crate::provider::ProfileId::parse(&body.profile)?;
    let config = state.config();
    let profile = config.providers.profiles.get(&profile_id).ok_or_else(|| {
        AppError::Validation(format!("provider profile '{}' was not found", body.profile))
    })?;
    if profile.local_runtime != Some(crate::provider::LocalRuntimeKind::Ollama) {
        return Err(AppError::Validation(format!(
            "provider profile '{}' is not an Ollama runtime",
            body.profile
        )));
    }
    drop(config);
    state
        .services
        .ollama_install
        .start(InstallRequest {
            chat_model: body.chat_model,
            batch_model: body.batch_model,
        })
        .await?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(InstallStartedResponse {
            message: "Install started".into(),
        }),
    ))
}

pub(crate) async fn cancel_install(
    State(state): State<Arc<AppState>>,
) -> Result<Json<InstallStartedResponse>, AppError> {
    state.services.ollama_install.cancel().await;
    Ok(Json(InstallStartedResponse {
        message: "Cancel requested".into(),
    }))
}

#[derive(Debug, Serialize)]
struct DownloadProgressDto {
    current_bytes: u64,
    total_bytes: Option<u64>,
    percent: Option<u8>,
}

impl From<&DownloadProgress> for DownloadProgressDto {
    fn from(p: &DownloadProgress) -> Self {
        Self {
            current_bytes: p.current_bytes,
            total_bytes: p.total_bytes,
            percent: p.percent,
        }
    }
}

#[derive(Debug, Serialize)]
struct PullProgressDto {
    status: String,
    completed_bytes: Option<u64>,
    total_bytes: Option<u64>,
    percent: Option<u8>,
}

impl From<&PullProgress> for PullProgressDto {
    fn from(p: &PullProgress) -> Self {
        Self {
            status: p.status.clone(),
            completed_bytes: p.completed_bytes,
            total_bytes: p.total_bytes,
            percent: p.percent,
        }
    }
}

#[derive(Debug, Serialize)]
struct InstallSnapshotDto {
    status: String,
    error: Option<String>,
    runtime: DownloadProgressDto,
    chat_model: PullProgressDto,
    batch_model: PullProgressDto,
}

impl From<&InstallSnapshot> for InstallSnapshotDto {
    fn from(s: &InstallSnapshot) -> Self {
        let (status, error) = match &s.status {
            InstallStatus::Idle => ("idle", None),
            InstallStatus::Running => ("running", None),
            InstallStatus::Complete => ("complete", None),
            InstallStatus::Canceled => ("canceled", None),
            InstallStatus::Failed(msg) => ("failed", Some(msg.clone())),
        };
        Self {
            status: status.to_string(),
            error,
            runtime: DownloadProgressDto::from(&s.runtime),
            chat_model: PullProgressDto::from(&s.chat_model),
            batch_model: PullProgressDto::from(&s.batch_model),
        }
    }
}

pub(crate) async fn install_status_stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.services.ollama_install.subscribe().await;
    let stream = WatchStream::new(rx).map(|snap| {
        let dto = InstallSnapshotDto::from(&snap);
        let payload = serde_json::to_string(&dto).unwrap_or_else(|_| "{}".into());
        Ok::<_, Infallible>(Event::default().event("install").data(payload))
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

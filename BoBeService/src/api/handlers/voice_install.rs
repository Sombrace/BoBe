use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;

use crate::app_state::AppState;
use crate::voice::install_service::{
    InstallStatus, ModelProgress, VoiceInstallSnapshot, VoiceModelKind,
};

#[derive(Serialize)]
pub(crate) struct InstallStatusResponse {
    pub status: InstallStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub models: Vec<ModelProgress>,
    pub installed: PresenceSnapshot,
}

#[derive(Serialize)]
pub(crate) struct PresenceSnapshot {
    pub tts: bool,
    pub stt: bool,
}

pub(crate) async fn status(State(state): State<Arc<AppState>>) -> Json<InstallStatusResponse> {
    let snap: VoiceInstallSnapshot = state.voice.voice_install.subscribe().await.borrow().clone();
    let install = state.voice.voice_install.as_ref();
    Json(InstallStatusResponse {
        status: snap.status,
        error: snap.error,
        models: snap.models,
        installed: PresenceSnapshot {
            tts: install.is_installed(VoiceModelKind::Tts),
            stt: install.is_installed(VoiceModelKind::Stt),
        },
    })
}

pub(crate) async fn start(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    start_result(state.voice.voice_install.start().await)
}

pub(crate) async fn start_tts(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    start_result(
        state
            .voice
            .voice_install
            .start_kind(VoiceModelKind::Tts)
            .await,
    )
}

pub(crate) async fn start_stt(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    start_result(
        state
            .voice
            .voice_install
            .start_kind(VoiceModelKind::Stt)
            .await,
    )
}

fn start_result(result: Result<(), crate::error::AppError>) -> axum::response::Response {
    match result {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(crate::error::AppError::Conflict(msg)) => (StatusCode::CONFLICT, msg).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(crate) async fn cancel(State(state): State<Arc<AppState>>) -> StatusCode {
    state.voice.voice_install.cancel().await;
    StatusCode::ACCEPTED
}

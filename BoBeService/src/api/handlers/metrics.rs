use std::sync::Arc;

use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;

use crate::app_state::AppState;

pub(crate) async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let body = state.infra.metrics_handle.render();
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

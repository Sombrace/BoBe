use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::error::AppError;
use crate::models::document::{Document, DocumentKind};
use crate::models::soul::Souls;
use crate::models::user_profile::UserProfiles;
use crate::services::documents::{DeleteOutcome, DocumentService};

pub(crate) trait DocumentRoutes: DocumentKind {
    fn service(state: &AppState) -> &DocumentService<Self>;
}

impl DocumentRoutes for Souls {
    fn service(state: &AppState) -> &DocumentService<Self> {
        &state.services.souls_service
    }
}

impl DocumentRoutes for UserProfiles {
    fn service(state: &AppState) -> &DocumentService<Self> {
        &state.services.user_profile_service
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct DocumentCreateRequest {
    name: String,
    content: String,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DocumentUpdateRequest {
    content: Option<String>,
    enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DocumentActionResponse<Id> {
    id: Id,
    name: String,
    enabled: bool,
    message: String,
}

const fn default_true() -> bool {
    true
}

pub(crate) async fn list<K: DocumentRoutes>(
    State(state): State<Arc<AppState>>,
) -> Result<Json<K::Listing>, AppError> {
    Ok(Json(K::service(&state).list(false).await?))
}

pub(crate) async fn create<K: DocumentRoutes>(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DocumentCreateRequest>,
) -> Result<(StatusCode, Json<Document<K>>), AppError> {
    let document = K::service(&state)
        .create(request.name, request.content, request.enabled)
        .await?;
    Ok((StatusCode::CREATED, Json(document)))
}

pub(crate) async fn update<K: DocumentRoutes>(
    State(state): State<Arc<AppState>>,
    Path(id): Path<K::Id>,
    Json(request): Json<DocumentUpdateRequest>,
) -> Result<Json<Document<K>>, AppError> {
    K::service(&state)
        .update(id, request.content, request.enabled)
        .await?
        .map(Json)
        .ok_or_else(|| not_found::<K>(id))
}

pub(crate) async fn enable<K: DocumentRoutes>(
    State(state): State<Arc<AppState>>,
    Path(id): Path<K::Id>,
) -> Result<Json<DocumentActionResponse<K::Id>>, AppError> {
    set_enabled(K::service(&state), id, true).await
}

pub(crate) async fn disable<K: DocumentRoutes>(
    State(state): State<Arc<AppState>>,
    Path(id): Path<K::Id>,
) -> Result<Json<DocumentActionResponse<K::Id>>, AppError> {
    set_enabled(K::service(&state), id, false).await
}

pub(crate) async fn delete<K: DocumentRoutes>(
    State(state): State<Arc<AppState>>,
    Path(id): Path<K::Id>,
) -> Result<StatusCode, AppError> {
    match K::service(&state).delete(id).await? {
        DeleteOutcome::Deleted => Ok(StatusCode::NO_CONTENT),
        DeleteOutcome::NotFound => Err(not_found::<K>(id)),
    }
}

async fn set_enabled<K: DocumentKind>(
    service: &DocumentService<K>,
    id: K::Id,
    enabled: bool,
) -> Result<Json<DocumentActionResponse<K::Id>>, AppError> {
    let document = service
        .set_enabled(id, enabled)
        .await?
        .ok_or_else(|| not_found::<K>(id))?;
    let change = if enabled { "enabled" } else { "disabled" };
    Ok(Json(DocumentActionResponse {
        id,
        name: document.name,
        enabled,
        message: format!("{} {change}", K::NOUN),
    }))
}

fn not_found<K: DocumentKind>(id: K::Id) -> AppError {
    AppError::NotFound(format!("{} {id} not found", K::NOUN))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDocumentRepo;
    use crate::db::test_helpers::in_memory_pool;

    fn json(value: &impl Serialize) -> String {
        serde_json::to_string(value).expect("serialize")
    }

    fn document_json<K: DocumentKind>(document: &Document<K>) -> String {
        format!(
            r#"{{"id":"{}","name":{},"content":{},"enabled":{},"is_default":{},"created_at":{},"updated_at":{}}}"#,
            document.id,
            json(&document.name),
            json(&document.content),
            document.enabled,
            document.is_default,
            json(&document.created_at),
            json(&document.updated_at),
        )
    }

    async fn keeps_its_wire_shape<K: DocumentKind>(list_field: &str, counts: &str) {
        let repo = Arc::new(SqliteDocumentRepo::<K>::new(in_memory_pool().await));
        let service = DocumentService::new(Arc::clone(&repo));
        let created = service
            .create("focus".into(), "# Focus mode".into(), true)
            .await
            .expect("create");
        assert_eq!(json(&created), document_json(&created));

        let stored = repo
            .get_by_id(created.id)
            .await
            .expect("get_by_id")
            .expect("stored document");
        assert_eq!(
            json(&service.list(false).await.expect("list")),
            format!(
                r#"{{"{list_field}":[{}],{counts}}}"#,
                document_json(&stored)
            )
        );

        let Json(action) = set_enabled(&service, created.id, false)
            .await
            .expect("disable");
        assert_eq!(
            json(&action),
            format!(
                r#"{{"id":"{}","name":"focus","enabled":false,"message":"{} disabled"}}"#,
                created.id,
                K::NOUN
            )
        );

        let missing = K::new_id();
        let error = set_enabled(&service, missing, true)
            .await
            .expect_err("missing document");
        assert_eq!(
            error.to_string(),
            format!("Not found: {} {missing} not found", K::NOUN)
        );
    }

    #[tokio::test]
    async fn souls_keep_their_wire_shape() {
        keeps_its_wire_shape::<Souls>("souls", r#""count":1"#).await;
    }

    #[tokio::test]
    async fn user_profiles_keep_their_wire_shape() {
        keeps_its_wire_shape::<UserProfiles>("profiles", r#""count":1,"enabled_count":1"#).await;
    }
}

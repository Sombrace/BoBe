use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use crate::error::AppError;

#[derive(Clone, Debug, FromRow, Serialize)]
pub(crate) struct EndpointRecord {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) label: String,
    pub(crate) room: Option<String>,
    pub(crate) certificate_sha256: Option<Vec<u8>>,
    pub(crate) protocol_min: i64,
    pub(crate) protocol_max: i64,
    pub(crate) capabilities_json: String,
    pub(crate) observed_capabilities_json: String,
    pub(crate) revoked_at: Option<DateTime<Utc>>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct EndpointUpsert {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) label: String,
    pub(crate) room: Option<String>,
    pub(crate) certificate_sha256: Option<Vec<u8>>,
    pub(crate) protocol_min: i64,
    pub(crate) protocol_max: i64,
    pub(crate) capabilities_json: String,
}

#[derive(Clone)]
pub(crate) struct EndpointRepo {
    pool: SqlitePool,
}

impl EndpointRepo {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub(crate) async fn upsert(&self, endpoint: &EndpointUpsert) -> Result<(), AppError> {
        validate(endpoint)?;
        let mut transaction = self.pool.begin().await?;
        upsert_active(&mut transaction, endpoint).await?;
        if endpoint.kind == "physical_body" {
            sqlx::query(
                "INSERT INTO endpoint_credentials
                 (id, endpoint_id, kind, certificate_sha256, generation)
                 VALUES ('body-cert-' || ?1, ?1, 'body_certificate', ?2, 1)
                 ON CONFLICT(id) DO UPDATE SET
                   certificate_sha256 = excluded.certificate_sha256,
                   revoked_at = NULL",
            )
            .bind(&endpoint.id)
            .bind(&endpoint.certificate_sha256)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn active_by_id(&self, id: &str) -> Result<Option<EndpointRecord>, AppError> {
        Ok(
            sqlx::query_as("SELECT * FROM endpoints WHERE id = ?1 AND revoked_at IS NULL")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub(crate) async fn record_authenticated_hello(
        &self,
        id: &str,
        capabilities_json: &str,
    ) -> Result<bool, AppError> {
        serde_json::from_str::<serde_json::Value>(capabilities_json)
            .map_err(|error| AppError::Config(format!("invalid endpoint capabilities: {error}")))?;
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE endpoints
             SET observed_capabilities_json = ?2,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND revoked_at IS NULL",
        )
        .bind(id)
        .bind(capabilities_json)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(result.rows_affected() == 1)
    }

    pub(crate) async fn update_observed_capabilities(
        &self,
        id: &str,
        capabilities_json: &str,
    ) -> Result<bool, AppError> {
        serde_json::from_str::<serde_json::Value>(capabilities_json)
            .map_err(|error| AppError::Config(format!("invalid endpoint capabilities: {error}")))?;
        let result = sqlx::query(
            "UPDATE endpoints SET observed_capabilities_json = ?2,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND revoked_at IS NULL",
        )
        .bind(id)
        .bind(capabilities_json)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub(crate) async fn list(&self) -> Result<Vec<EndpointRecord>, AppError> {
        Ok(
            sqlx::query_as("SELECT * FROM endpoints ORDER BY created_at, id")
                .fetch_all(&self.pool)
                .await?,
        )
    }
}

pub(crate) async fn upsert_active(
    transaction: &mut Transaction<'_, Sqlite>,
    endpoint: &EndpointUpsert,
) -> Result<(), AppError> {
    let upserted = sqlx::query(
        "INSERT INTO endpoints
         (id, kind, label, room, certificate_sha256, protocol_min, protocol_max, capabilities_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
           kind = excluded.kind,
           label = excluded.label,
           room = excluded.room,
           certificate_sha256 = excluded.certificate_sha256,
           protocol_min = excluded.protocol_min,
           protocol_max = excluded.protocol_max,
           capabilities_json = excluded.capabilities_json,
           updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE endpoints.revoked_at IS NULL",
    )
    .bind(&endpoint.id)
    .bind(&endpoint.kind)
    .bind(&endpoint.label)
    .bind(&endpoint.room)
    .bind(&endpoint.certificate_sha256)
    .bind(endpoint.protocol_min)
    .bind(endpoint.protocol_max)
    .bind(&endpoint.capabilities_json)
    .execute(&mut **transaction)
    .await?;
    if upserted.rows_affected() != 1 {
        return Err(AppError::Conflict(format!(
            "endpoint {} is revoked and cannot be reactivated",
            endpoint.id
        )));
    }
    Ok(())
}

pub(crate) fn validate(endpoint: &EndpointUpsert) -> Result<(), AppError> {
    if endpoint.id.is_empty() || endpoint.id.len() > 64 {
        return Err(AppError::Config(
            "endpoint id must contain 1-64 characters".into(),
        ));
    }
    if endpoint.id == "self" {
        return Err(AppError::Config("endpoint id self is reserved".into()));
    }
    if !matches!(
        endpoint.kind.as_str(),
        "physical_body" | "mac_app" | "other"
    ) {
        return Err(AppError::Config("unsupported endpoint kind".into()));
    }
    if endpoint.label.trim().is_empty() || endpoint.label.len() > 128 {
        return Err(AppError::Config(
            "endpoint label must contain 1-128 characters".into(),
        ));
    }
    if endpoint.protocol_min < 1 || endpoint.protocol_max < endpoint.protocol_min {
        return Err(AppError::Config("invalid endpoint protocol range".into()));
    }
    serde_json::from_str::<serde_json::Value>(&endpoint.capabilities_json)
        .map_err(|error| AppError::Config(format!("invalid endpoint capabilities: {error}")))?;
    if endpoint.kind == "physical_body"
        && endpoint
            .certificate_sha256
            .as_ref()
            .is_none_or(|digest| digest.len() != 32)
    {
        return Err(AppError::Config(
            "physical body endpoint requires a SHA-256 certificate digest".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn repo() -> EndpointRepo {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        EndpointRepo::new(pool)
    }

    fn endpoint(id: &str, digest: u8) -> EndpointUpsert {
        EndpointUpsert {
            id: id.into(),
            kind: "physical_body".into(),
            label: id.into(),
            room: None,
            certificate_sha256: Some(vec![digest; 32]),
            protocol_min: 1,
            protocol_max: 1,
            capabilities_json: "{}".into(),
        }
    }

    #[tokio::test]
    async fn stores_multiple_endpoints_with_their_certificates() {
        let repo = repo().await;
        repo.upsert(&endpoint("body-a", 1)).await.unwrap();
        repo.upsert(&endpoint("body-b", 2)).await.unwrap();
        assert_eq!(repo.list().await.unwrap().len(), 2);
        assert_eq!(
            repo.active_by_id("body-b")
                .await
                .unwrap()
                .unwrap()
                .certificate_sha256,
            Some(vec![2; 32])
        );
    }

    #[tokio::test]
    async fn the_self_endpoint_id_is_reserved_for_self_unpairing() {
        let repo = repo().await;
        let mut endpoint = endpoint("self", 9);
        assert!(repo.upsert(&endpoint).await.is_err());
        endpoint.id = "body-self".into();
        assert!(repo.upsert(&endpoint).await.is_ok());
    }

    #[tokio::test]
    async fn reported_capabilities_do_not_replace_owner_grants() {
        let repo = repo().await;
        let mut enrolled = endpoint("sensor", 7);
        enrolled.capabilities_json = r#"{"sensors":true}"#.into();
        repo.upsert(&enrolled).await.unwrap();
        repo.update_observed_capabilities("sensor", r#"{"text_input":true}"#)
            .await
            .unwrap();
        let stored = repo.active_by_id("sensor").await.unwrap().unwrap();
        assert_eq!(stored.capabilities_json, r#"{"sensors":true}"#);
        assert_eq!(stored.observed_capabilities_json, r#"{"text_input":true}"#);
    }

    #[tokio::test]
    async fn duplicate_certificate_is_rejected_and_revocation_hides_identity() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let repo = EndpointRepo::new(pool.clone());
        repo.upsert(&endpoint("body-a", 1)).await.unwrap();
        assert!(repo.upsert(&endpoint("body-b", 1)).await.is_err());
        let trust = crate::db::device_trust_repo::DeviceTrustRepo::new(pool.clone());
        assert!(trust.revoke_endpoint("body-a", "test").await.unwrap());
        assert!(repo.upsert(&endpoint("body-a", 3)).await.is_err());
        assert!(repo.upsert(&endpoint("body-c", 1)).await.is_err());
        let endpoint_digest: Vec<u8> =
            sqlx::query_scalar("SELECT certificate_sha256 FROM endpoints WHERE id = 'body-a'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(endpoint_digest, vec![1; 32]);
        let credential: (Vec<u8>, Option<String>) = sqlx::query_as(
            "SELECT certificate_sha256, revoked_at FROM endpoint_credentials
             WHERE id = 'body-cert-body-a'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(credential.0, vec![1; 32]);
        assert!(credential.1.is_some());
        assert!(repo.active_by_id("body-a").await.unwrap().is_none());
    }
}

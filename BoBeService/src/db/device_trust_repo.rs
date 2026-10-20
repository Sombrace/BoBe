use chrono::{DateTime, TimeDelta, Utc};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::api::middleware::ENDPOINT_CREDENTIAL_PREFIX;
use crate::api::scope::{Scope, ScopeSet};
use crate::error::AppError;
use crate::util::text::{constant_time_eq, hex_decode, hex_encode};

const SECRET_BYTES: usize = 32;

#[derive(FromRow)]
struct BodyCredentialRow {
    id: String,
    kind: String,
    label: String,
    room: Option<String>,
    certificate_sha256: Option<Vec<u8>>,
    protocol_min: i64,
    protocol_max: i64,
    capabilities_json: String,
    observed_capabilities_json: String,
    revoked_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    trust_certificate_sha256: Vec<u8>,
}

impl BodyCredentialRow {
    fn endpoint(self) -> crate::db::endpoint_repo::EndpointRecord {
        crate::db::endpoint_repo::EndpointRecord {
            id: self.id,
            kind: self.kind,
            label: self.label,
            room: self.room,
            certificate_sha256: self.certificate_sha256,
            protocol_min: self.protocol_min,
            protocol_max: self.protocol_max,
            capabilities_json: self.capabilities_json,
            observed_capabilities_json: self.observed_capabilities_json,
            revoked_at: self.revoked_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(FromRow)]
struct BearerAuthenticationRow {
    id: String,
    endpoint_id: String,
    verifier: Option<Vec<u8>>,
    issued_authority_epoch: Option<i64>,
    last_used_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub(crate) struct IssuedBearerCredential {
    pub(crate) endpoint_id: String,
    pub(crate) token: String,
    pub(crate) superseded_credential_ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthenticatedCredential {
    pub(crate) endpoint_id: String,
    pub(crate) credential_id: String,
    pub(crate) issued_authority_epoch: i64,
    pub(crate) scopes: ScopeSet,
}

#[derive(Clone)]
pub(crate) struct DeviceTrustRepo {
    pool: SqlitePool,
}

impl DeviceTrustRepo {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub(crate) async fn enroll_endpoint_with_bearer(
        &self,
        endpoint: &crate::db::endpoint_repo::EndpointUpsert,
        scopes: &[Scope],
        authority_epoch: i64,
        granted_by: &str,
        source_summary: Option<&str>,
    ) -> Result<IssuedBearerCredential, AppError> {
        if authority_epoch < 1 {
            return Err(AppError::Config("authority epoch must be positive".into()));
        }
        crate::db::endpoint_repo::validate(endpoint)?;
        if endpoint.kind == "physical_body" {
            return Err(AppError::Config(
                "bearer enrollment does not support physical body endpoints".into(),
            ));
        }

        let credential_id = Uuid::new_v4().to_string();
        let lookup_id = Uuid::new_v4().simple().to_string();
        let mut secret = zeroize::Zeroizing::new([0_u8; SECRET_BYTES]);
        rand::fill(secret.as_mut_slice());
        let verifier = bearer_verifier(&lookup_id, secret.as_slice());
        let token = format!(
            "{ENDPOINT_CREDENTIAL_PREFIX}{lookup_id}_{}",
            hex_encode(secret.as_slice())
        );
        drop(secret);

        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let enrolled_kind: Option<String> =
            sqlx::query_scalar("SELECT kind FROM endpoints WHERE id = ?1 AND revoked_at IS NULL")
                .bind(&endpoint.id)
                .fetch_optional(&mut *transaction)
                .await?;
        if let Some(enrolled_kind) = enrolled_kind.filter(|kind| *kind != endpoint.kind) {
            return Err(AppError::Conflict(format!(
                "endpoint {} is enrolled as {enrolled_kind}, not {}",
                endpoint.id, endpoint.kind
            )));
        }
        crate::db::endpoint_repo::upsert_active(&mut transaction, endpoint).await?;

        let active_scopes: Vec<String> = sqlx::query_scalar(
            "SELECT scope FROM endpoint_scope_grants
             WHERE endpoint_id = ?1 AND revoked_at IS NULL",
        )
        .bind(&endpoint.id)
        .fetch_all(&mut *transaction)
        .await?;
        let requested = scopes
            .iter()
            .map(|scope| scope.as_str())
            .collect::<std::collections::HashSet<_>>();
        for scope in active_scopes
            .iter()
            .filter(|scope| !requested.contains(scope.as_str()))
        {
            sqlx::query(
                "UPDATE endpoint_scope_grants
                 SET revoked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE endpoint_id = ?1 AND scope = ?2 AND revoked_at IS NULL",
            )
            .bind(&endpoint.id)
            .bind(scope)
            .execute(&mut *transaction)
            .await?;
        }
        for scope in scopes
            .iter()
            .filter(|scope| !active_scopes.iter().any(|active| active == scope.as_str()))
        {
            sqlx::query("INSERT INTO endpoint_scope_grants (endpoint_id, scope) VALUES (?1, ?2)")
                .bind(&endpoint.id)
                .bind(scope.as_str())
                .execute(&mut *transaction)
                .await?;
        }

        let superseded_credential_ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM endpoint_credentials
             WHERE endpoint_id = ?1 AND kind = 'bearer' AND revoked_at IS NULL",
        )
        .bind(&endpoint.id)
        .fetch_all(&mut *transaction)
        .await?;
        let generation: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(generation), 0) + 1
             FROM endpoint_credentials WHERE endpoint_id = ?1 AND kind = 'bearer'",
        )
        .bind(&endpoint.id)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO endpoint_credentials
             (id, endpoint_id, kind, lookup_id, verifier, issued_authority_epoch, generation)
             VALUES (?1, ?2, 'bearer', ?3, ?4, ?5, ?6)",
        )
        .bind(&credential_id)
        .bind(&endpoint.id)
        .bind(&lookup_id)
        .bind(verifier.as_slice())
        .bind(authority_epoch)
        .bind(generation)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE endpoint_credentials
             SET revoked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE endpoint_id = ?1 AND kind = 'bearer' AND id <> ?2
               AND revoked_at IS NULL",
        )
        .bind(&endpoint.id)
        .bind(&credential_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        tracing::info!(
            endpoint_id = %endpoint.id,
            credential_id = %credential_id,
            generation,
            authority_epoch,
            granted_by,
            source = source_summary,
            scopes = ?scopes.iter().map(|scope| scope.as_str()).collect::<Vec<_>>(),
            "security.endpoint_bearer_enrolled"
        );

        Ok(IssuedBearerCredential {
            endpoint_id: endpoint.id.clone(),
            token,
            superseded_credential_ids,
        })
    }

    pub(crate) async fn authenticate_bearer(
        &self,
        token: &str,
        current_authority_epoch: i64,
    ) -> Result<Option<AuthenticatedCredential>, AppError> {
        let Some((lookup_id, secret)) = parse_bearer(token) else {
            return Ok(None);
        };
        let record: Option<BearerAuthenticationRow> = sqlx::query_as(
            "SELECT c.id, c.endpoint_id, c.verifier,
                    c.issued_authority_epoch, c.last_used_at
             FROM endpoint_credentials c
             JOIN endpoints e ON e.id = c.endpoint_id
             WHERE c.lookup_id = ?1 AND c.kind = 'bearer'
               AND c.revoked_at IS NULL AND e.revoked_at IS NULL",
        )
        .bind(lookup_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(record) = record else {
            return Ok(None);
        };
        if record.issued_authority_epoch != Some(current_authority_epoch) {
            return Ok(None);
        }
        let expected = record.verifier.as_deref().unwrap_or_default();
        let actual = bearer_verifier(lookup_id, &secret);
        if !constant_time_eq(expected, &actual) {
            return Ok(None);
        }
        let scopes = self.active_scopes(&record.endpoint_id).await?;
        if record
            .last_used_at
            .is_none_or(|used| used <= Utc::now() - TimeDelta::minutes(5))
        {
            sqlx::query(
                "UPDATE endpoint_credentials
                 SET last_used_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE id = ?1 AND (
                   last_used_at IS NULL OR
                   last_used_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-5 minutes')
                 )",
            )
            .bind(&record.id)
            .execute(&self.pool)
            .await?;
        }
        Ok(Some(AuthenticatedCredential {
            endpoint_id: record.endpoint_id,
            credential_id: record.id,
            issued_authority_epoch: current_authority_epoch,
            scopes,
        }))
    }

    pub(crate) async fn active_scopes(&self, endpoint_id: &str) -> Result<ScopeSet, AppError> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT scope FROM endpoint_scope_grants
             WHERE endpoint_id = ?1 AND revoked_at IS NULL ORDER BY scope",
        )
        .bind(endpoint_id)
        .fetch_all(&self.pool)
        .await?;
        let scopes = rows
            .iter()
            .map(|value| {
                Scope::parse(value)
                    .ok_or_else(|| AppError::Config(format!("unknown stored scope {value}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ScopeSet::from_iter(scopes))
    }

    pub(crate) async fn revoke_endpoint(
        &self,
        endpoint_id: &str,
        reason: &str,
    ) -> Result<bool, AppError> {
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE endpoints
             SET revoked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND revoked_at IS NULL",
        )
        .bind(endpoint_id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            transaction.rollback().await?;
            return Ok(false);
        }
        sqlx::query(
            "UPDATE endpoint_credentials
             SET revoked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE endpoint_id = ?1 AND revoked_at IS NULL",
        )
        .bind(endpoint_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE endpoint_scope_grants
             SET revoked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE endpoint_id = ?1 AND revoked_at IS NULL",
        )
        .bind(endpoint_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        tracing::info!(endpoint_id, reason, "security.endpoint_revoked");
        Ok(true)
    }

    pub(crate) async fn active_body_credentials(
        &self,
    ) -> Result<Vec<(crate::db::endpoint_repo::EndpointRecord, Vec<u8>)>, AppError> {
        let rows: Vec<BodyCredentialRow> = sqlx::query_as(
            "SELECT e.id, e.kind, e.label, e.room, e.certificate_sha256,
                    e.protocol_min, e.protocol_max, e.capabilities_json, e.observed_capabilities_json,
                    e.revoked_at, e.created_at, e.updated_at,
                    c.certificate_sha256 AS trust_certificate_sha256
             FROM endpoints e
             JOIN endpoint_credentials c ON c.endpoint_id = e.id
             WHERE e.kind = 'physical_body' AND e.revoked_at IS NULL
               AND c.kind = 'body_certificate' AND c.revoked_at IS NULL",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let digest = row.trust_certificate_sha256.clone();
                (row.endpoint(), digest)
            })
            .collect())
    }
}

fn parse_bearer(token: &str) -> Option<(&str, Vec<u8>)> {
    let suffix = token.strip_prefix(ENDPOINT_CREDENTIAL_PREFIX)?;
    let (lookup_id, encoded_secret) = suffix.split_once('_')?;
    if lookup_id.len() != 32 || !lookup_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let secret = hex_decode(encoded_secret)?;
    (secret.len() == SECRET_BYTES).then_some((lookup_id, secret))
}

fn bearer_verifier(lookup_id: &str, secret: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"bobe.endpoint.bearer.v2\0");
    hasher.update(lookup_id.as_bytes());
    hasher.update(secret);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::endpoint_repo::{EndpointRepo, EndpointUpsert};
    use std::assert_matches;

    fn mac_endpoint(label: &str) -> EndpointUpsert {
        EndpointUpsert {
            id: "mac-a".into(),
            kind: "mac_app".into(),
            label: label.into(),
            room: None,
            certificate_sha256: None,
            protocol_min: 1,
            protocol_max: 1,
            capabilities_json: "{}".into(),
        }
    }

    async fn repo() -> DeviceTrustRepo {
        DeviceTrustRepo::new(crate::db::test_helpers::in_memory_pool().await)
    }

    async fn enroll(
        repo: &DeviceTrustRepo,
        scopes: &[Scope],
        authority_epoch: i64,
    ) -> IssuedBearerCredential {
        repo.enroll_endpoint_with_bearer(
            &mac_endpoint("Mac A"),
            scopes,
            authority_epoch,
            "test",
            None,
        )
        .await
        .unwrap()
    }

    async fn active_bearer_id(repo: &DeviceTrustRepo) -> String {
        sqlx::query_scalar(
            "SELECT id FROM endpoint_credentials
             WHERE endpoint_id = 'mac-a' AND kind = 'bearer' AND revoked_at IS NULL",
        )
        .fetch_one(&repo.pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn issues_unique_bearer_and_stores_only_verifier() {
        let repo = repo().await;
        let issued = enroll(&repo, &[Scope::ConversationRead], 4).await;
        assert!(issued.token.starts_with(ENDPOINT_CREDENTIAL_PREFIX));
        let credential_id = active_bearer_id(&repo).await;
        let record: (Option<Vec<u8>>, Option<String>) =
            sqlx::query_as("SELECT verifier, lookup_id FROM endpoint_credentials WHERE id = ?1")
                .bind(credential_id)
                .fetch_one(&repo.pool)
                .await
                .unwrap();
        assert_eq!(record.0.unwrap().len(), 32);
        assert!(
            !issued
                .token
                .contains(record.1.as_deref().unwrap_or_default())
                || issued.token.starts_with(ENDPOINT_CREDENTIAL_PREFIX)
        );
        let authenticated = repo
            .authenticate_bearer(&issued.token, 4)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(authenticated.endpoint_id, "mac-a");
        assert!(authenticated.scopes.contains(Scope::ConversationRead));
        assert!(!authenticated.scopes.contains(Scope::ProviderAdmin));
    }

    #[tokio::test]
    async fn enrollment_supersedes_bearers_and_replaces_scopes() {
        let repo = repo().await;
        let first = repo
            .enroll_endpoint_with_bearer(
                &mac_endpoint("First"),
                &[Scope::ConversationRead],
                4,
                "test",
                Some("127.0.0.1"),
            )
            .await
            .unwrap();
        let first_record: (String, i64) = sqlx::query_as(
            "SELECT id, generation FROM endpoint_credentials
             WHERE endpoint_id = 'mac-a' AND kind = 'bearer' AND revoked_at IS NULL",
        )
        .fetch_one(&repo.pool)
        .await
        .unwrap();
        let second = repo
            .enroll_endpoint_with_bearer(
                &mac_endpoint("Second"),
                &[Scope::ProviderAdmin],
                4,
                "test",
                Some("127.0.0.1"),
            )
            .await
            .unwrap();
        let second_record: (String, i64) = sqlx::query_as(
            "SELECT id, generation FROM endpoint_credentials
             WHERE endpoint_id = 'mac-a' AND kind = 'bearer' AND revoked_at IS NULL",
        )
        .fetch_one(&repo.pool)
        .await
        .unwrap();
        assert_eq!(first_record.1, 1);
        assert_eq!(second_record.1, 2);
        assert_eq!(
            second.superseded_credential_ids,
            vec![first_record.0.clone()]
        );
        assert!(
            repo.authenticate_bearer(&first.token, 4)
                .await
                .unwrap()
                .is_none()
        );
        let current = repo
            .authenticate_bearer(&second.token, 4)
            .await
            .unwrap()
            .unwrap();
        assert!(current.scopes.contains(Scope::ProviderAdmin));
        assert!(!current.scopes.contains(Scope::ConversationRead));
        let superseded_at: Option<String> =
            sqlx::query_scalar("SELECT revoked_at FROM endpoint_credentials WHERE id = ?1")
                .bind(&first_record.0)
                .fetch_one(&repo.pool)
                .await
                .unwrap();
        assert!(superseded_at.is_some());
    }

    #[tokio::test]
    async fn a_phone_pairing_cannot_take_over_a_mac_endpoint() {
        let repo = repo().await;
        let mac = enroll(&repo, Scope::ALL.as_slice(), 4).await;
        let mut phone = mac_endpoint("iPhone");
        phone.kind = crate::api::scope::ScopeProfile::Phone
            .endpoint_kind()
            .into();
        let refused = repo
            .enroll_endpoint_with_bearer(
                &phone,
                crate::api::scope::ScopeProfile::Phone.scopes(),
                4,
                "pairing",
                None,
            )
            .await;
        assert_matches!(refused, Err(AppError::Conflict(_)));
        let current = repo
            .authenticate_bearer(&mac.token, 4)
            .await
            .unwrap()
            .unwrap();
        assert!(current.scopes.contains(Scope::ProviderAdmin));
        let kind: String = sqlx::query_scalar("SELECT kind FROM endpoints WHERE id = 'mac-a'")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(kind, "mac_app");
    }

    #[tokio::test]
    async fn a_paired_phone_is_stored_as_another_endpoint_with_its_profile() {
        let repo = repo().await;
        let profile = crate::api::scope::ScopeProfile::Phone;
        let issued = repo
            .enroll_endpoint_with_bearer(
                &EndpointUpsert {
                    id: "phone-a".into(),
                    kind: profile.endpoint_kind().into(),
                    label: profile.default_label().into(),
                    room: None,
                    certificate_sha256: None,
                    protocol_min: 1,
                    protocol_max: 1,
                    capabilities_json: r#"{"scope_profile":"phone.v1"}"#.into(),
                },
                profile.scopes(),
                4,
                "pairing",
                None,
            )
            .await
            .unwrap();
        let stored = EndpointRepo::new(repo.pool.clone())
            .active_by_id("phone-a")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.kind, "other");
        assert_eq!(stored.label, "iPhone");
        assert!(stored.capabilities_json.contains("phone.v1"));
        let phone = repo
            .authenticate_bearer(&issued.token, 4)
            .await
            .unwrap()
            .unwrap();
        assert!(phone.scopes.contains(Scope::ConversationWrite));
        assert!(!phone.scopes.contains(Scope::SettingsWrite));
        assert!(
            repo.revoke_endpoint("phone-a", "owner_requested")
                .await
                .unwrap()
        );
        assert!(
            repo.authenticate_bearer(&issued.token, 4)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn enrollment_rolls_back_every_change_when_the_credential_is_not_stored() {
        let repo = repo().await;
        EndpointRepo::new(repo.pool.clone())
            .upsert(&mac_endpoint("Mac A"))
            .await
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER fail_credential_issue BEFORE INSERT ON endpoint_credentials
             BEGIN SELECT RAISE(FAIL, 'credential store unavailable'); END",
        )
        .execute(&repo.pool)
        .await
        .unwrap();
        assert!(
            repo.enroll_endpoint_with_bearer(
                &mac_endpoint("Changed"),
                &[Scope::ConversationRead],
                4,
                "test",
                None,
            )
            .await
            .is_err()
        );
        let label: String = sqlx::query_scalar("SELECT label FROM endpoints WHERE id = 'mac-a'")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        let credential_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM endpoint_credentials WHERE endpoint_id = 'mac-a'",
        )
        .fetch_one(&repo.pool)
        .await
        .unwrap();
        let scope_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM endpoint_scope_grants
             WHERE endpoint_id = 'mac-a' AND revoked_at IS NULL",
        )
        .fetch_one(&repo.pool)
        .await
        .unwrap();
        assert_eq!(label, "Mac A");
        assert_eq!(credential_count, 0);
        assert_eq!(scope_count, 0);
    }

    #[tokio::test]
    async fn authentication_updates_last_used_at_at_most_once_per_window() {
        let repo = repo().await;
        let issued = enroll(&repo, &[], 2).await;
        let credential_id = active_bearer_id(&repo).await;
        repo.authenticate_bearer(&issued.token, 2)
            .await
            .unwrap()
            .unwrap();
        let first: String =
            sqlx::query_scalar("SELECT last_used_at FROM endpoint_credentials WHERE id = ?1")
                .bind(&credential_id)
                .fetch_one(&repo.pool)
                .await
                .unwrap();
        repo.authenticate_bearer(&issued.token, 2)
            .await
            .unwrap()
            .unwrap();
        let second: String =
            sqlx::query_scalar("SELECT last_used_at FROM endpoint_credentials WHERE id = ?1")
                .bind(&credential_id)
                .fetch_one(&repo.pool)
                .await
                .unwrap();
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn revocation_cascades_to_credentials_and_grants() {
        let repo = repo().await;
        drop(enroll(&repo, &[Scope::ConversationRead], 2).await);
        let credential_id = active_bearer_id(&repo).await;

        assert!(
            repo.revoke_endpoint("mac-a", "owner_requested")
                .await
                .unwrap()
        );
        assert!(
            !repo
                .revoke_endpoint("mac-a", "owner_requested")
                .await
                .unwrap()
        );
        let credential_revoked_at: Option<String> =
            sqlx::query_scalar("SELECT revoked_at FROM endpoint_credentials WHERE id = ?1")
                .bind(credential_id)
                .fetch_one(&repo.pool)
                .await
                .unwrap();
        assert!(credential_revoked_at.is_some());
        let active_grants: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM endpoint_scope_grants
             WHERE endpoint_id = 'mac-a' AND revoked_at IS NULL",
        )
        .fetch_one(&repo.pool)
        .await
        .unwrap();
        assert_eq!(active_grants, 0);
    }

    #[tokio::test]
    async fn wrong_secret_epoch_and_revocation_fail_independently() {
        let repo = repo().await;
        let issued = enroll(&repo, &[], 2).await;
        let mut wrong = issued.token.clone();
        let replacement = if wrong.ends_with('0') { '1' } else { '0' };
        wrong.pop();
        wrong.push(replacement);
        assert!(repo.authenticate_bearer(&wrong, 2).await.unwrap().is_none());
        assert!(
            repo.authenticate_bearer(&issued.token, 3)
                .await
                .unwrap()
                .is_none()
        );
        assert!(repo.revoke_endpoint("mac-a", "test").await.unwrap());
        assert!(
            repo.authenticate_bearer(&issued.token, 2)
                .await
                .unwrap()
                .is_none()
        );
    }
}

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::api::scope::ScopeProfile;
use crate::app_state::AppState;
use crate::error::AppError;

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum AdminRequest {
    Pair { kind: String, label: String },
    List {},
    Revoke { endpoint_id: String },
    Backup {},
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum AdminResponse {
    Ok { result: serde_json::Value },
    Error { message: String, code: &'static str },
}

pub(crate) async fn serve(
    state: Arc<AppState>,
    shutdown: tokio_util::sync::CancellationToken,
) -> Result<(), AppError> {
    let path = state.infra.paths.admin_socket();
    if path.exists() {
        drop(std::fs::remove_file(&path));
    }
    let listener = UnixListener::bind(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    while let Some(accepted) = shutdown.run_until_cancelled(listener.accept()).await {
        let (stream, _) = accepted?;
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(error) = handle(state, stream).await {
                tracing::warn!(%error, "admin.request_failed");
            }
        });
    }
    drop(std::fs::remove_file(path));
    Ok(())
}

async fn handle(state: Arc<AppState>, stream: UnixStream) -> Result<(), AppError> {
    let (reader, mut writer) = stream.into_split();
    let mut line = String::new();
    BufReader::new(reader).read_line(&mut line).await?;
    if line.len() > 16 * 1024 {
        return Err(AppError::Validation("admin request is too large".into()));
    }
    let request: AdminRequest = serde_json::from_str(&line)?;
    let response = match execute(&state, request).await {
        Ok(result) => AdminResponse::Ok { result },
        Err(error) => refusal(error),
    };
    writer
        .write_all(format!("{}\n", serde_json::to_string(&response)?).as_bytes())
        .await?;
    writer.shutdown().await?;
    Ok(())
}

fn refusal(error: AppError) -> AdminResponse {
    match error {
        AppError::Validation(message) => AdminResponse::Error {
            message,
            code: "invalid",
        },
        AppError::NotNetworked(message) => AdminResponse::Error {
            message,
            code: "not_networked",
        },
        error => AdminResponse::Error {
            message: error.to_string(),
            code: "failed",
        },
    }
}

async fn execute(state: &AppState, request: AdminRequest) -> Result<serde_json::Value, AppError> {
    match request {
        AdminRequest::Pair { kind, label } => {
            let profile = ScopeProfile::from_kind(&kind).ok_or_else(|| {
                AppError::Validation(format!("{kind} cannot be paired; use mac_app or phone"))
            })?;
            let server = state.config().server.clone();
            let invitation = crate::api::handlers::pairing::invitation(
                &state.auth.pairing,
                &server,
                profile,
                &label,
            )
            .await?;
            Ok(serde_json::to_value(invitation)?)
        }
        AdminRequest::List {} => Ok(serde_json::to_value(state.body.endpoints.list().await?)?),
        AdminRequest::Revoke { endpoint_id } => {
            let revoked = state
                .revoke_endpoint(&endpoint_id, "owner_requested")
                .await?;
            Ok(serde_json::json!({ "revoked": revoked, "endpoint_id": endpoint_id }))
        }
        AdminRequest::Backup {} => {
            let path = crate::api::handlers::backup::create_live_backup(state).await?;
            Ok(serde_json::json!({ "path": path }))
        }
    }
}

pub(crate) async fn try_request(
    path: &std::path::Path,
    request: serde_json::Value,
) -> Result<Option<serde_json::Value>, AppError> {
    let stream = match UnixStream::connect(path).await {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(admin_socket_unavailable(path, error)),
    };
    exchange(stream, request).await.map(Some)
}

pub(crate) async fn request(
    path: &std::path::Path,
    request: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    let stream = UnixStream::connect(path)
        .await
        .map_err(|error| admin_socket_unavailable(path, error))?;
    exchange(stream, request).await
}

fn admin_socket_unavailable(path: &std::path::Path, error: std::io::Error) -> AppError {
    AppError::ServiceUnavailable(format!(
        "the BoBe daemon admin socket is unavailable at {}: {error}",
        path.display()
    ))
}

async fn exchange(
    stream: UnixStream,
    request: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    let (reader, mut writer) = stream.into_split();
    writer
        .write_all(format!("{}\n", serde_json::to_string(&request)?).as_bytes())
        .await?;
    writer.shutdown().await?;
    let mut line = String::new();
    BufReader::new(reader).read_line(&mut line).await?;
    let response: serde_json::Value = serde_json::from_str(&line)?;
    if response.get("status").and_then(serde_json::Value::as_str) == Some("error") {
        let message = response
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("admin request failed")
            .to_owned();
        return Err(
            match response.get("code").and_then(serde_json::Value::as_str) {
                Some("invalid") => AppError::Validation(message),
                Some("not_networked") => AppError::NotNetworked(message),
                _ => AppError::ServiceUnavailable(message),
            },
        );
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| AppError::Internal("admin response has no result".into()))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests panic on fixture failures")]
mod tests {
    use super::*;

    #[test]
    fn unit_requests_reject_unknown_fields() {
        assert!(serde_json::from_str::<AdminRequest>(r#"{"command":"list"}"#).is_ok());
        assert!(serde_json::from_str::<AdminRequest>(r#"{"command":"backup"}"#).is_ok());
        assert!(
            serde_json::from_str::<AdminRequest>(r#"{"command":"list","unexpected":true}"#,)
                .is_err()
        );
        assert!(
            serde_json::from_str::<AdminRequest>(r#"{"command":"backup","unexpected":true}"#,)
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_refused_request_returns_the_reason_it_was_refused() {
        let root = std::path::Path::new("/tmp")
            .join(format!("bobe-admin-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&root).expect("create test directory");
        let socket = root.join("admin.sock");
        let listener = UnixListener::bind(&socket).expect("bind admin socket fixture");
        let daemon = tokio::spawn(async move {
            for refused in [
                AppError::NotNetworked("A phone cannot reach this BoBe".into()),
                AppError::Validation("device label must contain 1-128 characters".into()),
                AppError::Internal("database closed".into()),
            ] {
                let (stream, _) = listener.accept().await.expect("accept admin request");
                let (reader, mut writer) = stream.into_split();
                let mut line = String::new();
                BufReader::new(reader)
                    .read_line(&mut line)
                    .await
                    .expect("read admin request");
                let response = serde_json::to_string(&refusal(refused)).expect("encode refusal");
                writer
                    .write_all(format!("{response}\n").as_bytes())
                    .await
                    .expect("write refusal");
            }
        });
        let pair = serde_json::json!({ "command": "pair", "kind": "phone", "label": "iPhone" });
        assert!(matches!(
            request(&socket, pair.clone()).await,
            Err(AppError::NotNetworked(message)) if message == "A phone cannot reach this BoBe"
        ));
        assert!(matches!(
            request(&socket, pair.clone()).await,
            Err(AppError::Validation(message))
                if message == "device label must contain 1-128 characters"
        ));
        assert!(matches!(
            request(&socket, pair).await,
            Err(AppError::ServiceUnavailable(message))
                if message == "Internal error: database closed"
        ));
        daemon.await.expect("admin fixture");
        std::fs::remove_dir_all(root).expect("remove test directory");
    }

    #[tokio::test]
    async fn optional_request_treats_absent_and_stale_sockets_as_offline() {
        let root = std::path::Path::new("/tmp")
            .join(format!("bobe-admin-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&root).expect("create test directory");
        let socket = root.join("admin.sock");
        let request = serde_json::json!({ "command": "backup" });

        assert!(
            try_request(&socket, request.clone())
                .await
                .expect("absent socket result")
                .is_none()
        );
        let listener = UnixListener::bind(&socket).expect("bind stale socket fixture");
        drop(listener);
        assert!(socket.exists());
        assert!(
            try_request(&socket, request)
                .await
                .expect("stale socket result")
                .is_none()
        );

        std::fs::remove_dir_all(root).expect("remove test directory");
    }
}

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use serde::Deserialize;
use tokio::sync::{Mutex, watch};
use tracing::{info, warn};

use crate::error::AppError;

const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);
const PULL_TIMEOUT: Duration = Duration::from_hours(2);
const STARTUP_POLL_INTERVAL: Duration = Duration::from_secs(1);
const STARTUP_MAX_ATTEMPTS: u32 = 30;

#[derive(Debug, Clone, Default)]
pub(crate) struct PullProgress {
    pub(crate) status: String,
    pub(crate) completed_bytes: Option<u64>,
    pub(crate) total_bytes: Option<u64>,
    pub(crate) percent: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<InstalledModel>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InstalledModel {
    pub(crate) name: String,
}

#[derive(Debug, Deserialize)]
struct OllamaPullEvent {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    completed: Option<u64>,
    #[serde(default)]
    total: Option<u64>,
    #[serde(default)]
    error: Option<String>,
}

pub(crate) struct OllamaManager {
    http_client: Arc<reqwest::Client>,
    base_url: String,
    child: Mutex<Option<tokio::process::Child>>,
}

impl OllamaManager {
    pub(crate) fn new(http_client: Arc<reqwest::Client>, base_url: &str) -> Self {
        Self {
            http_client,
            base_url: base_url.trim_end_matches('/').to_string(),
            child: Mutex::new(None),
        }
    }

    pub(crate) async fn health_check(&self) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        match self
            .http_client
            .get(&url)
            .timeout(HEALTH_TIMEOUT)
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    pub(crate) async fn ensure_daemon_running(
        &self,
        binary_path: Option<&Path>,
        auto_start: bool,
    ) -> Result<(), AppError> {
        if self.health_check().await {
            info!("ollama_manager.already_running");
            return Ok(());
        }

        if !auto_start {
            return Err(AppError::ServiceUnavailable(
                "Ollama not running and auto_start=false".into(),
            ));
        }

        let Some(binary) = binary_path else {
            return Err(AppError::Config(
                "Ollama binary path required when auto_start=true and no daemon is running".into(),
            ));
        };

        self.spawn_daemon(binary).await
    }

    async fn spawn_daemon(&self, binary: &Path) -> Result<(), AppError> {
        info!(binary = %binary.display(), "ollama_manager.spawning");

        let child = tokio::process::Command::new(binary)
            .arg("serve")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("OLLAMA_HOST", "127.0.0.1:11434")
            .env("OLLAMA_ORIGINS", "http://127.0.0.1:*")
            .kill_on_drop(true)
            .spawn()?;

        *self.child.lock().await = Some(child);

        for attempt in 0..STARTUP_MAX_ATTEMPTS {
            tokio::time::sleep(STARTUP_POLL_INTERVAL).await;
            if self.health_check().await {
                info!(attempts = attempt + 1, "ollama_manager.started");
                return Ok(());
            }
        }

        if let Some(mut child) = self.child.lock().await.take()
            && let Err(e) = child.start_kill()
        {
            warn!(err = %e, "ollama_manager.kill_after_timeout_failed");
        }

        Err(AppError::ServiceUnavailable(format!(
            "Ollama failed to become healthy within {STARTUP_MAX_ATTEMPTS}s"
        )))
    }

    pub(crate) async fn pull_model(
        &self,
        name: &str,
        progress_tx: &watch::Sender<PullProgress>,
        mut cancel_rx: watch::Receiver<bool>,
    ) -> Result<(), AppError> {
        let url = format!("{}/api/pull", self.base_url);
        info!(model = name, "ollama_manager.pull_starting");

        let request = self
            .http_client
            .post(&url)
            .json(&serde_json::json!({"name": name}))
            .timeout(PULL_TIMEOUT)
            .send();
        tokio::pin!(request);
        let resp = tokio::select! {
            biased;
            () = wait_for_cancel(&mut cancel_rx) => {
                info!(model = name, "ollama_manager.pull_canceled");
                return Err(AppError::Canceled("Model pull canceled".into()));
            }
            result = &mut request => result
                .map_err(|e| AppError::ServiceUnavailable(format!("ollama pull: {e}")))?,
        };

        if !resp.status().is_success() {
            return Err(AppError::ServiceUnavailable(format!(
                "ollama pull returned {}",
                resp.status()
            )));
        }

        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let throttle = Duration::from_millis(250);
        let mut last_emit = Instant::now();
        let mut last_status: Option<String> = None;
        let mut last_percent: Option<u8> = None;

        loop {
            let chunk = tokio::select! {
                biased;
                () = wait_for_cancel(&mut cancel_rx) => {
                    info!(model = name, "ollama_manager.pull_canceled");
                    return Err(AppError::Canceled("Model pull canceled".into()));
                }
                chunk = stream.next() => chunk,
            };
            let Some(chunk) = chunk else { break };

            let bytes =
                chunk.map_err(|e| AppError::ServiceUnavailable(format!("pull stream: {e}")))?;
            buf.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(nl) = buf.find('\n') {
                let line = buf[..nl].trim().to_string();
                buf.drain(..=nl);
                if line.is_empty() {
                    continue;
                }
                let Ok(event) = serde_json::from_str::<OllamaPullEvent>(&line) else {
                    continue;
                };

                if let Some(err) = event.error {
                    return Err(AppError::ServiceUnavailable(format!("ollama pull: {err}")));
                }

                let status = event.status.clone().unwrap_or_default();
                let percent = match (event.completed, event.total) {
                    (Some(c), Some(t)) if t > 0 => {
                        Some(((c as f64 / t as f64) * 100.0).min(100.0) as u8)
                    }
                    _ => None,
                };

                if status == "success" {
                    progress_tx
                        .send(PullProgress {
                            status: "success".into(),
                            completed_bytes: event.completed,
                            total_bytes: event.total,
                            percent: Some(100),
                        })
                        .ok();
                    info!(model = name, "ollama_manager.pull_complete");
                    return Ok(());
                }

                let status_changed = last_status.as_deref() != Some(status.as_str());
                if status_changed || percent != last_percent || last_emit.elapsed() >= throttle {
                    progress_tx
                        .send(PullProgress {
                            status: status.clone(),
                            completed_bytes: event.completed,
                            total_bytes: event.total,
                            percent,
                        })
                        .ok();
                    last_emit = Instant::now();
                    last_status = Some(status);
                    last_percent = percent;
                }
            }
        }

        Err(AppError::ServiceUnavailable(
            "Ollama pull ended without success confirmation".into(),
        ))
    }

    pub(crate) async fn list_installed_models(&self) -> Result<Vec<String>, AppError> {
        let models =
            fetch_installed_models(&self.http_client, &self.base_url, HEALTH_TIMEOUT).await?;
        Ok(models.into_iter().map(|model| model.name).collect())
    }

    pub(crate) fn root_from_provider_url(provider_url: &str) -> String {
        provider_url
            .trim_end_matches('/')
            .trim_end_matches("/v1")
            .to_string()
    }
}

pub(crate) async fn fetch_installed_models(
    client: &reqwest::Client,
    provider_url: &str,
    timeout: Duration,
) -> Result<Vec<InstalledModel>, AppError> {
    let root = OllamaManager::root_from_provider_url(provider_url);
    let response = client
        .get(format!("{root}/api/tags"))
        .timeout(timeout)
        .send()
        .await
        .map_err(|error| AppError::ServiceUnavailable(format!("ollama tags: {error}")))?;

    if !response.status().is_success() {
        return Err(AppError::ServiceUnavailable(format!(
            "ollama tags returned {}",
            response.status()
        )));
    }

    let tags: TagsResponse = response
        .json()
        .await
        .map_err(|error| AppError::ServiceUnavailable(format!("ollama tags parse: {error}")))?;
    Ok(tags.models)
}

async fn wait_for_cancel(cancel_rx: &mut watch::Receiver<bool>) {
    if *cancel_rx.borrow() {
        return;
    }
    while cancel_rx.changed().await.is_ok() {
        if *cancel_rx.borrow() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::convert::Infallible;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::Bytes;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio::sync::{Notify, watch};

    use super::{OllamaManager, PullProgress, fetch_installed_models};

    #[tokio::test]
    async fn stalled_model_pull_cancels_without_another_chunk() {
        crate::util::tls::install_crypto_provider().expect("TLS provider should install");
        let first_chunk_sent = Arc::new(Notify::new());
        let release_stream = Arc::new(Notify::new());
        let app = Router::new().route(
            "/api/pull",
            post({
                let first_chunk_sent = Arc::clone(&first_chunk_sent);
                let release_stream = Arc::clone(&release_stream);
                move || {
                    let first_chunk_sent = Arc::clone(&first_chunk_sent);
                    let release_stream = Arc::clone(&release_stream);
                    async move {
                        let stream = async_stream::stream! {
                            yield Ok::<_, Infallible>(Bytes::from_static(
                                b"{\"status\":\"pulling\",\"completed\":1,\"total\":2}\n"
                            ));
                            first_chunk_sent.notify_one();
                            release_stream.notified().await;
                        };
                        axum::body::Body::from_stream(stream)
                    }
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let address = listener.local_addr().expect("test address should exist");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test server should run");
        });

        let manager = OllamaManager::new(
            Arc::new(reqwest::Client::new()),
            &format!("http://{address}"),
        );
        let (progress_tx, _progress_rx) = watch::channel(PullProgress::default());
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let pull = tokio::spawn(async move {
            manager
                .pull_model("test-model", &progress_tx, cancel_rx)
                .await
        });

        first_chunk_sent.notified().await;
        cancel_tx.send(true).expect("cancel receiver should exist");
        let error = tokio::time::timeout(Duration::from_secs(1), pull)
            .await
            .expect("cancellation should be prompt")
            .expect("pull task should join")
            .expect_err("pull should be canceled");

        std::assert_matches!(error, crate::error::AppError::Canceled(_));
        release_stream.notify_waiters();
        server.abort();
        drop(server.await);
    }

    #[test]
    fn root_from_provider_url_strips_v1() {
        assert_eq!(
            OllamaManager::root_from_provider_url("http://127.0.0.1:11434/v1"),
            "http://127.0.0.1:11434"
        );
        assert_eq!(
            OllamaManager::root_from_provider_url("http://127.0.0.1:11434/v1/"),
            "http://127.0.0.1:11434"
        );
        assert_eq!(
            OllamaManager::root_from_provider_url("http://127.0.0.1:11434"),
            "http://127.0.0.1:11434"
        );
    }

    #[tokio::test]
    async fn installed_model_names_ignore_catalog_only_metadata() {
        crate::util::tls::install_crypto_provider().expect("TLS provider should install");
        let app = Router::new().route(
            "/api/tags",
            get(|| async {
                Json(json!({
                    "models": [{
                        "name": "qwen-vl",
                        "details": {
                            "family": "qwen",
                            "families": ["qwen", "vision"]
                        }
                    }, {
                        "name": "untagged-model",
                        "details": {
                            "family": null,
                            "families": null
                        }
                    }]
                }))
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let address = listener.local_addr().expect("test address should exist");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test server should run");
        });

        let models = fetch_installed_models(
            &reqwest::Client::new(),
            &format!("http://{address}/v1"),
            Duration::from_secs(1),
        )
        .await
        .expect("tags response should decode");

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].name, "qwen-vl");
        assert_eq!(models[1].name, "untagged-model");
        server.abort();
        drop(server.await);
    }
}

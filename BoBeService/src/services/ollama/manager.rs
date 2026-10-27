use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use serde::Deserialize;
use tokio::sync::{Mutex, watch};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::error::AppError;
use crate::util::process_group::{group_exists, signal_group};

const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_GRACE: Duration = Duration::from_secs(5);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(100);
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
    start: Mutex<()>,
}

fn serve_command(binary: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(binary);
    command
        .arg("serve")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env("OLLAMA_HOST", "127.0.0.1:11434")
        .env("OLLAMA_ORIGINS", "http://127.0.0.1:*")
        .env(
            "OLLAMA_CONTEXT_LENGTH",
            crate::provider::WorkerClass::largest_context_tokens()
                .next_multiple_of(4096)
                .to_string(),
        )
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    command
}

impl OllamaManager {
    pub(crate) fn new(http_client: Arc<reqwest::Client>, base_url: &str) -> Self {
        Self {
            http_client,
            base_url: base_url.trim_end_matches('/').to_string(),
            child: Mutex::new(None),
            start: Mutex::new(()),
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

    pub(crate) async fn ensure_daemon_running(&self, binary: &Path) -> Result<(), AppError> {
        let _start = self.start.lock().await;
        if self.health_check().await {
            info!("ollama_manager.already_running");
            return Ok(());
        }
        self.spawn_daemon(binary).await
    }

    pub(crate) async fn restart_with(&self, binary: &Path) -> Result<(), AppError> {
        let _start = self.start.lock().await;
        if let Some(child) = self.child.lock().await.take() {
            stop_process_group(child, STOP_GRACE).await;
        }
        if self.health_check().await {
            info!("ollama_manager.already_running");
            return Ok(());
        }
        self.spawn_daemon(binary).await
    }

    async fn spawn_daemon(&self, binary: &Path) -> Result<(), AppError> {
        info!(binary = %binary.display(), "ollama_manager.spawning");

        let child = serve_command(binary).spawn()?;

        *self.child.lock().await = Some(child);

        for attempt in 0..STARTUP_MAX_ATTEMPTS {
            tokio::time::sleep(STARTUP_POLL_INTERVAL).await;
            if self.health_check().await {
                info!(attempts = attempt + 1, "ollama_manager.started");
                return Ok(());
            }
        }

        if let Some(child) = self.child.lock().await.take() {
            stop_process_group(child, STOP_GRACE).await;
        }

        Err(AppError::ServiceUnavailable(format!(
            "Ollama failed to become healthy within {STARTUP_MAX_ATTEMPTS}s"
        )))
    }

    pub(crate) async fn stop(&self) {
        if let Some(child) = self.child.lock().await.take() {
            stop_process_group(child, STOP_GRACE).await;
        }
    }

    pub(crate) async fn pull_model(
        &self,
        name: &str,
        progress_tx: &watch::Sender<PullProgress>,
        cancel: CancellationToken,
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
            () = cancel.cancelled() => {
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
                () = cancel.cancelled() => {
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

impl Drop for OllamaManager {
    fn drop(&mut self) {
        if let Some(pid) = self
            .child
            .get_mut()
            .as_ref()
            .and_then(tokio::process::Child::id)
        {
            signal_group(pid, libc::SIGKILL);
        }
    }
}

async fn stop_process_group(mut child: tokio::process::Child, grace: Duration) {
    let Some(pid) = child.id() else {
        return;
    };
    info!(pid, "ollama_manager.stopping");
    let deadline = Instant::now() + grace;
    signal_group(pid, libc::SIGTERM);
    let leader_exited = tokio::time::timeout(grace, child.wait()).await.is_ok();
    while leader_exited && group_exists(pid) && Instant::now() < deadline {
        tokio::time::sleep(STOP_POLL_INTERVAL).await;
    }
    if !leader_exited || group_exists(pid) {
        warn!(pid, "ollama_manager.stop_forced");
        signal_group(pid, libc::SIGKILL);
        if !leader_exited {
            if let Err(error) = child.start_kill() {
                warn!(%error, pid, "ollama_manager.kill_failed");
            }
            let _ignored = child.wait().await;
        }
    }
    info!(pid, "ollama_manager.stopped");
}

#[cfg(test)]
mod tests {

    use std::convert::Infallible;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::Bytes;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio::sync::{Notify, watch};

    use super::{OllamaManager, PullProgress, fetch_installed_models, serve_command};
    use crate::provider::WorkerClass;

    #[test]
    fn managed_ollama_serves_every_bobe_prompt_without_truncation() {
        let command = serve_command(std::path::Path::new("/opt/ollama"));
        let context: usize = command
            .as_std()
            .get_envs()
            .find(|(key, _)| *key == "OLLAMA_CONTEXT_LENGTH")
            .and_then(|(_, value)| value?.to_str()?.parse().ok())
            .expect("the managed runtime sets a context length");
        for class in [
            WorkerClass::Goals,
            WorkerClass::Small,
            WorkerClass::Chat,
            WorkerClass::Consolidate,
        ] {
            assert!(context >= class.budget().context_tokens());
        }
    }
    #[cfg(unix)]
    use super::{group_exists, stop_process_group};

    #[tokio::test]
    async fn stalled_model_pull_cancels_without_another_chunk() {
        crate::util::tls::install_crypto_provider();
        let first_chunk_sent = Arc::new(Notify::new());
        let release_stream = Arc::new(Notify::new());
        let app = Router::new().route(
            "/api/pull",
            post({
                let first_chunk_sent = Arc::clone(&first_chunk_sent);
                let release_stream = Arc::clone(&release_stream);
                async move || {
                    let stream = async_stream::stream! {
                        yield Ok::<_, Infallible>(Bytes::from_static(
                            b"{\"status\":\"pulling\",\"completed\":1,\"total\":2}\n"
                        ));
                        first_chunk_sent.notify_one();
                        release_stream.notified().await;
                    };
                    axum::body::Body::from_stream(stream)
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
        let cancel = tokio_util::sync::CancellationToken::new();
        let pulling = cancel.clone();
        let pull = tokio::spawn(async move {
            manager
                .pull_model("test-model", &progress_tx, pulling)
                .await
        });

        first_chunk_sent.notified().await;
        cancel.cancel();
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
        crate::util::tls::install_crypto_provider();
        let app = Router::new().route(
            "/api/tags",
            get(async || {
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

    #[cfg(unix)]
    fn fake_runtime(
        name: &str,
        ignore_terminate: bool,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("bobe-ollama-stop-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        let runner_pid = dir.join("runner.pid");
        let trap = if ignore_terminate {
            "trap '' TERM\n"
        } else {
            ""
        };
        let binary = dir.join("ollama");
        std::fs::write(
            &binary,
            format!(
                "#!/bin/sh\n{trap}sleep 600 &\necho $! > '{}'\nwait\n",
                runner_pid.display()
            ),
        )
        .expect("fake runtime should be written");
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
            .expect("fake runtime should be executable");
        (binary, runner_pid)
    }

    #[cfg(unix)]
    async fn started_fake_runtime(
        name: &str,
        ignore_terminate: bool,
    ) -> (OllamaManager, u32, tokio::task::JoinHandle<()>) {
        crate::util::tls::install_crypto_provider();
        let app = Router::new().route("/api/tags", get(async || Json(json!({"models": []}))));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let address = listener.local_addr().expect("test address should exist");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test server should run");
        });
        let (binary, runner_pid) = fake_runtime(name, ignore_terminate);
        let manager = OllamaManager::new(
            Arc::new(reqwest::Client::new()),
            &format!("http://{address}"),
        );
        manager
            .spawn_daemon(&binary)
            .await
            .expect("fake runtime should start");
        let group = manager
            .child
            .lock()
            .await
            .as_ref()
            .and_then(tokio::process::Child::id)
            .expect("fake runtime should be running");
        tokio::time::timeout(Duration::from_secs(5), async {
            while !runner_pid.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("fake runtime should start its runner");
        (manager, group, server)
    }

    #[cfg(unix)]
    async fn wait_until_group_is_gone(group: u32) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while group_exists(group) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the runtime and its runner should be gone");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_ends_the_runtime_and_the_runner_it_started() {
        let (manager, group, server) = started_fake_runtime("graceful", false).await;
        assert!(group_exists(group));

        manager.stop().await;

        wait_until_group_is_gone(group).await;
        assert!(manager.child.lock().await.is_none());
        server.abort();
        drop(server.await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_kills_a_runtime_that_ignores_terminate() {
        let (manager, group, server) = started_fake_runtime("forced", true).await;
        let child = manager
            .child
            .lock()
            .await
            .take()
            .expect("fake runtime should be running");

        tokio::time::timeout(
            Duration::from_secs(5),
            stop_process_group(child, Duration::from_millis(300)),
        )
        .await
        .expect("a forced stop should be prompt");

        wait_until_group_is_gone(group).await;
        server.abort();
        drop(server.await);
    }
}

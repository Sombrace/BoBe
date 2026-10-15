use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex as SyncMutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, Notify, broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::process::{OwnedProcesses, ProcessPurpose, ProcessRequest, signal_group};
use crate::error::AppError;
use crate::provider::{ChatDelta, DomainTools, McpRuntimeConfig, SkillCreateParams, SkillStore};

const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
type Reply = Result<Value, String>;
type PendingReplies = HashMap<i64, oneshot::Sender<Reply>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeFiles {
    pub(crate) node: PathBuf,
    pub(crate) entry: PathBuf,
}

impl RuntimeFiles {
    pub(crate) fn installed(executable: &Path) -> Option<Self> {
        let parent = executable.parent()?;
        [
            Self {
                node: parent.join("node"),
                entry: parent.join("agent/dist/main.js"),
            },
            Self {
                node: parent.join("node"),
                entry: parent.join("../Resources/agent/dist/main.js"),
            },
        ]
        .into_iter()
        .find(|files| files.node.is_file() && files.entry.is_file())
    }

    pub(crate) fn agent_dir(&self) -> Result<&Path, AppError> {
        self.entry
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| AppError::Config("pi agent entry has no runtime directory".into()))
    }

    pub(crate) fn resolve() -> Result<Self, AppError> {
        let executable = std::env::current_exe()?.canonicalize()?;
        let configured_entry = std::env::var_os("BOBE_AGENT_ENTRY").map(PathBuf::from);
        let configured_node = std::env::var_os("BOBE_NODE_PATH").map(PathBuf::from);
        let files = match (configured_node, configured_entry) {
            (Some(node), Some(entry)) if node.is_absolute() && entry.is_absolute() => {
                Self { node, entry }
            }
            (None, None) => Self::installed(&executable)
                .or_else(|| {
                    cfg!(debug_assertions)
                        .then(|| {
                            which::which("node").ok().map(|node| Self {
                                node,
                                entry: Path::new(env!("CARGO_MANIFEST_DIR"))
                                    .join("../BoBeAgent/dist/main.js"),
                            })
                        })
                        .flatten()
                })
                .ok_or_else(|| {
                    AppError::Config(
                        "pi runtime is missing; build BoBeAgent or reinstall BoBe".into(),
                    )
                })?,
            _ => {
                return Err(AppError::Config(
                    "BOBE_NODE_PATH and BOBE_AGENT_ENTRY must both be absolute paths".into(),
                ));
            }
        };
        if !files.node.is_file() || !files.entry.is_file() {
            return Err(AppError::Config(
                "pi runtime files are missing; build BoBeAgent or reinstall BoBe".into(),
            ));
        }
        Ok(Self {
            node: files.node.canonicalize()?,
            entry: files.entry.canonicalize()?,
        })
    }
}

#[derive(Clone, Serialize)]
pub(super) struct InitializeParams {
    pub(super) protocol_version: u32,
    pub(super) data_root: PathBuf,
    pub(super) workspace: PathBuf,
    pub(super) skill_directories: Vec<PathBuf>,
    pub(super) profiles: Value,
    pub(super) routes: crate::provider::ProviderRoutes,
    pub(super) tools: Vec<Value>,
    pub(super) capability_instructions: String,
    pub(super) mcp: McpRuntimeConfig,
    pub(super) enabled_tools: std::collections::BTreeSet<crate::config::AgentTool>,
    pub(super) execution: crate::config::ExecutionLimits,
    pub(super) workload_budgets:
        std::collections::BTreeMap<String, crate::provider::budget::WorkloadBudget>,
}

#[derive(Clone)]
pub(super) struct DomainBridge {
    pub(super) tools: Arc<DomainTools>,
    pub(super) skills: Arc<SkillStore>,
    pub(super) jobs: Arc<crate::db::agent_job_repo::AgentJobRepo>,
    pub(super) conversations: Arc<crate::db::SqliteConversationRepo>,
    pub(super) background_route: crate::provider::WorkloadRoute,
    pub(super) background_enabled: bool,
    pub(super) execution: crate::config::ExecutionLimits,
}

#[derive(Clone)]
struct OperationAuthority {
    cancellation: CancellationToken,
    can_start_background: bool,
    channel: crate::models::ids::ConversationChannelId,
    goal_id: Option<crate::models::ids::GoalId>,
    run_id: Option<uuid::Uuid>,
    job_id: Option<uuid::Uuid>,
}
pub(super) struct Host {
    options: Mutex<InitializeParams>,
    process: Mutex<Option<Arc<Connection>>>,
    bridge: DomainBridge,
    notifications: broadcast::Sender<(String, Value)>,
    closed: AtomicBool,
}

pub(super) struct Connection {
    #[cfg(test)]
    process_id: Option<u32>,
    outgoing: mpsc::Sender<Value>,
    pending: SyncMutex<PendingReplies>,
    streams: SyncMutex<HashMap<i64, mpsc::Sender<ChatDelta>>>,
    domain_turns: SyncMutex<HashMap<i64, OperationAuthority>>,
    run_requests: SyncMutex<HashMap<uuid::Uuid, i64>>,
    next_id: AtomicI64,
    request_order: Mutex<()>,
    alive: AtomicBool,
    initialized: AtomicBool,
    exited: AtomicBool,
    closed: Notify,
    stop: CancellationToken,
    notifications: broadcast::Sender<(String, Value)>,
    processes: Arc<OwnedProcesses>,
}

pub(super) struct Request {
    pub(super) id: i64,
    pub(super) reply: oneshot::Receiver<Reply>,
    pub(super) events: Option<mpsc::Receiver<ChatDelta>>,
}

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum Incoming {
    Request {
        id: i64,
        method: String,
        params: Value,
    },
    Response {
        id: i64,
        result: Value,
    },
    Failure {
        id: i64,
        error: RemoteError,
    },
    Notification {
        method: String,
        params: Value,
    },
}

#[derive(Deserialize)]
struct RemoteError {
    message: String,
}

#[derive(Deserialize)]
struct StreamEvent {
    request_id: i64,
    delta: ChatDelta,
}

impl Host {
    pub(super) async fn cancel_run(&self, run_id: uuid::Uuid) -> Result<bool, AppError> {
        let connection = self.process.lock().await.clone();
        let mut cancelled = false;
        if let Some(connection) = connection {
            if !connection.is_alive()
                || !connection.initialized.load(Ordering::Acquire)
                || connection.stop.is_cancelled()
            {
                connection.stop_and_wait().await?;
            } else if let Some(request) = { lock(&connection.run_requests).get(&run_id).copied() } {
                connection.abort(request).await;
                cancelled = true;
            }
        }
        self.bridge.jobs.finish_input_origin(run_id).await?;
        Ok(cancelled)
    }

    pub(super) async fn steer_run(
        &self,
        run_id: uuid::Uuid,
        text: &str,
        follow_up: bool,
    ) -> Result<(), AppError> {
        let connection = self
            .process
            .lock()
            .await
            .clone()
            .ok_or_else(|| AppError::NotFound("agent is not running".into()))?;
        let request = lock(&connection.run_requests)
            .get(&run_id)
            .copied()
            .ok_or_else(|| AppError::NotFound("run is no longer executing".into()))?;
        connection
            .call(
                "steer",
                json!({"request_id":request,"text":text,"follow_up":follow_up}),
                Duration::from_secs(15),
            )
            .await?;
        Ok(())
    }

    pub(super) fn new(options: InitializeParams, bridge: DomainBridge) -> Arc<Self> {
        let (notifications, _) = broadcast::channel(128);
        Arc::new(Self {
            options: Mutex::new(options),
            process: Mutex::new(None),
            bridge,
            notifications,
            closed: AtomicBool::new(false),
        })
    }

    pub(super) fn subscribe(&self) -> broadcast::Receiver<(String, Value)> {
        self.notifications.subscribe()
    }

    pub(super) async fn connection(&self) -> Result<Arc<Connection>, AppError> {
        let mut process = self.process.lock().await;
        if self.closed.load(Ordering::Acquire) {
            return Err(AppError::ServiceUnavailable(
                "pi runtime has stopped".into(),
            ));
        }
        if let Some(connection) = process.as_ref() {
            if connection.alive.load(Ordering::Acquire)
                && connection.initialized.load(Ordering::Acquire)
                && !connection.stop.is_cancelled()
            {
                return Ok(Arc::clone(connection));
            }
            connection.stop_and_wait().await?;
            process.take();
        }
        let files = RuntimeFiles::resolve()?;
        self.start_connection(&mut process, &files).await
    }

    async fn start_connection(
        &self,
        process: &mut Option<Arc<Connection>>,
        files: &RuntimeFiles,
    ) -> Result<Arc<Connection>, AppError> {
        let options = self.options.lock().await.clone();
        let parameters = serde_json::to_value(&options)?;
        let connection = Connection::spawn(
            files,
            &options.workspace,
            self.bridge.clone(),
            self.notifications.clone(),
        )?;
        *process = Some(Arc::clone(&connection));
        let initializing = connection.stop.clone().drop_guard();
        let initialized = connection
            .call("initialize", parameters, CONTROL_TIMEOUT)
            .await;
        match initialized {
            Ok(result)
                if result.get("protocol_version").and_then(Value::as_u64) == Some(1)
                    && result.get("harness").and_then(Value::as_str) == Some("pi") =>
            {
                connection.initialized.store(true, Ordering::Release);
                initializing.disarm();
                Ok(connection)
            }
            result => {
                connection.stop_and_wait().await?;
                process.take();
                Err(AppError::Internal(match result {
                    Ok(_) => "pi worker returned an incompatible initialization contract".into(),
                    Err(error) => error.to_string(),
                }))
            }
        }
    }

    pub(super) async fn call(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, AppError> {
        self.connection().await?.call(method, params, timeout).await
    }

    pub(super) async fn reload(&self, mcp: Option<McpRuntimeConfig>) -> Result<(), AppError> {
        let directories = {
            let mut options = self.options.lock().await;
            if let Some(config) = &mcp {
                options.mcp.clone_from(config);
            }
            options.skill_directories.clone()
        };
        let connection = self.process.lock().await.clone();
        if let Some(connection) = connection.filter(|value| value.alive.load(Ordering::Acquire)) {
            let mut params = json!({"skill_directories": directories});
            if let Some(mcp) = mcp {
                params["mcp"] = serde_json::to_value(mcp)?;
            }
            connection.call("reload", params, CONTROL_TIMEOUT).await?;
        }
        Ok(())
    }

    pub(super) async fn close(&self) -> Result<(), AppError> {
        self.closed.store(true, Ordering::Release);
        self.shutdown().await
    }

    pub(super) async fn shutdown(&self) -> Result<(), AppError> {
        let mut process = self.process.lock().await;
        let Some(connection) = process.as_ref().cloned() else {
            return Ok(());
        };
        let stopping = connection.stop.clone().drop_guard();
        for cancellation in lock(&connection.domain_turns).values() {
            cancellation.cancellation.cancel();
        }
        let result = if connection.alive.load(Ordering::Acquire) {
            connection
                .call("shutdown", json!({}), Duration::from_secs(10))
                .await
                .map(drop)
        } else {
            Ok(())
        };
        connection.stop_and_wait().await?;
        process.take();
        stopping.disarm();
        result
    }
}

impl Connection {
    pub(super) fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn crash_worker(&self) {
        if let Some(pid) = self.process_id {
            signal_group(pid, libc::SIGKILL);
        }
    }

    fn spawn(
        files: &RuntimeFiles,
        workspace: &Path,
        bridge: DomainBridge,
        notifications: broadcast::Sender<(String, Value)>,
    ) -> Result<Arc<Self>, AppError> {
        let mut command = Command::new(&files.node);
        command.env_clear();
        for name in [
            "PATH",
            "HOME",
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "TZ",
            "TMPDIR",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "NODE_EXTRA_CA_CERTS",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .arg(&files.entry)
            .current_dir(workspace)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .map_err(|error| AppError::Internal(format!("start pi worker: {error}")))?;
        let pid = child.id();
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::Internal("pi worker stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Internal("pi worker stdout unavailable".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Internal("pi worker stderr unavailable".into()))?;
        let (outgoing, mut outgoing_rx) = mpsc::channel::<Value>(128);
        let stop = CancellationToken::new();
        let processes = OwnedProcesses::new(
            outgoing.clone(),
            stop.clone(),
            bridge.execution.process_concurrency,
        );
        let connection = Arc::new(Self {
            #[cfg(test)]
            process_id: pid,
            outgoing,
            pending: SyncMutex::new(HashMap::new()),
            streams: SyncMutex::new(HashMap::new()),
            domain_turns: SyncMutex::new(HashMap::new()),
            run_requests: SyncMutex::new(HashMap::new()),
            next_id: AtomicI64::new(1),
            request_order: Mutex::new(()),
            alive: AtomicBool::new(true),
            initialized: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            closed: Notify::new(),
            stop,
            processes,
            notifications,
        });
        let writer_connection = Arc::clone(&connection);
        let writer = tokio::spawn(async move {
            while let Some(message) = outgoing_rx.recv().await {
                let bytes = match serde_json::to_vec(&message) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        writer_connection.fail(&format!("encode pi message: {error}"));
                        break;
                    }
                };
                let write = async {
                    stdin.write_all(&bytes).await?;
                    stdin.write_all(b"\n").await?;
                    stdin.flush().await
                };
                if !matches!(
                    tokio::time::timeout(Duration::from_secs(5), write).await,
                    Ok(Ok(()))
                ) {
                    writer_connection.fail("pi worker input is unavailable");
                    writer_connection.stop.cancel();
                    break;
                }
            }
        });
        let reader_connection = Arc::clone(&connection);
        let reader = tokio::spawn(async move {
            reader_connection.read(BufReader::new(stdout), bridge).await;
        });
        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            loop {
                match read_line(&mut reader, 64 * 1024).await {
                    Ok(Some(line)) => {
                        let text = String::from_utf8_lossy(&line);
                        tracing::warn!(message = %crate::util::text::truncate_str(&text, 2048), "pi.worker");
                    }
                    Ok(None) => break,
                    Err(error) => {
                        tracing::warn!(%error, "pi.stderr_read_failed");
                        break;
                    }
                }
            }
        });
        let process_connection = Arc::clone(&connection);
        tokio::spawn(async move {
            let status = tokio::select! {
                status = child.wait() => status,
                () = process_connection.stop.cancelled() => {
                    if let Some(pid) = pid {
                        signal_group(pid, libc::SIGTERM);
                    }
                    if let Ok(status) = tokio::time::timeout(Duration::from_secs(3), child.wait()).await { status } else {
                        if let Some(pid) = pid {
                            signal_group(pid, libc::SIGKILL);
                        }
                        if let Err(error) = child.start_kill() {
                            tracing::warn!(%error, "pi.worker_kill_failed");
                        }
                        child.wait().await
                    }
                }
            };
            if let Some(pid) = pid {
                signal_group(pid, libc::SIGKILL);
            }
            if let Err(error) = process_connection.processes.stop_all().await {
                tracing::error!(%error,"pi.owned_process_shutdown_failed");
            }
            process_connection.fail("pi worker exited");
            if let Err(error) = status {
                tracing::error!(%error, "pi.worker_wait_failed");
            }
            writer.abort();
            reader.abort();
            stderr_task.abort();
            let _settled = tokio::join!(writer, reader, stderr_task);
            process_connection.exited.store(true, Ordering::Release);
            process_connection.closed.notify_waiters();
        });
        Ok(connection)
    }

    pub(super) async fn begin(
        self: &Arc<Self>,
        method: &str,
        params: Value,
        stream: bool,
    ) -> Result<Request, AppError> {
        let _order = self.request_order.lock().await;
        if !self.alive.load(Ordering::Acquire) {
            return Err(AppError::ServiceUnavailable(
                "pi worker is not running".into(),
            ));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if !(1..=9_007_199_254_740_991).contains(&id) {
            return Err(AppError::Internal(
                "pi request ID space is exhausted".into(),
            ));
        }
        let run_id = match method {
            "chat" => params.get("run_id"),
            "background" => params.get("job_id"),
            "auxiliary" => params.get("job").and_then(|job| job.get("job_id")),
            _ => None,
        }
        .filter(|value| !value.is_null())
        .map(|value| serde_json::from_value::<uuid::Uuid>(value.clone()))
        .transpose()?;
        let channel = params
            .get("session_key")
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()?
            .unwrap_or_default();
        let goal_id = params
            .get("goal_id")
            .filter(|value| !value.is_null())
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()?;
        if run_id.is_some_and(|run_id| lock(&self.run_requests).contains_key(&run_id)) {
            return Err(AppError::Conflict(
                "run already has an executing worker request".into(),
            ));
        }
        if lock(&self.pending).len() >= 256 {
            return Err(AppError::ServiceUnavailable(
                "pi request capacity reached".into(),
            ));
        }
        let message = json!({"id": id, "method": method, "params": params});
        if serde_json::to_vec(&message)?.len() > MAX_LINE_BYTES {
            return Err(AppError::Validation(
                "pi request exceeds the message limit".into(),
            ));
        }
        let (sender, reply) = oneshot::channel();
        lock(&self.pending).insert(id, sender);
        if let Some(run_id) = run_id {
            lock(&self.run_requests).insert(run_id, id);
        }
        if matches!(method, "chat" | "background") {
            lock(&self.domain_turns).insert(
                id,
                OperationAuthority {
                    cancellation: CancellationToken::new(),
                    can_start_background: method == "chat",
                    channel,
                    goal_id,
                    run_id,
                    job_id: if method == "background" { run_id } else { None },
                },
            );
        }
        let events = stream.then(|| {
            let (sender, receiver) = mpsc::channel(256);
            lock(&self.streams).insert(id, sender);
            receiver
        });
        if !matches!(
            tokio::time::timeout(Duration::from_secs(5), self.outgoing.send(message)).await,
            Ok(Ok(()))
        ) {
            self.finish(id);
            return Err(AppError::ServiceUnavailable(
                "pi request could not be sent".into(),
            ));
        }
        Ok(Request { id, reply, events })
    }

    pub(super) async fn call(
        self: &Arc<Self>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, AppError> {
        let request = self.begin(method, params, false).await?;
        let result = tokio::time::timeout(timeout, request.reply).await;
        self.finish(request.id);
        match result {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(error))) => Err(AppError::Internal(error)),
            Ok(Err(_)) => Err(AppError::ServiceUnavailable(
                "pi worker response was lost".into(),
            )),
            Err(_) => Err(AppError::Internal(format!(
                "pi {method} exceeded its deadline"
            ))),
        }
    }

    pub(super) async fn abort(self: &Arc<Self>, request_id: i64) {
        if let Some(cancellation) = lock(&self.domain_turns).get(&request_id) {
            cancellation.cancellation.cancel();
        }
        if let Err(error) = self
            .call(
                "abort",
                json!({"request_id": request_id}),
                Duration::from_secs(5),
            )
            .await
        {
            tracing::warn!(%error, request_id, "pi.abort_failed_stopping_worker");
            if let Err(error) = self.stop_and_wait().await {
                tracing::error!(%error, "pi.abort_shutdown_failed");
            }
        }
    }

    pub(super) fn finish(&self, id: i64) {
        lock(&self.pending).remove(&id);
        lock(&self.streams).remove(&id);
        lock(&self.domain_turns).remove(&id);
        lock(&self.run_requests).retain(|_, request| *request != id);
    }

    async fn read(self: &Arc<Self>, mut reader: impl AsyncBufRead + Unpin, bridge: DomainBridge) {
        let mut callbacks = tokio::task::JoinSet::new();
        let mut last_callback = 0_i64;
        loop {
            let bytes = match read_line(&mut reader, MAX_LINE_BYTES).await {
                Ok(Some(bytes)) => bytes,
                Ok(None) => break,
                Err(error) => {
                    self.fail(&format!("pi protocol read failed: {error}"));
                    break;
                }
            };
            let message = match serde_json::from_slice::<Incoming>(&bytes) {
                Ok(message) => message,
                Err(error) => {
                    self.fail(&format!("invalid pi protocol message: {error}"));
                    self.stop.cancel();
                    break;
                }
            };
            match message {
                Incoming::Response { id, result } => self.resolve(id, Ok(result)),
                Incoming::Failure { id, error } => self.resolve(id, Err(error.message)),
                Incoming::Request { id, method, params } => {
                    if id >= last_callback || callbacks.len() >= 256 {
                        self.fail("pi callback ordering or capacity limit was violated");
                        self.stop.cancel();
                        break;
                    }
                    last_callback = id;
                    let connection = Arc::clone(self);
                    let bridge = bridge.clone();
                    callbacks.spawn(async move {
                        let result = if id >= 0 {
                            Err(AppError::Validation(
                                "pi callback IDs must be negative".into(),
                            ))
                        } else {
                            bridge.invoke(&connection, &method, params).await
                        };
                        let message = match result {
                            Ok(result) => json!({"id": id, "result": result}),
                            Err(error) => {
                                json!({"id": id, "error": {"message": error.to_string()}})
                            }
                        };
                        if connection.outgoing.send(message).await.is_err() {
                            tracing::warn!(id, "pi.callback_response_lost");
                        }
                    });
                }
                Incoming::Notification { method, params } if method == "event" => {
                    match serde_json::from_value::<StreamEvent>(params) {
                        Ok(event) => {
                            let sender = lock(&self.streams).get(&event.request_id).cloned();
                            if let Some(sender) = sender
                                && sender.try_send(event.delta).is_err()
                            {
                                self.resolve(
                                    event.request_id,
                                    Err("pi output consumer fell behind".into()),
                                );
                                let connection = Arc::clone(self);
                                callbacks.spawn(async move {
                                    connection.abort(event.request_id).await;
                                });
                            }
                        }
                        Err(error) => {
                            self.fail(&format!("invalid pi event: {error}"));
                            self.stop.cancel();
                            break;
                        }
                    }
                }
                Incoming::Notification { method, params } => {
                    if matches!(method.as_str(), "auth_event" | "tool_activity") {
                        let _receivers = self.notifications.send((method, params));
                    } else {
                        tracing::warn!(method, "pi.unknown_notification");
                    }
                }
            }
            while let Some(result) = callbacks.try_join_next() {
                if let Err(error) = result {
                    tracing::error!(%error, "pi.callback_failed");
                }
            }
        }
        callbacks.abort_all();
        while let Some(result) = callbacks.join_next().await {
            if let Err(error) = result
                && !error.is_cancelled()
            {
                tracing::error!(%error, "pi.callback_shutdown_failed");
            }
        }
        self.fail("pi worker output closed");
        self.stop.cancel();
    }

    fn resolve(&self, id: i64, value: Reply) {
        if let Some(sender) = lock(&self.pending).remove(&id) {
            let _receiver = sender.send(value);
        } else if id < 1 || id >= self.next_id.load(Ordering::Acquire) {
            self.fail("pi worker returned an unknown response ID");
            self.stop.cancel();
        } else {
            tracing::debug!(id, "pi.response_after_cancellation");
        }
    }

    fn fail(&self, reason: &str) {
        if self.alive.swap(false, Ordering::AcqRel) {
            for (_, sender) in lock(&self.pending).drain() {
                let _receiver = sender.send(Err(reason.to_owned()));
            }
            lock(&self.streams).clear();
            for cancellation in lock(&self.domain_turns).values() {
                cancellation.cancellation.cancel();
            }
        }
    }

    pub(super) async fn stop_and_wait(&self) -> Result<(), AppError> {
        self.stop.cancel();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let notification = self.closed.notified();
                if self.exited.load(Ordering::Acquire) {
                    break;
                }
                notification.await;
            }
        })
        .await
        .map_err(|_| AppError::Internal("pi worker did not stop".into()))
    }
}

impl DomainBridge {
    async fn invoke(
        &self,
        connection: &Arc<Connection>,
        method: &str,
        params: Value,
    ) -> Result<Value, AppError> {
        if method == "process" {
            let request: ProcessRequest = serde_json::from_value(params)?;
            let run = if let ProcessRequest::Start {
                purpose: ProcessPurpose::Shell,
                request_id,
                ..
            } = &request
            {
                let cancellation = request_id
                    .and_then(|id| {
                        lock(&connection.domain_turns)
                            .get(&id)
                            .map(|run| run.cancellation.clone())
                    })
                    .ok_or_else(|| {
                        AppError::Conflict("shell process has no active owner execution".into())
                    })?;
                if cancellation.is_cancelled() {
                    return Err(AppError::Conflict("owner execution was cancelled".into()));
                }
                Some(cancellation)
            } else {
                None
            };
            return connection.processes.invoke(request, run).await;
        }
        if !matches!(
            method,
            "domain_tool" | "background_job" | "schedule" | "owner_input"
        ) {
            return Err(AppError::Validation(format!(
                "unknown pi callback '{method}'"
            )));
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Invocation {
            request_id: i64,
            name: String,
            arguments: Value,
            #[serde(default)]
            source_key: Option<String>,
        }
        let invocation: Invocation = serde_json::from_value(params)?;
        let authority = lock(&connection.domain_turns)
            .get(&invocation.request_id)
            .cloned()
            .ok_or_else(|| AppError::Conflict("domain tool has no admitted chat turn".into()))?;
        if method == "owner_input" {
            if !self.background_enabled || authority.cancellation.is_cancelled() {
                return Err(AppError::Conflict(
                    "durable owner input is unavailable to this execution".into(),
                ));
            }
            let run_id = authority
                .run_id
                .ok_or_else(|| AppError::Conflict("owner input needs a correlated run".into()))?;
            let result = match invocation.name.as_str() {
                "ask" => {
                    let source = invocation
                        .source_key
                        .filter(|value| {
                            value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                        })
                        .ok_or_else(|| {
                            AppError::Validation(
                                "owner input requires a stable source identity".into(),
                            )
                        })?;
                    self.jobs
                        .ask_owner(
                            &source,
                            crate::db::owner_input_repo::InputOrigin {
                                run_id,
                                job_id: authority.job_id,
                                channel: authority.channel,
                                goal_id: authority.goal_id,
                            },
                            serde_json::from_value(invocation.arguments)?,
                        )
                        .await?
                }
                "list" => {
                    let inputs = self
                        .jobs
                        .owner_inputs(authority.channel)
                        .await?
                        .into_iter()
                        .map(crate::db::owner_input_repo::OwnerInput::view)
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(serde_json::to_value(inputs)?);
                }
                action => {
                    #[derive(Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct Identity {
                        request_id: uuid::Uuid,
                    }
                    let request: Identity = serde_json::from_value(invocation.arguments)?;
                    match action {
                        "get" => {
                            let input = self.jobs.owner_input(request.request_id).await?;
                            if !input.visible_in(authority.channel) {
                                return Err(AppError::NotFound(
                                    "owner question is not in this channel".into(),
                                ));
                            }
                            input
                        }
                        "present" if authority.job_id.is_none() => {
                            self.jobs
                                .offer_owner_input(request.request_id, run_id, authority.channel)
                                .await?
                        }
                        "cancel" => {
                            self.jobs
                                .cancel_owner_input(request.request_id, authority.channel)
                                .await?
                        }
                        _ => return Err(AppError::Validation("unknown owner input action".into())),
                    }
                }
            };
            return Ok(serde_json::to_value(result.view()?)?);
        }
        if method == "background_job" {
            if !self.background_enabled
                || (!authority.can_start_background && invocation.name == "start")
                || authority.cancellation.is_cancelled()
            {
                return Err(AppError::Conflict(
                    "background job control is not available to this execution".into(),
                ));
            }
            return self
                .invoke_job(
                    &invocation.name,
                    invocation.arguments,
                    invocation.source_key,
                    authority.channel,
                )
                .await;
        }
        if method == "schedule" {
            if !self.background_enabled || authority.cancellation.is_cancelled() {
                return Err(AppError::Conflict(
                    "scheduling is not available to this execution".into(),
                ));
            }
            return self
                .invoke_schedule(
                    &invocation.name,
                    invocation.arguments,
                    invocation.source_key,
                    &authority,
                )
                .await;
        }
        tokio::select! {
            biased;
            () = authority.cancellation.cancelled() => Err(AppError::Conflict("domain tool was canceled with its owner execution".into())),
            result = self.invoke_tool(connection, &invocation.name, invocation.arguments) => result,
        }
    }

    async fn invoke_schedule(
        &self,
        name: &str,
        arguments: Value,
        source: Option<String>,
        authority: &OperationAuthority,
    ) -> Result<Value, AppError> {
        if name == "create" {
            let source = source
                .filter(|value| {
                    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                .ok_or_else(|| {
                    AppError::Validation("schedule requires a stable source identity".into())
                })?;
            let mut input: crate::db::agent_schedule_repo::ScheduleInput =
                serde_json::from_value(arguments)?;
            if authority.goal_id.is_some()
                && input.goal_id.is_some()
                && input.goal_id != authority.goal_id
            {
                return Err(AppError::Validation(
                    "goal work cannot reassign its scheduled continuation to another goal".into(),
                ));
            }
            input.goal_id = input.goal_id.or(authority.goal_id);
            self.tools.require_goal(input.goal_id).await?;
            return Ok(serde_json::to_value(
                self.jobs
                    .create_schedule(&source, input, authority.channel)
                    .await?,
            )?);
        }
        if name == "list" {
            return Ok(serde_json::to_value(self.jobs.schedules().await?)?);
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Identity {
            schedule_id: uuid::Uuid,
            generation: Option<i64>,
            at: Option<chrono::DateTime<chrono::Utc>>,
        }
        let identity: Identity = serde_json::from_value(arguments)?;
        let id = identity.schedule_id;
        let result = match name {
            "get" => self.jobs.schedule(id).await?,
            "pause" => self.jobs.set_schedule_enabled(id, false).await?,
            "resume" => self.jobs.set_schedule_enabled(id, true).await?,
            "reschedule" => {
                self.jobs
                    .reschedule(
                        id,
                        identity.generation.ok_or_else(|| {
                            AppError::Validation(
                                "reschedule requires the current generation".into(),
                            )
                        })?,
                        identity.at.ok_or_else(|| {
                            AppError::Validation("reschedule requires a time".into())
                        })?,
                    )
                    .await?
            }
            "cancel" => {
                self.jobs.delete_schedule(id).await?;
                return Ok(json!({"cancelled":true}));
            }
            _ => return Err(AppError::Validation("unknown schedule action".into())),
        };
        Ok(serde_json::to_value(result)?)
    }

    async fn invoke_job(
        &self,
        name: &str,
        arguments: Value,
        source_key: Option<String>,
        channel: crate::models::ids::ConversationChannelId,
    ) -> Result<Value, AppError> {
        match name {
            "start" => {
                let key = source_key
                    .filter(|value| {
                        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
                    .ok_or_else(|| {
                        AppError::Validation(
                            "background job requires a stable source identity".into(),
                        )
                    })?;
                let job = self
                    .jobs
                    .submit(
                        &key,
                        crate::db::agent_job_repo::JobInput {
                            channel_id: Some(channel),
                            ..serde_json::from_value(arguments)?
                        },
                        &self.background_route,
                    )
                    .await?;
                Ok(serde_json::to_value(job)?)
            }
            "list" => Ok(serde_json::to_value(self.jobs.list().await?)?),
            "get" | "cancel" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Identity {
                    job_id: uuid::Uuid,
                }
                let id: Identity = serde_json::from_value(arguments)?;
                let job = if name == "cancel" {
                    self.jobs.cancel(id.job_id).await?
                } else {
                    self.jobs.get(id.job_id).await?
                };
                Ok(serde_json::to_value(job)?)
            }
            _ => Err(AppError::Validation(
                "unknown background job operation".into(),
            )),
        }
    }

    async fn invoke_tool(
        &self,
        connection: &Arc<Connection>,
        name: &str,
        arguments: Value,
    ) -> Result<Value, AppError> {
        if name == crate::provider::history::HISTORY_RECALL {
            let matches = self
                .conversations
                .recall(serde_json::from_value(arguments)?)
                .await?;
            return Ok(json!({"success":true,"value":{"matches":matches}}));
        }
        let outcome = if name == crate::provider::SKILL_CREATE {
            let params: SkillCreateParams = serde_json::from_value(arguments)?;
            match self
                .skills
                .create_activated(params.name, params.description, params.body, || async {
                    connection
                        .call(
                            "reload",
                            json!({"skill_directories": self.skills.directories()}),
                            CONTROL_TIMEOUT,
                        )
                        .await
                        .map(drop)
                })
                .await
            {
                Ok(skill) => crate::provider::ToolOutcome::success(
                    json!({"ok":true,"skill":skill,"active":true}),
                ),
                Err(error) => crate::provider::ToolOutcome::failure(error),
            }
        } else {
            self.tools.invoke(name, arguments).await
        };
        Ok(serde_json::to_value(outcome)?)
    }
}

async fn read_line(
    reader: &mut (impl AsyncBufRead + Unpin),
    limit: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf().await?;
        if buffer.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "partial pi protocol line",
                ))
            };
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let length = newline.map_or(buffer.len(), |position| position + 1);
        if line.len() + length > limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "pi protocol line exceeds its limit",
            ));
        }
        line.extend_from_slice(&buffer[..length]);
        reader.consume(length);
        if newline.is_some() {
            return Ok(Some(line));
        }
    }
}

fn lock<T>(mutex: &SyncMutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        tracing::error!("pi state mutex was poisoned");
        poisoned.into_inner()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_fixture_matches_authoritative_rust_execution_limits() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../BoBeAgent/fixtures/host-contract-v1.json"
        ))
        .unwrap();
        let params = &fixture["initialize_request"]["params"];
        assert_eq!(
            params["execution"],
            serde_json::to_value(crate::config::ExecutionLimits::default()).unwrap()
        );
        for class in [
            crate::provider::WorkerClass::Chat,
            crate::provider::WorkerClass::Goals,
            crate::provider::WorkerClass::Small,
            crate::provider::WorkerClass::Consolidate,
        ] {
            assert_eq!(
                params["workload_budgets"][class.name()],
                serde_json::to_value(class.budget()).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn protocol_rejects_oversized_and_partial_lines() {
        let mut oversized = BufReader::new(&b"12345\n"[..]);
        assert!(read_line(&mut oversized, 4).await.is_err());
        let mut partial = BufReader::new(&b"{\"id\":1}"[..]);
        assert!(read_line(&mut partial, 1024).await.is_err());
        let mut valid = BufReader::new(&b"{}\n{}\n"[..]);
        assert_eq!(
            read_line(&mut valid, 1024).await.unwrap(),
            Some(b"{}\n".to_vec())
        );
        assert_eq!(
            read_line(&mut valid, 1024).await.unwrap(),
            Some(b"{}\n".to_vec())
        );
        assert!(read_line(&mut valid, 1024).await.unwrap().is_none());
    }

    #[test]
    fn protocol_distinguishes_callbacks_replies_and_events() {
        assert!(matches!(
            serde_json::from_value::<Incoming>(json!({"id":-1,"method":"domain_tool","params":{}}))
                .unwrap(),
            Incoming::Request { id: -1, .. }
        ));
        assert!(matches!(
            serde_json::from_value::<Incoming>(json!({"id":1,"result":null})).unwrap(),
            Incoming::Response {
                id: 1,
                result: Value::Null
            }
        ));
        assert!(
            serde_json::from_value::<Incoming>(
                json!({"id":1,"result":{},"error":{"message":"bad"}})
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn worker_callbacks_and_abort_do_not_block_protocol_responses() {
        let root = std::env::temp_dir().join(format!("bobe-pi-rpc-{}", uuid::Uuid::new_v4()));
        let paths = crate::util::paths::DataPaths::resolve(
            &root,
            &format!("sqlite:{}", root.join("db").display()),
        )
        .unwrap()
        .prepare_root()
        .unwrap();
        crate::provider::skills::ensure_skills(&root).await;
        let skills = SkillStore::prepare(&paths).await.unwrap();
        let memory = crate::provider::MemoryFile::new(root.join("memory.md"));
        let goals = Arc::new(crate::services::goals::goals_service::GoalsService::new(
            crate::services::goals::file_store::GoalFileStore::new(root.join("goals")),
        ));
        let tools = DomainTools::new(memory, goals, Arc::clone(&skills), true, true);
        let entry = root.join("protocol-fixture.mjs");
        tokio::fs::write(
            &entry,
            br#"
import readline from "node:readline";
import { writeFileSync } from "node:fs";
const send = value => process.stdout.write(JSON.stringify(value) + "\n");
const input = readline.createInterface({ input: process.stdin });
const callbacks = new Map();
input.on("line", line => {
  const message = JSON.parse(line);
  if (message.id < 0) {
    const id = callbacks.get(message.id);
    send({ method: "event", params: { request_id: id, delta: { type: "done" } } });
    send({ id, result: { tool_success: message.result.success } });
  } else if (message.method === "initialize") {
    if (message.params.profiles?.hold_initialization) {
      writeFileSync("initializing.pid", String(process.pid));
      return;
    }
    send({ id: message.id, result: { protocol_version: 1, harness: "pi", version: "1.2.0" } });
  } else if (message.method === "chat") {
    callbacks.set(-message.id, message.id);
    send({ method: "event", params: { request_id: message.id, delta: {
      type: "message_delta", message_id: "reply", text: "partial"
    } } });
    send({ id: -message.id, method: "domain_tool", params: { request_id: message.id, name: "bobe_memory_read", arguments: {} } });
  } else if (message.method === "abort") {
    send({ id: message.params.request_id, result: { aborted: true } });
    send({ id: message.id, result: {} });
  } else if (message.method === "shutdown") {
    process.stdout.write(JSON.stringify({ id: message.id, result: {} }) + "\n", () => process.exit(0));
  } else if (message.method !== "hold") {
    send({ id: message.id, error: { message: "unknown method" } });
  }
});
"#,
        )
        .await
        .unwrap();
        let files = RuntimeFiles {
            node: RuntimeFiles::resolve()
                .expect("pi tests require the built worker")
                .node,
            entry,
        };
        let bridge = DomainBridge {
            execution: crate::config::ExecutionLimits::default(),
            tools,
            skills,
            conversations: Arc::new(crate::db::SqliteConversationRepo::new(
                crate::db::test_helpers::in_memory_pool().await,
            )),
            jobs: crate::db::agent_job_repo::AgentJobRepo::new(
                crate::db::test_helpers::in_memory_pool().await,
            ),
            background_route: crate::config::Config::default().routes.goals,
            background_enabled: true,
        };
        let (notifications, _) = broadcast::channel(16);
        let connection = Connection::spawn(&files, &root, bridge.clone(), notifications).unwrap();
        connection
            .call("initialize", json!({}), Duration::from_secs(5))
            .await
            .unwrap();
        let mut request = connection.begin("chat", json!({}), true).await.unwrap();
        let mut events = request.events.take().unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap(),
            Some(ChatDelta::MessageDelta { text, .. }) if text == "partial"
        ));
        let reply = tokio::time::timeout(Duration::from_secs(5), request.reply)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(reply["tool_success"], true);
        connection.finish(request.id);
        let pending = connection.begin("hold", json!({}), false).await.unwrap();
        connection.abort(pending.id).await;
        let reply = tokio::time::timeout(Duration::from_secs(5), pending.reply)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(reply["aborted"], true);
        connection.finish(pending.id);
        connection
            .call("shutdown", json!({}), Duration::from_secs(5))
            .await
            .unwrap();
        connection.stop_and_wait().await.unwrap();
        let host = Host::new(
            InitializeParams {
                protocol_version: 1,
                data_root: root.clone(),
                workspace: root.clone(),
                skill_directories: Vec::new(),
                profiles: json!({"hold_initialization":true}),
                routes: crate::config::Config::default().routes,
                tools: Vec::new(),
                capability_instructions: String::new(),
                mcp: McpRuntimeConfig::default(),
                enabled_tools: std::collections::BTreeSet::default(),
                execution: crate::config::ExecutionLimits::default(),
                workload_budgets: std::collections::BTreeMap::default(),
            },
            bridge,
        );
        let starting = Arc::clone(&host);
        let cold_files = RuntimeFiles {
            node: files.node.clone(),
            entry: files.entry.clone(),
        };
        let cold = tokio::spawn(async move {
            let mut tracked = starting.process.lock().await;
            starting.start_connection(&mut tracked, &cold_files).await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while !root.join("initializing.pid").is_file() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        cold.abort();
        assert!(matches!(cold.await,Err(error) if error.is_cancelled()));
        let tracked = host
            .process
            .lock()
            .await
            .clone()
            .expect("starting worker remains owned after cancellation");
        assert!(tracked.stop.is_cancelled());
        assert!(!host.cancel_run(uuid::Uuid::new_v4()).await.unwrap());
        assert!(tracked.exited.load(Ordering::Acquire));
        assert!(!tracked.initialized.load(Ordering::Acquire));
        host.options.lock().await.profiles = json!({});
        {
            let mut owned = host.process.lock().await;
            owned.take();
            host.start_connection(&mut owned, &files).await.unwrap();
        }
        host.close().await.unwrap();
        assert!(host.connection().await.is_err());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}

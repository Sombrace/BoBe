use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::error::AppError;
use crate::util::process_group::{group_exists, signal_group};

const IO_DEADLINE: Duration = Duration::from_secs(5);
const TERMINATION_GRACE: Duration = Duration::from_secs(1);
const GROUP_EXIT_DEADLINE: Duration = Duration::from_secs(1);
const GROUP_EXIT_POLL: Duration = Duration::from_millis(10);

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ProcessRequest {
    Start {
        process_id: Uuid,
        purpose: ProcessPurpose,
        request_id: Option<i64>,
        program: String,
        args: Vec<String>,
        cwd: PathBuf,
        env: BTreeMap<String, String>,
    },
    Write {
        process_id: Uuid,
        data: String,
        #[serde(default)]
        end: bool,
    },
    Stop {
        process_id: Uuid,
    },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ProcessPurpose {
    Shell,
    Mcp,
}

struct ProcessEntry {
    input: mpsc::Sender<(Vec<u8>, bool)>,
    cancellation: CancellationToken,
    finished: watch::Receiver<bool>,
}

pub(super) struct OwnedProcesses {
    entries: Mutex<HashMap<Uuid, Arc<ProcessEntry>>>,
    capacity: Arc<Semaphore>,
    outgoing: mpsc::Sender<Value>,
    shutdown: CancellationToken,
}

struct ProcessGroup(Option<u32>);

impl ProcessGroup {
    fn kill(&mut self) {
        if let Some(pid) = self.0.take() {
            signal_group(pid, libc::SIGKILL);
        }
    }

    async fn settle(&mut self) -> std::io::Result<()> {
        let Some(pid) = self.0.take() else {
            return Ok(());
        };
        signal_group(pid, libc::SIGKILL);
        if group_exited(pid, Instant::now() + GROUP_EXIT_DEADLINE).await {
            Ok(())
        } else {
            Err(std::io::Error::other(
                "owned process group did not exit after SIGKILL",
            ))
        }
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.kill();
    }
}

impl OwnedProcesses {
    pub(super) fn new(
        outgoing: mpsc::Sender<Value>,
        shutdown: CancellationToken,
        capacity: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(HashMap::new()),
            capacity: Arc::new(Semaphore::new(capacity)),
            outgoing,
            shutdown,
        })
    }

    pub(super) async fn invoke(
        self: &Arc<Self>,
        request: ProcessRequest,
        run_cancellation: Option<CancellationToken>,
    ) -> Result<Value, AppError> {
        match request {
            ProcessRequest::Start {
                process_id,
                program,
                args,
                cwd,
                env,
                ..
            } => {
                if program.is_empty() || !cwd.is_absolute() || args.len() > 256 || env.len() > 256 {
                    return Err(AppError::Validation("invalid owned process launch".into()));
                }
                let permit = Arc::clone(&self.capacity)
                    .try_acquire_owned()
                    .map_err(|_| {
                        AppError::Conflict(
                            "owned process capacity is full; reduce concurrent work".into(),
                        )
                    })?;
                let cancellation = self.shutdown.child_token();
                let mut entries = lock(&self.entries);
                if cancellation.is_cancelled() {
                    return Err(AppError::ServiceUnavailable(
                        "worker process owner is shutting down".into(),
                    ));
                }
                if entries.contains_key(&process_id) {
                    return Err(AppError::Conflict(
                        "process identity is already active".into(),
                    ));
                }
                let mut command = Command::new(program);
                command
                    .args(args)
                    .current_dir(cwd)
                    .env_clear()
                    .envs(env)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true);
                #[cfg(unix)]
                command.process_group(0);
                let mut child = command.spawn()?;
                let pid = child
                    .id()
                    .ok_or_else(|| AppError::Internal("owned process has no pid".into()))?;
                let group = ProcessGroup(Some(pid));
                let stdin = child
                    .stdin
                    .take()
                    .ok_or_else(|| AppError::Internal("owned process stdin missing".into()))?;
                let stdout = child
                    .stdout
                    .take()
                    .ok_or_else(|| AppError::Internal("owned process stdout missing".into()))?;
                let stderr = child
                    .stderr
                    .take()
                    .ok_or_else(|| AppError::Internal("owned process stderr missing".into()))?;
                let (input, mut receiver) = mpsc::channel::<(Vec<u8>, bool)>(16);
                let (completed, finished) = watch::channel(false);
                entries.insert(
                    process_id,
                    Arc::new(ProcessEntry {
                        input,
                        cancellation: cancellation.clone(),
                        finished,
                    }),
                );
                drop(entries);
                let owner = Arc::clone(self);
                tokio::spawn(async move {
                    let _permit: OwnedSemaphorePermit = permit;
                    let mut group = group;
                    let io_cancel = cancellation.clone();
                    let writer = tokio::spawn(async move {
                        let mut stdin = stdin;
                        loop {
                            let Some(Some((bytes, end))) =
                                io_cancel.run_until_cancelled(receiver.recv()).await
                            else {
                                return;
                            };
                            let write = async {
                                stdin.write_all(&bytes).await?;
                                if end {
                                    stdin.shutdown().await?;
                                }
                                Ok::<_, std::io::Error>(())
                            };
                            if !matches!(tokio::time::timeout(IO_DEADLINE, write).await, Ok(Ok(())))
                            {
                                tracing::warn!(%process_id, "owned_process.input_failed");
                                io_cancel.cancel();
                                return;
                            }
                            if end {
                                return;
                            }
                        }
                    });
                    let mut readers = tokio::task::JoinSet::new();
                    readers.spawn(pump(
                        stdout,
                        owner.outgoing.clone(),
                        process_id,
                        "stdout",
                        cancellation.clone(),
                    ));
                    readers.spawn(pump(
                        stderr,
                        owner.outgoing.clone(),
                        process_id,
                        "stderr",
                        cancellation.clone(),
                    ));
                    let run_cancel = run_cancellation.unwrap_or_default();
                    let status = tokio::select! {
                        status = child.wait() => status,
                        () = cancellation.cancelled() => terminate(&mut child, pid).await,
                        () = run_cancel.cancelled() => terminate(&mut child, pid).await,
                    };
                    let settled = group.settle().await;
                    if let Err(error) = &settled {
                        tracing::error!(%error, %process_id, "owned_process.group_survived");
                    }
                    let status = status.and_then(|status| settled.map(|()| status));
                    writer.abort();
                    let _writer_finished = writer.await;
                    let drain = async {
                        while let Some(result) = readers.join_next().await {
                            if let Err(error) = result {
                                tracing::warn!(%error,%process_id,"owned_process.output_failed");
                            }
                        }
                    };
                    if tokio::time::timeout(IO_DEADLINE, drain).await.is_err() {
                        readers.abort_all();
                        while readers.join_next().await.is_some() {}
                    }
                    let (exit_code, error) = match status {
                        Ok(status) => {
                            #[cfg(unix)]
                            let code = {
                                use std::os::unix::process::ExitStatusExt;
                                status
                                    .code()
                                    .unwrap_or_else(|| 128 + status.signal().unwrap_or(1))
                            };
                            #[cfg(not(unix))]
                            let code = status.code().unwrap_or(1);
                            (code, None)
                        }
                        Err(error) => (1, Some(error.to_string())),
                    };
                    let event = json!({"method":"process_event","params":{
                        "process_id":process_id,"event":"exit","exit_code":exit_code,"error":error,
                    }});
                    if !owner.shutdown.is_cancelled()
                        && !matches!(
                            tokio::time::timeout(IO_DEADLINE, owner.outgoing.send(event)).await,
                            Ok(Ok(()))
                        )
                    {
                        tracing::warn!(%process_id,"owned_process.exit_delivery_failed");
                        owner.shutdown.cancel();
                    }
                    completed.send_replace(true);
                    lock(&owner.entries).remove(&process_id);
                });
                Ok(json!({"process_id":process_id}))
            }
            ProcessRequest::Write {
                process_id,
                data,
                end,
            } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|_| {
                        AppError::Validation("process input is not valid base64".into())
                    })?;
                if bytes.len() > 1024 * 1024 {
                    return Err(AppError::Validation(
                        "process input chunk is too large".into(),
                    ));
                }
                let entry = lock(&self.entries)
                    .get(&process_id)
                    .cloned()
                    .ok_or_else(|| {
                        AppError::NotFound("owned process is no longer active".into())
                    })?;
                tokio::time::timeout(IO_DEADLINE, entry.input.send((bytes, end)))
                    .await
                    .map_err(|_| AppError::ServiceUnavailable("process input stalled".into()))?
                    .map_err(|_| AppError::ServiceUnavailable("process input closed".into()))?;
                Ok(json!({}))
            }
            ProcessRequest::Stop { process_id } => {
                let entry = lock(&self.entries).get(&process_id).cloned();
                if let Some(entry) = entry {
                    entry.cancellation.cancel();
                    wait_finished(&entry).await?;
                }
                Ok(json!({}))
            }
        }
    }

    pub(super) async fn stop_all(&self) -> Result<(), AppError> {
        self.shutdown.cancel();
        let entries = lock(&self.entries).values().cloned().collect::<Vec<_>>();
        for entry in &entries {
            entry.cancellation.cancel();
        }
        let results =
            futures::future::join_all(entries.iter().map(|entry| wait_finished(entry))).await;
        for result in results {
            result?;
        }
        Ok(())
    }
}

async fn terminate(
    child: &mut tokio::process::Child,
    pid: u32,
) -> std::io::Result<std::process::ExitStatus> {
    signal_group(pid, libc::SIGTERM);
    let grace = Instant::now() + TERMINATION_GRACE;
    let status = if let Ok(status) = tokio::time::timeout_at(grace, child.wait()).await {
        status
    } else {
        signal_group(pid, libc::SIGKILL);
        child.start_kill()?;
        child.wait().await
    };
    group_exited(pid, grace).await;
    status
}

async fn group_exited(pid: u32, deadline: Instant) -> bool {
    loop {
        if !group_exists(pid) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(GROUP_EXIT_POLL).await;
    }
}

async fn wait_finished(entry: &ProcessEntry) -> Result<(), AppError> {
    let mut finished = entry.finished.clone();
    tokio::time::timeout(Duration::from_secs(8), finished.wait_for(|done| *done))
        .await
        .map_err(|_| AppError::Internal("owned process did not settle".into()))?
        .map(drop)
        .map_err(|_| AppError::Internal("process owner did not report settlement".into()))
}

async fn pump(
    mut reader: impl AsyncRead + Unpin,
    outgoing: mpsc::Sender<Value>,
    process_id: Uuid,
    stream: &'static str,
    cancellation: CancellationToken,
) {
    let mut bytes = [0; 8192];
    loop {
        let size = match reader.read(&mut bytes).await {
            Ok(0) => return,
            Ok(size) => size,
            Err(error) => {
                tracing::warn!(%error,%process_id,stream,"owned_process.read_failed");
                cancellation.cancel();
                return;
            }
        };
        let event = json!({"method":"process_event","params":{
            "process_id":process_id,"event":stream,
            "data":base64::engine::general_purpose::STANDARD.encode(&bytes[..size]),
        }});
        if !matches!(
            tokio::time::timeout(IO_DEADLINE, outgoing.send(event)).await,
            Ok(Ok(()))
        ) {
            cancellation.cancel();
            return;
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    crate::util::sync::lock(mutex, "owned process registry")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use serde_json::Value;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    use super::{OwnedProcesses, ProcessPurpose, ProcessRequest};
    use crate::util::process_group::{group_exists, signal_group};

    const TERM_IGNORING_MEMBER: &str =
        "sh -c 'trap \"\" TERM; echo $$ > member.pid; exec sleep 30' & echo $$ > leader.pid";

    fn start(root: &Path, process_id: Uuid, script: String) -> ProcessRequest {
        ProcessRequest::Start {
            process_id,
            purpose: ProcessPurpose::Shell,
            request_id: None,
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script],
            cwd: root.to_path_buf(),
            env: BTreeMap::from([("PATH".into(), std::env::var("PATH").unwrap())]),
        }
    }

    async fn workspace(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("bobe-owned-{name}-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        root
    }

    async fn read_pid(root: &Path, name: &str) -> u32 {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(text) = tokio::fs::read_to_string(root.join(name)).await
                    && let Ok(pid) = text.trim().parse()
                {
                    return pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }

    async fn exit_event(events: &mut mpsc::Receiver<Value>) -> Value {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let event = events.recv().await.unwrap();
                if event["params"]["event"] == "exit" {
                    return event;
                }
            }
        })
        .await
        .unwrap()
    }

    fn kill_survivors(leader: u32) -> bool {
        let survived = group_exists(leader);
        if survived {
            signal_group(leader, libc::SIGKILL);
        }
        survived
    }

    #[tokio::test]
    async fn stopping_a_shell_settles_group_members_that_ignore_term() {
        let root = workspace("stop").await;
        let (outgoing, mut events) = mpsc::channel(64);
        let processes = OwnedProcesses::new(outgoing, CancellationToken::new(), 4);
        let process_id = Uuid::new_v4();
        let script = format!("{TERM_IGNORING_MEMBER}; wait");
        processes
            .invoke(start(&root, process_id, script), None)
            .await
            .unwrap();
        let leader = read_pid(&root, "leader.pid").await;
        read_pid(&root, "member.pid").await;
        processes
            .invoke(ProcessRequest::Stop { process_id }, None)
            .await
            .unwrap();
        let survived = kill_survivors(leader);
        let exit = exit_event(&mut events).await;
        tokio::fs::remove_dir_all(&root).await.unwrap();
        assert!(
            !survived,
            "a stopped shell must not report settlement while its process group exists"
        );
        assert_eq!(exit["params"]["error"], Value::Null);
    }

    #[tokio::test]
    async fn exited_shell_settles_group_members_it_left_behind() {
        let root = workspace("exit").await;
        let (outgoing, mut events) = mpsc::channel(64);
        let processes = OwnedProcesses::new(outgoing, CancellationToken::new(), 4);
        let script =
            format!("{TERM_IGNORING_MEMBER}; while [ ! -s member.pid ]; do sleep 0.01; done");
        processes
            .invoke(start(&root, Uuid::new_v4(), script), None)
            .await
            .unwrap();
        let exit = exit_event(&mut events).await;
        let leader = read_pid(&root, "leader.pid").await;
        let survived = kill_survivors(leader);
        tokio::fs::remove_dir_all(&root).await.unwrap();
        assert!(
            !survived,
            "a shell's exit must not be reported while members of its process group remain"
        );
        assert_eq!(exit["params"]["exit_code"], 0);
        assert_eq!(exit["params"]["error"], Value::Null);
    }
}

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;

use super::host::Host;
use crate::error::AppError;
use crate::provider::ProfileId;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub(crate) enum LoginPhase {
    Preparing,
    AwaitingUser { url: String, code: String },
    Polling { url: String, code: String },
    AwaitingInput { message: String, password: bool },
    Completed,
    Failed { message: String },
    Canceled,
}

impl LoginPhase {
    pub(crate) const fn terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed { .. } | Self::Canceled)
    }
}

struct Login {
    sender: watch::Sender<LoginPhase>,
    receiver: watch::Receiver<LoginPhase>,
    task: JoinHandle<()>,
}

pub(crate) struct LoginCoordinator {
    host: Arc<Host>,
    sessions: Mutex<HashMap<ProfileId, Login>>,
    maintenance: Arc<AtomicBool>,
}

impl LoginCoordinator {
    pub(crate) fn new(runtime: &super::PiRuntime, maintenance: Arc<AtomicBool>) -> Arc<Self> {
        Arc::new(Self {
            host: Arc::clone(&runtime.host),
            sessions: Mutex::new(HashMap::new()),
            maintenance,
        })
    }

    pub(crate) async fn start(&self, profile: ProfileId) -> Result<(), AppError> {
        if self.maintenance.load(Ordering::Acquire) {
            return Err(AppError::Conflict("agent maintenance is active".into()));
        }
        let mut sessions = self.sessions.lock().await;
        if sessions
            .get(&profile)
            .is_some_and(|session| !session.task.is_finished())
        {
            return Err(AppError::Conflict(
                "provider login is already active".into(),
            ));
        }
        let (sender, receiver) = watch::channel(LoginPhase::Preparing);
        let task_sender = sender.clone();
        let host = Arc::clone(&self.host);
        let login_profile = profile.clone();
        let mut notifications = host.subscribe();
        let task = tokio::spawn(async move {
            let sender = task_sender;
            let params = json!({"profile":login_profile.as_str()});
            let request = host.call("auth_login", params, Duration::from_mins(15));
            tokio::pin!(request);
            loop {
                tokio::select! {
                    result = &mut request => {
                        let phase = match result {
                            Ok(_) => LoginPhase::Completed,
                            Err(error) => {
                                if let Err(cancel_error) = host.call(
                                    "auth_cancel", json!({"profile":login_profile.as_str()}),
                                    Duration::from_secs(5),
                                ).await {
                                    tracing::error!(%cancel_error, "pi.failed_login_cleanup");
                                }
                                LoginPhase::Failed { message: error.to_string() }
                            }
                        };
                        if !sender.borrow().terminal() {
                            sender.send_replace(phase);
                        }
                        return;
                    }
                    notification = notifications.recv() => {
                        match notification {
                            Ok((method, params)) if method == "auth_event"
                                && params.get("profile").and_then(serde_json::Value::as_str) == Some(login_profile.as_str()) => {
                                if let Some(Ok(phase)) = params.get("phase").cloned().map(serde_json::from_value::<LoginPhase>) { sender.send_replace(phase); } else {
                                    sender.send_replace(LoginPhase::Failed { message: "pi returned an invalid authentication event".into() });
                                    if let Err(error) = host.call("auth_cancel", json!({"profile":login_profile.as_str()}), Duration::from_secs(5)).await {
                                        tracing::error!(%error, "pi.auth_cancel_failed");
                                    }
                                    return;
                                }
                            }
                            Ok(_) => {}
                            Err(error) => {
                                sender.send_replace(LoginPhase::Failed { message: format!("provider login event stream failed: {error}") });
                                if let Err(error) = host.call("auth_cancel", json!({"profile":login_profile.as_str()}), Duration::from_secs(5)).await {
                                    tracing::error!(%error, "pi.auth_cancel_failed");
                                }
                                return;
                            }
                        }
                    }
                }
            }
        });
        sessions.insert(
            profile,
            Login {
                sender,
                receiver,
                task,
            },
        );
        Ok(())
    }

    pub(crate) async fn subscribe(
        &self,
        profile: &ProfileId,
    ) -> Option<watch::Receiver<LoginPhase>> {
        self.sessions
            .lock()
            .await
            .get(profile)
            .map(|session| session.receiver.clone())
    }

    pub(crate) async fn submit(&self, profile: &ProfileId, value: String) -> Result<(), AppError> {
        if value.len() > 16 * 1024 {
            return Err(AppError::Validation(
                "login input must not exceed 16384 bytes".into(),
            ));
        }
        self.host
            .call(
                "auth_submit",
                json!({"profile":profile.as_str(),"value":value}),
                Duration::from_secs(30),
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn cancel(&self, profile: &ProfileId) -> Result<(), AppError> {
        let login = self.sessions.lock().await.remove(profile);
        if let Some(login) = login {
            if !login.task.is_finished() {
                if let Err(error) = self
                    .host
                    .call(
                        "auth_cancel",
                        json!({"profile":profile.as_str()}),
                        Duration::from_secs(5),
                    )
                    .await
                {
                    login.sender.send_replace(LoginPhase::Failed {
                        message: error.to_string(),
                    });
                    if let Err(shutdown_error) = self.host.shutdown().await {
                        tracing::error!(%shutdown_error, "pi.login_cancellation_shutdown_failed");
                    }
                    login.task.abort();
                    let _result = login.task.await;
                    return Err(error);
                }
                login.sender.send_replace(LoginPhase::Canceled);
            }
            let mut task = login.task;
            match tokio::time::timeout(Duration::from_secs(5), &mut task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    return Err(AppError::Internal(format!(
                        "provider login task failed: {error}"
                    )));
                }
                Err(_) => {
                    task.abort();
                    let _result = task.await;
                    return Err(AppError::Internal("provider login did not stop".into()));
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn cancel_and_wait(&self) -> Result<(), AppError> {
        let profiles = self
            .sessions
            .lock()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for profile in profiles {
            self.cancel(&profile).await?;
        }
        Ok(())
    }
}

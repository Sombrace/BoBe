use std::path::{Path, PathBuf};

use crate::error::AppError;

#[cfg(target_os = "macos")]
const LAUNCHD_LABEL: &str = "com.bobe.daemon";
#[cfg(all(unix, not(target_os = "macos")))]
const SYSTEMD_UNIT: &str = "bobe.service";

pub(crate) struct ServicePaths {
    pub(crate) definition: PathBuf,
    pub(crate) logs: PathBuf,
    pub(crate) config_root: PathBuf,
}

#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn systemd_config_home(home: &Path, configured: Option<std::ffi::OsString>) -> PathBuf {
    configured
        .filter(|value| !value.is_empty() && Path::new(value).is_absolute())
        .map_or_else(|| home.join(".config"), PathBuf::from)
}

pub(crate) fn paths(
    home: &Path,
    data_root: &Path,
    config_root: &Path,
) -> Result<ServicePaths, AppError> {
    #[cfg(target_os = "macos")]
    let definition = home
        .join("Library/LaunchAgents")
        .join(format!("{LAUNCHD_LABEL}.plist"));
    #[cfg(all(unix, not(target_os = "macos")))]
    #[cfg(not(test))]
    let configured_systemd_home = std::env::var_os("XDG_CONFIG_HOME");
    #[cfg(all(unix, not(target_os = "macos"), test))]
    let configured_systemd_home = None;
    #[cfg(all(unix, not(target_os = "macos")))]
    let definition = systemd_config_home(home, configured_systemd_home)
        .join("systemd/user")
        .join(SYSTEMD_UNIT);
    #[cfg(not(unix))]
    return Err(AppError::Config("service management requires Unix".into()));
    Ok(ServicePaths {
        definition,
        logs: data_root.join("logs"),
        config_root: config_root.to_path_buf(),
    })
}

pub(crate) fn render(
    executable: &Path,
    agent: Option<&crate::agent::RuntimeFiles>,
    paths: &ServicePaths,
) -> Result<String, AppError> {
    let executable = path_text(executable)?;
    #[cfg(target_os = "macos")]
    let logs = path_text(&paths.logs)?;
    let config_root = path_text(&paths.config_root)?;
    let agent = agent
        .map(|files| Ok::<_, AppError>((path_text(&files.node)?, path_text(&files.entry)?)))
        .transpose()?;
    #[cfg(target_os = "macos")]
    {
        let agent_env = agent.map_or_else(String::new, |(node, entry)| {
            format!(
                "<key>BOBE_NODE_PATH</key><string>{}</string><key>BOBE_AGENT_ENTRY</key><string>{}</string>",
                xml(&node), xml(&entry),
            )
        });
        let log_file = format!("{logs}/bobe");
        Ok(format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LAUNCHD_LABEL}</string>
<key>ProgramArguments</key><array><string>{}</string><string>serve</string></array>
<key>EnvironmentVariables</key><dict><key>BOBE_DATA_DIR</key><string>{}</string><key>BOBE_LOGGING__FILE</key><string>{}</string>{agent_env}</dict>
<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
<key>ThrottleInterval</key><integer>10</integer>
<key>ExitTimeOut</key><integer>45</integer>
<key>StandardOutPath</key><string>/dev/null</string>
<key>StandardErrorPath</key><string>/dev/null</string>
<key>ProcessType</key><string>Background</string>
</dict></plist>
"#,
            xml(&executable),
            xml(&config_root),
            xml(&log_file)
        ))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let agent_env = agent.map_or_else(String::new, |(node, entry)| {
            format!(
                "Environment=\"BOBE_NODE_PATH={}\"\nEnvironment=\"BOBE_AGENT_ENTRY={}\"\n",
                systemd(&node),
                systemd(&entry),
            )
        });
        Ok(format!(
            "[Unit]\nDescription=BoBe personal companion daemon\nAfter=network-online.target\nWants=network-online.target\nStartLimitIntervalSec=300\nStartLimitBurst=5\n\n[Service]\nType=simple\nExecStart=\"{}\" serve\nEnvironment=\"BOBE_DATA_DIR={}\"\n{agent_env}Restart=on-failure\nRestartSec=10\nTimeoutStopSec=45\nStandardOutput=journal\nStandardError=journal\nSyslogIdentifier=bobe\n\n[Install]\nWantedBy=default.target\n",
            systemd(&executable),
            systemd(&config_root),
        ))
    }
    #[cfg(not(unix))]
    unreachable!()
}

pub(crate) struct ServiceStatus {
    pub(crate) installed: bool,
    pub(crate) running: bool,
    pub(crate) manages_data_root: bool,
}

pub(crate) async fn status(paths: &ServicePaths) -> Result<ServiceStatus, AppError> {
    let manages_data_root = definition_manages_data_root(paths).await?;
    #[cfg(target_os = "macos")]
    {
        let uid = user_id();
        ensure_launchd_gui_domain(&uid).await?;
        let target = format!("gui/{uid}/{LAUNCHD_LABEL}");
        let running = command_succeeds("/bin/launchctl", &["print", &target]).await?;
        Ok(ServiceStatus {
            installed: paths.definition.is_file(),
            running,
            manages_data_root,
        })
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let installed = paths.definition.is_file();
        let running = command_succeeds("systemctl", &["--user", "is-active", SYSTEMD_UNIT]).await?;
        Ok(ServiceStatus {
            installed,
            running,
            manages_data_root,
        })
    }
    #[cfg(not(unix))]
    unreachable!()
}

pub(crate) async fn install(
    executable: &Path,
    agent: Option<&crate::agent::RuntimeFiles>,
    paths: &ServicePaths,
) -> Result<(), AppError> {
    ensure_installable_definition(paths).await?;
    if agent.is_none() {
        return Err(AppError::Config(
            "service installation requires the complete packaged pi runtime".into(),
        ));
    }
    let definition = render(executable, agent, paths)?;
    ensure_executable(executable).await?;
    tokio::fs::create_dir_all(&paths.logs).await?;
    let parent = paths
        .definition
        .parent()
        .ok_or_else(|| AppError::Config("service definition has no parent".into()))?;
    tokio::fs::create_dir_all(parent).await?;
    #[cfg(all(unix, not(target_os = "macos")))]
    run_command("loginctl", &["enable-linger"])
        .await
        .map_err(|error| {
            AppError::Internal(format!(
                "could not enable the persistent user service: {error}; run `loginctl enable-linger` and retry"
            ))
        })?;
    replace_definition(&SystemServiceManager, paths, definition.as_bytes()).await
}

async fn ensure_executable(executable: &Path) -> Result<(), AppError> {
    let metadata = tokio::fs::metadata(executable).await.map_err(|error| {
        AppError::Config(format!(
            "service executable {} is unavailable: {error}",
            executable.display()
        ))
    })?;
    #[cfg(unix)]
    let runnable = {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let runnable = metadata.is_file();
    if !runnable {
        return Err(AppError::Config(format!(
            "service executable is not a runnable file: {}",
            executable.display()
        )));
    }
    Ok(())
}

trait ServiceManager {
    async fn check_definition(&self, definition: &Path) -> Result<(), AppError>;
    async fn running(&self) -> Result<bool, AppError>;
    async fn stop(&self) -> Result<(), AppError>;
    async fn reload(&self) -> Result<(), AppError>;
    async fn start(&self, paths: &ServicePaths) -> Result<(), AppError>;
}

struct SystemServiceManager;

impl ServiceManager for SystemServiceManager {
    async fn check_definition(&self, definition: &Path) -> Result<(), AppError> {
        check_definition(definition).await
    }

    async fn running(&self) -> Result<bool, AppError> {
        service_running().await
    }

    async fn stop(&self) -> Result<(), AppError> {
        disable_and_stop_if_loaded().await
    }

    async fn reload(&self) -> Result<(), AppError> {
        reload_definitions().await
    }

    async fn start(&self, paths: &ServicePaths) -> Result<(), AppError> {
        start(paths).await
    }
}

async fn service_running() -> Result<bool, AppError> {
    #[cfg(target_os = "macos")]
    {
        let uid = user_id();
        ensure_launchd_gui_domain(&uid).await?;
        let output = tokio::process::Command::new("/bin/launchctl")
            .args(["print", &format!("gui/{uid}/{LAUNCHD_LABEL}")])
            .output()
            .await?;
        if output.status.success() {
            Ok(launchd_reports_running(&String::from_utf8_lossy(
                &output.stdout,
            )))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if query_reports_absent(QueryKind::LaunchdJob, "", &stderr) {
                Ok(false)
            } else {
                Err(AppError::Internal(format!(
                    "launchctl query failed: {}",
                    stderr.trim()
                )))
            }
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        command_succeeds("systemctl", &["--user", "is-active", SYSTEMD_UNIT]).await
    }
    #[cfg(not(unix))]
    unreachable!()
}

#[cfg(any(test, target_os = "macos"))]
fn launchd_reports_running(description: &str) -> bool {
    description
        .lines()
        .any(|line| line.trim() == "state = running")
}

async fn check_definition(definition: &Path) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    return run_command("/usr/bin/plutil", &["-lint", &path_text(definition)?]).await;
    #[cfg(not(target_os = "macos"))]
    {
        let _ = definition;
        Ok(())
    }
}

async fn reload_definitions() -> Result<(), AppError> {
    #[cfg(all(unix, not(target_os = "macos")))]
    return run_systemctl(&["daemon-reload"]).await;
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    Ok(())
}

async fn replace_definition(
    manager: &impl ServiceManager,
    paths: &ServicePaths,
    definition: &[u8],
) -> Result<(), AppError> {
    let parent = paths
        .definition
        .parent()
        .ok_or_else(|| AppError::Config("service definition has no parent".into()))?;
    let name = paths
        .definition
        .file_name()
        .ok_or_else(|| AppError::Config("service definition has no name".into()))?
        .to_string_lossy();
    let staged = parent.join(format!(".{name}.bobe-new"));
    let prepared = async {
        crate::util::durable_fs::atomic_write(&staged, definition).await?;
        manager.check_definition(&staged).await?;
        let previous = match tokio::fs::read(&paths.definition).await {
            Ok(previous) => Some(previous),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let was_running = manager.running().await?;
        manager.stop().await?;
        Ok::<_, AppError>((previous, was_running))
    }
    .await;
    let (previous, was_running) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            drop(crate::util::durable_fs::durable_remove_file(&staged).await);
            return Err(error);
        }
    };
    let installed = async {
        tokio::fs::rename(&staged, &paths.definition).await?;
        crate::util::durable_fs::sync_directory(parent).await?;
        manager.reload().await?;
        manager.start(paths).await
    }
    .await;
    let Err(error) = installed else {
        return Ok(());
    };
    drop(crate::util::durable_fs::durable_remove_file(&staged).await);
    let restored = async {
        drop(manager.stop().await);
        match previous.as_deref() {
            Some(previous) => {
                crate::util::durable_fs::atomic_write(&paths.definition, previous).await?;
            }
            None => {
                crate::util::durable_fs::durable_remove_file(&paths.definition).await?;
            }
        }
        manager.reload().await?;
        if previous.is_some() && was_running {
            manager.start(paths).await?;
        }
        Ok::<(), AppError>(())
    }
    .await;
    Err(match restored {
        Ok(()) => error,
        Err(restore_error) => AppError::Internal(format!(
            "service installation failed: {error}; restoring the previous service failed: {restore_error}"
        )),
    })
}

pub(crate) async fn uninstall(paths: &ServicePaths) -> Result<(), AppError> {
    if !ensure_owned_definition_if_present(paths).await? {
        return Ok(());
    }
    disable_and_stop_if_loaded().await?;
    crate::util::durable_fs::durable_remove_file(&paths.definition).await?;
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        run_systemctl(&["daemon-reload"]).await?;
        drop(run_systemctl(&["reset-failed", SYSTEMD_UNIT]).await);
    }
    Ok(())
}

pub(crate) async fn stop(paths: &ServicePaths) -> Result<(), AppError> {
    if !ensure_owned_definition_if_present(paths).await? {
        return Ok(());
    }
    stop_if_loaded().await
}

pub(crate) async fn start_installed(paths: &ServicePaths) -> Result<(), AppError> {
    ensure_manages_data_root(paths).await?;
    start_loaded(paths).await
}

pub(crate) async fn restart(paths: &ServicePaths) -> Result<(), AppError> {
    ensure_manages_data_root(paths).await?;
    #[cfg(target_os = "macos")]
    return run_launchctl(&[
        "kickstart",
        "-k",
        &format!("gui/{}/{LAUNCHD_LABEL}", user_id()),
    ])
    .await;
    #[cfg(all(unix, not(target_os = "macos")))]
    return run_systemctl(&["restart", SYSTEMD_UNIT]).await;
    #[cfg(not(unix))]
    unreachable!()
}

async fn ensure_owned_definition_if_present(paths: &ServicePaths) -> Result<bool, AppError> {
    match tokio::fs::symlink_metadata(&paths.definition).await {
        Ok(_) => {
            ensure_manages_data_root(paths).await?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

async fn ensure_installable_definition(paths: &ServicePaths) -> Result<(), AppError> {
    ensure_owned_definition_if_present(paths).await.map(|_| ())
}

async fn ensure_manages_data_root(paths: &ServicePaths) -> Result<(), AppError> {
    if definition_manages_data_root(paths).await? {
        return Ok(());
    }
    Err(AppError::Conflict(
        "installed service does not manage this BoBe data root".into(),
    ))
}

async fn definition_manages_data_root(paths: &ServicePaths) -> Result<bool, AppError> {
    let definition = match tokio::fs::read_to_string(&paths.definition).await {
        Ok(definition) => definition,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let expected = render_data_root_marker(paths)?;
    Ok(definition.contains(&expected))
}

fn render_data_root_marker(paths: &ServicePaths) -> Result<String, AppError> {
    let config_root = path_text(&paths.config_root)?;
    #[cfg(target_os = "macos")]
    return Ok(format!(
        "<key>BOBE_DATA_DIR</key><string>{}</string>",
        xml(&config_root)
    ));
    #[cfg(all(unix, not(target_os = "macos")))]
    return Ok(format!(
        "Environment=\"BOBE_DATA_DIR={}\"",
        systemd(&config_root)
    ));
    #[cfg(not(unix))]
    unreachable!()
}

async fn start_loaded(paths: &ServicePaths) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    return run_launchctl(&[
        "bootstrap",
        &format!("gui/{}", user_id()),
        &path_text(&paths.definition)?,
    ])
    .await;
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = paths;
        return run_systemctl(&["start", SYSTEMD_UNIT]).await;
    }
    #[cfg(not(unix))]
    unreachable!()
}

async fn start(paths: &ServicePaths) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    return start_loaded(paths).await;
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = paths;
        return run_systemctl(&["enable", "--now", SYSTEMD_UNIT]).await;
    }
    #[cfg(not(unix))]
    unreachable!()
}

async fn stop_if_loaded() -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    {
        let uid = user_id();
        ensure_launchd_gui_domain(&uid).await?;
        let target = format!("gui/{uid}/{LAUNCHD_LABEL}");
        if !command_succeeds("/bin/launchctl", &["print", &target]).await? {
            return Ok(());
        }
        return run_launchctl(&["bootout", &target]).await;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if !command_succeeds("systemctl", &["--user", "is-active", SYSTEMD_UNIT]).await? {
            return Ok(());
        }
        return run_systemctl(&["stop", SYSTEMD_UNIT]).await;
    }
    #[cfg(not(unix))]
    unreachable!()
}

async fn disable_and_stop_if_loaded() -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    return stop_if_loaded().await;
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if !command_succeeds("systemctl", &["--user", "is-enabled", SYSTEMD_UNIT]).await?
            && !command_succeeds("systemctl", &["--user", "is-active", SYSTEMD_UNIT]).await?
        {
            return Ok(());
        }
        return run_systemctl(&["disable", "--now", SYSTEMD_UNIT]).await;
    }
    #[cfg(not(unix))]
    unreachable!()
}

#[cfg(target_os = "macos")]
fn user_id() -> String {
    crate::util::paths::effective_uid().to_string()
}

#[cfg(target_os = "macos")]
async fn ensure_launchd_gui_domain(uid: &str) -> Result<(), AppError> {
    let domain = format!("gui/{uid}");
    let output = tokio::process::Command::new("/bin/launchctl")
        .args(["print", &domain])
        .output()
        .await?;
    if output.status.success() {
        return Ok(());
    }
    Err(AppError::ServiceUnavailable(format!(
        "launchd GUI domain {domain} is unavailable; sign in to the macOS graphical session before managing the BoBe LaunchAgent (SSH alone cannot create that domain)"
    )))
}

#[cfg(target_os = "macos")]
async fn run_launchctl(args: &[&str]) -> Result<(), AppError> {
    let uid = user_id();
    ensure_launchd_gui_domain(&uid).await?;
    run_command("/bin/launchctl", args).await
}
#[cfg(all(unix, not(target_os = "macos")))]
async fn run_systemctl(args: &[&str]) -> Result<(), AppError> {
    let mut full = vec!["--user"];
    full.extend_from_slice(args);
    run_command("systemctl", &full).await
}

#[derive(Clone, Copy)]
enum QueryKind {
    LaunchdJob,
    SystemdEnabled,
    SystemdActive,
}

async fn command_succeeds(program: &str, args: &[&str]) -> Result<bool, AppError> {
    let output = tokio::process::Command::new(program)
        .args(args)
        .output()
        .await?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let kind = query_kind(args);
    if query_reports_present(kind, &stdout) {
        return Ok(true);
    }
    let absent = query_reports_absent(kind, &stdout, &stderr);
    if absent {
        Ok(false)
    } else {
        Err(AppError::Internal(format!(
            "{program} query failed: {}",
            if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            }
        )))
    }
}

fn query_reports_present(kind: QueryKind, stdout: &str) -> bool {
    matches!(kind, QueryKind::SystemdActive)
        && matches!(stdout.trim(), "activating" | "deactivating" | "reloading")
}

fn query_reports_absent(kind: QueryKind, stdout: &str, stderr: &str) -> bool {
    match kind {
        QueryKind::LaunchdJob => {
            stderr.contains("Could not find service") || stderr.contains("No such process")
        }
        QueryKind::SystemdEnabled => {
            matches!(stdout.trim(), "disabled" | "not-found")
                || (stderr.contains("Failed to get unit file state")
                    && (stderr.contains("No such file or directory")
                        || stderr.contains("not found")
                        || stderr.contains("does not exist")))
        }
        QueryKind::SystemdActive => matches!(stdout.trim(), "inactive" | "failed" | "unknown"),
    }
}

fn query_kind(args: &[&str]) -> QueryKind {
    if args.contains(&"print") {
        QueryKind::LaunchdJob
    } else if args.contains(&"is-enabled") {
        QueryKind::SystemdEnabled
    } else {
        QueryKind::SystemdActive
    }
}

async fn run_command(program: &str, args: &[&str]) -> Result<(), AppError> {
    let output = tokio::process::Command::new(program)
        .args(args)
        .output()
        .await?;
    if output.status.success() {
        return Ok(());
    }
    Err(AppError::Internal(format!(
        "{} failed: {}",
        program,
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

fn path_text(path: &Path) -> Result<String, AppError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| AppError::Config("service path is not valid UTF-8".into()))
}
#[cfg(target_os = "macos")]
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
#[cfg(all(unix, not(target_os = "macos")))]
fn systemd(value: &str) -> String {
    value
        .replace('%', "%%")
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::assert_matches;
    #[test]
    fn systemd_config_home_honors_xdg_override() {
        assert_eq!(
            systemd_config_home(
                Path::new("/home/test"),
                Some(std::ffi::OsString::from("/srv/test-config"))
            ),
            PathBuf::from("/srv/test-config")
        );
        assert_eq!(
            systemd_config_home(
                Path::new("/home/test"),
                Some(std::ffi::OsString::from("relative-config"))
            ),
            PathBuf::from("/home/test/.config")
        );
        assert_eq!(
            systemd_config_home(Path::new("/home/test"), None),
            PathBuf::from("/home/test/.config")
        );
    }

    #[test]
    fn missing_systemd_unit_is_an_absent_query_result() {
        assert!(query_reports_absent(
            QueryKind::SystemdEnabled,
            "",
            "Failed to get unit file state for bobe.service: No such file or directory"
        ));
        assert!(!query_reports_absent(
            QueryKind::SystemdEnabled,
            "",
            "Failed to connect to bus: No medium found"
        ));
        assert!(!query_reports_absent(
            QueryKind::SystemdEnabled,
            "",
            "Failed to connect to bus: No such file or directory"
        ));
    }

    #[test]
    fn transitional_systemd_states_are_present() {
        for state in ["activating", "deactivating", "reloading"] {
            assert!(query_reports_present(QueryKind::SystemdActive, state));
        }
        assert!(!query_reports_present(QueryKind::SystemdActive, "inactive"));
    }

    #[test]
    fn definition_uses_config_root_and_runtime_logs() {
        let paths = paths(
            Path::new("/home/a & b"),
            Path::new("/srv/bobe/runtime"),
            Path::new("/etc/bobe/config-root"),
        )
        .unwrap();
        let text = render(
            Path::new("/opt/BoBe & Co/bobe"),
            Some(&crate::agent::RuntimeFiles {
                node: PathBuf::from("/opt/BoBe & Co/node"),
                entry: PathBuf::from("/opt/BoBe & Co/agent/dist/main.js"),
            }),
            &paths,
        )
        .unwrap();
        assert!(text.contains("serve"));
        assert!(text.contains("BOBE_DATA_DIR"));
        assert!(text.contains("BOBE_NODE_PATH"));
        assert!(text.contains("BOBE_AGENT_ENTRY"));
        assert!(!text.contains("sudo"));
        assert_eq!(
            render_data_root_marker(&paths).unwrap(),
            if cfg!(target_os = "macos") {
                "<key>BOBE_DATA_DIR</key><string>/etc/bobe/config-root</string>"
            } else {
                "Environment=\"BOBE_DATA_DIR=/etc/bobe/config-root\""
            }
        );
        #[cfg(target_os = "macos")]
        {
            assert!(text.contains("ThrottleInterval"));
            assert!(text.contains("<key>ExitTimeOut</key><integer>45</integer>"));
            assert!(text.contains("/srv/bobe/runtime/logs/bobe"));
            assert!(text.contains("<key>BOBE_LOGGING__FILE</key>"));
            assert!(text.contains("<key>StandardOutPath</key><string>/dev/null</string>"));
            assert!(text.contains("<key>StandardErrorPath</key><string>/dev/null</string>"));
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            assert!(text.contains("StartLimitIntervalSec=300"));
            assert!(text.contains("StartLimitBurst=5"));
            assert!(text.contains("TimeoutStopSec=45"));
            assert!(text.contains("StandardOutput=journal"));
            assert!(!text.contains("StandardOutput=append:"));
        }
        #[cfg(target_os = "macos")]
        assert!(text.contains("&amp;"));
    }

    #[tokio::test]
    async fn service_install_rejects_foreign_definition_before_side_effects() {
        let root =
            std::env::temp_dir().join(format!("bobe-service-install-{}", uuid::Uuid::new_v4()));
        let home = root.join("home");
        let expected = root.join("expected");
        let other = root.join("other");
        let service_paths = paths(&home, &expected, &expected).unwrap();
        tokio::fs::create_dir_all(service_paths.definition.parent().unwrap())
            .await
            .unwrap();
        let foreign = render(
            Path::new("/opt/foreign-bobe"),
            None,
            &paths(&home, &other, &other).unwrap(),
        )
        .unwrap();
        tokio::fs::write(&service_paths.definition, &foreign)
            .await
            .unwrap();

        let error = install(Path::new("/opt/bobe"), None, &service_paths)
            .await
            .unwrap_err();
        assert_matches!(error, AppError::Conflict(_));
        assert_matches!(stop(&service_paths).await, Err(AppError::Conflict(_)));
        assert_matches!(uninstall(&service_paths).await, Err(AppError::Conflict(_)));

        assert_eq!(
            tokio::fs::read_to_string(&service_paths.definition)
                .await
                .unwrap(),
            foreign
        );
        assert!(!service_paths.logs.exists());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    struct FakeManager {
        fail: Option<&'static str>,
        calls: std::sync::Mutex<Vec<&'static str>>,
        running: bool,
    }

    impl FakeManager {
        fn new(fail: Option<&'static str>, running: bool) -> Self {
            Self {
                fail,
                calls: std::sync::Mutex::new(Vec::new()),
                running,
            }
        }

        fn call(&self, name: &'static str) -> Result<(), AppError> {
            let mut calls = self.calls.lock().unwrap();
            calls.push(name);
            let first = calls.iter().filter(|call| **call == name).count() == 1;
            if first && self.fail == Some(name) {
                return Err(AppError::Internal(format!("{name} failed")));
            }
            Ok(())
        }

        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[expect(
        clippy::unused_async_trait_impl,
        reason = "the fake answers synchronously"
    )]
    impl ServiceManager for FakeManager {
        async fn check_definition(&self, _definition: &Path) -> Result<(), AppError> {
            self.call("check")
        }

        async fn running(&self) -> Result<bool, AppError> {
            self.call("running").map(|()| self.running)
        }

        async fn stop(&self) -> Result<(), AppError> {
            self.call("stop")
        }

        async fn reload(&self) -> Result<(), AppError> {
            self.call("reload")
        }

        async fn start(&self, _paths: &ServicePaths) -> Result<(), AppError> {
            self.call("start")
        }
    }

    fn reinstall_fixture(previous: Option<&str>) -> (PathBuf, ServicePaths) {
        let root =
            std::env::temp_dir().join(format!("bobe-service-reinstall-{}", uuid::Uuid::new_v4()));
        let service_paths =
            paths(&root.join("home"), &root.join("data"), &root.join("data")).unwrap();
        std::fs::create_dir_all(service_paths.definition.parent().unwrap()).unwrap();
        if let Some(previous) = previous {
            std::fs::write(&service_paths.definition, previous).unwrap();
        }
        (root, service_paths)
    }

    fn staged_leftovers(service_paths: &ServicePaths) -> usize {
        std::fs::read_dir(service_paths.definition.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
            .count()
    }

    #[tokio::test]
    async fn a_successful_reinstall_validates_before_stopping_the_previous_service() {
        let (root, service_paths) = reinstall_fixture(Some("previous"));
        let manager = FakeManager::new(None, true);

        replace_definition(&manager, &service_paths, b"next")
            .await
            .unwrap();

        assert_eq!(
            manager.calls(),
            ["check", "running", "stop", "reload", "start"]
        );
        assert_eq!(std::fs::read(&service_paths.definition).unwrap(), b"next");
        assert_eq!(staged_leftovers(&service_paths), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_reinstall_that_fails_before_stopping_leaves_the_previous_service_untouched() {
        for failure in ["check", "running", "stop"] {
            let (root, service_paths) = reinstall_fixture(Some("previous"));
            let manager = FakeManager::new(Some(failure), true);

            let error = replace_definition(&manager, &service_paths, b"next")
                .await
                .expect_err("injected failure");

            assert!(error.to_string().contains(failure), "{failure}: {error}");
            assert!(!manager.calls().contains(&"start"), "{failure}");
            assert_eq!(
                std::fs::read(&service_paths.definition).unwrap(),
                b"previous",
                "{failure}"
            );
            assert_eq!(staged_leftovers(&service_paths), 0, "{failure}");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn a_reinstall_that_fails_after_stopping_restores_and_restarts_the_previous_service() {
        for failure in ["reload", "start"] {
            let (root, service_paths) = reinstall_fixture(Some("previous"));
            let manager = FakeManager::new(Some(failure), true);

            let error = replace_definition(&manager, &service_paths, b"next")
                .await
                .expect_err("injected failure");

            assert!(error.to_string().contains(failure), "{failure}: {error}");
            assert_eq!(
                std::fs::read(&service_paths.definition).unwrap(),
                b"previous",
                "{failure}"
            );
            let calls = manager.calls();
            assert_eq!(calls.last(), Some(&"start"), "{failure}: {calls:?}");
            assert_eq!(
                calls.iter().filter(|call| **call == "start").count(),
                if failure == "start" { 2 } else { 1 },
                "{failure}: {calls:?}"
            );
            assert_eq!(staged_leftovers(&service_paths), 0, "{failure}");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn a_failed_first_install_leaves_no_service_definition() {
        let (root, service_paths) = reinstall_fixture(None);
        let manager = FakeManager::new(Some("start"), false);

        replace_definition(&manager, &service_paths, b"next")
            .await
            .expect_err("injected failure");

        assert!(!service_paths.definition.exists());
        assert_eq!(
            manager.calls(),
            [
                "check", "running", "stop", "reload", "start", "stop", "reload"
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_a_running_launchd_job_counts_as_running() {
        assert!(launchd_reports_running(
            "gui/501/com.bobe.daemon = {\n\tactive count = 1\n\tstate = running\n\tjob state = running\n}"
        ));
        assert!(!launchd_reports_running(
            "gui/501/com.bobe.daemon = {\n\tstate = not running\n\t\tstate = active\n}"
        ));
    }

    #[tokio::test]
    async fn a_stopped_previous_service_is_restored_without_starting_it() {
        let (root, service_paths) = reinstall_fixture(Some("previous"));
        let manager = FakeManager::new(Some("start"), false);

        replace_definition(&manager, &service_paths, b"next")
            .await
            .expect_err("injected failure");

        assert_eq!(
            std::fs::read(&service_paths.definition).unwrap(),
            b"previous"
        );
        assert_eq!(
            manager.calls(),
            [
                "check", "running", "stop", "reload", "start", "stop", "reload"
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn absent_service_stop_and_uninstall_are_idempotent() {
        let root =
            std::env::temp_dir().join(format!("bobe-service-absent-{}", uuid::Uuid::new_v4()));
        let service_paths =
            paths(&root.join("home"), &root.join("data"), &root.join("data")).unwrap();

        assert!(stop(&service_paths).await.is_ok());
        assert!(uninstall(&service_paths).await.is_ok());
        assert!(!service_paths.definition.exists());
    }

    #[tokio::test]
    async fn service_definition_must_match_config_root() {
        let root =
            std::env::temp_dir().join(format!("bobe-service-definition-{}", uuid::Uuid::new_v4()));
        let home = root.join("home");
        let expected = root.join("expected");
        let other = root.join("other");
        let service_paths = paths(&home, &expected, &expected).unwrap();
        tokio::fs::create_dir_all(service_paths.definition.parent().unwrap())
            .await
            .unwrap();
        let other_paths = paths(&home, &other, &other).unwrap();
        let definition = render(Path::new("/opt/bobe"), None, &other_paths).unwrap();
        tokio::fs::write(&service_paths.definition, definition)
            .await
            .unwrap();
        assert!(!definition_manages_data_root(&service_paths).await.unwrap());
        assert_matches!(
            ensure_manages_data_root(&service_paths).await,
            Err(AppError::Conflict(_))
        );

        let definition = render(Path::new("/opt/bobe"), None, &service_paths).unwrap();
        tokio::fs::write(&service_paths.definition, definition)
            .await
            .unwrap();
        assert!(definition_manages_data_root(&service_paths).await.unwrap());
        assert!(ensure_manages_data_root(&service_paths).await.is_ok());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}

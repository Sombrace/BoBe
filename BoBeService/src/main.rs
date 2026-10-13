#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "test fixtures fail fast; production remains linted"
    )
)]

use clap::{Parser, Subcommand};
use secrecy::ExposeSecret;
use std::sync::Arc;

mod agent;
mod api;
mod app_state;
mod body;
mod bootstrap;
mod config;
mod constants;
mod db;
mod error;
mod mcp;
mod models;
mod ops;
mod owner_token;
mod provider;
mod runtime;
#[allow(unsafe_code)]
mod secrets;
mod services;
mod speech;
mod util;
mod voice;

#[derive(Parser)]
#[command(name = "bobe", about = "BoBe - Local-first proactive AI companion")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Serve {
        #[arg(short = 'H', long)]
        host: Option<String>,
        #[arg(short, long)]
        port: Option<u16>,
        #[arg(short, long)]
        log_level: Option<String>,
        #[arg(long, hide = true)]
        managed_bootstrap_stdin: bool,
    },
    Init {
        #[arg(long)]
        json: bool,
    },
    Setup,
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    WaitReady {
        #[arg(long, default_value = "90s", value_parser = parse_duration)]
        timeout: std::time::Duration,
        #[arg(long, default_value = "500ms", value_parser = parse_duration)]
        interval: std::time::Duration,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Doctor {
        #[arg(long)]
        probe: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Chat(ops::headless::ChatArgs),
    Jobs(ops::headless::JobsArgs),
    Runs(ops::headless::RunsArgs),
    Schedules(ops::headless::SchedulesArgs),
    Input(ops::headless::InputArgs),
    Agent {
        #[command(subcommand)]
        command: ops::headless::AgentCommands,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    Initialization {
        #[command(subcommand)]
        command: InitializationCommands,
    },
    Provider {
        #[command(subcommand)]
        command: ProviderCommands,
    },
    Model {
        #[command(subcommand)]
        command: ops::headless::ModelCommands,
    },
    LocalRuntime {
        #[command(subcommand)]
        command: ops::headless::LocalRuntimeCommands,
    },
    Memory {
        #[command(subcommand)]
        command: ops::headless::MemoryCommands,
    },
    Profile {
        #[command(subcommand)]
        command: ops::headless::ProfileCommands,
    },
    Skill {
        #[command(subcommand)]
        command: ops::headless::SkillCommands,
    },
    Workspace {
        #[command(subcommand)]
        command: ops::headless::WorkspaceCommands,
    },
    Endpoint {
        #[command(subcommand)]
        command: EndpointCommands,
    },
    Device {
        #[command(subcommand)]
        command: DeviceCommands,
    },
    #[command(name = "voice-models")]
    Models {
        #[command(subcommand)]
        command: ModelsCommands,
    },
    Service {
        #[command(subcommand)]
        command: ServiceCommands,
    },
    Backup {
        #[command(subcommand)]
        command: BackupCommands,
    },
    Update {
        #[command(subcommand)]
        command: UpdateCommands,
    },
    Version {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum UpdateCommands {
    Apply {
        #[arg(long)]
        manifest: std::path::PathBuf,
        #[arg(long)]
        signature: std::path::PathBuf,
        #[arg(long)]
        archive: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
enum BackupCommands {
    Create,
    List,
    Verify { path: std::path::PathBuf },
    Restore { path: std::path::PathBuf },
}

#[derive(Subcommand)]
enum ServiceCommands {
    Install,
    Status,
    Logs,
    Restart,
    Uninstall,
}

#[derive(Subcommand)]
enum ConfigCommands {
    Path {
        #[arg(long)]
        json: bool,
    },
    Show {
        #[arg(long)]
        json: bool,
    },
    Validate {
        #[arg(long)]
        json: bool,
    },
    Remote {
        #[arg(long)]
        host: String,
        #[arg(long = "allowed-host", required = true)]
        allowed_hosts: Vec<String>,
        #[arg(long)]
        tls_cert: Option<std::path::PathBuf>,
        #[arg(long)]
        tls_key: Option<std::path::PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Get {
        key: String,
        #[arg(long)]
        json: bool,
    },
    Unset {
        key: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Set {
        key: String,
        value: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand)]
enum ModelsCommands {
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Install {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Cancel {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand)]
enum EndpointCommands {
    Display {
        id: String,
        #[arg(long)]
        text: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Sensors {
        id: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    List {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Enroll {
        id: String,
        #[arg(long, value_parser = ["physical_body", "mac_app", "other"])]
        kind: String,
        #[arg(long)]
        label: String,
        #[arg(long)]
        room: Option<String>,
        #[arg(long)]
        certificate_sha256: Option<String>,
        #[arg(long, default_value_t = 1)]
        protocol_min: i64,
        #[arg(long, default_value_t = 1)]
        protocol_max: i64,
        #[arg(long)]
        capabilities: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Revoke {
        id: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand)]
enum DeviceCommands {
    Pair {
        #[arg(long, default_value = "mac_app", value_parser = ["mac_app", "phone"])]
        kind: String,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        json: bool,
    },
    List {
        #[arg(long)]
        json: bool,
    },
    Revoke {
        id: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ProviderCommands {
    List {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Upsert {
        id: String,
        #[arg(long, value_enum)]
        kind: provider::ProviderKind,
        #[arg(long, value_enum, default_value = "api-key")]
        auth_method: provider::ProviderAuthMethod,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        organization: Option<String>,
        #[arg(long)]
        offline: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Activate {
        id: String,
        #[arg(long, value_enum)]
        kind: provider::ProviderKind,
        #[arg(long, value_enum, default_value = "api-key")]
        auth_method: provider::ProviderAuthMethod,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        organization: Option<String>,
        #[arg(long)]
        offline: bool,
        #[arg(long, value_enum)]
        local_runtime: Option<provider::LocalRuntimeKind>,
        #[arg(long)]
        credential_stdin: bool,
        #[arg(long)]
        chat_model: Option<String>,
        #[arg(long)]
        goals_model: Option<String>,
        #[arg(long)]
        consolidate_model: Option<String>,
        #[arg(long)]
        small_model: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Credential {
        id: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    ClearCredential {
        id: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Remove {
        id: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Routes {
        #[arg(long, value_name = "PATH", conflicts_with = "stdin")]
        file: Option<std::path::PathBuf>,
        #[arg(long, conflicts_with = "file")]
        stdin: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Auth {
        #[command(subcommand)]
        command: ProviderAuthCommands,
    },
}

#[derive(Subcommand)]
enum ProviderAuthCommands {
    Logout {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Status {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Login {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Cancel {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand)]
enum InitializationCommands {
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Advance {
        state: db::runtime_initialization_repo::InitializationState,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

#[tokio::main]
#[allow(
    clippy::print_stderr,
    reason = "startup failures must be visible before tracing initializes"
)]
async fn main() -> std::process::ExitCode {
    match Box::pin(run()).await {
        Ok(exit) => exit.process(),
        Err(error) => {
            eprintln!("{error:#}");
            error
                .downcast_ref::<error::AppError>()
                .map_or(ops::exit::ExitCode::Failed, ops::exit::app_error_exit)
                .process()
        }
    }
}

async fn run() -> anyhow::Result<ops::exit::ExitCode> {
    let cli = Cli::parse();
    util::tls::install_crypto_provider()?;

    match cli.command {
        Commands::Serve {
            host,
            port,
            log_level,
            managed_bootstrap_stdin,
        } => {
            let config::LoadedConfig {
                mut config,
                paths,
                config_file,
            } = config::Config::load_operational()?;
            if let Some(host) = host {
                config.server.host = host;
            }
            if let Some(port) = port {
                config.server.port = port;
            }
            if let Some(log_level) = log_level {
                config.logging.level = log_level;
            }

            let paths = paths.prepare_root()?;
            config.data_dir = paths.root().to_string_lossy().into_owned();
            config.database.url = paths.database_url().to_owned();
            let runtime_locks = util::runtime_lock::RuntimeLocks::acquire(&paths)?;
            if config.server.api_token.expose_secret().is_empty() {
                config.server.api_token = secrecy::SecretString::from(owner_token::ensure(&paths)?);
            }
            if config.server.is_networked()
                && config.server.tls_cert_path.is_none()
                && config.server.tls_key_path.is_none()
            {
                let identity =
                    util::automatic_tls::ensure(&paths, &config.server.allowed_hosts).await?;
                config.server.tls_cert_path =
                    Some(identity.certificate_path.to_string_lossy().into_owned());
                config.server.tls_key_path =
                    Some(identity.private_key_path.to_string_lossy().into_owned());
                tracing::info!(
                    spki_sha256 = %identity.spki_sha256,
                    "server.automatic_tls_ready"
                );
            }
            util::logging::init_tracing(&config);

            tracing::info!(
                "Starting BoBe on {}:{}",
                config.server.host,
                config.server.port
            );

            validate_server_security(&config)?;
            let local_enrollment = if managed_bootstrap_stdin {
                use tokio::io::AsyncReadExt;
                let mut secret = Vec::new();
                tokio::io::stdin()
                    .take(4096)
                    .read_to_end(&mut secret)
                    .await?;
                if secret.len() != 32 {
                    anyhow::bail!("managed bootstrap secret must contain exactly 32 bytes");
                }
                api::handlers::local_enrollment::LocalEnrollmentBootstrap::from_secret(&secret)
            } else {
                api::handlers::local_enrollment::LocalEnrollmentBootstrap::disabled()
            };
            let shutdown = tokio_util::sync::CancellationToken::new();
            let state = bootstrap::run(
                config.clone(),
                paths,
                config_file,
                runtime_locks,
                local_enrollment,
            )
            .await?;
            api::handlers::conversation::recover_message_requests(&state).await?;
            let app = api::router::build_router(std::sync::Arc::clone(&state), shutdown.clone());

            let mut tasks = spawn_background_tasks(&state, &shutdown);

            let address = server_socket_addr(&config)?;
            tracing::info!(%address, "BoBe listening");

            let handle = axum_server::Handle::new();
            let shutdown_handle = handle.clone();
            let shutdown_for_server = shutdown.clone();
            let requests = Arc::clone(&state.runtime.message_requests);
            let managing_app = managed_bootstrap_stdin.then(managing_process_id);
            let signal_task = tokio::spawn(async move {
                tokio::select! {
                    () = wait_for_shutdown_signal() => {
                        tracing::info!("Shutdown signal received, stopping background tasks...");
                    }
                    () = wait_for_managing_app_exit(managing_app) => {
                        tracing::warn!("The app that started this daemon is gone, stopping...");
                    }
                }
                requests.begin_shutdown();
                shutdown_for_server.cancel();
                shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(5)));
            });

            let served = async {
                if let (Some(cert), Some(key)) = (
                    config.server.tls_cert_path.as_deref(),
                    config.server.tls_key_path.as_deref(),
                ) {
                    let tls =
                        axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?;
                    axum_server::bind(address)
                        .acceptor(util::listener::LocalPlainAcceptor::new(
                            axum_server::tls_rustls::RustlsAcceptor::new(tls),
                        ))
                        .handle(handle)
                        .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                        .await?;
                } else {
                    axum_server::bind(address)
                        .handle(handle)
                        .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                        .await?;
                }
                Ok::<(), std::io::Error>(())
            }
            .await;
            state.runtime.message_requests.begin_shutdown();
            shutdown.cancel();
            signal_task.abort();
            drain_background_tasks(&mut tasks).await;
            run_graceful_shutdown(&state).await;
            served?;
        }

        Commands::Init { json } => {
            let loaded = config::Config::load_operational()?;
            let validation = ops::config_report::validate(&loaded);
            if !validation.valid {
                emit_report(json, &validation, render_validation(&validation));
                return Ok(ops::exit::ExitCode::Invalid);
            }
            let mut config = loaded.config;
            let paths = loaded.paths.prepare_root()?;
            config.data_dir = paths.root().to_string_lossy().into_owned();
            config.database.url = paths.database_url().to_owned();
            let _locks = util::runtime_lock::RuntimeLocks::acquire(&paths)?;
            if config.server.api_token.expose_secret().is_empty() {
                config.server.api_token = secrecy::SecretString::from(owner_token::ensure(&paths)?);
            }
            let (pool, identity) = bootstrap::initialize_storage(&paths, false).await?;
            let initialization_state = db::runtime_initialization_repo::ensure(&pool).await?;
            let report = serde_json::json!({
                "schema": "bobe.cli.v1",
                "command": "init",
                "initialized": true,
                "data_root": paths.root(),
                "database_url": paths.database_url(),
                "runtime_authority_id": identity.runtime_authority_id,
                "runtime_authority_epoch": identity.runtime_authority_epoch,
                "runtime_incarnation_id": identity.runtime_incarnation_id,
                "schema_version": identity.schema_version,
                "daemon_version": identity.daemon_version,
                "initialization_state": initialization_state,
            });
            pool.close().await;
            emit_report(
                json,
                &report,
                format!(
                    "BoBe runtime initialized.\nData root: {}\nAuthority: {} epoch {}\nSchema: {}",
                    paths.root().display(),
                    identity.runtime_authority_id,
                    identity.runtime_authority_epoch,
                    identity.schema_version
                ),
            );
        }
        Commands::Status { json, url, token } => {
            let loaded = config::Config::load_operational()?;
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let (exit, report) = ops::status::collect(&target).await;
            emit_report(json, &report, render_status(&report));
            return Ok(exit);
        }
        Commands::WaitReady {
            timeout,
            interval,
            json,
            url,
            token,
        } => {
            let loaded = config::Config::load_operational()?;
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let (exit, report) = ops::status::wait_ready(&target, timeout, interval).await;
            emit_report(json, &report, render_status(&report));
            return Ok(exit);
        }
        Commands::Doctor {
            probe,
            json,
            url,
            token,
        } => {
            let loaded = config::Config::load_operational()?;
            let validation = ops::config_report::validate(&loaded);
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let (status_exit, status) = ops::status::collect(&target).await;
            let provider = if probe && status.reachable {
                match target.diagnose_provider().await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => serde_json::json!({"ok": false, "error": error.to_string()}),
                }
            } else {
                serde_json::json!({
                    "ok": null,
                    "status": "not_tested",
                    "message": "Provider/model execution was not tested. Run bobe doctor --probe (may incur provider charges).",
                })
            };
            let probe_ok = !probe || provider["ok"] == true;
            let report = serde_json::json!({
                "schema": "bobe.cli.v1",
                "command": "doctor",
                "check_scope": if probe { "runtime_and_provider" } else { "runtime_only" },
                "ok": validation.valid && status_exit == ops::exit::ExitCode::Success && probe_ok,
                "config": validation,
                "runtime": status,
                "provider": provider,
            });
            let exit = if report["config"]["valid"].as_bool() == Some(false) {
                ops::exit::ExitCode::Invalid
            } else if status_exit != ops::exit::ExitCode::Success {
                status_exit
            } else if !probe_ok {
                ops::exit::ExitCode::Failed
            } else {
                ops::exit::ExitCode::Success
            };
            emit_report(json, &report, render_doctor(&report));
            return Ok(exit);
        }
        Commands::Setup => {
            let loaded = config::Config::load_operational()?;
            let home = std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(std::path::PathBuf::from)
                .ok_or_else(|| error::AppError::Config("HOME is unavailable".into()))?;
            let service_paths = ops::service::paths(
                &home,
                loaded.paths.root(),
                loaded.config_file.parent().ok_or_else(|| {
                    error::AppError::Config("config file has no parent directory".into())
                })?,
            )?;
            let executable = std::env::current_exe()?.canonicalize()?;
            let agent = agent_runtime_for_executable(&executable);
            return Ok(ops::setup::execute(ops::setup::SetupContext {
                executable,
                agent,
                service_paths,
            })
            .await?);
        }
        Commands::Chat(args) => return ops::headless::execute_chat(args).await,
        Commands::Jobs(args) => return ops::headless::execute_jobs(args).await,
        Commands::Runs(args) => return ops::headless::execute_runs(args).await,
        Commands::Schedules(args) => return ops::headless::execute_schedules(args).await,
        Commands::Input(args) => return ops::headless::execute_input(args).await,
        Commands::Agent { command } => return ops::headless::execute_agent(command).await,
        Commands::Config { command } => {
            let loaded = config::Config::load_operational()?;
            match command {
                ConfigCommands::Path { json } => {
                    let report = ops::config_report::paths(&loaded);
                    emit_report(json, &report, render_config_paths(&report));
                }
                ConfigCommands::Show { json } => {
                    emit_report(
                        json,
                        &loaded.config,
                        serde_json::to_string_pretty(&loaded.config)?,
                    );
                }
                ConfigCommands::Validate { json } => {
                    let report = ops::config_report::validate(&loaded);
                    let exit = if report.valid {
                        ops::exit::ExitCode::Success
                    } else {
                        ops::exit::ExitCode::Invalid
                    };
                    emit_report(json, &report, render_validation(&report));
                    return Ok(exit);
                }
                ConfigCommands::Remote {
                    host,
                    allowed_hosts,
                    tls_cert,
                    tls_key,
                    json,
                } => {
                    let changes = remote_config_changes(
                        &loaded,
                        host,
                        allowed_hosts,
                        tls_cert.as_deref(),
                        tls_key.as_deref(),
                    )?;
                    let mut candidate = loaded.config.clone();
                    config::manager_fields::apply(
                        &mut candidate,
                        &changes.clone().into_iter().collect(),
                    );
                    let candidate = config::LoadedConfig {
                        config: candidate,
                        paths: loaded.paths.clone(),
                        config_file: loaded.config_file.clone(),
                    };
                    let validation = ops::config_report::validate(&candidate);
                    if !validation.valid {
                        emit_report(json, &validation, render_validation(&validation));
                        return Ok(ops::exit::ExitCode::Invalid);
                    }
                    let prepared_paths = loaded.paths.clone().prepare_root()?;
                    let _locks = util::runtime_lock::RuntimeLocks::acquire(&prepared_paths)?;
                    let config_root = loaded.config_file.parent().ok_or_else(|| {
                        error::AppError::Config("config file has no parent directory".into())
                    })?;
                    if !config::manager_persistence::persist(config_root, &changes) {
                        return Err(error::AppError::Io(std::io::Error::other(
                            "failed to persist remote configuration",
                        ))
                        .into());
                    }
                    let report = serde_json::json!({
                        "updated": true,
                        "restart_required": true,
                        "host": candidate.config.server.host,
                        "allowed_hosts": candidate.config.server.allowed_hosts,
                        "tls_cert_path": candidate.config.server.tls_cert_path,
                        "tls_key_path": candidate.config.server.tls_key_path,
                    });
                    emit_report(
                        json,
                        &report,
                        "Remote server configuration saved. Restart the BoBe daemon to activate it."
                            .into(),
                    );
                }
                ConfigCommands::Get { key, json } => {
                    let value = config_value(&loaded.config, &key).ok_or_else(|| {
                        error::AppError::Config(format!("unknown configuration key: {key}"))
                    })?;
                    let source = config_value_source(&loaded, &key)?;
                    let default = config_default_value(&loaded, &key);
                    let report = serde_json::json!({
                        "key": key,
                        "value": value,
                        "source": source,
                        "default": default,
                    });
                    emit_report(
                        json,
                        &report,
                        format!("{} (source: {})", report["value"], source),
                    );
                }
                ConfigCommands::Unset {
                    key,
                    json,
                    url,
                    token,
                } => {
                    if !config::manager_fields::is_known_key(&key) {
                        return Err(error::AppError::Config(format!(
                            "unknown configuration key: {key}"
                        ))
                        .into());
                    }
                    let report = if url.is_none() {
                        match remove_offline_config(&loaded, &key) {
                            Ok(()) => unset_config_report(&loaded, &key)?,
                            Err(error::AppError::Conflict(_)) => {
                                unset_live_config(&loaded.config, None, token.as_deref(), &key)
                                    .await?
                            }
                            Err(error) => return Err(error.into()),
                        }
                    } else {
                        unset_live_config(&loaded.config, url.as_deref(), token.as_deref(), &key)
                            .await?
                    };
                    emit_report(json, &report, format!("Removed override for {key}."));
                }
                ConfigCommands::Set {
                    key,
                    value,
                    json,
                    url,
                    token,
                } => {
                    if !config::manager_fields::is_known_key(&key) {
                        return Err(error::AppError::Config(format!(
                            "unknown configuration key: {key}"
                        ))
                        .into());
                    }
                    let value = serde_json::from_str::<serde_json::Value>(&value)
                        .unwrap_or(serde_json::Value::String(value));
                    let current_value = config_value(&loaded.config, &key).ok_or_else(|| {
                        error::AppError::Config(format!("unknown configuration key: {key}"))
                    })?;
                    if !config::manager_fields::compatible_json_type(&current_value, &value) {
                        return Err(error::AppError::Config(format!(
                            "invalid value type for {key}: expected {}, received {}",
                            json_type_name(&current_value),
                            json_type_name(&value)
                        ))
                        .into());
                    }
                    let mut updated = loaded.config.clone();
                    let changes = std::collections::HashMap::from([(key.clone(), value.clone())]);
                    config::manager_fields::apply(&mut updated, &changes);
                    let applied_value = config_value(&updated, &key).ok_or_else(|| {
                        error::AppError::Config(format!("unknown configuration key: {key}"))
                    })?;
                    if !config::manager_fields::equivalent_json_value(&applied_value, &value) {
                        return Err(error::AppError::Config(format!(
                            "value is not valid for {key}: requested {value}, effective value remains {applied_value}"
                        ))
                        .into());
                    }
                    let candidate = config::LoadedConfig {
                        config: updated,
                        paths: loaded.paths.clone(),
                        config_file: loaded.config_file.clone(),
                    };
                    let validation = ops::config_report::validate(&candidate);
                    if !validation.valid {
                        emit_report(json, &validation, render_validation(&validation));
                        return Ok(ops::exit::ExitCode::Invalid);
                    }
                    let report = if url.is_none() {
                        match persist_offline_config(&loaded, &key, &value) {
                            Ok(()) => {
                                serde_json::json!({"updated": true, "key": key, "value": value, "source": "local_files"})
                            }
                            Err(error::AppError::Conflict(_)) => {
                                update_live_config(
                                    &loaded.config,
                                    None,
                                    token.as_deref(),
                                    &key,
                                    &value,
                                )
                                .await?
                            }
                            Err(error) => return Err(error.into()),
                        }
                    } else {
                        update_live_config(
                            &loaded.config,
                            url.as_deref(),
                            token.as_deref(),
                            &key,
                            &value,
                        )
                        .await?
                    };
                    emit_report(json, &report, format!("Updated {}.", report["key"]));
                }
            }
        }
        Commands::Initialization { command } => {
            let loaded = config::Config::load_operational()?;
            match command {
                InitializationCommands::Status { json, url, token } => {
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target.get("/initialization").await?;
                    let status = response.status();
                    let report: serde_json::Value = response.json().await?;
                    emit_report(json, &report, serde_json::to_string_pretty(&report)?);
                    return Ok(if status.is_success() {
                        ops::exit::ExitCode::Success
                    } else {
                        ops::exit::ExitCode::Failed
                    });
                }
                InitializationCommands::Advance {
                    state,
                    json,
                    url,
                    token,
                } => {
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target
                        .post_json(
                            "/initialization",
                            &serde_json::json!({"initialization_state": state}),
                        )
                        .await?;
                    let status = response.status();
                    let report: serde_json::Value = response.json().await?;
                    emit_report(json, &report, serde_json::to_string_pretty(&report)?);
                    return Ok(if status.is_success() {
                        ops::exit::ExitCode::Success
                    } else {
                        ops::exit::ExitCode::Invalid
                    });
                }
            }
        }
        Commands::Endpoint { command } => {
            let loaded = config::Config::load_operational()?;
            match command {
                EndpointCommands::Display {
                    id,
                    text,
                    json,
                    url,
                    token,
                } => {
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target
                        .post_json(
                            &format!("/endpoints/{id}/display"),
                            &serde_json::json!({"text":text}),
                        )
                        .await?;
                    let (status, report) = response_json(response).await?;
                    emit_report(json, &report, serde_json::to_string_pretty(&report)?);
                    return Ok(http_exit(status));
                }
                EndpointCommands::Sensors {
                    id,
                    json,
                    url,
                    token,
                } => {
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target.get(&format!("/endpoints/{id}/sensors")).await?;
                    let (status, report) = response_json(response).await?;
                    emit_report(json, &report, serde_json::to_string_pretty(&report)?);
                    return Ok(http_exit(status));
                }
                EndpointCommands::List { json, url, token } => {
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target.get("/endpoints").await?;
                    let (status, report) = response_json(response).await?;
                    emit_report(json, &report, serde_json::to_string_pretty(&report)?);
                    return Ok(http_exit(status));
                }
                EndpointCommands::Enroll {
                    id,
                    kind,
                    label,
                    room,
                    certificate_sha256,
                    protocol_min,
                    protocol_max,
                    capabilities,
                    json,
                    url,
                    token,
                } => {
                    let capabilities = capabilities
                        .as_deref()
                        .map(serde_json::from_str)
                        .transpose()
                        .map_err(|error| {
                            error::AppError::Validation(format!(
                                "endpoint capabilities must be valid JSON: {error}"
                            ))
                        })?
                        .unwrap_or_else(|| serde_json::json!({}));
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target
                        .put_json(
                            &format!("/endpoints/{id}"),
                            &serde_json::json!({
                                "kind": kind,
                                "label": label,
                                "room": room,
                                "certificate_sha256": certificate_sha256,
                                "protocol_min": protocol_min,
                                "protocol_max": protocol_max,
                                "capabilities": capabilities,
                            }),
                        )
                        .await?;
                    let status = response.status();
                    if status == reqwest::StatusCode::NO_CONTENT {
                        let report = serde_json::json!({"id": id, "enrolled": true});
                        emit_report(json, &report, format!("Endpoint enrolled: {id}"));
                    } else {
                        let (_, report) = response_json(response).await?;
                        emit_report(json, &report, serde_json::to_string_pretty(&report)?);
                    }
                    return Ok(http_exit(status));
                }
                EndpointCommands::Revoke {
                    id,
                    json,
                    url,
                    token,
                } => {
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target.delete(&format!("/endpoints/{id}")).await?;
                    let status = response.status();
                    if status == reqwest::StatusCode::NO_CONTENT {
                        let report = serde_json::json!({"id": id, "revoked": true});
                        emit_report(json, &report, format!("Endpoint revoked: {id}"));
                    } else {
                        let (_, report) = response_json(response).await?;
                        emit_report(json, &report, serde_json::to_string_pretty(&report)?);
                    }
                    return Ok(http_exit(status));
                }
            }
        }
        Commands::Provider { command } => {
            let loaded = config::Config::load_operational()?;
            match command {
                ProviderCommands::List { json, url, token } => {
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target.get("/providers").await?;
                    let (status, report) = response_json(response).await?;
                    emit_report(json, &report, serde_json::to_string_pretty(&report)?);
                    return Ok(http_exit(status));
                }
                ProviderCommands::Upsert {
                    id,
                    kind,
                    auth_method,
                    base_url,
                    organization,
                    offline,
                    json,
                    url,
                    token,
                } => {
                    provider::ProfileId::parse(&id)?;
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target
                        .put_json(
                            &format!("/providers/{id}"),
                            &serde_json::json!({
                                "kind": kind,
                                "auth_method": auth_method,
                                "base_url": base_url,
                                "organization": organization,
                                "offline": offline,
                            }),
                        )
                        .await?;
                    let (status, report) = response_json(response).await?;
                    emit_report(
                        json,
                        &report,
                        render_provider_mutation("Provider profile updated", &report),
                    );
                    return Ok(http_exit(status));
                }
                ProviderCommands::Activate {
                    id,
                    kind,
                    auth_method,
                    base_url,
                    organization,
                    offline,
                    local_runtime,
                    credential_stdin,
                    chat_model,
                    goals_model,
                    consolidate_model,
                    small_model,
                    json,
                    url,
                    token,
                } => {
                    provider::ProfileId::parse(&id)?;
                    let api_key = credential_stdin.then(read_secret_stdin).transpose()?;
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target
                        .put_json(
                            &format!("/providers/{id}/activation"),
                            &serde_json::json!({
                                "kind": kind,
                                "auth_method": auth_method,
                                "base_url": base_url,
                                "organization": organization,
                                "offline": offline,
                                "local_runtime": local_runtime,
                                "api_key": api_key,
                                "chat_model": chat_model,
                                "goals_model": goals_model,
                                "consolidate_model": consolidate_model,
                                "small_model": small_model,
                            }),
                        )
                        .await?;
                    let (status, report) = response_json(response).await?;
                    emit_report(
                        json,
                        &report,
                        render_provider_mutation("Provider activated", &report),
                    );
                    return Ok(http_exit(status));
                }
                ProviderCommands::Credential {
                    id,
                    json,
                    url,
                    token,
                } => {
                    provider::ProfileId::parse(&id)?;
                    let api_key = read_secret_stdin()?;
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target
                        .put_json(
                            &format!("/providers/{id}/credential"),
                            &serde_json::json!({"api_key": api_key}),
                        )
                        .await?;
                    let (status, report) = response_json(response).await?;
                    emit_report(
                        json,
                        &report,
                        render_provider_mutation("Provider credential stored", &report),
                    );
                    return Ok(http_exit(status));
                }
                ProviderCommands::ClearCredential {
                    id,
                    json,
                    url,
                    token,
                } => {
                    provider::ProfileId::parse(&id)?;
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target
                        .delete(&format!("/providers/{id}/credential"))
                        .await?;
                    let (status, report) = response_json(response).await?;
                    emit_report(
                        json,
                        &report,
                        render_provider_mutation("Provider credential removed", &report),
                    );
                    return Ok(http_exit(status));
                }
                ProviderCommands::Remove {
                    id,
                    json,
                    url,
                    token,
                } => {
                    provider::ProfileId::parse(&id)?;
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target.delete(&format!("/providers/{id}")).await?;
                    let (status, report) = response_json(response).await?;
                    emit_report(
                        json,
                        &report,
                        render_provider_mutation("Provider profile removed", &report),
                    );
                    return Ok(http_exit(status));
                }
                ProviderCommands::Routes {
                    file,
                    stdin,
                    json,
                    url,
                    token,
                } => {
                    let routes = read_routes_input(file.as_deref(), stdin)?;
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target.put_json("/provider-routes", &routes).await?;
                    let (status, report) = response_json(response).await?;
                    emit_report(
                        json,
                        &report,
                        render_provider_mutation("Provider routes updated", &report),
                    );
                    return Ok(http_exit(status));
                }
                ProviderCommands::Auth { command } => {
                    let signing_out = matches!(&command, ProviderAuthCommands::Logout { .. });
                    match command {
                        ProviderAuthCommands::Status {
                            profile,
                            json,
                            url,
                            token,
                        } => {
                            let target = ops::status::OpsTarget::resolve(
                                &loaded.config,
                                url.as_deref(),
                                token.as_deref(),
                            )
                            .map_err(error::AppError::Config)?;
                            let profile = profile.unwrap_or_else(|| {
                                loaded.config.routes.chat.profile.as_str().into()
                            });
                            provider::ProfileId::parse(&profile)?;
                            let response = target.get(&format!("/auth/{profile}/status")).await?;
                            let status = response.status();
                            let body: serde_json::Value = response.json().await?;
                            emit_report(json, &body, serde_json::to_string_pretty(&body)?);
                            return Ok(if status.is_success() {
                                ops::exit::ExitCode::Success
                            } else {
                                ops::exit::ExitCode::Failed
                            });
                        }
                        ProviderAuthCommands::Login {
                            profile,
                            json,
                            url,
                            token,
                        } => {
                            let target = ops::status::OpsTarget::resolve(
                                &loaded.config,
                                url.as_deref(),
                                token.as_deref(),
                            )
                            .map_err(error::AppError::Config)?;
                            let profile = profile.unwrap_or_else(|| {
                                loaded.config.routes.chat.profile.as_str().into()
                            });
                            provider::ProfileId::parse(&profile)?;
                            let start =
                                target.post(&format!("/auth/{profile}/login/start")).await?;
                            if !start.status().is_success() {
                                return Err(error::AppError::Internal(format!(
                                    "Provider login start failed with HTTP {}",
                                    start.status()
                                ))
                                .into());
                            }
                            let phase = match wait_for_provider_login(&target, &profile).await {
                                Ok(phase) => phase,
                                Err(error) => {
                                    let cancellation =
                                        target.post(&format!("/auth/{profile}/login/cancel")).await;
                                    if !matches!(cancellation, Ok(response) if response.status().is_success())
                                    {
                                        tracing::error!(
                                            "provider login cleanup failed after CLI input ended"
                                        );
                                    }
                                    return Err(error);
                                }
                            };
                            let completed = phase["phase"] == "completed";
                            emit_report(json, &phase, serde_json::to_string_pretty(&phase)?);
                            return Ok(if completed {
                                ops::exit::ExitCode::Success
                            } else {
                                ops::exit::ExitCode::Failed
                            });
                        }
                        ProviderAuthCommands::Cancel {
                            profile,
                            json,
                            url,
                            token,
                        }
                        | ProviderAuthCommands::Logout {
                            profile,
                            json,
                            url,
                            token,
                        } => {
                            let target = ops::status::OpsTarget::resolve(
                                &loaded.config,
                                url.as_deref(),
                                token.as_deref(),
                            )
                            .map_err(error::AppError::Config)?;
                            let profile = profile.unwrap_or_else(|| {
                                loaded.config.routes.chat.profile.as_str().into()
                            });
                            provider::ProfileId::parse(&profile)?;
                            let action = if signing_out {
                                "logout"
                            } else {
                                "login/cancel"
                            };
                            let response =
                                target.post(&format!("/auth/{profile}/{action}")).await?;
                            if !response.status().is_success() {
                                anyhow::bail!(
                                    "provider authentication action failed: HTTP {}",
                                    response.status()
                                );
                            }
                            emit_report(
                                json,
                                &if signing_out {
                                    serde_json::json!({"profile":profile,"signed_out":true})
                                } else {
                                    serde_json::json!({"profile":profile,"canceled":true})
                                },
                                if signing_out {
                                    "Provider signed out."
                                } else {
                                    "Provider sign-in stopped."
                                }
                                .into(),
                            );
                            return Ok(ops::exit::ExitCode::Success);
                        }
                    }
                }
            }
        }
        Commands::Model { command } => {
            return ops::headless::execute_model(command).await;
        }
        Commands::LocalRuntime { command } => {
            return ops::headless::execute_local_runtime(command).await;
        }
        Commands::Memory { command } => {
            return ops::headless::execute_memory(command).await;
        }
        Commands::Profile { command } => {
            return ops::headless::execute_profile(command).await;
        }
        Commands::Skill { command } => {
            return ops::headless::execute_skill(command).await;
        }
        Commands::Workspace { command } => {
            return ops::headless::execute_workspace(command).await;
        }
        Commands::Device { command } => {
            let loaded = config::Config::load_operational()?;
            let path = loaded.paths.admin_socket();
            let (json, request) = match command {
                DeviceCommands::Pair { kind, label, json } => {
                    let profile = api::scope::ScopeProfile::from_kind(&kind).ok_or_else(|| {
                        error::AppError::Validation(format!(
                            "{kind} cannot be paired; use mac_app or phone"
                        ))
                    })?;
                    let label = label.unwrap_or_else(|| profile.default_label().to_owned());
                    (
                        json,
                        serde_json::json!({
                            "command": "pair",
                            "kind": profile.kind(),
                            "label": label,
                        }),
                    )
                }
                DeviceCommands::List { json } => (json, serde_json::json!({ "command": "list" })),
                DeviceCommands::Revoke { id, json } => (
                    json,
                    serde_json::json!({ "command": "revoke", "endpoint_id": id }),
                ),
            };
            let report = ops::admin_socket::request(&path, request).await?;
            let rendered = if report.get("code").is_some() {
                ops::device_pairing::render(&report)
            } else {
                serde_json::to_string_pretty(&report)?
            };
            emit_report(json, &report, rendered);
        }
        Commands::Models { command } => {
            let loaded = config::Config::load_operational()?;
            match command {
                ModelsCommands::Status { json, url, token } => {
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target.get("/voice/install/status").await?;
                    let status = response.status();
                    let body: serde_json::Value = response.json().await?;
                    emit_report(json, &body, serde_json::to_string_pretty(&body)?);
                    return Ok(if status.is_success() {
                        ops::exit::ExitCode::Success
                    } else {
                        ops::exit::ExitCode::Failed
                    });
                }
                ModelsCommands::Install { json, url, token } => {
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target.post("/voice/install/start").await?;
                    if !response.status().is_success()
                        && response.status() != reqwest::StatusCode::CONFLICT
                    {
                        return Err(error::AppError::Internal(format!(
                            "model install start failed with HTTP {}",
                            response.status()
                        ))
                        .into());
                    }
                    let snapshot = wait_for_model_install(&target).await?;
                    let completed = snapshot["status"] == "complete";
                    emit_report(json, &snapshot, serde_json::to_string_pretty(&snapshot)?);
                    return Ok(if completed {
                        ops::exit::ExitCode::Success
                    } else {
                        ops::exit::ExitCode::Failed
                    });
                }
                ModelsCommands::Cancel { json, url, token } => {
                    let target = ops::status::OpsTarget::resolve(
                        &loaded.config,
                        url.as_deref(),
                        token.as_deref(),
                    )
                    .map_err(error::AppError::Config)?;
                    let response = target.post("/voice/install/cancel").await?;
                    let report = serde_json::json!({
                        "cancel_requested": response.status().is_success()
                    });
                    emit_report(
                        json,
                        &report,
                        "Model install cancellation requested.".into(),
                    );
                    return Ok(if response.status().is_success() {
                        ops::exit::ExitCode::Success
                    } else {
                        ops::exit::ExitCode::Failed
                    });
                }
            }
        }
        #[allow(
            clippy::print_stdout,
            reason = "CLI service commands report operator results"
        )]
        Commands::Service { command } => {
            let loaded = config::Config::load_operational()?;
            let home = std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(std::path::PathBuf::from)
                .ok_or_else(|| error::AppError::Config("HOME is unavailable".into()))?;
            let service_paths = ops::service::paths(
                &home,
                loaded.paths.root(),
                loaded.config_file.parent().ok_or_else(|| {
                    error::AppError::Config("config file has no parent directory".into())
                })?,
            )?;
            match command {
                ServiceCommands::Install => {
                    let executable = std::env::current_exe()?.canonicalize()?;
                    let agent = agent_runtime_for_executable(&executable);
                    ops::service::install(&executable, agent.as_ref(), &service_paths).await?;
                    println!(
                        "BoBe service installed: {}",
                        service_paths.definition.display()
                    );
                }
                ServiceCommands::Status => {
                    let status = ops::service::status(&service_paths).await?;
                    println!(
                        "BoBe service: installed={}, running={} ({})",
                        status.installed,
                        status.running,
                        service_paths.definition.display()
                    );
                    return Ok(if status.installed && status.running {
                        ops::exit::ExitCode::Success
                    } else {
                        ops::exit::ExitCode::NotReady
                    });
                }
                ServiceCommands::Logs => {
                    let status = ops::service::status(&service_paths).await?;
                    if !status.installed || !status.manages_data_root {
                        return Err(error::AppError::Conflict(
                            "no installed service manages this BoBe data root".into(),
                        )
                        .into());
                    }
                    #[cfg(target_os = "macos")]
                    println!("{}", service_paths.logs.join("bobe").display());
                    #[cfg(all(unix, not(target_os = "macos")))]
                    println!("journalctl --user -t bobe --since today");
                }
                ServiceCommands::Restart => {
                    ops::service::restart(&service_paths).await?;
                    println!("BoBe service restarted.");
                }
                ServiceCommands::Uninstall => {
                    ops::service::uninstall(&service_paths).await?;
                    println!("BoBe service uninstalled.");
                }
            }
        }
        #[allow(
            clippy::print_stdout,
            reason = "backup commands report operator results"
        )]
        Commands::Backup { command } => {
            let loaded = config::Config::load_operational()?;
            match command {
                BackupCommands::Create => {
                    let live = ops::admin_socket::try_request(
                        &loaded.paths.admin_socket(),
                        serde_json::json!({ "command": "backup" }),
                    )
                    .await?;
                    if let Some(report) = live {
                        println!(
                            "{}",
                            report["path"].as_str().ok_or_else(|| {
                                error::AppError::Internal(
                                    "backup response did not include a path".into(),
                                )
                            })?
                        );
                    } else {
                        let paths = loaded.paths.prepare_root()?;
                        let _locks = util::runtime_lock::RuntimeLocks::acquire(&paths)?;
                        let (pool, identity) = bootstrap::initialize_storage(&paths, false).await?;
                        pool.close().await;
                        let backup = ops::backup::create(
                            &paths,
                            &loaded.config_file,
                            identity.runtime_authority_id,
                            identity.runtime_authority_epoch,
                            identity.schema_version,
                        )
                        .await?;
                        println!("{}", backup.display());
                    }
                }
                BackupCommands::List => {
                    for backup in ops::backup::list(&loaded.paths).await? {
                        println!("{}", backup.display());
                    }
                }
                BackupCommands::Verify { path } => {
                    let manifest =
                        ops::backup::verify(&path, bootstrap::database::CURRENT_SCHEMA_VERSION)
                            .await?;
                    println!(
                        "Backup verified: authority {} epoch {}, schema {}",
                        manifest.runtime_authority_id,
                        manifest.runtime_authority_epoch,
                        manifest.database_schema
                    );
                }
                BackupCommands::Restore { path } => {
                    let paths = loaded.paths.prepare_root()?;
                    ops::backup::verify_restore_topology(
                        &path,
                        &paths,
                        bootstrap::database::CURRENT_SCHEMA_VERSION,
                    )
                    .await?;
                    let home = std::env::var_os("HOME")
                        .filter(|value| !value.is_empty())
                        .map(std::path::PathBuf::from)
                        .ok_or_else(|| error::AppError::Config("HOME is unavailable".into()))?;
                    let service_paths = ops::service::paths(
                        &home,
                        paths.root(),
                        loaded.config_file.parent().ok_or_else(|| {
                            error::AppError::Config("config file has no parent directory".into())
                        })?,
                    )?;
                    let service_status = ops::service::status(&service_paths).await?;
                    let restart_service = service_status.installed
                        && service_status.running
                        && service_status.manages_data_root;
                    if restart_service {
                        ops::service::stop(&service_paths).await?;
                    }
                    let restore_result = async {
                        let locks = if restart_service {
                            util::runtime_lock::RuntimeLocks::acquire_after_service_stop(
                                &paths,
                                std::time::Duration::from_secs(15),
                            )
                            .await?
                        } else {
                            util::runtime_lock::RuntimeLocks::acquire(&paths)?
                        };
                        let outcome = ops::backup::restore(
                            &path,
                            &paths,
                            &loaded.config_file,
                            bootstrap::database::CURRENT_SCHEMA_VERSION,
                        )
                        .await?;
                        drop(locks);
                        Ok::<_, error::AppError>(outcome)
                    }
                    .await;
                    let outcome = match restore_result {
                        Ok(outcome) => outcome,
                        Err(restore_error) if restart_service => {
                            let restart = ops::service::start_installed(&service_paths).await;
                            return Err(match restart {
                                Ok(()) => restore_error.into(),
                                Err(restart_error) => error::AppError::Internal(format!(
                                    "restore failed: {restore_error}; previous service restart failed: {restart_error}"
                                ))
                                .into(),
                            });
                        }
                        Err(error) => return Err(error.into()),
                    };
                    if restart_service {
                        let restored = config::Config::load_operational()?;
                        if restored.paths.root() != paths.root() {
                            return Err(error::AppError::Conflict(
                                "restored configuration resolved to a different BoBe data root"
                                    .into(),
                            )
                            .into());
                        }
                        ops::service::start_installed(&service_paths).await?;
                        let target = ops::status::OpsTarget::resolve(&restored.config, None, None)
                            .map_err(error::AppError::Config)?;
                        let (exit, _) = ops::status::wait_ready_for(
                            &target,
                            std::time::Duration::from_mins(1),
                            std::time::Duration::from_millis(500),
                            ops::status::ReadinessLevel::Process,
                            Some(outcome.manifest.runtime_authority_id),
                            Some(outcome.restored_authority_epoch),
                            None,
                        )
                        .await;
                        if exit != ops::exit::ExitCode::Success {
                            return Err(error::AppError::Internal(
                                "restored service did not become ready with the restored authority epoch"
                                    .into(),
                            )
                            .into());
                        }
                    }
                    println!(
                        "Backup restored: authority {} advanced from epoch {} to {}{}",
                        outcome.manifest.runtime_authority_id,
                        outcome.manifest.runtime_authority_epoch,
                        outcome.restored_authority_epoch,
                        if outcome.cleanup_pending {
                            "; cleanup will resume on the next restore"
                        } else {
                            ""
                        }
                    );
                }
            }
        }
        #[allow(
            clippy::print_stdout,
            reason = "update command reports activation result"
        )]
        Commands::Update { command } => match command {
            UpdateCommands::Apply {
                manifest,
                signature,
                archive,
            } => {
                let loaded = config::Config::load_operational()?;
                let home = std::env::var_os("HOME")
                    .filter(|value| !value.is_empty())
                    .map(std::path::PathBuf::from)
                    .ok_or_else(|| error::AppError::Config("HOME is unavailable".into()))?;
                let current_executable = std::env::current_exe()?.canonicalize()?;
                let layout = ops::update::StandaloneLayout::discover(&current_executable)?;
                let versions = layout.versions_root.clone();
                std::fs::create_dir_all(&versions)?;
                let paths = loaded.paths.prepare_root()?;
                let service_paths = ops::service::paths(
                    &home,
                    paths.root(),
                    loaded.config_file.parent().ok_or_else(|| {
                        error::AppError::Config("config file has no parent directory".into())
                    })?,
                )?;
                let service_status = ops::service::status(&service_paths).await?;
                if service_status.installed && !service_status.manages_data_root {
                    return Err(error::AppError::Conflict(
                        "installed service does not manage this BoBe data root".into(),
                    )
                    .into());
                }
                let managed_service = service_status.installed && service_status.manages_data_root;
                if !managed_service {
                    let offline_check =
                        util::runtime_lock::RuntimeLocks::acquire(&paths).map_err(|_| {
                            error::AppError::Conflict(
                                "standalone updates require the unsupervised daemon to be stopped"
                                    .into(),
                            )
                        })?;
                    drop(offline_check);
                }
                let journal_path = versions.join("update-journal.json");
                if let Some(journal) =
                    ops::update::recover_staging_journal(&journal_path, &versions).await?
                {
                    if journal.managed_service {
                        ops::service::stop(&service_paths).await?;
                    }
                    if let Some(backup) = journal.backup {
                        let recovery_locks = if journal.managed_service {
                            util::runtime_lock::RuntimeLocks::acquire_after_service_stop(
                                &paths,
                                std::time::Duration::from_secs(15),
                            )
                            .await?
                        } else {
                            util::runtime_lock::RuntimeLocks::acquire(&paths)?
                        };
                        ops::backup::restore(
                            &backup,
                            &paths,
                            &loaded.config_file,
                            bootstrap::database::CURRENT_SCHEMA_VERSION,
                        )
                        .await?;
                        drop(recovery_locks);
                    }
                    ops::update::rollback(&versions, &journal.previous)?;
                    let root = versions.join(&journal.previous);
                    let executable = root.join("bin/bobe");
                    let agent = agent_runtime_for_executable(&executable);
                    if journal.managed_service {
                        ops::service::install(&executable, agent.as_ref(), &service_paths).await?;
                        let target = ops::status::OpsTarget::resolve(&loaded.config, None, None)
                            .map_err(error::AppError::Config)?;
                        let (recovery_exit, _) = ops::status::wait_ready_for(
                            &target,
                            std::time::Duration::from_mins(1),
                            std::time::Duration::from_millis(500),
                            ops::status::ReadinessLevel::Process,
                            journal.authority_id,
                            None,
                            Some(journal.previous_version.as_str()),
                        )
                        .await;
                        if recovery_exit != ops::exit::ExitCode::Success {
                            return Err(error::AppError::Internal(
                                "recovered previous service did not become ready".into(),
                            )
                            .into());
                        }
                    }
                    ops::update::discard_candidate(&versions, &journal.candidate).await?;
                    ops::update::clear_journal(&journal_path).await?;
                }
                let manifest_bytes = std::fs::read(&manifest)?;
                let signature = std::fs::read_to_string(signature)?;
                let platform = ops::update::current_platform();
                let database = paths.database_file().ok_or_else(|| {
                    error::AppError::Config(
                        "standalone updates require a file-backed SQLite database".into(),
                    )
                })?;
                let current_database_schema =
                    bootstrap::database::read_schema_version(&database).await?;
                let update = ops::update::verify_manifest(
                    &manifest_bytes,
                    signature.trim(),
                    platform,
                    current_database_schema,
                )?;
                ops::update::ensure_candidate_available(&versions, &update.version).await?;
                let current_agent = agent_runtime_for_executable(&current_executable);
                let active_previous = ops::update::seed_previous_version(
                    &versions,
                    &current_executable,
                    current_agent.as_ref(),
                    env!("CARGO_PKG_VERSION"),
                )
                .await?;
                let previous_version = ops::update::version_for_target(
                    &versions,
                    &active_previous,
                    env!("CARGO_PKG_VERSION"),
                )?;
                ops::update::install_launcher(&layout)?;
                ops::update::write_journal(
                    &journal_path,
                    &ops::update::UpdateJournal {
                        phase: "staging".into(),
                        previous: active_previous.clone(),
                        previous_version: previous_version.clone(),
                        candidate: update.version.clone(),
                        backup: None,
                        authority_id: None,
                        managed_service,
                    },
                )
                .await?;
                let staged = match ops::update::stage_archive(&update, &archive, &versions).await {
                    Ok(staged) => staged,
                    Err(stage_error) => {
                        ops::update::discard_candidate(&versions, &update.version).await?;
                        ops::update::clear_journal(&journal_path).await?;
                        return Err(stage_error.into());
                    }
                };
                ops::update::write_journal(
                    &journal_path,
                    &ops::update::UpdateJournal {
                        phase: "stopping".into(),
                        previous: active_previous.clone(),
                        previous_version: previous_version.clone(),
                        candidate: update.version.clone(),
                        backup: None,
                        authority_id: None,
                        managed_service,
                    },
                )
                .await?;
                if managed_service && service_status.running {
                    ops::service::stop(&service_paths).await?;
                }
                let post_stop = async {
                    let locks = if managed_service && service_status.running {
                        util::runtime_lock::RuntimeLocks::acquire_after_service_stop(
                            &paths,
                            std::time::Duration::from_secs(15),
                        )
                        .await?
                    } else {
                        util::runtime_lock::RuntimeLocks::acquire(&paths)?
                    };
                    let (pool, identity) = bootstrap::initialize_storage(&paths, false).await?;
                    pool.close().await;
                    if identity.schema_version != update.database_schema {
                        return Err(error::AppError::Conflict(
                            "live database is incompatible with the candidate update".into(),
                        ));
                    }
                    let backup = ops::backup::create(
                        &paths,
                        &loaded.config_file,
                        identity.runtime_authority_id,
                        identity.runtime_authority_epoch,
                        identity.schema_version,
                    )
                    .await?;
                    ops::backup::verify(&backup, bootstrap::database::CURRENT_SCHEMA_VERSION)
                        .await?;
                    ops::update::write_journal(
                        &journal_path,
                        &ops::update::UpdateJournal {
                            phase: "backup_verified".into(),
                            previous: active_previous.clone(),
                            previous_version: previous_version.clone(),
                            candidate: update.version.clone(),
                            backup: Some(backup.clone()),
                            authority_id: Some(identity.runtime_authority_id),
                            managed_service,
                        },
                    )
                    .await?;
                    drop(locks);
                    let previous = ops::update::activate(&versions, &update.version)?
                        .unwrap_or(active_previous.clone());
                    ops::update::write_journal(
                        &journal_path,
                        &ops::update::UpdateJournal {
                            phase: "activated".into(),
                            previous: previous.clone(),
                            previous_version: previous_version.clone(),
                            candidate: update.version.clone(),
                            backup: Some(backup.clone()),
                            authority_id: Some(identity.runtime_authority_id),
                            managed_service,
                        },
                    )
                    .await?;
                    Ok::<_, error::AppError>((backup, previous, identity.runtime_authority_id))
                }
                .await;
                let (backup, previous, authority_id) = match post_stop {
                    Ok(value) => value,
                    Err(error) => {
                        if !managed_service {
                            return Err(error.into());
                        }
                        let previous_root = versions.join(&active_previous);
                        let previous_executable = previous_root.join("bin/bobe");
                        let previous_agent = agent_runtime_for_executable(&previous_executable);
                        let restart = ops::service::install(
                            &previous_executable,
                            previous_agent.as_ref(),
                            &service_paths,
                        )
                        .await;
                        return Err(match restart {
                            Ok(()) => error.into(),
                            Err(restart_error) => error::AppError::Internal(format!(
                                "update preparation failed: {error}; previous service restart failed: {restart_error}"
                            ))
                            .into(),
                        });
                    }
                };
                let executable = staged.join("bin/bobe");
                let agent = agent_runtime_for_executable(&executable);
                let activation = async {
                    if !managed_service {
                        return Ok::<(), error::AppError>(());
                    }
                    ops::service::install(&executable, agent.as_ref(), &service_paths).await?;
                    let target = ops::status::OpsTarget::resolve(&loaded.config, None, None)
                        .map_err(error::AppError::Config)?;
                    let (exit, _) = ops::status::wait_ready_for(
                        &target,
                        std::time::Duration::from_mins(1),
                        std::time::Duration::from_millis(500),
                        ops::status::ReadinessLevel::Process,
                        Some(authority_id),
                        None,
                        Some(&update.version),
                    )
                    .await;
                    if exit != ops::exit::ExitCode::Success {
                        return Err(error::AppError::Internal(
                            "updated service did not become ready".into(),
                        ));
                    }
                    Ok::<(), error::AppError>(())
                }
                .await;
                if let Err(activation_error) = activation {
                    let rollback_result = async {
                        ops::service::stop(&service_paths).await?;
                        let rollback_locks =
                            util::runtime_lock::RuntimeLocks::acquire_after_service_stop(
                                &paths,
                                std::time::Duration::from_secs(15),
                            )
                            .await?;
                        ops::backup::restore(
                            &backup,
                            &paths,
                            &loaded.config_file,
                            bootstrap::database::CURRENT_SCHEMA_VERSION,
                        )
                        .await?;
                        drop(rollback_locks);
                        ops::update::rollback(&versions, &previous)?;
                        let previous_root = versions.join(&previous);
                        let previous_executable = previous_root.join("bin/bobe");
                        let previous_agent = agent_runtime_for_executable(&previous_executable);
                        ops::service::install(
                            &previous_executable,
                            previous_agent.as_ref(),
                            &service_paths,
                        )
                        .await?;
                        let target = ops::status::OpsTarget::resolve(&loaded.config, None, None)
                            .map_err(error::AppError::Config)?;
                        let (exit, _) = ops::status::wait_ready_for(
                            &target,
                            std::time::Duration::from_mins(1),
                            std::time::Duration::from_millis(500),
                            ops::status::ReadinessLevel::Process,
                            Some(authority_id),
                            None,
                            Some(&previous_version),
                        )
                        .await;
                        if exit != ops::exit::ExitCode::Success {
                            return Err(error::AppError::Internal(
                                "previous service did not become ready after rollback".into(),
                            ));
                        }
                        Ok::<(), error::AppError>(())
                    }
                    .await;
                    return Err(match rollback_result {
                        Ok(()) => {
                            ops::update::clear_journal(&journal_path).await?;
                            activation_error.into()
                        }
                        Err(rollback_error) => error::AppError::Internal(format!(
                            "update failed: {activation_error}; rollback failed: {rollback_error}; backup: {}",
                            backup.display()
                        ))
                        .into(),
                    });
                }
                println!("Activated BoBe standalone {}", update.version);
                ops::update::clear_journal(&journal_path).await?;
            }
        },
        #[allow(clippy::print_stdout)]
        Commands::Version { json } => {
            if json {
                let report = serde_json::json!({
                    "schema": 1,
                    "version": env!("CARGO_PKG_VERSION"),
                    "agent_harness": "pi",
                    "agent_version": "1.0.3",
                    "database_schema": bootstrap::database::CURRENT_SCHEMA_VERSION,
                    "attach_protocol_min": constants::attach::PROTOCOL_MIN,
                    "attach_protocol_max": constants::attach::PROTOCOL_MAX,
                });
                emit_report(true, &report, String::new());
            } else {
                println!("BoBe v{}", env!("CARGO_PKG_VERSION"));
            }
        }
    }

    Ok(ops::exit::ExitCode::Success)
}

fn agent_runtime_for_executable(executable: &std::path::Path) -> Option<agent::RuntimeFiles> {
    agent::RuntimeFiles::installed(executable)
}

fn remote_config_changes(
    loaded: &config::LoadedConfig,
    host: String,
    allowed_hosts: Vec<String>,
    tls_cert: Option<&std::path::Path>,
    tls_key: Option<&std::path::Path>,
) -> Result<std::collections::BTreeMap<String, serde_json::Value>, error::AppError> {
    host.parse::<std::net::IpAddr>().map_err(|error| {
        error::AppError::Validation(format!("remote host must be an IP address: {error}"))
    })?;
    if allowed_hosts.iter().any(|host| host.trim().is_empty()) {
        return Err(error::AppError::Validation(
            "allowed hosts must not be empty".into(),
        ));
    }
    let mut changes = std::collections::BTreeMap::from([
        ("server.host".into(), serde_json::Value::String(host)),
        (
            "server.allowed_hosts".into(),
            serde_json::to_value(allowed_hosts)?,
        ),
    ]);
    match (tls_cert, tls_key) {
        (None, None) => {
            changes.insert("server.tls_cert_path".into(), serde_json::Value::Null);
            changes.insert("server.tls_key_path".into(), serde_json::Value::Null);
        }
        (Some(tls_cert), Some(tls_key)) => {
            let config_root = loaded.config_file.parent().ok_or_else(|| {
                error::AppError::Config("config file has no parent directory".into())
            })?;
            let tls_cert = normalize_config_path(tls_cert, config_root, "TLS certificate")?;
            let tls_key = normalize_config_path(tls_key, config_root, "TLS private key")?;
            if !tls_cert.is_file() {
                return Err(error::AppError::Validation(format!(
                    "TLS certificate does not exist: {}",
                    tls_cert.display()
                )));
            }
            if !tls_key.is_file() {
                return Err(error::AppError::Validation(format!(
                    "TLS private key does not exist: {}",
                    tls_key.display()
                )));
            }
            changes.insert(
                "server.tls_cert_path".into(),
                serde_json::Value::String(tls_cert.to_string_lossy().into_owned()),
            );
            changes.insert(
                "server.tls_key_path".into(),
                serde_json::Value::String(tls_key.to_string_lossy().into_owned()),
            );
        }
        _ => {
            return Err(error::AppError::Validation(
                "--tls-cert and --tls-key must be provided together".into(),
            ));
        }
    }
    Ok(changes)
}

fn normalize_config_path(
    path: &std::path::Path,
    config_root: &std::path::Path,
    label: &str,
) -> Result<std::path::PathBuf, error::AppError> {
    let expanded = crate::util::paths::expand_tilde(&path.to_string_lossy());
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        config_root.join(expanded)
    };
    absolute.canonicalize().map_err(|error| {
        error::AppError::Validation(format!(
            "{label} could not be resolved: {} ({error})",
            absolute.display()
        ))
    })
}

async fn response_json(
    response: reqwest::Response,
) -> anyhow::Result<(reqwest::StatusCode, serde_json::Value)> {
    let status = response.status();
    let body = response.text().await?;
    let report = serde_json::from_str(&body).unwrap_or_else(|_| {
        serde_json::json!({
            "error": {
                "code": "HTTP_ERROR",
                "message": if body.is_empty() {
                    format!("daemon returned HTTP {status}")
                } else {
                    body
                }
            }
        })
    });
    Ok((status, report))
}

fn http_exit(status: reqwest::StatusCode) -> ops::exit::ExitCode {
    if status.is_success() {
        ops::exit::ExitCode::Success
    } else if status.is_client_error() {
        ops::exit::ExitCode::Invalid
    } else {
        ops::exit::ExitCode::Failed
    }
}

fn render_provider_mutation(action: &str, report: &serde_json::Value) -> String {
    if report["updated"].as_bool() == Some(true) {
        format!(
            "{action}.{}",
            if report["restart_required"].as_bool() == Some(true) {
                " Restart the BoBe daemon to activate this change"
            } else {
                ""
            }
        )
    } else {
        serde_json::to_string_pretty(report).unwrap_or_else(|_| format!("{action} failed."))
    }
}

fn read_secret_stdin() -> Result<String, error::AppError> {
    use std::io::Read;

    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(error::AppError::Validation(
            "provider credential must be piped through standard input".into(),
        ));
    }
    let mut value = String::new();
    std::io::stdin().read_to_string(&mut value)?;
    let value = value.trim().to_owned();
    if value.is_empty() || value.len() > 1024 {
        return Err(error::AppError::Validation(
            "provider API key must be 1-1024 characters".into(),
        ));
    }
    Ok(value)
}

fn read_routes_input(
    path: Option<&std::path::Path>,
    stdin_requested: bool,
) -> Result<serde_json::Value, error::AppError> {
    use std::io::Read;

    let input = if let Some(path) = path {
        std::fs::read_to_string(path)?
    } else if stdin_requested || !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        input
    } else {
        return Err(error::AppError::Validation(
            "provider routes require --file PATH or JSON on standard input".into(),
        ));
    };
    let routes: provider::ProviderRoutes = serde_json::from_str(&input).map_err(|error| {
        error::AppError::Validation(format!("invalid provider routes JSON: {error}"))
    })?;
    serde_json::to_value(routes).map_err(error::AppError::from)
}

async fn unset_live_config(
    config: &config::Config,
    url: Option<&str>,
    token: Option<&str>,
    key: &str,
) -> anyhow::Result<serde_json::Value> {
    let target =
        ops::status::OpsTarget::resolve(config, url, token).map_err(error::AppError::Config)?;
    let response = target.delete(&format!("/config/{key}")).await?;
    if !response.status().is_success() {
        return Err(error::AppError::Internal(format!(
            "daemon rejected configuration removal with HTTP {}",
            response.status()
        ))
        .into());
    }
    let mut report: serde_json::Value = response.json().await?;
    report["key"] = serde_json::Value::String(key.to_owned());
    report["source"] = serde_json::Value::String("daemon".into());
    Ok(report)
}

fn remove_offline_config(loaded: &config::LoadedConfig, key: &str) -> Result<(), error::AppError> {
    let prepared_paths = loaded.paths.clone().prepare_root()?;
    let _locks = util::runtime_lock::RuntimeLocks::acquire(&prepared_paths)?;
    let config_root = loaded
        .config_file
        .parent()
        .ok_or_else(|| error::AppError::Config("config file has no parent directory".into()))?;
    if config::manager_persistence::remove(config_root, key) {
        Ok(())
    } else {
        Err(error::AppError::Io(std::io::Error::other(
            "failed to remove persisted configuration override",
        )))
    }
}

fn unset_config_report(
    loaded: &config::LoadedConfig,
    key: &str,
) -> Result<serde_json::Value, error::AppError> {
    let config_root = loaded
        .config_file
        .parent()
        .ok_or_else(|| error::AppError::Config("config file has no parent directory".into()))?;
    let reloaded = config::Config::load_from(&loaded.config_file, config_root)?;
    let value = config_value(&reloaded.config, key)
        .ok_or_else(|| error::AppError::Config(format!("unknown configuration key: {key}")))?;
    Ok(serde_json::json!({
        "updated": true,
        "key": key,
        "value": value,
        "source": config_value_source(&reloaded, key)?,
    }))
}

async fn update_live_config(
    config: &config::Config,
    url: Option<&str>,
    token: Option<&str>,
    key: &str,
    value: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let target =
        ops::status::OpsTarget::resolve(config, url, token).map_err(error::AppError::Config)?;
    let response = target
        .patch_json(
            "/config",
            &serde_json::json!({"changes": {key.to_owned(): value.clone()}}),
        )
        .await?;
    if !response.status().is_success() {
        return Err(error::AppError::Internal(format!(
            "daemon rejected configuration update with HTTP {}",
            response.status()
        ))
        .into());
    }
    let mut report: serde_json::Value = response.json().await?;
    report["key"] = serde_json::Value::String(key.to_owned());
    report["value"] = value.clone();
    report["source"] = serde_json::Value::String("daemon".into());
    Ok(report)
}

fn persist_offline_config(
    loaded: &config::LoadedConfig,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), error::AppError> {
    let prepared_paths = loaded.paths.clone().prepare_root()?;
    let _locks = util::runtime_lock::RuntimeLocks::acquire(&prepared_paths)?;
    let config_root = loaded
        .config_file
        .parent()
        .ok_or_else(|| error::AppError::Config("config file has no parent directory".into()))?;
    if config::manager_persistence::persist(
        config_root,
        &std::collections::BTreeMap::from([(key.to_owned(), value.clone())]),
    ) {
        Ok(())
    } else {
        Err(error::AppError::Io(std::io::Error::other(
            "failed to persist configuration",
        )))
    }
}

async fn wait_for_model_install(
    target: &ops::status::OpsTarget,
) -> anyhow::Result<serde_json::Value> {
    loop {
        let response = target.get("/voice/install/status").await?;
        let snapshot: serde_json::Value = response.json().await?;
        if matches!(
            snapshot["status"].as_str(),
            Some("complete" | "failed" | "canceled")
        ) {
            return Ok(snapshot);
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

async fn wait_for_provider_login(
    target: &ops::status::OpsTarget,
    profile: &str,
) -> anyhow::Result<serde_json::Value> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_mins(15);
    let mut last_phase = String::new();
    loop {
        if tokio::time::Instant::now() >= deadline {
            let response = target
                .post(&format!("/auth/{profile}/login/cancel"))
                .await?;
            if !response.status().is_success() {
                anyhow::bail!(
                    "provider login timed out and cancellation failed: HTTP {}",
                    response.status()
                );
            }
            return Ok(serde_json::json!({
                "phase": "failed",
                "message": "Provider login timed out after 15 minutes"
            }));
        }
        let response = target.get(&format!("/auth/{profile}/login/status")).await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            anyhow::bail!(
                "provider login is no longer active; check authentication status before retrying"
            );
        }
        if !response.status().is_success() {
            anyhow::bail!("provider login status failed: HTTP {}", response.status());
        }
        let typed_phase: agent::auth::LoginPhase = response.json().await?;
        let phase = serde_json::to_value(typed_phase)?;
        let name = phase["phase"].as_str().unwrap_or("unknown");
        if name != last_phase {
            print_login_phase(&phase);
            if name == "awaiting_input" {
                let password = phase["password"].as_bool().unwrap_or(false);
                let value = tokio::task::spawn_blocking(move || read_provider_login_line(password))
                    .await??;
                let response = target
                    .post_json(
                        &format!("/auth/{profile}/login/submit"),
                        &serde_json::json!({"value":value}),
                    )
                    .await?;
                if !response.status().is_success() {
                    anyhow::bail!(
                        "provider login input was rejected: HTTP {}",
                        response.status()
                    );
                }
            }
            last_phase = name.to_owned();
        }
        if matches!(name, "completed" | "failed" | "canceled") {
            return Ok(phase);
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

#[allow(unsafe_code)]
fn read_provider_login_line(password: bool) -> Result<String, error::AppError> {
    use std::io::{BufRead, IsTerminal, Read};
    #[cfg(unix)]
    struct EchoGuard {
        fd: std::os::fd::RawFd,
        original: libc::termios,
    }
    #[cfg(unix)]
    impl Drop for EchoGuard {
        fn drop(&mut self) {
            if unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &raw const self.original) } != 0 {
                tracing::error!(error = %std::io::Error::last_os_error(), "provider.login_terminal_restore_failed");
            }
        }
    }
    #[cfg(unix)]
    let _echo = if password && std::io::stdin().is_terminal() {
        use std::os::fd::AsRawFd;
        let fd = std::io::stdin().as_raw_fd();
        let mut attributes = std::mem::MaybeUninit::<libc::termios>::uninit();
        if unsafe { libc::tcgetattr(fd, attributes.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let original = unsafe { attributes.assume_init() };
        let mut hidden = original;
        hidden.c_lflag &= !libc::ECHO;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw const hidden) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Some(EchoGuard { fd, original })
    } else {
        None
    };
    #[cfg(not(unix))]
    if password && std::io::stdin().is_terminal() {
        return Err(error::AppError::Validation(
            "private login input must be piped through stdin".into(),
        ));
    }
    let mut bytes = Vec::new();
    let read = std::io::stdin()
        .lock()
        .take(16 * 1024 + 1)
        .read_until(b'\n', &mut bytes)?;
    if read == 0 {
        return Err(error::AppError::Validation(
            "provider login needs interactive input; stdin closed".into(),
        ));
    }
    let value = String::from_utf8(bytes)
        .map_err(|_| error::AppError::Validation("provider login input must be UTF-8".into()))?;
    let value = value.trim().to_owned();
    if value.len() > 16 * 1024 {
        return Err(error::AppError::Validation(
            "provider login input must not exceed 16384 bytes".into(),
        ));
    }
    Ok(value)
}

#[allow(clippy::print_stderr)]
fn print_login_phase(phase: &serde_json::Value) {
    match phase["phase"].as_str() {
        Some("preparing") => eprintln!("Preparing provider sign-in..."),
        Some("awaiting_user" | "polling") => eprintln!(
            "Open {} and enter code {}",
            phase["url"].as_str().unwrap_or("the device sign-in page"),
            phase["code"].as_str().unwrap_or("(unavailable)")
        ),
        Some("awaiting_input") => eprintln!(
            "{}",
            phase["message"]
                .as_str()
                .unwrap_or("Enter the provider authorization response:")
        ),
        Some("completed") => eprintln!("Provider sign-in completed."),
        Some("failed") => eprintln!(
            "Provider sign-in failed: {}",
            phase["message"].as_str().unwrap_or("unknown error")
        ),
        Some("canceled") => eprintln!("Provider sign-in canceled."),
        _ => {}
    }
}

fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn config_environment_key(key: &str) -> String {
    format!("BOBE_{}", key.replace('.', "__").to_ascii_uppercase())
}

fn config_value_source(
    loaded: &config::LoadedConfig,
    key: &str,
) -> Result<&'static str, error::AppError> {
    if std::env::var_os(config_environment_key(key)).is_some() {
        return Ok("environment");
    }
    let source = match std::fs::read_to_string(&loaded.config_file) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok("default"),
        Err(error) => return Err(error.into()),
    };
    let doc: toml_edit::DocumentMut = source
        .parse()
        .map_err(|error: toml_edit::TomlError| error::AppError::Config(error.to_string()))?;
    let parts: Vec<&str> = key.split('.').collect();
    let persisted = if let Some((field, sections)) = parts.split_last() {
        let mut table = doc.as_table();
        let mut valid_path = true;
        for section in sections {
            let Some(next) = table.get(section).and_then(toml_edit::Item::as_table) else {
                valid_path = false;
                break;
            };
            table = next;
        }
        valid_path && table.contains_key(field)
    } else {
        false
    };
    Ok(if persisted { "config_file" } else { "default" })
}

fn config_default_value(loaded: &config::LoadedConfig, key: &str) -> serde_json::Value {
    let bootstrap = loaded
        .config_file
        .parent()
        .map_or_else(String::new, |path| path.to_string_lossy().into_owned());
    let defaults = config::Config {
        data_dir: bootstrap,
        database: config::DatabaseConfig { url: String::new() },
        ..config::Config::default()
    };
    config_value(&defaults, key).unwrap_or(serde_json::Value::Null)
}

fn config_value(config: &config::Config, key: &str) -> Option<serde_json::Value> {
    if !config::manager_fields::is_known_key(key) {
        return None;
    }
    config::manager_fields::value(config, key)
}

fn parse_duration(value: &str) -> Result<std::time::Duration, String> {
    let (number, unit) = value
        .find(|character: char| !character.is_ascii_digit())
        .map_or((value, "s"), |index| value.split_at(index));
    let number: u64 = number
        .parse()
        .map_err(|_| "duration must start with an integer".to_string())?;
    match unit {
        "ms" => Ok(std::time::Duration::from_millis(number)),
        "s" => Ok(std::time::Duration::from_secs(number)),
        "m" => Ok(std::time::Duration::from_secs(number.saturating_mul(60))),
        _ => Err("duration unit must be ms, s, or m".into()),
    }
}

#[allow(clippy::print_stdout)]
fn emit_report<T: serde::Serialize>(json: bool, report: &T, human: String) {
    if json {
        match serde_json::to_string_pretty(report) {
            Ok(serialized) => println!("{serialized}"),
            Err(error) => println!("{{\"error\":\"{error}\"}}"),
        }
    } else {
        println!("{human}");
    }
}

fn render_status(report: &ops::status::StatusReport) -> String {
    if let Some(readiness) = &report.readiness {
        if readiness.status == "ready" {
            return format!(
                "BoBe is ready at {}\nAuthority: {} epoch {}\nIncarnation: {}\nDaemon: {}  Schema: {}",
                report.target,
                readiness.runtime_authority_id,
                readiness.runtime_authority_epoch,
                readiness.runtime_incarnation_id,
                readiness.daemon_version,
                readiness.schema_version
            );
        }
        return format!(
            "BoBe is not ready at {}\nBlocking: {}",
            report.target,
            readiness.blocking_reasons.join(", ")
        );
    }
    format!(
        "BoBe is not reachable at {}{}",
        report.target,
        report
            .error
            .as_deref()
            .map_or_else(String::new, |error| format!("\n{error}"))
    )
}

fn render_config_paths(report: &ops::config_report::ConfigPathsReport) -> String {
    format!(
        "Config file: {}{}\nData root: {}\nDatabase: {}\nModels: {}",
        report.config_file.display(),
        if report.config_file_exists {
            ""
        } else {
            " (not created)"
        },
        report.data_root.display(),
        report.database_url,
        report.models_dir.display()
    )
}

fn render_validation(report: &ops::config_report::ValidationReport) -> String {
    if report.valid && report.findings.is_empty() {
        return "Configuration is valid.".into();
    }
    let mut lines = vec![if report.valid {
        "Configuration is valid with warnings:".to_string()
    } else {
        "Configuration is invalid:".to_string()
    }];
    lines.extend(report.findings.iter().map(|finding| {
        format!(
            "- [{}] {}: {}",
            finding.severity, finding.id, finding.message
        )
    }));
    lines.join("\n")
}

fn render_doctor(report: &serde_json::Value) -> String {
    let config_valid = report["config"]["valid"].as_bool().unwrap_or(false);
    let runtime_ready = report["runtime"]["readiness"]["status"] == "ready";
    let provider = match report["provider"]["ok"].as_bool() {
        Some(true) => "model execution, streaming, and diagnostic tool passed".to_owned(),
        Some(false) => format!(
            "failed — {}",
            report["provider"]["error"]
                .as_str()
                .unwrap_or("provider diagnostic failed")
        ),
        None => "not tested — run bobe doctor --probe (may incur provider charges)".to_owned(),
    };
    format!(
        "BoBe doctor\n- Configuration: {}\n- Runtime: {}\n- Provider: {}",
        if config_valid { "ok" } else { "error" },
        if runtime_ready { "ready" } else { "not ready" },
        provider,
    )
}

fn server_socket_addr(config: &config::Config) -> anyhow::Result<std::net::SocketAddr> {
    let ip = config.server.host.parse::<std::net::IpAddr>()?;
    Ok(std::net::SocketAddr::new(ip, config.server.port))
}

fn validate_server_security(config: &config::Config) -> anyhow::Result<()> {
    let host = config.server.host.parse::<std::net::IpAddr>()?;
    let exposes_public_host = !config.server.allowed_hosts.is_empty();
    if host.is_loopback() && !exposes_public_host {
        return Ok(());
    }
    anyhow::ensure!(
        !config.server.allowed_hosts.is_empty(),
        "non-loopback bind requires at least one durable server.allowed_hosts entry"
    );
    anyhow::ensure!(
        config.server.tls_cert_path.is_some() == config.server.tls_key_path.is_some(),
        "manual TLS configuration requires both server.tls_cert_path and server.tls_key_path"
    );
    Ok(())
}

const MANAGING_APP_POLL: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(unix)]
fn managing_process_id() -> u32 {
    std::os::unix::process::parent_id()
}

#[cfg(not(unix))]
fn managing_process_id() -> u32 {
    0
}

fn managing_app_is_gone(started_by: u32, parent_now: u32) -> bool {
    started_by <= 1 || parent_now != started_by
}

async fn wait_for_managing_app_exit(started_by: Option<u32>) {
    let Some(started_by) = started_by.filter(|_| cfg!(unix)) else {
        return std::future::pending().await;
    };
    let mut poll = tokio::time::interval(MANAGING_APP_POLL);
    loop {
        poll.tick().await;
        if managing_app_is_gone(started_by, managing_process_id()) {
            return;
        }
    }
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(error) => {
                    tracing::warn!(%error, "failed to install SIGTERM handler; waiting for Ctrl-C");
                    if let Err(error) = tokio::signal::ctrl_c().await {
                        tracing::error!(%error, "failed to wait for Ctrl-C");
                    }
                    return;
                }
            };
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    tracing::error!(%error, "failed to wait for Ctrl-C");
                }
            }
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to wait for Ctrl-C");
    }
}

fn spawn_background_tasks(
    state: &std::sync::Arc<app_state::AppState>,
    shutdown: &tokio_util::sync::CancellationToken,
) -> tokio::task::JoinSet<&'static str> {
    let mut set = tokio::task::JoinSet::new();
    {
        let background = Arc::clone(&state.runtime.background);
        let token = shutdown.clone();
        set.spawn(async move {
            background.run(token).await;
            "background_work"
        });
    }

    {
        let state = Arc::clone(state);
        let token = shutdown.clone();
        set.spawn(async move {
            if let Err(error) = ops::admin_socket::serve(state, token).await {
                tracing::error!(%error, "admin.socket_failed");
            }
            "admin_socket"
        });
    }

    {
        let session = std::sync::Arc::clone(&state.runtime.runtime_session);
        let initialization = Arc::clone(&state.infra.initialization);
        let token = shutdown.clone();
        set.spawn(async move {
            if !wait_for_initialization(&initialization, &token).await {
                return "runtime_session";
            }
            tokio::select! {
                () = session.run() => {}
                () = token.cancelled() => {
                    session.stop().await;
                }
            }
            "runtime_session"
        });
    }

    if state.config().memory.enabled {
        let scheduler = runtime::consolidation::ConsolidationScheduler::new(
            std::sync::Arc::clone(&state.runtime.workers),
            std::sync::Arc::clone(&state.runtime.memory_file),
            std::sync::Arc::clone(&state.runtime.personal_data_lifecycle),
        );
        let token = shutdown.clone();
        let initialization = Arc::clone(&state.infra.initialization);
        set.spawn(async move {
            if wait_for_initialization(&initialization, &token).await {
                scheduler.run(token).await;
            }
            "consolidation"
        });
    }

    {
        let pool = state.infra.db.clone();
        let config = std::sync::Arc::clone(&state.infra.config);
        let token = shutdown.clone();
        set.spawn(async move {
            loop {
                crate::services::storage_retention::run(&pool, &config.load()).await;
                tokio::select! {
                    () = tokio::time::sleep(crate::services::storage_retention::RETENTION_INTERVAL) => {}
                    () = token.cancelled() => break,
                }
            }
            "storage_retention"
        });
    }

    {
        let body_config = state.config().body.clone();
        if body_config.enabled {
            let gateway = Arc::clone(&state.body.gateway);
            let identities = Arc::clone(&state.body.identities);
            let endpoints = Arc::clone(&state.body.endpoints);
            let registry_lock = Arc::clone(&state.body.registry_lock);
            let status_gateway = Arc::clone(&gateway);
            let token = shutdown.clone();
            set.spawn(async move {
                if let Err(error) = body::server::run(
                    gateway,
                    identities,
                    endpoints,
                    registry_lock,
                    body_config,
                    token,
                )
                .await
                {
                    status_gateway.mark_failed();
                    tracing::error!(%error, "body.gateway_failed");
                }
                "body_gateway"
            });
        }
    }

    set
}

async fn wait_for_initialization(
    initialization: &arc_swap::ArcSwap<db::runtime_initialization_repo::InitializationState>,
    shutdown: &tokio_util::sync::CancellationToken,
) -> bool {
    loop {
        if shutdown.is_cancelled() {
            return false;
        }
        if **initialization.load() == db::runtime_initialization_repo::InitializationState::Ready {
            return true;
        }
        tokio::select! {
            () = shutdown.cancelled() => return false,
            () = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
        }
    }
}

async fn drain_background_tasks(tasks: &mut tokio::task::JoinSet<&'static str>) {
    const DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
    let deadline = tokio::time::Instant::now() + DRAIN_TIMEOUT;

    while !tasks.is_empty() {
        match tokio::time::timeout_at(deadline, tasks.join_next()).await {
            Ok(Some(Ok(name))) => tracing::info!(task = name, "background_task.stopped"),
            Ok(Some(Err(e))) => tracing::error!(error = %e, "background task panicked"),
            Ok(None) => break,
            Err(_) => {
                let remaining = tasks.len();
                tracing::warn!(remaining, "background task drain timeout, abandoning");
                tasks.abort_all();
                while let Some(result) = tasks.join_next().await {
                    if let Err(error) = result
                        && !error.is_cancelled()
                    {
                        tracing::error!(%error,"background_task.failed_during_abort");
                    }
                }
                return;
            }
        }
    }
}

async fn drain_in_flight_text_turns(counter: &std::sync::atomic::AtomicUsize) {
    use std::sync::atomic::Ordering;
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);
    const DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
    let start = tokio::time::Instant::now();
    loop {
        let n = counter.load(Ordering::Acquire);
        if n == 0 {
            return;
        }
        if start.elapsed() >= DEADLINE {
            tracing::warn!(in_flight = n, "text_turn_drain_timeout");
            return;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn run_graceful_shutdown(state: &std::sync::Arc<app_state::AppState>) {
    tracing::info!("Stopping mDNS...");
    state.infra.mdns_announcer.stop().await;

    tracing::info!("Signaling SSE clients to disconnect...");
    state.infra.connection_manager.disconnect_all();

    tracing::info!("Cancelling in-flight installs...");
    state.voice.voice_install.cancel().await;
    state.voice.voice_install.await_idle().await;

    tracing::info!("Stopping pi agent...");
    state.runtime.workers.shutdown_all().await;

    drain_in_flight_text_turns(&state.runtime.in_flight_text_turns).await;

    tracing::info!("Closing database pool...");
    state.infra.db.close().await;

    tracing::info!("BoBe shutdown complete");
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests panic on invalid CLI fixtures")]
mod tests {
    #[test]
    fn a_managed_daemon_stops_once_its_app_is_no_longer_its_parent() {
        assert!(!super::managing_app_is_gone(4_242, 4_242));
        assert!(super::managing_app_is_gone(4_242, 1));
        assert!(super::managing_app_is_gone(4_242, 9_000));
        assert!(super::managing_app_is_gone(1, 1));
    }

    #[tokio::test]
    async fn an_unmanaged_daemon_never_watches_a_parent() {
        let watch = super::wait_for_managing_app_exit(None);
        let waited = tokio::time::timeout(std::time::Duration::from_millis(50), watch).await;
        assert!(waited.is_err());
    }

    #[tokio::test]
    async fn initialization_defers_companion_schedulers_without_blocking_shutdown() {
        use db::runtime_initialization_repo::InitializationState;
        let initialization = arc_swap::ArcSwap::from_pointee(InitializationState::ProviderRequired);
        let shutdown = tokio_util::sync::CancellationToken::new();
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(10),
                wait_for_initialization(&initialization, &shutdown),
            )
            .await
            .is_err()
        );
        initialization.store(Arc::new(InitializationState::PersonalizationRequired));
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(10),
                wait_for_initialization(&initialization, &shutdown),
            )
            .await
            .is_err()
        );
        initialization.store(Arc::new(InitializationState::Ready));
        assert!(wait_for_initialization(&initialization, &shutdown).await);
        shutdown.cancel();
        assert!(!wait_for_initialization(&initialization, &shutdown).await);
    }

    #[test]
    fn agent_tool_selection_supports_authenticated_remote_targets() {
        let cli = Cli::try_parse_from([
            "bobe",
            "agent",
            "tools",
            "disable",
            "bash",
            "background_jobs",
            "--url",
            "https://authority.example/bobe",
            "--token",
            "fixture-token",
            "--json",
        ])
        .expect("remote tools command");
        let Commands::Agent {
            command:
                ops::headless::AgentCommands::Tools {
                    command: ops::headless::ToolCommands::Disable { tools, json },
                    url,
                    token,
                },
        } = cli.command
        else {
            panic!("expected remote tool selection")
        };
        assert!(json);
        assert_eq!(
            tools,
            [config::AgentTool::Bash, config::AgentTool::BackgroundJobs]
        );
        assert_eq!(url.as_deref(), Some("https://authority.example/bobe"));
        assert_eq!(token.as_deref(), Some("fixture-token"));
    }

    #[test]
    fn doctor_reports_provider_checks_as_explicit_and_optional() {
        let cli =
            Cli::try_parse_from(["bobe", "doctor", "--probe", "--json"]).expect("doctor probe");
        assert!(matches!(
            cli.command,
            Commands::Doctor {
                probe: true,
                json: true,
                ..
            }
        ));
        let report = serde_json::json!({"config":{"valid":true}, "runtime":{"readiness":{"status":"ready"}}, "provider":{"ok":null}});
        assert!(render_doctor(&report).contains("not tested"));
        let report = serde_json::json!({"config":{"valid":true}, "runtime":{"readiness":{"status":"ready"}}, "provider":{"ok":false,"error":"model unavailable"}});
        assert!(render_doctor(&report).contains("failed — model unavailable"));
    }

    use super::*;

    #[test]
    fn non_loopback_requires_complete_security_configuration() {
        let mut config = config::Config::default();
        config.server.host = "0.0.0.0".into();
        assert!(validate_server_security(&config).is_err());

        config.server.allowed_hosts = vec!["bobe.example.test".into()];
        assert!(validate_server_security(&config).is_ok());

        config.server.tls_cert_path = Some("cert.pem".into());
        assert!(validate_server_security(&config).is_err());
        config.server.tls_key_path = Some("key.pem".into());
        assert!(validate_server_security(&config).is_ok());
    }

    #[test]
    fn loopback_does_not_require_remote_transport_credentials() {
        let config = config::Config::default();
        assert!(validate_server_security(&config).is_ok());
    }

    #[test]
    fn loopback_with_public_allowed_host_uses_automatic_tls_and_device_auth() {
        let mut config = config::Config::default();
        config.server.allowed_hosts = vec!["bobe.example.test".into()];
        assert!(validate_server_security(&config).is_ok());
    }

    #[test]
    fn socket_address_supports_ipv4_and_ipv6_structurally() {
        let mut config = config::Config::default();
        assert_eq!(
            server_socket_addr(&config).expect("valid IPv4"),
            "127.0.0.1:8766".parse().expect("fixture")
        );
        config.server.host = "::1".into();
        assert_eq!(
            server_socket_addr(&config).expect("valid IPv6"),
            "[::1]:8766".parse().expect("fixture")
        );
    }

    #[test]
    fn standalone_agent_is_discovered_in_versioned_distribution() {
        let root = std::env::temp_dir().join(format!("bobe-helper-path-{}", uuid::Uuid::new_v4()));
        let bin = root.join("share/bobe/versions/1.2.3/bin");
        std::fs::create_dir_all(&bin).expect("version bin");
        let executable = bin.join("bobe");
        let agent = bin.join("agent");
        std::fs::create_dir_all(agent.join("dist")).unwrap();
        std::fs::write(&executable, "bobe").expect("executable fixture");
        std::fs::write(bin.join("node"), "node").expect("runtime fixture");
        std::fs::write(agent.join("dist/main.js"), "worker").expect("entry fixture");
        assert_eq!(
            agent_runtime_for_executable(&executable),
            Some(agent::RuntimeFiles {
                node: bin.join("node"),
                entry: agent.join("dist/main.js"),
            })
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn missing_remote_tls_path_identifies_the_material() {
        let root = std::env::temp_dir().join(format!("bobe-remote-path-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("config root");
        let error = normalize_config_path(
            std::path::Path::new("missing-key.pem"),
            &root,
            "TLS private key",
        )
        .expect_err("missing key must fail");
        assert!(matches!(error, error::AppError::Validation(_)));
        let message = error.to_string();
        assert!(message.contains("TLS private key"));
        assert!(message.contains(&root.join("missing-key.pem").to_string_lossy().into_owned()));
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn remote_tls_path_is_canonicalized_relative_to_config_root() {
        let root = std::env::temp_dir().join(format!("bobe-remote-path-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("config root");
        let certificate = root.join("certificate.pem");
        std::fs::write(&certificate, "certificate").expect("certificate");
        assert_eq!(
            normalize_config_path(
                std::path::Path::new("certificate.pem"),
                &root,
                "TLS certificate",
            )
            .expect("canonical path"),
            certificate.canonicalize().expect("canonical fixture")
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn remote_config_cli_requires_complete_atomic_inputs() {
        let cli = Cli::try_parse_from([
            "bobe",
            "config",
            "remote",
            "--host",
            "0.0.0.0",
            "--allowed-host",
            "bobe.example.test",
            "--tls-cert",
            "cert.pem",
            "--tls-key",
            "key.pem",
            "--json",
        ])
        .expect("valid remote config");
        match cli.command {
            Commands::Config {
                command:
                    ConfigCommands::Remote {
                        host,
                        allowed_hosts,
                        tls_cert,
                        tls_key,
                        json,
                    },
            } => {
                assert_eq!(host, "0.0.0.0");
                assert_eq!(allowed_hosts, ["bobe.example.test"]);
                assert_eq!(tls_cert, Some(std::path::PathBuf::from("cert.pem")));
                assert_eq!(tls_key, Some(std::path::PathBuf::from("key.pem")));
                assert!(json);
            }
            _ => panic!("expected remote config command"),
        }
        assert!(
            Cli::try_parse_from([
                "bobe",
                "config",
                "remote",
                "--host",
                "0.0.0.0",
                "--tls-cert",
                "cert.pem",
                "--tls-key",
                "key.pem",
            ])
            .is_err()
        );
    }

    #[test]
    fn serve_cli_leaves_config_overrides_unset_by_default() {
        let cli = Cli::try_parse_from(["bobe", "serve"]).expect("valid serve command");
        match cli.command {
            Commands::Serve {
                host,
                port,
                log_level,
                managed_bootstrap_stdin,
            } => {
                assert!(host.is_none());
                assert!(port.is_none());
                assert!(log_level.is_none());
                assert!(!managed_bootstrap_stdin);
            }
            Commands::Version { .. }
            | Commands::Init { .. }
            | Commands::Setup
            | Commands::Status { .. }
            | Commands::WaitReady { .. }
            | Commands::Doctor { .. }
            | Commands::Chat(..)
            | Commands::Jobs(..)
            | Commands::Runs(..)
            | Commands::Schedules(..)
            | Commands::Input(..)
            | Commands::Agent { .. }
            | Commands::Config { .. }
            | Commands::Initialization { .. }
            | Commands::Provider { .. }
            | Commands::Model { .. }
            | Commands::LocalRuntime { .. }
            | Commands::Memory { .. }
            | Commands::Profile { .. }
            | Commands::Skill { .. }
            | Commands::Workspace { .. }
            | Commands::Endpoint { .. }
            | Commands::Device { .. }
            | Commands::Models { .. }
            | Commands::Service { .. }
            | Commands::Backup { .. }
            | Commands::Update { .. } => panic!("expected serve command"),
        }
    }

    #[test]
    fn device_pair_accepts_a_phone_and_leaves_its_label_to_the_kind() {
        let cli = Cli::try_parse_from(["bobe", "device", "pair", "--kind", "phone", "--json"])
            .expect("valid phone pairing");
        let Commands::Device {
            command: DeviceCommands::Pair { kind, label, json },
        } = cli.command
        else {
            panic!("expected device pair command");
        };
        assert_eq!(kind, "phone");
        assert_eq!(label, None);
        assert!(json);
        assert!(Cli::try_parse_from(["bobe", "device", "pair", "--kind", "other"]).is_err());
    }

    #[test]
    fn serve_cli_accepts_explicit_overrides() {
        let cli = Cli::try_parse_from([
            "bobe",
            "serve",
            "--host",
            "127.0.0.2",
            "--port",
            "9000",
            "--log-level",
            "debug",
        ])
        .expect("valid serve overrides");
        match cli.command {
            Commands::Serve {
                host,
                port,
                log_level,
                managed_bootstrap_stdin,
            } => {
                assert_eq!(host.as_deref(), Some("127.0.0.2"));
                assert_eq!(port, Some(9000));
                assert_eq!(log_level.as_deref(), Some("debug"));
                assert!(!managed_bootstrap_stdin);
            }
            Commands::Version { .. }
            | Commands::Init { .. }
            | Commands::Setup
            | Commands::Status { .. }
            | Commands::WaitReady { .. }
            | Commands::Doctor { .. }
            | Commands::Chat(..)
            | Commands::Jobs(..)
            | Commands::Runs(..)
            | Commands::Schedules(..)
            | Commands::Input(..)
            | Commands::Agent { .. }
            | Commands::Config { .. }
            | Commands::Initialization { .. }
            | Commands::Provider { .. }
            | Commands::Model { .. }
            | Commands::LocalRuntime { .. }
            | Commands::Memory { .. }
            | Commands::Profile { .. }
            | Commands::Skill { .. }
            | Commands::Workspace { .. }
            | Commands::Endpoint { .. }
            | Commands::Device { .. }
            | Commands::Models { .. }
            | Commands::Service { .. }
            | Commands::Backup { .. }
            | Commands::Update { .. } => panic!("expected serve command"),
        }
    }
}

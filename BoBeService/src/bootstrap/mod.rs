pub(crate) mod database;
mod infra;
pub(crate) mod mcp_loader;
pub(crate) mod release_0_2;
mod repos;
mod voice_loader;
mod wiring;

use std::sync::Arc;

use arc_swap::ArcSwap;
use tracing::{info, warn};

use crate::app_state::AppState;
use crate::config::Config;
use crate::error::AppError;

use mcp_loader::load_mcp_servers;
use voice_loader::build_voice_engines_snapshot;

pub(crate) async fn initialize_storage(
    paths: &crate::util::paths::DataPaths,
) -> Result<
    (
        sqlx::SqlitePool,
        crate::db::runtime_authority_repo::RuntimeIdentity,
    ),
    AppError,
> {
    if crate::ops::backup::restore_journal_pending(paths) {
        return Err(AppError::Conflict(
            "an interrupted restore has not been reconciled; start BoBe again to finish or roll it back"
                .into(),
        ));
    }
    let (pool, schema_version) = database::open(paths).await?;
    let identity = crate::db::runtime_authority_repo::claim(&pool, paths, schema_version).await?;
    Ok((pool, identity))
}

pub(crate) async fn run(
    config: Config,
    paths: crate::util::paths::DataPaths,
    config_file: std::path::PathBuf,
    runtime_locks: crate::util::runtime_lock::RuntimeLocks,
    local_enrollment: Arc<crate::api::handlers::local_enrollment::LocalEnrollmentBootstrap>,
) -> Result<Arc<AppState>, AppError> {
    crate::util::tls::install_crypto_provider()?;
    let (pool, identity) = initialize_storage(&paths).await?;
    let initialization = crate::db::runtime_initialization_repo::ensure(&pool).await?;
    let identity = Arc::new(identity);
    info!(
        authority_id = %identity.runtime_authority_id,
        authority_epoch = identity.runtime_authority_epoch,
        incarnation_id = %identity.runtime_incarnation_id,
        daemon_version = identity.daemon_version,
        schema_version = identity.schema_version,
        started_at = %identity.started_at,
        data_root = %paths.root().display(),
        "runtime.identity"
    );
    let secret_store = crate::secrets::default_secret_store(&paths)
        .map_err(|error| AppError::Config(format!("could not open secret store: {error}")))?;
    let headless_secret_store = crate::secrets::headless_secret_store(&paths).map_err(|error| {
        AppError::Config(format!("could not open headless secret store: {error}"))
    })?;
    run_with_config(
        config,
        paths,
        config_file,
        runtime_locks,
        pool,
        identity,
        initialization,
        secret_store,
        headless_secret_store,
        local_enrollment,
    )
    .await
}

async fn run_with_config(
    config: Config,
    paths: crate::util::paths::DataPaths,
    config_file: std::path::PathBuf,
    runtime_locks: crate::util::runtime_lock::RuntimeLocks,
    pool: sqlx::SqlitePool,
    identity: Arc<crate::db::runtime_authority_repo::RuntimeIdentity>,
    initialization: crate::db::runtime_initialization_repo::InitializationState,
    secret_store: Arc<dyn crate::secrets::SecretStore>,
    headless_secret_store: Arc<dyn crate::secrets::SecretStore>,
    local_enrollment: Arc<crate::api::handlers::local_enrollment::LocalEnrollmentBootstrap>,
) -> Result<Arc<AppState>, AppError> {
    let data_root = paths.root().to_path_buf();
    let config_root = config_file
        .parent()
        .ok_or_else(|| AppError::Config("config file has no parent directory".into()))?
        .to_path_buf();

    let infra = infra::Infrastructure::build(&config, identity.runtime_incarnation_id)?;
    let repos = repos::Repositories::from_pool(&pool);
    let body_identities =
        crate::body::identity::BodyIdentityIndex::load(repos.device_trust_repo.as_ref()).await?;

    if config.mcp.enabled
        && let Err(e) = crate::mcp::config::ensure_mcp_config_exists(
            &data_root,
            config.mcp.config_file.as_deref(),
        )
    {
        warn!(error = %e, "bootstrap.ensure_mcp_config_failed");
    }

    let memory_file = {
        let path = data_root.join("memory.md");
        crate::provider::MemoryFile::new(path)
    };
    let skill_store = crate::provider::SkillStore::prepare(&paths).await?;
    let agent_workspace = crate::provider::AgentWorkspace::prepare(&paths)?;
    let mcp = load_mcp_servers(&config);
    let goal_file_store =
        crate::services::goals::file_store::GoalFileStore::new(data_root.join("goals"));
    let goals_service = Arc::new(crate::services::goals::goals_service::GoalsService::new(
        goal_file_store,
    ));
    let domain_tools = crate::provider::DomainTools::new(
        Arc::clone(&memory_file),
        Arc::clone(&goals_service),
        Arc::clone(&skill_store),
        config.memory.enabled,
        config.goals.enabled,
    );
    let voice_ws_active = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let voice_sink = Arc::new(crate::voice::sinks::VoiceSink::new());
    let voice_engines: Arc<ArcSwap<crate::voice::engines::VoiceEnginesSnapshot>> = Arc::new(
        ArcSwap::from_pointee(crate::voice::engines::VoiceEnginesSnapshot::default()),
    );
    {
        let voice_engines = Arc::clone(&voice_engines);
        let models_root = paths.models_dir();
        tokio::spawn(async move {
            if let Err(error) =
                crate::voice::install_service::reconcile_install_state(&models_root).await
            {
                tracing::warn!(%error, "bootstrap.voice_install_recovery_failed");
            }
            let snap = build_voice_engines_snapshot(models_root).await;
            info!(
                complete = snap.is_complete(),
                "bootstrap.voice_engines_loaded"
            );
            voice_engines.store(Arc::new(snap));
        });
    }
    let http_client = Arc::new(
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .read_timeout(std::time::Duration::from_secs(30))
            .user_agent(concat!(
                "bobe-daemon/",
                env!("CARGO_PKG_VERSION"),
                " (+https://www.bobebot.com)"
            ))
            .build()?,
    );

    let authority_lifecycle = Arc::new(crate::provider::AgentAuthorityLifecycle::default());
    let personal_data_lifecycle = Arc::new(tokio::sync::RwLock::new(()));
    let agent_jobs = crate::db::agent_job_repo::AgentJobRepo::with_limits(
        pool.clone(),
        config.agent.execution.clone(),
    );
    let runs = Arc::new(crate::runtime::run_registry::RunRegistry::new());
    agent_jobs.recover().await?;
    let souls_service = Arc::new(crate::services::documents::SoulsService::new(Arc::clone(
        &repos.soul_repo,
    )));
    let user_profile_service = Arc::new(crate::services::documents::UserProfileService::new(
        Arc::clone(&repos.user_profile_repo),
    ));
    let persona = crate::provider::PersonaSource::new(
        Arc::clone(&souls_service),
        Arc::clone(&user_profile_service),
    );
    let agent = crate::agent::PiRuntime::new(
        &config,
        Arc::new(paths.clone()),
        headless_secret_store.as_ref(),
        Arc::clone(&persona),
        Arc::clone(&skill_store),
        domain_tools,
        Arc::clone(&agent_jobs),
        Arc::clone(&repos.conversation_repo),
        Arc::clone(&authority_lifecycle),
        mcp,
        Arc::clone(&voice_sink),
        Arc::clone(&voice_engines),
    )?;
    let workers = Arc::clone(&agent);

    let wired = wiring::wire(
        &config,
        &config_root,
        &infra,
        &repos,
        Arc::clone(&workers),
        souls_service,
        user_profile_service,
        persona,
        goals_service,
        Arc::clone(&personal_data_lifecycle),
        Arc::clone(&agent_jobs),
        Arc::clone(&runs),
    )
    .await;

    if config.seed_default_documents {
        if let Err(e) = crate::db::seeding::seed_default_souls(repos.soul_repo.as_ref()).await {
            tracing::warn!(error = %e, "bootstrap.soul_seeding_failed");
        }
        if let Err(e) =
            crate::services::goals::seeding::seed_sample_goal(&pool, wired.goals_service.as_ref())
                .await
        {
            tracing::warn!(error = %e, "bootstrap.goal_seeding_failed");
        }
    }

    infra.mdns_announcer.start().await;

    print_banner(&infra.config_arc.load());

    let ollama_install = {
        let data_dir = data_root.clone();
        let binary = Arc::new(crate::services::ollama::binary_manager::BinaryManager::new(
            &data_dir,
            Arc::clone(&http_client),
        ));
        let base_url = config
            .providers
            .profiles
            .values()
            .find(|profile| {
                profile.local_runtime == Some(crate::provider::LocalRuntimeKind::Ollama)
            })
            .and_then(|profile| profile.base_url.as_deref())
            .map_or_else(
                || crate::constants::DEFAULT_OLLAMA_BASE_URL.to_owned(),
                crate::services::ollama::manager::OllamaManager::root_from_provider_url,
            );
        let manager = Arc::new(crate::services::ollama::manager::OllamaManager::new(
            Arc::clone(&http_client),
            &base_url,
        ));
        crate::services::ollama::install_service::OllamaInstallService::new(binary, manager)
    };

    let voice_install = {
        let http = Arc::clone(&http_client);
        let models_root = paths.models_dir();
        let models_root_for_reload = models_root.clone();
        let engines_for_reload = Arc::clone(&voice_engines);
        let on_complete: Arc<
            dyn Fn() -> futures::future::BoxFuture<
                    'static,
                    crate::voice::install_service::VoiceEngineAvailability,
                > + Send
                + Sync,
        > = Arc::new(move || {
            let engines = Arc::clone(&engines_for_reload);
            let models_root = models_root_for_reload.clone();
            Box::pin(async move {
                let snap = build_voice_engines_snapshot(models_root).await;
                let availability = crate::voice::install_service::VoiceEngineAvailability {
                    stt: snap.supports_daemon_stt(),
                    tts: snap.is_complete(),
                };
                info!(
                    tts = availability.tts,
                    stt = availability.stt,
                    "voice_install.engines_reloaded"
                );
                engines.store(Arc::new(snap));
                availability
            })
        });
        crate::voice::install_service::VoiceInstallService::new(http, models_root, on_complete)
    };

    let in_flight_text_turns = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let provider_login =
        crate::agent::auth::LoginCoordinator::new(&agent, authority_lifecycle.maintenance_flag());

    let initialization = Arc::new(ArcSwap::from_pointee(initialization));
    let background = crate::runtime::background_work::BackgroundWork::new(
        agent_jobs,
        Arc::clone(&workers),
        Arc::clone(&wired.runtime_session),
        Arc::clone(&personal_data_lifecycle),
        Arc::clone(&initialization),
        Arc::clone(&runs),
        Arc::clone(&infra.connection_manager),
        Arc::clone(&wired.goals_service),
        Arc::clone(&infra.config_arc),
        config
            .agent
            .enabled_tools
            .contains(&crate::config::AgentTool::BackgroundJobs),
    );
    let message_requests = crate::runtime::message_requests::MessageRequests::new(
        pool.clone(),
        Arc::clone(&wired.runtime_session),
        Arc::clone(&in_flight_text_turns),
        Arc::clone(&runs),
        Arc::clone(&personal_data_lifecycle),
    );
    let body_gateway = crate::body::gateway::BodyGateway::new(
        config.body.controller_epoch,
        config.body.enabled,
        crate::body::gateway::BodyRuntime {
            requests: Arc::clone(&message_requests),
            config: Arc::clone(&infra.config_arc),
            runtime_session: Arc::clone(&wired.runtime_session),
            voice_engines: Arc::clone(&voice_engines),
            voice_sink: Arc::clone(&voice_sink),
        },
        load_body_update_offer(&config.body),
    );

    let state = Arc::new(AppState {
        infra: Arc::new(crate::app_state::Infrastructure {
            db: pool,
            paths: Arc::new(paths),
            _runtime_locks: runtime_locks,
            identity,
            initialization,
            config: Arc::clone(&infra.config_arc),
            config_root: Arc::new(config_root),
            config_manager: wired.config_manager,
            config_write_lock: Arc::clone(&infra.config_write_lock),
            provider_restart_pending: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            event_queue: infra.event_queue,
            connection_manager: infra.connection_manager,
            mdns_announcer: infra.mdns_announcer,
            secret_store,
            headless_secret_store,
            mcp_config_lock: Arc::new(tokio::sync::Mutex::new(())),
        }),
        runtime: Arc::new(crate::app_state::RuntimeContext {
            runtime_session: wired.runtime_session,
            workers,
            active_providers: config.providers.clone(),
            active_routes: config.routes.clone(),
            active_agent: config.agent.clone(),
            provider_probe_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            initialization_lock: tokio::sync::Mutex::new(()),
            memory_file,
            skill_store,
            agent_workspace,
            personal_data_lifecycle,
            agent_authority_lifecycle: Arc::clone(&authority_lifecycle),
            in_flight_text_turns,
            runs,
            message_requests,
            background,
        }),
        voice: Arc::new(crate::app_state::VoiceContext {
            voice_install,
            voice_engines,
            voice_ws_active,
            voice_sink,
        }),
        services: Arc::new(crate::app_state::DomainServices {
            souls_service: wired.souls_service,
            user_profile_service: wired.user_profile_service,
            goals_service: wired.goals_service,
            ollama_install,
        }),
        auth: Arc::new(crate::app_state::AuthContext {
            pairing: crate::api::pairing::PairingManager::new(),
            local_enrollment,
            provider_login,
            agent,
        }),
        body: Arc::new(crate::app_state::BodyContext {
            gateway: body_gateway,
            endpoints: Arc::clone(&repos.endpoint_repo),
            trust: Arc::clone(&repos.device_trust_repo),
            identities: body_identities,
            registry_lock: Arc::new(tokio::sync::Mutex::new(())),
        }),
    });

    Ok(state)
}

fn print_banner(config: &Config) {
    let chat_profile = config.routes.chat.profile.as_str();
    let provider = config
        .providers
        .profiles
        .get(&config.routes.chat.profile)
        .map_or_else(
            || "unavailable".into(),
            |profile| format!("{:?}", profile.kind),
        );
    info!("═══════════════════════════════════════════════════════");
    info!("  BoBe Server Started");
    info!("  Chat provider: {chat_profile} ({provider})");
    info!("  Capture enabled: {}", config.capture.enabled);
    info!("  Tools (MCP) enabled: {}", config.mcp.enabled);
    info!("═══════════════════════════════════════════════════════");
}

fn load_body_update_offer(
    body: &crate::config::BodyConfig,
) -> Option<Arc<crate::body::update::UpdateOffer>> {
    let configured = body.update_offer_path.as_ref()?;
    let path = std::path::Path::new(configured);
    match crate::body::update::UpdateOffer::load(path) {
        Ok(offer) => {
            info!(
                path = %path.display(),
                build_number = offer.build_number(),
                version = %offer.manifest().version,
                key_id = %offer.manifest().key_id,
                image_size = offer.manifest().image_size,
                "body.update_offer_loaded"
            );
            Some(Arc::new(offer))
        }
        Err(error) => {
            warn!(
                path = %path.display(),
                err = %error,
                "body.update_offer_rejected"
            );
            None
        }
    }
}

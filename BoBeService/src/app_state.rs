use arc_swap::ArcSwap;
use sqlx::sqlite::SqlitePool;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use tokio::sync::{Mutex, RwLock};

use crate::agent::PiRuntime;
use crate::config::Config;
use crate::config::manager::ConfigManager;
use crate::provider::MemoryFile;
use crate::provider::{AgentWorkspace, SkillStore};
use crate::runtime::session::RuntimeSession;
use crate::secrets::SecretStore;
use crate::services::documents::{SoulsService, UserProfileService};
use crate::services::goals::goals_service::GoalsService;
use crate::services::ollama::install_service::OllamaInstallService;
use crate::util::network::MdnsAnnouncer;
use crate::util::sse::connection_manager::SseConnectionManager;
use crate::util::sse::event_queue::EventQueue;
use crate::voice::engines::VoiceEnginesSnapshot;
use crate::voice::install_service::VoiceInstallService;
use crate::voice::sinks::VoiceSink;

pub(crate) struct Infrastructure {
    pub(crate) db: SqlitePool,
    pub(crate) paths: Arc<crate::util::paths::DataPaths>,
    pub(crate) _runtime_locks: crate::util::runtime_lock::RuntimeLocks,
    pub(crate) identity: Arc<crate::db::runtime_authority_repo::RuntimeIdentity>,
    pub(crate) initialization:
        tokio::sync::watch::Sender<crate::db::runtime_initialization_repo::InitializationState>,
    pub(crate) config: Arc<ArcSwap<Config>>,
    pub(crate) config_root: Arc<std::path::PathBuf>,
    pub(crate) config_manager: Arc<ConfigManager>,
    pub(crate) config_write_lock: Arc<std::sync::Mutex<()>>,
    pub(crate) provider_restart_pending: Arc<AtomicBool>,
    pub(crate) event_queue: Arc<EventQueue>,
    pub(crate) connection_manager: Arc<SseConnectionManager>,
    pub(crate) mdns_announcer: Arc<MdnsAnnouncer>,
    pub(crate) secret_store: Arc<dyn SecretStore>,
    pub(crate) headless_secret_store: Arc<dyn SecretStore>,
    pub(crate) mcp_config_lock: Arc<Mutex<()>>,
}

pub(crate) struct RuntimeContext {
    pub(crate) runtime_session: Arc<RuntimeSession>,
    pub(crate) workers: Arc<PiRuntime>,
    pub(crate) active_providers: crate::provider::ProvidersConfig,
    pub(crate) active_routes: crate::provider::ProviderRoutes,
    pub(crate) active_agent: crate::config::AgentConfig,
    pub(crate) provider_probe_active: Arc<AtomicBool>,
    pub(crate) initialization_lock: Mutex<()>,
    pub(crate) memory_file: Arc<MemoryFile>,
    pub(crate) skill_store: Arc<SkillStore>,
    pub(crate) agent_workspace: Arc<AgentWorkspace>,
    pub(crate) personal_data_lifecycle: Arc<RwLock<()>>,
    pub(crate) agent_authority_lifecycle: Arc<crate::provider::AgentAuthorityLifecycle>,
    pub(crate) in_flight_text_turns: Arc<AtomicUsize>,
    pub(crate) runs: Arc<crate::runtime::run_registry::RunRegistry>,
    pub(crate) message_requests: Arc<crate::runtime::message_requests::MessageRequests>,
    pub(crate) background: Arc<crate::runtime::background_work::BackgroundWork>,
}

pub(crate) struct VoiceContext {
    pub(crate) voice_install: Arc<VoiceInstallService>,
    pub(crate) voice_engines: Arc<ArcSwap<VoiceEnginesSnapshot>>,
    pub(crate) voice_ws_active: Arc<AtomicBool>,
    pub(crate) voice_sink: Arc<VoiceSink>,
}

pub(crate) struct DomainServices {
    pub(crate) souls_service: Arc<SoulsService>,
    pub(crate) user_profile_service: Arc<UserProfileService>,
    pub(crate) goals_service: Arc<GoalsService>,
    pub(crate) ollama_install: Arc<OllamaInstallService>,
}

pub(crate) struct AuthContext {
    pub(crate) pairing: Arc<crate::api::pairing::PairingManager>,
    pub(crate) local_enrollment:
        Arc<crate::api::handlers::local_enrollment::LocalEnrollmentBootstrap>,
    pub(crate) provider_login: Arc<crate::agent::auth::LoginCoordinator>,
    pub(crate) agent: Arc<PiRuntime>,
}

pub(crate) struct BodyContext {
    pub(crate) gateway: Arc<crate::body::gateway::BodyGateway>,
    pub(crate) endpoints: Arc<crate::db::endpoint_repo::EndpointRepo>,
    pub(crate) trust: Arc<crate::db::device_trust_repo::DeviceTrustRepo>,
    pub(crate) identities: Arc<crate::body::identity::BodyIdentityIndex>,
    pub(crate) registry_lock: Arc<Mutex<()>>,
}

pub(crate) struct AppState {
    pub(crate) infra: Arc<Infrastructure>,
    pub(crate) runtime: Arc<RuntimeContext>,
    pub(crate) voice: Arc<VoiceContext>,
    pub(crate) services: Arc<DomainServices>,
    pub(crate) auth: Arc<AuthContext>,
    pub(crate) body: Arc<BodyContext>,
}

impl AppState {
    pub(crate) fn config(&self) -> arc_swap::Guard<Arc<Config>> {
        self.infra.config.load()
    }

    pub(crate) fn disconnect_credentials(&self, credential_ids: &[String]) {
        for credential_id in credential_ids {
            self.infra
                .connection_manager
                .disconnect_credential(credential_id);
        }
    }

    pub(crate) async fn disconnect_endpoint(&self, endpoint_id: &str) {
        self.body.gateway.revoke_endpoint(endpoint_id).await;
        self.infra
            .connection_manager
            .disconnect_endpoint(endpoint_id);
    }

    pub(crate) async fn apply_endpoint_registry_change(
        &self,
        endpoint_id: &str,
    ) -> Result<(), crate::error::AppError> {
        self.body.identities.refresh(&self.body.trust).await?;
        self.disconnect_endpoint(endpoint_id).await;
        Ok(())
    }

    pub(crate) async fn revoke_endpoint(
        &self,
        endpoint_id: &str,
        reason: &str,
    ) -> Result<bool, crate::error::AppError> {
        let _guard = self.body.registry_lock.lock().await;
        let revoked = self.body.trust.revoke_endpoint(endpoint_id, reason).await?;
        if revoked {
            self.apply_endpoint_registry_change(endpoint_id).await?;
        }
        Ok(revoked)
    }
}

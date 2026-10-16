use axum::{
    Router, middleware as axum_middleware,
    routing::{get, post},
};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::timeout::TimeoutLayer;

use super::handlers;
use super::middleware::{
    AllowedHosts, AllowedOrigins, ApiToken, bearer_auth, host_validation, personal_data_exclusion,
    request_logging, scope_enforcement,
};
use crate::app_state::AppState;
use crate::models::soul::Souls;
use crate::models::user_profile::UserProfiles;

pub(crate) fn build_router(
    state: Arc<AppState>,
    shutdown: tokio_util::sync::CancellationToken,
) -> Router {
    let cfg = state.config();

    let origins: Vec<axum::http::HeaderValue> = cfg
        .cors_origins_vec()
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::PATCH,
            axum::http::Method::DELETE,
        ])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
        ])
        .allow_credentials(true);

    let allowed_hosts =
        AllowedHosts::new(&cfg.server.host, cfg.server.port, &cfg.server.allowed_hosts);
    let api_token = ApiToken::new(cfg.server.api_token.clone());
    let allowed_origins = AllowedOrigins::new(cfg.cors_origins_vec());

    let local_enrollment = Router::new()
        .route(
            "/v2/local-enrollment/complete",
            post(handlers::local_enrollment::complete),
        )
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            std::time::Duration::from_secs(10),
        ))
        .layer(axum_middleware::from_fn(request_logging))
        .layer(tower::limit::ConcurrencyLimitLayer::new(1))
        .with_state(Arc::clone(&state));

    let pairing = Router::new()
        .route("/v2/pairing/begin", post(handlers::pairing::begin))
        .route("/v2/pairing/complete", post(handlers::pairing::complete))
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            std::time::Duration::from_secs(15),
        ))
        .layer(axum_middleware::from_fn(request_logging))
        .layer(axum_middleware::from_fn(host_validation))
        .layer(axum::Extension(allowed_hosts.clone()))
        .layer(tower::limit::ConcurrencyLimitLayer::new(8))
        .with_state(Arc::clone(&state));

    let long_lived = Router::new()
        .route("/events", get(handlers::events::stream_events))
        .route("/voice/stream", get(handlers::voice::voice_stream))
        .route("/voice/preview", get(handlers::voice_preview::upgrade))
        .route(
            "/local-runtime/status",
            get(handlers::local_runtime::install_status_stream),
        )
        .route(
            "/auth/{profile}/login/events",
            get(handlers::provider_login::events),
        );

    let privacy = Router::new()
        .route(
            "/privacy/data",
            axum::routing::delete(handlers::privacy::purge),
        )
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            crate::constants::privacy::HTTP_TIMEOUT,
        ));

    let diagnostics = Router::new()
        .route(
            "/diagnostics/provider",
            post(handlers::health::diagnose_provider),
        )
        .route(
            "/initialization",
            get(handlers::initialization::get).post(handlers::initialization::advance),
        )
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            std::time::Duration::from_secs(90),
        ));

    let agent_context = Router::new()
        .route("/agent/context", get(handlers::agent_context::status))
        .route(
            "/agent/context/compact",
            post(handlers::agent_context::compact),
        )
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            std::time::Duration::from_secs(315),
        ));

    let configuration = Router::new()
        .route(
            "/config",
            axum::routing::patch(handlers::config_admin::update),
        )
        .route(
            "/config/{key}",
            axum::routing::delete(handlers::config_admin::unset),
        )
        .route(
            "/settings",
            get(handlers::settings::get_settings).patch(handlers::settings::update_settings),
        )
        .route(
            "/tools/mcp/config",
            get(handlers::tools_mcp::get_mcp_config)
                .put(handlers::tools_mcp::save_mcp_config)
                .delete(handlers::tools_mcp::reset_mcp_config),
        )
        .layer(axum_middleware::from_fn_with_state(
            Arc::clone(&state),
            personal_data_exclusion,
        ))
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            crate::services::mcp_config_service::REQUEST_TIMEOUT,
        ));

    let short_lived = Router::new()
        .route("/owner-input", get(handlers::owner_input::list))
        .route("/owner-input/{id}", get(handlers::owner_input::get))
        .route(
            "/owner-input/{id}/answer",
            post(handlers::owner_input::answer),
        )
        .route(
            "/owner-input/{id}/presented",
            post(handlers::owner_input::presented),
        )
        .route(
            "/owner-input/{id}/cancel",
            post(handlers::owner_input::cancel),
        )
        .route(
            "/agent/schedules",
            get(handlers::agent_schedules::list).post(handlers::agent_schedules::create),
        )
        .route(
            "/agent/schedules/{id}",
            get(handlers::agent_schedules::get).delete(handlers::agent_schedules::delete),
        )
        .route(
            "/agent/schedules/{id}/enabled",
            axum::routing::put(handlers::agent_schedules::set_enabled),
        )
        .route(
            "/agent/schedules/{id}/time",
            axum::routing::put(handlers::agent_schedules::reschedule),
        )
        .route(
            "/agent/jobs",
            get(handlers::agent_jobs::list).post(handlers::agent_jobs::start),
        )
        .route("/agent/jobs/{id}", get(handlers::agent_jobs::get))
        .route(
            "/agent/jobs/{id}/cancel",
            post(handlers::agent_jobs::cancel),
        )
        .route(
            "/agent/jobs/{id}/resume",
            post(handlers::agent_jobs::resume),
        )
        .route(
            "/agent/tools",
            get(handlers::agent_tools::get).put(handlers::agent_tools::set),
        )
        .route(crate::constants::attach::PATH, post(handlers::hello::hello))
        .route("/health", get(handlers::health::health_check))
        .route("/readyz", get(handlers::health::readiness))
        .route("/status", get(handlers::health::get_status))
        .route(
            "/conversation/current",
            get(handlers::conversation::current_conversation),
        )
        .route("/message", post(handlers::conversation::send_message))
        .route("/providers", get(handlers::providers::list))
        .route("/endpoints", get(handlers::endpoints::list))
        .route(
            "/endpoints/self",
            axum::routing::delete(handlers::endpoints::revoke_self),
        )
        .route(
            "/pairing/invitations",
            post(handlers::pairing::create_invitation),
        )
        .route(
            "/endpoints/{id}/display",
            post(handlers::endpoints::display_text),
        )
        .route("/endpoints/{id}/sensors", get(handlers::endpoints::sensors))
        .route(
            "/endpoints/{id}",
            axum::routing::put(handlers::endpoints::upsert).delete(handlers::endpoints::revoke),
        )
        .route(
            "/providers/{id}",
            axum::routing::put(handlers::providers::upsert).delete(handlers::providers::remove),
        )
        .route(
            "/providers/{id}/activation",
            axum::routing::put(handlers::providers::activate),
        )
        .route(
            "/providers/{id}/credential",
            axum::routing::put(handlers::providers::set_credential)
                .delete(handlers::providers::clear_credential),
        )
        .route(
            "/providers/{id}/probe",
            post(handlers::health::diagnose_profile),
        )
        .route(
            "/auth/{profile}/logout",
            post(handlers::provider_login::logout),
        )
        .route(
            "/provider-routes",
            axum::routing::put(handlers::providers::update_routes)
                .patch(handlers::providers::patch_routes),
        )
        .route(
            "/runs/{run_id}/cancel",
            post(handlers::conversation::cancel_run),
        )
        .route("/runs", get(handlers::conversation::list_runs))
        .route(
            "/conversation/{turn_id}/presented",
            post(handlers::conversation::presented),
        )
        .route(
            "/runs/{run_id}/inputs/{input_id}",
            axum::routing::put(handlers::conversation::submit_run_input),
        )
        .route(
            "/body/requests/{request_id}",
            post(handlers::body_requests::complete),
        )
        .route(
            "/goals",
            get(handlers::goals::list_goals).post(handlers::goals::create_goal),
        )
        .route(
            "/goals/{goal_id}",
            axum::routing::patch(handlers::goals::update_goal).delete(handlers::goals::delete_goal),
        )
        .route(
            "/goals/{goal_id}/complete",
            post(handlers::goals::complete_goal),
        )
        .route(
            "/goals/{goal_id}/archive",
            post(handlers::goals::archive_goal),
        )
        .route(
            "/memory",
            get(handlers::memories::get_memory).put(handlers::memories::update_memory),
        )
        .route(
            "/souls",
            get(handlers::documents::list::<Souls>).post(handlers::documents::create::<Souls>),
        )
        .route(
            "/souls/{soul_id}",
            axum::routing::patch(handlers::documents::update::<Souls>)
                .delete(handlers::documents::delete::<Souls>),
        )
        .route(
            "/souls/{soul_id}/enable",
            post(handlers::documents::enable::<Souls>),
        )
        .route(
            "/souls/{soul_id}/disable",
            post(handlers::documents::disable::<Souls>),
        )
        .route(
            "/user-profiles",
            get(handlers::documents::list::<UserProfiles>)
                .post(handlers::documents::create::<UserProfiles>),
        )
        .route(
            "/user-profiles/{profile_id}",
            axum::routing::patch(handlers::documents::update::<UserProfiles>)
                .delete(handlers::documents::delete::<UserProfiles>),
        )
        .route(
            "/user-profiles/{profile_id}/enable",
            post(handlers::documents::enable::<UserProfiles>),
        )
        .route(
            "/user-profiles/{profile_id}/disable",
            post(handlers::documents::disable::<UserProfiles>),
        )
        .route("/skills", get(handlers::skills::list))
        .route("/skills/reload", post(handlers::skills::reload))
        .route("/skills/{id}", get(handlers::skills::read))
        .route("/workspace", get(handlers::workspace::list))
        .route(
            "/workspace/file",
            get(handlers::workspace::read).put(handlers::workspace::write),
        )
        .route(
            "/auth/{profile}/status",
            get(handlers::provider_login::auth_status),
        )
        .route(
            "/auth/{profile}/login/start",
            post(handlers::provider_login::start_login),
        )
        .route(
            "/auth/{profile}/login/cancel",
            post(handlers::provider_login::cancel_login),
        )
        .route(
            "/auth/{profile}/login/status",
            get(handlers::provider_login::status),
        )
        .route(
            "/auth/{profile}/login/submit",
            post(handlers::provider_login::submit),
        )
        .route("/models", get(handlers::models::list_models))
        .route(
            "/local-runtime/install",
            post(handlers::local_runtime::start_install),
        )
        .route(
            "/local-runtime/cancel",
            post(handlers::local_runtime::cancel_install),
        )
        .route(
            "/tools/mcp/config/validate",
            post(handlers::tools_mcp::validate_mcp_config),
        )
        .route(
            "/voice/install/status",
            get(handlers::voice_install::status),
        )
        .route("/voice/install/start", post(handlers::voice_install::start))
        .route(
            "/voice/install/start/tts",
            post(handlers::voice_install::start_tts),
        )
        .route(
            "/voice/install/start/stt",
            post(handlers::voice_install::start_stt),
        )
        .route(
            "/voice/install/cancel",
            post(handlers::voice_install::cancel),
        )
        .layer(axum_middleware::from_fn_with_state(
            Arc::clone(&state),
            personal_data_exclusion,
        ))
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            std::time::Duration::from_secs(30),
        ));

    local_enrollment.merge(pairing).merge(
        short_lived
            .merge(configuration)
            .merge(privacy)
            .merge(diagnostics)
            .merge(agent_context)
            .merge(long_lived)
            .layer(axum::Extension(shutdown))
            .layer(axum::Extension(allowed_origins))
            .layer(axum_middleware::from_fn(request_logging))
            .layer(axum_middleware::from_fn(scope_enforcement))
            .layer(axum_middleware::from_fn_with_state(
                Arc::clone(&state),
                bearer_auth,
            ))
            .layer(axum::Extension(api_token))
            .layer(axum_middleware::from_fn(host_validation))
            .layer(axum::Extension(allowed_hosts))
            .layer(cors)
            .layer(tower::limit::ConcurrencyLimitLayer::new(64))
            .with_state(state),
    )
}

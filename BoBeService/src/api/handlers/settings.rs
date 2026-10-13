use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::bootstrap::mcp_loader::load_mcp_servers;
use crate::config::{BodyTheme, PauseSensitivity};
use crate::error::AppError;

#[derive(Debug, Serialize)]
pub(crate) struct SettingsResponse {
    pub(crate) body_theme: BodyTheme,
    pub(crate) body_connected: bool,
    pub(crate) capture_enabled: bool,
    pub(crate) capture_interval_seconds: u64,
    pub(crate) checkin_enabled: bool,
    pub(crate) checkin_times: Vec<String>,
    pub(crate) checkin_jitter_minutes: u32,
    pub(crate) conversation_inactivity_timeout_seconds: u64,
    pub(crate) conversation_auto_close_minutes: u64,
    pub(crate) goal_check_interval_seconds: f64,
    pub(crate) mcp_enabled: bool,
    pub(crate) voice_enabled: bool,
    pub(crate) voice_persona: String,
    pub(crate) voice_speed: f32,
    pub(crate) voice_stt_language: String,
    pub(crate) voice_pause_sensitivity: PauseSensitivity,
    pub(crate) voice_show_partial_caption: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsUpdateRequest {
    pub(crate) body_theme: Option<BodyTheme>,
    pub(crate) capture_enabled: Option<bool>,
    pub(crate) capture_interval_seconds: Option<u64>,
    pub(crate) checkin_enabled: Option<bool>,
    pub(crate) checkin_times: Option<Vec<String>>,
    pub(crate) checkin_jitter_minutes: Option<u32>,
    pub(crate) conversation_inactivity_timeout_seconds: Option<u64>,
    pub(crate) conversation_auto_close_minutes: Option<u64>,
    pub(crate) goal_check_interval_seconds: Option<f64>,
    pub(crate) mcp_enabled: Option<bool>,
    pub(crate) voice_enabled: Option<bool>,
    pub(crate) voice_persona: Option<String>,
    pub(crate) voice_speed: Option<f32>,
    pub(crate) voice_stt_language: Option<String>,
    pub(crate) voice_pause_sensitivity: Option<PauseSensitivity>,
    pub(crate) voice_show_partial_caption: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SettingsUpdateResponse {
    pub(crate) message: String,
    pub(crate) applied_fields: Vec<String>,
    pub(crate) restart_required_fields: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) persist_failed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) body_theme_delivered: Option<bool>,
}

pub(crate) async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SettingsResponse>, AppError> {
    let config = state.config();
    Ok(Json(SettingsResponse {
        body_theme: config.body.theme,
        body_connected: state.body.gateway.has_connected_body(),
        capture_enabled: config.capture.enabled,
        capture_interval_seconds: config.capture.interval_seconds,
        checkin_enabled: config.checkin.enabled,
        checkin_times: config.checkin_times_vec().to_vec(),
        checkin_jitter_minutes: config.checkin.jitter_minutes,
        conversation_inactivity_timeout_seconds: config.conversation.inactivity_timeout_seconds,
        conversation_auto_close_minutes: config.conversation.auto_close_minutes,
        goal_check_interval_seconds: config.goals.check_interval_seconds,
        mcp_enabled: config.mcp.enabled,
        voice_enabled: config.voice.enabled,
        voice_persona: config.voice.persona.clone(),
        voice_speed: config.voice.speed,
        voice_stt_language: config.voice.stt_language.clone(),
        voice_pause_sensitivity: config.voice.pause_sensitivity,
        voice_show_partial_caption: config.voice.show_partial_caption,
    }))
}

pub(crate) async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SettingsUpdateRequest>,
) -> Result<Json<SettingsUpdateResponse>, AppError> {
    let mut changes: HashMap<String, serde_json::Value> = HashMap::new();
    macro_rules! collect_opt {
        ($field:ident, $key:literal) => {
            if let Some(ref value) = body.$field {
                changes.insert(
                    $key.to_owned(),
                    serde_json::to_value(value).unwrap_or_default(),
                );
            }
        };
    }

    collect_opt!(body_theme, "body.theme");
    collect_opt!(capture_enabled, "capture.enabled");
    collect_opt!(capture_interval_seconds, "capture.interval_seconds");
    collect_opt!(checkin_enabled, "checkin.enabled");
    collect_opt!(checkin_jitter_minutes, "checkin.jitter_minutes");
    collect_opt!(
        conversation_inactivity_timeout_seconds,
        "conversation.inactivity_timeout_seconds"
    );
    collect_opt!(
        conversation_auto_close_minutes,
        "conversation.auto_close_minutes"
    );
    collect_opt!(goal_check_interval_seconds, "goals.check_interval_seconds");
    collect_opt!(mcp_enabled, "mcp.enabled");
    collect_opt!(voice_enabled, "voice.enabled");
    collect_opt!(voice_persona, "voice.persona");
    collect_opt!(voice_speed, "voice.speed");
    collect_opt!(voice_stt_language, "voice.stt_language");
    collect_opt!(voice_pause_sensitivity, "voice.pause_sensitivity");
    collect_opt!(voice_show_partial_caption, "voice.show_partial_caption");

    if let Some(ref value) = body.checkin_times {
        changes.insert(
            "checkin.times".to_owned(),
            serde_json::to_value(value).unwrap_or_default(),
        );
    }
    if changes.is_empty() {
        return Ok(Json(SettingsUpdateResponse {
            message: "No changes provided".into(),
            applied_fields: vec![],
            restart_required_fields: vec![],
            persist_failed: false,
            body_theme_delivered: None,
        }));
    }

    let mcp_config_guard = if body.mcp_enabled.is_some() {
        Some(state.infra.mcp_config_lock.lock().await)
    } else {
        None
    };
    let previous_mcp_enabled = state.config().mcp.enabled;
    let result = tokio::task::block_in_place(|| state.infra.config_manager.update(&changes))?;
    let current_config = (**state.config()).clone();
    let body_theme_changed = result
        .applied_fields
        .iter()
        .any(|field| field == "body.theme");
    if previous_mcp_enabled != current_config.mcp.enabled {
        state
            .runtime
            .workers
            .apply_mcp_config(load_mcp_servers(&current_config))
            .await?;
    }
    drop(mcp_config_guard);
    let body_theme_delivered = if body_theme_changed {
        Some(
            state
                .body
                .gateway
                .apply_theme(current_config.body.theme)
                .await,
        )
    } else {
        None
    };

    if result.applied_fields.iter().any(|field| {
        matches!(
            field.as_str(),
            "capture.enabled" | "capture.interval_seconds"
        )
    }) {
        state.runtime.runtime_session.wake_capture();
    }
    if result.applied_fields.iter().any(|field| {
        matches!(
            field.as_str(),
            "checkin.enabled" | "checkin.times" | "checkin.jitter_minutes"
        )
    }) {
        state.runtime.runtime_session.reconfigure_checkins().await;
    }

    let total = result.applied_fields.len() + result.restart_required_fields.len();
    Ok(Json(SettingsUpdateResponse {
        message: format!("Updated {total} setting(s)"),
        applied_fields: result.applied_fields,
        restart_required_fields: result.restart_required_fields,
        persist_failed: result.persist_failed,
        body_theme_delivered,
    }))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests panic on precondition failures")]
mod tests {
    use super::*;

    #[test]
    fn rejects_removed_provider_fields() {
        let result = serde_json::from_str::<SettingsUpdateRequest>(r#"{"engine":"local"}"#);
        assert!(result.is_err());
        let result = serde_json::from_str::<SettingsUpdateRequest>(
            r#"{"provider_base_url":"http://localhost:11434/v1"}"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn rejects_unknown_and_wrongly_typed_fields() {
        assert!(serde_json::from_str::<SettingsUpdateRequest>(r#"{"unknown":true}"#).is_err());
        assert!(
            serde_json::from_str::<SettingsUpdateRequest>(r#"{"capture_enabled":"yes"}"#).is_err()
        );
    }
}

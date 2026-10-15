use serde::{Deserialize, Serialize};

use crate::app_state::Infrastructure;
use crate::db::runtime_initialization_repo::InitializationState;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReadinessStatus {
    Ready,
    Initializing,
    NotReady,
}

impl ReadinessStatus {
    pub(crate) const fn is_serving(self) -> bool {
        matches!(self, Self::Ready | Self::Initializing)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BlockingReason {
    DatabaseUnavailable,
    InitializationIncomplete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct Readiness {
    pub(crate) status: ReadinessStatus,
    pub(crate) blocking_reasons: Vec<BlockingReason>,
    pub(crate) initialization_state: InitializationState,
}

impl Readiness {
    pub(crate) async fn check(infra: &Infrastructure) -> Self {
        let database_available = sqlx::query("SELECT 1").fetch_one(&infra.db).await.is_ok();
        Self::evaluate(database_available, **infra.initialization.load())
    }

    pub(crate) fn evaluate(
        database_available: bool,
        initialization_state: InitializationState,
    ) -> Self {
        let (status, blocking_reasons) = if !database_available {
            (
                ReadinessStatus::NotReady,
                vec![BlockingReason::DatabaseUnavailable],
            )
        } else if initialization_state == InitializationState::Ready {
            (ReadinessStatus::Ready, Vec::new())
        } else {
            (
                ReadinessStatus::Initializing,
                vec![BlockingReason::InitializationIncomplete],
            )
        };
        Self {
            status,
            blocking_reasons,
            initialization_state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operational_readiness_requires_a_database_and_completed_initialization() {
        for initialization_state in [
            InitializationState::ProviderRequired,
            InitializationState::PersonalizationRequired,
        ] {
            assert_eq!(
                Readiness::evaluate(true, initialization_state),
                Readiness {
                    status: ReadinessStatus::Initializing,
                    blocking_reasons: vec![BlockingReason::InitializationIncomplete],
                    initialization_state,
                }
            );
        }
        assert_eq!(
            Readiness::evaluate(true, InitializationState::Ready),
            Readiness {
                status: ReadinessStatus::Ready,
                blocking_reasons: Vec::new(),
                initialization_state: InitializationState::Ready,
            }
        );
        for initialization_state in [
            InitializationState::ProviderRequired,
            InitializationState::Ready,
        ] {
            assert_eq!(
                Readiness::evaluate(false, initialization_state),
                Readiness {
                    status: ReadinessStatus::NotReady,
                    blocking_reasons: vec![BlockingReason::DatabaseUnavailable],
                    initialization_state,
                }
            );
        }
    }

    #[test]
    fn a_runtime_serves_surfaces_while_setup_is_incomplete() {
        assert!(ReadinessStatus::Ready.is_serving());
        assert!(ReadinessStatus::Initializing.is_serving());
        assert!(!ReadinessStatus::NotReady.is_serving());
    }

    #[test]
    fn readiness_uses_the_wire_values_every_surface_reads() {
        assert_eq!(
            serde_json::to_value(Readiness::evaluate(
                true,
                InitializationState::PersonalizationRequired
            ))
            .unwrap(),
            serde_json::json!({
                "status": "initializing",
                "blocking_reasons": ["initialization_incomplete"],
                "initialization_state": "personalization_required",
            })
        );
        assert_eq!(
            serde_json::to_value(Readiness::evaluate(false, InitializationState::Ready)).unwrap(),
            serde_json::json!({
                "status": "not_ready",
                "blocking_reasons": ["database_unavailable"],
                "initialization_state": "ready",
            })
        );
        for (status, wire) in [
            (ReadinessStatus::Ready, "ready"),
            (ReadinessStatus::Initializing, "initializing"),
            (ReadinessStatus::NotReady, "not_ready"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<ReadinessStatus>(wire.into()).unwrap(),
                status
            );
        }
    }
}

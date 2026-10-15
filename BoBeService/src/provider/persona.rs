use std::sync::Arc;

use serde::Serialize;
use tracing::warn;

use crate::services::documents::{SoulsService, UserProfileService};

const DECISION_PROFILES_MAX: usize = 4;
const DECISION_PROFILE_MAX_BYTES: usize = 1_500;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Persona {
    pub(crate) souls: Vec<String>,
    pub(crate) profiles: Vec<String>,
}

pub(crate) struct PersonaSource {
    souls: Arc<SoulsService>,
    profiles: Arc<UserProfileService>,
}

impl PersonaSource {
    pub(crate) fn new(souls: Arc<SoulsService>, profiles: Arc<UserProfileService>) -> Arc<Self> {
        Arc::new(Self { souls, profiles })
    }

    pub(crate) async fn current(&self) -> Persona {
        let (souls, profiles) = tokio::join!(self.souls(), self.profiles());
        Persona { souls, profiles }
    }

    pub(crate) async fn decision_profiles(&self) -> Vec<String> {
        bounded_profiles(&self.profiles().await)
    }

    async fn profiles(&self) -> Vec<String> {
        self.profiles.list(true).await.map_or_else(
            |error| {
                warn!(%error, "persona.profiles_unavailable");
                Vec::new()
            },
            |summary| {
                summary
                    .profiles
                    .into_iter()
                    .map(|profile| profile.content)
                    .collect()
            },
        )
    }

    async fn souls(&self) -> Vec<String> {
        self.souls.list(true).await.map_or_else(
            |error| {
                warn!(%error, "persona.souls_unavailable");
                Vec::new()
            },
            |summary| summary.souls.into_iter().map(|soul| soul.content).collect(),
        )
    }
}

fn bounded_profiles(profiles: &[String]) -> Vec<String> {
    profiles
        .iter()
        .map(|profile| profile.trim())
        .filter(|profile| !profile.is_empty())
        .take(DECISION_PROFILES_MAX)
        .map(|profile| {
            crate::util::text::truncate_str(profile, DECISION_PROFILE_MAX_BYTES).to_owned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_see_bounded_owner_profiles() {
        let long = format!(
            "Reach me only after six. {}",
            "é".repeat(DECISION_PROFILE_MAX_BYTES)
        );
        let mut profiles = vec!["  ".to_owned(), long];
        profiles.extend((0..DECISION_PROFILES_MAX).map(|index| format!("  Profile {index}  ")));
        let bounded = bounded_profiles(&profiles);
        assert_eq!(bounded.len(), DECISION_PROFILES_MAX);
        assert!(bounded[0].starts_with("Reach me only after six."));
        assert!(bounded[0].len() <= DECISION_PROFILE_MAX_BYTES);
        assert_eq!(bounded[1], "Profile 0");
        assert_eq!(
            bounded[DECISION_PROFILES_MAX - 1],
            format!("Profile {}", DECISION_PROFILES_MAX - 2)
        );
    }

    #[test]
    fn persona_matches_the_worker_chat_contract() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../BoBeAgent/fixtures/host-contract-v1.json"
        ))
        .unwrap();
        let persona = Persona {
            souls: vec!["Speak plainly.".into()],
            profiles: vec!["The owner is learning Spanish.".into()],
        };
        assert_eq!(
            serde_json::to_value(&persona).unwrap(),
            fixture["chat_request"]["params"]["persona"]
        );
    }

    #[tokio::test]
    async fn persona_carries_only_enabled_owner_documents() {
        let pool = crate::db::test_helpers::in_memory_pool().await;
        let souls = Arc::new(SoulsService::new(Arc::new(crate::db::SqliteSoulRepo::new(
            pool.clone(),
        ))));
        let profiles = Arc::new(UserProfileService::new(Arc::new(
            crate::db::SqliteUserProfileRepo::new(pool.clone()),
        )));
        let source = PersonaSource::new(Arc::clone(&souls), Arc::clone(&profiles));
        assert_eq!(source.current().await, Persona::default());

        souls
            .create("plain".into(), "Speak plainly and briefly.".into(), true)
            .await
            .unwrap();
        souls
            .create("loud".into(), "Shout every single sentence.".into(), false)
            .await
            .unwrap();
        profiles
            .create(
                "owner".into(),
                "The owner is learning Spanish.".into(),
                true,
            )
            .await
            .unwrap();
        assert_eq!(
            source.current().await,
            Persona {
                souls: vec!["Speak plainly and briefly.".into()],
                profiles: vec!["The owner is learning Spanish.".into()],
            }
        );
        assert_eq!(
            source.decision_profiles().await,
            vec!["The owner is learning Spanish.".to_owned()]
        );
        pool.close().await;
    }
}

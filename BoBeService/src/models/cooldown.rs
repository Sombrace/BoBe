use chrono::{DateTime, Duration, Utc};

use super::ids::CooldownId;

#[derive(Debug, Clone)]
pub(crate) struct CooldownInfo {
    pub(crate) remaining: Duration,
    pub(crate) cooldown_type: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub(crate) struct Cooldown {
    pub(crate) id: CooldownId,
    pub(crate) last_engagement: Option<DateTime<Utc>>,
    pub(crate) last_user_response: Option<DateTime<Utc>>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl Cooldown {
    pub(crate) fn new() -> Self {
        let now = Utc::now();
        Self {
            id: CooldownId::new(),
            last_engagement: None,
            last_user_response: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub(crate) fn check_cooldown(
        &self,
        base_minutes: i64,
        extended_minutes: i64,
    ) -> Option<CooldownInfo> {
        let now = Utc::now();

        if let Some(last_response) = self.last_user_response {
            let extended = Duration::minutes(extended_minutes);
            let elapsed = now - last_response;
            if elapsed < extended {
                return Some(CooldownInfo {
                    remaining: extended - elapsed,
                    cooldown_type: "user_response".to_owned(),
                });
            }
        }

        if let Some(last_eng) = self.last_engagement {
            let base = Duration::minutes(base_minutes);
            let elapsed = now - last_eng;
            if elapsed < base {
                return Some(CooldownInfo {
                    remaining: base - elapsed,
                    cooldown_type: "ai_engagement".to_owned(),
                });
            }
        }

        None
    }
}

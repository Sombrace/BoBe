use std::sync::Arc;

use sqlx::sqlite::SqlitePool;

use crate::db::device_trust_repo::DeviceTrustRepo;
use crate::db::endpoint_repo::EndpointRepo;
use crate::db::{
    SqliteConversationRepo, SqliteCooldownRepo, SqliteSoulRepo, SqliteUserProfileRepo,
};

pub(crate) struct Repositories {
    pub(crate) conversation_repo: Arc<SqliteConversationRepo>,
    pub(crate) endpoint_repo: Arc<EndpointRepo>,
    pub(crate) device_trust_repo: Arc<DeviceTrustRepo>,
    pub(crate) cooldown_repo: Arc<SqliteCooldownRepo>,
    pub(crate) soul_repo: Arc<SqliteSoulRepo>,
    pub(crate) user_profile_repo: Arc<SqliteUserProfileRepo>,
}

impl Repositories {
    pub(crate) fn from_pool(pool: &SqlitePool) -> Self {
        Self {
            conversation_repo: Arc::new(SqliteConversationRepo::new(pool.clone())),
            endpoint_repo: Arc::new(EndpointRepo::new(pool.clone())),
            device_trust_repo: Arc::new(DeviceTrustRepo::new(pool.clone())),
            cooldown_repo: Arc::new(SqliteCooldownRepo::new(pool.clone())),
            soul_repo: Arc::new(SqliteSoulRepo::new(pool.clone())),
            user_profile_repo: Arc::new(SqliteUserProfileRepo::new(pool.clone())),
        }
    }
}

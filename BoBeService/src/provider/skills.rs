use std::path::Path;

use tracing::{debug, warn};

use super::WorkerClass;

pub(crate) const CHAT_SKILL_MD: &str = include_str!("../../../BoBeAgent/assets/chat.md");

const SHIPPED_SKILLS: &[(WorkerClass, &str)] = &[(WorkerClass::Chat, CHAT_SKILL_MD)];

pub(crate) async fn ensure_skills(data_dir: &Path) {
    for &(workload, content) in SHIPPED_SKILLS {
        let dir = data_dir.join("skills").join(workload.name());
        let path = dir.join("SKILL.md");
        if path.exists() {
            debug!(workload = workload.name(), path = %path.display(), "skill exists, leaving as-is");
            continue;
        }
        if let Err(error) = tokio::fs::create_dir_all(&dir).await {
            warn!(workload = workload.name(), %error, "could not create skill directory");
            continue;
        }
        if let Err(error) = tokio::fs::write(&path, content).await {
            warn!(workload = workload.name(), %error, "could not write skill");
            continue;
        }
        tracing::info!(workload = workload.name(), path = %path.display(), "wrote default skill");
    }
}

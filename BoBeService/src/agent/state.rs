use crate::error::AppError;
use crate::util::paths::{DataPaths, prepare_private_directory, validate_private_tree};

pub(crate) fn prepare(paths: &DataPaths) -> Result<(), AppError> {
    prepare_private_directory(&paths.agent_dir(), "pi authority state")?;
    prepare_private_directory(&paths.agent_dir().join("auth"), "pi authentication state")?;
    prepare_private_directory(&paths.agent_dir().join("sessions"), "pi session state")?;
    validate_private_tree(&paths.agent_dir(), "pi authority state")
}

pub(crate) async fn purge(paths: &DataPaths) -> Result<(), AppError> {
    let root = paths.agent_dir();
    prepare(paths)?;
    let mut entries = tokio::fs::read_dir(&root).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_name() == "auth" {
            validate_private_tree(&entry.path(), "pi authentication state")?;
            continue;
        }
        crate::util::durable_fs::durable_remove_entry(&entry.path()).await?;
    }
    prepare(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn purge_removes_context_but_keeps_authority_credentials() {
        let root = std::env::temp_dir().join(format!("bobe-pi-state-{}", uuid::Uuid::new_v4()));
        let paths = DataPaths::resolve(&root, &format!("sqlite:{}", root.join("db").display()))
            .unwrap()
            .prepare_root()
            .unwrap();
        prepare(&paths).unwrap();
        tokio::fs::write(paths.agent_dir().join("auth/test.json"), b"credential")
            .await
            .unwrap();
        tokio::fs::write(paths.agent_dir().join("session.json"), b"pointer")
            .await
            .unwrap();
        tokio::fs::write(paths.agent_dir().join("sessions/test.jsonl"), b"private")
            .await
            .unwrap();
        purge(&paths).await.unwrap();
        assert!(paths.agent_dir().join("auth/test.json").is_file());
        assert!(!paths.agent_dir().join("session.json").exists());
        assert!(!paths.agent_dir().join("sessions/test.jsonl").exists());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::Mutex;

use crate::error::AppError;

const PRIMARY_SKILL_ID: &str = "chat";
const LEARNED_DIRECTORY: &str = "learned";
const MAX_SKILLS: usize = 64;
const MAX_DESCRIPTION_BYTES: usize = 240;
const MAX_SKILL_BODY_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SkillSummary {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) primary: bool,
}

pub(crate) struct SkillStore {
    root: PathBuf,
    write_lock: Mutex<()>,
}

#[derive(Debug, Default)]
pub(crate) struct SkillPurgeReport {
    pub(crate) learned_skills: usize,
    pub(crate) primary_reset: bool,
}

impl SkillStore {
    pub(crate) async fn prepare(
        paths: &crate::util::paths::DataPaths,
    ) -> Result<Arc<Self>, AppError> {
        let root = paths.prepare_skills_dir()?;
        tokio::fs::create_dir_all(root.join(LEARNED_DIRECTORY)).await?;
        Ok(Arc::new(Self {
            root,
            write_lock: Mutex::new(()),
        }))
    }

    pub(crate) async fn list(&self) -> Result<Vec<SkillSummary>, AppError> {
        let mut skills = Vec::new();
        let primary_path = self.primary_path();
        if primary_path.is_file() {
            let content = read_regular_file(&primary_path).await?;
            let metadata = parse_metadata(&content)?;
            skills.push(SkillSummary {
                id: PRIMARY_SKILL_ID.into(),
                name: metadata.name,
                description: metadata.description,
                primary: true,
            });
        }

        let learned = self.learned_root();
        let mut entries = match tokio::fs::read_dir(&learned).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(skills),
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            if skills.len() > MAX_SKILLS {
                break;
            }
            let metadata = entry.metadata().await?;
            if !metadata.is_dir() || entry.file_type().await?.is_symlink() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            if validate_skill_name(&id).is_err() {
                continue;
            }
            let path = entry.path().join("SKILL.md");
            if !path.is_file() {
                continue;
            }
            let content = read_regular_file(&path).await?;
            let parsed = parse_metadata(&content)?;
            skills.push(SkillSummary {
                id,
                name: parsed.name,
                description: parsed.description,
                primary: false,
            });
        }
        skills.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(skills)
    }

    pub(crate) async fn read(&self, id: &str) -> Result<String, AppError> {
        let path = self.skill_path(id)?;
        read_regular_file(&path).await
    }

    pub(crate) async fn create_activated<F, Fut>(
        &self,
        name: String,
        description: String,
        body: String,
        mut activate: F,
    ) -> Result<SkillSummary, AppError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<(), AppError>>,
    {
        validate_skill_input(&name, &description, &body)?;
        let _guard = self.write_lock.lock().await;
        let target = self.learned_root().join(&name).join("SKILL.md");
        let previous = match read_regular_file(&target).await {
            Ok(content) => Some(content),
            Err(AppError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let skill = self.publish_unlocked(name, description, body).await?;
        if let Err(activation_error) = activate().await {
            let rollback = match previous {
                Some(content) => {
                    crate::util::durable_fs::atomic_write(&target, content.as_bytes()).await
                }
                None => crate::util::durable_fs::durable_remove_file(&target)
                    .await
                    .map(drop),
            };
            let reload_rollback = activate().await;
            return Err(match (rollback, reload_rollback) {
                (Ok(()), Ok(())) => activation_error,
                (rollback, reload) => AppError::Internal(format!(
                    "skill activation failed: {activation_error}; rollback: {}; runtime rollback: {}",
                    format_result(rollback),
                    format_result(reload)
                )),
            });
        }
        Ok(skill)
    }

    pub(crate) fn directories(&self) -> Vec<PathBuf> {
        vec![self.root.clone(), self.learned_root()]
    }

    pub(crate) async fn purge_personal_state(&self) -> Result<SkillPurgeReport, AppError> {
        let _guard = self.write_lock.lock().await;
        let learned_root = self.learned_root();
        let current_primary = read_regular_file(&self.primary_path()).await?;
        let report = SkillPurgeReport {
            learned_skills: count_skill_directories(&learned_root).await?,
            primary_reset: current_primary != super::skills::CHAT_SKILL_MD,
        };

        crate::util::durable_fs::durable_remove_dir_all(&learned_root).await?;
        tokio::fs::create_dir_all(&learned_root).await?;
        crate::util::durable_fs::atomic_write(
            &self.primary_path(),
            super::skills::CHAT_SKILL_MD.as_bytes(),
        )
        .await?;
        Ok(report)
    }

    async fn publish_unlocked(
        &self,
        name: String,
        description: String,
        body: String,
    ) -> Result<SkillSummary, AppError> {
        self.ensure_canonical_name_available(&name).await?;
        let target_dir = self.learned_root().join(&name);
        let target = target_dir.join("SKILL.md");
        if !target.is_file()
            && self
                .list()
                .await?
                .into_iter()
                .filter(|skill| !skill.primary)
                .count()
                >= MAX_SKILLS
        {
            return Err(AppError::Validation(format!(
                "at most {MAX_SKILLS} learned skills may be active"
            )));
        }
        ensure_contained_directory(&self.learned_root(), &target_dir).await?;
        let description = description.trim().to_owned();
        let rendered = render_skill(&name, &description, &body)?;
        parse_metadata(&rendered)?;
        crate::util::durable_fs::atomic_write(&target, rendered.as_bytes()).await?;
        Ok(SkillSummary {
            id: name.clone(),
            name,
            description,
            primary: false,
        })
    }

    async fn ensure_canonical_name_available(&self, candidate: &str) -> Result<(), AppError> {
        let primary_content = read_regular_file(&self.primary_path()).await?;
        let primary_name = parse_metadata(&primary_content)?.name;
        if candidate == PRIMARY_SKILL_ID || candidate == primary_name {
            return Err(AppError::Validation(format!(
                "'{candidate}' is reserved for BoBe's primary behavior skill"
            )));
        }
        if self
            .list()
            .await?
            .into_iter()
            .any(|skill| !skill.primary && skill.name == candidate && skill.id != candidate)
        {
            return Err(AppError::Conflict(format!(
                "another learned skill already uses canonical name '{candidate}'"
            )));
        }
        Ok(())
    }

    fn primary_path(&self) -> PathBuf {
        self.root.join(PRIMARY_SKILL_ID).join("SKILL.md")
    }

    fn learned_root(&self) -> PathBuf {
        self.root.join(LEARNED_DIRECTORY)
    }

    fn skill_path(&self, id: &str) -> Result<PathBuf, AppError> {
        if id == PRIMARY_SKILL_ID {
            return Ok(self.primary_path());
        }
        validate_skill_name(id)?;
        Ok(self.learned_root().join(id).join("SKILL.md"))
    }
}

#[derive(Debug)]
struct SkillMetadata {
    name: String,
    description: String,
}

fn parse_metadata(content: &str) -> Result<SkillMetadata, AppError> {
    let rest = content
        .strip_prefix("---\n")
        .ok_or_else(|| AppError::Validation("skill frontmatter is missing".into()))?;
    let (frontmatter, body) = rest
        .split_once("\n---\n")
        .ok_or_else(|| AppError::Validation("skill frontmatter is not terminated".into()))?;
    if body.trim().is_empty() {
        return Err(AppError::Validation("skill body must not be empty".into()));
    }
    let mut name = None;
    let mut description = None;
    for line in frontmatter.lines() {
        let Some((key, value)) = line.split_once(':') else {
            return Err(AppError::Validation(
                "skill frontmatter must contain key/value lines".into(),
            ));
        };
        let value = value.trim();
        match key.trim() {
            "name" => name = Some(value.to_owned()),
            "description" => {
                description = Some(if value.starts_with('"') {
                    serde_json::from_str(value).map_err(|error| {
                        AppError::Validation(format!(
                            "skill description string is invalid: {error}"
                        ))
                    })?
                } else {
                    value.to_owned()
                });
            }
            "version" | "author" | "license" => {}
            "allowed-tools" | "hooks" | "mcp" => {
                return Err(AppError::Validation(format!(
                    "skill frontmatter field '{}' is not permitted",
                    key.trim()
                )));
            }
            other => {
                return Err(AppError::Validation(format!(
                    "unknown skill frontmatter field: {other}"
                )));
            }
        }
    }
    let name = name.ok_or_else(|| AppError::Validation("skill name is missing".into()))?;
    let description =
        description.ok_or_else(|| AppError::Validation("skill description is missing".into()))?;
    validate_description(&description)?;
    Ok(SkillMetadata { name, description })
}

fn render_skill(name: &str, description: &str, body: &str) -> Result<String, AppError> {
    let description = serde_json::to_string(description.trim())?;
    Ok(format!(
        "---\nname: {name}\ndescription: {description}\nversion: 1\nauthor: BoBe\nlicense: MIT\n---\n\n{}\n",
        body.trim()
    ))
}

fn validate_skill_input(name: &str, description: &str, body: &str) -> Result<(), AppError> {
    validate_skill_name(name)?;
    validate_description(description)?;
    validate_body(body)
}

fn validate_skill_name(name: &str) -> Result<(), AppError> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(AppError::Validation(
            "skill name must be 1-64 lowercase letters, digits, or hyphens".into(),
        ))
    }
}

fn validate_description(description: &str) -> Result<(), AppError> {
    let trimmed = description.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_DESCRIPTION_BYTES || trimmed.contains(['\r', '\n'])
    {
        Err(AppError::Validation(format!(
            "skill description must be one line and at most {MAX_DESCRIPTION_BYTES} bytes"
        )))
    } else {
        Ok(())
    }
}

fn validate_body(body: &str) -> Result<(), AppError> {
    let trimmed = body.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_SKILL_BODY_BYTES || trimmed.contains('\0') {
        Err(AppError::Validation(format!(
            "skill body must be 1-{MAX_SKILL_BODY_BYTES} bytes without NUL characters"
        )))
    } else {
        Ok(())
    }
}

async fn read_regular_file(path: &Path) -> Result<String, AppError> {
    let metadata = tokio::fs::symlink_metadata(path).await?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::Config(format!(
            "skill path must be a regular file: {}",
            path.display()
        )));
    }
    let bytes = tokio::fs::read(path).await?;
    if bytes.len() > MAX_SKILL_BODY_BYTES + 4_096 {
        return Err(AppError::Config(format!(
            "skill file is too large: {}",
            path.display()
        )));
    }
    String::from_utf8(bytes)
        .map_err(|error| AppError::Config(format!("skill file is not UTF-8: {error}")))
}

async fn count_skill_directories(root: &Path) -> Result<usize, AppError> {
    let mut entries = match tokio::fs::read_dir(root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    let mut count = 0;
    while let Some(entry) = entries.next_entry().await? {
        let file_type = entry.file_type().await?;
        if file_type.is_dir() && !file_type.is_symlink() && entry.path().join("SKILL.md").is_file()
        {
            count += 1;
        }
    }
    Ok(count)
}

async fn ensure_contained_directory(root: &Path, target: &Path) -> Result<(), AppError> {
    tokio::fs::create_dir_all(root).await?;
    let root = tokio::fs::canonicalize(root).await?;
    if !target.starts_with(&root) {
        return Err(AppError::Validation(
            "skill directory escapes the canonical skill root".into(),
        ));
    }
    if let Ok(metadata) = tokio::fs::symlink_metadata(target).await
        && metadata.file_type().is_symlink()
    {
        return Err(AppError::Validation(
            "skill directory must not be a symbolic link".into(),
        ));
    }
    tokio::fs::create_dir_all(target).await?;
    let canonical = tokio::fs::canonicalize(target).await?;
    if !canonical.starts_with(&root) {
        return Err(AppError::Validation(
            "skill directory resolves outside the skill root".into(),
        ));
    }
    Ok(())
}

fn format_result(result: Result<(), AppError>) -> String {
    match result {
        Ok(()) => "ok".into(),
        Err(error) => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (PathBuf, crate::util::paths::DataPaths) {
        let root = std::env::temp_dir().join(format!("bobe-skills-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("skills/chat")).unwrap();
        std::fs::write(
            root.join("skills/chat/SKILL.md"),
            super::super::skills::CHAT_SKILL_MD,
        )
        .unwrap();
        let paths = crate::util::paths::DataPaths::resolve(
            &root,
            &format!("sqlite:{}", root.join("data.sqlite3").display()),
        )
        .unwrap()
        .prepare_root()
        .unwrap();
        (root, paths)
    }

    #[tokio::test]
    async fn create_activates_and_replaces_skill_atomically() {
        let (root, paths) = paths();
        let store = SkillStore::prepare(&paths).await.unwrap();

        let created = store
            .create_activated(
                "release-check".into(),
                "Verify a release before publishing".into(),
                "# Release check\n\nRun the repository gates.".into(),
                || async { Ok(()) },
            )
            .await
            .unwrap();
        assert_eq!(created.id, "release-check");
        assert!(
            store
                .read("release-check")
                .await
                .unwrap()
                .contains("Run the repository gates")
        );

        store
            .create_activated(
                "release-check".into(),
                "Verify every release".into(),
                "# Release check\n\nRun all product gates.".into(),
                || async { Ok(()) },
            )
            .await
            .unwrap();
        let replaced = store.read("release-check").await.unwrap();
        assert!(replaced.contains("Run all product gates"));
        assert!(!replaced.contains("Run the repository gates"));
        drop(tokio::fs::remove_dir_all(root).await);
    }

    #[tokio::test]
    async fn primary_names_are_reserved() {
        let (root, paths) = paths();
        let store = SkillStore::prepare(&paths).await.unwrap();

        for name in ["chat", "bobe-chat"] {
            let error = store
                .create_activated(
                    name.into(),
                    "Shadow the primary skill".into(),
                    "# Duplicate\n\nThis must not be created.".into(),
                    || async { Ok(()) },
                )
                .await
                .unwrap_err();
            assert!(error.to_string().contains("reserved"));
        }
        drop(tokio::fs::remove_dir_all(root).await);
    }

    #[tokio::test]
    async fn create_rechecks_other_learned_canonical_names() {
        let (root, paths) = paths();
        let store = SkillStore::prepare(&paths).await.unwrap();
        let other = store.learned_root().join("other");
        tokio::fs::create_dir_all(&other).await.unwrap();
        tokio::fs::write(
            other.join("SKILL.md"),
            render_skill(
                "collision",
                "Existing canonical collision",
                "# Existing\n\nKeep this skill.",
            )
            .unwrap(),
        )
        .await
        .unwrap();

        let error = store
            .create_activated(
                "collision".into(),
                "Collision candidate".into(),
                "# Candidate\n\nDo the candidate procedure.".into(),
                || async { Ok(()) },
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("canonical name"));
        drop(tokio::fs::remove_dir_all(root).await);
    }

    #[tokio::test]
    async fn purge_resets_primary_and_removes_learned_state() {
        let (root, paths) = paths();
        let store = SkillStore::prepare(&paths).await.unwrap();
        tokio::fs::write(
            store.primary_path(),
            render_skill(
                "bobe-chat",
                "Customized companion behavior",
                "# Customized primary\n\nKeep this only until privacy purge.",
            )
            .unwrap(),
        )
        .await
        .unwrap();
        store
            .create_activated(
                "temporary-check".into(),
                "Temporary reusable check".into(),
                "# Check\n\nRun it.".into(),
                || async { Ok(()) },
            )
            .await
            .unwrap();

        let report = store.purge_personal_state().await.unwrap();

        assert_eq!(report.learned_skills, 1);
        assert!(report.primary_reset);
        assert_eq!(
            tokio::fs::read_to_string(store.primary_path())
                .await
                .unwrap(),
            super::super::skills::CHAT_SKILL_MD
        );
        assert!(store.read("temporary-check").await.is_err());
        drop(tokio::fs::remove_dir_all(root).await);
    }

    #[test]
    fn rejects_permission_grants_in_frontmatter() {
        let error =
            parse_metadata("---\nname: bad\ndescription: bad\nallowed-tools: shell\n---\n\nDo it.")
                .unwrap_err();
        assert!(error.to_string().contains("not permitted"));
    }

    #[test]
    fn rendered_description_quotes_yaml_significant_text() {
        let rendered = render_skill(
            "release-check",
            "Verify release: before publishing",
            "# Release\n\nVerify it.",
        )
        .unwrap();

        assert!(rendered.contains(r#"description: "Verify release: before publishing""#));
        assert_eq!(
            parse_metadata(&rendered).unwrap().description,
            "Verify release: before publishing"
        );
    }
}

use std::sync::Arc;

use chrono::Utc;
use tracing::{info, warn};

use crate::error::AppError;
use crate::models::ids::GoalId;
use crate::models::types::GoalStatus;

use super::file_store::GoalFileStore;
use super::goal_md::GoalDoc;

pub(crate) struct GoalsService {
    store: Arc<GoalFileStore>,
}

pub(crate) struct GoalPatch {
    pub(crate) title: Option<String>,
    pub(crate) status: Option<GoalStatus>,
    pub(crate) priority: Option<u8>,
    pub(crate) summary: Option<String>,
    pub(crate) why_it_matters: Option<String>,
    pub(crate) how_working_on_it: Option<String>,
    pub(crate) patterns_observed: Option<String>,
    pub(crate) attitude_feelings: Option<String>,
    pub(crate) open_questions: Option<String>,
    pub(crate) notes: Option<String>,
}

impl GoalsService {
    pub(crate) fn new(store: Arc<GoalFileStore>) -> Self {
        Self { store }
    }

    pub(crate) async fn list_all(&self) -> Result<Vec<GoalDoc>, AppError> {
        let mut goals = self.store.list().await?;
        sort_goals(&mut goals);
        Ok(goals)
    }

    pub(crate) async fn list_active(&self) -> Result<Vec<GoalDoc>, AppError> {
        let mut goals = self.store.list().await?;
        goals.retain(|g| g.status == GoalStatus::Active);
        sort_goals(&mut goals);
        Ok(goals)
    }

    pub(crate) async fn get(&self, id: GoalId) -> Result<Option<GoalDoc>, AppError> {
        self.store.get(id).await
    }

    pub(crate) async fn create(&self, mut doc: GoalDoc) -> Result<GoalDoc, AppError> {
        doc.id = GoalId::new();
        let doc = prepare_new(doc)?;
        self.store.save(&doc).await?;
        info!(
            goal_id = %doc.id,
            title = %doc.title,
            "goals_service.created"
        );
        Ok(doc)
    }

    pub(crate) async fn create_if_absent(&self, doc: GoalDoc) -> Result<bool, AppError> {
        let doc = prepare_new(doc)?;
        let created = self.store.save_if_absent(&doc).await?;
        if created {
            info!(
                goal_id = %doc.id,
                title = %doc.title,
                "goals_service.created"
            );
        }
        Ok(created)
    }

    pub(crate) async fn update(
        &self,
        id: GoalId,
        patch: GoalPatch,
    ) -> Result<Option<GoalDoc>, AppError> {
        let GoalPatch {
            title,
            status,
            priority,
            summary,
            why_it_matters,
            how_working_on_it,
            patterns_observed,
            attitude_feelings,
            open_questions,
            notes,
        } = patch;
        let title = title.map(|title| normalize_title(&title)).transpose()?;
        if let Some(priority) = priority {
            validate_priority(priority)?;
        }
        for (field, value) in [
            ("summary", summary.as_deref()),
            ("why_it_matters", why_it_matters.as_deref()),
            ("how_working_on_it", how_working_on_it.as_deref()),
            ("patterns_observed", patterns_observed.as_deref()),
            ("attitude_feelings", attitude_feelings.as_deref()),
            ("open_questions", open_questions.as_deref()),
            ("notes", notes.as_deref()),
        ] {
            if let Some(value) = value {
                validate_section_text(field, value)?;
            }
        }
        let updated = self
            .store
            .update(id, move |mut doc| {
                let mut changed = false;
                if let Some(t) = title {
                    doc.title = t;
                    changed = true;
                }
                if let Some(s) = status {
                    doc.status = s;
                    changed = true;
                }
                if let Some(p) = priority {
                    doc.priority = p;
                    changed = true;
                }
                if let Some(s) = summary {
                    doc.summary = s;
                    changed = true;
                }
                if let Some(w) = why_it_matters {
                    doc.why_it_matters = w;
                    changed = true;
                }
                if let Some(h) = how_working_on_it {
                    doc.how_working_on_it = h;
                    changed = true;
                }
                if let Some(p) = patterns_observed {
                    doc.patterns_observed = p;
                    changed = true;
                }
                if let Some(a) = attitude_feelings {
                    doc.attitude_feelings = a;
                    changed = true;
                }
                if let Some(q) = open_questions {
                    doc.open_questions = q;
                    changed = true;
                }
                if let Some(n) = notes {
                    doc.notes = n;
                    changed = true;
                }
                if changed {
                    doc.updated_at = Utc::now();
                }
                doc
            })
            .await?;

        if updated.is_some() {
            info!(goal_id = %id, "goals_service.updated");
        }
        Ok(updated)
    }

    pub(crate) async fn set_status(
        &self,
        id: GoalId,
        status: GoalStatus,
    ) -> Result<Option<GoalDoc>, AppError> {
        self.update(
            id,
            GoalPatch {
                title: None,
                status: Some(status),
                priority: None,
                summary: None,
                why_it_matters: None,
                how_working_on_it: None,
                patterns_observed: None,
                attitude_feelings: None,
                open_questions: None,
                notes: None,
            },
        )
        .await
    }

    pub(crate) async fn delete_all(&self) -> Result<usize, AppError> {
        self.store.delete_all().await
    }

    pub(crate) async fn delete(&self, id: GoalId) -> Result<bool, AppError> {
        let removed = self.store.delete(id).await?;
        if removed {
            info!(goal_id = %id, "goals_service.deleted");
        } else {
            warn!(goal_id = %id, "goals_service.delete_missing");
        }
        Ok(removed)
    }
}

fn prepare_new(mut doc: GoalDoc) -> Result<GoalDoc, AppError> {
    doc.title = normalize_title(&doc.title)?;
    validate_priority(doc.priority)?;
    validate_goal_sections(&doc)?;
    let now = Utc::now();
    doc.created_at = now;
    doc.updated_at = now;
    Ok(doc)
}

fn normalize_title(title: &str) -> Result<String, AppError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(AppError::Validation("title must not be empty".into()));
    }
    if title.contains(['\r', '\n']) {
        return Err(AppError::Validation("title must be a single line".into()));
    }
    Ok(title.to_owned())
}

fn validate_priority(priority: u8) -> Result<(), AppError> {
    if priority > 5 {
        return Err(AppError::Validation(
            "priority must be 0-5 (5 = highest)".into(),
        ));
    }
    Ok(())
}

fn validate_goal_sections(doc: &GoalDoc) -> Result<(), AppError> {
    for (field, value) in [
        ("summary", doc.summary.as_str()),
        ("why_it_matters", doc.why_it_matters.as_str()),
        ("how_working_on_it", doc.how_working_on_it.as_str()),
        ("patterns_observed", doc.patterns_observed.as_str()),
        ("attitude_feelings", doc.attitude_feelings.as_str()),
        ("open_questions", doc.open_questions.as_str()),
        ("notes", doc.notes.as_str()),
    ] {
        validate_section_text(field, value)?;
    }
    Ok(())
}

fn validate_section_text(field: &str, value: &str) -> Result<(), AppError> {
    if value
        .lines()
        .map(str::trim_start)
        .any(|line| line.starts_with("# ") || line.starts_with("## "))
    {
        return Err(AppError::Validation(format!(
            "{field} must not contain Markdown section headings"
        )));
    }
    Ok(())
}

fn sort_goals(goals: &mut [GoalDoc]) {
    goals.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then(b.updated_at.cmp(&a.updated_at))
    });
}

#[cfg(test)]
mod tests {
    use super::{normalize_title, validate_section_text};

    #[test]
    fn title_rejects_markdown_line_injection() {
        let error = normalize_title("Goal\n## Injected section").err();
        assert!(
            error
                .as_ref()
                .is_some_and(|error| error.to_string().contains("single line"))
        );
    }

    #[test]
    fn section_text_rejects_canonical_heading_injection() {
        for value in [
            "Normal\n# Replaced title",
            "Normal\n## Notes\nInjected",
            "  ## Notes",
        ] {
            let error = validate_section_text("summary", value).err();
            assert!(
                error.as_ref().is_some_and(|error| {
                    error.to_string().contains("Markdown section headings")
                })
            );
        }
    }
}

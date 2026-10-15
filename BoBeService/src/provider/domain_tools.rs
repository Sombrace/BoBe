use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::models::ids::GoalId;
use crate::models::types::GoalStatus;
use crate::services::goals::goal_md::GoalDoc;
use crate::services::goals::goals_service::{GoalPatch, GoalsService};

use super::{MemoryFile, SkillStore};

pub(crate) const MEMORY_APPEND: &str = "bobe_memory_append";
pub(crate) const MEMORY_READ: &str = "bobe_memory_read";
pub(crate) const MEMORY_REPLACE: &str = "bobe_memory_replace";
pub(crate) const MEMORY_REMOVE: &str = "bobe_memory_remove";
pub(crate) const GOAL_LIST: &str = "bobe_goal_list";
pub(crate) const GOAL_CREATE: &str = "bobe_goal_create";
pub(crate) const GOAL_UPDATE: &str = "bobe_goal_update";
pub(crate) const SKILL_LIST: &str = "bobe_skill_list";
pub(crate) const SKILL_READ: &str = "bobe_skill_read";
pub(crate) const SKILL_CREATE: &str = "bobe_skill_create";

#[derive(Debug, Clone)]
pub(crate) struct ToolSpec {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Capabilities {
    pub(crate) memory: bool,
    pub(crate) goals: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct ToolOutcome {
    pub(crate) success: bool,
    pub(crate) value: Value,
}

pub(crate) struct DomainTools {
    memory: Arc<MemoryFile>,
    goals: Arc<GoalsService>,
    skills: Arc<SkillStore>,
    memory_enabled: bool,
    goals_enabled: bool,
}

impl DomainTools {
    pub(crate) async fn require_goal(
        &self,
        goal: Option<GoalId>,
    ) -> Result<(), crate::error::AppError> {
        if let Some(goal) = goal {
            if !self.goals_enabled {
                return Err(crate::error::AppError::Conflict(
                    "goals are disabled".into(),
                ));
            }
            self.goals
                .get(goal)
                .await?
                .ok_or_else(|| crate::error::AppError::NotFound("goal was not found".into()))?;
        }
        Ok(())
    }

    pub(crate) fn new(
        memory: Arc<MemoryFile>,
        goals: Arc<GoalsService>,
        skills: Arc<SkillStore>,
        memory_enabled: bool,
        goals_enabled: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            memory,
            goals,
            skills,
            memory_enabled,
            goals_enabled,
        })
    }

    pub(crate) fn specs(&self) -> Vec<ToolSpec> {
        let mut specs = Vec::new();
        if self.memory_enabled {
            specs.extend([
                spec(
                MEMORY_READ,
                "Read BoBe's current memory document and revision before replacing or removing an entry.",
            ),
            spec(
                MEMORY_APPEND,
                "Append one durable fact to BoBe's structured memory. Never use for transient chat.",
            ),
                spec(
                    MEMORY_REPLACE,
                    "Replace one exact current memory line using the revision returned by bobe_memory_read.",
                ),
                spec(
                    MEMORY_REMOVE,
                    "Remove one exact current memory line using the revision returned by bobe_memory_read.",
                ),
            ]);
        }
        if self.goals_enabled {
            specs.extend([
                spec(
                    GOAL_LIST,
                    "List the user's BoBe goals before discussing or changing them.",
                ),
                spec(
                    GOAL_CREATE,
                    "Create a durable, personally meaningful goal when tracking it will help the user.",
                ),
                spec(
                    GOAL_UPDATE,
                    "Update an existing goal's living-document fields or lifecycle status.",
                ),
            ]);
        }
        specs.extend([
            spec(SKILL_LIST, "List reusable BoBe skills."),
            spec(SKILL_READ, "Read one reusable skill before applying it."),
            spec(
                SKILL_CREATE,
                "Atomically create or replace a reusable skill and make it available to BoBe.",
            ),
        ]);
        specs
    }

    pub(crate) fn tool_spec(&self, name: &str) -> Option<ToolSpec> {
        self.specs().into_iter().find(|spec| spec.name == name)
    }

    pub(crate) fn memory_file(&self) -> Arc<MemoryFile> {
        Arc::clone(&self.memory)
    }

    pub(crate) const fn memory_enabled(&self) -> bool {
        self.memory_enabled
    }

    pub(crate) const fn capabilities(&self) -> Capabilities {
        Capabilities {
            memory: self.memory_enabled,
            goals: self.goals_enabled,
        }
    }

    pub(crate) fn is_enabled(&self, name: &str) -> bool {
        self.tool_spec(name).is_some()
    }

    pub(crate) async fn invoke(&self, name: &str, arguments: Value) -> ToolOutcome {
        if !self.is_enabled(name) {
            return ToolOutcome::failure(format!("tool is disabled or unknown: {name}"));
        }
        match name {
            MEMORY_READ => self.read_memory().await,
            MEMORY_APPEND => match decode(arguments) {
                Ok(params) => self.append_memory(params).await,
                Err(error) => invalid_arguments(name, error),
            },
            GOAL_LIST => match decode(arguments) {
                Ok(params) => self.list_goals(params).await,
                Err(error) => invalid_arguments(name, error),
            },
            GOAL_CREATE => match decode(arguments) {
                Ok(params) => self.create_goal(params).await,
                Err(error) => invalid_arguments(name, error),
            },
            GOAL_UPDATE => match decode(arguments) {
                Ok(params) => self.update_goal(params).await,
                Err(error) => invalid_arguments(name, error),
            },
            MEMORY_REPLACE => match decode(arguments) {
                Ok(params) => self.replace_memory(params).await,
                Err(error) => invalid_arguments(name, error),
            },
            MEMORY_REMOVE => match decode(arguments) {
                Ok(params) => self.remove_memory(params).await,
                Err(error) => invalid_arguments(name, error),
            },
            SKILL_LIST => self.list_skills().await,
            SKILL_READ => match decode(arguments) {
                Ok(params) => self.read_skill(params).await,
                Err(error) => invalid_arguments(name, error),
            },
            SKILL_CREATE => ToolOutcome::failure(
                "bobe_skill_create requires the active pi session for skill activation",
            ),
            _ => ToolOutcome::failure(format!("unknown tool: {name}")),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_default(
        memory: Arc<MemoryFile>,
        skills: Arc<SkillStore>,
        memory_enabled: bool,
        goals_enabled: bool,
    ) -> Arc<Self> {
        let root = std::env::temp_dir().join(format!("bobe-test-tools-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("test data root");
        Self::new(
            memory,
            Arc::new(GoalsService::new(
                crate::services::goals::file_store::GoalFileStore::new(root.join("goals")),
            )),
            skills,
            memory_enabled,
            goals_enabled,
        )
    }

    async fn read_memory(&self) -> ToolOutcome {
        match self.memory.read().await {
            Ok(body) => ToolOutcome::success(json!({
                "ok": true,
                "revision": self.memory.revision(),
                "body": body,
            })),
            Err(error) => ToolOutcome::failure(error),
        }
    }

    async fn append_memory(&self, params: MemoryAppendParams) -> ToolOutcome {
        if params.text.trim().is_empty() {
            return ToolOutcome::failure("memory text must not be empty");
        }
        if params.text.contains(['\r', '\n']) {
            return ToolOutcome::failure("memory text must be a single line");
        }
        match self
            .memory
            .append_under(params.section.heading(), &params.text)
            .await
        {
            Ok(()) => ToolOutcome::success(json!({
                "ok": true,
                "section": params.section.heading(),
                "revision": self.memory.revision(),
            })),
            Err(error) => ToolOutcome::failure(error),
        }
    }

    async fn list_goals(&self, params: GoalListParams) -> ToolOutcome {
        match self.goals.list_all().await {
            Ok(goals) => {
                let goals = goals
                    .into_iter()
                    .filter(|goal| {
                        params.include_inactive
                            || matches!(goal.status, GoalStatus::Active | GoalStatus::Paused)
                    })
                    .map(|goal| {
                        json!({
                            "id": goal.id, "title": goal.title, "status": goal.status,
                            "priority": goal.priority, "summary": goal.summary,
                            "why_it_matters": goal.why_it_matters,
                            "how_working_on_it": goal.how_working_on_it,
                            "patterns_observed": goal.patterns_observed,
                            "attitude_feelings": goal.attitude_feelings,
                            "open_questions": goal.open_questions, "notes": goal.notes,
                        })
                    })
                    .collect::<Vec<_>>();
                ToolOutcome::success(json!({ "ok": true, "count": goals.len(), "goals": goals }))
            }
            Err(error) => ToolOutcome::failure(error),
        }
    }

    async fn replace_memory(&self, params: MemoryReplaceParams) -> ToolOutcome {
        match self
            .memory
            .replace_exact_line(
                params.expected_revision,
                &params.current_line,
                &params.replacement_line,
            )
            .await
        {
            Ok(()) => ToolOutcome::success(json!({
                "ok": true,
                "revision": self.memory.revision(),
            })),
            Err(error) => ToolOutcome::failure(error),
        }
    }

    async fn remove_memory(&self, params: MemoryRemoveParams) -> ToolOutcome {
        match self
            .memory
            .remove_exact_line(params.expected_revision, &params.current_line)
            .await
        {
            Ok(()) => ToolOutcome::success(json!({
                "ok": true,
                "revision": self.memory.revision(),
            })),
            Err(error) => ToolOutcome::failure(error),
        }
    }

    async fn list_skills(&self) -> ToolOutcome {
        match self.skills.list().await {
            Ok(skills) => ToolOutcome::success(json!({
                "ok": true,
                "count": skills.len(),
                "skills": skills,
            })),
            Err(error) => ToolOutcome::failure(error),
        }
    }

    async fn read_skill(&self, params: SkillReadParams) -> ToolOutcome {
        match self.skills.read(&params.id).await {
            Ok(body) => ToolOutcome::success(json!({
                "ok": true,
                "id": params.id,
                "body": body,
            })),
            Err(error) => ToolOutcome::failure(error),
        }
    }

    async fn create_goal(&self, params: GoalCreateParams) -> ToolOutcome {
        let mut goal = GoalDoc::new(params.title, params.summary);
        goal.why_it_matters = params.why_it_matters;
        goal.priority = params.priority.unwrap_or(2);
        match self.goals.create(goal).await {
            Ok(goal) => ToolOutcome::success(goal_result(&goal)),
            Err(error) => ToolOutcome::failure(error),
        }
    }

    async fn update_goal(&self, params: GoalUpdateParams) -> ToolOutcome {
        let id = match params.id.parse::<GoalId>() {
            Ok(id) => id,
            Err(error) => return ToolOutcome::failure(format!("invalid goal id: {error}")),
        };
        match self
            .goals
            .update(
                id,
                GoalPatch {
                    title: params.title,
                    status: params.status.map(GoalStatus::from),
                    priority: params.priority,
                    summary: params.summary,
                    why_it_matters: params.why_it_matters,
                    how_working_on_it: params.how_working_on_it,
                    patterns_observed: params.patterns_observed,
                    attitude_feelings: params.attitude_feelings,
                    open_questions: params.open_questions,
                    notes: params.notes,
                },
            )
            .await
        {
            Ok(Some(goal)) => ToolOutcome::success(goal_result(&goal)),
            Ok(None) => ToolOutcome::failure(format!("goal {id} was not found")),
            Err(error) => ToolOutcome::failure(error),
        }
    }
}

impl ToolOutcome {
    pub(crate) fn success(value: Value) -> Self {
        Self {
            success: true,
            value,
        }
    }

    pub(crate) fn failure(error: impl std::fmt::Display) -> Self {
        Self {
            success: false,
            value: json!({ "ok": false, "error": error.to_string() }),
        }
    }
}

fn spec(name: &'static str, description: &'static str) -> ToolSpec {
    ToolSpec { name, description }
}

fn invalid_arguments(name: &str, error: serde_json::Error) -> ToolOutcome {
    ToolOutcome::failure(format!("invalid arguments for {name}: {error}"))
}

fn decode<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, serde_json::Error> {
    serde_json::from_value(value)
}

#[derive(Debug, Deserialize, serde::Serialize, JsonSchema)]
pub(crate) struct MemoryReadParams {}

#[derive(Debug, Clone, Copy, Deserialize, serde::Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemorySection {
    Profile,
    ActiveGoals,
    LongTerm,
    Recent,
}

impl MemorySection {
    const fn heading(self) -> &'static str {
        match self {
            Self::Profile => "Profile",
            Self::ActiveGoals => "Active Goals",
            Self::LongTerm => "Long-term",
            Self::Recent => "Recent",
        }
    }
}

#[derive(Debug, Deserialize, serde::Serialize, JsonSchema)]
pub(crate) struct MemoryAppendParams {
    section: MemorySection,
    text: String,
}

#[derive(Debug, Deserialize, serde::Serialize, JsonSchema)]
pub(crate) struct MemoryReplaceParams {
    expected_revision: u64,
    current_line: String,
    replacement_line: String,
}

#[derive(Debug, Deserialize, serde::Serialize, JsonSchema)]
pub(crate) struct MemoryRemoveParams {
    expected_revision: u64,
    current_line: String,
}

#[derive(Debug, Deserialize, serde::Serialize, JsonSchema)]
pub(crate) struct GoalListParams {
    #[serde(default)]
    include_inactive: bool,
}

#[derive(Debug, Deserialize, serde::Serialize, JsonSchema)]
pub(crate) struct GoalCreateParams {
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    why_it_matters: String,
    priority: Option<u8>,
}

#[derive(Debug, Clone, Copy, Deserialize, serde::Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum GoalStatusParam {
    Active,
    Paused,
    Completed,
    Archived,
}

impl From<GoalStatusParam> for GoalStatus {
    fn from(status: GoalStatusParam) -> Self {
        match status {
            GoalStatusParam::Active => Self::Active,
            GoalStatusParam::Paused => Self::Paused,
            GoalStatusParam::Completed => Self::Completed,
            GoalStatusParam::Archived => Self::Archived,
        }
    }
}

#[derive(Debug, Deserialize, serde::Serialize, JsonSchema)]
pub(crate) struct GoalUpdateParams {
    id: String,
    title: Option<String>,
    status: Option<GoalStatusParam>,
    priority: Option<u8>,
    summary: Option<String>,
    why_it_matters: Option<String>,
    how_working_on_it: Option<String>,
    patterns_observed: Option<String>,
    attitude_feelings: Option<String>,
    open_questions: Option<String>,
    notes: Option<String>,
}

#[derive(Debug, Deserialize, serde::Serialize, JsonSchema)]
pub(crate) struct SkillListParams {}

#[derive(Debug, Deserialize, serde::Serialize, JsonSchema)]
pub(crate) struct SkillReadParams {
    id: String,
}

#[derive(Debug, Deserialize, serde::Serialize, JsonSchema)]
pub(crate) struct SkillCreateParams {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) body: String,
}

fn goal_result(goal: &GoalDoc) -> Value {
    json!({
        "ok": true, "id": goal.id, "title": goal.title, "status": goal.status,
        "priority": goal.priority, "summary": goal.summary,
        "why_it_matters": goal.why_it_matters, "how_working_on_it": goal.how_working_on_it,
        "patterns_observed": goal.patterns_observed, "attitude_feelings": goal.attitude_feelings,
        "open_questions": goal.open_questions, "notes": goal.notes, "updated_at": goal.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use crate::services::goals::file_store::GoalFileStore;

    use super::*;

    #[tokio::test]
    async fn executor_validates_and_mutates_real_stores() {
        let root = std::env::temp_dir().join(format!("bobe-tools-{}", uuid::Uuid::new_v4()));
        let memory = MemoryFile::new(root.join("memory.md"));
        let goals = Arc::new(GoalsService::new(GoalFileStore::new(root.join("goals"))));
        std::fs::create_dir_all(&root).unwrap();
        let paths = crate::util::paths::DataPaths::resolve(
            &root,
            &format!("sqlite:{}", root.join("data.sqlite3").display()),
        )
        .unwrap()
        .prepare_root()
        .unwrap();
        let skills = SkillStore::prepare(&paths).await.unwrap();
        let tools = DomainTools::new(
            Arc::clone(&memory),
            Arc::clone(&goals),
            Arc::clone(&skills),
            true,
            true,
        );

        let memory_result = tools
            .invoke(
                MEMORY_APPEND,
                json!({"section":"recent","text":"User likes tea"}),
            )
            .await;
        assert!(memory_result.success);
        assert!(memory.read().await.unwrap().contains("User likes tea"));

        let invalid = tools
            .invoke(GOAL_CREATE, json!({"title":"Too urgent","priority":9}))
            .await;
        assert!(!invalid.success);
        assert!(goals.list_all().await.unwrap().is_empty());
        drop(tokio::fs::remove_dir_all(root).await);
    }
}

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::{Harness, ReasoningEffort};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Store {
    pub schema_version: u32,
    pub projects: BTreeMap<String, ProjectState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectState {
    pub root: String,
    pub active_run: Option<String>,
    pub runs: BTreeMap<String, Run>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub status: RunStatus,
    pub base_branch: String,
    pub base_workspace_id: String,
    #[serde(alias = "orchestrator")]
    pub lead: AgentRef,
    pub created_unix_ms: u128,
    #[serde(alias = "next_worker")]
    pub next_agent: u32,
    #[serde(alias = "workers")]
    pub agents: BTreeMap<String, Agent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRef {
    pub name: String,
    pub harness: Harness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: ReasoningEffort,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub title: String,
    pub task: String,
    pub scope: Vec<String>,
    pub acceptance: Vec<String>,
    #[serde(default = "generalist_role")]
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role_description: String,
    pub harness: Harness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: ReasoningEffort,
    #[serde(default)]
    pub yolo: bool,
    #[serde(default = "worktree_enabled")]
    pub use_worktree: bool,
    pub branch: String,
    pub base_sha: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claimed_commits: Vec<String>,
    pub agent_name: String,
    pub status: AgentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_agent_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<AgentReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Active,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Starting,
    Working,
    Blocked,
    Failed,
    Cancelled,
    Completed,
    Integrating,
    Integrated,
    Conflict,
}

impl AgentStatus {
    pub fn occupies_slot(&self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Working | Self::Blocked | Self::Integrating
        )
    }

    pub fn reserves_scope(&self) -> bool {
        matches!(
            self,
            Self::Starting
                | Self::Working
                | Self::Blocked
                | Self::Completed
                | Self::Integrating
                | Self::Conflict
        )
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Failed | Self::Cancelled | Self::Integrated | Self::Conflict
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRequest {
    pub title: String,
    pub task: String,
    pub scope: Vec<String>,
    pub acceptance: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<Harness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentReport {
    pub status: ReportStatus,
    pub summary: String,
    #[serde(default)]
    pub tests: Vec<String>,
    #[serde(default)]
    pub changed_paths: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Completed,
    Blocked,
    Failed,
}

impl AgentRequest {
    pub fn validate_and_normalize(mut self) -> anyhow::Result<Self> {
        anyhow::ensure!(!self.title.trim().is_empty(), "title cannot be empty");
        anyhow::ensure!(!self.task.trim().is_empty(), "task cannot be empty");
        anyhow::ensure!(!self.scope.is_empty(), "scope cannot be empty");
        anyhow::ensure!(
            !self.acceptance.is_empty(),
            "acceptance criteria cannot be empty"
        );
        self.title = self.title.trim().to_string();
        self.task = self.task.trim().to_string();
        self.role = self.role.map(|role| role.trim().to_string());
        anyhow::ensure!(self.role.as_deref() != Some(""), "role cannot be empty");
        self.scope = self
            .scope
            .into_iter()
            .map(|path| normalize_scope(&path))
            .collect::<anyhow::Result<Vec<_>>>()?;
        self.scope.sort();
        self.scope.dedup();
        self.acceptance = self
            .acceptance
            .into_iter()
            .map(|criterion| criterion.trim().to_string())
            .collect();
        anyhow::ensure!(
            self.acceptance
                .iter()
                .all(|criterion| !criterion.is_empty()),
            "acceptance criteria cannot contain empty values"
        );
        if self
            .model
            .as_deref()
            .is_some_and(|model| model.trim().is_empty())
        {
            anyhow::bail!("model cannot be empty");
        }
        Ok(self)
    }
}

fn generalist_role() -> String {
    crate::config::GENERALIST_ROLE.into()
}

fn worktree_enabled() -> bool {
    true
}

pub fn normalize_scope(value: &str) -> anyhow::Result<String> {
    let value = value.trim().trim_start_matches("./").trim_end_matches('/');
    anyhow::ensure!(!value.is_empty(), "scope cannot contain an empty path");
    let path = std::path::Path::new(value);
    anyhow::ensure!(
        !path.is_absolute(),
        "scope paths must be repository-relative"
    );
    anyhow::ensure!(
        !path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir)),
        "scope paths cannot contain .."
    );
    Ok(value.to_string())
}

pub fn scopes_overlap(left: &[String], right: &[String]) -> bool {
    left.iter().any(|a| {
        right.iter().any(|b| {
            a == b
                || a.strip_prefix(b).is_some_and(|rest| rest.starts_with('/'))
                || b.strip_prefix(a).is_some_and(|rest| rest.starts_with('/'))
        })
    })
}

pub fn path_within_scope(path: &str, scope: &[String]) -> bool {
    scope.iter().any(|allowed| {
        path == allowed
            || path
                .strip_prefix(allowed)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_scopes_until_work_is_integrated_or_abandoned() {
        for status in [
            AgentStatus::Starting,
            AgentStatus::Working,
            AgentStatus::Blocked,
            AgentStatus::Completed,
            AgentStatus::Integrating,
            AgentStatus::Conflict,
        ] {
            assert!(status.reserves_scope(), "{status:?} should reserve scope");
        }
        for status in [
            AgentStatus::Failed,
            AgentStatus::Cancelled,
            AgentStatus::Integrated,
        ] {
            assert!(!status.reserves_scope(), "{status:?} should release scope");
        }
    }

    #[test]
    fn normalizes_and_rejects_unsafe_scope() {
        assert_eq!(normalize_scope("./src/api/").unwrap(), "src/api");
        assert!(normalize_scope("../secret").is_err());
        assert!(normalize_scope("/tmp/file").is_err());
    }

    #[test]
    fn detects_prefix_overlap_only_on_path_boundaries() {
        assert!(scopes_overlap(&["src".into()], &["src/api".into()]));
        assert!(!scopes_overlap(&["src".into()], &["src-old".into()]));
    }

    #[test]
    fn checks_changed_path_against_scope_boundary() {
        assert!(path_within_scope("src/api/mod.rs", &["src/api".into()]));
        assert!(path_within_scope("Cargo.toml", &["Cargo.toml".into()]));
        assert!(!path_within_scope(
            "src/api-old/mod.rs",
            &["src/api".into()]
        ));
    }

    #[test]
    fn normalizes_and_rejects_agent_roles() {
        let request: AgentRequest = serde_json::from_value(serde_json::json!({
            "title": "Test API",
            "task": "Run API tests",
            "scope": ["src/api"],
            "acceptance": ["Tests pass"],
            "role": " qa "
        }))
        .unwrap();
        assert_eq!(
            request.validate_and_normalize().unwrap().role.as_deref(),
            Some("qa")
        );

        let request: AgentRequest = serde_json::from_value(serde_json::json!({
            "title": "Test API",
            "task": "Run API tests",
            "scope": ["src/api"],
            "acceptance": ["Tests pass"],
            "role": " "
        }))
        .unwrap();
        assert!(request.validate_and_normalize().is_err());
    }

    #[test]
    fn treats_agents_from_older_state_as_worktree_agents() {
        let agent: Agent = serde_json::from_value(serde_json::json!({
            "id": "agent-1",
            "title": "Legacy agent",
            "task": "Test compatibility",
            "scope": ["src"],
            "acceptance": ["Tests pass"],
            "harness": "codex",
            "branch": "cadence/legacy/agent-1",
            "base_sha": "abc123",
            "agent_name": "cadence-legacy-a1",
            "status": "working"
        }))
        .unwrap();

        assert!(agent.use_worktree);
        assert!(!agent.yolo);
        assert!(agent.claimed_commits.is_empty());
        assert_eq!(agent.reasoning_effort, ReasoningEffort::Default);
    }

    #[test]
    fn reads_runs_with_older_role_field_names() {
        let run: Run = serde_json::from_value(serde_json::json!({
            "id": "run-legacy",
            "status": "active",
            "base_branch": "main",
            "base_workspace_id": "workspace-1",
            "orchestrator": {
                "name": "cadence-orch-legacy",
                "harness": "codex"
            },
            "created_unix_ms": 1,
            "next_worker": 1,
            "workers": {}
        }))
        .unwrap();

        assert_eq!(run.lead.name, "cadence-orch-legacy");
        assert_eq!(run.next_agent, 1);
        assert!(run.agents.is_empty());
    }
}

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::Harness;

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
    pub orchestrator: AgentRef,
    pub created_unix_ms: u128,
    pub next_worker: u32,
    pub workers: BTreeMap<String, Worker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRef {
    pub name: String,
    pub harness: Harness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Worker {
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
    #[serde(default = "worktree_enabled")]
    pub use_worktree: bool,
    pub branch: String,
    pub base_sha: String,
    pub agent_name: String,
    pub status: WorkerStatus,
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
    pub report: Option<WorkerReport>,
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
pub enum WorkerStatus {
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

impl WorkerStatus {
    pub fn occupies_slot(&self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Working | Self::Blocked | Self::Integrating
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
pub struct WorkerRequest {
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerReport {
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

impl WorkerRequest {
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
    fn normalizes_and_rejects_worker_roles() {
        let request: WorkerRequest = serde_json::from_value(serde_json::json!({
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

        let request: WorkerRequest = serde_json::from_value(serde_json::json!({
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
    fn treats_workers_from_older_state_as_worktree_workers() {
        let worker: Worker = serde_json::from_value(serde_json::json!({
            "id": "worker-1",
            "title": "Legacy worker",
            "task": "Test compatibility",
            "scope": ["src"],
            "acceptance": ["Tests pass"],
            "harness": "codex",
            "branch": "cadence/legacy/worker-1",
            "base_sha": "abc123",
            "agent_name": "cadence-legacy-w1",
            "status": "working"
        }))
        .unwrap();

        assert!(worker.use_worktree);
    }
}

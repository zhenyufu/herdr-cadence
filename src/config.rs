use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const CONFIG_RELATIVE_PATH: &str = ".herdr/cadence.toml";
pub const GENERALIST_ROLE: &str = "generalist";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub enabled: bool,
    #[serde(default)]
    pub yolo: bool,
    #[serde(default = "default_worker_role")]
    pub worker_default: String,
    pub orchestrator: AgentConfig,
    pub git: GitConfig,
    pub workers: WorkerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub harness: Harness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: ReasoningEffort,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parallel: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub roles: BTreeMap<String, RoleConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoleConfig {
    pub description: String,
    pub harness: WorkerHarness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: ReasoningEffort,
    #[serde(default)]
    pub version_control_mode: VersionControlMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GitConfig {
    pub auto_integrate: bool,
    pub cleanup_on_success: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    Codex,
    Opencode,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    #[default]
    Default,
    Low,
    Medium,
    High,
    Xhigh,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VersionControlMode {
    #[default]
    SharedCheckout,
    GitWorktree,
}

impl ReasoningEffort {
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
            Self::Xhigh => Some("xhigh"),
        }
    }
}

impl Harness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Opencode => "opencode",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WorkerHarness {
    Inherit,
    Codex,
    Opencode,
}

impl WorkerHarness {
    pub fn resolve(self, orchestrator: Harness) -> Harness {
        match self {
            Self::Inherit => orchestrator,
            Self::Codex => Harness::Codex,
            Self::Opencode => Harness::Opencode,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: 1,
            enabled: true,
            yolo: false,
            worker_default: GENERALIST_ROLE.into(),
            orchestrator: AgentConfig {
                harness: Harness::Codex,
                model: Some("gpt-5.6-terra".into()),
                reasoning_effort: ReasoningEffort::High,
                max_parallel: Some(4),
            },
            git: GitConfig {
                auto_integrate: true,
                cleanup_on_success: true,
            },
            workers: WorkerConfig {
                roles: default_roles(),
            },
        }
    }
}

impl Config {
    pub fn path(project_root: &Path) -> PathBuf {
        project_root.join(CONFIG_RELATIVE_PATH)
    }

    pub fn load(project_root: &Path) -> Result<Self> {
        let path = Self::path(project_root);
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("Cadence is not enabled: {} is missing", path.display()))?;
        let config: Self = toml::from_str(&raw)
            .with_context(|| format!("invalid Cadence config at {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn create(project_root: &Path) -> Result<PathBuf> {
        let path = Self::path(project_root);
        if path.exists() {
            bail!("refusing to overwrite existing {}", path.display());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(&Self::default())?;
        fs::write(&path, raw)?;
        Ok(path)
    }

    pub fn save(&self, project_root: &Path) -> Result<()> {
        self.validate()?;
        fs::write(Self::path(project_root), toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!("unsupported config schema_version {}", self.schema_version);
        }
        if !(1..=16).contains(&self.max_parallel()) {
            bail!("orchestrator.max_parallel must be between 1 and 16");
        }
        validate_model(self.orchestrator.model.as_deref())?;
        validate_reasoning_effort(
            "orchestrator",
            self.orchestrator.harness,
            self.orchestrator.model.as_deref(),
            self.orchestrator.reasoning_effort,
        )?;
        if self.worker_default.trim() != self.worker_default || self.worker_default.is_empty() {
            bail!("worker_default cannot be empty or have surrounding whitespace");
        }
        if !self.workers.roles.contains_key(&self.worker_default) {
            bail!(
                "worker_default references unknown role {:?}",
                self.worker_default
            );
        }
        for (name, role) in &self.workers.roles {
            if name.trim() != name || name.is_empty() {
                bail!("worker role names cannot be empty or have surrounding whitespace");
            }
            validate_role(name, &role.description, role.model.as_deref())?;
            validate_reasoning_effort(
                &format!("workers.roles.{name}"),
                role.harness.resolve(self.orchestrator.harness),
                role.model.as_deref(),
                role.reasoning_effort,
            )?;
        }
        Ok(())
    }

    pub fn max_parallel(&self) -> usize {
        self.orchestrator.max_parallel.unwrap_or(4)
    }
}

impl WorkerConfig {
    pub fn role(
        &self,
        name: &str,
    ) -> Result<(
        &str,
        WorkerHarness,
        Option<&str>,
        ReasoningEffort,
        VersionControlMode,
    )> {
        let role = self
            .roles
            .get(name)
            .with_context(|| format!("unknown Worker role {name:?}"))?;
        Ok((
            &role.description,
            role.harness,
            role.model.as_deref(),
            role.reasoning_effort,
            role.version_control_mode,
        ))
    }

    pub fn role_catalog(&self) -> String {
        self.roles
            .iter()
            .map(|(name, role)| {
                (
                    name.as_str(),
                    role.description.as_str(),
                    role.version_control_mode,
                )
            })
            .map(|(name, description, mode)| format!("- {name} [{}]: {description}", mode.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn uses_git_worktrees(&self) -> bool {
        self.roles
            .values()
            .any(|role| role.version_control_mode == VersionControlMode::GitWorktree)
    }
}

impl VersionControlMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SharedCheckout => "shared-checkout",
            Self::GitWorktree => "git-worktree",
        }
    }
}

fn default_worker_role() -> String {
    GENERALIST_ROLE.into()
}

fn default_roles() -> BTreeMap<String, RoleConfig> {
    [
        (
            GENERALIST_ROLE.into(),
            RoleConfig {
                description:
                    "Use for general implementation tasks that do not match a specialized role"
                        .into(),
                harness: WorkerHarness::Codex,
                model: Some("gpt-5.6-terra".into()),
                reasoning_effort: ReasoningEffort::Medium,
                version_control_mode: VersionControlMode::GitWorktree,
            },
        ),
        (
            "planner".into(),
            RoleConfig {
                description: "Use for plan mode".into(),
                harness: WorkerHarness::Codex,
                model: Some("gpt-5.6-sol".into()),
                reasoning_effort: ReasoningEffort::Xhigh,
                version_control_mode: VersionControlMode::SharedCheckout,
            },
        ),
        (
            "research".into(),
            RoleConfig {
                description: "Use for investigation and evidence gathering".into(),
                harness: WorkerHarness::Codex,
                model: Some("gpt-5.6-sol".into()),
                reasoning_effort: ReasoningEffort::High,
                version_control_mode: VersionControlMode::SharedCheckout,
            },
        ),
        (
            "qa".into(),
            RoleConfig {
                description: "Use for test planning, validation, and regression investigation"
                    .into(),
                harness: WorkerHarness::Codex,
                model: Some("gpt-5.6-luna".into()),
                reasoning_effort: ReasoningEffort::Medium,
                version_control_mode: VersionControlMode::SharedCheckout,
            },
        ),
    ]
    .into_iter()
    .collect()
}

fn validate_role(name: &str, description: &str, model: Option<&str>) -> Result<()> {
    if description.trim().is_empty() {
        bail!("worker role {name:?} description cannot be empty");
    }
    validate_model(model)
}

fn validate_model(model: Option<&str>) -> Result<()> {
    if model.is_some_and(|model| model.trim().is_empty()) {
        bail!("model values cannot be empty");
    }
    Ok(())
}

fn validate_reasoning_effort(
    config_path: &str,
    harness: Harness,
    model: Option<&str>,
    reasoning_effort: ReasoningEffort,
) -> Result<()> {
    if harness == Harness::Opencode
        && reasoning_effort != ReasoningEffort::Default
        && model.is_none()
    {
        bail!(
            "{config_path}.reasoning_effort requires an explicit OpenCode model so Cadence can select its variant"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_round_trips() {
        let raw = toml::to_string_pretty(&Config::default()).unwrap();
        let parsed: Config = toml::from_str(&raw).unwrap();
        assert_eq!(parsed, Config::default());
        assert!(raw.contains("max_parallel = 4"));
        assert!(!parsed.yolo);
        assert_eq!(parsed.worker_default, GENERALIST_ROLE);
        assert_eq!(parsed.orchestrator.max_parallel, Some(4));
        let generalist = parsed.workers.role(GENERALIST_ROLE).unwrap();
        assert_eq!(generalist.1, WorkerHarness::Codex);
        assert_eq!(generalist.2, Some("gpt-5.6-terra"));
        assert_eq!(generalist.3, ReasoningEffort::Medium);
        assert_eq!(generalist.4, VersionControlMode::GitWorktree);
    }

    #[test]
    fn rejects_unbounded_parallelism() {
        let mut config = Config::default();
        config.orchestrator.max_parallel = Some(0);
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_unknown_version_control_modes() {
        let raw = toml::to_string_pretty(&Config::default())
            .unwrap()
            .replacen(
                "version_control_mode = \"git-worktree\"",
                "version_control_mode = \"branch\"",
                1,
            );
        assert!(toml::from_str::<Config>(&raw).is_err());
    }

    #[test]
    fn rejects_removed_worker_yolo_setting() {
        let raw = toml::to_string_pretty(&Config::default()).unwrap().replace(
            "[workers.roles.generalist]\n",
            "[workers.roles.generalist]\nyolo_with_worktrees_only = true\n",
        );
        assert!(toml::from_str::<Config>(&raw).is_err());
    }

    #[test]
    fn rejects_old_nested_worktree_setting() {
        let raw = toml::to_string_pretty(&Config::default())
            .unwrap()
            .replace("[git]\n", "[git]\nuse_worktrees = false\n");
        assert!(toml::from_str::<Config>(&raw).is_err());
    }

    #[test]
    fn rejects_removed_global_worktree_settings() {
        for setting in ["enable_git_worktree = true", "use_git_worktrees = true"] {
            let raw = toml::to_string_pretty(&Config::default())
                .unwrap()
                .replace("yolo = false", &format!("yolo = false\n{setting}"));
            assert!(toml::from_str::<Config>(&raw).is_err());
        }
    }

    #[test]
    fn accepts_configured_models() {
        let mut config = Config::default();
        config.orchestrator.model = Some("orchestrator-model".into());
        config.workers.roles.get_mut(GENERALIST_ROLE).unwrap().model = Some("worker-model".into());

        let raw = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&raw).unwrap();

        assert_eq!(parsed, config);
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn accepts_extra_high_reasoning() {
        let mut config = Config::default();
        config.orchestrator.reasoning_effort = ReasoningEffort::Xhigh;

        let raw = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&raw).unwrap();

        assert_eq!(parsed.orchestrator.reasoning_effort, ReasoningEffort::Xhigh);
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn rejects_blank_models() {
        let mut config = Config::default();
        config.orchestrator.model = Some("   ".into());

        assert!(config.validate().is_err());
    }

    #[test]
    fn requires_a_model_for_opencode_reasoning() {
        let mut config = Config::default();
        config.orchestrator.harness = Harness::Opencode;
        config.orchestrator.model = None;

        assert!(config.validate().is_err());

        config.orchestrator.model = Some("openai/gpt-5.2".into());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn resolves_fully_configured_roles() {
        let mut config = Config::default();
        config.workers.roles.insert(
            "research".into(),
            RoleConfig {
                description: "Investigate options and gather evidence".into(),
                harness: WorkerHarness::Opencode,
                model: Some("research-model".into()),
                reasoning_effort: ReasoningEffort::Low,
                version_control_mode: VersionControlMode::SharedCheckout,
            },
        );

        let generalist = config.workers.role(GENERALIST_ROLE).unwrap();
        assert_eq!(generalist.1, WorkerHarness::Codex);
        assert_eq!(generalist.2, Some("gpt-5.6-terra"));
        assert_eq!(generalist.3, ReasoningEffort::Medium);
        assert_eq!(generalist.4, VersionControlMode::GitWorktree);

        let research = config.workers.role("research").unwrap();
        assert_eq!(research.0, "Investigate options and gather evidence");
        assert_eq!(research.1, WorkerHarness::Opencode);
        assert_eq!(research.2, Some("research-model"));
        assert_eq!(research.3, ReasoningEffort::Low);
        assert_eq!(research.4, VersionControlMode::SharedCheckout);
        assert!(
            config
                .workers
                .role_catalog()
                .contains("- research [shared-checkout]:")
        );
        assert!(config.validate().is_ok());

        let raw = toml::to_string_pretty(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&raw).unwrap(), config);
        assert!(config.workers.role("missing").is_err());
    }

    #[test]
    fn rejects_worker_config_without_the_default_role() {
        let raw = r#"
schema_version = 1
enabled = true

[orchestrator]
harness = "codex"

[workers]

[git]
auto_integrate = true
cleanup_on_success = true
"#;

        let config: Config = toml::from_str(raw).unwrap();
        assert!(!config.yolo);
        assert_eq!(config.worker_default, GENERALIST_ROLE);
        assert_eq!(config.orchestrator.max_parallel, None);
        assert_eq!(config.max_parallel(), 4);
        assert!(config.workers.roles.is_empty());
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_an_unknown_default_role() {
        let config = Config {
            worker_default: "missing".into(),
            ..Config::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_roles() {
        let mut config = Config::default();
        config.workers.roles.insert(
            " invalid ".into(),
            RoleConfig {
                description: "Ambiguous fallback".into(),
                harness: WorkerHarness::Inherit,
                model: None,
                reasoning_effort: ReasoningEffort::Default,
                version_control_mode: VersionControlMode::SharedCheckout,
            },
        );
        assert!(config.validate().is_err());

        config.workers.roles.remove(" invalid ");
        config.workers.roles.get_mut("qa").unwrap().description = " ".into();
        assert!(config.validate().is_err());
    }
}

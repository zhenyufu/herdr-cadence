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
    #[serde(default)]
    pub use_git_worktrees: bool,
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
    pub harness: WorkerHarness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: ReasoningEffort,
    #[serde(default)]
    pub yolo_with_worktrees_only: bool,
    #[serde(default = "default_generalist_description")]
    pub generalist_description: String,
    // Accepted for schema v1 compatibility; new configs place this under orchestrator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parallel: Option<usize>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
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
            use_git_worktrees: false,
            orchestrator: AgentConfig {
                harness: Harness::Codex,
                model: Some("gpt-5.6-sol".into()),
                reasoning_effort: ReasoningEffort::High,
                max_parallel: Some(4),
            },
            git: GitConfig {
                auto_integrate: true,
                cleanup_on_success: true,
            },
            workers: WorkerConfig {
                harness: WorkerHarness::Inherit,
                model: None,
                reasoning_effort: ReasoningEffort::Default,
                yolo_with_worktrees_only: false,
                generalist_description: default_generalist_description(),
                max_parallel: None,
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
        let raw = toml::to_string_pretty(&Self::default())?.replacen(
            "harness = \"inherit\"\n",
            "harness = \"inherit\"\n# model = \"your-model-id\"\n",
            1,
        );
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
        if self.orchestrator.max_parallel.is_some() && self.workers.max_parallel.is_some() {
            bail!("max_parallel cannot be set under both orchestrator and workers");
        }
        if !(1..=16).contains(&self.max_parallel()) {
            bail!("orchestrator.max_parallel must be between 1 and 16");
        }
        if self.workers.yolo_with_worktrees_only && !self.use_git_worktrees {
            bail!("workers.yolo_with_worktrees_only requires use_git_worktrees = true");
        }
        validate_model(self.orchestrator.model.as_deref())?;
        validate_reasoning_effort(
            "orchestrator",
            self.orchestrator.harness,
            self.orchestrator.model.as_deref(),
            self.orchestrator.reasoning_effort,
        )?;
        validate_role(
            GENERALIST_ROLE,
            &self.workers.generalist_description,
            self.workers.model.as_deref(),
        )?;
        validate_reasoning_effort(
            "workers",
            self.workers.harness.resolve(self.orchestrator.harness),
            self.workers.model.as_deref(),
            self.workers.reasoning_effort,
        )?;
        for (name, role) in &self.workers.roles {
            if name == GENERALIST_ROLE {
                bail!(
                    "workers.roles.generalist is reserved; configure the generalist under [workers]"
                );
            }
            if name.trim() != name || name.is_empty() {
                bail!("worker role names cannot be empty or have surrounding whitespace");
            }
            validate_role(name, &role.description, role.model.as_deref())?;
            validate_reasoning_effort(
                &format!("workers.roles.{name}"),
                role.harness.resolve(self.orchestrator.harness),
                role.model.as_deref(),
                role.reasoning_effort
                    .unwrap_or(self.workers.reasoning_effort),
            )?;
        }
        Ok(())
    }

    pub fn max_parallel(&self) -> usize {
        self.orchestrator
            .max_parallel
            .or(self.workers.max_parallel)
            .unwrap_or(4)
    }
}

impl WorkerConfig {
    pub fn role(
        &self,
        name: Option<&str>,
    ) -> Result<(&str, WorkerHarness, Option<&str>, ReasoningEffort)> {
        let name = name.unwrap_or(GENERALIST_ROLE);
        if name == GENERALIST_ROLE {
            return Ok((
                &self.generalist_description,
                self.harness,
                self.model.as_deref(),
                self.reasoning_effort,
            ));
        }
        let role = self
            .roles
            .get(name)
            .with_context(|| format!("unknown Worker role {name:?}"))?;
        Ok((
            &role.description,
            role.harness,
            role.model.as_deref(),
            role.reasoning_effort.unwrap_or(self.reasoning_effort),
        ))
    }

    pub fn role_catalog(&self) -> String {
        std::iter::once((GENERALIST_ROLE, self.generalist_description.as_str()))
            .chain(
                self.roles
                    .iter()
                    .map(|(name, role)| (name.as_str(), role.description.as_str())),
            )
            .map(|(name, description)| format!("- {name}: {description}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn default_generalist_description() -> String {
    "Use for general implementation tasks that do not match a specialized role".into()
}

fn default_roles() -> BTreeMap<String, RoleConfig> {
    [
        (
            "research".into(),
            RoleConfig {
                description: "Use for investigation and evidence gathering".into(),
                harness: WorkerHarness::Inherit,
                model: None,
                reasoning_effort: None,
            },
        ),
        (
            "qa".into(),
            RoleConfig {
                description: "Use for test planning, validation, and regression investigation"
                    .into(),
                harness: WorkerHarness::Inherit,
                model: None,
                reasoning_effort: None,
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
        assert_eq!(parsed.orchestrator.max_parallel, Some(4));
        assert_eq!(parsed.workers.max_parallel, None);
        assert!(!parsed.workers.yolo_with_worktrees_only);
        assert!(!parsed.use_git_worktrees);
    }

    #[test]
    fn rejects_unbounded_parallelism() {
        let mut config = Config::default();
        config.orchestrator.max_parallel = Some(0);
        assert!(config.validate().is_err());
    }

    #[test]
    fn requires_worktrees_for_yolo_workers() {
        let mut config = Config::default();
        config.workers.yolo_with_worktrees_only = true;
        assert!(config.validate().is_err());

        config.use_git_worktrees = true;
        assert!(config.validate().is_ok());

        config.use_git_worktrees = false;
        config.workers.yolo_with_worktrees_only = false;
        config.yolo = true;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn rejects_old_nested_worktree_setting() {
        let raw = toml::to_string_pretty(&Config::default())
            .unwrap()
            .replace("[git]\n", "[git]\nuse_worktrees = false\n");
        assert!(toml::from_str::<Config>(&raw).is_err());
    }

    #[test]
    fn accepts_configured_models() {
        let mut config = Config::default();
        config.orchestrator.model = Some("orchestrator-model".into());
        config.workers.model = Some("worker-model".into());

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
    fn resolves_named_roles_and_generalist_fallback() {
        let mut config = Config::default();
        config.workers.roles.insert(
            "research".into(),
            RoleConfig {
                description: "Investigate options and gather evidence".into(),
                harness: WorkerHarness::Opencode,
                model: Some("research-model".into()),
                reasoning_effort: Some(ReasoningEffort::Low),
            },
        );

        let generalist = config.workers.role(None).unwrap();
        assert_eq!(generalist.1, WorkerHarness::Inherit);
        assert_eq!(generalist.2, None);
        assert_eq!(generalist.3, ReasoningEffort::Default);

        let research = config.workers.role(Some("research")).unwrap();
        assert_eq!(research.0, "Investigate options and gather evidence");
        assert_eq!(research.1, WorkerHarness::Opencode);
        assert_eq!(research.2, Some("research-model"));
        assert_eq!(research.3, ReasoningEffort::Low);
        assert!(config.workers.role_catalog().contains("- research:"));
        assert!(config.validate().is_ok());

        let raw = toml::to_string_pretty(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&raw).unwrap(), config);
        assert!(config.workers.role(Some("missing")).is_err());
    }

    #[test]
    fn loads_worker_config_without_roles() {
        let raw = r#"
schema_version = 1
enabled = true

[orchestrator]
harness = "codex"

[workers]
harness = "inherit"
max_parallel = 4

[git]
auto_integrate = true
cleanup_on_success = true
"#;

        let config: Config = toml::from_str(raw).unwrap();
        assert!(!config.yolo);
        assert_eq!(config.orchestrator.max_parallel, None);
        assert_eq!(config.max_parallel(), 4);
        assert!(!config.workers.yolo_with_worktrees_only);
        assert_eq!(
            config.workers.generalist_description,
            default_generalist_description()
        );
        assert!(config.workers.roles.is_empty());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn rejects_parallelism_in_both_config_sections() {
        let mut config = Config::default();
        config.workers.max_parallel = Some(2);

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_reserved_or_invalid_roles() {
        let mut config = Config::default();
        config.workers.roles.insert(
            GENERALIST_ROLE.into(),
            RoleConfig {
                description: "Ambiguous fallback".into(),
                harness: WorkerHarness::Inherit,
                model: None,
                reasoning_effort: None,
            },
        );
        assert!(config.validate().is_err());

        config.workers.roles.clear();
        config.workers.roles.insert(
            "qa".into(),
            RoleConfig {
                description: " ".into(),
                harness: WorkerHarness::Inherit,
                model: None,
                reasoning_effort: None,
            },
        );
        assert!(config.validate().is_err());
    }
}

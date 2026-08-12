use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const CONFIG_RELATIVE_PATH: &str = ".cadence.toml";
pub const CONFIG_SCHEMA_VERSION: u32 = 2;
pub const GENERALIST_ROLE: &str = "generalist";
pub const DEFAULT_CONFIG_TOML: &str = r#"schema_version = 2
enabled = true
# Give the Lead and every agent unrestricted host access.
yolo = false
agent_default = "generalist" # default role when no better match

[lead]
harness = "codex"
model = "gpt-5.6-terra"
reasoning_effort = "high"
max_parallel = 4 # Maximum concurrent agents; 1-16

[git]
auto_integrate = true # Applies only to agents using shared-checkout.
cleanup_on_success = true # Remove successful agent tabs or worktrees after integration.

# A role selects ordered runner profiles. The first is primary; later entries are
# launch-time fallbacks for provider availability failures. Roles come first so
# the workflow stays readable; runner profiles may be defined below them.

# [agents.roles.new_role]
# description = "Handles work that matches this role's specialty"
# runners = ["codex-terra-medium"]
# version_control_mode = "shared-checkout" # shared-checkout | git-worktree

[agents.roles.generalist]
description = "Implements general changes that do not require a specialized role"
runners = ["codex-terra-medium"]
version_control_mode = "shared-checkout"

# Common Workflow: planner -> researcher -> developer -> qa
[agents.roles.planner]
description = "Plans complex work and identifies dependencies, risks, and acceptance criteria. Write to implementation-plan.md"
runners = ["codex-sol-high"]
version_control_mode = "shared-checkout"

[agents.roles.researcher]
description = "Investigates questions and gathers evidence before implementation"
runners = ["codex-terra-high"]
version_control_mode = "shared-checkout"

[agents.roles.developer]
description = "Writes code"
runners = ["codex-terra-medium"]
version_control_mode = "git-worktree"

[agents.roles.reviewer]
description = "Reviews code implementation"
runners = ["claude-opus-high", "codex-terra-high"]
version_control_mode = "shared-checkout"

[agents.roles.qa]
description = "Validates behavior, tests changes, and investigates regressions"
runners = ["codex-terra-medium"]
version_control_mode = "shared-checkout"

[agents.runners.codex-terra-medium]
harness = "codex"
model = "gpt-5.6-terra"
reasoning_effort = "medium"

[agents.runners.codex-terra-high]
harness = "codex"
model = "gpt-5.6-terra"
reasoning_effort = "high"

[agents.runners.codex-sol-high]
harness = "codex"
model = "gpt-5.6-sol"
reasoning_effort = "high"

[agents.runners.claude-opus-high]
harness = "claude"
model = "opus"
reasoning_effort = "high"
"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub enabled: bool,
    #[serde(default)]
    pub yolo: bool,
    #[serde(default = "default_agent_role")]
    pub agent_default: String,
    pub lead: AgentConfig,
    pub git: GitConfig,
    pub agents: AgentRoles,
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
pub struct AgentRoles {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub roles: BTreeMap<String, RoleConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub runners: BTreeMap<String, RunnerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoleConfig {
    pub description: String,
    pub runners: Vec<String>,
    #[serde(default)]
    pub version_control_mode: VersionControlMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunnerConfig {
    pub harness: Harness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: ReasoningEffort,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ResolvedRunner {
    pub name: String,
    pub harness: Harness,
    pub model: Option<String>,
    pub reasoning_effort: ReasoningEffort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRole {
    pub description: String,
    pub runners: Vec<ResolvedRunner>,
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
    Claude,
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
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            enabled: true,
            yolo: false,
            agent_default: GENERALIST_ROLE.into(),
            lead: AgentConfig {
                harness: Harness::Codex,
                model: Some("gpt-5.6-terra".into()),
                reasoning_effort: ReasoningEffort::High,
                max_parallel: Some(4),
            },
            git: GitConfig {
                auto_integrate: true,
                cleanup_on_success: true,
            },
            agents: AgentRoles {
                roles: default_roles(),
                runners: default_runners(),
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
        fs::write(&path, DEFAULT_CONFIG_TOML)?;
        Ok(path)
    }

    pub fn save(&self, project_root: &Path) -> Result<()> {
        self.validate()?;
        fs::write(Self::path(project_root), toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            bail!("unsupported config schema_version {}", self.schema_version);
        }
        if !(1..=16).contains(&self.max_parallel()) {
            bail!("lead.max_parallel must be between 1 and 16");
        }
        validate_model(self.lead.model.as_deref())?;
        validate_reasoning_effort(
            "lead",
            self.lead.harness,
            self.lead.model.as_deref(),
            self.lead.reasoning_effort,
        )?;
        if self.agent_default.trim() != self.agent_default || self.agent_default.is_empty() {
            bail!("agent_default cannot be empty or have surrounding whitespace");
        }
        if !self.agents.roles.contains_key(&self.agent_default) {
            bail!(
                "agent_default references unknown role {:?}",
                self.agent_default
            );
        }
        for (name, runner) in &self.agents.runners {
            validate_name("runner", name)?;
            validate_model(runner.model.as_deref())?;
            validate_reasoning_effort(
                &format!("agents.runners.{name}"),
                runner.harness,
                runner.model.as_deref(),
                runner.reasoning_effort,
            )?;
        }
        for (name, role) in &self.agents.roles {
            if name.trim() != name || name.is_empty() {
                bail!("agent role names cannot be empty or have surrounding whitespace");
            }
            validate_role(name, role)?;
            let unique_runners = role.runners.iter().collect::<BTreeSet<_>>();
            if unique_runners.len() != role.runners.len() {
                bail!("agents.roles.{name}.runners cannot contain duplicates");
            }
            for runner in &role.runners {
                validate_name("runner reference", runner)?;
                if !self.agents.runners.contains_key(runner) {
                    bail!("agents.roles.{name} references unknown runner {runner:?}");
                }
            }
        }
        Ok(())
    }

    pub fn max_parallel(&self) -> usize {
        self.lead.max_parallel.unwrap_or(4)
    }
}

impl AgentRoles {
    pub fn role(&self, name: &str) -> Result<ResolvedRole> {
        let role = self
            .roles
            .get(name)
            .with_context(|| format!("unknown agent role {name:?}"))?;
        let runners = role
            .runners
            .iter()
            .map(|runner_name| {
                let runner = self
                    .runners
                    .get(runner_name)
                    .with_context(|| format!("unknown runner {runner_name:?}"))?;
                Ok(ResolvedRunner {
                    name: runner_name.clone(),
                    harness: runner.harness,
                    model: runner.model.clone(),
                    reasoning_effort: runner.reasoning_effort,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ResolvedRole {
            description: role.description.clone(),
            runners,
            version_control_mode: role.version_control_mode,
        })
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

fn default_agent_role() -> String {
    GENERALIST_ROLE.into()
}

fn default_roles() -> BTreeMap<String, RoleConfig> {
    [
        (
            GENERALIST_ROLE.into(),
            RoleConfig {
                description: "Implements general changes that do not require a specialized role"
                    .into(),
                runners: vec!["codex-terra-medium".into()],
                version_control_mode: VersionControlMode::SharedCheckout,
            },
        ),
        (
            "planner".into(),
            RoleConfig {
                description: "Plans complex work and identifies dependencies, risks, and acceptance criteria. Write to implementation-plan.md".into(),
                runners: vec!["codex-sol-high".into()],
                version_control_mode: VersionControlMode::SharedCheckout,
            },
        ),
        (
            "researcher".into(),
            RoleConfig {
                description: "Investigates questions and gathers evidence before implementation"
                    .into(),
                runners: vec!["codex-terra-high".into()],
                version_control_mode: VersionControlMode::SharedCheckout,
            },
        ),
        (
            "developer".into(),
            RoleConfig {
                description: "Writes code".into(),
                runners: vec!["codex-terra-medium".into()],
                version_control_mode: VersionControlMode::GitWorktree,
            },
        ),
        (
            "reviewer".into(),
            RoleConfig {
                description: "Reviews code implementation".into(),
                runners: vec!["claude-opus-high".into(), "codex-terra-high".into()],
                version_control_mode: VersionControlMode::SharedCheckout,
            },
        ),
        (
            "qa".into(),
            RoleConfig {
                description: "Validates behavior, tests changes, and investigates regressions"
                    .into(),
                runners: vec!["codex-terra-medium".into()],
                version_control_mode: VersionControlMode::SharedCheckout,
            },
        ),
    ]
    .into_iter()
    .collect()
}

fn default_runners() -> BTreeMap<String, RunnerConfig> {
    [
        (
            "codex-terra-medium".into(),
            RunnerConfig {
                harness: Harness::Codex,
                model: Some("gpt-5.6-terra".into()),
                reasoning_effort: ReasoningEffort::Medium,
            },
        ),
        (
            "codex-terra-high".into(),
            RunnerConfig {
                harness: Harness::Codex,
                model: Some("gpt-5.6-terra".into()),
                reasoning_effort: ReasoningEffort::High,
            },
        ),
        (
            "codex-sol-high".into(),
            RunnerConfig {
                harness: Harness::Codex,
                model: Some("gpt-5.6-sol".into()),
                reasoning_effort: ReasoningEffort::High,
            },
        ),
        (
            "claude-opus-high".into(),
            RunnerConfig {
                harness: Harness::Claude,
                model: Some("opus".into()),
                reasoning_effort: ReasoningEffort::High,
            },
        ),
    ]
    .into_iter()
    .collect()
}

fn validate_role(name: &str, role: &RoleConfig) -> Result<()> {
    if role.description.trim().is_empty() {
        bail!("agent role {name:?} description cannot be empty");
    }
    if role.runners.is_empty() {
        bail!("agents.roles.{name}.runners cannot be empty");
    }
    Ok(())
}

fn validate_name(kind: &str, name: &str) -> Result<()> {
    if name.trim() != name || name.is_empty() {
        bail!("{kind} names cannot be empty or have surrounding whitespace");
    }
    Ok(())
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
        assert_eq!(parsed.agent_default, GENERALIST_ROLE);
        assert_eq!(parsed.lead.max_parallel, Some(4));
        let generalist = parsed.agents.role(GENERALIST_ROLE).unwrap();
        assert_eq!(generalist.runners[0].name, "codex-terra-medium");
        assert_eq!(generalist.runners[0].harness, Harness::Codex);
        assert_eq!(
            generalist.runners[0].model.as_deref(),
            Some("gpt-5.6-terra")
        );
        assert_eq!(
            generalist.runners[0].reasoning_effort,
            ReasoningEffort::Medium
        );
        assert_eq!(
            generalist.version_control_mode,
            VersionControlMode::SharedCheckout
        );
    }

    #[test]
    fn documented_config_matches_init_config() {
        let generated: Config = toml::from_str(DEFAULT_CONFIG_TOML).unwrap();
        assert_eq!(generated, Config::default());

        let readme = include_str!("../README.md");
        let documented = readme
            .split_once("## Configuration\n")
            .unwrap()
            .1
            .split_once("```toml\n")
            .unwrap()
            .1
            .split_once("\n```")
            .unwrap()
            .0;
        assert_eq!(documented, DEFAULT_CONFIG_TOML.trim_end());
    }

    #[test]
    fn rejects_unbounded_parallelism() {
        let mut config = Config::default();
        config.lead.max_parallel = Some(0);
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_the_pre_runner_config_schema() {
        let config = Config {
            schema_version: 1,
            ..Config::default()
        };
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
    fn rejects_removed_agent_yolo_setting() {
        let raw = toml::to_string_pretty(&Config::default()).unwrap().replace(
            "[agents.roles.generalist]\n",
            "[agents.roles.generalist]\nyolo_with_worktrees_only = true\n",
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
    fn accepts_configured_runner_models() {
        let mut config = Config::default();
        config.lead.model = Some("lead-model".into());
        config
            .agents
            .runners
            .get_mut("codex-terra-medium")
            .unwrap()
            .model = Some("agent-model".into());

        let raw = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&raw).unwrap();

        assert_eq!(parsed, config);
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn accepts_extra_high_reasoning() {
        let mut config = Config::default();
        config.lead.reasoning_effort = ReasoningEffort::Xhigh;

        let raw = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&raw).unwrap();

        assert_eq!(parsed.lead.reasoning_effort, ReasoningEffort::Xhigh);
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn rejects_blank_models() {
        let mut config = Config::default();
        config.lead.model = Some("   ".into());

        assert!(config.validate().is_err());
    }

    #[test]
    fn requires_a_model_for_opencode_reasoning() {
        let mut config = Config::default();
        config.lead.harness = Harness::Opencode;
        config.lead.model = None;

        assert!(config.validate().is_err());

        config.lead.model = Some("openai/gpt-5.2".into());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn accepts_claude_leads_and_runners() {
        let mut config = Config::default();
        config.lead.harness = Harness::Claude;
        config.lead.model = Some("opus".into());

        assert!(config.validate().is_ok());
        assert_eq!(
            config.agents.role("reviewer").unwrap().runners[0].harness,
            Harness::Claude
        );

        let raw = toml::to_string_pretty(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&raw).unwrap(), config);
    }

    #[test]
    fn resolves_fully_configured_roles_and_runners() {
        let mut config = Config::default();
        config.agents.runners.insert(
            "researcher-primary".into(),
            RunnerConfig {
                harness: Harness::Opencode,
                model: Some("researcher-model".into()),
                reasoning_effort: ReasoningEffort::Low,
            },
        );
        config.agents.roles.insert(
            "researcher".into(),
            RoleConfig {
                description: "Investigate options and gather evidence".into(),
                runners: vec!["researcher-primary".into(), "codex-terra-high".into()],
                version_control_mode: VersionControlMode::SharedCheckout,
            },
        );

        let generalist = config.agents.role(GENERALIST_ROLE).unwrap();
        assert_eq!(generalist.runners[0].harness, Harness::Codex);
        assert_eq!(
            generalist.version_control_mode,
            VersionControlMode::SharedCheckout
        );

        let researcher = config.agents.role("researcher").unwrap();
        assert_eq!(
            researcher.description,
            "Investigate options and gather evidence"
        );
        assert_eq!(researcher.runners[0].name, "researcher-primary");
        assert_eq!(researcher.runners[0].harness, Harness::Opencode);
        assert_eq!(
            researcher.runners[0].model.as_deref(),
            Some("researcher-model")
        );
        assert_eq!(researcher.runners[0].reasoning_effort, ReasoningEffort::Low);
        assert_eq!(
            researcher.version_control_mode,
            VersionControlMode::SharedCheckout
        );
        assert!(
            config
                .agents
                .role_catalog()
                .contains("- researcher [shared-checkout]:")
        );
        assert!(config.validate().is_ok());

        let raw = toml::to_string_pretty(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&raw).unwrap(), config);
        assert!(config.agents.role("missing").is_err());
    }

    #[test]
    fn rejects_agent_config_without_the_default_role() {
        let raw = r#"
schema_version = 1
enabled = true

[lead]
harness = "codex"

[agents]

[git]
auto_integrate = true
cleanup_on_success = true
"#;

        let config: Config = toml::from_str(raw).unwrap();
        assert!(!config.yolo);
        assert_eq!(config.agent_default, GENERALIST_ROLE);
        assert_eq!(config.lead.max_parallel, None);
        assert_eq!(config.max_parallel(), 4);
        assert!(config.agents.roles.is_empty());
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_an_unknown_default_role() {
        let config = Config {
            agent_default: "missing".into(),
            ..Config::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_roles() {
        let mut config = Config::default();
        config.agents.roles.insert(
            " invalid ".into(),
            RoleConfig {
                description: "Ambiguous fallback".into(),
                runners: vec!["codex-terra-medium".into()],
                version_control_mode: VersionControlMode::SharedCheckout,
            },
        );
        assert!(config.validate().is_err());

        config.agents.roles.remove(" invalid ");
        config.agents.roles.get_mut("qa").unwrap().description = " ".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_empty_or_unknown_role_runners() {
        let mut config = Config::default();
        config.agents.roles.get_mut("qa").unwrap().runners.clear();
        assert!(config.validate().is_err());

        config.agents.roles.get_mut("qa").unwrap().runners = vec!["missing".into()];
        assert!(config.validate().is_err());

        config.agents.roles.get_mut("qa").unwrap().runners =
            vec!["codex-terra-medium".into(), "codex-terra-medium".into()];
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_single_harness_role_configuration() {
        let raw = DEFAULT_CONFIG_TOML
            .replace("runners = [\"codex-terra-medium\"]", "harness = \"codex\"");
        assert!(toml::from_str::<Config>(&raw).is_err());
    }
}

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::config::{Config, Harness, VersionControlMode};
use crate::git;
use crate::herdr::Herdr;
use crate::model::{
    Agent, AgentRef, AgentReport, AgentRequest, AgentStatus, ProjectState, ReportStatus, Run,
    RunStatus, path_within_scope, scopes_overlap,
};
use crate::prompts;
use crate::state::{StateStore, project_key};

pub struct App {
    pub root: PathBuf,
    pub state: StateStore,
    pub binary: PathBuf,
    pub herdr: Herdr,
}

impl App {
    pub fn new(root: PathBuf, state_dir: PathBuf) -> Result<Self> {
        let root = git::repository_root(&root)?;
        Self::new_runtime(root, state_dir)
    }

    pub fn new_runtime(root: PathBuf, state_dir: PathBuf) -> Result<Self> {
        Ok(Self {
            root,
            state: StateStore::new(state_dir),
            binary: std::env::current_exe().context("cannot resolve Cadence executable")?,
            herdr: Herdr::from_env(),
        })
    }

    pub fn init_project(&self) -> Result<Value> {
        let path = Config::create(&self.root)?;
        Ok(json!({"enabled": true, "config": path}))
    }

    pub fn disable_project(&self) -> Result<Value> {
        let key = project_key(&self.root);
        let store = self.state.read()?;
        if let Some(project) = store.projects.get(&key)
            && project
                .active_run
                .as_ref()
                .and_then(|id| project.runs.get(id))
                .is_some_and(|run| run.status == RunStatus::Active)
        {
            bail!("finish the active Cadence run before disabling this project");
        }
        let mut config = Config::load(&self.root)?;
        config.enabled = false;
        config.save(&self.root)?;
        Ok(json!({"enabled": false, "config": Config::path(&self.root)}))
    }

    pub fn start(&self, workspace_id: &str) -> Result<Value> {
        let config = self.enabled_config()?;
        let checkout_clean = git::is_clean(&self.root)?;
        let branch = git::current_branch(&self.root)?;
        let key = project_key(&self.root);

        let run = self.state.update(|store| {
            let project = store
                .projects
                .entry(key.clone())
                .or_insert_with(|| ProjectState {
                    root: self.root.display().to_string(),
                    active_run: None,
                    runs: Default::default(),
                });
            if let Some(run) = project
                .active_run
                .as_ref()
                .and_then(|id| project.runs.get(id))
                .filter(|run| run.status == RunStatus::Active)
            {
                return Ok(run.clone());
            }
            let now = unix_ms();
            let id = format!("run-{now}-{}", &key[..8]);
            let name = format!("cadence-lead-{}", &key[..8]);
            let run = Run {
                id: id.clone(),
                status: RunStatus::Active,
                base_branch: branch.clone(),
                base_workspace_id: workspace_id.to_string(),
                lead: AgentRef {
                    name,
                    harness: config.lead.harness,
                    model: config.lead.model.clone(),
                    reasoning_effort: config.lead.reasoning_effort,
                    workspace_id: None,
                    tab_id: None,
                    pane_id: None,
                },
                created_unix_ms: now,
                next_agent: 1,
                agents: Default::default(),
                last_error: None,
            };
            project.active_run = Some(id.clone());
            project.runs.insert(id, run.clone());
            Ok(run)
        })?;

        if self.herdr.agent_exists(&run.lead.name) {
            self.herdr.focus_agent(&run.lead.name)?;
            return Ok(json!({"status": "focused", "run_id": run.id, "agent": run.lead.name}));
        }

        let state_dir = self.state.dir().display().to_string();
        let env = [
            ("CADENCE_BIN", self.binary.display().to_string()),
            ("CADENCE_STATE_DIR", state_dir),
            ("CADENCE_PROJECT_ROOT", self.root.display().to_string()),
            ("CADENCE_RUN_ID", run.id.clone()),
        ];
        let terminal = self
            .herdr
            .create_lead_tab(workspace_id, &self.root, &env)
            .context("failed to create the Lead tab")?;
        self.state.update(|store| {
            let stored = active_run_mut(store, &key)?;
            stored.base_workspace_id = workspace_id.to_string();
            stored.lead.workspace_id = terminal.workspace_id.clone();
            stored.lead.tab_id = Some(terminal.tab_id.clone());
            stored.lead.pane_id = Some(terminal.pane_id.clone());
            Ok(())
        })?;
        let agent_args = yolo_agent_args(run.lead.harness, config.yolo);
        let launch = self.herdr.start_agent(
            &run.lead.name,
            run.lead.harness,
            &terminal.pane_id,
            run.lead.model.as_deref(),
            run.lead.reasoning_effort,
            &agent_args,
        );
        if let Err(error) = launch {
            self.set_run_error(&key, &run.id, &error.to_string())?;
            return Err(error.context("failed to start the Lead"));
        }
        let prompt = prompts::lead(
            &self.binary,
            self.state.dir(),
            &self.root,
            &run,
            &config,
            checkout_clean,
        );
        self.herdr.prompt_agent(&run.lead.name, &prompt)?;
        self.state.update(|store| {
            active_run_mut(store, &key)?.last_error = None;
            Ok(())
        })?;
        Ok(
            json!({"status": "started", "run_id": run.id, "agent": run.lead.name, "checkout_clean": checkout_clean, "tab_id": terminal.tab_id, "pane_id": terminal.pane_id}),
        )
    }

    pub fn status(&self) -> Result<Value> {
        let config = Config::load(&self.root);
        let key = project_key(&self.root);
        let store = self.state.read()?;
        let project = store.projects.get(&key);
        let run = project.and_then(|project| {
            project
                .active_run
                .as_ref()
                .and_then(|id| project.runs.get(id))
        });
        let config_valid = config.is_ok();
        let enabled = config.as_ref().ok().map(|config| config.enabled);
        let checkout_clean = git::is_clean(&self.root)?;
        let value = json!({
            "project": self.root,
            "enabled": enabled,
            "config_valid": config_valid,
            "checkout_clean": checkout_clean,
            "active_run": run,
        });
        let project_name = self
            .root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project");
        let run_summary = value
            .get("active_run")
            .and_then(Value::as_object)
            .and_then(|run| run.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("none");
        let notification = format!(
            "Project: {project_name}\nConfig: {}\nEnabled: {}\nCheckout: {}\nRun: {run_summary}",
            if config_valid { "valid" } else { "invalid" },
            match enabled {
                Some(true) => "yes",
                Some(false) => "no",
                None => "unknown",
            },
            if checkout_clean { "clean" } else { "dirty" },
        );
        let _ = self
            .herdr
            .show_notification("Cadence status", &notification);
        Ok(value)
    }

    pub fn validate_config(&self) -> Result<Value> {
        let config = Config::load(&self.root)?;
        let roles = config
            .agents
            .roles
            .keys()
            .map(String::as_str)
            .map(|name| {
                let (description, harness, model, reasoning_effort, version_control_mode) =
                    config.agents.role(name)?;
                Ok(json!({
                    "name": name,
                    "description": description,
                    "harness": harness.resolve(config.lead.harness),
                    "model": model,
                    "reasoning_effort": reasoning_effort,
                    "version_control_mode": version_control_mode,
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({
            "valid": true,
            "config": Config::path(&self.root),
            "enabled": config.enabled,
            "yolo": config.yolo,
            "agent_default": config.agent_default,
            "lead": {
                "harness": config.lead.harness,
                "model": config.lead.model,
                "reasoning_effort": config.lead.reasoning_effort,
                "max_parallel": config.max_parallel(),
            },
            "roles": roles,
        }))
    }

    pub fn spawn_agent(&self, request_file: &Path) -> Result<Value> {
        let config = self.enabled_config()?;
        git::ensure_clean(&self.root)
            .context("cannot spawn an agent until the base checkout is committed or stashed")?;
        let request: AgentRequest = serde_json::from_reader(
            fs::File::open(request_file)
                .with_context(|| format!("cannot open {}", request_file.display()))?,
        )?;
        let request = request.validate_and_normalize()?;
        let role_name = request
            .role
            .as_deref()
            .unwrap_or(config.agent_default.as_str());
        let (
            role_description,
            role_harness,
            role_model,
            role_reasoning_effort,
            version_control_mode,
        ) = config.agents.role(role_name)?;
        let use_worktree = version_control_mode == VersionControlMode::GitWorktree;
        let role_name = role_name.to_string();
        let role_description = role_description.to_string();
        let role_model = role_model.map(str::to_string);
        let max_parallel = config.max_parallel();
        let key = project_key(&self.root);
        if !use_worktree {
            let run = self.active_run_snapshot(&key)?;
            ensure!(
                git::current_branch(&self.root)? == run.base_branch,
                "base checkout changed branches; expected {}",
                run.base_branch
            );
        }
        let base_sha = git::head(&self.root)?;
        let agent = self.state.update(|store| {
            let run = active_run_mut(store, &key)?;
            ensure!(run.status == RunStatus::Active, "Cadence run is not active");
            let active = run
                .agents
                .values()
                .filter(|a| a.status.occupies_slot())
                .count();
            ensure!(
                active < max_parallel,
                "agent limit reached ({})",
                max_parallel
            );
            if let Some(conflict) = run
                .agents
                .values()
                .filter(|a| a.status.occupies_slot())
                .find(|a| scopes_overlap(&a.scope, &request.scope))
            {
                bail!(
                    "scope overlaps active agent {} ({})",
                    conflict.id,
                    conflict.title
                );
            }
            let number = run.next_agent;
            run.next_agent += 1;
            let id = format!("agent-{number}");
            let harness = request
                .harness
                .unwrap_or_else(|| role_harness.resolve(config.lead.harness));
            let model = request.model.clone().or_else(|| role_model.clone());
            let reasoning_effort = request.reasoning_effort.unwrap_or(role_reasoning_effort);
            let title_slug = slug(&request.title, 20);
            let title_slug = if title_slug.is_empty() {
                "task".to_string()
            } else {
                title_slug
            };
            let branch = if use_worktree {
                format!("cadence/{}/{}-{}", short_run_id(&run.id), id, title_slug)
            } else {
                run.base_branch.clone()
            };
            let agent = Agent {
                id: id.clone(),
                title: request.title.clone(),
                task: request.task.clone(),
                scope: request.scope.clone(),
                acceptance: request.acceptance.clone(),
                role: role_name.clone(),
                role_description: role_description.clone(),
                harness,
                model,
                reasoning_effort,
                yolo: config.yolo,
                use_worktree,
                branch,
                base_sha: base_sha.clone(),
                agent_name: format!("cadence-{}-a{number}", &key[..6]),
                status: AgentStatus::Starting,
                workspace_id: None,
                tab_id: None,
                pane_id: None,
                checkout_path: None,
                observed_agent_status: None,
                report: None,
                error: None,
            };
            run.agents.insert(id, agent.clone());
            Ok(agent)
        })?;

        let run = self.active_run_snapshot(&key)?;
        let label = format!(
            "[{}] {}",
            truncate(&display_role(&agent.role), 20),
            truncate(&agent.title, 40)
        );
        let terminal = match if agent.use_worktree {
            self.herdr
                .create_agent_worktree(&self.root, &agent.branch, &agent.base_sha, &label)
        } else {
            self.herdr
                .create_agent_tab(&run.base_workspace_id, &self.root, &label)
        } {
            Ok(terminal) => terminal,
            Err(error) => {
                self.fail_agent(&key, &agent.id, &error.to_string())?;
                let resource = if agent.use_worktree {
                    "worktree"
                } else {
                    "tab"
                };
                return Err(error.context(format!("failed to create agent {resource}")));
            }
        };
        self.state.update(|store| {
            let stored = agent_mut(active_run_mut(store, &key)?, &agent.id)?;
            stored.workspace_id = terminal.workspace_id.clone();
            stored.tab_id = Some(terminal.tab_id.clone());
            stored.pane_id = Some(terminal.pane_id.clone());
            stored.checkout_path = terminal.checkout_path.clone();
            Ok(())
        })?;
        let agent_args = agent_launch_args(&agent, self.state.dir());
        if let Err(error) = self.herdr.start_agent(
            &agent.agent_name,
            agent.harness,
            &terminal.pane_id,
            agent.model.as_deref(),
            agent.reasoning_effort,
            &agent_args,
        ) {
            self.fail_agent(&key, &agent.id, &error.to_string())?;
            return Err(error.context("failed to start agent"));
        }
        let prompt = prompts::agent(&self.binary, self.state.dir(), &self.root, &run.id, &agent);
        self.herdr.prompt_agent(&agent.agent_name, &prompt)?;
        self.state.update(|store| {
            agent_mut(active_run_mut(store, &key)?, &agent.id)?.status = AgentStatus::Working;
            Ok(())
        })?;
        let display_name = agent_display_name(&agent);
        Ok(
            json!({"status": "working", "agent_id": agent.id, "display_name": display_name, "role": agent.role, "agent": agent.agent_name, "model": agent.model, "reasoning_effort": agent.reasoning_effort, "branch": agent.branch, "workspace_id": terminal.workspace_id, "pane_id": terminal.pane_id}),
        )
    }

    pub fn list_agents(&self) -> Result<Value> {
        let key = project_key(&self.root);
        let run = self.active_run_snapshot(&key)?;
        Ok(json!({"run_id": run.id, "agents": run.agents.values().collect::<Vec<_>>() }))
    }

    pub fn agent_status(&self, agent_id: &str) -> Result<Value> {
        let key = project_key(&self.root);
        let run = self.active_run_snapshot(&key)?;
        let agent = run.agents.get(agent_id).context("unknown agent")?;
        Ok(serde_json::to_value(agent)?)
    }

    pub fn agent_report(&self, agent_id: &str) -> Result<Value> {
        let key = project_key(&self.root);
        let run = self.active_run_snapshot(&key)?;
        let agent = run.agents.get(agent_id).context("unknown agent")?;
        Ok(
            json!({"agent_id": agent_id, "status": agent.status, "report": agent.report, "error": agent.error}),
        )
    }

    pub fn complete_agent(&self, agent_id: &str, report_file: &Path) -> Result<Value> {
        let config = self.enabled_config()?;
        let mut report: AgentReport = serde_json::from_reader(
            fs::File::open(report_file)
                .with_context(|| format!("cannot open {}", report_file.display()))?,
        )?;
        ensure!(
            !report.summary.trim().is_empty(),
            "report summary cannot be empty"
        );
        let key = project_key(&self.root);
        let display_name = {
            let run = self.active_run_snapshot(&key)?;
            agent_display_name(run.agents.get(agent_id).context("unknown agent")?)
        };
        match report.status {
            ReportStatus::Blocked => {
                self.store_report(&key, agent_id, report, AgentStatus::Blocked)?;
                self.notify(
                    &key,
                    &format!(
                        "{display_name} is blocked (internal ID: {agent_id}). Inspect its report and follow up."
                    ),
                );
                self.agent_status(agent_id)
            }
            ReportStatus::Failed => {
                self.store_report(&key, agent_id, report, AgentStatus::Failed)?;
                self.notify(
                    &key,
                    &format!(
                        "{display_name} failed (internal ID: {agent_id}). Its changes were retained."
                    ),
                );
                self.agent_status(agent_id)
            }
            ReportStatus::Completed => {
                let run = self.active_run_snapshot(&key)?;
                let agent = run.agents.get(agent_id).context("unknown agent")?;
                let checkout = PathBuf::from(
                    agent
                        .checkout_path
                        .as_deref()
                        .context("agent checkout is unavailable")?,
                );
                let (agent_head, changed_paths) = if agent.use_worktree {
                    git::ensure_clean(&checkout)?;
                    let agent_head = git::head(&checkout)?;
                    ensure!(
                        git::is_ancestor(&checkout, &agent.base_sha, &agent_head)?,
                        "Agent history no longer descends from its assigned base"
                    );
                    ensure!(agent_head != agent.base_sha, "Agent produced no commits");
                    if let Some(reported) = report.commit_sha.as_deref() {
                        ensure!(
                            reported == agent_head,
                            "reported commit_sha is not Agent HEAD"
                        );
                    }
                    let changed_paths =
                        git::changed_paths(&checkout, &agent.base_sha, &agent_head)?;
                    (Some(agent_head), changed_paths)
                } else if let Some(reported_commit) = report.commit_sha.as_deref() {
                    let agent_commit = git::resolve_commit(&checkout, reported_commit)?;
                    ensure!(agent_commit != agent.base_sha, "Agent produced no commits");
                    ensure!(
                        git::is_ancestor(&checkout, &agent.base_sha, &agent_commit)?,
                        "Agent commit does not descend from its assigned base"
                    );
                    let current_head = git::head(&checkout)?;
                    ensure!(
                        git::is_ancestor(&checkout, &agent_commit, &current_head)?,
                        "Agent commit is not on the current base branch"
                    );
                    let changed_paths = git::changed_paths_for_commit(&checkout, &agent_commit)?;
                    (Some(agent_commit), changed_paths)
                } else {
                    ensure!(
                        report.changed_paths.is_empty(),
                        "shared-checkout agents must report commit_sha when files changed"
                    );
                    (None, Vec::new())
                };
                ensure!(
                    agent_head.is_none() || !changed_paths.is_empty(),
                    "Agent commits changed no paths"
                );
                let outside_scope = changed_paths
                    .iter()
                    .filter(|path| !path_within_scope(path, &agent.scope))
                    .cloned()
                    .collect::<Vec<_>>();
                ensure!(
                    outside_scope.is_empty(),
                    "Agent changed paths outside its reserved scope: {}",
                    outside_scope.join(", ")
                );
                report.commit_sha = agent_head;
                report.changed_paths = changed_paths;
                self.store_report(&key, agent_id, report, AgentStatus::Completed)?;
                if agent.use_worktree {
                    self.notify(
                        &key,
                        &format!(
                            "{display_name} completed (internal ID: {agent_id}). Review its report, then run `agent integrate {agent_id}` to accept the isolated commit."
                        ),
                    );
                    self.agent_status(agent_id)
                } else if config.git.auto_integrate {
                    self.integrate_agent(agent_id)
                } else {
                    self.agent_status(agent_id)
                }
            }
        }
    }

    pub fn integrate_agent(&self, agent_id: &str) -> Result<Value> {
        let config = self.enabled_config()?;
        let key = project_key(&self.root);
        let run = self.active_run_snapshot(&key)?;
        let agent = run.agents.get(agent_id).context("unknown agent")?.clone();
        ensure!(
            matches!(agent.status, AgentStatus::Completed | AgentStatus::Conflict),
            "Agent must have a completed report before integration"
        );
        self.state.update(|store| {
            agent_mut(active_run_mut(store, &key)?, agent_id)?.status = AgentStatus::Integrating;
            Ok(())
        })?;
        let result = (|| -> Result<()> {
            git::ensure_clean(&self.root)?;
            ensure!(
                git::current_branch(&self.root)? == run.base_branch,
                "base checkout changed branches; expected {}",
                run.base_branch
            );
            if !agent.use_worktree {
                if let Some(commit) = agent
                    .report
                    .as_ref()
                    .and_then(|report| report.commit_sha.as_deref())
                {
                    let current_head = git::head(&self.root)?;
                    ensure!(
                        git::is_ancestor(&self.root, commit, &current_head)?,
                        "Agent commit is not on the current base branch"
                    );
                }
                return Ok(());
            }
            git::ensure_clean(&self.root)?;
            let checkout = PathBuf::from(
                agent
                    .checkout_path
                    .as_deref()
                    .context("agent checkout is unavailable")?,
            );
            let agent_head = git::head(&checkout)?;
            let commits = git::commits_between(&checkout, &agent.base_sha, &agent_head)?;
            git::cherry_pick(&self.root, &commits)
        })();
        match result {
            Ok(()) => {
                self.state.update(|store| {
                    let stored = agent_mut(active_run_mut(store, &key)?, agent_id)?;
                    stored.status = AgentStatus::Integrated;
                    stored.error = None;
                    Ok(())
                })?;
                let message = if agent.use_worktree {
                    if config.git.cleanup_on_success {
                        format!(
                            "{} integrated successfully (internal ID: {agent_id}); its agent and worktree were cleaned up.",
                            agent_display_name(&agent)
                        )
                    } else {
                        format!(
                            "{} integrated successfully (internal ID: {agent_id}); its agent and worktree were retained by configuration.",
                            agent_display_name(&agent)
                        )
                    }
                } else if config.git.cleanup_on_success {
                    format!(
                        "{} completed successfully on the shared base branch (internal ID: {agent_id}); its agent and tab were cleaned up.",
                        agent_display_name(&agent)
                    )
                } else {
                    format!(
                        "{} completed successfully on the shared base branch (internal ID: {agent_id}); its agent and tab were retained by configuration.",
                        agent_display_name(&agent)
                    )
                };
                if config.git.cleanup_on_success {
                    if self.herdr.agent_exists(&agent.agent_name) {
                        let _ = self.herdr.send_ctrl_c(&agent.agent_name);
                    }
                    self.cleanup_agent(&key, agent_id)?;
                }
                self.notify(&key, &message);
                self.agent_status(agent_id)
            }
            Err(error) => {
                self.state.update(|store| {
                    let stored = agent_mut(active_run_mut(store, &key)?, agent_id)?;
                    stored.status = AgentStatus::Conflict;
                    stored.error = Some(error.to_string());
                    Ok(())
                })?;
                self.notify(&key, &format!("{} could not integrate (internal ID: {agent_id}). Its changes and isolated resources were retained.", agent_display_name(&agent)));
                self.agent_status(agent_id)
            }
        }
    }

    pub fn prompt_agent(&self, agent_id: &str, prompt_file: &Path) -> Result<Value> {
        let prompt = fs::read_to_string(prompt_file)?;
        ensure!(!prompt.trim().is_empty(), "prompt cannot be empty");
        let key = project_key(&self.root);
        let run = self.active_run_snapshot(&key)?;
        let agent = run.agents.get(agent_id).context("unknown agent")?;
        ensure!(
            self.herdr.agent_exists(&agent.agent_name),
            "agent is not running"
        );
        self.herdr.prompt_agent(&agent.agent_name, &prompt)?;
        self.state.update(|store| {
            let agent = agent_mut(active_run_mut(store, &key)?, agent_id)?;
            agent.status = AgentStatus::Working;
            agent.error = None;
            Ok(())
        })?;
        self.agent_status(agent_id)
    }

    pub fn cancel_agent(&self, agent_id: &str) -> Result<Value> {
        let key = project_key(&self.root);
        let run = self.active_run_snapshot(&key)?;
        let agent = run.agents.get(agent_id).context("unknown agent")?;
        if self.herdr.agent_exists(&agent.agent_name) {
            self.herdr.send_ctrl_c(&agent.agent_name)?;
        }
        self.state.update(|store| {
            agent_mut(active_run_mut(store, &key)?, agent_id)?.status = AgentStatus::Cancelled;
            Ok(())
        })?;
        self.agent_status(agent_id)
    }

    pub fn finish_run(&self) -> Result<Value> {
        let config = self.enabled_config()?;
        let key = project_key(&self.root);
        let run = self.active_run_snapshot(&key)?;
        if let Some(agent) = run.agents.values().find(|a| !a.status.is_terminal()) {
            bail!(
                "Agent {} is not in a terminal state ({:?})",
                agent.id,
                agent.status
            );
        }
        if config.git.cleanup_on_success
            && let Some(agent) = run.agents.values().find(|a| {
                a.status == AgentStatus::Integrated
                    && (a.workspace_id.is_some() || a.tab_id.is_some())
            })
        {
            bail!(
                "Agent {} is integrated but its resources have not been cleaned up",
                agent.id
            );
        }
        self.state.update(|store| {
            let project = store.projects.get_mut(&key).context("unknown project")?;
            let run_id = project.active_run.clone().context("no active run")?;
            project.runs.get_mut(&run_id).context("unknown run")?.status = RunStatus::Completed;
            project.active_run = None;
            Ok(())
        })?;
        Ok(json!({"status": "completed", "run_id": run.id}))
    }

    pub fn handle_event(&self, event_name: &str, event_json: &str) -> Result<Value> {
        let event: Value = serde_json::from_str(event_json)?;
        let data = event.get("data").unwrap_or(&event);
        let pane_id = data
            .get("pane_id")
            .and_then(Value::as_str)
            .context("event omitted pane_id")?;
        let Some((key, run_id, agent_id, agent)) = self.find_agent_by_pane(pane_id)? else {
            return Ok(json!({"ignored": true, "reason": "pane is not owned by Cadence"}));
        };
        let project_root = PathBuf::from(self.project_root_for_key(&key)?);
        if !Config::load(&project_root).is_ok_and(|config| config.enabled) {
            return Ok(json!({"ignored": true, "reason": "project is not enabled"}));
        }

        match event_name {
            "pane.agent_status_changed" => {
                let status = data
                    .get("agent_status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let previous = agent.observed_agent_status.as_deref();
                self.state.update(|store| {
                    agent_mut(run_mut(store, &key, &run_id)?, &agent_id)?.observed_agent_status =
                        Some(status.to_string());
                    Ok(())
                })?;
                if status == "blocked" && previous != Some("blocked") {
                    self.notify(
                        &key,
                        &format!(
                            "{} is waiting for input (internal ID: {agent_id}).",
                            agent_display_name(&agent)
                        ),
                    );
                } else if matches!(status, "idle" | "done")
                    && agent.status == AgentStatus::Working
                    && !matches!(previous, Some("idle" | "done"))
                {
                    self.notify(
                        &key,
                        &format!(
                            "{} is idle but has not submitted a completion report (internal ID: {agent_id}).",
                            agent_display_name(&agent)
                        ),
                    );
                }
            }
            "pane.agent_detected" => {
                let released = data
                    .get("released")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let agent_missing = data.get("agent").is_none_or(Value::is_null);
                if released || agent_missing {
                    self.handle_agent_exit(&key, &run_id, &agent_id, &agent)?;
                }
            }
            "pane.exited" => self.handle_agent_exit(&key, &run_id, &agent_id, &agent)?,
            _ => {}
        }
        Ok(json!({"handled": true, "agent_id": agent_id, "event": event_name}))
    }

    pub fn startup(&self) -> Result<Value> {
        let store = self.state.read()?;
        let mut reconciled = 0usize;
        for (key, project) in store.projects {
            let root = PathBuf::from(&project.root);
            if !Config::load(&root).is_ok_and(|config| config.enabled) {
                continue;
            }
            let Some(run_id) = project.active_run else {
                continue;
            };
            let Some(run) = project.runs.get(&run_id) else {
                continue;
            };
            for agent in run.agents.values() {
                if !self.herdr.agent_exists(&agent.agent_name) {
                    self.handle_agent_exit(&key, &run_id, &agent.id, agent)?;
                    reconciled += 1;
                }
            }
            if !self.herdr.agent_exists(&run.lead.name) {
                self.set_run_error(
                    &key,
                    &run_id,
                    "Lead is not running; invoke Cadence start to resume",
                )?;
            }
        }
        Ok(json!({"reconciled": reconciled}))
    }

    fn enabled_config(&self) -> Result<Config> {
        let config = Config::load(&self.root)?;
        ensure!(config.enabled, "Cadence is disabled for this project");
        Ok(config)
    }

    fn active_run_snapshot(&self, key: &str) -> Result<Run> {
        let store = self.state.read()?;
        let project = store
            .projects
            .get(key)
            .context("Cadence has no run for this project")?;
        let run_id = project
            .active_run
            .as_ref()
            .context("Cadence has no active run")?;
        Ok(project
            .runs
            .get(run_id)
            .context("active run state is missing")?
            .clone())
    }

    fn store_report(
        &self,
        key: &str,
        agent_id: &str,
        report: AgentReport,
        status: AgentStatus,
    ) -> Result<()> {
        self.state.update(|store| {
            let agent = agent_mut(active_run_mut(store, key)?, agent_id)?;
            agent.report = Some(report);
            agent.status = status;
            Ok(())
        })
    }

    fn fail_agent(&self, key: &str, agent_id: &str, message: &str) -> Result<()> {
        self.state.update(|store| {
            let agent = agent_mut(active_run_mut(store, key)?, agent_id)?;
            agent.status = AgentStatus::Failed;
            agent.error = Some(message.to_string());
            Ok(())
        })
    }

    fn set_run_error(&self, key: &str, run_id: &str, message: &str) -> Result<()> {
        self.state.update(|store| {
            run_mut(store, key, run_id)?.last_error = Some(message.to_string());
            Ok(())
        })
    }

    fn notify(&self, key: &str, message: &str) {
        if let Ok(run) = self.active_run_snapshot(key) {
            let _ = self
                .herdr
                .prompt_agent(&run.lead.name, &format!("Cadence update: {message}"));
        }
    }

    fn find_agent_by_pane(&self, pane_id: &str) -> Result<Option<(String, String, String, Agent)>> {
        let store = self.state.read()?;
        for (key, project) in store.projects {
            let Some(run_id) = project.active_run else {
                continue;
            };
            let Some(run) = project.runs.get(&run_id) else {
                continue;
            };
            if let Some(agent) = run
                .agents
                .values()
                .find(|a| a.pane_id.as_deref() == Some(pane_id))
            {
                return Ok(Some((key, run_id, agent.id.clone(), agent.clone())));
            }
        }
        Ok(None)
    }

    fn project_root_for_key(&self, key: &str) -> Result<String> {
        self.state
            .read()?
            .projects
            .get(key)
            .map(|project| project.root.clone())
            .context("unknown project")
    }

    fn handle_agent_exit(
        &self,
        key: &str,
        run_id: &str,
        agent_id: &str,
        agent: &Agent,
    ) -> Result<()> {
        if agent.status == AgentStatus::Integrated {
            let root = PathBuf::from(self.project_root_for_key(key)?);
            if Config::load(&root).is_ok_and(|config| config.git.cleanup_on_success) {
                self.cleanup_agent(key, agent_id)?;
            }
        } else if !agent.status.is_terminal() {
            self.state.update(|store| {
                let stored = agent_mut(run_mut(store, key, run_id)?, agent_id)?;
                stored.status = AgentStatus::Failed;
                stored.error = Some("agent exited without a terminal completion report".into());
                Ok(())
            })?;
            self.notify(
                key,
                &format!(
                    "{} exited without completing (internal ID: {agent_id}); its changes and isolated resources were retained.",
                    agent_display_name(agent)
                ),
            );
        }
        Ok(())
    }

    fn cleanup_agent(&self, key: &str, agent_id: &str) -> Result<()> {
        let run = self.active_run_snapshot(key)?;
        let agent = run.agents.get(agent_id).context("unknown agent")?.clone();
        if agent.use_worktree {
            if let Some(workspace_id) = agent.workspace_id.as_deref() {
                self.herdr.remove_worktree(workspace_id)?;
            }
        } else if let Some(tab_id) = agent.tab_id.as_deref() {
            let _ = self.herdr.close_tab(tab_id);
        }
        self.state.update(|store| {
            let stored = agent_mut(active_run_mut(store, key)?, agent_id)?;
            stored.workspace_id = None;
            stored.tab_id = None;
            stored.pane_id = None;
            stored.checkout_path = None;
            Ok(())
        })?;
        if agent.use_worktree {
            let root = PathBuf::from(self.project_root_for_key(key)?);
            git::delete_branch(&root, &agent.branch)?;
        }
        Ok(())
    }
}

pub fn context_project_path() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("CADENCE_PROJECT_ROOT") {
        return Ok(PathBuf::from(path));
    }
    if let Ok(raw) = std::env::var("HERDR_PLUGIN_CONTEXT_JSON") {
        let context: Value = serde_json::from_str(&raw)?;
        for field in ["focused_pane_cwd", "workspace_cwd"] {
            if let Some(path) = context.get(field).and_then(Value::as_str) {
                return Ok(PathBuf::from(path));
            }
        }
        if let Some(path) = context
            .pointer("/worktree/checkout_path")
            .and_then(Value::as_str)
        {
            return Ok(PathBuf::from(path));
        }
    }
    Ok(std::env::current_dir()?)
}

pub fn context_workspace_id() -> Result<String> {
    if let Ok(id) = std::env::var("HERDR_WORKSPACE_ID") {
        return Ok(id);
    }
    let raw = std::env::var("HERDR_PLUGIN_CONTEXT_JSON")?;
    serde_json::from_str::<Value>(&raw)?
        .get("workspace_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .context("Cadence start must be invoked from a Herdr workspace")
}

fn active_run_mut<'a>(store: &'a mut crate::model::Store, key: &str) -> Result<&'a mut Run> {
    let project = store
        .projects
        .get_mut(key)
        .context("unknown Cadence project")?;
    let run_id = project
        .active_run
        .clone()
        .context("no active Cadence run")?;
    project
        .runs
        .get_mut(&run_id)
        .context("active run state is missing")
}

fn run_mut<'a>(store: &'a mut crate::model::Store, key: &str, run_id: &str) -> Result<&'a mut Run> {
    store
        .projects
        .get_mut(key)
        .and_then(|project| project.runs.get_mut(run_id))
        .context("unknown Cadence run")
}

fn agent_mut<'a>(run: &'a mut Run, agent_id: &str) -> Result<&'a mut Agent> {
    run.agents.get_mut(agent_id).context("unknown agent")
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn short_run_id(run_id: &str) -> String {
    run_id
        .strip_prefix("run-")
        .unwrap_or(run_id)
        .chars()
        .take(12)
        .collect()
}

fn slug(value: &str, max: usize) -> String {
    let mut output = String::new();
    let mut dash = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            output.push(character);
            dash = false;
        } else if !dash && !output.is_empty() {
            output.push('-');
            dash = true;
        }
        if output.len() >= max {
            break;
        }
    }
    output.trim_matches('-').to_string()
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn agent_launch_args(agent: &Agent, state_dir: &Path) -> Vec<String> {
    if agent.yolo {
        return yolo_agent_args(agent.harness, true);
    }
    if !agent.use_worktree {
        return Vec::new();
    }
    match agent.harness {
        Harness::Codex => vec![
            "--sandbox".into(),
            "workspace-write".into(),
            "--ask-for-approval".into(),
            "never".into(),
            "--add-dir".into(),
            state_dir.display().to_string(),
        ],
        Harness::Opencode => Vec::new(),
    }
}

fn yolo_agent_args(harness: Harness, yolo: bool) -> Vec<String> {
    if !yolo {
        return Vec::new();
    }
    match harness {
        Harness::Codex => vec!["--dangerously-bypass-approvals-and-sandbox".into()],
        Harness::Opencode => vec!["--auto".into()],
    }
}

fn display_role(role: &str) -> String {
    if role.eq_ignore_ascii_case("qa") {
        return "QA".into();
    }
    if role.eq_ignore_ascii_case("researcher") {
        return "Researcher".into();
    }
    role.split(|character: char| !character.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn agent_display_name(agent: &Agent) -> String {
    format!("[{}] {}", display_role(&agent.role), agent.title)
}

#[cfg(test)]
mod tests {
    use super::display_role;

    #[test]
    fn formats_agent_roles_for_labels() {
        assert_eq!(display_role("researcher"), "Researcher");
        assert_eq!(display_role("qa"), "QA");
        assert_eq!(display_role("docs_writer"), "Docs Writer");
    }
}

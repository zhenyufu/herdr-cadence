use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::config::{Config, GENERALIST_ROLE, Harness};
use crate::git;
use crate::herdr::Herdr;
use crate::model::{
    AgentRef, ProjectState, ReportStatus, Run, RunStatus, Worker, WorkerReport, WorkerRequest,
    WorkerStatus, path_within_scope, scopes_overlap,
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

    pub fn enable_project(&self) -> Result<Value> {
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
            let name = format!("cadence-orch-{}", &key[..8]);
            let run = Run {
                id: id.clone(),
                status: RunStatus::Active,
                base_branch: branch.clone(),
                base_workspace_id: workspace_id.to_string(),
                orchestrator: AgentRef {
                    name,
                    harness: config.orchestrator.harness,
                    model: config.orchestrator.model.clone(),
                    reasoning_effort: config.orchestrator.reasoning_effort,
                    workspace_id: None,
                    tab_id: None,
                    pane_id: None,
                },
                created_unix_ms: now,
                next_worker: 1,
                workers: Default::default(),
                last_error: None,
            };
            project.active_run = Some(id.clone());
            project.runs.insert(id, run.clone());
            Ok(run)
        })?;

        if self.herdr.agent_exists(&run.orchestrator.name) {
            self.herdr.focus_agent(&run.orchestrator.name)?;
            return Ok(
                json!({"status": "focused", "run_id": run.id, "agent": run.orchestrator.name}),
            );
        }

        let state_dir = self.state.dir().display().to_string();
        let env = [
            ("CADENCE_BIN", self.binary.display().to_string()),
            ("CADENCE_STATE_DIR", state_dir),
            ("CADENCE_PROJECT_ROOT", self.root.display().to_string()),
            ("CADENCE_RUN_ID", run.id.clone()),
        ];
        let existing_workspace = run
            .orchestrator
            .workspace_id
            .as_deref()
            .filter(|id| self.herdr.workspace_exists(id));
        let terminal = if let Some(workspace_id) = existing_workspace {
            self.herdr
                .create_orchestrator_tab(workspace_id, &self.root, &env)
                .context("failed to create the Orchestrator tab")?
        } else {
            self.herdr
                .create_orchestrator_workspace(&self.root, &env)
                .context("failed to create the Orchestrator workspace")?
        };
        self.state.update(|store| {
            let stored = active_run_mut(store, &key)?;
            if let Some(workspace_id) = terminal.workspace_id.clone() {
                stored.base_workspace_id = workspace_id.clone();
                stored.orchestrator.workspace_id = Some(workspace_id);
            }
            stored.orchestrator.tab_id = Some(terminal.tab_id.clone());
            stored.orchestrator.pane_id = Some(terminal.pane_id.clone());
            Ok(())
        })?;
        let agent_args = yolo_agent_args(run.orchestrator.harness, config.yolo);
        let launch = self.herdr.start_agent(
            &run.orchestrator.name,
            run.orchestrator.harness,
            &terminal.pane_id,
            run.orchestrator.model.as_deref(),
            run.orchestrator.reasoning_effort,
            &agent_args,
        );
        if let Err(error) = launch {
            self.set_run_error(&key, &run.id, &error.to_string())?;
            return Err(error.context("failed to start the Orchestrator"));
        }
        let prompt = prompts::orchestrator(
            &self.binary,
            self.state.dir(),
            &self.root,
            &run,
            &config,
            checkout_clean,
        );
        self.herdr.prompt_agent(&run.orchestrator.name, &prompt)?;
        self.state.update(|store| {
            active_run_mut(store, &key)?.last_error = None;
            Ok(())
        })?;
        Ok(
            json!({"status": "started", "run_id": run.id, "agent": run.orchestrator.name, "checkout_clean": checkout_clean, "tab_id": terminal.tab_id, "pane_id": terminal.pane_id}),
        )
    }

    pub fn status(&self) -> Result<Value> {
        let config = Config::load(&self.root)?;
        let key = project_key(&self.root);
        let store = self.state.read()?;
        let project = store.projects.get(&key);
        let run = project.and_then(|project| {
            project
                .active_run
                .as_ref()
                .and_then(|id| project.runs.get(id))
        });
        Ok(json!({
            "project": self.root,
            "enabled": config.enabled,
            "checkout_clean": git::is_clean(&self.root)?,
            "active_run": run,
        }))
    }

    pub fn validate_config(&self) -> Result<Value> {
        let config = Config::load(&self.root)?;
        let roles = std::iter::once(GENERALIST_ROLE)
            .chain(config.workers.roles.keys().map(String::as_str))
            .map(|name| {
                let (description, harness, model, reasoning_effort) =
                    config.workers.role(Some(name))?;
                Ok(json!({
                    "name": name,
                    "description": description,
                    "harness": harness.resolve(config.orchestrator.harness),
                    "model": model,
                    "reasoning_effort": reasoning_effort,
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({
            "valid": true,
            "config": Config::path(&self.root),
            "enabled": config.enabled,
            "yolo": config.yolo,
            "use_git_worktrees": config.use_git_worktrees,
            "orchestrator": {
                "harness": config.orchestrator.harness,
                "model": config.orchestrator.model,
                "reasoning_effort": config.orchestrator.reasoning_effort,
                "max_parallel": config.max_parallel(),
            },
            "roles": roles,
        }))
    }

    pub fn spawn_worker(&self, request_file: &Path) -> Result<Value> {
        let config = self.enabled_config()?;
        let use_worktree = config.use_git_worktrees;
        git::ensure_clean(&self.root)
            .context("cannot spawn a Worker until the base checkout is committed or stashed")?;
        let request: WorkerRequest = serde_json::from_reader(
            fs::File::open(request_file)
                .with_context(|| format!("cannot open {}", request_file.display()))?,
        )?;
        let request = request.validate_and_normalize()?;
        let role_name = request.role.as_deref().unwrap_or(GENERALIST_ROLE);
        let (role_description, role_harness, role_model, role_reasoning_effort) =
            config.workers.role(Some(role_name))?;
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
        let worker = self.state.update(|store| {
            let run = active_run_mut(store, &key)?;
            ensure!(run.status == RunStatus::Active, "Cadence run is not active");
            let active = run
                .workers
                .values()
                .filter(|w| w.status.occupies_slot())
                .count();
            ensure!(
                active < max_parallel,
                "worker limit reached ({})",
                max_parallel
            );
            if let Some(conflict) = run
                .workers
                .values()
                .filter(|w| w.status.occupies_slot())
                .find(|w| scopes_overlap(&w.scope, &request.scope))
            {
                bail!(
                    "scope overlaps active Worker {} ({})",
                    conflict.id,
                    conflict.title
                );
            }
            let number = run.next_worker;
            run.next_worker += 1;
            let id = format!("worker-{number}");
            let harness = request
                .harness
                .unwrap_or_else(|| role_harness.resolve(config.orchestrator.harness));
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
            let worker = Worker {
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
                agent_name: format!("cadence-{}-w{number}", &key[..6]),
                status: WorkerStatus::Starting,
                workspace_id: None,
                tab_id: None,
                pane_id: None,
                checkout_path: None,
                observed_agent_status: None,
                report: None,
                error: None,
            };
            run.workers.insert(id, worker.clone());
            Ok(worker)
        })?;

        let run = self.active_run_snapshot(&key)?;
        let label = format!(
            "[{}] {}",
            truncate(&display_role(&worker.role), 20),
            truncate(&worker.title, 40)
        );
        let terminal = match if worker.use_worktree {
            self.herdr
                .create_worker_worktree(&self.root, &worker.branch, &worker.base_sha, &label)
        } else {
            self.herdr
                .create_worker_tab(&run.base_workspace_id, &self.root, &label)
        } {
            Ok(terminal) => terminal,
            Err(error) => {
                self.fail_worker(&key, &worker.id, &error.to_string())?;
                let resource = if worker.use_worktree {
                    "worktree"
                } else {
                    "tab"
                };
                return Err(error.context(format!("failed to create Worker {resource}")));
            }
        };
        self.state.update(|store| {
            let stored = worker_mut(active_run_mut(store, &key)?, &worker.id)?;
            stored.workspace_id = terminal.workspace_id.clone();
            stored.tab_id = Some(terminal.tab_id.clone());
            stored.pane_id = Some(terminal.pane_id.clone());
            stored.checkout_path = terminal.checkout_path.clone();
            Ok(())
        })?;
        let agent_args = worker_agent_args(&worker, self.state.dir());
        if let Err(error) = self.herdr.start_agent(
            &worker.agent_name,
            worker.harness,
            &terminal.pane_id,
            worker.model.as_deref(),
            worker.reasoning_effort,
            &agent_args,
        ) {
            self.fail_worker(&key, &worker.id, &error.to_string())?;
            return Err(error.context("failed to start Worker agent"));
        }
        let prompt = prompts::worker(&self.binary, self.state.dir(), &self.root, &run.id, &worker);
        self.herdr.prompt_agent(&worker.agent_name, &prompt)?;
        self.state.update(|store| {
            worker_mut(active_run_mut(store, &key)?, &worker.id)?.status = WorkerStatus::Working;
            Ok(())
        })?;
        let display_name = worker_display_name(&worker);
        Ok(
            json!({"status": "working", "worker_id": worker.id, "display_name": display_name, "role": worker.role, "agent": worker.agent_name, "model": worker.model, "reasoning_effort": worker.reasoning_effort, "branch": worker.branch, "workspace_id": terminal.workspace_id, "pane_id": terminal.pane_id}),
        )
    }

    pub fn list_workers(&self) -> Result<Value> {
        let key = project_key(&self.root);
        let run = self.active_run_snapshot(&key)?;
        Ok(json!({"run_id": run.id, "workers": run.workers.values().collect::<Vec<_>>() }))
    }

    pub fn worker_status(&self, worker_id: &str) -> Result<Value> {
        let key = project_key(&self.root);
        let run = self.active_run_snapshot(&key)?;
        let worker = run.workers.get(worker_id).context("unknown Worker")?;
        Ok(serde_json::to_value(worker)?)
    }

    pub fn worker_report(&self, worker_id: &str) -> Result<Value> {
        let key = project_key(&self.root);
        let run = self.active_run_snapshot(&key)?;
        let worker = run.workers.get(worker_id).context("unknown Worker")?;
        Ok(
            json!({"worker_id": worker_id, "status": worker.status, "report": worker.report, "error": worker.error}),
        )
    }

    pub fn complete_worker(&self, worker_id: &str, report_file: &Path) -> Result<Value> {
        let config = self.enabled_config()?;
        let mut report: WorkerReport = serde_json::from_reader(
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
            worker_display_name(run.workers.get(worker_id).context("unknown Worker")?)
        };
        match report.status {
            ReportStatus::Blocked => {
                self.store_report(&key, worker_id, report, WorkerStatus::Blocked)?;
                self.notify(
                    &key,
                    &format!(
                        "{display_name} is blocked (internal ID: {worker_id}). Inspect its report and follow up."
                    ),
                );
                self.worker_status(worker_id)
            }
            ReportStatus::Failed => {
                self.store_report(&key, worker_id, report, WorkerStatus::Failed)?;
                self.notify(
                    &key,
                    &format!(
                        "{display_name} failed (internal ID: {worker_id}). Its changes were retained."
                    ),
                );
                self.worker_status(worker_id)
            }
            ReportStatus::Completed => {
                let run = self.active_run_snapshot(&key)?;
                let worker = run.workers.get(worker_id).context("unknown Worker")?;
                let checkout = PathBuf::from(
                    worker
                        .checkout_path
                        .as_deref()
                        .context("Worker checkout is unavailable")?,
                );
                let (worker_head, changed_paths) = if worker.use_worktree {
                    git::ensure_clean(&checkout)?;
                    let worker_head = git::head(&checkout)?;
                    ensure!(
                        git::is_ancestor(&checkout, &worker.base_sha, &worker_head)?,
                        "Worker history no longer descends from its assigned base"
                    );
                    ensure!(worker_head != worker.base_sha, "Worker produced no commits");
                    if let Some(reported) = report.commit_sha.as_deref() {
                        ensure!(
                            reported == worker_head,
                            "reported commit_sha is not Worker HEAD"
                        );
                    }
                    let changed_paths =
                        git::changed_paths(&checkout, &worker.base_sha, &worker_head)?;
                    (worker_head, changed_paths)
                } else {
                    let reported_commit = report
                        .commit_sha
                        .as_deref()
                        .context("shared-checkout Workers must report commit_sha")?;
                    let worker_commit = git::resolve_commit(&checkout, reported_commit)?;
                    ensure!(
                        worker_commit != worker.base_sha,
                        "Worker produced no commits"
                    );
                    ensure!(
                        git::is_ancestor(&checkout, &worker.base_sha, &worker_commit)?,
                        "Worker commit does not descend from its assigned base"
                    );
                    let current_head = git::head(&checkout)?;
                    ensure!(
                        git::is_ancestor(&checkout, &worker_commit, &current_head)?,
                        "Worker commit is not on the current base branch"
                    );
                    let changed_paths = git::changed_paths_for_commit(&checkout, &worker_commit)?;
                    (worker_commit, changed_paths)
                };
                ensure!(!changed_paths.is_empty(), "Worker commits changed no paths");
                let outside_scope = changed_paths
                    .iter()
                    .filter(|path| !path_within_scope(path, &worker.scope))
                    .cloned()
                    .collect::<Vec<_>>();
                ensure!(
                    outside_scope.is_empty(),
                    "Worker changed paths outside its reserved scope: {}",
                    outside_scope.join(", ")
                );
                report.commit_sha = Some(worker_head);
                report.changed_paths = changed_paths;
                self.store_report(&key, worker_id, report, WorkerStatus::Completed)?;
                if worker.use_worktree {
                    self.notify(
                        &key,
                        &format!(
                            "{display_name} completed (internal ID: {worker_id}). Review its report, then run `worker integrate {worker_id}` to accept the isolated commit."
                        ),
                    );
                    self.worker_status(worker_id)
                } else if config.git.auto_integrate {
                    self.integrate_worker(worker_id)
                } else {
                    self.worker_status(worker_id)
                }
            }
        }
    }

    pub fn integrate_worker(&self, worker_id: &str) -> Result<Value> {
        let config = self.enabled_config()?;
        let key = project_key(&self.root);
        let run = self.active_run_snapshot(&key)?;
        let worker = run
            .workers
            .get(worker_id)
            .context("unknown Worker")?
            .clone();
        ensure!(
            matches!(
                worker.status,
                WorkerStatus::Completed | WorkerStatus::Conflict
            ),
            "Worker must have a completed report before integration"
        );
        self.state.update(|store| {
            worker_mut(active_run_mut(store, &key)?, worker_id)?.status = WorkerStatus::Integrating;
            Ok(())
        })?;
        let result = (|| -> Result<()> {
            git::ensure_clean(&self.root)?;
            ensure!(
                git::current_branch(&self.root)? == run.base_branch,
                "base checkout changed branches; expected {}",
                run.base_branch
            );
            if !worker.use_worktree {
                let commit = worker
                    .report
                    .as_ref()
                    .and_then(|report| report.commit_sha.as_deref())
                    .context("Worker report omitted commit_sha")?;
                let current_head = git::head(&self.root)?;
                ensure!(
                    git::is_ancestor(&self.root, commit, &current_head)?,
                    "Worker commit is not on the current base branch"
                );
                return Ok(());
            }
            git::ensure_clean(&self.root)?;
            let checkout = PathBuf::from(
                worker
                    .checkout_path
                    .as_deref()
                    .context("Worker checkout is unavailable")?,
            );
            let worker_head = git::head(&checkout)?;
            let commits = git::commits_between(&checkout, &worker.base_sha, &worker_head)?;
            git::cherry_pick(&self.root, &commits)
        })();
        match result {
            Ok(()) => {
                self.state.update(|store| {
                    let stored = worker_mut(active_run_mut(store, &key)?, worker_id)?;
                    stored.status = WorkerStatus::Integrated;
                    stored.error = None;
                    Ok(())
                })?;
                let message = if worker.use_worktree {
                    if config.git.cleanup_on_success {
                        format!(
                            "{} integrated successfully (internal ID: {worker_id}); its agent and worktree were cleaned up.",
                            worker_display_name(&worker)
                        )
                    } else {
                        format!(
                            "{} integrated successfully (internal ID: {worker_id}); its agent and worktree were retained by configuration.",
                            worker_display_name(&worker)
                        )
                    }
                } else if config.git.cleanup_on_success {
                    format!(
                        "{} completed successfully on the shared base branch (internal ID: {worker_id}); its agent and tab were cleaned up.",
                        worker_display_name(&worker)
                    )
                } else {
                    format!(
                        "{} completed successfully on the shared base branch (internal ID: {worker_id}); its agent and tab were retained by configuration.",
                        worker_display_name(&worker)
                    )
                };
                if config.git.cleanup_on_success {
                    if self.herdr.agent_exists(&worker.agent_name) {
                        let _ = self.herdr.send_ctrl_c(&worker.agent_name);
                    }
                    self.cleanup_worker(&key, worker_id)?;
                }
                self.notify(&key, &message);
                self.worker_status(worker_id)
            }
            Err(error) => {
                self.state.update(|store| {
                    let stored = worker_mut(active_run_mut(store, &key)?, worker_id)?;
                    stored.status = WorkerStatus::Conflict;
                    stored.error = Some(error.to_string());
                    Ok(())
                })?;
                self.notify(&key, &format!("{} could not integrate (internal ID: {worker_id}). Its changes and isolated resources were retained.", worker_display_name(&worker)));
                self.worker_status(worker_id)
            }
        }
    }

    pub fn prompt_worker(&self, worker_id: &str, prompt_file: &Path) -> Result<Value> {
        let prompt = fs::read_to_string(prompt_file)?;
        ensure!(!prompt.trim().is_empty(), "prompt cannot be empty");
        let key = project_key(&self.root);
        let run = self.active_run_snapshot(&key)?;
        let worker = run.workers.get(worker_id).context("unknown Worker")?;
        ensure!(
            self.herdr.agent_exists(&worker.agent_name),
            "Worker agent is not running"
        );
        self.herdr.prompt_agent(&worker.agent_name, &prompt)?;
        self.state.update(|store| {
            let worker = worker_mut(active_run_mut(store, &key)?, worker_id)?;
            worker.status = WorkerStatus::Working;
            worker.error = None;
            Ok(())
        })?;
        self.worker_status(worker_id)
    }

    pub fn cancel_worker(&self, worker_id: &str) -> Result<Value> {
        let key = project_key(&self.root);
        let run = self.active_run_snapshot(&key)?;
        let worker = run.workers.get(worker_id).context("unknown Worker")?;
        if self.herdr.agent_exists(&worker.agent_name) {
            self.herdr.send_ctrl_c(&worker.agent_name)?;
        }
        self.state.update(|store| {
            worker_mut(active_run_mut(store, &key)?, worker_id)?.status = WorkerStatus::Cancelled;
            Ok(())
        })?;
        self.worker_status(worker_id)
    }

    pub fn finish_run(&self) -> Result<Value> {
        let config = self.enabled_config()?;
        let key = project_key(&self.root);
        let run = self.active_run_snapshot(&key)?;
        if let Some(worker) = run.workers.values().find(|w| !w.status.is_terminal()) {
            bail!(
                "Worker {} is not in a terminal state ({:?})",
                worker.id,
                worker.status
            );
        }
        if config.git.cleanup_on_success
            && let Some(worker) = run.workers.values().find(|w| {
                w.status == WorkerStatus::Integrated
                    && (w.workspace_id.is_some() || w.tab_id.is_some())
            })
        {
            bail!(
                "Worker {} is integrated but its resources have not been cleaned up",
                worker.id
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
        let Some((key, run_id, worker_id, worker)) = self.find_worker_by_pane(pane_id)? else {
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
                let previous = worker.observed_agent_status.as_deref();
                self.state.update(|store| {
                    worker_mut(run_mut(store, &key, &run_id)?, &worker_id)?.observed_agent_status =
                        Some(status.to_string());
                    Ok(())
                })?;
                if status == "blocked" && previous != Some("blocked") {
                    self.notify(
                        &key,
                        &format!(
                            "{} is waiting for input (internal ID: {worker_id}).",
                            worker_display_name(&worker)
                        ),
                    );
                } else if matches!(status, "idle" | "done")
                    && worker.status == WorkerStatus::Working
                    && !matches!(previous, Some("idle" | "done"))
                {
                    self.notify(
                        &key,
                        &format!(
                            "{} is idle but has not submitted a completion report (internal ID: {worker_id}).",
                            worker_display_name(&worker)
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
                    self.handle_worker_exit(&key, &run_id, &worker_id, &worker)?;
                }
            }
            "pane.exited" => self.handle_worker_exit(&key, &run_id, &worker_id, &worker)?,
            _ => {}
        }
        Ok(json!({"handled": true, "worker_id": worker_id, "event": event_name}))
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
            for worker in run.workers.values() {
                if !self.herdr.agent_exists(&worker.agent_name) {
                    self.handle_worker_exit(&key, &run_id, &worker.id, worker)?;
                    reconciled += 1;
                }
            }
            if !self.herdr.agent_exists(&run.orchestrator.name) {
                self.set_run_error(
                    &key,
                    &run_id,
                    "Orchestrator is not running; invoke Cadence start to resume",
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
        worker_id: &str,
        report: WorkerReport,
        status: WorkerStatus,
    ) -> Result<()> {
        self.state.update(|store| {
            let worker = worker_mut(active_run_mut(store, key)?, worker_id)?;
            worker.report = Some(report);
            worker.status = status;
            Ok(())
        })
    }

    fn fail_worker(&self, key: &str, worker_id: &str, message: &str) -> Result<()> {
        self.state.update(|store| {
            let worker = worker_mut(active_run_mut(store, key)?, worker_id)?;
            worker.status = WorkerStatus::Failed;
            worker.error = Some(message.to_string());
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
            let _ = self.herdr.prompt_agent(
                &run.orchestrator.name,
                &format!("Cadence update: {message}"),
            );
        }
    }

    fn find_worker_by_pane(
        &self,
        pane_id: &str,
    ) -> Result<Option<(String, String, String, Worker)>> {
        let store = self.state.read()?;
        for (key, project) in store.projects {
            let Some(run_id) = project.active_run else {
                continue;
            };
            let Some(run) = project.runs.get(&run_id) else {
                continue;
            };
            if let Some(worker) = run
                .workers
                .values()
                .find(|w| w.pane_id.as_deref() == Some(pane_id))
            {
                return Ok(Some((key, run_id, worker.id.clone(), worker.clone())));
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

    fn handle_worker_exit(
        &self,
        key: &str,
        run_id: &str,
        worker_id: &str,
        worker: &Worker,
    ) -> Result<()> {
        if worker.status == WorkerStatus::Integrated {
            let root = PathBuf::from(self.project_root_for_key(key)?);
            if Config::load(&root).is_ok_and(|config| config.git.cleanup_on_success) {
                self.cleanup_worker(key, worker_id)?;
            }
        } else if !worker.status.is_terminal() {
            self.state.update(|store| {
                let stored = worker_mut(run_mut(store, key, run_id)?, worker_id)?;
                stored.status = WorkerStatus::Failed;
                stored.error = Some("agent exited without a terminal completion report".into());
                Ok(())
            })?;
            self.notify(
                key,
                &format!(
                    "{} exited without completing (internal ID: {worker_id}); its changes and isolated resources were retained.",
                    worker_display_name(worker)
                ),
            );
        }
        Ok(())
    }

    fn cleanup_worker(&self, key: &str, worker_id: &str) -> Result<()> {
        let run = self.active_run_snapshot(key)?;
        let worker = run
            .workers
            .get(worker_id)
            .context("unknown Worker")?
            .clone();
        if worker.use_worktree {
            if let Some(workspace_id) = worker.workspace_id.as_deref() {
                self.herdr.remove_worktree(workspace_id)?;
            }
        } else if let Some(tab_id) = worker.tab_id.as_deref() {
            let _ = self.herdr.close_tab(tab_id);
        }
        self.state.update(|store| {
            let stored = worker_mut(active_run_mut(store, key)?, worker_id)?;
            stored.workspace_id = None;
            stored.tab_id = None;
            stored.pane_id = None;
            stored.checkout_path = None;
            Ok(())
        })?;
        if worker.use_worktree {
            let root = PathBuf::from(self.project_root_for_key(key)?);
            git::delete_branch(&root, &worker.branch)?;
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

fn worker_mut<'a>(run: &'a mut Run, worker_id: &str) -> Result<&'a mut Worker> {
    run.workers.get_mut(worker_id).context("unknown Worker")
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

fn worker_agent_args(worker: &Worker, state_dir: &Path) -> Vec<String> {
    if worker.yolo {
        return yolo_agent_args(worker.harness, true);
    }
    if !worker.use_worktree {
        return Vec::new();
    }
    match worker.harness {
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
    if role.eq_ignore_ascii_case("research") || role.eq_ignore_ascii_case("researcher") {
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

fn worker_display_name(worker: &Worker) -> String {
    format!("[{}] {}", display_role(&worker.role), worker.title)
}

#[cfg(test)]
mod tests {
    use super::display_role;

    #[test]
    fn formats_worker_roles_for_labels() {
        assert_eq!(display_role("research"), "Researcher");
        assert_eq!(display_role("qa"), "QA");
        assert_eq!(display_role("docs_writer"), "Docs Writer");
    }
}

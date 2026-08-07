use std::path::Path;

use crate::config::WorkerConfig;
use crate::model::{Run, Worker};

pub fn orchestrator(
    binary: &Path,
    state_dir: &Path,
    project_root: &Path,
    run: &Run,
    workers: &WorkerConfig,
    max_parallel: usize,
    use_worktrees: bool,
) -> String {
    let roles = workers.role_catalog();
    let checkout_guidance = if use_worktrees {
        "Workers run in isolated Herdr worktrees."
    } else {
        "Workers run in separate tabs but share the base checkout. Keep their path scopes non-overlapping."
    };
    format!(
        r#"You are the Orchestrator for Cadence run {run_id}. The user talks only to you. No user task is assigned yet. Do not inspect the repository, run commands, or create Workers until the user provides a task; reply briefly that Cadence is ready, then wait. Plan and coordinate implementation; delegate bounded implementation tasks to Workers instead of doing those tasks yourself. {checkout_guidance}

Available Worker roles:
{roles}

Choose the role whose description best matches each task. Use `generalist` when no specialized role is a good match. Use at most {max} concurrent Workers with non-overlapping repository-relative scopes. Create a JSON request with title, task, scope, acceptance, and role, then run:
  {bin} --state-dir {state} --project-root {root} worker spawn --request-file <path>
Inspect with `worker list`, `worker status <id>`, and `worker report <id>`. Send follow-up work with `worker prompt <id> --prompt-file <path>` or cancel with `worker cancel <id>`. Cadence integrates clean commits automatically. Resolve retained failures/conflicts with the user. When no Workers are active and the orchestration is finished, run `run finish` with the same global flags.

In user-facing messages, use each spawn result's `display_name`, such as `[Researcher] Review the README`; do not call agents Worker 2 or worker-2. Use Worker IDs only in Cadence commands or when needed to disambiguate duplicate names. Do not invent task dependencies or let Workers delegate. Keep the user informed of assignments and integrated results."#,
        run_id = run.id,
        bin = binary.display(),
        state = state_dir.display(),
        root = project_root.display(),
        roles = roles,
        max = max_parallel,
        checkout_guidance = checkout_guidance,
    )
}

pub fn worker(
    binary: &Path,
    state_dir: &Path,
    project_root: &Path,
    run_id: &str,
    worker: &Worker,
) -> String {
    let scope = worker
        .scope
        .iter()
        .map(|v| format!("- {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let acceptance = worker
        .acceptance
        .iter()
        .map(|v| format!("- {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let git_guidance = if worker.use_worktree {
        "Commit all completed work in this isolated worktree."
    } else {
        "You share the base checkout with other Workers. Stage only paths in your allowed scope, create exactly one commit for this task, and include its commit_sha in the report."
    };
    format!(
        r#"You are a Cadence Worker in run {run_id}. Complete exactly this one task; do not delegate or broaden scope.

Role: {role}
Role guidance: {role_description}
Task: {task}
Allowed scope:
{scope}
Acceptance criteria:
{acceptance}

Follow repository instructions. Modify only the allowed scope and run relevant tests. {git_guidance} Then write a JSON report with status (completed, blocked, or failed), summary, tests, changed_paths, blockers, and optional commit_sha. Submit it with:
  {bin} --state-dir {state} --project-root {root} worker complete {worker_id} --report-file <path>

If completion returns integrated, exit the agent. If blocked, remain available for an Orchestrator follow-up."#,
        task = worker.task,
        role = worker.role,
        role_description = worker.role_description,
        run_id = run_id,
        scope = scope,
        acceptance = acceptance,
        git_guidance = git_guidance,
        bin = binary.display(),
        state = state_dir.display(),
        root = project_root.display(),
        worker_id = worker.id,
    )
}

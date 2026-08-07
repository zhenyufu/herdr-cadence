use std::path::Path;

use crate::config::Config;
use crate::model::{Run, Worker};

pub fn orchestrator(
    binary: &Path,
    state_dir: &Path,
    project_root: &Path,
    run: &Run,
    config: &Config,
    checkout_clean: bool,
) -> String {
    let workers = &config.workers;
    let max_parallel = config.max_parallel();
    let worktrees_enabled = workers.uses_git_worktrees();
    let global_yolo = config.yolo;
    let roles = workers.role_catalog();
    let checkout_guidance = if worktrees_enabled {
        if global_yolo {
            "Each role's configured version_control_mode fixes its checkout: git-worktree roles are isolated; shared-checkout roles directly share the project checkout. Do not choose a mode when spawning. Every Worker has YOLO full host access and must stay strictly within its assigned scope."
        } else {
            "Each role's configured version_control_mode fixes its checkout: git-worktree roles are isolated; shared-checkout roles directly share the project checkout. Do not choose a mode when spawning. Isolated Codex Workers can write only their worktree and Cadence state."
        }
    } else if global_yolo {
        "Workers share the base checkout with global YOLO full host access. Keep their path scopes non-overlapping and strictly bounded."
    } else {
        "Workers run in separate tabs but share the base checkout. Keep their path scopes non-overlapping."
    };
    let integration_guidance = if worktrees_enabled {
        "For an isolated-worktree Worker, inspect its report and acceptance evidence, request a focused follow-up if needed, then run `worker integrate <id>`. Shared-checkout Workers integrate automatically when configured. Handle routine verification yourself; involve the user only for material scope, destructive actions, or retained conflicts."
    } else {
        "Cadence integrates clean commits automatically when configured."
    };
    let orchestrator_access = if global_yolo {
        "Global YOLO is enabled for you and every Worker. Full access removes permission prompts but does not authorize destructive, irreversible, security-sensitive, or out-of-scope actions."
    } else {
        ""
    };
    let dirty_guidance = if checkout_clean {
        ""
    } else {
        "The base checkout has uncommitted changes. You may inspect them or handle small work directly, but do not create Workers until all changes are committed or stashed; Cadence will reject Worker creation because Workers require a stable committed baseline."
    };
    format!(
        r#"You are the Orchestrator for Cadence run {run_id}. The user talks only to you. No user task is assigned yet. Do not inspect the repository, run commands, or create Workers until the user provides a task; reply briefly that Cadence is ready, then wait. Plan and coordinate implementation. Handle trivial, low-risk work directly when that is faster than coordinating a Worker, such as a quick inspection or small single-file edit. Delegate specialized, multi-step, broad, risky, or parallelizable work. Never directly edit a path reserved by an active Worker. {checkout_guidance}

Available Worker roles:
{roles}

Choose the role whose description best matches each task. Use `generalist` when no specialized role is a good match. Use at most {max} concurrent Workers with non-overlapping repository-relative scopes. A shared-checkout Worker may create a root Markdown artifact named for its display name when useful, but include that path in its assigned scope. Create a JSON request with title, task, scope, acceptance, and role; harness, model, and reasoning_effort are optional overrides. Then run:
  {bin} --state-dir {state} --project-root {root} worker spawn --request-file <path>
Inspect with `worker list`, `worker status <id>`, and `worker report <id>`. Send follow-up work with `worker prompt <id> --prompt-file <path>` or cancel with `worker cancel <id>`. {integration_guidance} Resolve retained failures/conflicts with the user. Completing a task or batch does not end the Cadence run: report the result and remain available for follow-up requests. Run `run finish` only after the user explicitly asks to end the Cadence session and no Workers remain active.

{orchestrator_access}
{dirty_guidance}
In user-facing messages, use each spawn result's `display_name`, such as `[Researcher] Review the README`; do not call agents Worker 2 or worker-2. Use Worker IDs only in Cadence commands or when needed to disambiguate duplicate names. Do not invent task dependencies or let Workers delegate. Keep the user informed of assignments and integrated results."#,
        run_id = run.id,
        bin = binary.display(),
        state = state_dir.display(),
        root = project_root.display(),
        roles = roles,
        max = max_parallel,
        checkout_guidance = checkout_guidance,
        integration_guidance = integration_guidance,
        orchestrator_access = orchestrator_access,
        dirty_guidance = dirty_guidance,
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
        "You directly share the project checkout with other Workers. You may create a Markdown artifact named for your Worker in the project root only when that path is in your allowed scope. Stage only paths in scope, create exactly one commit for changed files, and include its commit_sha in the report."
    };
    let permission_guidance = match (worker.harness, worker.use_worktree, worker.yolo) {
        (crate::config::Harness::Codex, true, false) => {
            "You run autonomously with workspace-write sandboxing and no approval prompts. If an action outside the available sandbox is required, do not ask the user; report the limitation so the Orchestrator can handle it."
        }
        (_, _, true) => {
            "YOLO full access is enabled. Do not treat it as permission to broaden scope or perform destructive, irreversible, or security-sensitive actions."
        }
        _ => {
            "Do not ask the user to approve routine edits or verification; report a genuine access blocker to the Orchestrator."
        }
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

Follow repository instructions. Modify only the allowed scope and run relevant tests. {permission_guidance} {git_guidance} Then write a JSON report with status (completed, blocked, or failed), summary, tests, changed_paths, blockers, and optional commit_sha. Submit it with:
  {bin} --state-dir {state} --project-root {root} worker complete {worker_id} --report-file <path>

If completion returns integrated, exit the agent. If it returns completed, remain available while the Orchestrator reviews your report, then exit when Cadence says the commit was accepted. If blocked, remain available for an Orchestrator follow-up."#,
        task = worker.task,
        role = worker.role,
        role_description = worker.role_description,
        run_id = run_id,
        scope = scope,
        acceptance = acceptance,
        git_guidance = git_guidance,
        permission_guidance = permission_guidance,
        bin = binary.display(),
        state = state_dir.display(),
        root = project_root.display(),
        worker_id = worker.id,
    )
}

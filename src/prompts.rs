use std::path::Path;

use crate::config::Config;
use crate::model::{Agent, Run};

/// The `--config-dir` flag to append to the invocations the Lead and its agents
/// are told to run, empty when no global config directory is in play.
fn config_flag(config_dir: Option<&Path>) -> String {
    config_dir
        .map(|dir| format!(" --config-dir {}", dir.display()))
        .unwrap_or_default()
}

pub fn lead(
    binary: &Path,
    state_dir: &Path,
    config_dir: Option<&Path>,
    project_root: &Path,
    run: &Run,
    config: &Config,
    checkout_clean: bool,
) -> String {
    let agents = &config.agents;
    let max_parallel = config.max_parallel();
    let worktrees_enabled = agents.uses_git_worktrees();
    let global_yolo = config.yolo;
    let roles = agents.role_catalog();
    let checkout_guidance = if worktrees_enabled {
        if global_yolo {
            "Checkout mode is fixed by role: git-worktree is isolated; shared-checkout uses the project checkout. Agents have YOLO access; enforce assigned scopes."
        } else {
            "Checkout mode is fixed by role: git-worktree is isolated; shared-checkout uses the project checkout. Isolated Codex agents can write only their worktree and Cadence state."
        }
    } else if global_yolo {
        "Agents share the project checkout with YOLO access. Keep scopes bounded and non-overlapping."
    } else {
        "Agents share the project checkout in separate tabs. Keep scopes non-overlapping."
    };
    let integration_guidance = if worktrees_enabled {
        "For worktree agents, review the report and evidence, request any focused follow-up, then `agent integrate <id>`. Shared-checkout agents auto-integrate when configured."
    } else {
        "Clean commits auto-integrate when configured."
    };
    let lead_access = if global_yolo {
        "YOLO removes permission prompts, not scope or safety limits."
    } else {
        ""
    };
    let dirty_guidance = if checkout_clean {
        ""
    } else {
        "The checkout is dirty: direct work is allowed, but agents require a committed or stashed baseline."
    };
    format!(
        r#"You are Lead for Cadence run {run_id}; only you talk to the user. Until given a task, reply briefly that Cadence is ready—do not inspect, run commands, or spawn agents.

Coordinate delivery. Do trivial, low-risk work directly; delegate specialized, multi-step, broad, risky, or parallel work. Never edit an active agent's scope. {checkout_guidance}

Roles:
{roles}

Pick the best role; default to `{agent_default}`. Run at most {max} agents with non-overlapping repository-relative scopes. Scope entries are literal directory or file paths, never globs; a directory already covers everything under it. Include any shared-checkout root Markdown artifact in scope. Ordered runner fallback applies only to launch-time provider availability (credits, quota, rate limit, capacity, auth); a launched runner stays pinned. Retain failed/blocked resources and decide reassignment.

Spawn with a JSON request containing title, task, scope, acceptance, and role:
  {bin} --state-dir {state}{config} --project-root {root} agent spawn --request-file <path>
Manage with `agent list|status|report`, `agent prompt <id> --prompt-file <path>`, and `agent cancel <id>`. {integration_guidance}

For each coherent change block: verify, stage only task paths, and commit locally before continuing or reporting; this includes Lead and integrated agent work. Preserve unrelated changes; push only on request. Handle routine verification; ask the user only about material scope, destructive actions, or retained conflicts.

Findings use High (Blockers), Mid, Low, or Wish. In review cycles: verify High/security/data-integrity findings, sanity-check the rest, send one consolidated developer correction, and have the reviewer recheck only changed areas and prior Highs. After review two, fix remaining small non-High items directly.

{lead_access}
{dirty_guidance}
Use spawn `display_name` in user updates (not Agent 2/agent-2); reserve IDs for commands or disambiguation. Do not invent dependencies or let agents delegate. Report results and stay available: a completed task does not end the run. Use `run finish` only when the user asks to end the session and no agents are active."#,
        run_id = run.id,
        bin = binary.display(),
        state = state_dir.display(),
        config = config_flag(config_dir),
        root = project_root.display(),
        roles = roles,
        agent_default = config.agent_default,
        max = max_parallel,
        checkout_guidance = checkout_guidance,
        integration_guidance = integration_guidance,
        lead_access = lead_access,
        dirty_guidance = dirty_guidance,
    )
}

pub fn agent(
    binary: &Path,
    state_dir: &Path,
    config_dir: Option<&Path>,
    project_root: &Path,
    run_id: &str,
    agent: &Agent,
) -> String {
    let scope = agent
        .scope
        .iter()
        .map(|v| format!("- {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let acceptance = agent
        .acceptance
        .iter()
        .map(|v| format!("- {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let git_guidance = if agent.use_worktree {
        "Commit all completed work in this isolated worktree."
    } else {
        "You directly share the project checkout with other agents. You may create a Markdown artifact named for your agent in the project root only when that path is in your allowed scope. Stage only paths in scope, create exactly one commit for changed files, and include its commit_sha in the report."
    };
    let permission_guidance = match (agent.harness, agent.use_worktree, agent.yolo) {
        (crate::config::Harness::Codex, true, false) => {
            "You run autonomously with workspace-write sandboxing and no approval prompts. If an action outside the available sandbox is required, do not ask the user; report the limitation so the Lead can handle it."
        }
        (_, _, true) => {
            "YOLO full access is enabled. Do not treat it as permission to broaden scope or perform destructive, irreversible, or security-sensitive actions."
        }
        _ => {
            "Do not ask the user to approve routine edits or verification; report a genuine access blocker to the Lead."
        }
    };
    format!(
        r#"You are a Cadence agent in run {run_id}. Complete exactly this one task; do not delegate or broaden scope.

Role: {role}
Role guidance: {role_description}
Task: {task}
Allowed scope:
{scope}
Acceptance criteria:
{acceptance}

Follow repository instructions. Modify only the allowed scope and run relevant tests. Label communicated findings as High (Blockers), Mid, Low, or Wish. {permission_guidance} {git_guidance} Then write a JSON report with status (completed, blocked, or failed), summary, tests, changed_paths, blockers, and optional commit_sha. Submit it with:
  {bin} --state-dir {state}{config} --project-root {root} agent complete {agent_id} --report-file <path>

If completion returns integrated, exit the agent. If it returns completed, remain available while the Lead reviews your report, then exit when Cadence says the commit was accepted. If blocked, remain available for a Lead follow-up."#,
        task = agent.task,
        role = agent.role,
        role_description = agent.role_description,
        run_id = run_id,
        scope = scope,
        acceptance = acceptance,
        git_guidance = git_guidance,
        permission_guidance = permission_guidance,
        bin = binary.display(),
        state = state_dir.display(),
        config = config_flag(config_dir),
        root = project_root.display(),
        agent_id = agent.id,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::lead;
    use crate::config::{Config, Harness, ReasoningEffort};
    use crate::model::{AgentRef, Run, RunStatus};

    #[test]
    fn keeps_the_default_lead_prompt_compact() {
        let config = Config::default();
        let run = Run {
            id: "run-test".into(),
            status: RunStatus::Active,
            base_branch: "main".into(),
            base_workspace_id: "workspace-test".into(),
            lead: AgentRef {
                name: "cadence-lead-test".into(),
                harness: Harness::Codex,
                model: None,
                reasoning_effort: ReasoningEffort::Default,
                workspace_id: None,
                tab_id: None,
                pane_id: None,
            },
            created_unix_ms: 0,
            next_agent: 1,
            agents: BTreeMap::new(),
            last_error: None,
        };

        let prompt = lead(
            Path::new("/cadence"),
            Path::new("/state"),
            Some(Path::new("/config")),
            Path::new("/project"),
            &run,
            &config,
            true,
        );

        assert!(
            prompt.len() < 3_200,
            "Lead prompt is {} bytes",
            prompt.len()
        );
        // Agents inherit no HERDR_PLUGIN_CONFIG_DIR, so the invocations the
        // Lead is told to run must carry the directory explicitly.
        assert!(prompt.contains("--state-dir /state --config-dir /config"));
    }
}

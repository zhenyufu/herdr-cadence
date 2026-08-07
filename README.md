# Cadence

Cadence is a Herdr plugin that gives one Codex or OpenCode **Orchestrator** a small fleet of **Workers**. You talk to the Orchestrator in its dedicated Herdr workspace; Cadence creates Worker tabs, tracks reports, and can optionally isolate work in Git worktrees.

## Install

Requires Herdr 0.7.5+, Git, and either Codex or OpenCode.

```sh
herdr plugin install zhenyufu/herdr-cadence
```

Inside the target repository's Herdr workspace:

```sh
herdr plugin action invoke herdr-cadence.enable-project
```

Edit and commit `.herdr/cadence.toml`, then start the conversational Orchestrator:

```sh
herdr plugin action invoke herdr-cadence.start
```

Cadence is globally installed but only acts in repositories with an enabled config. It never creates or changes `AGENTS.md`; role and task context is injected when each agent starts.

## Codex approvals

Codex may request permission when an agent runs the Cadence binary outside its project sandbox. At the first request, use Codex's option to always allow the suggested command prefix. To configure it manually, add this rule to `~/.codex/rules/default.rules`:

```python
prefix_rule(
    pattern=["<cadence-bin>", "--state-dir", "<cadence-state-dir>", "--project-root"],
    decision="allow",
    justification="Allow Cadence to coordinate Workers",
)
```

Replace `<cadence-bin>` and `<cadence-state-dir>` with the absolute values shown in the permission request, then restart the Codex agents. The rule applies to Cadence in any enabled project while remaining scoped to that binary and state directory.

## Configuration

```toml
schema_version = 1
enabled = true
yolo = false # full access for the Orchestrator and every Worker

[orchestrator]
harness = "codex" # or "opencode"
# model = "your-model-id" # optional; omit to use the harness default
max_parallel = 4

[workers]
harness = "inherit" # or "codex" / "opencode"
# model = "your-model-id" # optional; omit to use the harness default
yolo_with_worktrees_only = false # full access for isolated Workers only
generalist_description = "Use for general implementation tasks that do not match a specialized role"

[workers.roles.research]
description = "Use for investigation and evidence gathering"
harness = "inherit"
# model = "your-research-model-id"

[workers.roles.qa]
description = "Use for test planning, validation, and regression investigation"
harness = "codex"
# model = "your-qa-model-id"

[git]
use_worktrees = false
auto_integrate = true
cleanup_on_success = true
```

The top-level `[workers]` harness and model define the `generalist` fallback. The Orchestrator chooses a named role from its description and uses `generalist` when none is a good match. When `model` is omitted, Cadence lets the selected harness use its default model. Workers can inherit the Orchestrator's harness, but not its configured model. A Worker request can override its role's harness/model, but cannot exceed `max_parallel` or overlap another active Worker's path scope.

For compatibility, Cadence still accepts `max_parallel` under `[workers]` in existing schema-version-1 configs, but it cannot be set in both sections. Workers use the shared project checkout by default; set `git.use_worktrees = true` for isolated branches and checkouts.

Isolated Codex Workers run autonomously with `workspace-write` sandboxing and no approval prompts. They can write their worktree and Cadence state; unavailable external operations fail for the Worker to report to the Orchestrator. Top-level `yolo = true` is an unrestricted global override for the Orchestrator and every Worker and does not require worktrees. For full-access Workers with a normally supervised Orchestrator, set `workers.yolo_with_worktrees_only = true`; this Worker-only mode requires worktrees. Both modes pass Codex's dangerous approval-and-sandbox bypass (or OpenCode auto-approval) to the affected agents. Worktrees isolate Git changes, not host access.

Successful Workers must commit clean work. In shared-checkout mode, each Worker commits only its reserved scope directly on the base branch. In worktree mode, the Orchestrator reviews each completed report before invoking integration; Cadence then cherry-picks the accepted commits, terminates that Worker, and removes its worktree when `cleanup_on_success = true`. `git.auto_integrate` applies only to shared-checkout Workers. Completed worktrees remain available until review, while failed, cancelled, or conflicting worktrees are retained for inspection; failed cherry-picks are aborted automatically.

## Injected context

Cadence sends one compact startup prompt to each agent:

- The Orchestrator receives its run ID, coordination rules, checkout mode, concurrency limit, configured role names and descriptions, and the commands for managing Workers and finishing the run.
- A Worker receives only its role and role description, assigned task, allowed path scope, acceptance criteria, checkout-specific Git instructions, and the command for submitting its report.

Cadence does not inject source files, diffs, the full configuration or state store, other Workers' tasks or reports, or conversation history. The selected harness may independently load repository instructions such as `AGENTS.md` and inspect files as it works. Later Cadence messages are short status notifications to the Orchestrator or explicit follow-up prompts sent to a Worker.

View the current run with:

```sh
herdr plugin action invoke herdr-cadence.status
```

## Develop

```sh
./scripts/build-local.sh
herdr plugin link "$PWD"
cargo test
```

Releases publish checksummed macOS and Linux binaries for arm64 and x86-64. The Herdr installer downloads the matching binary, so users do not need Rust.

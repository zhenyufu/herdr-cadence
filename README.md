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
use_git_worktrees = false

[orchestrator]
harness = "codex" # or "opencode"
model = "gpt-5.6-sol" # optional; omit to use the harness default
reasoning_effort = "high" # default / low / medium / high / xhigh
max_parallel = 4

[git]
auto_integrate = true
cleanup_on_success = true

[workers]
harness = "inherit" # or "codex" / "opencode"
# model = "your-model-id" # optional; omit to use the harness default
reasoning_effort = "default" # inherited by roles unless overridden
yolo_with_worktrees_only = false # full access for isolated Workers only
generalist_description = "Use for general implementation tasks that do not match a specialized role"

[workers.roles.research]
description = "Use for investigation and evidence gathering"
harness = "inherit"
# model = "your-research-model-id"
# reasoning_effort = "high"

[workers.roles.qa]
description = "Use for test planning, validation, and regression investigation"
harness = "inherit"
# model = "your-qa-model-id"
# reasoning_effort = "high"
```

The top-level `[workers]` harness, model, and reasoning effort define the `generalist` fallback. The Orchestrator chooses a named role from its description and uses `generalist` when none is a good match. Named roles inherit the Worker reasoning effort unless they override it. A Worker request can override its role's harness, model, or reasoning effort, but cannot exceed `max_parallel` or overlap another active Worker's path scope.

Reasoning effort accepts `default`, `low`, `medium`, `high`, and `xhigh`. Cadence maps it to Codex's `model_reasoning_effort` and to an OpenCode model variant. OpenCode reasoning requires an explicit `provider/model`; for example, `model = "openai/gpt-5.2"` with `reasoning_effort = "high"` launches `openai/gpt-5.2#high`. Variant availability remains model-specific, including `xhigh`. When `model` is omitted, Cadence lets the selected harness use its default model. Workers can inherit the Orchestrator's harness, but not its configured model.

For compatibility, Cadence still accepts `max_parallel` under `[workers]` in existing schema-version-1 configs, but it cannot be set in both sections. Workers use the shared project checkout by default; set top-level `use_git_worktrees = true` for isolated branches and checkouts.

Isolated Codex Workers run autonomously with `workspace-write` sandboxing and no approval prompts. They can write their worktree and Cadence state; unavailable external operations fail for the Worker to report to the Orchestrator. Top-level `yolo = true` is an unrestricted global override for the Orchestrator and every Worker and does not require worktrees. For full-access Workers with a normally supervised Orchestrator, set `workers.yolo_with_worktrees_only = true`; this Worker-only mode requires worktrees. Both modes pass Codex's dangerous approval-and-sandbox bypass (or OpenCode auto-approval) to the affected agents. Worktrees isolate Git changes, not host access.

Successful Workers must commit clean work. In shared-checkout mode, each Worker commits only its reserved scope directly on the base branch. In worktree mode, the Orchestrator reviews each completed report before invoking integration. After successful integration, `cleanup_on_success = true` terminates the Worker and removes its tab or worktree; the Orchestrator and Cadence run remain active for follow-up tasks. `git.auto_integrate` applies only to shared-checkout Workers. Completed worktrees remain available until review, while failed, cancelled, or conflicting worktrees are retained for inspection; failed cherry-picks are aborted automatically. The run ends only when the user explicitly asks the Orchestrator to finish the Cadence session.

## Injected context

Cadence sends one compact startup prompt to each agent:

- The Orchestrator receives its run ID, coordination rules, checkout mode, concurrency limit, configured role names and descriptions, and the commands for managing Workers and finishing the run. It may handle trivial, low-risk work directly and delegates larger or specialized tasks.
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

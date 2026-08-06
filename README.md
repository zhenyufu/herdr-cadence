# Cadence

Cadence is a Herdr plugin that gives one Codex or OpenCode **Orchestrator** a small fleet of **Workers**. You talk to the Orchestrator; Cadence creates Worker tabs, tracks reports, and can optionally isolate work in Git worktrees.

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

## Configuration

```toml
schema_version = 1
enabled = true

[orchestrator]
harness = "codex" # or "opencode"
# model = "your-model-id" # optional; omit to use the harness default
max_parallel = 4

[workers]
harness = "inherit" # or "codex" / "opencode"
# model = "your-model-id" # optional; omit to use the harness default
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

Successful Workers must commit clean work. In shared-checkout mode, each Worker commits only its reserved scope directly on the base branch. In worktree mode, Cadence cherry-picks completed commits and cleans successful worktrees after their agents exit. Failed, cancelled, or conflicting worktrees are retained for inspection; failed cherry-picks are aborted automatically.

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

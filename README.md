# Cadence

Cadence is a Herdr plugin that gives one Codex or OpenCode **Orchestrator** a small fleet of isolated **Workers**. You talk to the Orchestrator; Cadence creates Worker worktrees and tabs, tracks reports, and integrates clean commits.

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

[workers]
harness = "inherit" # or "codex" / "opencode"
# model = "your-model-id" # optional; omit to use the harness default
max_parallel = 4

[git]
auto_integrate = true
cleanup_on_success = true
```

When `model` is omitted, Cadence lets the selected harness use its default model. Workers can inherit the Orchestrator's harness, but not its configured model; set `workers.model` when all Workers should use a specific model. A Worker request can override its default harness/model, but cannot exceed `max_parallel` or overlap another active Worker's path scope.

Successful Workers must commit clean work. Cadence cherry-picks their commits onto the original branch and cleans their worktree after the agent exits. Failed, cancelled, or conflicting Workers are retained for inspection; failed cherry-picks are aborted automatically.

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

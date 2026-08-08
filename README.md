# Cadence
Cadence is a light Orchestrating plugin for Herdr that provides one **Lead** and a fleet of **agents**. Talk to the Lead, and it will spin up agents with different roles fully integrated with Herdr tabs and git worktrees.

## Supported harness
Supported:
* Codex
Untested:
* Opencode

## Install and usage

Requires Herdr 0.7.5+, Git, and the agent harness

```sh
herdr plugin install zhenyufu/herdr-cadence
```

Init a cadence config inside the target repository's Herdr workspace:

```sh
herdr plugin action invoke herdr-cadence.init
```

Edit and commit `.herdr/cadence.toml`, then start the conversational Lead:

```sh
herdr plugin action invoke herdr-cadence.start
```

View the current run status:

```sh
herdr plugin action invoke herdr-cadence.status
```

Validate the project configuration and print its resolved Lead and agent-role settings with:

```sh
herdr plugin action invoke herdr-cadence.validate-config
```

Alias for the actions
```sh
alias cadence-init="herdr plugin action invoke herdr-cadence.init"
alias cadence-start="herdr plugin action invoke herdr-cadence.start"
alias cadence-check="herdr plugin action invoke herdr-cadence.status && herdr plugin action invoke herdr-cadence.validate-config"
```
Cadence is globally installed but only acts in repositories with an enabled config.
It never creates or changes `AGENTS.md`; role and task context is injected when each agent starts.

The Lead can start while the repository has uncommitted changes and may inspect or handle small work directly.
Agent creation remains blocked until the base checkout is committed or stashed so every agent receives a stable baseline; worktree integration also requires a clean base checkout.

## Codex approvals

Codex may request permission when an agent runs the Cadence binary outside its project sandbox.
At the first request, use Codex's option to always allow the suggested command prefix.
To configure it manually, add this rule to `~/.codex/rules/default.rules`:

```python
prefix_rule(
    pattern=["<cadence-bin>", "--state-dir", "<cadence-state-dir>", "--project-root"],
    decision="allow",
    justification="Allow Cadence to coordinate agents",
)
```

Replace `<cadence-bin>` and `<cadence-state-dir>` with the absolute values shown in the permission request, then restart the Codex agents.
The rule applies to Cadence in any enabled project while remaining scoped to that binary and state directory.

## Configuration

```toml
schema_version = 1
enabled = true
yolo = false # full access for the Lead and every agent
agent_default = "generalist"

[lead]
harness = "codex" # or "opencode"
model = "gpt-5.6-terra" # optional; omit to use the harness default
reasoning_effort = "high" # default / low / medium / high / xhigh
max_parallel = 4

[git]
auto_integrate = true
cleanup_on_success = true

[agents.roles.generalist]
description = "Use for general implementation tasks that do not match a specialized role"
harness = "codex" # or "opencode" / "inherit"
model = "gpt-5.6-terra" # optional; omit to use the harness default
reasoning_effort = "medium"
version_control_mode = "git-worktree" # shared-checkout / git-worktree

[agents.roles.planner]
description = "Use for plan mode"
harness = "codex"
model = "gpt-5.6-sol"
reasoning_effort = "xhigh"
version_control_mode = "shared-checkout"

[agents.roles.research]
description = "Use for investigation and evidence gathering"
harness = "codex"
model = "gpt-5.6-sol"
reasoning_effort = "high"
version_control_mode = "shared-checkout"

[agents.roles.qa]
description = "Use for test planning, validation, and regression investigation"
harness = "codex"
model = "gpt-5.6-luna"
reasoning_effort = "medium"
version_control_mode = "shared-checkout"
```

Every agent role is fully defined under `[agents.roles.<name>]`.
The Lead chooses a role from its description and uses top-level `agent_default` when none is a better match.
An agent request can override its role's harness, model, or reasoning effort, but cannot exceed `lead.max_parallel` or overlap another active agent's path scope.

Reasoning effort accepts `default`, `low`, `medium`, `high`, and `xhigh`.
Cadence maps it to Codex's `model_reasoning_effort` and to an OpenCode model variant.
OpenCode reasoning requires an explicit `provider/model`; for example, `model = "openai/gpt-5.2"` with `reasoning_effort = "high"` launches `openai/gpt-5.2#high`.
Variant availability remains model-specific, including `xhigh`.
When `model` is omitted, Cadence lets the selected harness use its default model.
Agents can inherit the Lead's harness, but not its configured model.

Each role owns its checkout behavior.
`version_control_mode = "shared-checkout"` gives the agent direct access to the shared project checkout; it may complete with a report only or commit files in its assigned scope, including a root Markdown artifact named for the agent.
`version_control_mode = "git-worktree"` gives every agent in that role an isolated branch and checkout.
Agent requests cannot override the role's mode.

Isolated Codex agents run autonomously with `workspace-write` sandboxing and no approval prompts.
They can write their worktree and Cadence state; unavailable external operations fail for the agent to report to the Lead.
Top-level `yolo = true` is the single unrestricted override for the Lead and every agent and does not require worktrees.
It passes Codex's dangerous approval-and-sandbox bypass (or OpenCode auto-approval) to every agent.
Worktrees isolate Git changes, not host access.

Agents in `shared-checkout` mode commit only when they change files; report-only work needs no commit.
A `git-worktree` agent must commit clean work, and the Lead reviews its completed report before invoking integration.
After successful integration, `cleanup_on_success = true` terminates the agent and removes its tab or worktree; the Lead and Cadence run remain active for follow-up tasks.
`git.auto_integrate` applies only to shared-checkout agents.
Completed worktrees remain available until review, while failed, cancelled, or conflicting worktrees are retained for inspection; failed cherry-picks are aborted automatically.
The run ends only when the user explicitly asks the Lead to finish the Cadence session.

## Injected context

Cadence sends one compact startup prompt to each agent:

- The Lead receives its run ID, coordination rules, checkout mode, concurrency limit, configured role names and descriptions, and the commands for managing agents and finishing the run.
  It may handle trivial, low-risk work directly and delegates larger or specialized tasks.
- An agent receives only its role and role description, assigned task, allowed path scope, acceptance criteria, checkout-specific Git instructions, and the command for submitting its report.

Cadence does not inject source files, diffs, the full configuration or state store, other agents' tasks or reports, or conversation history.
The selected harness may independently load repository instructions such as `AGENTS.md` and inspect files as it works.
Later Cadence messages are short status notifications to the Lead or explicit follow-up prompts sent to an agent.


## Develop

```sh
./scripts/build-local.sh
herdr plugin link "$PWD"
cargo test
```

Releases publish checksummed macOS and Linux binaries for arm64 and x86-64.
The Herdr installer downloads the matching binary, so users do not need Rust.

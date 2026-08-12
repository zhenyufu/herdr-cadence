# Cadence
Cadence is a light Orchestrating plugin for Herdr that provides one **Lead** and a fleet of **agents**. Talk to the Lead, and it will spin up agents with different roles fully integrated with Herdr tabs and git worktrees.

## Supported harness
| Harness| As Lead | As Agent | 
| Codex | Supported | Supported | 
| Claude | Untested | Supported* |
| Opencode | Untested | Untested | 

* need to manually run and accept once: 
```
claude  --dangerously-skip-permissions

```

## Install and usage

Requires Herdr 0.7.5+, Git, and the agent harness

```sh
herdr plugin install zhenyufu/herdr-cadence
```

Init a cadence config inside the target repository's Herdr workspace:

```sh
herdr plugin action invoke herdr-cadence.init
```

Edit and commit `.cadence.toml`, then start the conversational Lead:

```sh
herdr plugin action invoke herdr-cadence.start
```

View the current status as a notification :

```sh
herdr plugin action invoke herdr-cadence.status
```

Alias for the actions
```sh
alias cadence-init="herdr plugin action invoke herdr-cadence.init"
alias cadence-start="herdr plugin action invoke herdr-cadence.start"
alias cadence-status="herdr plugin action invoke herdr-cadence.status"
```
Cadence is globally installed but only acts in repositories with an enabled config.
It never creates or changes `AGENTS.md`; role and task context is injected when each agent starts.

The Lead can start while the repository has uncommitted changes and may inspect or handle small work directly.
Starting Cadence opens a focused Lead tab in the invoking Herdr workspace; it leaves the current pane alone and uses that workspace as the shared-checkout base for agents.
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

`init` writes `.cadence.toml` from the [canonical initial configuration](src/config.rs).

```toml
schema_version = 2
enabled = true
# Give the Lead and every agent unrestricted host access.
yolo = false
agent_default = "generalist" # default role when no better match

[lead]
harness = "codex"
model = "gpt-5.6-terra"
reasoning_effort = "high"
max_parallel = 4 # Maximum concurrent agents; 1-16

[git]
auto_integrate = true # Applies only to agents using shared-checkout.
cleanup_on_success = true # Remove successful agent tabs or worktrees after integration.

# A role selects ordered runner profiles. The first is primary; later entries are
# launch-time fallbacks for provider availability failures. Roles come first so
# the workflow stays readable; runner profiles may be defined below them.

# [agents.roles.new_role]
# description = "Handles work that matches this role's specialty"
# runners = ["codex-terra-medium"]
# version_control_mode = "shared-checkout" # shared-checkout | git-worktree

[agents.roles.generalist]
description = "Implements general changes that do not require a specialized role"
runners = ["codex-terra-medium"]
version_control_mode = "shared-checkout"

# Common Workflow: planner -> researcher -> developer -> qa
[agents.roles.planner]
description = "Plans complex work and identifies dependencies, risks, and acceptance criteria. Write to implementation-plan.md"
runners = ["codex-sol-high"]
version_control_mode = "shared-checkout"

[agents.roles.researcher]
description = "Investigates questions and gathers evidence before implementation"
runners = ["codex-terra-high"]
version_control_mode = "shared-checkout"

[agents.roles.developer]
description = "Writes code"
runners = ["codex-terra-medium"]
version_control_mode = "git-worktree"

[agents.roles.reviewer]
description = "Reviews code implementation"
runners = ["claude-opus-high", "codex-terra-high"]
version_control_mode = "shared-checkout"

[agents.roles.qa]
description = "Validates behavior, tests changes, and investigates regressions"
runners = ["codex-terra-medium"]
version_control_mode = "shared-checkout"

[agents.runners.codex-terra-medium]
harness = "codex"
model = "gpt-5.6-terra"
reasoning_effort = "medium"

[agents.runners.codex-terra-high]
harness = "codex"
model = "gpt-5.6-terra"
reasoning_effort = "high"

[agents.runners.codex-sol-high]
harness = "codex"
model = "gpt-5.6-sol"
reasoning_effort = "high"

[agents.runners.claude-opus-high]
harness = "claude"
model = "opus"
reasoning_effort = "high"
```

Cadence uses the first runner as the primary and tries later runners only when
the agent cannot launch because its provider is unavailable (for example,
exhausted credits, quota, rate limits, capacity, or authentication). Once an
agent launches, Cadence pins that runner. If it later exits or blocks, Cadence
retains its resources and tells the Lead to decide whether reassignment is safe.
The `harness`, `model`, and `reasoning_effort` fields belong to runners, not
roles; previous single-harness role configuration is unsupported.

## Injected context

Cadence sends one compact startup prompt to each agent:

- The Lead receives its run ID, coordination rules, checkout mode, concurrency limit, configured role names and descriptions, and the commands for managing agents and finishing the run.
  It may handle trivial, low-risk work directly and delegates larger or specialized tasks. It commits each coherent, verified change block locally—whether Lead-authored or agent-integrated—while preserving unrelated edits, and pushes only when asked. For review cycles, it verifies blockers and security/data-integrity findings, consolidates corrections into one developer pass, limits the re-review to changed areas and prior blockers, and handles small findings itself after the second review.
- An agent receives only its role and role description, assigned task, allowed path scope, acceptance criteria, checkout-specific Git instructions, priority labels, and the command for submitting its report.

Leads and agents label findings as `High (Blockers)`, `Mid`, `Low`, or `Wish`.

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

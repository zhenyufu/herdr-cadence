# Cadence Development

Cadence is a Rust Herdr plugin supporting Codex and OpenCode.

## Constraints

- Keep Markdown and injected prompts small.
- Do not generate or modify `AGENTS.md` in enabled projects.
- Runtime roles are injected by Cadence; do not define Orchestrator or Worker roles here.
- Preserve project opt-in through `.herdr/cadence.toml`.
- Keep unrelated Herdr sessions inert.
- Retain failed or conflicted Worker worktrees.

## Verification

Run before finishing:

- `cargo fmt -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --locked`
- `sh -n scripts/*.sh`
- `git diff --check`

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repo() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-b", "main"]);
    git(
        temp.path(),
        &["config", "user.email", "cadence@example.test"],
    );
    git(temp.path(), &["config", "user.name", "Cadence Test"]);
    fs::write(temp.path().join("README.md"), "# Test\n").unwrap();
    git(temp.path(), &["add", "README.md"]);
    git(temp.path(), &["commit", "-m", "initial"]);
    temp
}

fn cadence(root: &Path, state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--project-root",
            root.to_str().unwrap(),
        ])
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn enables_and_reports_project_status() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    let enabled = cadence(repo.path(), state.path(), &["action", "enable-project"]);
    assert!(
        enabled.status.success(),
        "{}",
        String::from_utf8_lossy(&enabled.stderr)
    );
    let config = fs::read_to_string(repo.path().join(".herdr/cadence.toml")).unwrap();
    assert!(config.contains("harness = \"codex\""));
    assert!(!repo.path().join("AGENTS.md").exists());

    let status = cadence(repo.path(), state.path(), &["action", "status"]);
    assert!(status.status.success());
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["enabled"], true);
    assert!(value["active_run"].is_null());
}

#[test]
fn starts_orchestrator_and_spawns_scoped_worker_with_fake_herdr() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    assert!(
        cadence(repo.path(), state.path(), &["action", "enable-project"])
            .status
            .success()
    );
    let config_path = repo.path().join(".herdr/cadence.toml");
    let config = fs::read_to_string(&config_path)
        .unwrap()
        .replace("harness = \"codex\"", "harness = \"opencode\"")
        .replace("harness = \"inherit\"", "harness = \"codex\"");
    fs::write(&config_path, config).unwrap();
    git(repo.path(), &["add", ".herdr/cadence.toml"]);
    git(repo.path(), &["commit", "-m", "enable cadence"]);

    let fake_dir = tempfile::tempdir().unwrap();
    let fake = fake_dir.path().join("herdr");
    let log = fake_dir.path().join("calls.log");
    let worker_path = fake_dir.path().join("worker");
    fs::create_dir(&worker_path).unwrap();
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$*" >> '{}'
if [ "$1 $2" = "agent get" ]; then exit 1; fi
if [ "$1 $2" = "tab create" ]; then
  printf '%s\n' '{{"id":"test","result":{{"tab":{{"tab_id":"tab-1"}},"root_pane":{{"pane_id":"pane-orch"}}}}}}'
elif [ "$1 $2" = "worktree create" ]; then
  printf '%s\n' '{{"id":"test","result":{{"workspace":{{"workspace_id":"worker-ws"}},"tab":{{"tab_id":"worker-tab"}},"root_pane":{{"pane_id":"pane-worker"}},"worktree":{{"path":"{}"}}}}}}'
elif [ "$1 $2" = "worktree remove" ]; then
  git -C '{}' worktree remove --force '{}'
  printf '%s\n' '{{"id":"test","result":{{}}}}'
else
  printf '%s\n' '{{"id":"test","result":{{}}}}'
fi
"#,
        log.display(),
        worker_path.display(),
        repo.path().display(),
        worker_path.display()
    );
    fs::write(&fake, script).unwrap();
    let mut permissions = fs::metadata(&fake).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake, permissions).unwrap();

    let start = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "action",
            "start",
        ])
        .env("HERDR_BIN_PATH", &fake)
        .env("HERDR_WORKSPACE_ID", "base-ws")
        .output()
        .unwrap();
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );

    let request = fake_dir.path().join("request.json");
    fs::write(
        &request,
        r#"{"title":"Add API","task":"Implement the API","scope":["src/api"],"acceptance":["Tests pass"]}"#,
    )
    .unwrap();
    let spawn = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "worker",
            "spawn",
            "--request-file",
            request.to_str().unwrap(),
        ])
        .env("HERDR_BIN_PATH", &fake)
        .output()
        .unwrap();
    assert!(
        spawn.status.success(),
        "{}",
        String::from_utf8_lossy(&spawn.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&spawn.stdout).unwrap();
    assert_eq!(value["worker_id"], "worker-1");
    assert_eq!(value["workspace_id"], "worker-ws");

    let calls = fs::read_to_string(log).unwrap();
    assert!(calls.contains("tab create --workspace base-ws"));
    assert!(calls.contains("agent start cadence-orch-"));
    assert!(calls.contains("--kind opencode"));
    assert!(calls.contains("worktree create --workspace base-ws"));
    assert!(calls.contains("agent start cadence-"));
    assert!(calls.contains("--kind codex"));
    assert!(calls.contains("agent prompt"));

    fs::remove_dir(&worker_path).unwrap();
    let branch = value["branch"].as_str().unwrap();
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-b",
            branch,
            worker_path.to_str().unwrap(),
            "HEAD",
        ],
    );
    fs::create_dir_all(worker_path.join("src/api")).unwrap();
    fs::write(worker_path.join("src/api/mod.rs"), "pub fn ready() {}\n").unwrap();
    git(&worker_path, &["add", "src/api/mod.rs"]);
    git(&worker_path, &["commit", "-m", "add api"]);
    let report = fake_dir.path().join("report.json");
    fs::write(
        &report,
        r#"{"status":"completed","summary":"Added API","tests":["cargo test"],"changed_paths":[],"blockers":[]}"#,
    )
    .unwrap();
    let complete = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "worker",
            "complete",
            "worker-1",
            "--report-file",
            report.to_str().unwrap(),
        ])
        .env("HERDR_BIN_PATH", &fake)
        .output()
        .unwrap();
    assert!(
        complete.status.success(),
        "{}",
        String::from_utf8_lossy(&complete.stderr)
    );
    let completed: serde_json::Value = serde_json::from_slice(&complete.stdout).unwrap();
    assert_eq!(completed["status"], "integrated");
    assert_eq!(completed["report"]["changed_paths"][0], "src/api/mod.rs");
    assert_eq!(
        fs::read_to_string(repo.path().join("src/api/mod.rs")).unwrap(),
        "pub fn ready() {}\n"
    );
}

#[test]
fn ignores_events_from_unrelated_non_git_workspaces() {
    let workspace = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            workspace.path().to_str().unwrap(),
            "event",
        ])
        .env("HERDR_PLUGIN_EVENT", "pane.agent_status_changed")
        .env(
            "HERDR_PLUGIN_EVENT_JSON",
            r#"{"event":"pane.agent_status_changed","data":{"pane_id":"unrelated","agent_status":"working"}}"#,
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ignored"], true);
}

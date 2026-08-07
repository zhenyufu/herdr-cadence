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

fn git_stdout(root: &Path, args: &[&str]) -> String {
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
    String::from_utf8(output.stdout).unwrap().trim().to_string()
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
    assert!(config.contains("generalist_description ="));
    assert!(config.contains("# [workers.roles.research]"));
    let parsed: herdr_cadence::config::Config = toml::from_str(&config).unwrap();
    assert_eq!(parsed.orchestrator.max_parallel, Some(4));
    assert_eq!(parsed.workers.max_parallel, None);
    assert!(!parsed.git.use_worktrees);
    assert!(!repo.path().join("AGENTS.md").exists());

    let status = cadence(repo.path(), state.path(), &["action", "status"]);
    assert!(status.status.success());
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["enabled"], true);
    assert!(value["active_run"].is_null());
}

#[test]
fn runs_worker_in_shared_checkout_by_default() {
    run_worker_flow(false);
}

#[test]
fn runs_worker_in_configured_worktree() {
    run_worker_flow(true);
}

fn run_worker_flow(use_worktrees: bool) {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    assert!(
        cadence(repo.path(), state.path(), &["action", "enable-project"])
            .status
            .success()
    );
    let config_path = repo.path().join(".herdr/cadence.toml");
    let mut config = fs::read_to_string(&config_path)
        .unwrap()
        .replacen(
            "harness = \"codex\"\n# model = \"your-model-id\"",
            "harness = \"opencode\"\nmodel = \"orchestrator-model\"",
            1,
        )
        .replacen(
            "harness = \"inherit\"\n# model = \"your-model-id\"",
            "harness = \"codex\"\nmodel = \"worker-model\"",
            1,
        );
    if use_worktrees {
        config = config.replace("use_worktrees = false", "use_worktrees = true");
    }
    config.push_str(
        "\n[workers.roles.qa]\ndescription = \"Use for test validation\"\nharness = \"opencode\"\nmodel = \"qa-model\"\n",
    );
    toml::from_str::<herdr_cadence::config::Config>(&config).unwrap();
    fs::write(&config_path, config).unwrap();
    git(repo.path(), &["add", ".herdr/cadence.toml"]);
    git(repo.path(), &["commit", "-m", "enable cadence"]);

    let fake_dir = tempfile::tempdir().unwrap();
    let fake = fake_dir.path().join("herdr");
    let log = fake_dir.path().join("calls.log");
    let busy_once = fake_dir.path().join("busy-once");
    let shell_ready_once = fake_dir.path().join("shell-ready-once");
    let worker_path = fake_dir.path().join("worker");
    fs::create_dir(&worker_path).unwrap();
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$*" >> '{}'
if [ "$1 $2" = "agent get" ]; then exit 1; fi
if [ "$1 $2" = "pane process-info" ]; then
  if [ ! -e '{}' ]; then
    : > '{}'
    printf '%s\n' '{{"id":"test","result":{{"type":"pane_process_info","process_info":{{"pane_id":"test","shell_pid":123,"foreground_process_group_id":456}}}}}}'
  else
    printf '%s\n' '{{"id":"test","result":{{"type":"pane_process_info","process_info":{{"pane_id":"test","shell_pid":123,"foreground_process_group_id":123}}}}}}'
  fi
  exit 0
fi
case "$*" in
  "agent start cadence-orch-"*)
    if [ ! -e '{}' ]; then
      : > '{}'
      printf '%s\n' '{{"error":{{"code":"agent_pane_busy","message":"pane is not ready"}}}}' >&2
      exit 1
    fi
    ;;
esac
if [ "$1 $2" = "workspace create" ]; then
  printf '%s\n' '{{"id":"test","result":{{"workspace":{{"workspace_id":"orch-ws"}},"tab":{{"tab_id":"tab-orch"}},"root_pane":{{"pane_id":"pane-orch"}}}}}}'
elif [ "$1 $2" = "tab create" ]; then
  case "$*" in
    *"(Cadence)"*) tab_id="tab-orch"; pane_id="pane-orch" ;;
    *) tab_id="tab-worker"; pane_id="pane-worker" ;;
  esac
  printf '%s\n' "{{\"id\":\"test\",\"result\":{{\"tab\":{{\"tab_id\":\"$tab_id\"}},\"root_pane\":{{\"pane_id\":\"$pane_id\"}}}}}}"
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
        shell_ready_once.display(),
        shell_ready_once.display(),
        busy_once.display(),
        busy_once.display(),
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
    let store: serde_json::Value =
        serde_json::from_slice(&fs::read(state.path().join("state.json")).unwrap()).unwrap();
    let project = store["projects"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    let active_run = project["active_run"].as_str().unwrap();
    let run = &project["runs"][active_run];
    assert_eq!(run["base_workspace_id"], "orch-ws");
    assert_eq!(run["orchestrator"]["workspace_id"], "orch-ws");
    if use_worktrees {
        let resumed = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
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
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
    }

    let request = fake_dir.path().join("request.json");
    fs::write(
        &request,
        r#"{"title":"Add API","task":"Implement the API","scope":["src/api"],"acceptance":["Tests pass"],"role":"qa"}"#,
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
    assert_eq!(value["display_name"], "[QA] Add API");
    assert_eq!(value["role"], "qa");
    if use_worktrees {
        assert_eq!(value["workspace_id"], "worker-ws");
    } else {
        assert!(value["workspace_id"].is_null());
    }

    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains(&format!(
        "workspace create --cwd {} --label {} (Cadence)",
        repo.path().canonicalize().unwrap().display(),
        repo.path().file_name().unwrap().to_string_lossy()
    )));
    assert_eq!(calls.matches("workspace create --cwd").count(), 1);
    assert!(calls.contains("tab rename tab-orch"));
    assert!(calls.contains("(Cadence)"));
    assert_eq!(
        calls.matches("pane process-info --pane pane-orch").count(),
        if use_worktrees { 3 } else { 2 }
    );
    assert!(calls.contains("agent start cadence-orch-"));
    assert_eq!(
        calls.matches("agent start cadence-orch-").count(),
        if use_worktrees { 3 } else { 2 }
    );
    assert!(calls.contains("--kind opencode"));
    assert!(calls.contains("- qa: Use for test validation"));
    assert!(calls.contains("Use `generalist` when no specialized role is a good match"));
    assert!(calls.contains("No user task is assigned yet"));
    assert!(calls.contains("reply briefly that Cadence is ready, then wait"));
    assert!(calls.contains("use each spawn result's `display_name`"));
    assert!(calls.contains("do not call agents Worker 2 or worker-2"));
    assert!(calls.contains(
        "--kind opencode --pane pane-orch --timeout 120000 -- --model orchestrator-model"
    ));
    if use_worktrees {
        assert!(calls.contains("workspace get orch-ws"));
        assert!(calls.contains("tab create --workspace orch-ws"));
        assert!(calls.contains("(Cadence) --focus"));
        assert!(calls.contains(&format!(
            "worktree create --cwd {}",
            repo.path().canonicalize().unwrap().display()
        )));
        assert!(!calls.contains("worktree create --workspace"));
        assert!(calls.contains("Workers run in isolated Herdr worktrees"));
    } else {
        assert!(!calls.contains("worktree create --cwd"));
        assert!(calls.contains("tab create --workspace orch-ws"));
        assert!(calls.contains("--label [QA] Add API --no-focus"));
        assert!(calls.contains("share the base checkout"));
        assert!(calls.contains("create exactly one commit for this task"));
    }
    assert!(calls.contains("agent start cadence-"));
    assert!(
        calls.contains("--kind opencode --pane pane-worker --timeout 120000 -- --model qa-model")
    );
    assert!(calls.contains("Role: qa"));
    assert!(calls.contains("Role guidance: Use for test validation"));
    assert!(calls.contains("agent prompt"));

    let checkout = if use_worktrees {
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
        worker_path.clone()
    } else {
        assert_eq!(value["branch"], "main");
        repo.path().to_path_buf()
    };
    fs::create_dir_all(checkout.join("src/api")).unwrap();
    fs::write(checkout.join("src/api/mod.rs"), "pub fn ready() {}\n").unwrap();
    git(&checkout, &["add", "src/api/mod.rs"]);
    git(&checkout, &["commit", "-m", "add api"]);
    let commit_sha = git_stdout(&checkout, &["rev-parse", "HEAD"]);
    let report = fake_dir.path().join("report.json");
    fs::write(
        &report,
        format!(
            r#"{{"status":"completed","summary":"Added API","tests":["cargo test"],"changed_paths":[],"blockers":[],"commit_sha":"{commit_sha}"}}"#
        ),
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
    let calls = fs::read_to_string(&log).unwrap();
    if use_worktrees {
        assert!(calls.contains("worktree remove --workspace worker-ws"));
    } else {
        assert!(calls.contains("tab close tab-worker"));
    }
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

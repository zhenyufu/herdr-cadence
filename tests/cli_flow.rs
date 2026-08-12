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
    let enabled = cadence(repo.path(), state.path(), &["action", "init"]);
    assert!(
        enabled.status.success(),
        "{}",
        String::from_utf8_lossy(&enabled.stderr)
    );
    let config = fs::read_to_string(repo.path().join(".cadence.toml")).unwrap();
    assert!(config.contains("harness = \"codex\""));
    assert!(config.contains("model = \"gpt-5.6-terra\""));
    assert!(config.contains("reasoning_effort = \"high\""));
    assert!(config.contains("version_control_mode = \"git-worktree\""));
    assert!(config.contains("version_control_mode = \"shared-checkout\""));
    assert!(config.contains("agent_default = \"generalist\""));
    assert!(config.contains("[agents.roles.generalist]"));
    assert!(config.contains("description = \"Implements general changes"));
    assert!(config.contains("[agents.roles.planner]"));
    assert!(!config.contains("[agents.roles.planning]"));
    assert!(config.contains("[agents.roles.researcher]"));
    assert!(config.contains("[agents.roles.developer]"));
    assert!(config.contains("[agents.roles.qa]"));
    assert!(config.find("[git]").unwrap() < config.find("[agents.roles.").unwrap());
    let parsed: herdr_cadence::config::Config = toml::from_str(&config).unwrap();
    assert!(
        parsed
            .agents
            .roles
            .values()
            .all(|role| role.harness != herdr_cadence::config::AgentHarness::Inherit)
    );
    assert_eq!(parsed.lead.max_parallel, Some(4));
    assert_eq!(parsed.lead.model.as_deref(), Some("gpt-5.6-terra"));
    assert_eq!(parsed.agent_default, "generalist");
    let generalist = parsed.agents.roles.get("generalist").unwrap();
    assert_eq!(generalist.model.as_deref(), Some("gpt-5.6-terra"));
    assert_eq!(
        generalist.reasoning_effort,
        herdr_cadence::config::ReasoningEffort::Medium
    );
    assert!(!parsed.yolo);
    assert_eq!(
        generalist.version_control_mode,
        herdr_cadence::config::VersionControlMode::SharedCheckout
    );
    assert!(!repo.path().join(".herdr").exists());
    assert!(!repo.path().join("AGENTS.md").exists());

    let status = cadence(repo.path(), state.path(), &["action", "status"]);
    assert!(status.status.success());
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["enabled"], true);
    assert_eq!(value["config_valid"], true);
    assert_eq!(value["checkout_clean"], false);
    assert!(value["active_run"].is_null());
}

#[test]
fn reports_config_parse_error_causes() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    assert!(
        cadence(repo.path(), state.path(), &["action", "init"])
            .status
            .success()
    );
    let config_path = repo.path().join(".cadence.toml");
    let config = fs::read_to_string(&config_path).unwrap().replacen(
        "harness = \"codex\"",
        "harness = \"invalid\"",
        1,
    );
    fs::write(config_path, config).unwrap();

    let status = cadence(repo.path(), state.path(), &["action", "status"]);

    assert!(status.status.success());
    let status_value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status_value["config_valid"], false);
    assert!(status_value["enabled"].is_null());

    let validation = cadence(repo.path(), state.path(), &["action", "validate-config"]);
    assert!(!validation.status.success());
    let error: serde_json::Value = serde_json::from_slice(&validation.stderr).unwrap();
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("invalid Cadence config")
    );
    assert!(error["causes"].as_array().unwrap().iter().any(|cause| {
        cause
            .as_str()
            .unwrap()
            .contains("unknown variant `invalid`")
    }));
}

#[test]
fn validates_and_resolves_project_config() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    assert!(
        cadence(repo.path(), state.path(), &["action", "init"])
            .status
            .success()
    );

    let validation = cadence(repo.path(), state.path(), &["action", "validate-config"]);

    assert!(
        validation.status.success(),
        "{}",
        String::from_utf8_lossy(&validation.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&validation.stdout).unwrap();
    assert_eq!(value["valid"], true);
    assert_eq!(value["enabled"], true);
    assert_eq!(value["agent_default"], "generalist");
    assert_eq!(value["lead"]["harness"], "codex");
    assert_eq!(value["lead"]["model"], "gpt-5.6-terra");
    assert_eq!(value["lead"]["reasoning_effort"], "high");
    assert_eq!(value["lead"]["max_parallel"], 4);
    let roles = value["roles"].as_array().unwrap();
    assert!(roles.iter().any(|role| role["name"] == "generalist"));
    assert!(roles.iter().any(|role| role["name"] == "qa"));
    assert!(roles.iter().any(|role| role["name"] == "researcher"));
    let planner = roles.iter().find(|role| role["name"] == "planner").unwrap();
    assert_eq!(planner["model"], "gpt-5.6-sol");
    assert_eq!(planner["reasoning_effort"], "high");
    assert_eq!(planner["version_control_mode"], "shared-checkout");
    assert!(roles.iter().any(|role| {
        role["name"] == "researcher" && role["version_control_mode"] == "shared-checkout"
    }));
    assert!(
        roles.iter().any(|role| {
            role["name"] == "qa" && role["version_control_mode"] == "shared-checkout"
        })
    );
}

#[test]
fn rejects_agent_checkout_overrides() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    assert!(
        cadence(repo.path(), state.path(), &["action", "init"])
            .status
            .success()
    );
    git(repo.path(), &["add", ".cadence.toml"]);
    git(repo.path(), &["commit", "-m", "enable cadence"]);
    let request = state.path().join("request.json");
    fs::write(
        &request,
        r#"{"title":"Review API","task":"Review the API","scope":["src/api"],"acceptance":["Review complete"],"use_git_worktree":true}"#,
    )
    .unwrap();

    let spawned = cadence(
        repo.path(),
        state.path(),
        &[
            "agent",
            "spawn",
            "--request-file",
            request.to_str().unwrap(),
        ],
    );

    assert!(!spawned.status.success());
    let error: serde_json::Value = serde_json::from_slice(&spawned.stderr).unwrap();
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("use_git_worktree")
    );
}

#[test]
fn runs_agent_in_shared_checkout_by_default() {
    run_agent_flow(false, false, false, false);
}

#[test]
fn runs_agent_in_configured_worktree() {
    run_agent_flow(true, false, false, false);
}

#[test]
fn runs_every_agent_in_global_yolo() {
    run_agent_flow(false, true, false, false);
}

#[test]
fn starts_dirty_but_blocks_agents_until_clean() {
    run_agent_flow(true, false, true, false);
}

#[test]
fn rejects_an_earlier_out_of_scope_shared_checkout_commit() {
    run_agent_flow(false, false, false, true);
}

fn run_agent_flow(
    use_worktree: bool,
    global_yolo: bool,
    dirty_at_start: bool,
    create_out_of_scope_commit: bool,
) {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    assert!(
        cadence(repo.path(), state.path(), &["action", "init"])
            .status
            .success()
    );
    let config_path = repo.path().join(".cadence.toml");
    let mut config: herdr_cadence::config::Config =
        toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    config.lead.harness = herdr_cadence::config::Harness::Opencode;
    config.lead.model = Some("openai/lead-model".into());
    let qa = config.agents.roles.get_mut("qa").unwrap();
    qa.description = "Validates test behavior".into();
    qa.model = Some("qa-model".into());
    qa.reasoning_effort = herdr_cadence::config::ReasoningEffort::Low;
    if use_worktree {
        qa.version_control_mode = herdr_cadence::config::VersionControlMode::GitWorktree;
    }
    if global_yolo {
        config.yolo = true;
    }
    config.validate().unwrap();
    fs::write(&config_path, toml::to_string_pretty(&config).unwrap()).unwrap();
    git(repo.path(), &["add", ".cadence.toml"]);
    git(repo.path(), &["commit", "-m", "enable cadence"]);
    let dirty_path = repo.path().join("uncommitted.txt");
    if dirty_at_start {
        fs::write(&dirty_path, "uncommitted work\n").unwrap();
    }

    let fake_dir = tempfile::tempdir().unwrap();
    let fake = fake_dir.path().join("herdr");
    let log = fake_dir.path().join("calls.log");
    let busy_once = fake_dir.path().join("busy-once");
    let shell_ready_once = fake_dir.path().join("shell-ready-once");
    let lead_started = fake_dir.path().join("lead-started");
    let agent_path = fake_dir.path().join("agent");
    fs::create_dir(&agent_path).unwrap();
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$*" >> '{}'
if [ "$1 $2" = "agent get" ]; then
  case "$3" in
    cadence-lead-*)
      if [ -e '{}' ]; then
        printf '%s\n' '{{"id":"test","result":{{}}}}'
        exit 0
      fi
      ;;
    cadence-??????-a*)
      printf '%s\n' '{{"id":"test","result":{{}}}}'
      exit 0
      ;;
  esac
  exit 1
fi
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
  "agent start cadence-lead-"*)
    if [ ! -e '{}' ]; then
      : > '{}'
      printf '%s\n' '{{"error":{{"code":"agent_pane_busy","message":"pane is not ready"}}}}' >&2
      exit 1
    fi
    ;;
esac
case "$*" in
  "agent start cadence-lead-"*) : > '{}' ;;
esac
if [ "$1 $2" = "workspace create" ]; then
  printf '%s\n' '{{"id":"test","result":{{"workspace":{{"workspace_id":"lead-ws"}},"tab":{{"tab_id":"tab-lead"}},"root_pane":{{"pane_id":"pane-lead"}}}}}}'
elif [ "$1 $2" = "tab create" ]; then
  case "$*" in
    *"[Lead]"*) tab_id="tab-lead"; pane_id="pane-lead" ;;
    *) tab_id="tab-agent"; pane_id="pane-agent" ;;
  esac
  printf '%s\n' "{{\"id\":\"test\",\"result\":{{\"tab\":{{\"tab_id\":\"$tab_id\"}},\"root_pane\":{{\"pane_id\":\"$pane_id\"}}}}}}"
elif [ "$1 $2" = "worktree create" ]; then
  printf '%s\n' '{{"id":"test","result":{{"workspace":{{"workspace_id":"agent-ws"}},"tab":{{"tab_id":"agent-tab"}},"root_pane":{{"pane_id":"pane-agent"}},"worktree":{{"path":"{}"}}}}}}'
elif [ "$1 $2" = "worktree remove" ]; then
  if [ "${{CADENCE_TEST_FAIL_WORKTREE_REMOVE:-}}" = "1" ]; then
    printf '%s\n' 'forced worktree cleanup failure' >&2
    exit 1
  fi
  git -C '{}' worktree remove --force '{}'
  printf '%s\n' '{{"id":"test","result":{{}}}}'
else
  printf '%s\n' '{{"id":"test","result":{{}}}}'
fi
"#,
        log.display(),
        lead_started.display(),
        shell_ready_once.display(),
        shell_ready_once.display(),
        busy_once.display(),
        busy_once.display(),
        lead_started.display(),
        agent_path.display(),
        repo.path().display(),
        agent_path.display()
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
    let started: serde_json::Value = serde_json::from_slice(&start.stdout).unwrap();
    assert_eq!(started["checkout_clean"], !dirty_at_start);
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
    assert_eq!(run["base_workspace_id"], "base-ws");
    assert_eq!(run["lead"]["workspace_id"], "base-ws");
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
    let resumed: serde_json::Value = serde_json::from_slice(&resumed.stdout).unwrap();
    assert_eq!(resumed["status"], "focused");

    fs::remove_file(&lead_started).unwrap();
    let restarted = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "action",
            "start",
        ])
        .env("HERDR_BIN_PATH", &fake)
        .env("HERDR_WORKSPACE_ID", "resumed-ws")
        .output()
        .unwrap();
    assert!(
        restarted.status.success(),
        "{}",
        String::from_utf8_lossy(&restarted.stderr)
    );
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains("tab create --workspace resumed-ws"));
    let store: serde_json::Value =
        serde_json::from_slice(&fs::read(state.path().join("state.json")).unwrap()).unwrap();
    let project = store["projects"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    let active_run = project["active_run"].as_str().unwrap();
    assert_eq!(
        project["runs"][active_run]["base_workspace_id"],
        "resumed-ws"
    );

    let request = fake_dir.path().join("request.json");
    fs::write(
        &request,
        r#"{"title":"Add API","task":"Implement the API","scope":["src/api"],"acceptance":["Tests pass"],"role":"qa"}"#,
    )
    .unwrap();
    if dirty_at_start {
        let blocked = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
            .args([
                "--state-dir",
                state.path().to_str().unwrap(),
                "--project-root",
                repo.path().to_str().unwrap(),
                "agent",
                "spawn",
                "--request-file",
                request.to_str().unwrap(),
            ])
            .env("HERDR_BIN_PATH", &fake)
            .output()
            .unwrap();
        assert!(!blocked.status.success());
        let error: serde_json::Value = serde_json::from_slice(&blocked.stderr).unwrap();
        assert!(
            error["error"]
                .as_str()
                .unwrap()
                .contains("cannot spawn an agent")
        );
        assert!(
            error["causes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|cause| { cause.as_str().unwrap().contains("Git worktree is dirty") })
        );
        fs::remove_file(&dirty_path).unwrap();
    }
    let spawn = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "agent",
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
    assert_eq!(value["agent_id"], "agent-1");
    assert_eq!(value["display_name"], "[QA] Add API");
    assert_eq!(value["role"], "qa");
    assert_eq!(value["model"], "qa-model");
    assert_eq!(value["reasoning_effort"], "low");
    if use_worktree {
        assert_eq!(value["workspace_id"], "agent-ws");
    } else {
        assert!(value["workspace_id"].is_null());
    }

    let calls = fs::read_to_string(&log).unwrap();
    assert!(!calls.contains("workspace create --cwd"));
    assert!(!calls.contains("tab rename tab-lead"));
    assert!(calls.contains("[Lead]"));
    assert_eq!(
        calls.matches("pane process-info --pane pane-lead").count(),
        3
    );
    assert!(calls.contains("agent start cadence-lead-"));
    assert_eq!(calls.matches("agent start cadence-lead-").count(), 3);
    assert!(calls.contains("--kind opencode"));
    assert!(calls.contains(&format!(
        "- qa [{}]: Validates test behavior",
        if use_worktree {
            "git-worktree"
        } else {
            "shared-checkout"
        }
    )));
    assert!(calls.contains(
        "Use the configured default role `generalist` when no specialized role is a good match"
    ));
    assert!(calls.contains("No user task is assigned yet"));
    assert!(calls.contains("reply briefly that Cadence is ready, then wait"));
    assert!(calls.contains("Handle trivial, low-risk work directly"));
    assert!(calls.contains("Never directly edit a path reserved by an active agent"));
    assert!(calls.contains("When an agent comes back with work, make a local commit"));
    assert_eq!(
        calls.contains("The base checkout has uncommitted changes"),
        dirty_at_start
    );
    assert!(calls.contains("Completing a task or batch does not end the Cadence run"));
    assert!(calls.contains("only after the user explicitly asks to end the Cadence session"));
    assert!(calls.contains("use each spawn result's `display_name`"));
    assert!(calls.contains("do not call agents Agent 2 or agent-2"));
    let lead_launch =
        "--kind opencode --pane pane-lead --timeout 120000 -- --model openai/lead-model#high";
    assert!(calls.contains(lead_launch));
    if global_yolo {
        assert!(calls.contains(&format!("{lead_launch} --auto")));
        assert!(calls.contains("Global YOLO is enabled for you and every agent"));
    } else {
        assert!(!calls.contains(&format!("{lead_launch} --auto")));
    }
    assert!(calls.contains("Each role's configured version_control_mode fixes its checkout"));
    if use_worktree {
        assert_eq!(calls.matches("tab create --workspace base-ws").count(), 1);
        assert!(calls.contains(&format!(
            "--label [Lead] {} --focus",
            repo.path().file_name().unwrap().to_string_lossy()
        )));
        assert!(calls.contains(&format!(
            "worktree create --cwd {}",
            repo.path().canonicalize().unwrap().display()
        )));
        assert!(!calls.contains("worktree create --workspace"));
        assert!(calls.contains("this isolated worktree"));
    } else {
        assert!(!calls.contains("worktree create --cwd"));
        assert_eq!(calls.matches("tab create --workspace base-ws").count(), 1);
        assert_eq!(
            calls.matches("tab create --workspace resumed-ws").count(),
            2
        );
        assert!(calls.contains("--label [QA] Add API --no-focus"));
        assert!(calls.contains("directly share the project checkout"));
        assert!(calls.contains("create exactly one commit for changed files"));
    }
    assert!(calls.contains("agent start cadence-"));
    let agent_launch = "--kind codex --pane pane-agent --timeout 120000 -- --model qa-model --config model_reasoning_effort=\"low\"";
    assert!(calls.contains(agent_launch));
    if global_yolo {
        assert!(calls.contains(&format!(
            "{agent_launch} --dangerously-bypass-approvals-and-sandbox"
        )));
    } else if use_worktree {
        assert!(calls.contains(&format!(
            "{agent_launch} --sandbox workspace-write --ask-for-approval never --add-dir {}",
            state.path().display()
        )));
        assert!(calls.contains("If an action outside the available sandbox is required"));
    } else {
        assert!(calls.contains(&format!(
            "{agent_launch} --add-dir {}",
            state.path().display()
        )));
        assert!(!calls.contains("--ask-for-approval never"));
        assert!(!calls.contains("--dangerously-bypass-approvals-and-sandbox"));
    }
    assert!(calls.contains("Role: qa"));
    assert!(calls.contains("Role guidance: Validates test behavior"));
    assert!(calls.contains("agent prompt"));

    let early_follow_up = if !use_worktree && !create_out_of_scope_commit {
        fs::write(
            &request,
            r#"{"title":"Update docs","task":"Update the docs","scope":["docs"],"acceptance":["Docs are current"],"role":"researcher"}"#,
        )
        .unwrap();
        let follow_up = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
            .args([
                "--state-dir",
                state.path().to_str().unwrap(),
                "--project-root",
                repo.path().to_str().unwrap(),
                "agent",
                "spawn",
                "--request-file",
                request.to_str().unwrap(),
            ])
            .env("HERDR_BIN_PATH", &fake)
            .output()
            .unwrap();
        assert!(
            follow_up.status.success(),
            "{}",
            String::from_utf8_lossy(&follow_up.stderr)
        );
        Some(serde_json::from_slice(&follow_up.stdout).unwrap())
    } else {
        None
    };

    let checkout = if use_worktree {
        fs::remove_dir(&agent_path).unwrap();
        let branch = value["branch"].as_str().unwrap();
        git(
            repo.path(),
            &[
                "worktree",
                "add",
                "-b",
                branch,
                agent_path.to_str().unwrap(),
                "HEAD",
            ],
        );
        agent_path.clone()
    } else {
        assert_eq!(value["branch"], "main");
        repo.path().to_path_buf()
    };
    if create_out_of_scope_commit {
        fs::write(checkout.join("outside.txt"), "outside scope\n").unwrap();
        git(&checkout, &["add", "outside.txt"]);
        git(&checkout, &["commit", "-m", "change outside scope"]);
    }
    fs::create_dir_all(checkout.join("src/api")).unwrap();
    if !use_worktree && !create_out_of_scope_commit {
        fs::write(checkout.join("src/api/types.rs"), "pub struct Api;\n").unwrap();
        git(&checkout, &["add", "src/api/types.rs"]);
        git(&checkout, &["commit", "-m", "add api types"]);
    }
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
            "agent",
            "complete",
            "agent-1",
            "--report-file",
            report.to_str().unwrap(),
        ])
        .env("HERDR_BIN_PATH", &fake)
        .output()
        .unwrap();
    if create_out_of_scope_commit {
        assert!(!complete.status.success());
        let error = String::from_utf8_lossy(&complete.stderr);
        assert!(error.contains("Unattributed commit"), "{error}");
        assert!(error.contains("outside.txt"), "{error}");
        return;
    }
    assert!(
        complete.status.success(),
        "{}",
        String::from_utf8_lossy(&complete.stderr)
    );
    let mut completed: serde_json::Value = serde_json::from_slice(&complete.stdout).unwrap();
    assert_eq!(
        completed["status"],
        if use_worktree {
            "completed"
        } else {
            "integrated"
        }
    );
    assert_eq!(completed["report"]["changed_paths"][0], "src/api/mod.rs");
    if !use_worktree {
        assert_eq!(completed["claimed_commits"].as_array().unwrap().len(), 2);
        assert_eq!(completed["report"]["changed_paths"][1], "src/api/types.rs");
    }
    if use_worktree {
        assert!(!repo.path().join("src/api/mod.rs").exists());
        fs::write(
            &request,
            r#"{"title":"Overlap API","task":"Change the API again","scope":["src/api"],"acceptance":["Tests pass"],"role":"qa"}"#,
        )
        .unwrap();
        let overlapping_spawn = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
            .args([
                "--state-dir",
                state.path().to_str().unwrap(),
                "--project-root",
                repo.path().to_str().unwrap(),
                "agent",
                "spawn",
                "--request-file",
                request.to_str().unwrap(),
            ])
            .env("HERDR_BIN_PATH", &fake)
            .output()
            .unwrap();
        assert!(!overlapping_spawn.status.success());
        assert!(
            String::from_utf8_lossy(&overlapping_spawn.stderr)
                .contains("scope overlaps active agent agent-1")
        );
        let integrate = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
            .args([
                "--state-dir",
                state.path().to_str().unwrap(),
                "--project-root",
                repo.path().to_str().unwrap(),
                "agent",
                "integrate",
                "agent-1",
            ])
            .env("HERDR_BIN_PATH", &fake)
            .output()
            .unwrap();
        assert!(
            integrate.status.success(),
            "{}",
            String::from_utf8_lossy(&integrate.stderr)
        );
        completed = serde_json::from_slice(&integrate.stdout).unwrap();
        assert_eq!(completed["status"], "integrated");
    }
    assert_eq!(
        fs::read_to_string(repo.path().join("src/api/mod.rs")).unwrap(),
        "pub fn ready() {}\n"
    );
    let status = cadence(repo.path(), state.path(), &["action", "status"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["active_run"]["id"], active_run);

    let follow_up: serde_json::Value = if let Some(follow_up) = early_follow_up {
        follow_up
    } else {
        fs::write(
            &request,
            r#"{"title":"Update docs","task":"Update the docs","scope":["docs"],"acceptance":["Docs are current"],"role":"researcher"}"#,
        )
        .unwrap();
        let follow_up = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
            .args([
                "--state-dir",
                state.path().to_str().unwrap(),
                "--project-root",
                repo.path().to_str().unwrap(),
                "agent",
                "spawn",
                "--request-file",
                request.to_str().unwrap(),
            ])
            .env("HERDR_BIN_PATH", &fake)
            .output()
            .unwrap();
        assert!(
            follow_up.status.success(),
            "{}",
            String::from_utf8_lossy(&follow_up.stderr)
        );
        serde_json::from_slice(&follow_up.stdout).unwrap()
    };
    assert_eq!(follow_up["agent_id"], "agent-2");
    assert_eq!(follow_up["branch"], "main");
    assert!(follow_up["workspace_id"].is_null());

    fs::create_dir_all(repo.path().join("docs")).unwrap();
    fs::write(repo.path().join("docs/readme.md"), "Current docs\n").unwrap();
    git(repo.path(), &["add", "docs/readme.md"]);
    git(repo.path(), &["commit", "-m", "update docs"]);
    let research_commit = git_stdout(repo.path(), &["rev-parse", "HEAD"]);
    fs::write(
        &report,
        format!(
            r#"{{"status":"completed","summary":"Research complete","tests":[],"changed_paths":[],"blockers":[],"commit_sha":"{research_commit}"}}"#
        ),
    )
    .unwrap();
    let research_complete = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "agent",
            "complete",
            "agent-2",
            "--report-file",
            report.to_str().unwrap(),
        ])
        .env("HERDR_BIN_PATH", &fake)
        .output()
        .unwrap();
    assert!(
        research_complete.status.success(),
        "{}",
        String::from_utf8_lossy(&research_complete.stderr)
    );
    let research_complete: serde_json::Value =
        serde_json::from_slice(&research_complete.stdout).unwrap();
    assert_eq!(research_complete["status"], "integrated");
    assert_eq!(research_complete["report"]["commit_sha"], research_commit);
    assert_eq!(
        research_complete["report"]["changed_paths"][0],
        "docs/readme.md"
    );

    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains("agent send-keys cadence-"));
    assert!(calls.contains("ctrl+c"));
    let completion_notification = calls.rfind("agent prompt cadence-lead-").unwrap();
    let agent_interrupt = calls.rfind("agent send-keys cadence-").unwrap();
    assert!(
        completion_notification < agent_interrupt,
        "Lead completion notification must precede interrupting the completing agent"
    );
    if use_worktree {
        let tab_close = calls.rfind("tab close agent-tab").unwrap();
        let worktree_remove = calls
            .rfind("worktree remove --workspace agent-ws --force")
            .unwrap();
        assert!(
            tab_close < worktree_remove,
            "The worktree-backed agent tab must close before its workspace is removed"
        );
    } else {
        let tab_close = calls.rfind("tab close tab-agent").unwrap();
        assert!(
            completion_notification < tab_close,
            "Lead completion notification must precede closing the completing agent's tab"
        );
    }

    let state_path = state.path().join("state.json");
    let mut store_before_finish: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    let project = store_before_finish["projects"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    let integrated_agent = &mut project["runs"][&active_run]["agents"]["agent-1"];
    if use_worktree {
        integrated_agent["workspace_id"] = "stale-workspace".into();
    } else {
        integrated_agent["tab_id"] = "stale-tab".into();
    }
    fs::write(
        &state_path,
        serde_json::to_vec_pretty(&store_before_finish).unwrap(),
    )
    .unwrap();
    let finished_run = store_before_finish["projects"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()["runs"][&active_run]
        .clone();
    if use_worktree {
        let blocked_finish = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
            .args([
                "--state-dir",
                state.path().to_str().unwrap(),
                "--project-root",
                repo.path().to_str().unwrap(),
                "run",
                "finish",
            ])
            .env("HERDR_BIN_PATH", &fake)
            .env("CADENCE_TEST_FAIL_WORKTREE_REMOVE", "1")
            .output()
            .unwrap();
        assert!(!blocked_finish.status.success());
        assert!(String::from_utf8_lossy(&blocked_finish.stderr).contains("run finish --force"));
    }
    let mut finish_command = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"));
    finish_command.args([
        "--state-dir",
        state.path().to_str().unwrap(),
        "--project-root",
        repo.path().to_str().unwrap(),
        "run",
        "finish",
    ]);
    if use_worktree {
        finish_command
            .arg("--force")
            .env("CADENCE_TEST_FAIL_WORKTREE_REMOVE", "1");
    }
    let finish = finish_command
        .env("HERDR_BIN_PATH", &fake)
        .output()
        .unwrap();
    assert!(
        finish.status.success(),
        "{}",
        String::from_utf8_lossy(&finish.stderr)
    );
    let finished: serde_json::Value = serde_json::from_slice(&finish.stdout).unwrap();
    assert_eq!(finished["run_id"], active_run);
    assert_eq!(
        finished["cleanup_warnings"].as_array().unwrap().is_empty(),
        !use_worktree
    );
    let mut store: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    let project = store["projects"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert!(project["active_run"].is_null());
    assert!(project["runs"].as_object().unwrap().is_empty());

    if !use_worktree && !global_yolo {
        let project = store["projects"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        let mut legacy_run = finished_run;
        legacy_run["id"] = "legacy-completed-run".into();
        legacy_run["status"] = "completed".into();
        project["runs"]
            .as_object_mut()
            .unwrap()
            .insert("legacy-completed-run".into(), legacy_run);
        fs::write(&state_path, serde_json::to_vec_pretty(&store).unwrap()).unwrap();

        let restart = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"))
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
            restart.status.success(),
            "{}",
            String::from_utf8_lossy(&restart.stderr)
        );
        let restarted_store: serde_json::Value =
            serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
        let project = restarted_store["projects"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap();
        assert!(project["runs"].get("legacy-completed-run").is_none());
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

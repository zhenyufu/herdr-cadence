#![cfg(unix)]

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

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

fn cadence_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_herdr-cadence"));
    for variable in [
        "CADENCE_STATE_DIR",
        "CADENCE_PROJECT_ROOT",
        "CADENCE_CONFIG_DIR",
        "CADENCE_RUN_ID",
        "HERDR_PLUGIN_STATE_DIR",
        "HERDR_PLUGIN_CONFIG_DIR",
        "HERDR_PLUGIN_EVENT",
        "HERDR_PLUGIN_EVENT_JSON",
        "HERDR_PLUGIN_CONTEXT_JSON",
        "HERDR_WORKSPACE_ID",
        "HERDR_BIN_PATH",
        "CADENCE_TEST_FAIL_WORKTREE_REMOVE",
    ] {
        command.env_remove(variable);
    }
    command.env("HERDR_BIN_PATH", "/dev/null");
    command
}

fn cadence(root: &Path, state: &Path, args: &[&str]) -> Output {
    cadence_command()
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

fn cadence_with_config_dir(root: &Path, state: &Path, config_dir: &Path, args: &[&str]) -> Output {
    cadence_command()
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--project-root",
            root.to_str().unwrap(),
            "--config-dir",
            config_dir.to_str().unwrap(),
        ])
        .args(args)
        .output()
        .unwrap()
}

fn write_agent_cancel_fixture(
    repo: &Path,
    state: &Path,
    agents: serde_json::Map<String, serde_json::Value>,
) {
    let key = herdr_cadence::state::project_key(repo);
    let store = json!({
        "schema_version": 1,
        "projects": {
            key: {
                "root": repo.display().to_string(),
                "active_run": "run-cancel",
                "runs": {
                    "run-cancel": {
                        "id": "run-cancel",
                        "status": "active",
                        "base_branch": "main",
                        "base_workspace_id": "base-workspace",
                        "lead": {
                            "name": "cadence-lead-cancel",
                            "harness": "codex"
                        },
                        "created_unix_ms": 1,
                        "next_agent": 1,
                        "agents": agents
                    }
                }
            }
        }
    });
    fs::write(
        state.join("state.json"),
        serde_json::to_vec_pretty(&store).unwrap(),
    )
    .unwrap();
}

fn cancel_agent_json(
    repo: &Path,
    state: &Path,
    fake: &Path,
    agent_id: &str,
    force: bool,
) -> Output {
    let mut args = vec!["agent", "cancel", agent_id];
    if force {
        args.push("--force");
    }
    cadence_command()
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--project-root",
            repo.to_str().unwrap(),
        ])
        .args(args)
        .env("HERDR_BIN_PATH", fake)
        .output()
        .unwrap()
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn start_prune_fixture(
    repo: &Path,
    state: &Path,
    live_lead: bool,
) -> (tempfile::TempDir, String, serde_json::Value) {
    let initialized = cadence(repo, state, &["action", "init"]);
    assert!(
        initialized.status.success(),
        "{}",
        String::from_utf8_lossy(&initialized.stderr)
    );

    let key = herdr_cadence::state::project_key(repo);
    let lead_name = format!("cadence-lead-{key}");
    let active_run = json!({
        "id": "run-active",
        "status": "active",
        "base_branch": "main",
        "base_workspace_id": "original-workspace",
        "lead": {
            "name": lead_name,
            "harness": "claude",
            "model": "old-lead-model",
            "reasoning_effort": "low",
            "workspace_id": "existing-workspace",
            "tab_id": "existing-tab",
            "pane_id": "existing-pane"
        },
        "created_unix_ms": 123,
        "next_agent": 41,
        "agents": {
            "clean": {
                "id": "clean",
                "title": "Clean integrated agent",
                "task": "Finished task",
                "scope": ["src/old"],
                "acceptance": ["Task is complete"],
                "harness": "codex",
                "use_worktree": true,
                "branch": "cadence/old/clean",
                "base_sha": "base",
                "agent_name": "cadence-old-clean",
                "status": "integrated",
                "report": {
                    "status": "completed",
                    "summary": "Finished task"
                }
            },
            "working": {
                "id": "working",
                "title": "Current agent",
                "task": "Current task",
                "scope": ["src/current"],
                "acceptance": ["Task is complete"],
                "harness": "codex",
                "use_worktree": true,
                "branch": "cadence/current/working",
                "base_sha": "base",
                "agent_name": "cadence-current-working",
                "status": "working"
            },
            "resumable": {
                "id": "resumable",
                "title": "Resumable shared-checkout agent",
                "task": "Current shared-checkout task",
                "scope": ["src/shared"],
                "acceptance": ["Task is complete"],
                "harness": "codex",
                "use_worktree": false,
                "branch": "main",
                "base_sha": "base",
                "agent_name": "cadence-current-resumable",
                "status": "working"
            },
            "claimed": {
                "id": "claimed",
                "title": "Claimed integrated agent",
                "task": "Finished task with claimed commits",
                "scope": ["src/claimed"],
                "acceptance": ["Task is complete"],
                "harness": "codex",
                "use_worktree": false,
                "branch": "main",
                "base_sha": "base",
                "claimed_commits": ["claimed-commit"],
                "agent_name": "cadence-old-claimed",
                "status": "integrated"
            },
            "legacy": {
                "id": "legacy",
                "title": "Legacy integrated agent",
                "task": "Finished task with legacy attribution",
                "scope": ["src/legacy"],
                "acceptance": ["Task is complete"],
                "harness": "codex",
                "use_worktree": false,
                "branch": "main",
                "base_sha": "base",
                "agent_name": "cadence-old-legacy",
                "status": "integrated",
                "report": {
                    "status": "completed",
                    "summary": "Finished task",
                    "commit_sha": "legacy-commit"
                }
            },
            "retained": {
                "id": "retained",
                "title": "Retained integrated agent",
                "task": "Finished task with a resource",
                "scope": ["src/retained"],
                "acceptance": ["Task is complete"],
                "harness": "codex",
                "use_worktree": true,
                "branch": "cadence/old/retained",
                "base_sha": "base",
                "agent_name": "cadence-old-retained",
                "status": "integrated",
                "workspace_id": "stale-workspace"
            }
        }
    });
    let old_run = json!({
        "id": "run-old",
        "status": "completed",
        "base_branch": "main",
        "base_workspace_id": "old-workspace",
        "lead": {"name": "cadence-lead-old", "harness": "codex"},
        "created_unix_ms": 99,
        "next_agent": 2,
        "agents": {}
    });
    let other_project = json!({
        "root": "/sentinel/project",
        "active_run": "sentinel-run",
        "runs": {
            "sentinel-run": {
                "id": "sentinel-run",
                "status": "active",
                "base_branch": "main",
                "base_workspace_id": "sentinel-workspace",
                "lead": {"name": "sentinel-lead", "harness": "codex"},
                "created_unix_ms": 456,
                "next_agent": 1,
                "agents": {}
            }
        }
    });
    let original_store = json!({
        "schema_version": 1,
        "projects": {
            key.clone(): {
                "root": repo.display().to_string(),
                "active_run": "run-active",
                "runs": {
                    "run-active": active_run,
                    "run-old": old_run
                }
            },
            "sentinel-project": other_project
        }
    });
    fs::write(
        state.join("state.json"),
        serde_json::to_vec_pretty(&original_store).unwrap(),
    )
    .unwrap();
    let expected_store = serde_json::to_value(
        serde_json::from_value::<herdr_cadence::model::Store>(original_store).unwrap(),
    )
    .unwrap();

    let fake_dir = tempfile::tempdir().unwrap();
    let fake = fake_dir.path().join("herdr");
    let live_marker = fake_dir.path().join("live-lead");
    if live_lead {
        fs::write(&live_marker, "live\n").unwrap();
    }
    let script = format!(
        r#"#!/bin/sh
if [ "$1 $2" = "agent get" ]; then
  case "$3" in
    cadence-lead-*)
      if [ -e '{}' ]; then
        exit 0
      fi
      ;;
  esac
  printf '%s\n' '{{"error":{{"code":"agent_not_found"}}}}' >&2
  exit 1
fi
if [ "$1 $2" = "tab create" ]; then
  printf '%s\n' '{{"id":"test","result":{{"tab":{{"tab_id":"relaunched-tab"}},"root_pane":{{"pane_id":"relaunched-pane"}}}}}}'
  exit 0
fi
if [ "$1 $2" = "pane process-info" ]; then
  printf '%s\n' '{{"id":"test","result":{{"type":"pane_process_info","process_info":{{"pane_id":"relaunched-pane","shell_pid":123,"foreground_process_group_id":123}}}}}}'
  exit 0
fi
exit 0
"#,
        live_marker.display()
    );
    fs::write(&fake, script).unwrap();
    let mut permissions = fs::metadata(&fake).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake, permissions).unwrap();
    (fake_dir, key, expected_store)
}

fn run_start_prune_fixture(live_lead: bool) {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    let (fake_dir, key, original_store) = start_prune_fixture(repo.path(), state.path(), live_lead);
    let result = cadence_command()
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "action",
            "start",
        ])
        .env("HERDR_BIN_PATH", fake_dir.path().join("herdr"))
        .env("HERDR_WORKSPACE_ID", "new-workspace")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result_value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(
        result_value["status"],
        if live_lead { "focused" } else { "started" }
    );

    let state_path = state.path().join("state.json");
    let stored: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    assert_eq!(stored["schema_version"], 1);
    let project = &stored["projects"][&key];
    assert_eq!(project["active_run"], "run-active");
    assert_eq!(
        project["runs"]["run-old"],
        original_store["projects"][&key]["runs"]["run-old"]
    );
    assert_eq!(
        stored["projects"]["sentinel-project"],
        original_store["projects"]["sentinel-project"]
    );

    let mut expected_run = original_store["projects"][&key]["runs"]["run-active"].clone();
    expected_run["agents"]
        .as_object_mut()
        .unwrap()
        .remove("clean");
    let agents = project["runs"]["run-active"]["agents"].as_object().unwrap();
    assert_eq!(agents["resumable"]["status"], "working");
    assert_eq!(agents["resumable"]["use_worktree"], false);
    assert_eq!(
        agents["claimed"]["claimed_commits"],
        json!(["claimed-commit"])
    );
    assert_eq!(agents["legacy"]["report"]["commit_sha"], "legacy-commit");
    assert!(!agents.contains_key("clean"));
    if !live_lead {
        expected_run["base_workspace_id"] = "new-workspace".into();
        expected_run["lead"]["harness"] = "codex".into();
        expected_run["lead"]["model"] = "gpt-5.6-terra".into();
        expected_run["lead"]["reasoning_effort"] = "high".into();
        expected_run["lead"]["workspace_id"] = "new-workspace".into();
        expected_run["lead"]["tab_id"] = "relaunched-tab".into();
        expected_run["lead"]["pane_id"] = "relaunched-pane".into();
    }
    assert_eq!(project["runs"]["run-active"], expected_run);
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
    assert!(config.contains("schema_version = 2"));
    assert!(config.contains("agent_default = \"generalist\""));
    assert!(config.contains("[agents.roles.generalist]"));
    assert!(config.contains("runners = [\"codex-terra-medium\"]"));
    assert!(config.contains("[agents.runners.codex-terra-medium]"));
    assert!(config.contains("description = \"Implements general changes"));
    assert!(config.contains("[agents.roles.planner]"));
    assert!(!config.contains("[agents.roles.planning]"));
    assert!(config.contains("[agents.roles.researcher]"));
    assert!(config.contains("[agents.roles.developer]"));
    assert!(config.contains("[agents.roles.reviewer]"));
    assert!(config.contains("[agents.roles.qa]"));
    assert!(config.find("[git]").unwrap() < config.find("[agents.roles.").unwrap());
    let parsed: herdr_cadence::config::Config = toml::from_str(&config).unwrap();
    assert!(
        parsed
            .agents
            .roles
            .values()
            .all(|role| !role.runners.is_empty())
    );
    assert_eq!(parsed.lead.max_parallel, Some(4));
    assert_eq!(parsed.lead.model.as_deref(), Some("gpt-5.6-terra"));
    assert_eq!(parsed.agent_default, "generalist");
    let generalist = parsed.agents.roles.get("generalist").unwrap();
    assert_eq!(generalist.runners[0], "codex-terra-medium");
    let roles_using_claude = parsed
        .agents
        .roles
        .iter()
        .filter(|(_, role)| {
            role.runners.iter().any(|runner| {
                parsed.agents.runners[runner].harness == herdr_cadence::config::Harness::Claude
            })
        })
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(roles_using_claude, ["reviewer"]);
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
fn status_fixture_does_not_invoke_ambient_herdr_from_path() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    assert!(
        cadence(repo.path(), state.path(), &["action", "init"])
            .status
            .success()
    );

    let fake_dir = tempfile::tempdir().unwrap();
    let fake_herdr = fake_dir.path().join("herdr");
    let invocation_marker = fake_dir.path().join("invoked");
    fs::write(
        &fake_herdr,
        format!(
            "#!/bin/sh\nprintf '%s\\n' invoked > '{}'\n",
            invocation_marker.display()
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&fake_herdr).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake_herdr, permissions).unwrap();
    let path = format!(
        "{}:{}",
        fake_dir.path().display(),
        env::var("PATH").unwrap_or_default()
    );

    let status = cadence_command()
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "action",
            "status",
        ])
        .env("PATH", path)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(!invocation_marker.exists());
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
    assert_eq!(planner["runners"][0]["name"], "codex-sol-high");
    assert_eq!(planner["runners"][0]["model"], "gpt-5.6-sol");
    assert_eq!(planner["runners"][0]["reasoning_effort"], "high");
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
fn falls_back_to_the_global_config_when_no_project_config_exists() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    fs::write(
        config_dir.path().join("cadence.toml"),
        herdr_cadence::config::DEFAULT_CONFIG_TOML,
    )
    .unwrap();

    let status = cadence_with_config_dir(
        repo.path(),
        state.path(),
        config_dir.path(),
        &["action", "status"],
    );
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["enabled"], true);
    assert_eq!(value["config_valid"], true);

    let validation = cadence_with_config_dir(
        repo.path(),
        state.path(),
        config_dir.path(),
        &["action", "validate-config"],
    );
    assert!(
        validation.status.success(),
        "{}",
        String::from_utf8_lossy(&validation.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&validation.stdout).unwrap();
    assert_eq!(value["valid"], true);
    assert_eq!(
        value["config"],
        config_dir.path().join("cadence.toml").to_str().unwrap()
    );
    assert!(!repo.path().join(".cadence.toml").exists());
}

#[test]
fn accepts_a_dotted_global_config_file_name() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    // A project .cadence.toml moved into the config directory keeps its name.
    fs::write(
        config_dir.path().join(".cadence.toml"),
        herdr_cadence::config::DEFAULT_CONFIG_TOML,
    )
    .unwrap();

    let validation = cadence_with_config_dir(
        repo.path(),
        state.path(),
        config_dir.path(),
        &["action", "validate-config"],
    );
    assert!(
        validation.status.success(),
        "{}",
        String::from_utf8_lossy(&validation.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&validation.stdout).unwrap();
    assert_eq!(value["valid"], true);
    assert_eq!(
        value["config"],
        config_dir.path().join(".cadence.toml").to_str().unwrap()
    );
}

#[test]
fn reads_the_global_config_dir_from_the_herdr_plugin_env() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    fs::write(
        config_dir.path().join("cadence.toml"),
        herdr_cadence::config::DEFAULT_CONFIG_TOML,
    )
    .unwrap();

    // No --config-dir flag: the directory comes from Herdr's plugin env var.
    let validation = cadence_command()
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
        ])
        .args(["action", "validate-config"])
        .env("HERDR_PLUGIN_CONFIG_DIR", config_dir.path())
        .output()
        .unwrap();
    assert!(
        validation.status.success(),
        "{}",
        String::from_utf8_lossy(&validation.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&validation.stdout).unwrap();
    assert_eq!(
        value["config"],
        config_dir.path().join("cadence.toml").to_str().unwrap()
    );
}

#[test]
fn project_config_overrides_the_global_config() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    // A global config that would fail validation if it were ever read.
    fs::write(
        config_dir.path().join("cadence.toml"),
        "this is not valid toml",
    )
    .unwrap();

    assert!(
        cadence_with_config_dir(
            repo.path(),
            state.path(),
            config_dir.path(),
            &["action", "init"]
        )
        .status
        .success()
    );

    let validation = cadence_with_config_dir(
        repo.path(),
        state.path(),
        config_dir.path(),
        &["action", "validate-config"],
    );
    assert!(
        validation.status.success(),
        "{}",
        String::from_utf8_lossy(&validation.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&validation.stdout).unwrap();
    assert_eq!(value["valid"], true);
    assert_eq!(
        value["config"],
        repo.path().join(".cadence.toml").to_str().unwrap()
    );
}

#[test]
fn explicit_cli_routing_options_override_inherited_cadence_environment() {
    let inherited_root = repo();
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    let inherited_state = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    let inherited_config_dir = tempfile::tempdir().unwrap();
    fs::write(
        config_dir.path().join("cadence.toml"),
        herdr_cadence::config::DEFAULT_CONFIG_TOML,
    )
    .unwrap();
    fs::write(
        inherited_config_dir.path().join("cadence.toml"),
        "this is not valid toml",
    )
    .unwrap();

    let validation = cadence_command()
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "--config-dir",
            config_dir.path().to_str().unwrap(),
            "action",
            "validate-config",
        ])
        .env("CADENCE_STATE_DIR", inherited_state.path())
        .env("CADENCE_PROJECT_ROOT", inherited_root.path())
        .env("CADENCE_CONFIG_DIR", inherited_config_dir.path())
        .env("CADENCE_RUN_ID", "stale-inherited-run")
        .output()
        .unwrap();

    assert!(
        validation.status.success(),
        "{}",
        String::from_utf8_lossy(&validation.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&validation.stdout).unwrap();
    assert_eq!(value["valid"], true);
    assert_eq!(
        value["config"],
        config_dir.path().join("cadence.toml").to_str().unwrap()
    );
    assert!(!repo.path().join(".cadence.toml").exists());
    assert!(!inherited_root.path().join(".cadence.toml").exists());
}

#[test]
fn reports_missing_config_when_neither_project_nor_global_config_exists() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();

    let validation = cadence_with_config_dir(
        repo.path(),
        state.path(),
        config_dir.path(),
        &["action", "validate-config"],
    );
    assert!(!validation.status.success());
    let error: serde_json::Value = serde_json::from_slice(&validation.stderr).unwrap();
    assert!(error["error"].as_str().unwrap().contains("not enabled"));
}

#[test]
fn action_start_persists_pruning_before_focusing_a_live_lead() {
    run_start_prune_fixture(true);
}

#[test]
fn action_start_persists_pruning_before_relaunching_a_missing_lead() {
    run_start_prune_fixture(false);
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
fn rejects_agent_runner_overrides() {
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
        r#"{"title":"Review API","task":"Review the API","scope":["src/api"],"acceptance":["Review complete"],"harness":"claude"}"#,
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
    assert!(String::from_utf8_lossy(&spawned.stderr).contains("harness"));
}

#[test]
fn runs_agent_in_shared_checkout_by_default() {
    run_agent_flow(false, false, false, false, false, false);
}

#[test]
fn runs_agent_in_configured_worktree() {
    run_agent_flow(true, false, false, false, false, false);
}

#[test]
fn retries_integrated_worktree_agent_cleanup_when_lead_becomes_idle() {
    run_agent_flow(true, false, false, false, false, true);
}

#[test]
fn runs_every_agent_in_global_yolo() {
    run_agent_flow(false, true, false, false, false, false);
}

#[test]
fn codex_compact_hook_emits_session_start_context_json() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    write_agent_cancel_fixture(repo.path(), state.path(), Default::default());
    let output = cadence_command()
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "hook",
            "codex-session-start",
        ])
        .env("CADENCE_RUN_ID", "run-cancel")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["hookSpecificOutput"]["hookEventName"], "SessionStart");
    let context = value["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("Lead for Cadence run run-cancel"));
    assert!(context.contains("only you talk to the user"));
    assert!(context.contains("agent spawn"));
    assert!(context.contains("run finish only when the user asks"));
    assert!(context.contains("Cadence Agent.status is authoritative"));
    assert!(context.contains("observed_agent_status is advisory"));
    assert!(context.contains("must never alone trigger cancellation"));
    assert!(context.contains("confirmed exit"));
    assert!(context.contains("nonresponse after follow-up/progress checks"));
    assert!(
        context.contains("verified blocker requiring reassignment (including stale base metadata)")
    );
    assert!(context.contains("Within the assigned task, recover without asking permission"));
    assert!(context.contains("preserve accepted work for integration by the replacement"));
}

#[test]
fn agent_cancel_requires_force_and_preserves_cancellation_lifecycle_rules() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    let fake_dir = tempfile::tempdir().unwrap();
    let fake = fake_dir.path().join("herdr");
    let log = fake_dir.path().join("calls.log");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
        log.display()
    );
    fs::write(&fake, script).unwrap();
    let mut permissions = fs::metadata(&fake).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake, permissions).unwrap();

    let statuses = [
        ("starting", "starting", "working"),
        ("working", "working", "idle"),
        ("blocked", "blocked", "blocked"),
        ("completed", "completed", "done"),
        ("conflict", "conflict", "idle"),
        ("integrating", "integrating", "working"),
        ("integrated", "integrated", "idle"),
        ("failed", "failed", "unknown"),
        ("cancelled", "cancelled", "done"),
    ];
    let agents = statuses
        .iter()
        .map(|(id, status, observed)| {
            (
                (*id).to_string(),
                json!({
                    "id": id,
                    "title": id,
                    "task": "Test cancellation",
                    "scope": ["src"],
                    "acceptance": ["Tests pass"],
                    "harness": "codex",
                    "branch": "main",
                    "base_sha": "base",
                    "agent_name": format!("cadence-cancel-{id}"),
                    "status": status,
                    "observed_agent_status": observed,
                    "use_worktree": false,
                    "pane_id": format!("pane-{id}"),
                    "tab_id": format!("tab-{id}")
                }),
            )
        })
        .collect();
    write_agent_cancel_fixture(repo.path(), state.path(), agents);

    let before = fs::read(state.path().join("state.json")).unwrap();
    for (id, status, observed) in statuses.iter().take(5) {
        let refused = cancel_agent_json(repo.path(), state.path(), &fake, id, false);
        assert!(!refused.status.success());
        let error: serde_json::Value = serde_json::from_slice(&refused.stderr).unwrap();
        let message = error["error"].as_str().unwrap();
        assert!(message.contains("without `--force`"), "{message}");
        assert!(
            message.contains(&format!("lifecycle status is `{status}`")),
            "{message}"
        );
        assert!(
            message.contains(&format!("observed status is `{observed}`")),
            "{message}"
        );
    }
    assert_eq!(fs::read(state.path().join("state.json")).unwrap(), before);
    assert!(!log.exists(), "no-force cancellation must not call Herdr");

    for id in ["starting", "working", "blocked", "completed", "conflict"] {
        let forced = cancel_agent_json(repo.path(), state.path(), &fake, id, true);
        assert!(
            forced.status.success(),
            "{}",
            String::from_utf8_lossy(&forced.stderr)
        );
        let forced: serde_json::Value = serde_json::from_slice(&forced.stdout).unwrap();
        assert_eq!(forced["status"], "cancelled");
        if matches!(id, "completed" | "conflict") {
            assert_eq!(forced["pane_id"], format!("pane-{id}"));
            assert_eq!(forced["tab_id"], format!("tab-{id}"));
        }
    }
    let calls_after_force = fs::read_to_string(&log).unwrap();
    assert!(calls_after_force.contains("agent send-keys cadence-cancel-starting ctrl+c"));
    assert!(calls_after_force.contains("agent get cadence-cancel-working"));
    assert!(calls_after_force.contains("agent send-keys cadence-cancel-working ctrl+c"));
    assert!(calls_after_force.contains("agent send-keys cadence-cancel-blocked ctrl+c"));
    assert!(calls_after_force.contains("agent send-keys cadence-cancel-completed ctrl+c"));
    assert!(calls_after_force.contains("agent send-keys cadence-cancel-conflict ctrl+c"));

    for (id, status, observed) in statuses.iter().skip(5) {
        let before_terminal = fs::read(state.path().join("state.json")).unwrap();
        let calls_before_terminal = fs::read_to_string(&log).unwrap();
        let refused = cancel_agent_json(repo.path(), state.path(), &fake, id, true);
        assert!(!refused.status.success());
        let error: serde_json::Value = serde_json::from_slice(&refused.stderr).unwrap();
        let message = error["error"].as_str().unwrap();
        assert!(
            message.contains(&format!("lifecycle status is `{status}`")),
            "{message}"
        );
        assert!(
            message.contains(&format!("observed status is `{observed}`")),
            "{message}"
        );
        assert!(
            message.contains("only Starting, Working, Blocked, Completed, or Conflict"),
            "{message}"
        );
        assert_eq!(
            fs::read(state.path().join("state.json")).unwrap(),
            before_terminal
        );
        assert_eq!(fs::read_to_string(&log).unwrap(), calls_before_terminal);
    }
}

#[test]
fn agent_cancel_and_integrate_are_serialized() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    let initialized = cadence(repo.path(), state.path(), &["action", "init"]);
    assert!(
        initialized.status.success(),
        "{}",
        String::from_utf8_lossy(&initialized.stderr)
    );
    git(repo.path(), &["add", ".cadence.toml"]);
    git(repo.path(), &["commit", "-m", "enable Cadence"]);

    let fake_dir = tempfile::tempdir().unwrap();
    let fake = fake_dir.path().join("herdr");
    let cancel_started = fake_dir.path().join("cancel-started");
    let cancel_release = fake_dir.path().join("cancel-release");
    let calls = fake_dir.path().join("calls.log");
    let script = format!(
        r#"#!/bin/sh
if [ "$1 $2" = "agent send-keys" ]; then
  printf '%s\n' "$*" >> '{}'
fi
if [ "$1 $2" = "agent get" ] && [ "$3" = "cadence-cancel-race" ]; then
  touch '{}'
  while [ ! -e '{}' ]; do sleep 0.01; done
fi
exit 0
"#,
        calls.display(),
        cancel_started.display(),
        cancel_release.display()
    );
    fs::write(&fake, script).unwrap();
    let mut permissions = fs::metadata(&fake).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake, permissions).unwrap();

    write_agent_cancel_fixture(
        repo.path(),
        state.path(),
        [(
            "race".into(),
            json!({
                "id": "race",
                "title": "Race agent",
                "task": "Test cancellation serialization",
                "scope": ["src"],
                "acceptance": ["Tests pass"],
                "harness": "codex",
                "branch": "main",
                "base_sha": "base",
                "agent_name": "cadence-cancel-race",
                "status": "completed",
                "observed_agent_status": "done",
                "use_worktree": false,
                "pane_id": "pane-race",
                "tab_id": "tab-race"
            }),
        )]
        .into_iter()
        .collect(),
    );

    let cancellation = cadence_command()
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "agent",
            "cancel",
            "race",
            "--force",
        ])
        .env("HERDR_BIN_PATH", &fake)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_file(&cancel_started);

    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(state.path().join("state.lock"))
        .unwrap();
    assert!(
        lock.try_lock_shared().is_err(),
        "cancellation must hold the state lock during the Herdr call"
    );

    let integration = cadence_command()
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "agent",
            "integrate",
            "race",
        ])
        .env("HERDR_BIN_PATH", &fake)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    fs::write(&cancel_release, "release\n").unwrap();

    let cancellation = cancellation.wait_with_output().unwrap();
    assert!(
        cancellation.status.success(),
        "{}",
        String::from_utf8_lossy(&cancellation.stderr)
    );
    let cancelled: serde_json::Value = serde_json::from_slice(&cancellation.stdout).unwrap();
    assert_eq!(cancelled["status"], "cancelled");

    let integration = integration.wait_with_output().unwrap();
    assert!(!integration.status.success());
    let integration_error: serde_json::Value = serde_json::from_slice(&integration.stderr).unwrap();
    assert!(
        integration_error["error"]
            .as_str()
            .unwrap()
            .contains("completed report")
    );
    assert_eq!(
        fs::read_to_string(&calls)
            .unwrap()
            .matches("agent send-keys cadence-cancel-race ctrl+c")
            .count(),
        1,
        "only the cancellation that won may send Ctrl-C"
    );

    let integration_started = fake_dir.path().join("integration-started");
    let integration_release = fake_dir.path().join("integration-release");
    let git_dir = tempfile::tempdir().unwrap();
    let real_git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|directory| directory.join("git"))
        .find(|path| path.is_file())
        .expect("git must be available for the CLI flow tests");
    let git_wrapper = git_dir.path().join("git");
    let git_script = format!(
        r#"#!/bin/sh
if [ "$1" = "-C" ] && [ "$3 $4" = "status --porcelain" ]; then
  touch '{}'
  while [ ! -e '{}' ]; do sleep 0.01; done
fi
exec '{}' "$@"
"#,
        integration_started.display(),
        integration_release.display(),
        real_git.display()
    );
    fs::write(&git_wrapper, git_script).unwrap();
    let mut permissions = fs::metadata(&git_wrapper).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&git_wrapper, permissions).unwrap();

    let integration_herdr = fake_dir.path().join("integration-herdr");
    let integration_calls = fake_dir.path().join("integration-calls.log");
    let integration_herdr_script = format!(
        "#!/bin/sh\nif [ \"$1 $2\" = \"agent send-keys\" ]; then printf '%s\\n' \"$*\" >> '{}'; fi\nexit 0\n",
        integration_calls.display()
    );
    fs::write(&integration_herdr, integration_herdr_script).unwrap();
    let mut permissions = fs::metadata(&integration_herdr).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&integration_herdr, permissions).unwrap();

    write_agent_cancel_fixture(
        repo.path(),
        state.path(),
        [(
            "race".into(),
            json!({
                "id": "race",
                "title": "Race agent",
                "task": "Test integration serialization",
                "scope": ["src"],
                "acceptance": ["Tests pass"],
                "harness": "codex",
                "branch": "main",
                "base_sha": "base",
                "agent_name": "cadence-cancel-race",
                "status": "completed",
                "observed_agent_status": "done",
                "use_worktree": false,
                "pane_id": "pane-race",
                "tab_id": "tab-race"
            }),
        )]
        .into_iter()
        .collect(),
    );

    let integration = cadence_command()
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "agent",
            "integrate",
            "race",
        ])
        .env("HERDR_BIN_PATH", &integration_herdr)
        .env(
            "PATH",
            std::env::join_paths(
                [git_dir.path().to_path_buf()]
                    .into_iter()
                    .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
            )
            .unwrap(),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_file(&integration_started);

    let calls_before_cancel = fs::read_to_string(&integration_calls).unwrap_or_default();
    let refused = cancel_agent_json(repo.path(), state.path(), &integration_herdr, "race", true);
    assert!(!refused.status.success());
    let refused_error: serde_json::Value = serde_json::from_slice(&refused.stderr).unwrap();
    assert!(
        refused_error["error"]
            .as_str()
            .unwrap()
            .contains("lifecycle status is `integrating`")
    );
    assert_eq!(
        fs::read_to_string(&integration_calls).unwrap_or_default(),
        calls_before_cancel,
        "cancellation must not call Herdr after integration enters Integrating"
    );

    fs::write(&integration_release, "release\n").unwrap();
    let integration = integration.wait_with_output().unwrap();
    assert!(
        integration.status.success(),
        "{}",
        String::from_utf8_lossy(&integration.stderr)
    );
    let integrated: serde_json::Value = serde_json::from_slice(&integration.stdout).unwrap();
    assert_eq!(integrated["status"], "integrated");
    assert_eq!(
        fs::read_to_string(&integration_calls)
            .unwrap()
            .matches("agent send-keys cadence-cancel-race ctrl+c")
            .count(),
        1,
        "integration cleanup may send Ctrl-C only after it has won"
    );
}

#[test]
fn agent_idle_notification_is_advisory_and_keeps_working_lifecycle() {
    let repo = repo();
    let state = tempfile::tempdir().unwrap();
    assert!(
        cadence(repo.path(), state.path(), &["action", "init"])
            .status
            .success()
    );
    let fake_dir = tempfile::tempdir().unwrap();
    let fake = fake_dir.path().join("herdr");
    let log = fake_dir.path().join("calls.log");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
        log.display()
    );
    fs::write(&fake, script).unwrap();
    let mut permissions = fs::metadata(&fake).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake, permissions).unwrap();
    write_agent_cancel_fixture(
        repo.path(),
        state.path(),
        [(
            "working".into(),
            json!({
                "id": "working",
                "title": "Working agent",
                "task": "Test idle observation",
                "scope": ["src"],
                "acceptance": ["Tests pass"],
                "harness": "codex",
                "branch": "main",
                "base_sha": "base",
                "agent_name": "cadence-cancel-working",
                "status": "working",
                "pane_id": "pane-working",
                "use_worktree": false
            }),
        )]
        .into_iter()
        .collect(),
    );

    let event = cadence_command()
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "event",
        ])
        .env("HERDR_BIN_PATH", &fake)
        .env("HERDR_PLUGIN_EVENT", "pane.agent_status_changed")
        .env(
            "HERDR_PLUGIN_EVENT_JSON",
            r#"{"event":"pane.agent_status_changed","data":{"pane_id":"pane-working","agent_status":"idle"}}"#,
        )
        .output()
        .unwrap();
    assert!(
        event.status.success(),
        "{}",
        String::from_utf8_lossy(&event.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&event.stdout).unwrap();
    assert_eq!(value["agent_id"], "working");
    let status = cadence(repo.path(), state.path(), &["agent", "status", "working"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["status"], "working");
    assert_eq!(status["observed_agent_status"], "idle");
    let notification = fs::read_to_string(&log).unwrap();
    assert!(notification.contains("lifecycle remains working"));
    assert!(notification.contains("runtime observation is advisory"));
    assert!(notification.contains("do not cancel solely from it"));
}

#[test]
fn starts_codex_leads_with_a_compact_session_hook() {
    run_agent_flow_with_lead(
        false,
        false,
        false,
        false,
        false,
        false,
        herdr_cadence::config::Harness::Codex,
    );
}

#[test]
fn keeps_claude_lead_instructions_as_a_post_launch_prompt() {
    run_agent_flow_with_lead(
        false,
        false,
        false,
        false,
        false,
        false,
        herdr_cadence::config::Harness::Claude,
    );
}

#[test]
fn starts_a_pi_lead_with_thinking_flag() {
    run_agent_flow_with_lead(
        false,
        false,
        false,
        false,
        false,
        false,
        herdr_cadence::config::Harness::Pi,
    );
}

#[test]
fn runs_omp_lead_and_worktree_worker_with_interactive_approval() {
    run_omp_agent_flow(true, false, false, false);
}

#[test]
fn runs_omp_lead_and_shared_worker_with_auto_approval() {
    run_omp_agent_flow(false, true, false, false);
}

#[test]
fn relaunches_omp_lead_and_runs_worker_without_overriding_native_defaults() {
    run_omp_agent_flow(false, false, true, false);
}

#[test]
fn falls_back_from_omp_to_codex_and_persists_the_selected_runner() {
    run_omp_agent_flow(false, false, false, true);
}

#[test]
fn relaunches_lead_from_current_config_and_refreshes_persisted_settings() {
    run_agent_flow_with_relaunch();
}

#[test]
fn starts_dirty_but_blocks_agents_until_clean() {
    run_agent_flow(true, false, true, false, false, false);
}

#[test]
fn rejects_an_earlier_out_of_scope_shared_checkout_commit() {
    run_agent_flow(false, false, false, true, false, false);
}

#[test]
fn falls_back_to_the_next_runner_after_credit_exhaustion() {
    run_agent_flow(false, false, false, false, true, false);
}

fn run_agent_flow(
    use_worktree: bool,
    global_yolo: bool,
    dirty_at_start: bool,
    create_out_of_scope_commit: bool,
    force_primary_credit_failure: bool,
    force_tab_cleanup_retry: bool,
) {
    run_agent_flow_with_lead(
        use_worktree,
        global_yolo,
        dirty_at_start,
        create_out_of_scope_commit,
        force_primary_credit_failure,
        force_tab_cleanup_retry,
        herdr_cadence::config::Harness::Opencode,
    );
}

fn run_agent_flow_with_lead(
    use_worktree: bool,
    global_yolo: bool,
    dirty_at_start: bool,
    create_out_of_scope_commit: bool,
    force_primary_credit_failure: bool,
    force_tab_cleanup_retry: bool,
    lead_harness: herdr_cadence::config::Harness,
) {
    run_agent_flow_with_lead_options(
        use_worktree,
        global_yolo,
        dirty_at_start,
        create_out_of_scope_commit,
        force_primary_credit_failure,
        force_tab_cleanup_retry,
        LeadFlowSettings {
            lead_harness,
            worker_harness: herdr_cadence::config::Harness::Codex,
            worker_native_defaults: false,
            relaunch_lead: None,
        },
    );
}

fn run_agent_flow_with_relaunch() {
    run_agent_flow_with_lead_options(
        false,
        false,
        false,
        false,
        false,
        false,
        LeadFlowSettings {
            lead_harness: herdr_cadence::config::Harness::Codex,
            worker_harness: herdr_cadence::config::Harness::Codex,
            worker_native_defaults: false,
            relaunch_lead: Some((
                herdr_cadence::config::Harness::Opencode,
                Some("openai/relaunch-model".into()),
                herdr_cadence::config::ReasoningEffort::Low,
                true,
            )),
        },
    );
}

fn run_omp_agent_flow(
    use_worktree: bool,
    global_yolo: bool,
    native_defaults: bool,
    force_primary_credit_failure: bool,
) {
    run_agent_flow_with_lead_options(
        use_worktree,
        global_yolo,
        false,
        false,
        force_primary_credit_failure,
        false,
        LeadFlowSettings {
            lead_harness: herdr_cadence::config::Harness::Omp,
            worker_harness: herdr_cadence::config::Harness::Omp,
            worker_native_defaults: native_defaults,
            relaunch_lead: native_defaults.then_some((
                herdr_cadence::config::Harness::Omp,
                None,
                herdr_cadence::config::ReasoningEffort::Default,
                false,
            )),
        },
    );
}

struct LeadFlowSettings {
    lead_harness: herdr_cadence::config::Harness,
    worker_harness: herdr_cadence::config::Harness,
    worker_native_defaults: bool,
    relaunch_lead: Option<(
        herdr_cadence::config::Harness,
        Option<String>,
        herdr_cadence::config::ReasoningEffort,
        bool,
    )>,
}

fn run_agent_flow_with_lead_options(
    use_worktree: bool,
    global_yolo: bool,
    dirty_at_start: bool,
    create_out_of_scope_commit: bool,
    force_primary_credit_failure: bool,
    force_tab_cleanup_retry: bool,
    settings: LeadFlowSettings,
) {
    let LeadFlowSettings {
        lead_harness,
        worker_harness,
        worker_native_defaults,
        relaunch_lead,
    } = settings;
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
    config.lead.harness = lead_harness;
    config.lead.model = Some("openai/lead-model".into());
    {
        let qa = config.agents.roles.get_mut("qa").unwrap();
        qa.description = "Validates test behavior".into();
        qa.runners = vec!["qa-primary".into(), "qa-backup".into()];
        if use_worktree {
            qa.version_control_mode = herdr_cadence::config::VersionControlMode::GitWorktree;
        }
    }
    config.agents.runners.insert(
        "qa-primary".into(),
        herdr_cadence::config::RunnerConfig {
            harness: worker_harness,
            model: (!worker_native_defaults).then(|| "qa-model".into()),
            reasoning_effort: if worker_native_defaults {
                herdr_cadence::config::ReasoningEffort::Default
            } else {
                herdr_cadence::config::ReasoningEffort::Low
            },
        },
    );
    config.agents.runners.insert(
        "qa-backup".into(),
        herdr_cadence::config::RunnerConfig {
            harness: if worker_harness == herdr_cadence::config::Harness::Omp {
                herdr_cadence::config::Harness::Codex
            } else {
                herdr_cadence::config::Harness::Opencode
            },
            model: Some("backup-model".into()),
            reasoning_effort: herdr_cadence::config::ReasoningEffort::High,
        },
    );
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
    let primary_credit_failure = fake_dir.path().join("primary-credit-failure");
    let tab_close_failure = fake_dir.path().join("tab-close-failure");
    let agent_path = fake_dir.path().join("agent");
    fs::create_dir(&agent_path).unwrap();
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$*" >> '{}'
if [ "$1 $2" = "agent start" ]; then
  name="$3"
  if [ "${{#name}}" -gt 32 ]; then
    printf '%s\n' 'invalid_agent_name: exceeds 32 characters' >&2
    exit 1
  fi
  case "$name" in
    [!a-z]*|*[!a-z0-9_-]*|'') exit 1 ;;
  esac
  case "$*" in
    *"--kind omp "*)
      case "$*" in
        *" {omp_permission}"*) ;;
        *) printf '%s\n' 'OMP launch missing explicit approval policy' >&2; exit 1 ;;
      esac
      case "$*" in
        *"{omp_forbidden_permission}"*|*"--sandbox"*|*"--add-dir"*|*"--dangerously-"*|*"--ask-for-approval"*)
          printf '%s\n' 'OMP launch contains incompatible permission flags' >&2
          exit 1
          ;;
      esac
      ;;
  esac
fi
if [ "$1 $2" = "agent get" ]; then
  case "$3" in
    cadence-lead-*)
      if [ -e '{}' ]; then
        printf '%s\n' '{{"id":"test","result":{{}}}}'
        exit 0
      fi
      ;;
    cadence-??????-a*|cadence-run-*-a*|cad-????????????????-a*)
      printf '%s\n' '{{"id":"test","result":{{"agent":{{"tab_id":"agent-tab"}}}}}}'
      exit 0
      ;;
  esac
  printf '%s\n' '{{"error":{{"code":"agent_not_found"}}}}' >&2
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
case "$*" in
  "agent start cadence-"*|"agent start cad-"*)
    if [ -e '{}' ]; then
      case "$*" in
        *"--model qa-model"*)
          printf '%s\n' 'credit exhausted' >&2
          exit 1
          ;;
      esac
    fi
    ;;
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
elif [ "$1 $2" = "tab close" ]; then
  if [ "$3" = "agent-tab" ]; then
    printf '%s\n' '{{"error":{{"code":"tab_close_failed","message":"cannot close the last tab in a workspace"}},"id":"test"}}' >&2
    exit 1
  fi
  if [ -e '{}' ]; then
    rm '{}'
    printf '%s\n' 'forced tab cleanup failure' >&2
    exit 1
  fi
  printf '%s\n' '{{"id":"test","result":{{}}}}'
elif [ "$1 $2" = "worktree remove" ]; then
  if [ -e '{}' ]; then
    rm '{}'
    printf '%s\n' 'forced worktree cleanup failure' >&2
    exit 1
  fi
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
        primary_credit_failure.display(),
        agent_path.display(),
        tab_close_failure.display(),
        tab_close_failure.display(),
        tab_close_failure.display(),
        tab_close_failure.display(),
        repo.path().display(),
        agent_path.display(),
        omp_permission = if global_yolo {
            "--auto-approve"
        } else {
            "--approval-mode always-ask"
        },
        omp_forbidden_permission = if global_yolo {
            "--approval-mode"
        } else {
            "--auto-approve"
        },
    );
    fs::write(&fake, script).unwrap();
    let mut permissions = fs::metadata(&fake).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake, permissions).unwrap();
    if force_primary_credit_failure {
        fs::write(&primary_credit_failure, "1\n").unwrap();
    }

    let start = cadence_command()
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
    assert_eq!(run["lead"]["harness"], lead_harness.as_str());
    assert_eq!(run["lead"]["model"], "openai/lead-model");
    assert_eq!(run["lead"]["reasoning_effort"], "high");
    let initial_run_id = active_run.to_string();
    let initial_lead_name = run["lead"]["name"].as_str().unwrap().to_string();
    if let Some((harness, model, reasoning_effort, yolo)) = relaunch_lead.as_ref() {
        config.lead.harness = *harness;
        config.lead.model = model.clone();
        config.lead.reasoning_effort = *reasoning_effort;
        config.yolo = *yolo;
        config.validate().unwrap();
        fs::write(&config_path, toml::to_string_pretty(&config).unwrap()).unwrap();
        git(repo.path(), &["add", ".cadence.toml"]);
        git(repo.path(), &["commit", "-m", "update Lead config"]);
    }
    let resumed = cadence_command()
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
    if relaunch_lead.is_some() {
        let focused_store: serde_json::Value =
            serde_json::from_slice(&fs::read(state.path().join("state.json")).unwrap()).unwrap();
        let focused_project = focused_store["projects"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap();
        let focused_run = &focused_project["runs"][&initial_run_id];
        assert_eq!(focused_run["lead"]["name"], initial_lead_name);
        assert_eq!(focused_run["lead"]["harness"], lead_harness.as_str());
        assert_eq!(focused_run["lead"]["model"], "openai/lead-model");
        assert_eq!(focused_run["lead"]["reasoning_effort"], "high");
    }

    let calls_before_relaunch = fs::read_to_string(&log).unwrap();
    fs::remove_file(&lead_started).unwrap();
    let restarted = cadence_command()
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
    if let Some((harness, model, reasoning_effort, yolo)) = relaunch_lead.as_ref() {
        let relaunch_calls = &calls[calls_before_relaunch.len()..];
        let model_arg = model
            .as_deref()
            .map(|model| format!(" --model {model}"))
            .unwrap_or_default();
        let reasoning = reasoning_effort.as_str();
        let model = model.as_deref().unwrap_or_default();
        let expected_launch = match harness {
            herdr_cadence::config::Harness::Claude => format!(
                "--kind claude --pane pane-lead --timeout 120000 -- --model {model} --effort {}",
                reasoning_effort.as_str().unwrap()
            ),
            herdr_cadence::config::Harness::Codex => format!(
                "--kind codex --pane pane-lead --timeout 120000 -- --model {model} --config model_reasoning_effort=\"{}\"",
                reasoning_effort.as_str().unwrap()
            ),
            herdr_cadence::config::Harness::Opencode => format!(
                "--kind opencode --pane pane-lead --timeout 120000 -- --model {model}#{}",
                reasoning_effort.as_str().unwrap()
            ),
            herdr_cadence::config::Harness::Pi => format!(
                "--kind pi --pane pane-lead --timeout 120000 -- --model {model} --thinking {}",
                reasoning_effort.as_str().unwrap()
            ),
            herdr_cadence::config::Harness::Omp => format!(
                "--kind omp --pane pane-lead --timeout 120000 --{model_arg}{} {}",
                reasoning
                    .map(|effort| format!(" --thinking {effort}"))
                    .unwrap_or_default(),
                if *yolo {
                    "--auto-approve"
                } else {
                    "--approval-mode always-ask"
                },
            ),
        };
        assert!(relaunch_calls.contains(&expected_launch));
        assert!(!relaunch_calls.contains("openai/lead-model"));
        if *harness == herdr_cadence::config::Harness::Omp {
            let launch = relaunch_calls
                .lines()
                .find(|line| line.starts_with("agent start cadence-lead-"))
                .unwrap();
            if model_arg.is_empty() {
                assert!(!launch.contains("--model"));
            }
            if reasoning.is_none() {
                assert!(!launch.contains("--thinking"));
            }
        }
        if *yolo {
            let yolo_arg = match harness {
                herdr_cadence::config::Harness::Claude => Some("--dangerously-skip-permissions"),
                herdr_cadence::config::Harness::Codex => {
                    Some("--dangerously-bypass-approvals-and-sandbox")
                }
                herdr_cadence::config::Harness::Opencode => Some("--auto"),
                // pi has no permission-bypass flag.
                herdr_cadence::config::Harness::Pi => None,
                herdr_cadence::config::Harness::Omp => Some("--auto-approve"),
            };
            if let Some(yolo_arg) = yolo_arg {
                assert!(relaunch_calls.contains(yolo_arg));
            }
        }
        if *harness != herdr_cadence::config::Harness::Codex {
            assert!(!relaunch_calls.contains("hooks.SessionStart"));
            assert!(!relaunch_calls.contains("hooks.state="));
        }
    }
    let store: serde_json::Value =
        serde_json::from_slice(&fs::read(state.path().join("state.json")).unwrap()).unwrap();
    let project = store["projects"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    let active_run = project["active_run"].as_str().unwrap();
    assert_eq!(active_run, initial_run_id);
    assert_eq!(
        project["runs"][active_run]["lead"]["name"],
        initial_lead_name
    );
    if let Some((harness, model, reasoning_effort, _)) = relaunch_lead.as_ref() {
        assert_eq!(
            project["runs"][active_run]["lead"]["harness"],
            harness.as_str()
        );
        assert_eq!(project["runs"][active_run]["lead"]["model"], json!(model));
        assert_eq!(
            project["runs"][active_run]["lead"]["reasoning_effort"],
            reasoning_effort.as_str().unwrap_or("default")
        );
    }
    let agent_yolo = relaunch_lead
        .as_ref()
        .map_or(global_yolo, |(_, _, _, yolo)| *yolo);
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
        let blocked = cadence_command()
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
    let spawn = cadence_command()
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
    assert_eq!(
        value["runner"],
        if force_primary_credit_failure {
            "qa-backup"
        } else {
            "qa-primary"
        }
    );
    assert_eq!(
        value["model"],
        if force_primary_credit_failure {
            json!("backup-model")
        } else if worker_native_defaults {
            serde_json::Value::Null
        } else {
            json!("qa-model")
        }
    );
    let selected_harness = if force_primary_credit_failure {
        if worker_harness == herdr_cadence::config::Harness::Omp {
            herdr_cadence::config::Harness::Codex
        } else {
            herdr_cadence::config::Harness::Opencode
        }
    } else {
        worker_harness
    };
    assert_eq!(value["harness"], selected_harness.as_str());
    assert_eq!(
        value["reasoning_effort"],
        if force_primary_credit_failure {
            "high"
        } else if worker_native_defaults {
            "default"
        } else {
            "low"
        }
    );
    let agent_state = cadence(repo.path(), state.path(), &["agent", "status", "agent-1"]);
    assert!(agent_state.status.success());
    let agent_state: serde_json::Value = serde_json::from_slice(&agent_state.stdout).unwrap();
    assert_eq!(agent_state["harness"], value["harness"]);
    assert_eq!(agent_state["model"], value["model"]);
    assert_eq!(agent_state["reasoning_effort"], value["reasoning_effort"]);
    assert_eq!(agent_state["yolo"], agent_yolo);
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
    match lead_harness {
        herdr_cadence::config::Harness::Codex => {
            let lead_launch = "--kind codex --pane pane-lead --timeout 120000 -- --model openai/lead-model --config model_reasoning_effort=\"high\" --config hooks.SessionStart=[{matcher=\"^compact$\",hooks=[{type=\"command\",command=\"'";
            assert!(calls.contains(lead_launch));
            assert!(calls.contains(
                "--config hooks.state={\"/<session-flags>/config.toml:session_start:0:0\"={trusted_hash=\"sha256:"
            ));
            assert!(!calls.contains("dangerously-bypass-hook-trust"));
            assert!(calls.contains("agent prompt cadence-lead-"));
            assert!(!calls.contains("developer_instructions="));
        }
        herdr_cadence::config::Harness::Claude => {
            let lead_launch = "--kind claude --pane pane-lead --timeout 120000 -- --model openai/lead-model --effort high";
            assert!(calls.contains(lead_launch));
            assert!(calls.contains("agent prompt cadence-lead-"));
            assert!(!calls.contains("developer_instructions="));
        }
        herdr_cadence::config::Harness::Opencode => {
            let lead_launch = "--kind opencode --pane pane-lead --timeout 120000 -- --model openai/lead-model#high";
            assert!(calls.contains(lead_launch));
            assert!(calls.contains("agent prompt cadence-lead-"));
            assert!(!calls.contains("developer_instructions="));
            if global_yolo {
                assert!(calls.contains(&format!("{lead_launch} --auto")));
            } else {
                assert!(!calls.contains(&format!("{lead_launch} --auto")));
            }
        }
        herdr_cadence::config::Harness::Pi => {
            let lead_launch = "--kind pi --pane pane-lead --timeout 120000 -- --model openai/lead-model --thinking high";
            assert!(calls.contains(lead_launch));
            assert!(calls.contains("agent prompt cadence-lead-"));
            assert!(!calls.contains("developer_instructions="));
        }
        herdr_cadence::config::Harness::Omp => {
            let permission = if global_yolo {
                "--auto-approve"
            } else {
                "--approval-mode always-ask"
            };
            let lead_launch = format!(
                "--kind omp --pane pane-lead --timeout 120000 -- --model openai/lead-model --thinking high {permission}"
            );
            assert!(calls.contains(&lead_launch));
            assert!(calls.contains("agent prompt cadence-lead-"));
        }
    }
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
    } else {
        assert!(!calls.contains("worktree create --cwd"));
        assert_eq!(calls.matches("tab create --workspace base-ws").count(), 1);
        assert_eq!(
            calls.matches("tab create --workspace resumed-ws").count(),
            2
        );
        assert!(calls.contains("--label [QA] Add API --no-focus"));
    }
    assert!(calls.contains("agent start cadence-"));
    let run_digest = <sha2::Sha256 as sha2::Digest>::digest(active_run.as_bytes());
    let run_key: String = run_digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert!(calls.contains(&format!("agent start cad-{run_key}-a1")));
    let agent_launch = if worker_harness == herdr_cadence::config::Harness::Omp {
        if worker_native_defaults {
            "--kind omp --pane pane-agent --timeout 120000 --"
        } else {
            "--kind omp --pane pane-agent --timeout 120000 -- --model qa-model --thinking low"
        }
    } else {
        "--kind codex --pane pane-agent --timeout 120000 -- --model qa-model --config model_reasoning_effort=\"low\""
    };
    assert!(calls.contains(agent_launch));
    let selected_launch = if force_primary_credit_failure {
        if selected_harness == herdr_cadence::config::Harness::Codex {
            "--kind codex --pane pane-agent --timeout 120000 -- --model backup-model --config model_reasoning_effort=\"high\""
        } else {
            "--kind opencode --pane pane-agent --timeout 120000 -- --model backup-model#high"
        }
    } else {
        agent_launch
    };
    assert!(calls.contains(selected_launch));
    if selected_harness == herdr_cadence::config::Harness::Omp {
        let permission = if agent_yolo {
            "--auto-approve"
        } else {
            "--approval-mode always-ask"
        };
        assert!(calls.contains(&format!("{selected_launch} {permission}")));
        let launch = calls
            .lines()
            .find(|line| line.starts_with(&format!("agent start cad-{run_key}-a1 ")))
            .unwrap();
        assert!(!launch.contains("--sandbox"));
        assert!(!launch.contains("--add-dir"));
        if worker_native_defaults {
            assert!(!launch.contains("--model"));
            assert!(!launch.contains("--thinking"));
        }
    } else if selected_harness == herdr_cadence::config::Harness::Codex && agent_yolo {
        assert!(calls.contains(&format!(
            "{selected_launch} --dangerously-bypass-approvals-and-sandbox"
        )));
    } else if selected_harness == herdr_cadence::config::Harness::Codex && use_worktree {
        assert!(calls.contains(&format!(
            "{selected_launch} --sandbox workspace-write --ask-for-approval never --add-dir {}",
            state.path().display()
        )));
    } else if selected_harness == herdr_cadence::config::Harness::Codex {
        assert!(calls.contains(&format!(
            "{selected_launch} --add-dir {}",
            state.path().display()
        )));
        assert!(!calls.contains("--ask-for-approval never"));
        assert!(!calls.contains("--dangerously-bypass-approvals-and-sandbox"));
    }
    assert!(calls.contains("agent prompt"));

    let early_follow_up = if !use_worktree && !create_out_of_scope_commit {
        fs::write(
            &request,
            r#"{"title":"Update docs","task":"Update the docs","scope":["docs"],"acceptance":["Docs are current"],"role":"researcher"}"#,
        )
        .unwrap();
        let follow_up = cadence_command()
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
    let complete = cadence_command()
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "agent",
            "complete",
            "agent-1",
            "--run-id",
            active_run,
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
    if worker_harness == herdr_cadence::config::Harness::Omp {
        let agent_report = cadence(repo.path(), state.path(), &["agent", "report", "agent-1"]);
        assert!(agent_report.status.success());
        let agent_report: serde_json::Value = serde_json::from_slice(&agent_report.stdout).unwrap();
        assert_eq!(agent_report["status"], completed["status"]);
        assert_eq!(agent_report["report"]["summary"], "Added API");
        assert_eq!(agent_report["report"]["commit_sha"], commit_sha);
        assert_eq!(agent_report["report"]["changed_paths"][0], "src/api/mod.rs");
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(state.path().join("state.json")).unwrap()).unwrap();
        let project = stored["projects"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap();
        let agent = &project["runs"][active_run]["agents"]["agent-1"];
        assert_eq!(agent["harness"], selected_harness.as_str());
        assert_eq!(agent["model"], value["model"]);
        assert_eq!(agent["reasoning_effort"], value["reasoning_effort"]);
        assert_eq!(agent["yolo"], agent_yolo);
        assert_eq!(agent["report"]["commit_sha"], commit_sha);
    }
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
        let overlapping_spawn = cadence_command()
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
        if force_tab_cleanup_retry {
            let deferred = cadence_command()
                .args([
                    "--state-dir",
                    state.path().to_str().unwrap(),
                    "--project-root",
                    repo.path().to_str().unwrap(),
                    "event",
                ])
                .env("HERDR_BIN_PATH", &fake)
                .env("HERDR_PLUGIN_EVENT", "pane.agent_status_changed")
                .env(
                    "HERDR_PLUGIN_EVENT_JSON",
                    r#"{"event":"pane.agent_status_changed","data":{"pane_id":"pane-lead","agent_status":"idle"}}"#,
                )
                .output()
                .unwrap();
            assert!(deferred.status.success());
            let deferred: serde_json::Value = serde_json::from_slice(&deferred.stdout).unwrap();
            assert_eq!(deferred["cleanup_deferred"], true);
            fs::write(&tab_close_failure, "1\n").unwrap();
        }
        let integrate = cadence_command()
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
        if force_tab_cleanup_retry {
            assert!(integrate.status.success());
            let integration: serde_json::Value = serde_json::from_slice(&integrate.stdout).unwrap();
            assert_eq!(integration["status"], "integrated");
            assert!(
                integration["cleanup_warning"]
                    .as_str()
                    .unwrap()
                    .contains("forced worktree cleanup failure")
            );
            let retained = cadence(repo.path(), state.path(), &["agent", "status", "agent-1"]);
            assert!(retained.status.success());
            let retained: serde_json::Value = serde_json::from_slice(&retained.stdout).unwrap();
            assert_eq!(retained["status"], "integrated");
            assert_eq!(retained["tab_id"], "agent-tab");
            assert_eq!(retained["workspace_id"], "agent-ws");
            assert_eq!(retained["cleanup_attempts"], 1);

            let idle = cadence_command()
                .args([
                    "--state-dir",
                    state.path().to_str().unwrap(),
                    "--project-root",
                    repo.path().to_str().unwrap(),
                    "event",
                ])
                .env("HERDR_BIN_PATH", &fake)
                .env("HERDR_PLUGIN_EVENT", "pane.agent_status_changed")
                .env(
                    "HERDR_PLUGIN_EVENT_JSON",
                    r#"{"event":"pane.agent_status_changed","data":{"pane_id":"pane-lead","agent_status":"idle"}}"#,
                )
                .output()
                .unwrap();
            assert!(
                idle.status.success(),
                "{}",
                String::from_utf8_lossy(&idle.stderr)
            );
            let idle: serde_json::Value = serde_json::from_slice(&idle.stdout).unwrap();
            assert_eq!(idle["lead"], true);
            assert_eq!(idle["reconciled"], 1);
            assert!(idle["cleanup_warnings"].as_array().unwrap().is_empty());

            let cleaned = cadence(repo.path(), state.path(), &["agent", "status", "agent-1"]);
            completed = serde_json::from_slice(&cleaned.stdout).unwrap();
            assert!(completed["tab_id"].is_null());
            assert!(completed["workspace_id"].is_null());
            assert!(completed["checkout_path"].is_null());

            let state_path = state.path().join("state.json");
            let mut limited: serde_json::Value =
                serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
            let project = limited["projects"]
                .as_object_mut()
                .unwrap()
                .values_mut()
                .next()
                .unwrap();
            let agent = &mut project["runs"][&active_run]["agents"]["agent-1"];
            agent["tab_id"] = "retry-limit-tab".into();
            agent["cleanup_attempts"] = 1.into();
            fs::write(&state_path, serde_json::to_vec_pretty(&limited).unwrap()).unwrap();
            fs::write(&tab_close_failure, "1\n").unwrap();

            let exhausted = cadence_command()
                .args([
                    "--state-dir",
                    state.path().to_str().unwrap(),
                    "--project-root",
                    repo.path().to_str().unwrap(),
                    "event",
                ])
                .env("HERDR_BIN_PATH", &fake)
                .env("HERDR_PLUGIN_EVENT", "pane.agent_status_changed")
                .env(
                    "HERDR_PLUGIN_EVENT_JSON",
                    r#"{"event":"pane.agent_status_changed","data":{"pane_id":"pane-lead","agent_status":"idle"}}"#,
                )
                .output()
                .unwrap();
            assert!(exhausted.status.success());
            let exhausted: serde_json::Value = serde_json::from_slice(&exhausted.stdout).unwrap();
            assert_eq!(exhausted["reconciled"], 0);
            assert_eq!(exhausted["cleanup_warnings"].as_array().unwrap().len(), 1);
            assert!(
                exhausted["cleanup_warnings"][0]
                    .as_str()
                    .unwrap()
                    .contains("manual cleanup is required")
            );

            let exhausted_status =
                cadence(repo.path(), state.path(), &["agent", "status", "agent-1"]);
            let exhausted_status: serde_json::Value =
                serde_json::from_slice(&exhausted_status.stdout).unwrap();
            assert_eq!(exhausted_status["cleanup_attempts"], 2);

            let no_third_attempt = cadence_command()
                .args([
                    "--state-dir",
                    state.path().to_str().unwrap(),
                    "--project-root",
                    repo.path().to_str().unwrap(),
                    "event",
                ])
                .env("HERDR_BIN_PATH", &fake)
                .env("HERDR_PLUGIN_EVENT", "pane.agent_status_changed")
                .env(
                    "HERDR_PLUGIN_EVENT_JSON",
                    r#"{"event":"pane.agent_status_changed","data":{"pane_id":"pane-lead","agent_status":"idle"}}"#,
                )
                .output()
                .unwrap();
            assert!(no_third_attempt.status.success());
            assert_eq!(
                fs::read_to_string(&log)
                    .unwrap()
                    .matches("tab close retry-limit-tab")
                    .count(),
                1
            );
        } else {
            assert!(
                integrate.status.success(),
                "{}",
                String::from_utf8_lossy(&integrate.stderr)
            );
            completed = serde_json::from_slice(&integrate.stdout).unwrap();
        }
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
        let follow_up = cadence_command()
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
    if worker_harness == herdr_cadence::config::Harness::Omp {
        assert_eq!(follow_up["harness"], "codex");
        assert_eq!(follow_up["model"], "gpt-5.6-terra");
        assert_eq!(follow_up["reasoning_effort"], "high");
        let calls = fs::read_to_string(&log).unwrap();
        let launch = calls
            .lines()
            .find(|line| line.starts_with(&format!("agent start cad-{run_key}-a2 ")))
            .unwrap();
        assert!(launch.contains("--kind codex"));
        assert_eq!(
            launch.contains("--dangerously-bypass-approvals-and-sandbox"),
            agent_yolo
        );
        assert!(!launch.contains("--approval-mode"));
        assert!(!launch.contains("--auto-approve"));
    }

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
    let research_complete = cadence_command()
        .args([
            "--state-dir",
            state.path().to_str().unwrap(),
            "--project-root",
            repo.path().to_str().unwrap(),
            "agent",
            "complete",
            "agent-2",
            "--run-id",
            active_run,
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
    assert!(calls.contains(&format!("agent send-keys cad-{run_key}-a")));
    assert!(calls.contains("ctrl+c"));
    let completion_notification = calls.rfind("agent prompt cadence-lead-").unwrap();
    let agent_interrupt = calls
        .rfind(&format!("agent send-keys cad-{run_key}-a"))
        .unwrap();
    assert!(
        completion_notification < agent_interrupt,
        "Lead completion notification must precede interrupting the completing agent"
    );
    if use_worktree {
        let worktree_remove = calls
            .rfind("worktree remove --workspace agent-ws --force")
            .unwrap();
        assert!(
            !calls[..worktree_remove].contains("tab close agent-tab"),
            "Worktree cleanup must remove its workspace without closing the workspace's last tab"
        );
        assert!(!calls.contains("tab close agent-tab"));
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
        let blocked_finish = cadence_command()
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
    let mut finish_command = cadence_command();
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

        let restart = cadence_command()
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
    let output = cadence_command()
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

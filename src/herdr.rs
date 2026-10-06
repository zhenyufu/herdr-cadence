use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::config::{Harness, ReasoningEffort};

const SHELL_READY_ATTEMPTS: usize = 100;
const SHELL_READY_RETRY_DELAY: Duration = Duration::from_millis(100);
const AGENT_PANE_BUSY_RETRY_DELAYS: [Duration; 8] = [
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(400),
    Duration::from_millis(800),
    Duration::from_secs(1),
    Duration::from_secs(1),
    Duration::from_secs(1),
];

#[derive(Debug, Clone)]
pub struct Herdr {
    binary: String,
}

#[derive(Debug, Clone)]
pub struct CreatedTerminal {
    pub workspace_id: Option<String>,
    pub tab_id: String,
    pub pane_id: String,
    pub checkout_path: Option<String>,
}

const CODEX_SESSION_HOOK_KEY: &str = "/<session-flags>/config.toml:session_start:0:0";
const CODEX_SESSION_HOOK_MATCHER: &str = "^compact$";

impl Herdr {
    pub fn from_env() -> Self {
        Self {
            binary: std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into()),
        }
    }

    fn output<I, S>(&self, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Command::new(&self.binary)
            .args(args)
            .output()
            .with_context(|| format!("failed to run {}", self.binary))
    }

    fn checked<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.output(args)?;
        if !output.status.success() {
            bail!(
                "Herdr command failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn agent_exists(&self, name: &str) -> Result<bool> {
        let output = self.output(["agent", "get", name])?;
        resource_exists(&output, "agent_not_found")
    }

    pub fn agent_tab_id(&self, name: &str) -> Result<Option<String>> {
        let output = self.output(["agent", "get", name])?;
        if !output.status.success() {
            if has_error_code(&output, "agent_not_found") {
                return Ok(None);
            }
            return Err(command_error(&output));
        }
        let value: Value =
            serde_json::from_slice(&output.stdout).context("Herdr returned invalid agent JSON")?;
        Ok(value
            .pointer("/result/agent/tab_id")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    pub fn workspace_exists(&self, workspace_id: &str) -> Result<bool> {
        let output = self.output(["workspace", "get", workspace_id])?;
        resource_exists(&output, "workspace_not_found")
    }

    pub fn focus_agent(&self, name: &str) -> Result<()> {
        self.checked(["agent", "focus", name])?;
        Ok(())
    }

    pub fn prompt_agent(&self, name: &str, prompt: &str) -> Result<()> {
        self.checked(["agent", "prompt", name, prompt])?;
        Ok(())
    }

    pub fn show_notification(&self, title: &str, body: &str) -> Result<()> {
        self.checked(["notification", "show", title, "--body", body])?;
        Ok(())
    }

    pub fn send_ctrl_c(&self, name: &str) -> Result<()> {
        self.checked(["agent", "send-keys", name, "ctrl+c"])?;
        Ok(())
    }

    pub fn create_lead_tab(
        &self,
        workspace_id: &str,
        root: &Path,
        env: &[(&str, String)],
    ) -> Result<CreatedTerminal> {
        let label = lead_label(root);
        let mut args = vec![
            "tab".to_string(),
            "create".into(),
            "--workspace".into(),
            workspace_id.into(),
            "--cwd".into(),
            root.display().to_string(),
            "--label".into(),
            label,
            "--focus".into(),
        ];
        for (key, value) in env {
            args.push("--env".into());
            args.push(format!("{key}={value}"));
        }
        let mut terminal = parse_created(&self.checked(args)?)?;
        terminal.workspace_id = Some(workspace_id.to_string());
        Ok(terminal)
    }

    pub fn create_agent_worktree(
        &self,
        root: &Path,
        branch: &str,
        base: &str,
        label: &str,
    ) -> Result<CreatedTerminal> {
        let args = vec![
            "worktree".to_string(),
            "create".into(),
            "--cwd".into(),
            root.display().to_string(),
            "--branch".into(),
            branch.into(),
            "--base".into(),
            base.into(),
            "--label".into(),
            label.into(),
            "--no-focus".into(),
            "--json".into(),
        ];
        parse_created(&self.checked(args)?)
    }

    pub fn create_agent_tab(
        &self,
        workspace_id: &str,
        root: &Path,
        label: &str,
    ) -> Result<CreatedTerminal> {
        let args = vec![
            "tab".to_string(),
            "create".into(),
            "--workspace".into(),
            workspace_id.into(),
            "--cwd".into(),
            root.display().to_string(),
            "--label".into(),
            label.into(),
            "--no-focus".into(),
        ];
        let mut terminal = parse_created(&self.checked(args)?)?;
        terminal.workspace_id = None;
        terminal.checkout_path = Some(root.display().to_string());
        Ok(terminal)
    }

    pub fn start_agent(
        &self,
        name: &str,
        harness: Harness,
        pane_id: &str,
        model: Option<&str>,
        reasoning_effort: ReasoningEffort,
        agent_args: &[String],
    ) -> Result<()> {
        self.start_agent_with_options(name, harness, pane_id, model, reasoning_effort, agent_args)
    }

    pub fn start_codex_lead(
        &self,
        name: &str,
        pane_id: &str,
        model: Option<&str>,
        reasoning_effort: ReasoningEffort,
        agent_args: &[String],
        cadence_binary: &Path,
    ) -> Result<()> {
        let mut args = codex_session_hook_args(cadence_binary)?;
        args.extend(agent_args.iter().cloned());
        self.start_agent_with_options(
            name,
            Harness::Codex,
            pane_id,
            model,
            reasoning_effort,
            &args,
        )
    }

    fn start_agent_with_options(
        &self,
        name: &str,
        harness: Harness,
        pane_id: &str,
        model: Option<&str>,
        reasoning_effort: ReasoningEffort,
        agent_args: &[String],
    ) -> Result<()> {
        self.wait_for_available_shell(pane_id)?;
        let args = start_agent_args(name, harness, pane_id, model, reasoning_effort, agent_args)?;
        for delay in AGENT_PANE_BUSY_RETRY_DELAYS {
            let output = self.output(&args)?;
            if output.status.success() {
                return Ok(());
            }
            if !has_error_code(&output, "agent_pane_busy") {
                return Err(command_error(&output));
            }
            thread::sleep(delay);
        }
        let output = self.output(args)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(command_error(&output))
        }
    }

    fn wait_for_available_shell(&self, pane_id: &str) -> Result<()> {
        for attempt in 0..SHELL_READY_ATTEMPTS {
            let raw = self.checked(["pane", "process-info", "--pane", pane_id])?;
            if shell_is_foreground(&raw)? {
                return Ok(());
            }
            if attempt + 1 < SHELL_READY_ATTEMPTS {
                thread::sleep(SHELL_READY_RETRY_DELAY);
            }
        }
        bail!("Herdr pane {pane_id} did not become an available shell")
    }

    pub fn remove_worktree(&self, workspace_id: &str) -> Result<()> {
        self.checked(["worktree", "remove", "--workspace", workspace_id, "--force"])?;
        Ok(())
    }

    pub fn close_tab(&self, tab_id: &str) -> Result<()> {
        self.checked(["tab", "close", tab_id])?;
        Ok(())
    }

    pub fn tab_exists(&self, tab_id: &str) -> Result<bool> {
        let output = self.output(["tab", "get", tab_id])?;
        if output.status.success() {
            Ok(true)
        } else if has_error_code(&output, "tab_not_found") {
            Ok(false)
        } else {
            Err(command_error(&output))
        }
    }
}

fn launch_model(
    harness: Harness,
    model: Option<&str>,
    reasoning_effort: ReasoningEffort,
) -> Result<Option<String>> {
    let Some(reasoning_effort) = reasoning_effort.as_str() else {
        return Ok(model.map(str::to_string));
    };
    if harness != Harness::Opencode {
        return Ok(model.map(str::to_string));
    }
    let model = model.context(
        "OpenCode reasoning_effort requires an explicit model so Cadence can select its variant",
    )?;
    let model = model.split_once('#').map_or(model, |(model, _)| model);
    Ok(Some(format!("{model}#{reasoning_effort}")))
}

fn resource_exists(output: &Output, not_found: &str) -> Result<bool> {
    if output.status.success() {
        Ok(true)
    } else if has_error_code(output, not_found) {
        Ok(false)
    } else {
        Err(command_error(output))
    }
}

fn start_agent_args(
    name: &str,
    harness: Harness,
    pane_id: &str,
    model: Option<&str>,
    reasoning_effort: ReasoningEffort,
    agent_args: &[String],
) -> Result<Vec<String>> {
    let model = launch_model(harness, model, reasoning_effort)?;
    let mut args = vec![
        "agent".to_string(),
        "start".into(),
        name.into(),
        "--kind".into(),
        harness.as_str().into(),
        "--pane".into(),
        pane_id.into(),
        "--timeout".into(),
        "120000".into(),
    ];
    if model.is_some() || reasoning_effort.as_str().is_some() || !agent_args.is_empty() {
        args.push("--".into());
    }
    if let Some(model) = &model {
        args.extend(["--model".into(), model.clone()]);
    }
    if let Some(reasoning_effort) = reasoning_effort.as_str() {
        match harness {
            Harness::Claude => args.extend(["--effort".into(), reasoning_effort.into()]),
            Harness::Codex => args.extend([
                "--config".into(),
                format!("model_reasoning_effort=\"{reasoning_effort}\""),
            ]),
            Harness::Opencode => {}
            Harness::Pi | Harness::Omp => {
                args.extend(["--thinking".into(), reasoning_effort.into()])
            }
        }
    }
    args.extend(agent_args.iter().cloned());
    Ok(args)
}

fn codex_session_hook_args(cadence_binary: &Path) -> Result<Vec<String>> {
    let command = format!("{} hook codex-session-start", shell_quote(cadence_binary));
    let command_literal = serde_json::to_string(&command)
        .context("failed to encode Codex SessionStart hook command")?;
    let trust_hash = codex_session_hook_hash(&command);
    let hook_config = format!(
        "hooks.SessionStart=[{{matcher=\"{CODEX_SESSION_HOOK_MATCHER}\",hooks=[{{type=\"command\",command={command_literal},async=false,timeout=600}}]}}]"
    );
    let state_config =
        format!("hooks.state={{\"{CODEX_SESSION_HOOK_KEY}\"={{trusted_hash=\"{trust_hash}\"}}}}");
    Ok(vec![
        "--config".into(),
        hook_config,
        "--config".into(),
        state_config,
    ])
}

fn codex_session_hook_hash(command: &str) -> String {
    let identity = json!({
        "event_name": "session_start",
        "hooks": [{
            "async": false,
            "command": command,
            "timeout": 600,
            "type": "command",
        }],
        "matcher": CODEX_SESSION_HOOK_MATCHER,
    });
    let canonical = canonical_json(&identity);
    let serialized = serde_json::to_vec(&canonical).expect("hook identity is serializable");
    let digest = Sha256::digest(serialized);
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut canonical = serde_json::Map::new();
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                canonical.insert(key.clone(), canonical_json(&map[&key]));
            }
            Value::Object(canonical)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

fn shell_quote(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn lead_label(root: &Path) -> String {
    let project = root
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("project");
    format!("[Lead] {project}")
}

fn command_error(output: &Output) -> anyhow::Error {
    anyhow::anyhow!(
        "Herdr command failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn has_error_code(output: &Output, expected: &str) -> bool {
    serde_json::from_slice::<Value>(&output.stderr)
        .is_ok_and(|value| value.pointer("/error/code").and_then(Value::as_str) == Some(expected))
}

fn shell_is_foreground(raw: &str) -> Result<bool> {
    let value: Value = serde_json::from_str(raw).context("Herdr returned invalid JSON")?;
    let process = value
        .pointer("/result/process_info")
        .or_else(|| value.get("process_info"))
        .context("Herdr response omitted result.process_info")?;
    let shell_pid = process.get("shell_pid").and_then(Value::as_u64);
    let foreground_process_group_id = process
        .get("foreground_process_group_id")
        .and_then(Value::as_u64);
    Ok(shell_pid.is_some() && shell_pid == foreground_process_group_id)
}

fn parse_created(raw: &str) -> Result<CreatedTerminal> {
    let value: Value = serde_json::from_str(raw).context("Herdr returned invalid JSON")?;
    if let Some(error) = value.get("error") {
        bail!("Herdr returned an error: {error}");
    }
    let result = value.get("result").unwrap_or(&value);
    let workspace_id = result
        .pointer("/workspace/workspace_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let tab_id = result
        .pointer("/tab/tab_id")
        .and_then(Value::as_str)
        .context("Herdr response omitted tab.tab_id")?
        .to_string();
    let pane_id = result
        .pointer("/root_pane/pane_id")
        .and_then(Value::as_str)
        .context("Herdr response omitted root_pane.pane_id")?
        .to_string();
    let checkout_path = result
        .pointer("/worktree/path")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            result
                .pointer("/workspace/worktree/checkout_path")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    Ok(CreatedTerminal {
        workspace_id,
        tab_id,
        pane_id,
        checkout_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_created_response() {
        let parsed = parse_created(
            r#"{"id":"1","result":{"type":"worktree_created","workspace":{"workspace_id":"w1","worktree":{"checkout_path":"/tmp/w"}},"tab":{"tab_id":"t1"},"root_pane":{"pane_id":"p1"},"worktree":{"path":"/tmp/w"}}}"#,
        )
        .unwrap();
        assert_eq!(parsed.workspace_id.as_deref(), Some("w1"));
        assert_eq!(parsed.pane_id, "p1");
        assert_eq!(parsed.checkout_path.as_deref(), Some("/tmp/w"));
    }

    #[test]
    fn labels_the_lead_as_project_lead() {
        assert_eq!(
            lead_label(Path::new("/tmp/example-project")),
            "[Lead] example-project"
        );
    }

    #[test]
    fn maps_opencode_reasoning_to_model_variant() {
        assert_eq!(
            launch_model(
                Harness::Opencode,
                Some("openai/gpt-5.2#low"),
                ReasoningEffort::Xhigh,
            )
            .unwrap()
            .as_deref(),
            Some("openai/gpt-5.2#xhigh")
        );
        assert!(launch_model(Harness::Opencode, None, ReasoningEffort::High).is_err());
    }

    #[test]
    fn maps_claude_model_and_reasoning_to_native_flags() {
        let args = start_agent_args(
            "reviewer",
            Harness::Claude,
            "pane-1",
            Some("opus"),
            ReasoningEffort::High,
            &["--dangerously-skip-permissions".into()],
        )
        .unwrap();

        assert_eq!(
            args,
            [
                "agent",
                "start",
                "reviewer",
                "--kind",
                "claude",
                "--pane",
                "pane-1",
                "--timeout",
                "120000",
                "--",
                "--model",
                "opus",
                "--effort",
                "high",
                "--dangerously-skip-permissions",
            ]
        );
    }

    #[test]
    fn passes_pi_and_omp_models_with_an_explicit_thinking_override() {
        for (harness, kind) in [(Harness::Pi, "pi"), (Harness::Omp, "omp")] {
            let args = start_agent_args(
                "researcher",
                harness,
                "pane-1",
                Some("zai/glm-5.3-flash:low"),
                ReasoningEffort::High,
                &[],
            )
            .unwrap();

            assert_eq!(
                args,
                [
                    "agent",
                    "start",
                    "researcher",
                    "--kind",
                    kind,
                    "--pane",
                    "pane-1",
                    "--timeout",
                    "120000",
                    "--",
                    "--model",
                    "zai/glm-5.3-flash:low",
                    "--thinking",
                    "high",
                ]
            );
        }
    }

    #[test]
    fn preserves_pi_and_omp_model_thinking_suffix_without_an_explicit_effort() {
        for (harness, kind) in [(Harness::Pi, "pi"), (Harness::Omp, "omp")] {
            let args = start_agent_args(
                "researcher",
                harness,
                "pane-1",
                Some("zai/glm-5.3-flash:high"),
                ReasoningEffort::Default,
                &[],
            )
            .unwrap();

            assert_eq!(
                args,
                [
                    "agent",
                    "start",
                    "researcher",
                    "--kind",
                    kind,
                    "--pane",
                    "pane-1",
                    "--timeout",
                    "120000",
                    "--",
                    "--model",
                    "zai/glm-5.3-flash:high",
                ]
            );
        }
    }

    #[test]
    fn passes_pi_and_omp_thinking_levels_without_a_model() {
        for (harness, kind) in [(Harness::Pi, "pi"), (Harness::Omp, "omp")] {
            let args = start_agent_args(
                "researcher",
                harness,
                "pane-1",
                None,
                ReasoningEffort::Medium,
                &[],
            )
            .unwrap();

            assert_eq!(
                args,
                [
                    "agent",
                    "start",
                    "researcher",
                    "--kind",
                    kind,
                    "--pane",
                    "pane-1",
                    "--timeout",
                    "120000",
                    "--",
                    "--thinking",
                    "medium",
                ]
            );
        }
    }

    #[test]
    fn generates_a_quoted_compact_hook_and_exact_trust_config() {
        let args = codex_session_hook_args(Path::new("/tmp/Cadence Agent's/bin")).unwrap();
        assert_eq!(args.len(), 4);
        assert_eq!(args[0], "--config");
        let hook_value = args[1].split_once('=').unwrap().1;
        let hook_table: toml::Value = toml::from_str(&format!("value = {hook_value}"))
            .expect("generated hook config should be valid TOML");
        assert_eq!(
            hook_table["value"][0]["matcher"].as_str(),
            Some("^compact$")
        );
        assert_eq!(
            hook_table["value"][0]["hooks"][0]["command"].as_str(),
            Some("'/tmp/Cadence Agent'\\''s/bin' hook codex-session-start")
        );
        assert_eq!(
            hook_table["value"][0]["hooks"][0]["async"].as_bool(),
            Some(false)
        );
        assert_eq!(
            hook_table["value"][0]["hooks"][0]["timeout"].as_integer(),
            Some(600)
        );
        assert_eq!(args[2], "--config");
        assert!(args[3].starts_with(
            "hooks.state={\"/<session-flags>/config.toml:session_start:0:0\"={trusted_hash=\"sha256:"
        ));
        assert!(args[3].ends_with("\"}}"));
        assert!(
            !args
                .iter()
                .any(|arg| arg.contains("dangerously-bypass-hook-trust"))
        );
    }

    #[test]
    fn uses_codex_normalized_hook_hash() {
        assert_eq!(
            codex_session_hook_hash("true"),
            "sha256:7c0553aa6deec045741238c315c995e8222b35db4b5964ca839118324e0a2765"
        );
    }
}

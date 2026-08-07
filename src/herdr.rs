use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

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

    pub fn agent_exists(&self, name: &str) -> bool {
        self.output(["agent", "get", name])
            .is_ok_and(|output| output.status.success())
    }

    pub fn workspace_exists(&self, workspace_id: &str) -> bool {
        self.output(["workspace", "get", workspace_id])
            .is_ok_and(|output| output.status.success())
    }

    pub fn focus_agent(&self, name: &str) -> Result<()> {
        self.checked(["agent", "focus", name])?;
        Ok(())
    }

    pub fn prompt_agent(&self, name: &str, prompt: &str) -> Result<()> {
        self.checked(["agent", "prompt", name, prompt])?;
        Ok(())
    }

    pub fn send_ctrl_c(&self, name: &str) -> Result<()> {
        self.checked(["agent", "send-keys", name, "ctrl+c"])?;
        Ok(())
    }

    pub fn create_orchestrator_tab(
        &self,
        workspace_id: &str,
        root: &Path,
        env: &[(&str, String)],
    ) -> Result<CreatedTerminal> {
        let label = orchestrator_label(root);
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

    pub fn create_orchestrator_workspace(
        &self,
        root: &Path,
        env: &[(&str, String)],
    ) -> Result<CreatedTerminal> {
        let label = orchestrator_label(root);
        let mut args = vec![
            "workspace".to_string(),
            "create".into(),
            "--cwd".into(),
            root.display().to_string(),
            "--label".into(),
            label.clone(),
            "--focus".into(),
        ];
        for (key, value) in env {
            args.push("--env".into());
            args.push(format!("{key}={value}"));
        }
        let terminal = parse_created(&self.checked(args)?)?;
        self.checked(["tab", "rename", &terminal.tab_id, &label])?;
        Ok(terminal)
    }

    pub fn create_worker_worktree(
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

    pub fn create_worker_tab(
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
        self.wait_for_available_shell(pane_id)?;
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
        if harness == Harness::Codex
            && let Some(reasoning_effort) = reasoning_effort.as_str()
        {
            args.extend([
                "--config".into(),
                format!("model_reasoning_effort=\"{reasoning_effort}\""),
            ]);
        }
        args.extend(agent_args.iter().cloned());
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
}

fn launch_model(
    harness: Harness,
    model: Option<&str>,
    reasoning_effort: ReasoningEffort,
) -> Result<Option<String>> {
    let Some(reasoning_effort) = reasoning_effort.as_str() else {
        return Ok(model.map(str::to_string));
    };
    if harness == Harness::Codex {
        return Ok(model.map(str::to_string));
    }
    let model = model.context(
        "OpenCode reasoning_effort requires an explicit model so Cadence can select its variant",
    )?;
    let model = model.split_once('#').map_or(model, |(model, _)| model);
    Ok(Some(format!("{model}#{reasoning_effort}")))
}

fn orchestrator_label(root: &Path) -> String {
    let project = root
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("project");
    format!("[Cadence]{project}")
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
}

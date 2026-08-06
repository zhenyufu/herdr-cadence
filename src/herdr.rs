use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
use serde_json::Value;

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
        let mut args = vec![
            "tab".to_string(),
            "create".into(),
            "--workspace".into(),
            workspace_id.into(),
            "--cwd".into(),
            root.display().to_string(),
            "--label".into(),
            "Cadence Orchestrator".into(),
            "--focus".into(),
        ];
        for (key, value) in env {
            args.push("--env".into());
            args.push(format!("{key}={value}"));
        }
        parse_created(&self.checked(args)?)
    }

    pub fn create_worker_worktree(
        &self,
        workspace_id: &str,
        root: &Path,
        branch: &str,
        base: &str,
        label: &str,
    ) -> Result<CreatedTerminal> {
        let args = vec![
            "worktree".to_string(),
            "create".into(),
            "--workspace".into(),
            workspace_id.into(),
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
        harness: &str,
        pane_id: &str,
        model: Option<&str>,
    ) -> Result<()> {
        let mut args = vec![
            "agent".to_string(),
            "start".into(),
            name.into(),
            "--kind".into(),
            harness.into(),
            "--pane".into(),
            pane_id.into(),
            "--timeout".into(),
            "120000".into(),
        ];
        if let Some(model) = model {
            args.extend(["--".into(), "--model".into(), model.into()]);
        }
        self.checked(args)?;
        Ok(())
    }

    pub fn remove_worktree(&self, workspace_id: &str) -> Result<()> {
        self.checked(["worktree", "remove", "--workspace", workspace_id])?;
        Ok(())
    }

    pub fn close_tab(&self, tab_id: &str) -> Result<()> {
        self.checked(["tab", "close", tab_id])?;
        Ok(())
    }
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
}

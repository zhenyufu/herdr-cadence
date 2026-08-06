use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};

fn git(root: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git in {}", root.display()))
}

fn checked(root: &Path, args: &[&str]) -> Result<String> {
    let output = git(root, args)?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn repository_root(path: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(checked(
        path,
        &["rev-parse", "--show-toplevel"],
    )?))
}

pub fn current_branch(root: &Path) -> Result<String> {
    let branch = checked(root, &["branch", "--show-current"])?;
    if branch.is_empty() {
        bail!("Cadence requires a checked-out branch, not detached HEAD");
    }
    Ok(branch)
}

pub fn head(root: &Path) -> Result<String> {
    checked(root, &["rev-parse", "HEAD"])
}

pub fn ensure_clean(root: &Path) -> Result<()> {
    let status = checked(root, &["status", "--porcelain"])?;
    if !status.is_empty() {
        bail!("Git worktree is dirty; commit or stash changes before continuing");
    }
    Ok(())
}

pub fn is_ancestor(root: &Path, ancestor: &str, descendant: &str) -> Result<bool> {
    let output = git(root, &["merge-base", "--is-ancestor", ancestor, descendant])?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => bail!(
            "git merge-base failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    }
}

pub fn commits_between(root: &Path, base: &str, head: &str) -> Result<Vec<String>> {
    let range = format!("{base}..{head}");
    let output = checked(root, &["rev-list", "--reverse", &range])?;
    Ok(output
        .lines()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .collect())
}

pub fn changed_paths(root: &Path, base: &str, head: &str) -> Result<Vec<String>> {
    let range = format!("{base}..{head}");
    let output = git(root, &["diff", "--name-only", "-z", &range])?;
    if !output.status.success() {
        bail!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            String::from_utf8(path.to_vec())
                .context("Cadence cannot integrate a non-UTF-8 Git path")
        })
        .collect()
}

pub fn cherry_pick(root: &Path, commits: &[String]) -> Result<()> {
    if commits.is_empty() {
        bail!("Worker produced no commits");
    }
    let mut command = Command::new("git");
    command.arg("-C").arg(root).arg("cherry-pick").args(commits);
    let output = command.output()?;
    if output.status.success() {
        return Ok(());
    }
    let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let _ = git(root, &["cherry-pick", "--abort"]);
    bail!("cherry-pick failed and was aborted: {message}")
}

pub fn delete_branch(root: &Path, branch: &str) -> Result<()> {
    anyhow::ensure!(
        branch.starts_with("cadence/"),
        "refusing to delete non-Cadence branch"
    );
    let reference = format!("refs/heads/{branch}");
    let exists = git(root, &["show-ref", "--verify", "--quiet", &reference])?;
    if exists.status.code() == Some(1) {
        return Ok(());
    }
    anyhow::ensure!(
        exists.status.success(),
        "failed to inspect Cadence branch {branch}"
    );
    checked(root, &["branch", "-D", branch])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn command(root: &Path, args: &[&str]) {
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

    fn repository() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        command(temp.path(), &["init", "-b", "main"]);
        command(
            temp.path(),
            &["config", "user.email", "cadence@example.test"],
        );
        command(temp.path(), &["config", "user.name", "Cadence Test"]);
        fs::write(temp.path().join("file.txt"), "base\n").unwrap();
        command(temp.path(), &["add", "file.txt"]);
        command(temp.path(), &["commit", "-m", "base"]);
        temp
    }

    #[test]
    fn cherry_picks_worker_commit() {
        let repo = repository();
        let worker_dir = tempfile::tempdir().unwrap();
        let worker_path = worker_dir.path().join("worker");
        command(
            repo.path(),
            &[
                "worktree",
                "add",
                "-b",
                "cadence/test/worker-1",
                worker_path.to_str().unwrap(),
            ],
        );
        let base = head(repo.path()).unwrap();
        fs::write(worker_path.join("worker.txt"), "done\n").unwrap();
        command(&worker_path, &["add", "worker.txt"]);
        command(&worker_path, &["commit", "-m", "worker"]);
        let worker_head = head(&worker_path).unwrap();
        let commits = commits_between(&worker_path, &base, &worker_head).unwrap();
        assert_eq!(
            changed_paths(&worker_path, &base, &worker_head).unwrap(),
            ["worker.txt"]
        );
        cherry_pick(repo.path(), &commits).unwrap();
        assert_eq!(
            fs::read_to_string(repo.path().join("worker.txt")).unwrap(),
            "done\n"
        );
        ensure_clean(repo.path()).unwrap();
    }

    #[test]
    fn aborts_conflicting_cherry_pick() {
        let repo = repository();
        let worker_dir = tempfile::tempdir().unwrap();
        let worker_path = worker_dir.path().join("worker");
        command(
            repo.path(),
            &[
                "worktree",
                "add",
                "-b",
                "cadence/test/worker-2",
                worker_path.to_str().unwrap(),
            ],
        );
        let base = head(repo.path()).unwrap();
        fs::write(worker_path.join("file.txt"), "worker\n").unwrap();
        command(&worker_path, &["add", "file.txt"]);
        command(&worker_path, &["commit", "-m", "worker conflict"]);
        let worker_head = head(&worker_path).unwrap();
        fs::write(repo.path().join("file.txt"), "orchestrator\n").unwrap();
        command(repo.path(), &["add", "file.txt"]);
        command(repo.path(), &["commit", "-m", "base conflict"]);
        let before = head(repo.path()).unwrap();
        let commits = commits_between(&worker_path, &base, &worker_head).unwrap();
        assert!(cherry_pick(repo.path(), &commits).is_err());
        assert_eq!(head(repo.path()).unwrap(), before);
        ensure_clean(repo.path()).unwrap();
        assert_eq!(
            fs::read_to_string(repo.path().join("file.txt")).unwrap(),
            "orchestrator\n"
        );
    }
}

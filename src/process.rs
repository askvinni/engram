//! Running external commands. Every helper here checks the exit status and
//! puts the command's stderr into the error, so callers never have to.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, Output, Stdio};

use crate::config;

/// Longest argument shown in full when a command is described in an error;
/// prompts and issue bodies are cut so the message stays readable.
const DESCRIBED_ARG_MAX: usize = 60;

/// `program arg ...` for error messages, each overlong argument shortened.
pub fn describe(cmd: &Command) -> String {
    std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|a| shorten(&a.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shorten(arg: &str) -> String {
    match arg.char_indices().nth(DESCRIBED_ARG_MAX) {
        Some((idx, _)) => format!("{}\u{2026}", &arg[..idx]),
        None => arg.to_string(),
    }
}

/// Runs `cmd` with stdin closed and returns its output whatever the exit
/// status. Fails only when the command can't be started.
pub fn spawn(cmd: &mut Command) -> Result<Output> {
    cmd.stdin(Stdio::null())
        .output()
        .with_context(|| format!("running `{}`", describe(cmd)))
}

/// The error for a command that exited unsuccessfully: its stderr, or its
/// stdout when stderr is empty.
pub fn failure(cmd: &Command, output: &Output) -> anyhow::Error {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    anyhow::anyhow!("`{}` failed: {detail}", describe(cmd))
}

/// Runs `cmd` and returns its trimmed stdout; a non-zero exit is an error.
pub fn run_checked(cmd: &mut Command) -> Result<String> {
    let output = spawn(cmd)?;
    if !output.status.success() {
        return Err(failure(cmd, &output));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Like [`run_checked`], but a failure whose stderr contains `benign` (e.g.
/// "already exists") counts as success.
pub fn run_tolerating(cmd: &mut Command, benign: &str) -> Result<()> {
    let output = spawn(cmd)?;
    if output.status.success() || String::from_utf8_lossy(&output.stderr).contains(benign) {
        return Ok(());
    }
    Err(failure(cmd, &output))
}

/// `git` run in `repo_root`, for commands that stay local.
pub fn git(repo_root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_root);
    cmd
}

/// `git` for commands that talk to `origin`, with the configured GitHub
/// identity applied: an HTTPS remote using gh as credential helper would
/// otherwise authenticate as the active gh account.
pub fn git_remote(repo_root: &Path) -> Result<Command> {
    let mut cmd = git(repo_root);
    config::with_github_identity(&mut cmd)?;
    Ok(cmd)
}

/// `git diff --cached --quiet` exits 1 when something is staged.
const DIFF_QUIET_CHANGES: i32 = 1;

/// Whether the index holds staged changes in `repo_root`.
pub fn has_staged_changes(repo_root: &Path) -> Result<bool> {
    let mut cmd = git(repo_root);
    cmd.args(["diff", "--cached", "--quiet"]);
    let output = spawn(&mut cmd)?;
    match output.status.code() {
        Some(0) => Ok(false),
        Some(DIFF_QUIET_CHANGES) => Ok(true),
        _ => Err(failure(&cmd, &output)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_shortens_long_arguments_only() {
        let long = "é".repeat(DESCRIBED_ARG_MAX + 5);
        let mut cmd = Command::new("claude");
        cmd.args(["-p", &long, "--output-format", "text"]);
        let described = describe(&cmd);
        let want = format!(
            "claude -p {}\u{2026} --output-format text",
            "é".repeat(DESCRIBED_ARG_MAX)
        );
        assert_eq!(described, want);
    }

    #[test]
    fn run_checked_returns_stdout_or_stderr_in_the_error() {
        let ok = run_checked(Command::new("sh").args(["-c", "echo ' hi '"])).unwrap();
        assert_eq!(ok, "hi");

        let err = run_checked(Command::new("sh").args(["-c", "echo boom >&2; exit 3"]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("failed: boom"), "{err}");

        let err = run_checked(Command::new("sh").args(["-c", "echo only-stdout; exit 1"]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("failed: only-stdout"), "{err}");
    }

    #[test]
    fn run_tolerating_accepts_only_the_benign_failure() {
        assert!(run_tolerating(
            Command::new("sh").args(["-c", "echo 'label already exists' >&2; exit 1"]),
            "already exists"
        )
        .is_ok());
        assert!(run_tolerating(
            Command::new("sh").args(["-c", "echo 'no such repo' >&2; exit 1"]),
            "already exists"
        )
        .is_err());
    }

    #[test]
    fn has_staged_changes_tracks_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        run_checked(git(root).args(["init", "-q"])).unwrap();
        assert!(!has_staged_changes(root).unwrap());
        std::fs::write(root.join("f"), "x").unwrap();
        run_checked(git(root).args(["add", "f"])).unwrap();
        assert!(has_staged_changes(root).unwrap());
    }
}

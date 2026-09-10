use anyhow::{Context, Result};
use std::process::Command;
use std::sync::OnceLock;

#[allow(dead_code)]
static KATA_AVAILABLE: OnceLock<bool> = OnceLock::new();

/// Returns true iff `kata` is on PATH and the current repo has a `.kata.toml` binding.
/// The result is cached for the lifetime of the process.
pub fn kata_available() -> bool {
    *KATA_AVAILABLE.get_or_init(detect)
}

fn detect() -> bool {
    let binary_ok = Command::new("kata")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());

    if !binary_ok {
        return false;
    }

    crate::config::find_repo_root()
        .map(|root| root.join(".kata.toml").exists())
        .unwrap_or(false)
}

/// Create a kata issue and return its short ref (e.g. "abc4").
/// The idempotency key ensures retries after partial failure don't duplicate the issue.
pub fn create(title: &str, body: &str, idempotency_key: &str) -> Result<String> {
    let output = Command::new("kata")
        .args([
            "create",
            title,
            "--body",
            body,
            "--idempotency-key",
            idempotency_key,
            "--json",
        ])
        .output()
        .context("running kata create")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("kata create failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8(output.stdout).context("kata create output not UTF-8")?;
    let v: serde_json::Value =
        serde_json::from_str(&stdout).context("parsing kata create JSON output")?;
    v["issue"]["short_id"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| {
            anyhow::anyhow!("kata create JSON missing issue.short_id: {}", stdout.trim())
        })
}

/// Close a kata issue with commit evidence. If the issue was already deleted or
/// never existed (out-of-band), treats that as success rather than failure —
/// the intent (closed) has effectively been satisfied.
pub fn close(kata_ref: &str, message: &str, commit_sha: &str) -> Result<()> {
    let output = Command::new("kata")
        .args([
            "close",
            kata_ref,
            "--done",
            "--message",
            message,
            "--commit",
            commit_sha,
            "--json",
        ])
        .output()
        .context("running kata close")?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let kind = serde_json::from_str::<serde_json::Value>(&stderr)
        .ok()
        .and_then(|v| v["error"]["kind"].as_str().map(|s| s.to_string()));

    if kind.as_deref() == Some("not_found") {
        return Ok(());
    }

    anyhow::bail!("kata close failed: {}", stderr.trim());
}

/// Post a comment to a kata issue.
pub fn comment(kata_ref: &str, body: &str) -> Result<()> {
    let output = Command::new("kata")
        .args(["comment", kata_ref, "--body", body])
        .output()
        .context("running kata comment")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("kata comment failed: {}", stderr.trim());
    }
    Ok(())
}

/// Parse the `Kata: <ref>` line that `plan::new` appends to a GitHub issue body.
pub fn parse_ref(issue_body: &str) -> Option<String> {
    issue_body
        .lines()
        .find_map(|line| line.trim().strip_prefix("Kata:"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kata_available_returns_bool() {
        // Just verify it runs without panicking; actual value depends on the environment.
        let _ = kata_available();
    }

    #[test]
    fn parse_ref_finds_kata_line() {
        let body = "**Why**\nSome text.\n\nKata: abc4";
        assert_eq!(parse_ref(body), Some("abc4".to_string()));
    }

    #[test]
    fn parse_ref_returns_none_when_absent() {
        let body = "**Why**\nSome text with no kata ref.";
        assert_eq!(parse_ref(body), None);
    }

    #[test]
    fn parse_ref_ignores_blank_ref() {
        let body = "Kata: \n";
        assert_eq!(parse_ref(body), None);
    }
}

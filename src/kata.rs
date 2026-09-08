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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kata_available_returns_bool() {
        // Just verify it runs without panicking; actual value depends on the environment.
        let _ = kata_available();
    }
}

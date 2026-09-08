use std::process::Command;
use std::sync::OnceLock;

#[allow(dead_code)]
static KATA_AVAILABLE: OnceLock<bool> = OnceLock::new();

/// Returns true iff `kata` is on PATH and the current repo has a `.kata.toml` binding.
/// The result is cached for the lifetime of the process.
#[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kata_available_returns_bool() {
        // Just verify it runs without panicking; actual value depends on the environment.
        let _ = kata_available();
    }
}

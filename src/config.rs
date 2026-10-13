use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    pub github: GitHubConfig,
    pub memory: MemoryConfig,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct GitHubConfig {
    pub repo: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryConfig {
    pub default_categories: Vec<String>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            default_categories: vec![
                "patterns".into(),
                "tripwires".into(),
                "architecture".into(),
                "testing".into(),
            ],
        }
    }
}

impl Config {
    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = repo_root.join(".engram/config.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&contents).context("parsing .engram/config.toml")
    }

    pub fn save(&self, repo_root: &Path) -> Result<()> {
        let dir = repo_root.join(".engram");
        std::fs::create_dir_all(&dir)?;
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(dir.join("config.toml"), contents)?;
        Ok(())
    }

    pub fn repo(&self) -> Option<&str> {
        self.github.repo.as_deref()
    }
}

pub fn find_repo_root() -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("running git rev-parse")?;
    if !output.status.success() {
        anyhow::bail!("not in a git repository");
    }
    let path = String::from_utf8(output.stdout)?.trim().to_string();
    Ok(PathBuf::from(path))
}

/// Prefix on every error raised because a command needs GitHub but the repo is
/// local-only. Keeps the "requires GitHub" wording greppable and consistent.
const REQUIRES_GITHUB: &str = "this command requires GitHub";

static GH_AVAILABLE: OnceLock<bool> = OnceLock::new();

/// Returns true iff the `gh` CLI is on PATH and runs. Cached for the process.
/// Mirrors `kata::kata_available()` — every optional `gh` step is gated on this.
pub fn gh_available() -> bool {
    *GH_AVAILABLE.get_or_init(|| {
        Command::new("gh")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

/// Returns true iff the repo has at least one git remote configured.
pub fn has_remote(repo_root: &Path) -> bool {
    Command::new("git")
        .arg("remote")
        .current_dir(repo_root)
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.trim_ascii().is_empty())
}

/// Best-effort `owner/name` lookup via `gh repo view`. Returns `None` — without
/// shelling out to `gh` — when the repo has no remote or `gh` is unavailable,
/// so local-only repos never touch GitHub.
pub fn infer_repo(repo_root: &Path) -> Option<String> {
    if !has_remote(repo_root) || !gh_available() {
        return None;
    }
    let output = Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "-q",
            ".nameWithOwner",
        ])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if output.status.success() {
        String::from_utf8(output.stdout)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    } else {
        None
    }
}

/// Explains why GitHub is unusable, or `None` when the prerequisites are met.
fn github_unavailable_reason(
    gh_ok: bool,
    configured: bool,
    remote_ok: bool,
) -> Option<&'static str> {
    if !gh_ok {
        Some("the `gh` CLI is not installed or not on PATH")
    } else if !configured && !remote_ok {
        Some("this repo has no git remote and no `github.repo` in .engram/config.toml")
    } else {
        None
    }
}

/// Resolves the GitHub `owner/name` for commands that are GitHub-only
/// (`plan`, `learn`, `objective`, `compact`, `memory migrate`). Fails loudly with a
/// "requires GitHub" error in local-only repos instead of degrading silently.
pub fn resolve_repo(cfg: &Config, repo_root: &Path) -> Result<String> {
    let configured = cfg.repo();
    if let Some(reason) =
        github_unavailable_reason(gh_available(), configured.is_some(), has_remote(repo_root))
    {
        anyhow::bail!(
            "{REQUIRES_GITHUB}: {reason}. Local-only repos support `engram init` and memory capture; \
             add a GitHub remote (or set `github.repo` in .engram/config.toml) and install `gh` to use it"
        );
    }
    if let Some(repo) = configured {
        return Ok(repo.to_string());
    }
    infer_repo(repo_root).ok_or_else(|| {
        anyhow::anyhow!(
            "{REQUIRES_GITHUB}: could not resolve a GitHub repo from the git remote \
             (is `gh` authenticated?). Set `github.repo` in .engram/config.toml or run `engram init`"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn has_remote_false_for_fresh_repo() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        assert!(!has_remote(dir.path()));
    }

    #[test]
    fn has_remote_true_once_origin_added() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(
            dir.path(),
            &["remote", "add", "origin", "https://example.com/x/y.git"],
        );
        assert!(has_remote(dir.path()));
    }

    #[test]
    fn infer_repo_none_without_remote() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        assert_eq!(infer_repo(dir.path()), None);
    }

    #[test]
    fn resolve_repo_errors_with_requires_github_without_remote() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        let err = resolve_repo(&Config::default(), dir.path()).unwrap_err();
        assert!(err.to_string().contains(REQUIRES_GITHUB), "{err}");
    }

    #[test]
    fn unavailable_reason_missing_gh_wins_even_if_configured() {
        assert!(github_unavailable_reason(false, true, true)
            .unwrap()
            .contains("`gh`"));
    }

    #[test]
    fn unavailable_reason_no_remote_and_unconfigured() {
        assert!(github_unavailable_reason(true, false, false)
            .unwrap()
            .contains("no git remote"));
    }

    #[test]
    fn unavailable_reason_configured_repo_needs_no_remote() {
        assert_eq!(github_unavailable_reason(true, true, false), None);
        assert_eq!(github_unavailable_reason(true, false, true), None);
    }
}

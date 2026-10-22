use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    pub github: GitHubConfig,
    pub memory: MemoryConfig,
    #[serde(default)]
    pub dispatch: DispatchConfig,
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

/// `[dispatch]`: templates for the branches and draft PRs `engram dispatch`
/// creates (see `dispatch::render` for the placeholders), and the command it
/// launches in each workspace.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DispatchConfig {
    pub branch: String,
    pub pr_title: String,
    /// Shell command that starts Claude in the pane, used verbatim (it may
    /// carry flags). Unset means the `claude` dispatch resolved on PATH.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude: Option<String>,
}

impl Default for DispatchConfig {
    fn default() -> Self {
        Self {
            branch: DEFAULT_BRANCH_TEMPLATE.into(),
            pr_title: DEFAULT_PR_TITLE_TEMPLATE.into(),
            claude: None,
        }
    }
}

const DEFAULT_BRANCH_TEMPLATE: &str = "{number}-{slug}";
const DEFAULT_PR_TITLE_TEMPLATE: &str = "{title}";

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

/// Per-clone git config naming the GitHub account engram's `gh` calls (and the
/// sessions dispatch launches) act as. It belongs to the person, not the repo,
/// so it lives in git config rather than the committed `.engram/config.toml`.
pub const GITHUB_USER_KEY: &str = "engram.githubUser";

static GITHUB_USER: OnceLock<Option<String>> = OnceLock::new();

/// `git config engram.githubUser` for the current repo, if set. Cached.
pub fn github_user() -> Option<String> {
    GITHUB_USER
        .get_or_init(|| {
            let output = Command::new("git")
                .args(["config", "--get", GITHUB_USER_KEY])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            String::from_utf8(output.stdout)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .clone()
}

/// Environment variables gh reads a token from, in its own precedence order.
/// When either is set, gh already has an identity and engram leaves it alone.
const GH_TOKEN_ENVS: &[&str] = &["GH_TOKEN", "GITHUB_TOKEN"];

/// The configured user whose token engram must resolve: none when a token
/// env var is set, since an existing one always wins.
fn token_user_to_resolve<'a>(
    env_tokens: &[Option<&str>],
    user: Option<&'a str>,
) -> Option<&'a str> {
    user.filter(|_| env_tokens.iter().flatten().all(|t| t.is_empty()))
}

/// Why the configured account's token couldn't be looked up. Carries gh's
/// stderr only: the token itself never goes into a message.
fn token_lookup_error(user: &str, stderr: &str) -> String {
    format!(
        "{GITHUB_USER_KEY} is {user:?} but `gh auth token --user {user}` failed ({}). \
         Log in as {user} with `gh auth login`, or unset it with \
         `git config --unset {GITHUB_USER_KEY}`",
        stderr.trim()
    )
}

/// `GH_TOKEN` to set on `gh` subprocesses, resolved once per process. The
/// error is kept as a string so the cached value can be cloned to callers.
static GH_TOKEN: OnceLock<std::result::Result<Option<String>, String>> = OnceLock::new();

fn gh_token() -> Result<Option<String>> {
    GH_TOKEN
        .get_or_init(|| {
            let env_tokens: Vec<Option<String>> = GH_TOKEN_ENVS
                .iter()
                .map(|v| std::env::var(v).ok())
                .collect();
            let env_tokens: Vec<Option<&str>> = env_tokens.iter().map(Option::as_deref).collect();
            let user = github_user();
            let Some(user) = token_user_to_resolve(&env_tokens, user.as_deref()) else {
                return Ok(None);
            };
            // Never `gh auth switch`: that would change the user's active account.
            let output = Command::new("gh")
                .args(["auth", "token", "--user", user])
                .output()
                .map_err(|e| format!("running gh auth token: {e}"))?;
            let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !output.status.success() || token.is_empty() {
                return Err(token_lookup_error(
                    user,
                    &String::from_utf8_lossy(&output.stderr),
                ));
            }
            Ok(Some(token))
        })
        .clone()
        .map_err(anyhow::Error::msg)
}

/// Applies the configured GitHub identity (`engram.githubUser`) to a `gh`
/// command. Every `gh` call that talks to GitHub goes through this.
pub fn with_github_identity(cmd: &mut Command) -> Result<&mut Command> {
    if let Some(token) = gh_token()? {
        cmd.env("GH_TOKEN", token);
    }
    Ok(cmd)
}

/// `gh` with the configured GitHub identity applied.
pub fn gh_command() -> Result<Command> {
    let mut cmd = Command::new("gh");
    with_github_identity(&mut cmd)?;
    Ok(cmd)
}

/// Best-effort `owner/name` lookup via `gh repo view`. Returns `None` — without
/// shelling out to `gh` — when the repo has no remote or `gh` is unavailable,
/// so local-only repos never touch GitHub. Fails only when the configured
/// `engram.githubUser` has no usable token, which must not pass silently as
/// "no GitHub repo".
pub fn infer_repo(repo_root: &Path) -> Result<Option<String>> {
    if !has_remote(repo_root) || !gh_available() {
        return Ok(None);
    }
    let Ok(output) = gh_command()?
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
    else {
        return Ok(None);
    };
    if !output.status.success() {
        return Ok(None);
    }
    Ok(String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()))
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
    // Surface a bad `engram.githubUser` now, with its own message, instead of
    // as a confusing failure of the first `gh` call.
    gh_token()?;
    if let Some(repo) = configured {
        return Ok(repo.to_string());
    }
    infer_repo(repo_root)?.ok_or_else(|| {
        anyhow::anyhow!(
            "{REQUIRES_GITHUB}: could not resolve a GitHub repo from the git remote \
             (is `gh` authenticated?). Set `github.repo` in .engram/config.toml or run `engram init`"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

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
        assert_eq!(infer_repo(dir.path()).unwrap(), None);
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

    #[rstest]
    #[case::defaults("", "{number}-{slug}", "{title}", None)]
    #[case::partial(
        "[dispatch]\nbranch = \"{slug}\"\nclaude = \"claude --model opus\"\n",
        "{slug}",
        "{title}",
        Some("claude --model opus")
    )]
    fn dispatch_section_fills_in_defaults(
        #[case] section: &str,
        #[case] branch: &str,
        #[case] pr_title: &str,
        #[case] claude: Option<&str>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".engram")).unwrap();
        let text = format!("[github]\n\n[memory]\ndefault_categories = []\n{section}");
        std::fs::write(dir.path().join(".engram/config.toml"), text).unwrap();
        let d = Config::load(dir.path()).unwrap().dispatch;
        assert_eq!(
            (d.branch.as_str(), d.pr_title.as_str(), d.claude.as_deref()),
            (branch, pr_title, claude)
        );
    }

    #[rstest]
    #[case::gh_token_set([Some("tok"), None], Some("me"), None)]
    #[case::github_token_set([None, Some("tok")], Some("me"), None)]
    #[case::blank_gh_token_ignored([Some(""), Some("tok")], Some("me"), None)]
    #[case::no_env_token([None, None], Some("me"), Some("me"))]
    #[case::blank_env_tokens([Some(""), Some("")], Some("me"), Some("me"))]
    #[case::no_user([None, None], None, None)]
    fn github_user_token_is_resolved_only_without_an_env_token(
        #[case] env: [Option<&str>; 2],
        #[case] user: Option<&str>,
        #[case] want: Option<&str>,
    ) {
        assert_eq!(token_user_to_resolve(&env, user), want);
    }
}

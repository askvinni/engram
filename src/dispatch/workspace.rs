//! The kwt worktree, tmux session and Claude launch for a dispatched plan.
//! See the tmux/kwt invariants in CLAUDE.md before changing anything here.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::config;
use crate::process::run_checked;

use super::{build_prompt, DispatchPlan, Run, READY_NO_RELAUNCH};

pub(super) const SHELLS: &[&str] = &["zsh", "bash", "fish", "sh", "dash", "ksh"];
/// How long to wait for Claude to replace the shell in a pane, then to stay there.
pub(super) const LAUNCH_TIMEOUT: Duration = Duration::from_secs(15);
pub(super) const LAUNCH_SETTLE: Duration = Duration::from_secs(3);
pub(super) const LAUNCH_POLL: Duration = Duration::from_millis(500);
/// `kwt open --start-session` attempts, waiting `KWT_OPEN_BACKOFF` times
/// the attempt number between them (1s, then 2s).
pub(super) const KWT_OPEN_ATTEMPTS: u64 = 3;
pub(super) const KWT_OPEN_BACKOFF: Duration = Duration::from_secs(1);
pub(super) const PANE_TAIL_LINES: usize = 5;
/// Overrides the launched command above `[dispatch] claude` (verbatim; for tests).
pub(super) const CLAUDE_OVERRIDE_ENV: &str = "ENGRAM_DISPATCH_CLAUDE";
/// Prompt files live under `$XDG_STATE_HOME/<PROMPT_SUBDIR>`, outside every worktree.
pub(super) const PROMPT_SUBDIR: &str = "engram/dispatch";
pub(super) const XDG_STATE_HOME: &str = "XDG_STATE_HOME";
/// `$XDG_STATE_HOME` when unset, relative to `$HOME`.
pub(super) const DEFAULT_STATE_HOME: &str = ".local/state";
/// `NAME=value` words whose name contains one of these are redacted for display.
pub(super) const CREDENTIAL_MARKERS: &[&str] = &["TOKEN", "SECRET", "KEY", "PASSWORD"];
/// Permission mode the dispatched Claude session starts in.
pub(super) const PERMISSION_MODE: &str = "auto";
/// What launching Claude in a plan's workspace came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Launch {
    /// A new session took over the pane.
    Launched,
    /// The pane already runs something else (most likely Claude).
    AlreadyRunning(String),
    /// `--no-launch`.
    Skipped,
    /// The PR is ready for review, so its session isn't relaunched.
    Ready,
}

impl Run<'_> {
    /// Create or reuse the plan's worktree and tmux session, launch Claude in
    /// it (unless `--no-launch` or the PR is ready), and print the report.
    pub(super) fn open_workspace(
        &self,
        t: &DispatchPlan,
        kata_ref: &str,
        pr_url: &str,
    ) -> Result<Launch> {
        let worktree = if let Some(path) = &t.worktree {
            eprintln!("worktree: reusing {}", path.display());
            path.clone()
        } else {
            run_checked(self.kwt().args(["add", &t.branch, "--no-launch"]))?;
            let path = self
                .kwt_worktree(&t.branch)?
                .with_context(|| format!("kwt added {} but `kwt list` can't find it", t.branch))?;
            eprintln!("worktree: created {}", path.display());
            path
        };
        let endpoint = self.start_session(&worktree)?;
        let (session, socket) = (&endpoint.session_name, &endpoint.tmux_socket_name);
        eprintln!("tmux: session {session} (socket {socket})");

        let launch = if self.opts.no_launch {
            Launch::Skipped
        } else if t.ready() {
            eprintln!("claude: {READY_NO_RELAUNCH}");
            Launch::Ready
        } else {
            self.launch_claude(t, &worktree, &endpoint, &build_prompt(t, kata_ref, pr_url))?
        };
        print!(
            "#{}  kata {kata_ref}  {} \u{2190} {}\n      pr: {pr_url}\n      worktree: {}\n      \
             tmux: {session} (socket {socket})  claude: {launch}\n",
            t.number,
            t.branch,
            t.base,
            worktree.display()
        );
        Ok(launch)
    }

    pub(super) fn kwt(&self) -> Command {
        let mut cmd = Command::new(&self.tools.kwt);
        cmd.current_dir(self.root);
        cmd
    }

    pub(super) fn kwt_worktree(&self, branch: &str) -> Result<Option<PathBuf>> {
        let out = run_checked(self.kwt().args(["list", "--json"]))?;
        Ok(select_worktree(&out, branch)?.filter(|p| p.is_dir()))
    }

    pub(super) fn start_session(&self, worktree: &Path) -> Result<Endpoint> {
        // --start-session is the only mode of `kwt open` that never attaches
        // the calling terminal. kwt runs its own tmux server, so the endpoint
        // comes from its JSON rather than from `tmux ls`. Creating a workspace
        // occasionally fails transiently inside kwt ("respawn pane failed: fork
        // failed"); --start-session is idempotent, so retry.
        let mut attempt = 0;
        loop {
            let open = run_checked(
                self.kwt()
                    .arg("open")
                    .arg(worktree)
                    .args(["--start-session", "--json"]),
            );
            match open {
                Ok(out) => return parse_endpoint(&out),
                Err(e) if attempt + 1 >= KWT_OPEN_ATTEMPTS => return Err(e),
                Err(e) => {
                    attempt += 1;
                    eprintln!(
                        "kwt open failed (attempt {attempt}/{KWT_OPEN_ATTEMPTS}), retrying: {e:#}"
                    );
                    std::thread::sleep(KWT_OPEN_BACKOFF * u32::try_from(attempt).unwrap_or(1));
                }
            }
        }
    }

    /// The claude command the pane runs (see [`claude_command`]).
    pub(super) fn claude(&self) -> String {
        claude_command(
            std::env::var(CLAUDE_OVERRIDE_ENV).ok(),
            self.cfg.dispatch.claude.as_deref(),
            &self.tools.claude,
        )
    }

    /// Run `tmux` against kwt's server for `endpoint`. kwt (and Ghosthub) run
    /// tmux with `TMUX_TMPDIR` unset; honouring it here would address a
    /// different server than the one holding the workspace.
    pub(super) fn tmux(&self, endpoint: &Endpoint, args: &[&str]) -> Result<String> {
        run_checked(
            Command::new(&self.tools.tmux)
                .env_remove("TMUX_TMPDIR")
                .args(["-L", &endpoint.tmux_socket_name])
                .args(args),
        )
    }

    /// Type the claude command into the session's pane, then wait until
    /// Claude has taken the pane over and is still there a moment later.
    pub(super) fn launch_claude(
        &self,
        t: &DispatchPlan,
        worktree: &Path,
        endpoint: &Endpoint,
        prompt: &str,
    ) -> Result<Launch> {
        let session = endpoint.session_name.as_str();
        let pane = || {
            self.tmux(
                endpoint,
                &[
                    "display-message",
                    "-p",
                    "-t",
                    session,
                    "#{pane_current_command}",
                ],
            )
        };
        let current = pane()?;
        if !is_shell(&current) {
            return Ok(Launch::AlreadyRunning(current));
        }

        // The prompt lives outside the worktree so nothing from the dispatch
        // can be committed, and the keystrokes stay short and quoting-safe.
        let prompt_file = prompt_path(
            std::env::var_os(XDG_STATE_HOME).map(PathBuf::from),
            std::env::var_os("HOME").map(PathBuf::from),
            self.repo,
            t.number,
        )?;
        if let Some(dir) = prompt_file.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&prompt_file, prompt)
            .with_context(|| format!("writing {}", prompt_file.display()))?;

        let user = config::github_user();
        let cmd = launch_command(&self.claude(), user.as_deref(), worktree, &prompt_file);
        self.tmux(endpoint, &["send-keys", "-t", session, "-l", &cmd])?;
        self.tmux(endpoint, &["send-keys", "-t", session, "Enter"])?;

        // Keystrokes landing is not a launch: wait for Claude to take over the
        // pane and still be there a moment later, so an immediate exit is reported.
        let deadline = Instant::now() + LAUNCH_TIMEOUT;
        while is_shell(&pane()?) && Instant::now() < deadline {
            std::thread::sleep(LAUNCH_POLL);
        }
        if !is_shell(&pane()?) {
            std::thread::sleep(LAUNCH_SETTLE);
        }
        if is_shell(&pane()?) {
            let out = self.tmux(endpoint, &["capture-pane", "-p", "-t", session])?;
            let lines: Vec<&str> = out.lines().collect();
            let tail = &lines[lines.len().saturating_sub(PANE_TAIL_LINES)..];
            anyhow::bail!(
                "Claude exited right after launch; pane shows:\n        {}",
                tail.join("\n        ")
            );
        }
        Ok(Launch::Launched)
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct KwtEntry {
    pub(super) path: PathBuf,
    #[serde(default)]
    pub(super) branch: Option<String>,
}

/// Path of the worktree whose branch is exactly `branch`. `kwt get` can't be
/// used: it matches fuzzily and may return another plan's worktree.
pub(super) fn select_worktree(json: &str, branch: &str) -> Result<Option<PathBuf>> {
    if json.trim().is_empty() {
        return Ok(None);
    }
    let entries: Option<Vec<KwtEntry>> =
        serde_json::from_str(json).context("parsing `kwt list --json` output")?;
    Ok(entries
        .unwrap_or_default()
        .into_iter()
        .find(|e| e.branch.as_deref() == Some(branch))
        .map(|e| e.path))
}

#[derive(Debug, Deserialize, PartialEq)]
pub(super) struct Endpoint {
    #[serde(default)]
    pub(super) session_name: String,
    #[serde(default)]
    pub(super) tmux_socket_name: String,
}

pub(super) fn parse_endpoint(json: &str) -> Result<Endpoint> {
    let endpoint: Endpoint =
        serde_json::from_str(json).context("parsing `kwt open --json` output")?;
    if endpoint.session_name.is_empty() || endpoint.tmux_socket_name.is_empty() {
        anyhow::bail!("kwt open returned no tmux endpoint: {}", json.trim());
    }
    Ok(endpoint)
}

pub(super) fn is_shell(pane_command: &str) -> bool {
    SHELLS.contains(&pane_command.trim().trim_start_matches('-'))
}

/// `s` in single quotes for a POSIX shell, whatever it contains.
pub(super) fn single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r#"'"'"'"#))
}

/// `s` as one shell word: as it is when it needs no quoting, else single-quoted.
pub(super) fn shell_quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        return s.to_string();
    }
    single_quote(s)
}

/// `$XDG_STATE_HOME/engram/dispatch/<owner>-<name>-<N>.md`, the state home
/// defaulting to `~/.local/state`: outside every worktree.
pub(super) fn prompt_path(
    state_home: Option<PathBuf>,
    home: Option<PathBuf>,
    repo: &str,
    number: u64,
) -> Result<PathBuf> {
    let state_home = match state_home.filter(|p| !p.as_os_str().is_empty()) {
        Some(p) => p,
        None => home
            .context("HOME is not set, so the dispatch prompt file has nowhere to go")?
            .join(DEFAULT_STATE_HOME),
    };
    Ok(state_home
        .join(PROMPT_SUBDIR)
        .join(format!("{}-{number}.md", repo.replace('/', "-"))))
}

/// `ENGRAM_DISPATCH_CLAUDE`, else `[dispatch] claude` (both verbatim), else
/// the resolved `claude` path.
pub(super) fn claude_command(
    env_override: Option<String>,
    configured: Option<&str>,
    resolved: &Path,
) -> String {
    env_override
        .filter(|c| !c.is_empty())
        .or_else(|| configured.filter(|c| !c.is_empty()).map(str::to_string))
        .unwrap_or_else(|| shell_quote(&resolved.to_string_lossy()))
}

/// `cmd` for display: the value of every `NAME=value` word whose name looks
/// like a credential is hidden, so a configured command never prints a token.
pub(super) fn redact_secrets(cmd: &str) -> String {
    cmd.split(' ')
        .map(|word| match word.split_once('=') {
            Some((name, _))
                if CREDENTIAL_MARKERS
                    .iter()
                    .any(|s| name.to_ascii_uppercase().contains(s)) =>
            {
                format!("{name}=\u{2026}")
            }
            _ => word.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn launch_command(
    claude: &str,
    github_user: Option<&str>,
    worktree: &Path,
    prompt_file: &Path,
) -> String {
    // The pane's shell can start outside the worktree: it inherits the tmux
    // server's cwd, which may be a since-deleted directory, and Claude refuses
    // to start there. cd explicitly rather than trusting the pane.
    let cd = format!("cd {} && ", shell_quote(&worktree.to_string_lossy()));
    // The pane's shell resolves the token itself (tmux panes don't inherit
    // engram's env), so it never appears in the keystrokes, the scrollback, or
    // engram's own command line. A plain assignment chained with && (not a
    // `VAR=… claude` prefix) gates the launch on the lookup: a failure leaves
    // the pane at the shell, which launch verification reports, instead of
    // starting Claude as the active gh account.
    let token = github_user
        .map(|u| {
            format!(
                "GH_TOKEN=\"$(gh auth token --user {})\" && export GH_TOKEN && ",
                single_quote(u)
            )
        })
        .unwrap_or_default();
    format!(
        "{cd}{token}{claude} --permission-mode {PERMISSION_MODE} \"$(cat {})\"",
        shell_quote(&prompt_file.to_string_lossy()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::test_support::*;
    use rstest::rstest;

    const WORKTREES: &str = r#"[
      {"path":"/repo","branch":"main","is_main":true},
      {"path":"/wt/mfbq","branch":"mfbq-dispatch-single-plan","session_name":"s1"},
      {"path":"/wt/a56d","branch":"a56d-dispatch-preflight-config"}
    ]"#;

    #[rstest]
    #[case::exact(WORKTREES, "mfbq-dispatch-single-plan", Ok(Some("/wt/mfbq")))]
    #[case::no_substring_match(WORKTREES, "single-plan", Ok(None))]
    #[case::empty_list("[]", "main", Ok(None))]
    #[case::no_output("", "main", Ok(None))]
    #[case::not_json("not json", "main", Err("parsing `kwt list --json` output"))]
    fn select_worktree_matches_branch_exactly(
        #[case] json: &str,
        #[case] branch: &str,
        #[case] want: Want<Option<&str>>,
    ) {
        let want = want.map(|p| p.map(PathBuf::from));
        assert_result(select_worktree(json, branch), want);
    }

    #[rstest]
    #[case::plain("/bin/claude --model opus", "/bin/claude --model opus")]
    #[case::credentials(
        "GH_TOKEN=a API_KEY=b Secret=c DEBUG=1 claude",
        "GH_TOKEN=\u{2026} API_KEY=\u{2026} Secret=\u{2026} DEBUG=1 claude"
    )]
    fn redact_secrets_hides_credential_assignments(#[case] cmd: &str, #[case] want: &str) {
        assert_eq!(redact_secrets(cmd), want);
    }

    #[rstest]
    // tmux reports login shells with a leading dash, and output ends in a newline.
    fn is_shell_accepts_shells(#[values("zsh", "bash", "-zsh", "zsh\n")] pane: &str) {
        assert!(is_shell(pane));
    }

    #[rstest]
    fn is_shell_rejects_programs(#[values("claude", "", "zshrc")] pane: &str) {
        assert!(!is_shell(pane));
    }

    #[rstest]
    #[case::kwt_open(
        r#"{"path":"/wt","session_name":"engram-143","tmux_socket_name":"kwt","created":true}"#,
        Ok(("engram-143", "kwt"))
    )]
    #[case::no_socket(r#"{"session_name":"s"}"#, Err("no tmux endpoint"))]
    fn parse_endpoint_cases(#[case] json: &str, #[case] want: Want<(&str, &str)>) {
        let want = want.map(|(session, socket)| Endpoint {
            session_name: session.into(),
            tmux_socket_name: socket.into(),
        });
        assert_result(parse_endpoint(json), want);
    }

    #[rstest]
    #[case::empty("", "''")]
    #[case::single_quote("it's", r#"'it'"'"'s'"#)]
    #[case::substitution("$(x)", "'$(x)'")]
    fn shell_quote_cases(#[case] raw: &str, #[case] quoted: &str) {
        assert_eq!(shell_quote(raw), quoted);
    }

    #[rstest]
    #[case::quoted_worktree(
        "'/Apps/My Claude/claude'",
        None,
        "/wt/my tree",
        "cd '/wt/my tree' && '/Apps/My Claude/claude' --permission-mode auto \"$(cat /p.md)\""
    )]
    #[case::token_for_user(
        "claude",
        Some("askvinni"),
        "/wt",
        "cd /wt && GH_TOKEN=\"$(gh auth token --user 'askvinni')\" && export GH_TOKEN \
         && claude --permission-mode auto \"$(cat /p.md)\""
    )]
    #[case::quoted_user(
        "claude",
        Some("a b"),
        "/wt",
        "cd /wt && GH_TOKEN=\"$(gh auth token --user 'a b')\" && export GH_TOKEN \
         && claude --permission-mode auto \"$(cat /p.md)\""
    )]
    fn launch_command_quotes_paths_and_gates_the_token(
        #[case] claude: &str,
        #[case] token_user: Option<&str>,
        #[case] worktree: &str,
        #[case] want: &str,
    ) {
        let got = launch_command(claude, token_user, Path::new(worktree), Path::new("/p.md"));
        assert_eq!(got, want);
    }

    #[rstest]
    #[case::env_wins(Some("fake"), Some("claude --x"), "fake")]
    #[case::blank_env_falls_to_config(Some(""), Some("claude --x"), "claude --x")]
    #[case::blank_config_falls_to_resolved(None, Some(""), "'/Apps/My Claude/claude'")]
    #[case::resolved_path_quoted(None, None, "'/Apps/My Claude/claude'")]
    fn claude_command_precedence(
        #[case] env: Option<&str>,
        #[case] config: Option<&str>,
        #[case] want: &str,
    ) {
        let resolved = Path::new("/Apps/My Claude/claude");
        assert_eq!(claude_command(env.map(Into::into), config, resolved), want);
    }

    #[rstest]
    #[case::xdg(Some("/xdg"), Some("/home/u"), Some("/xdg/engram/dispatch/o-r-7.md"))]
    #[case::blank_xdg_uses_home(
        Some(""),
        Some("/home/u"),
        Some("/home/u/.local/state/engram/dispatch/o-r-7.md")
    )]
    #[case::neither(None, None, None)]
    fn prompt_path_cases(
        #[case] xdg: Option<&str>,
        #[case] home: Option<&str>,
        #[case] want: Option<&str>,
    ) {
        let got = prompt_path(xdg.map(Into::into), home.map(Into::into), "o/r", 7);
        assert_eq!(got.ok(), want.map(PathBuf::from));
    }
}

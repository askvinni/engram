//! `engram dispatch`: hand plan issues to background Claude sessions.
//!
//! Dispatch is the only engram command that needs kwt, tmux and kata, so it
//! checks for them up front (`preflight`) instead of degrading silently.
//!
//! It plans without mutating anything, confirms, then executes.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::github::{self, OpenPr, PrWithBody};
use crate::kata;
use crate::objective;

/// kwt is often only a shell alias to the copy bundled with Ghosthub, which
/// `Command::new("kwt")` can't see.
const GHOSTHUB_KWT: &str = "/Applications/Ghosthub.app/Contents/Helpers/kwt";
const TOOLS: &[(&str, &str)] = &[
    ("gh", "--version"),
    ("kata", "--version"),
    ("kwt", "--version"),
    ("tmux", "-V"),
    ("claude", "--version"),
];
const SLUG_MAX: usize = 40;
const PLAN_LABEL: &str = "engram-plan";
const OBJECTIVE_LABEL: &str = "engram-objective";
const SHELLS: &[&str] = &["zsh", "bash", "fish", "sh", "dash", "ksh"];
/// How long to wait for Claude to replace the shell in a pane, then to stay there.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(15);
const LAUNCH_SETTLE: Duration = Duration::from_secs(3);
const LAUNCH_POLL: Duration = Duration::from_millis(500);
/// `kwt open --start-session` attempts; backoff is 1s, then 2s.
const KWT_OPEN_ATTEMPTS: u64 = 3;
const PANE_TAIL_LINES: usize = 5;
/// Replaces the launched `claude` command, verbatim (for tests).
const CLAUDE_OVERRIDE_ENV: &str = "ENGRAM_DISPATCH_CLAUDE";
/// Prompt files live under `$XDG_STATE_HOME/<PROMPT_SUBDIR>`, outside every worktree.
const PROMPT_SUBDIR: &str = "engram/dispatch";
const READY_NO_RELAUNCH: &str = "PR is ready for review \u{2014} not relaunching";
const LAUNCHED: &str = "launched";
/// `git ls-remote --exit-code` exits 2 when the ref is absent.
const LS_REMOTE_NO_MATCH: i32 = 2;

/// Paths of the tools dispatch runs itself (gh and kata go through github.rs
/// and kata.rs by name; preflight only checks that they work).
#[derive(Debug)]
pub struct Toolchain {
    kwt: PathBuf,
    tmux: PathBuf,
    claude: PathBuf,
}

fn resolve_tool(name: &str, path_var: Option<&OsStr>, fallback: Option<&Path>) -> Option<PathBuf> {
    path_var
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(name))
        .chain(fallback.map(Path::to_path_buf))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn check_tools(repo_root: &Path) -> (BTreeMap<&'static str, PathBuf>, Vec<String>) {
    let path_var = std::env::var_os("PATH");
    let mut found = BTreeMap::new();
    let mut missing = Vec::new();
    for &(name, flag) in TOOLS {
        let fallback = (name == "kwt").then(|| Path::new(GHOSTHUB_KWT));
        let ok = |p: &PathBuf| {
            Command::new(p)
                .arg(flag)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        match resolve_tool(name, path_var.as_deref(), fallback).filter(ok) {
            Some(p) => {
                found.insert(name, p);
            }
            None => missing.push(name),
        }
    }
    let kata_bound = repo_root.join(".kata.toml").exists();
    let problems = tool_problems(&missing, found.contains_key("kata"), kata_bound);
    (found, problems)
}

fn tool_problems(missing: &[&str], kata_installed: bool, kata_bound: bool) -> Vec<String> {
    let mut out = Vec::new();
    if !missing.is_empty() {
        out.push(format!("missing: {}", missing.join(", ")));
    }
    // A missing kata binary is reported above; flag the binding only when kata
    // itself is installed, so the fix is unambiguous.
    if kata_installed && !kata_bound {
        out.push("repo not bound to kata, run `kata init`".to_string());
    }
    out
}

pub fn unavailable_reasons(repo_root: &Path) -> Vec<String> {
    check_tools(repo_root).1
}

pub fn preflight(repo_root: &Path) -> Result<Toolchain> {
    let (mut found, problems) = check_tools(repo_root);
    if !problems.is_empty() {
        anyhow::bail!("engram dispatch can't run: {}", problems.join("; "));
    }
    let mut take = |name: &str| {
        found
            .remove(name)
            .with_context(|| format!("{name} resolved but not recorded"))
    };
    Ok(Toolchain {
        kwt: take("kwt")?,
        tmux: take("tmux")?,
        claude: take("claude")?,
    })
}

/// What `[dispatch]` templates can say about a plan issue.
#[derive(Debug, Default)]
struct PlanFields {
    number: u64,
    /// The plan's kata mirror, used only in the hint for missing metadata.
    kata_ref: Option<String>,
    title: String,
    meta: BTreeMap<String, String>,
}

/// `title` without a leading node-id bracket such as `[5.1] `; other bracket
/// groups (`[ENG-7] ...`) stay.
fn strip_node_prefix(title: &str) -> &str {
    let Some((inner, rest)) = title
        .trim_start()
        .strip_prefix('[')
        .and_then(|r| r.split_once(']'))
    else {
        return title;
    };
    let is_node_id = inner
        .split('.')
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
    if is_node_id {
        rest
    } else {
        title
    }
}

/// Branch-safe slug: lowercase ASCII letters, digits and single dashes, at
/// most `SLUG_MAX` long (cut at a word boundary), node-id prefix dropped.
fn slugify(title: &str) -> String {
    let kept: String = strip_node_prefix(title)
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == ' ' || *c == '-')
        .collect();
    let mut slug = String::new();
    for c in kept.trim().chars().map(|c| if c == ' ' { '-' } else { c }) {
        if !(c == '-' && slug.ends_with('-')) {
            slug.push(c);
        }
    }
    if slug.len() > SLUG_MAX {
        // ASCII only, so byte indices are char indices.
        let cut = slug[..=SLUG_MAX].rfind('-').unwrap_or(SLUG_MAX);
        slug.truncate(cut);
    }
    slug.trim_end_matches('-').to_string()
}

/// `template` with each `{name}` replaced by its value for `plan`. Every
/// missing `{meta[key]}` is reported at once.
fn fill_template(template: &str, plan: &PlanFields) -> Result<String> {
    let bad = |why: String| anyhow::anyhow!("bad template `{template}`: {why}");
    let mut out = String::new();
    let mut missing_meta: Vec<&str> = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or_else(|| bad("unclosed `{`".into()))?;
        let name = &after[..close];
        match name.strip_prefix("meta[").and_then(|r| r.strip_suffix(']')) {
            Some(key) => match plan.meta.get(key) {
                Some(value) => out.push_str(value),
                None if !missing_meta.contains(&key) => missing_meta.push(key),
                None => {}
            },
            None => out.push_str(&placeholder_value(name, plan).ok_or_else(|| {
                bad(format!(
                    "unknown placeholder `{{{name}}}` (expected number, title, slug or meta[key])"
                ))
            })?),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    if !missing_meta.is_empty() {
        let kata_ref = plan.kata_ref.as_deref().unwrap_or("<kata-ref>");
        let keys: Vec<String> = missing_meta.iter().map(|k| format!("'{k}'")).collect();
        let hints: Vec<String> = missing_meta
            .iter()
            .map(|k| format!("kata meta set {kata_ref} {k} <value>"))
            .collect();
        return Err(bad(format!(
            "plan #{} has no kata metadata {} ({})",
            plan.number,
            keys.join(", "),
            hints.join("; ")
        )));
    }
    Ok(out)
}

/// The value of a non-meta placeholder, or `None` if `name` isn't one.
fn placeholder_value(name: &str, plan: &PlanFields) -> Option<String> {
    match name {
        "number" => Some(plan.number.to_string()),
        "title" => Some(plan.title.clone()),
        "slug" => Some(slugify(&plan.title)),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, clap::Args)]
pub struct DispatchOptions {
    /// Print the plan and change nothing
    #[arg(long)]
    pub dry_run: bool,
    /// Don't ask for confirmation (required without a terminal)
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Set everything up but leave the tmux pane at a shell
    #[arg(long)]
    pub no_launch: bool,
}

/// Refuse anything but an open plan issue (plan label), including plain kata
/// issues.
fn check_kind(number: u64, issue: &github::Issue) -> Result<()> {
    let state = &issue.state;
    if !state.eq_ignore_ascii_case("open") {
        anyhow::bail!("#{number} is not open (state: {state})");
    }
    if !issue.has_label(PLAN_LABEL) {
        anyhow::bail!("#{number} is not an engram-plan issue (no `{PLAN_LABEL}` label)");
    }
    Ok(())
}

fn confirm() -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("stdin is not a terminal — pass --yes to dispatch without confirmation");
    }
    eprint!("Proceed? [Y/n] ");
    std::io::stderr().flush().context("flushing prompt")?;
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .context("reading confirmation")?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    ))
}

#[derive(Debug)]
struct Target {
    number: u64,
    title: String,
    body: String,
    /// The plan's kata mirror; without one, execute creates it as `plan new`
    /// would, with `meta` and `parent`.
    kata_ref: Option<String>,
    claimed_by_me: bool,
    meta: kata::TrackerMeta,
    parent: Option<String>,
    /// `{kata_ref}` in the branch or PR title is a placeholder until the mirror exists.
    kata_ref_pending: bool,
    branch: String,
    local: bool,
    remote: bool,
    pr_title: String,
    /// The base the PR targets: the default branch or a found PR's base.
    base: String,
    pr: Option<OpenPr>,
    /// How an open PR closing the plan was found; its branch and title then
    /// replace the templates'.
    pr_link: Option<String>,
    worktree: Option<PathBuf>,
}

impl Target {
    fn vars(&self, kata_ref: Option<String>, meta: BTreeMap<String, String>) -> TemplateVars {
        TemplateVars {
            number: self.number,
            kata_ref,
            title: self.title.clone(),
            meta,
        }
    }

    fn ready(&self) -> bool {
        self.pr.as_ref().is_some_and(|pr| !pr.is_draft)
    }

    fn pr_base(&self) -> &str {
        self.pr.as_ref().map_or(&self.base, |pr| &pr.base)
    }

    fn base_mismatch(&self) -> Option<String> {
        let pr = self.pr.as_ref().filter(|pr| pr.base != self.base)?;
        Some(format!("targets {}, not {}", pr.base, self.base))
    }

    fn warn_base_mismatch(&self) {
        if let (Some(pr), Some(why)) = (&self.pr, self.base_mismatch()) {
            eprintln!("warning: existing PR {} {why}", pr.url);
        }
    }
}

fn describe(t: &Target, no_launch: bool) -> String {
    let kata = match &t.kata_ref {
        Some(r) if t.claimed_by_me => format!("{r} (claimed by you)"),
        Some(r) => format!("{r} (will claim)"),
        None => "(no mirror, will create and claim)".to_string(),
    };
    let state = match (t.local, t.remote) {
        (true, _) => "reuse local branch",
        (false, true) => "track existing origin branch",
        (false, false) => "create from origin with an empty wip commit",
    };
    let (branch_note, title_note) = if t.pr_link.is_some() {
        (
            format!("reused from the existing PR; {state}"),
            " (existing PR)",
        )
    } else if t.kata_ref_pending {
        (
            format!("{PENDING_KATA_REF} filled in once the mirror exists; {state}"),
            "",
        )
    } else {
        (state.to_string(), "")
    };
    let pr = match (&t.pr, t.base_mismatch(), &t.pr_link) {
        (None, ..) => "(will open a draft)".to_string(),
        (Some(pr), Some(why), _) => format!("{} (reuse; {why})", pr.url),
        (Some(pr), None, Some(how)) => format!("{} (reuse; {how})", pr.url),
        (Some(pr), None, None) => format!("{} (reuse)", pr.url),
    };
    let worktree = t
        .worktree
        .as_ref()
        .map_or("(will create with kwt)".to_string(), |p| {
            format!("{} (reuse)", p.display())
        });
    let launch = if no_launch {
        "skip (--no-launch)".to_string()
    } else if t.ready() {
        format!("skip ({READY_NO_RELAUNCH})")
    } else {
        "claude in the workspace's tmux pane".to_string()
    };
    format!(
        "Dispatch plan #{}: {}\n  kata:     {kata}\n  branch:   {} \u{2190} {} ({branch_note})\n  \
         PR title: {}{title_note}\n  PR:       {pr}\n  worktree: {worktree}\n  launch:   {launch}\n",
        t.number, t.title, t.branch, t.base, t.pr_title
    )
}

/// Whether the mirror still needs claiming; fails, before anything is
/// mutated, when another actor owns it.
fn needs_claim(owner: Option<&str>, me: &str, kata_ref: &str) -> Result<bool> {
    match owner {
        None | Some("") => Ok(true),
        Some(o) if o == me => Ok(false),
        Some(o) => anyhow::bail!(
            "kata {kata_ref} is claimed by {o}; ask them to release it (kata unassign {kata_ref})"
        ),
    }
}

/// The kata actor, resolved like kata does: the first non-empty of
/// `KATA_AUTHOR` and `USER`, else git's `user.name` (only asked when needed).
fn pick_actor(
    env: [Option<String>; 2],
    git_name: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let non_empty = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    env.into_iter()
        .find_map(non_empty)
        .or_else(|| non_empty(git_name()))
}

/// Refuse a mirror kata no longer treats as open (closed, or drifted from GitHub).
fn check_mirror_open(number: u64, kata_ref: &str, status: &str) -> Result<()> {
    if !status.eq_ignore_ascii_case("open") {
        anyhow::bail!(
            "kata mirror {kata_ref} of #{number} is {status}, not open — \
             run `engram kata sync --issue {number}` to reconcile it"
        );
    }
    Ok(())
}

struct Ctx<'a> {
    root: &'a Path,
    repo: &'a str,
    cfg: &'a Config,
    tools: &'a Toolchain,
    opts: DispatchOptions,
    open_prs: OnceCell<Vec<PrWithBody>>,
    default: OnceCell<String>,
    actor: OnceCell<String>,
}

fn cached<T>(cell: &OnceCell<T>, init: impl FnOnce() -> Result<T>) -> Result<&T> {
    if let Some(v) = cell.get() {
        return Ok(v);
    }
    let v = init()?;
    Ok(cell.get_or_init(|| v))
}

/// Plan, confirm and execute `engram dispatch` for plan issue `number`.
pub fn dispatch(
    repo_root: &Path,
    repo: &str,
    cfg: &Config,
    tools: &Toolchain,
    number: u64,
    opts: DispatchOptions,
) -> Result<()> {
    let cx = Ctx {
        root: repo_root,
        repo,
        cfg,
        tools,
        opts,
        open_prs: OnceCell::new(),
        default: OnceCell::new(),
        actor: OnceCell::new(),
    };
    // Planning: no mutations.
    let mut t = cx.plan_target(number)?;
    print!("{}", describe(&t, opts.no_launch));
    if opts.dry_run {
        return Ok(());
    }
    if !opts.yes && !confirm()? {
        anyhow::bail!("dispatch cancelled");
    }
    cx.execute(&mut t)
}

impl Ctx<'_> {
    /// Open PRs with bodies, listed once per run.
    fn open_prs(&self) -> Result<&[PrWithBody]> {
        cached(&self.open_prs, || {
            github::list_prs_with_body(self.repo, "open").context("listing open PRs")
        })
        .map(Vec::as_slice)
    }

    fn default_branch(&self) -> Result<&str> {
        cached(&self.default, || {
            let head =
                run(git(self.root).args(["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]));
            match head.as_deref().map(|h| h.strip_prefix("origin/")) {
                Ok(Some(name)) if !name.is_empty() => Ok(name.to_string()),
                _ => github::default_branch(self.repo),
            }
        })
        .map(String::as_str)
    }

    fn actor(&self) -> Result<&str> {
        cached(&self.actor, || {
            let env = ["KATA_AUTHOR", "USER"].map(|v| std::env::var(v).ok());
            pick_actor(env, || {
                run(git(self.root).args(["config", "user.name"])).ok()
            })
            .context(
                "can't tell which kata actor you are: set KATA_AUTHOR (or USER, or git user.name)",
            )
        })
        .map(String::as_str)
    }

    /// For a plan with an `Objective: #N (node X)` marker: the objective's
    /// (number, node, title, body). Fails when the objective or node can't be
    /// found, or node X still waits on others.
    fn objective_of(
        &self,
        number: u64,
        plan_body: &str,
    ) -> Result<Option<(u64, String, String, String)>> {
        let Some((obj, node_id)) = objective::parse_objective_marker(plan_body) else {
            return Ok(None);
        };
        let issue = github::get_issue(self.repo, obj).with_context(|| {
            format!("reading objective #{obj} of plan #{number} (node {node_id})")
        })?;
        let body = issue.body.unwrap_or_default();
        let nodes = objective::parse_nodes_from_comment(&body).unwrap_or_default();
        let node = nodes.iter().find(|n| n.id == node_id).with_context(|| {
            format!(
                "plan #{number} names node {node_id} of objective #{obj}, which has no such node"
            )
        })?;
        let blockers = objective::blocked_by(node, &nodes);
        if !blockers.is_empty() {
            anyhow::bail!(
                "plan #{number} is node {node_id} of objective #{obj}, which is blocked \
                 by node(s) {} that are not done yet",
                blockers.join(", ")
            );
        }
        Ok(Some((obj, node_id, issue.title, body)))
    }

    fn mirror_claimed_by_me(
        &self,
        number: u64,
        kata_ref: &str,
    ) -> Result<(bool, BTreeMap<String, String>)> {
        let issue = kata::show(kata_ref)
            .with_context(|| format!("reading kata mirror {kata_ref} of #{number}"))?;
        check_mirror_open(number, kata_ref, &issue.status)?;
        let needs = needs_claim(issue.owner.as_deref(), self.actor()?, kata_ref)?;
        Ok((!needs, kata::metadata_map(&issue.metadata)))
    }

    /// Work out everything dispatch will do for plan `number`, changing
    /// nothing.
    fn plan_target(&self, number: u64) -> Result<Target> {
        let issue = github::get_issue(self.repo, number)
            .with_context(|| format!("fetching issue #{number}"))?;
        if issue.has_label(OBJECTIVE_LABEL) {
            anyhow::bail!("#{number} is an engram-objective, not a plan issue");
        }
        check_kind(number, &issue)?;
        let body = issue.body.unwrap_or_default();
        let objective = self.objective_of(number, &body)?;
        let (mut meta, mut parent) = (kata::TrackerMeta::default(), None);
        let (kata_ref, claimed_by_me, meta_map) = match kata::find_mirror(self.repo, number, &body)?
        {
            Some(r) => {
                let (claimed, map) = self.mirror_claimed_by_me(number, &r)?;
                (Some(r), claimed, map)
            }
            None => {
                (meta, parent) = match &objective {
                    Some((_, _, obj_title, obj_body)) => {
                        let parent = kata::parse_ref(obj_body);
                        let meta = objective::node_tracker_meta(
                            &kata::TrackerMeta::default(),
                            &issue.title,
                            &body,
                            obj_title,
                            obj_body,
                            parent.as_deref(),
                        );
                        (meta, parent)
                    }
                    None => (
                        kata::TrackerMeta::resolve(None, None, &issue.title, &body),
                        None,
                    ),
                };
                (None, false, meta.to_map())
            }
        };
        let mut t = Target {
            number,
            title: issue.title,
            body,
            kata_ref,
            claimed_by_me,
            meta,
            parent,
            kata_ref_pending: false,
            branch: String::new(),
            local: false,
            remote: false,
            pr_title: String::new(),
            base: String::new(),
            pr: None,
            pr_link: None,
            worktree: None,
        };
        let vars = t.vars(t.kata_ref.clone(), meta_map);
        let branch_pending;
        (t.branch, branch_pending) = render_planned(&self.cfg.dispatch.branch, &vars)?;
        let (pr_title, title_pending) = render_planned(&self.cfg.dispatch.pr_title, &vars)?;
        t.pr_title = pr_title;

        // Work started under any branch name is found through the open PR that
        // closes the plan, so a retitled plan or changed template can't start a
        // second branch, PR and session: GitHub's closing link first, else (for
        // stacked PRs, which GitHub never links) `Closes #N` in a PR body,
        // preferring the PR on the branch the template renders.
        let linked = github::open_closing_pr(self.repo, number)
            .with_context(|| format!("looking up open PRs linked to #{number}"))?;
        let found = match linked {
            Some(pr) => Some((pr, format!("linked to #{number}"))),
            None => {
                let head = (!branch_pending).then_some(t.branch.as_str());
                github::select_body_closing_pr(self.open_prs()?, self.repo, number, head)
                    .cloned()
                    .map(|pr| (pr, format!("found by `Closes #{number}` in its body")))
            }
        };
        if let Some((pr, how)) = found {
            t.branch = pr.head.clone();
            t.pr_title = pr.title.clone();
            t.base = pr.base.clone();
            t.pr_link = Some(how);
            t.pr = Some(pr);
            self.lookup_branch(&mut t)?;
            return Ok(t);
        }
        t.kata_ref_pending = branch_pending || title_pending;
        t.base = self.default_branch()?.to_string();
        // A pending branch name is only known once the mirror exists; execute
        // validates and looks it up then.
        if !branch_pending {
            self.lookup_branch(&mut t)?;
        }
        Ok(t)
    }

    fn lookup_branch(&self, t: &mut Target) -> Result<()> {
        let b = &t.branch;
        if run(git(self.root).args(["check-ref-format", "--branch", b])).is_err() {
            anyhow::bail!(
                "`{b}` is not a valid branch name — check the [dispatch] branch template"
            );
        }
        t.local = run(git(self.root).args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{b}"),
        ]))
        .is_ok();
        let output = git(self.root)
            .args([
                "ls-remote",
                "--exit-code",
                "--heads",
                "origin",
                &format!("refs/heads/{b}"),
            ])
            .stdin(Stdio::null())
            .output()
            .context("running git ls-remote")?;
        t.remote = match output.status.code() {
            Some(0) => true,
            Some(LS_REMOTE_NO_MATCH) => false,
            _ => anyhow::bail!(
                "git ls-remote origin failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        };
        if t.pr.is_none() {
            t.pr = github::pr_for_branch(self.repo, b, "open")?;
        }
        // A branch whose PR already merged (e.g. into a stacked base, which
        // leaves the plan open) must not get a second PR from the merged head.
        if t.pr.is_none() && (t.local || t.remote) {
            if let Some(pr) = github::pr_for_branch(self.repo, b, "merged")? {
                anyhow::bail!(
                    "{b} was already merged into {} ({}); not opening another PR from it \u{2014} \
                     mark the plan's node done or run `engram plan land {}`",
                    pr.base,
                    pr.url,
                    t.number
                );
            }
        }
        t.worktree = self.kwt_worktree(&t.branch)?;
        Ok(())
    }

    /// Carry out a confirmed target and print its report. Every step looks up
    /// existing state first, so a re-run after a partial failure resumes.
    fn execute(&self, t: &mut Target) -> Result<()> {
        let (kata_ref, claimed_by_me, created) = match &t.kata_ref {
            Some(r) => (r.clone(), t.claimed_by_me, false),
            // Same idempotency key as `plan new`, so the two paths can't
            // duplicate the mirror; it may return one someone already claimed.
            None => {
                let url = format!("https://github.com/{}/issues/{}", self.repo, t.number);
                let kata_ref = kata::create(
                    &t.title,
                    &kata::mirror_body(&url, &t.body),
                    &kata::plan_idempotency_key(t.number),
                    t.parent.as_deref(),
                    &t.meta,
                )
                .context("creating kata mirror")?;
                eprintln!("kata: created mirror {kata_ref}");
                let owner = kata::show(&kata_ref)
                    .with_context(|| format!("reading kata mirror {kata_ref}"))?
                    .owner;
                let claimed = !needs_claim(owner.as_deref(), self.actor()?, &kata_ref)?;
                (kata_ref, claimed, true)
            }
        };
        if claimed_by_me {
            eprintln!("kata: {kata_ref} already claimed by you");
        } else {
            kata::claim(&kata_ref).with_context(|| format!("claiming kata {kata_ref}"))?;
            eprintln!("kata: claimed {kata_ref}");
        }
        // Only once the claim holds does anything change on GitHub.
        if created {
            let body = kata::with_ref_line(&t.body, &kata_ref);
            if let Err(e) = github::update_issue_body(self.repo, t.number, &body) {
                eprintln!("warning: could not append Kata ref to GitHub issue: {e:#}");
            }
        }
        if t.kata_ref_pending {
            let vars = t.vars(Some(kata_ref.clone()), t.meta.to_map());
            t.branch = render(&self.cfg.dispatch.branch, &vars)?;
            t.pr_title = render(&self.cfg.dispatch.pr_title, &vars)?;
            t.kata_ref_pending = false;
            self.lookup_branch(t)?;
            eprintln!("branch: rendered {} with kata {kata_ref}", t.branch);
        }

        run(git(self.root).args(["fetch", "origin", &t.base]))?;
        self.ensure_branch(t)?;
        if t.remote {
            eprintln!("push: skipped, origin already has {}", t.branch);
        } else {
            run(git(self.root).args(["push", "-u", "origin", &t.branch]))?;
            eprintln!("push: pushed {} to origin", t.branch);
        }

        let pr_url = match &t.pr {
            Some(pr) => {
                t.warn_base_mismatch();
                eprintln!("pr: reusing {}", pr.url);
                pr.url.clone()
            }
            None => {
                let body = pr_body(t.number, &kata_ref);
                let url =
                    github::create_draft_pr(self.repo, &t.branch, &t.base, &t.pr_title, &body)?;
                eprintln!("pr: opened draft {url}");
                url
            }
        };

        let worktree = match &t.worktree {
            Some(path) => {
                eprintln!("worktree: reusing {}", path.display());
                path.clone()
            }
            None => {
                run(self.kwt().args(["add", &t.branch, "--no-launch"]))?;
                let path = self.kwt_worktree(&t.branch)?.with_context(|| {
                    format!("kwt added {} but `kwt list` can't find it", t.branch)
                })?;
                eprintln!("worktree: created {}", path.display());
                path
            }
        };
        let endpoint = self.start_session(&worktree)?;
        let (session, socket) = (&endpoint.session_name, &endpoint.tmux_socket_name);
        eprintln!("tmux: session {session} (socket {socket})");

        let launch = if self.opts.no_launch {
            "not launched (--no-launch)".to_string()
        } else if t.ready() {
            eprintln!("claude: {READY_NO_RELAUNCH}");
            READY_NO_RELAUNCH.to_string()
        } else {
            self.launch_claude(
                t,
                &worktree,
                &endpoint,
                &build_prompt(t, &kata_ref, &pr_url),
            )?
        };
        print!(
            "#{}  kata {kata_ref}  {} \u{2190} {}\n      pr: {pr_url}\n      worktree: {}\n      \
             tmux: {session} (socket {socket})  claude: {launch}\n",
            t.number,
            t.branch,
            t.base,
            worktree.display()
        );
        Ok(())
    }

    /// Create the branch without checking anything out, so the user's checkout
    /// and HEAD stay untouched.
    fn ensure_branch(&self, t: &Target) -> Result<()> {
        let b = &t.branch;
        if t.local {
            eprintln!("branch: {b} exists, reusing it");
        } else if t.remote {
            // Dispatched from another machine: track the remote branch rather
            // than creating a divergent wip commit.
            let refspec = format!("refs/heads/{b}:refs/remotes/origin/{b}");
            run(git(self.root).args(["fetch", "origin", &refspec]))?;
            run(git(self.root).args(["branch", "--track", b, &format!("origin/{b}")]))?;
            eprintln!("branch: {b} tracks origin/{b}");
        } else {
            // Base the wip commit on the freshly fetched remote ref, not local
            // <default>, which can hold unpushed or stale commits.
            let base_ref = format!("origin/{}", t.base);
            let tree = run(git(self.root).args(["rev-parse", &format!("{base_ref}^{{tree}}")]))?;
            let msg = format!("wip: #{} {}", t.number, t.title);
            let commit =
                run(git(self.root).args(["commit-tree", &tree, "-p", &base_ref, "-m", &msg]))?;
            run(git(self.root).args(["branch", b, &commit]))?;
            eprintln!("branch: created {b} on {base_ref}");
        }
        Ok(())
    }

    fn kwt(&self) -> Command {
        let mut cmd = Command::new(&self.tools.kwt);
        cmd.current_dir(self.root);
        cmd
    }

    fn kwt_worktree(&self, branch: &str) -> Result<Option<PathBuf>> {
        let out = run(self.kwt().args(["list", "--json"]))?;
        Ok(select_worktree(&out, branch)?.filter(|p| p.is_dir()))
    }

    fn start_session(&self, worktree: &Path) -> Result<Endpoint> {
        // --start-session is the only mode of `kwt open` that never attaches
        // the calling terminal. kwt runs its own tmux server, so the endpoint
        // comes from its JSON rather than from `tmux ls`. Creating a workspace
        // occasionally fails transiently inside kwt ("respawn pane failed: fork
        // failed"); --start-session is idempotent, so retry.
        let mut attempt = 0;
        loop {
            let open = run(self
                .kwt()
                .arg("open")
                .arg(worktree)
                .args(["--start-session", "--json"]));
            match open {
                Ok(out) => return parse_endpoint(&out),
                Err(e) if attempt + 1 >= KWT_OPEN_ATTEMPTS => return Err(e),
                Err(e) => {
                    attempt += 1;
                    eprintln!(
                        "kwt open failed (attempt {attempt}/{KWT_OPEN_ATTEMPTS}), retrying: {e:#}"
                    );
                    std::thread::sleep(Duration::from_secs(attempt));
                }
            }
        }
    }

    fn launch_claude(
        &self,
        t: &Target,
        worktree: &Path,
        endpoint: &Endpoint,
        prompt: &str,
    ) -> Result<String> {
        let session = endpoint.session_name.as_str();
        // kwt (and Ghosthub) run tmux with TMUX_TMPDIR unset; honouring it here
        // would address a different server than the one holding the workspace.
        let tmux = |args: &[&str]| {
            run(Command::new(&self.tools.tmux)
                .env_remove("TMUX_TMPDIR")
                .args(["-L", &endpoint.tmux_socket_name])
                .args(args))
        };
        let pane = || {
            tmux(&[
                "display-message",
                "-p",
                "-t",
                session,
                "#{pane_current_command}",
            ])
        };
        let current = pane()?;
        if !is_shell(&current) {
            return Ok(format!("already running ({current})"));
        }

        // The prompt lives outside the worktree so nothing from the dispatch
        // can be committed, and the keystrokes stay short and quoting-safe.
        let prompt_file = prompt_path(
            std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
            std::env::var_os("HOME").map(PathBuf::from),
            self.repo,
            t.number,
        )?;
        if let Some(dir) = prompt_file.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&prompt_file, prompt)
            .with_context(|| format!("writing {}", prompt_file.display()))?;

        let claude = claude_command(std::env::var(CLAUDE_OVERRIDE_ENV).ok(), &self.tools.claude);
        let cmd = launch_command(&claude, worktree, &prompt_file);
        tmux(&["send-keys", "-t", session, "-l", &cmd])?;
        tmux(&["send-keys", "-t", session, "Enter"])?;

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
            let out = tmux(&["capture-pane", "-p", "-t", session])?;
            let lines: Vec<&str> = out.lines().collect();
            let tail = &lines[lines.len().saturating_sub(PANE_TAIL_LINES)..];
            anyhow::bail!(
                "Claude exited right after launch; pane shows:\n        {}",
                tail.join("\n        ")
            );
        }
        Ok(LAUNCHED.to_string())
    }
}

fn pr_body(number: u64, kata_ref: &str) -> String {
    format!("Work in progress for engram plan #{number} (kata `{kata_ref}`).\n\nCloses #{number}")
}

fn build_prompt(t: &Target, kata_ref: &str, pr_url: &str) -> String {
    let n = t.number;
    format!(
        "Work on engram plan #{n}: {}\nkata issue: {kata_ref}\n\n{}\n\n---\n\
         A draft pull request is already open for this branch: {pr_url} (base `{}`).\n\
         Do not open a new PR and do not change its base.\n\
         Keep the `Closes #{n}` line in the PR body if you edit it.\n\
         When the implementation is complete and tests pass, push your commits to this branch \
         and run `gh pr ready` on the existing PR, adding a summary comment of what changed and why.\n",
        t.title,
        t.body.trim_end(),
        t.pr_base()
    )
}

#[derive(Debug, Deserialize)]
struct KwtEntry {
    path: PathBuf,
    #[serde(default)]
    branch: Option<String>,
}

/// Path of the worktree whose branch is exactly `branch`. `kwt get` can't be
/// used: it matches fuzzily and may return another plan's worktree.
fn select_worktree(json: &str, branch: &str) -> Result<Option<PathBuf>> {
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
struct Endpoint {
    #[serde(default)]
    session_name: String,
    #[serde(default)]
    tmux_socket_name: String,
}

fn parse_endpoint(json: &str) -> Result<Endpoint> {
    let endpoint: Endpoint =
        serde_json::from_str(json).context("parsing `kwt open --json` output")?;
    if endpoint.session_name.is_empty() || endpoint.tmux_socket_name.is_empty() {
        anyhow::bail!("kwt open returned no tmux endpoint: {}", json.trim());
    }
    Ok(endpoint)
}

fn is_shell(pane_command: &str) -> bool {
    SHELLS.contains(&pane_command.trim().trim_start_matches('-'))
}

fn shell_quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r#"'"'"'"#))
}

/// `$XDG_STATE_HOME/engram/dispatch/<owner>-<name>-<N>.md`, the state home
/// defaulting to `~/.local/state`: outside every worktree.
fn prompt_path(
    state_home: Option<PathBuf>,
    home: Option<PathBuf>,
    repo: &str,
    number: u64,
) -> Result<PathBuf> {
    let state_home = match state_home.filter(|p| !p.as_os_str().is_empty()) {
        Some(p) => p,
        None => home
            .context("HOME is not set, so the dispatch prompt file has nowhere to go")?
            .join(".local/state"),
    };
    Ok(state_home
        .join(PROMPT_SUBDIR)
        .join(format!("{}-{number}.md", repo.replace('/', "-"))))
}

/// `ENGRAM_DISPATCH_CLAUDE` (verbatim), else the resolved `claude` path.
fn claude_command(env_override: Option<String>, resolved: &Path) -> String {
    env_override
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| shell_quote(&resolved.to_string_lossy()))
}

fn launch_command(claude: &str, worktree: &Path, prompt_file: &Path) -> String {
    // The pane's shell can start outside the worktree: it inherits the tmux
    // server's cwd, which may be a since-deleted directory, and Claude refuses
    // to start there. cd explicitly rather than trusting the pane.
    let cd = format!("cd {} && ", shell_quote(&worktree.to_string_lossy()));
    format!(
        "{cd}{claude} --permission-mode auto \"$(cat {})\"",
        shell_quote(&prompt_file.to_string_lossy()),
    )
}

fn git(repo_root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_root);
    cmd
}

fn run(cmd: &mut Command) -> Result<String> {
    let desc = std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|a| a.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let output = cmd
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running `{desc}`"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() {
            &stdout
        } else {
            stderr.trim()
        };
        anyhow::bail!("`{desc}` failed: {detail}");
    }
    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn plan(kata_ref: Option<&str>) -> PlanFields {
        PlanFields {
            number: 142,
            kata_ref: kata_ref.map(Into::into),
            title: "Add dispatch preflight!".into(),
            meta: BTreeMap::from([
                ("jira".into(), "ENG-7".into()),
                ("type".into(), "feat".into()),
            ]),
        }
    }

    /// The expected value, or text the error's full message contains.
    type Want<T> = std::result::Result<T, &'static str>;

    fn assert_result<T: PartialEq + std::fmt::Debug>(got: Result<T>, want: Want<T>) {
        match (got, want) {
            (Ok(got), Ok(want)) => assert_eq!(got, want),
            (Err(e), Err(part)) => assert!(format!("{e:#}").contains(part), "{e:#}"),
            (got, want) => panic!("got {got:?}, want {want:?}"),
        }
    }

    fn assert_mentions(text: &str, has: &[&str], lacks: &[&str]) {
        for part in has {
            assert!(text.contains(part), "{part:?} missing from\n{text}");
        }
        for part in lacks {
            assert!(!text.contains(part), "{part:?} unexpected in\n{text}");
        }
    }

    #[rstest]
    #[case::all_placeholders(
        "{meta[type]}/{meta[jira]}/{number}-{slug}: {title}",
        Some("a56d"),
        Ok("feat/ENG-7/142-add-dispatch-preflight: Add dispatch preflight!")
    )]
    #[case::stray_close_brace_is_literal("{number}}", None, Ok("142}"))]
    #[case::missing_meta(
        "{meta[team]}-{meta[jira]}-{meta[area]}-{meta[team]}",
        Some("a56d"),
        Err(
            "plan #142 has no kata metadata 'team', 'area' \
             (kata meta set a56d team <value>; kata meta set a56d area <value>)"
        )
    )]
    #[case::missing_meta_without_mirror("{meta[team]}", None, Err("kata meta set <kata-ref> team"))]
    #[case::kata_ref_is_gone("{kata_ref}", Some("a56d"), Err("unknown placeholder `{kata_ref}`"))]
    #[case::unknown_placeholder("{short_id}-{slug}", None, Err("unknown placeholder `{short_id}`"))]
    #[case::unclosed("{number", None, Err("unclosed `{`"))]
    fn fill_template_fills_placeholders_or_explains(
        #[case] template: &str,
        #[case] kata_ref: Option<&str>,
        #[case] want: std::result::Result<&str, &str>,
    ) {
        match (fill_template(template, &plan(kata_ref)), want) {
            (Ok(got), Ok(want)) => assert_eq!(got, want),
            (Err(e), Err(part)) => assert!(e.to_string().contains(part), "{e}"),
            (got, want) => panic!("got {got:?}, want {want:?}"),
        }
    }

    #[rstest]
    #[case::punctuation_and_dash_runs("Fix: the -- login  race!", "fix-the-login-race")]
    #[case::nothing_left("!!! ??? ...", "")]
    #[case::node_prefix_dropped_and_cut_at_a_word(
        "[5.1] Add dispatch preflight, [dispatch] config and doctor check",
        "add-dispatch-preflight-dispatch-config"
    )]
    #[case::numeric_node_prefix_dropped("[12] Fix", "fix")]
    #[case::other_brackets_kept("[ENG-7] Fix login", "eng-7-fix-login")]
    // An empty id part means it isn't a node id.
    #[case::not_a_node_id("[5.] Fix", "5-fix")]
    #[case::one_long_word_cut_hard(&"x".repeat(50), &"x".repeat(SLUG_MAX))]
    fn slugify_cases(#[case] title: &str, #[case] slug: &str) {
        assert_eq!(slugify(title), slug);
    }

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

    #[cfg(unix)]
    #[test]
    fn resolve_tool_prefers_path_then_executable_fallback() {
        use std::os::unix::fs::PermissionsExt;
        let exe = |dir: &Path| {
            let p = dir.join("kwt");
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        let [bin, other, empty] = [(); 3].map(|_| tempfile::tempdir().unwrap());
        let (on_path, fallback) = (exe(bin.path()), exe(other.path()));
        let path = |d: &tempfile::TempDir| std::env::join_paths([d.path()]).unwrap();
        let kwt = |d| resolve_tool("kwt", Some(&path(d)), Some(&fallback));
        assert_eq!(kwt(&bin), Some(on_path));
        assert_eq!(kwt(&empty), Some(fallback.clone()));
        // Present but not executable, and no fallback.
        std::fs::write(empty.path().join("tmux"), "").unwrap();
        assert_eq!(resolve_tool("tmux", Some(&path(&empty)), None), None);
    }

    #[rstest]
    #[case::missing_tools(&["kwt", "tmux"], true, true, &["missing: kwt, tmux"])]
    #[case::unbound(&[], true, false, &["repo not bound to kata, run `kata init`"])]
    #[case::binding_unchecked_without_kata(&["kata"], false, false, &["missing: kata"])]
    #[case::ready(&[], true, true, &[])]
    fn tool_problems_cases(
        #[case] missing: &[&str],
        #[case] kata_installed: bool,
        #[case] kata_bound: bool,
        #[case] want: &[&str],
    ) {
        assert_eq!(tool_problems(missing, kata_installed, kata_bound), want);
    }

    fn target() -> Target {
        Target {
            number: 143,
            title: "[5.2] Add engram dispatch".into(),
            body: "Objective: #141 (node 5.2)\n\n## Why\nUse `$HOME` and \"quotes\".\n".into(),
            kata_ref: Some("mfbq".into()),
            claimed_by_me: false,
            meta: kata::TrackerMeta::default(),
            parent: None,
            kata_ref_pending: false,
            branch: "143-add-engram-dispatch".into(),
            local: false,
            remote: false,
            pr_title: "[5.2] Add engram dispatch".into(),
            base: "main".into(),
            pr: None,
            pr_link: None,
            worktree: None,
        }
    }

    fn with_pr(mut t: Target, base: &str, draft: bool) -> Target {
        t.pr = Some(OpenPr {
            url: "https://github.com/o/r/pull/9".into(),
            base: base.into(),
            head: t.branch.clone(),
            is_draft: draft,
            title: t.pr_title.clone(),
        });
        t
    }

    /// Found by its body, ready for review, its branch also local; `base` is
    /// the computed one, the PR's own is `main`.
    fn reused(base: &str) -> Target {
        let mut t = with_pr(target(), "main", false);
        t.pr_link = Some("found by `Closes #143` in its body".into());
        t.local = true;
        t.base = base.into();
        t
    }

    #[rstest]
    #[case::fresh(None, "main", &["(base `main`)"], &[])]
    // A reused PR's real base wins over the computed one.
    #[case::reused_pr(Some("old-base"), "computed-base", &["(base `old-base`)"], &["computed-base"])]
    fn prompt_carries_plan_and_base(
        #[case] pr_base: Option<&str>,
        #[case] base: &str,
        #[case] has: &[&str],
        #[case] lacks: &[&str],
    ) {
        let mut t = pr_base.map_or_else(target, |b| with_pr(target(), b, true));
        t.base = base.into();
        let p = build_prompt(&t, "mfbq", "https://github.com/o/r/pull/9");
        let always = [
            "Work on engram plan #143: [5.2] Add engram dispatch\nkata issue: mfbq\n\n",
            "## Why\nUse `$HOME` and \"quotes\".\n\n---\n",
            "https://github.com/o/r/pull/9 (base `",
            "Keep the `Closes #143` line",
            "`gh pr ready`",
        ];
        assert_mentions(&p, &[&always[..], has].concat(), lacks);
    }

    #[test]
    fn pr_body_closes_the_plan() {
        assert_eq!(
            pr_body(143, "mfbq"),
            "Work in progress for engram plan #143 (kata `mfbq`).\n\nCloses #143"
        );
    }

    #[rstest]
    #[case::fresh(target(), false, &[
        "kata:     mfbq (will claim)",
        "143-add-engram-dispatch \u{2190} main (create from origin with an empty wip commit)",
        "PR:       (will open a draft)\n  worktree: (will create with kwt)\n  launch:   claude",
    ])]
    #[case::reused_ready_pr(reused("main"), false, &[
        "(reused from the existing PR; reuse local branch)",
        "PR title: [5.2] Add engram dispatch (existing PR)",
        "pull/9 (reuse; found by `Closes #143` in its body)",
        "skip (PR is ready for review \u{2014} not relaunching)",
    ])]
    #[case::reused_pr_on_another_base(reused("a56d-x"), true, &[
        "pull/9 (reuse; targets main, not a56d-x)",
        "skip (--no-launch)",
    ])]
    #[case::mirror_pending(Target { kata_ref: None, kata_ref_pending: true, ..target() }, false, &[
        "(no mirror, will create and claim)", "<kata-ref> filled in once the mirror exists",
    ])]
    fn describe_says_what_is_reused_and_how(
        #[case] t: Target,
        #[case] no_launch: bool,
        #[case] has: &[&str],
    ) {
        assert_mentions(&describe(&t, no_launch), has, &[]);
    }

    #[rstest]
    #[case::open_plan("OPEN", &[PLAN_LABEL], Ok(()))]
    #[case::unlabelled("OPEN", &[], Err("not an engram-plan issue"))]
    #[case::objective("OPEN", &[OBJECTIVE_LABEL], Err("not an engram-plan issue"))]
    #[case::closed("CLOSED", &[PLAN_LABEL], Err("not open"))]
    fn check_kind_refuses_closed_and_unlabelled_issues(
        #[case] state: &str,
        #[case] labels: &[&str],
        #[case] want: Want<()>,
    ) {
        let labels = labels.iter().map(|l| github::Label { name: (*l).into() });
        let issue = github::Issue {
            title: String::new(),
            body: None,
            state: state.into(),
            labels: labels.collect(),
        };
        assert_result(check_kind(1, &issue), want);
    }

    #[rstest]
    #[case::kata_author_first([Some("ka"), Some("u")], Some("g"), Some("ka"))]
    #[case::user_trimmed([None, Some(" u ")], Some("g"), Some("u"))]
    #[case::blanks_fall_back_to_git([Some(""), Some(" ")], Some("g"), Some("g"))]
    #[case::nobody([None, None], Some(""), None)]
    fn pick_actor_prefers_kata_author_then_user_then_git(
        #[case] env: [Option<&str>; 2],
        #[case] git: Option<&str>,
        #[case] want: Option<&str>,
    ) {
        let env = env.map(|v| v.map(String::from));
        assert_eq!(pick_actor(env, || git.map(String::from)).as_deref(), want);
    }

    #[rstest]
    #[case::open("OPEN", Ok(()))]
    #[case::closed("closed", Err("mfbq of #143 is closed"))]
    fn check_mirror_open_refuses_non_open(#[case] status: &str, #[case] want: Want<()>) {
        assert_result(check_mirror_open(143, "mfbq", status), want);
    }

    #[rstest]
    #[case::unowned(None, Ok(true))]
    #[case::blank_owner(Some(""), Ok(true))]
    #[case::mine(Some("me"), Ok(false))]
    #[case::someone_else(Some("bot"), Err("claimed by bot"))]
    fn needs_claim_by_owner(#[case] owner: Option<&str>, #[case] want: Want<bool>) {
        assert_result(needs_claim(owner, "me", "ab12"), want);
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

    #[test]
    fn launch_command_cds_into_the_quoted_worktree() {
        assert_eq!(
            launch_command(
                "'/Apps/My Claude/claude'",
                Path::new("/wt/my tree"),
                Path::new("/p.md")
            ),
            "cd '/wt/my tree' && '/Apps/My Claude/claude' --permission-mode auto \"$(cat /p.md)\""
        );
    }

    #[rstest]
    #[case::env_override(Some("fake"), "fake")]
    #[case::blank_env_ignored(Some(""), "'/Apps/My Claude/claude'")]
    #[case::resolved_path_quoted(None, "'/Apps/My Claude/claude'")]
    fn claude_command_precedence(#[case] env: Option<&str>, #[case] want: &str) {
        let resolved = Path::new("/Apps/My Claude/claude");
        assert_eq!(claude_command(env.map(Into::into), resolved), want);
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

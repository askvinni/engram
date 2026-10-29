//! `engram dispatch`: hand plan issues to background Claude sessions.
//!
//! Dispatch is the only engram command that needs kwt, tmux and kata, so it
//! checks for them up front (`preflight`) instead of degrading silently.
//!
//! Plans and objectives share one pipeline: collect the plan issues to
//! dispatch (an objective contributes its unblocked nodes' plans), plan each
//! without mutating anything, confirm, then carry them out one at a time.
//! `--stack` follows the same steps for one objective's dependency chains: a
//! stack is only an ordering plus a base policy (each node on its
//! dependency's branch).

use anyhow::{Context, Result};
use serde::Deserialize;
use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::config::{self, Config};
use crate::github::{self, OpenPr, PrState, PrWithBody, OBJECTIVE_LABEL, PLAN_LABEL};
use crate::kata;
use crate::objective::{self, NodeStatus, ObjectiveNode};
use crate::process::{self, git, git_remote, run_checked};

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
const SHELLS: &[&str] = &["zsh", "bash", "fish", "sh", "dash", "ksh"];
/// How long to wait for Claude to replace the shell in a pane, then to stay there.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(15);
const LAUNCH_SETTLE: Duration = Duration::from_secs(3);
const LAUNCH_POLL: Duration = Duration::from_millis(500);
/// `kwt open --start-session` attempts, waiting `KWT_OPEN_BACKOFF` times
/// the attempt number between them (1s, then 2s).
const KWT_OPEN_ATTEMPTS: u64 = 3;
const KWT_OPEN_BACKOFF: Duration = Duration::from_secs(1);
const PANE_TAIL_LINES: usize = 5;
/// Overrides the launched command above `[dispatch] claude` (verbatim; for tests).
const CLAUDE_OVERRIDE_ENV: &str = "ENGRAM_DISPATCH_CLAUDE";
/// Prompt files live under `$XDG_STATE_HOME/<PROMPT_SUBDIR>`, outside every worktree.
const PROMPT_SUBDIR: &str = "engram/dispatch";
const XDG_STATE_HOME: &str = "XDG_STATE_HOME";
/// `$XDG_STATE_HOME` when unset, relative to `$HOME`.
const DEFAULT_STATE_HOME: &str = ".local/state";
const READY_NO_RELAUNCH: &str = "PR is ready for review \u{2014} not relaunching";
/// Where kata reads the actor from, in its order (then git's `user.name`).
const ACTOR_ENV_VARS: [&str; 2] = ["KATA_AUTHOR", "USER"];
/// `NAME=value` words whose name contains one of these are redacted for display.
const CREDENTIAL_MARKERS: &[&str] = &["TOKEN", "SECRET", "KEY", "PASSWORD"];
/// Permission mode the dispatched Claude session starts in.
const PERMISSION_MODE: &str = "auto";
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
    let kata_bound = repo_root.join(kata::KATA_CONFIG).exists();
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
/// (`capture::slugify` makes memory file names and keeps every word.)
fn branch_slug(title: &str) -> String {
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
        return Err(bad(missing_meta_hint(plan, &missing_meta)));
    }
    Ok(out)
}

/// Which metadata keys plan #N lacks, and how to supply each: kata metadata
/// on an existing mirror, else (the mirror doesn't exist yet) a line in the
/// plan body, which only `jira` and `type` can come from.
fn missing_meta_hint(plan: &PlanFields, keys: &[&str]) -> String {
    let quoted: Vec<String> = keys.iter().map(|k| format!("'{k}'")).collect();
    let hints: Vec<String> = keys
        .iter()
        .map(|&key| match (plan.kata_ref.as_deref(), key) {
            (Some(r), _) => format!("kata meta set {r} {key} <value>"),
            (None, kata::JIRA_META_KEY) => "add a `Jira: KEY-1` line to the plan issue body".into(),
            (None, kata::TYPE_META_KEY) => "add a `Type: feat` line to the plan issue body".into(),
            (None, _) => format!(
                "'{key}' only comes from kata metadata: mirror the plan in kata, then \
                 `kata meta set <kata-ref> {key} <value>`"
            ),
        })
        .collect();
    format!(
        "plan #{} has no metadata {} ({})",
        plan.number,
        quoted.join(", "),
        hints.join("; ")
    )
}

/// The value of a non-meta placeholder, or `None` if `name` isn't one.
fn placeholder_value(name: &str, plan: &PlanFields) -> Option<String> {
    match name {
        "number" => Some(plan.number.to_string()),
        "title" => Some(plan.title.clone()),
        "slug" => Some(branch_slug(&plan.title)),
        _ => None,
    }
}

/// `fill_template` for `[dispatch] pr_title`, refusing a blank title that
/// `gh pr create` would only reject after the branch was pushed.
fn fill_pr_title(template: &str, plan: &PlanFields) -> Result<String> {
    let title = fill_template(template, plan)?;
    if title.trim().is_empty() {
        anyhow::bail!(
            "[dispatch] pr_title `{template}` renders an empty PR title for plan #{}",
            plan.number
        );
    }
    Ok(title)
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
    /// Dispatch one objective's dependency chains as stacked PRs, each
    /// based on its dependency's branch
    #[arg(long)]
    pub stack: bool,
}

/// What a stack node's branch starts from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Base {
    /// The default branch.
    Main,
    /// The branch of the node with this ID.
    OnNode(String),
}

/// An objective node in stack order.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StackedNode {
    node_id: String,
    base: Base,
}

/// An objective's unfinished nodes in stack order, and the ones left out.
#[derive(Debug, PartialEq)]
struct StackOrder {
    stacked: Vec<StackedNode>,
    /// (node ID, why it is skipped)
    skipped: Vec<(String, String)>,
}

/// A stack node's branch and PR URL (while planning, the branch stands in
/// for a PR not opened yet): what dependents start from and what is linked.
#[derive(Debug, Clone)]
struct PlacedNode {
    branch: String,
    pr: String,
}

impl PlacedNode {
    fn of(plan: &DispatchPlan) -> Self {
        let pr = plan.pr.as_ref().map_or(&plan.branch, |pr| &pr.url);
        PlacedNode {
            branch: plan.branch.clone(),
            pr: pr.clone(),
        }
    }
}

/// Refuse a pull request or an issue that isn't open.
fn require_open_issue(number: u64, issue: &github::Issue) -> Result<()> {
    if issue.is_pull_request() {
        anyhow::bail!("#{number} is a pull request, not an engram-plan or engram-objective issue");
    }
    if !issue.is_open() {
        let hint = if issue.has_label(PLAN_LABEL) {
            format!("; if its work merged, run `engram plan land {number}`")
        } else {
            String::new()
        };
        anyhow::bail!("#{number} is not open (state: {}){hint}", issue.state);
    }
    Ok(())
}

fn neither_plan_nor_objective(number: u64) -> anyhow::Error {
    anyhow::anyhow!(
        "#{number} is neither an engram-plan nor an engram-objective issue \
         (no `{PLAN_LABEL}` label and no objective node list)"
    )
}

/// Refuse anything but an open plan (plan label) or objective (node list; the
/// objective label alone is an error), including plain kata issues.
fn require_plan_or_objective(number: u64, issue: &github::Issue, has_nodes: bool) -> Result<()> {
    require_open_issue(number, issue)?;
    if has_nodes {
        return Ok(());
    }
    if issue.has_label(OBJECTIVE_LABEL) {
        anyhow::bail!(
            "#{number} is an engram-objective without a node list — \
             was it created with `engram objective new`?"
        );
    }
    if !issue.has_label(PLAN_LABEL) {
        return Err(neither_plan_nor_objective(number));
    }
    Ok(())
}

/// Refuse anything but an open plan issue.
fn require_plan(number: u64, issue: &github::Issue) -> Result<()> {
    if issue.has_label(OBJECTIVE_LABEL) {
        anyhow::bail!("#{number} is an engram-objective, not a plan issue");
    }
    require_open_issue(number, issue)?;
    if !issue.has_label(PLAN_LABEL) {
        return Err(neither_plan_nor_objective(number));
    }
    Ok(())
}

/// Unfinished nodes that wait on nothing. `in_progress` counts: the reuse
/// checks (open PR, worktree, running Claude, ready PR) keep a re-run from
/// launching twice.
fn ready_nodes(nodes: &[ObjectiveNode]) -> Vec<&ObjectiveNode> {
    nodes
        .iter()
        .filter(|n| n.status != NodeStatus::Done && objective::blocked_by(n, nodes).is_empty())
        .collect()
}

fn blocked_nodes(nodes: &[ObjectiveNode]) -> Vec<(String, Vec<&str>)> {
    nodes
        .iter()
        .filter(|n| n.status != NodeStatus::Done)
        .map(|n| (n.id.clone(), objective::blocked_by(n, nodes)))
        .filter(|(_, b)| !b.is_empty())
        .collect()
}

/// Order an objective's unfinished nodes for a stack: dependencies first, ties
/// in objective order. No unfinished dependency: based on the default branch;
/// one: on its branch; several: on the one that transitively depends on all
/// the others, else skipped as a diamond (and so are its dependents). Returns
/// the nodes in order and the skipped ones; fails on a cycle.
fn order_for_stack(nodes: &[ObjectiveNode]) -> Result<StackOrder> {
    let unfinished: Vec<&ObjectiveNode> = nodes
        .iter()
        .filter(|n| n.status != NodeStatus::Done)
        .collect();
    // Each node's unfinished dependencies, deduplicated, in declaration order.
    let deps: BTreeMap<&str, Vec<&str>> = unfinished
        .iter()
        .map(|n| {
            let mut seen = BTreeSet::new();
            let mut d = objective::blocked_by(n, nodes);
            d.retain(|id| seen.insert(*id));
            (n.id.as_str(), d)
        })
        .collect();
    let deps_of = |n: &ObjectiveNode| deps[n.id.as_str()].as_slice();
    let mut ordered: Vec<&ObjectiveNode> = Vec::new();
    let mut placed: BTreeSet<&str> = BTreeSet::new();
    while let Some(next) = unfinished
        .iter()
        .find(|n| !placed.contains(n.id.as_str()) && deps_of(n).iter().all(|d| placed.contains(d)))
    {
        placed.insert(&next.id);
        ordered.push(next);
    }
    if ordered.len() < unfinished.len() {
        let stuck: Vec<&str> = unfinished
            .iter()
            .map(|n| n.id.as_str())
            .filter(|id| !placed.contains(id))
            .collect();
        anyhow::bail!(
            "dependency cycle among objective nodes {}; fix their depends_on",
            stuck.join(", ")
        );
    }

    let mut stacked = Vec::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    for n in ordered {
        let deps = deps_of(n);
        if let Some(dep) = deps.iter().find(|d| skipped.iter().any(|(id, _)| id == *d)) {
            skipped.push((n.id.clone(), format!("depends on skipped node {dep}")));
            continue;
        }
        let covering = deps.iter().find(|d| {
            let above = objective::unfinished_ancestors(d, nodes);
            deps.iter().all(|o| o == *d || above.contains(*o))
        });
        let base = match (deps, covering) {
            ([], _) => Base::Main,
            (_, Some(d)) => Base::OnNode(d.to_string()),
            (_, None) => {
                skipped.push((
                    n.id.clone(),
                    format!(
                        "diamond: waits on {} and none of them depends on the others; \
                         dispatch it once all but one are done",
                        deps.join(", ")
                    ),
                ));
                continue;
            }
        };
        stacked.push(StackedNode {
            node_id: n.id.clone(),
            base,
        });
    }
    Ok(StackOrder { stacked, skipped })
}

/// Split stacked nodes into maximal linear chains, bottom to top, each with
/// the base its bottom sits on. A chain starts on the default branch or below
/// a node with several stacked children, and follows single children.
fn straight_chains(nodes: &[StackedNode]) -> Vec<(Base, Vec<String>)> {
    let children = |id: &str| -> Vec<&str> {
        nodes
            .iter()
            .filter(|t| t.base == Base::OnNode(id.to_string()))
            .map(|t| t.node_id.as_str())
            .collect()
    };
    nodes
        .iter()
        .filter(|t| match &t.base {
            Base::Main => true,
            Base::OnNode(parent) => children(parent).len() != 1,
        })
        .map(|t| {
            let mut chain = vec![t.node_id.clone()];
            while let [only] = children(&chain[chain.len() - 1])[..] {
                chain.push(only.to_string());
            }
            (t.base.clone(), chain)
        })
        .collect()
}

/// The chains worth `gh stack link`: two or more PRs, at least one of them
/// newly dispatched. Chains made only of reused PRs are left as they are.
fn chains_to_link(nodes: &[StackedNode], fresh: &BTreeSet<String>) -> Vec<(Base, Vec<String>)> {
    let mut chains = straight_chains(nodes);
    chains.retain(|(_, chain)| chain.len() >= 2 && chain.iter().any(|id| fresh.contains(id)));
    chains
}

/// The branch a stack node (or chain) based on `base` starts from: `main`,
/// or its dependency's branch once that is placed (`None` until then).
fn base_branch(base: &Base, main: &str, placed: &BTreeMap<String, PlacedNode>) -> Option<String> {
    match base {
        Base::Main => Some(main.to_string()),
        Base::OnNode(id) => placed.get(id).map(|p| p.branch.clone()),
    }
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

/// Where a plan's branch already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BranchState {
    /// A local branch (whether or not origin has it).
    Local,
    /// Only on origin, e.g. dispatched from another machine.
    Remote,
    /// Nowhere yet.
    Missing,
}

impl BranchState {
    fn of(local: bool, on_origin: bool) -> Self {
        match (local, on_origin) {
            (true, _) => BranchState::Local,
            (false, true) => BranchState::Remote,
            (false, false) => BranchState::Missing,
        }
    }

    /// What dispatch will do with the branch.
    fn plan(self) -> &'static str {
        match self {
            BranchState::Local => "reuse local branch",
            BranchState::Remote => "track existing origin branch",
            BranchState::Missing => "create from origin with an empty wip commit",
        }
    }
}

/// How an open PR closing the plan was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrLink {
    /// GitHub links it as closing the plan.
    Linked,
    /// Its body says `Closes #N` (stacked PRs, which GitHub never links).
    BodyKeyword,
}

impl PrLink {
    fn describe(self, number: u64) -> String {
        match self {
            PrLink::Linked => format!("linked to #{number}"),
            PrLink::BodyKeyword => format!("found by `Closes #{number}` in its body"),
        }
    }
}

#[derive(Debug)]
struct DispatchPlan {
    number: u64,
    title: String,
    body: String,
    /// The plan's kata mirror; without one, `carry_out` creates it as `plan new`
    /// would, with `meta` and `parent`.
    kata_ref: Option<String>,
    claimed_by_me: bool,
    meta: kata::TrackerMeta,
    parent: Option<String>,
    branch: String,
    branch_state: BranchState,
    /// Whether origin has the branch (a local branch may be pushed or not).
    on_origin: bool,
    pr_title: String,
    /// The base the PR targets: the default branch, a found PR's base, or in a
    /// stack the dependency's branch.
    base: String,
    pr: Option<OpenPr>,
    /// How an open PR closing the plan was found; its branch and title then
    /// replace the templates'.
    pr_link: Option<PrLink>,
    worktree: Option<PathBuf>,
    /// `(objective, node ID)` when the plan's marker names an existing node.
    objective_node: Option<(u64, String)>,
    /// In a stack, the dependency branch the PR is based on (`None` on the
    /// default branch and outside a stack).
    stacked_on: Option<String>,
}

impl DispatchPlan {
    /// Base a stack node on `base`; a reused PR keeps its own base.
    fn set_stack_base(&mut self, base: String, main: &str) {
        self.base = base;
        let pr_base = self.pr_base();
        self.stacked_on = (pr_base != main).then(|| pr_base.to_string());
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

    /// The warning to print when reusing a PR whose base isn't the planned one.
    fn base_mismatch_warning(&self) -> Option<String> {
        let (pr, why) = (self.pr.as_ref()?, self.base_mismatch()?);
        Some(format!("warning: existing PR {} {why}", pr.url))
    }
}

fn format_plan(t: &DispatchPlan, no_launch: bool, claude: &str) -> String {
    let kata = match &t.kata_ref {
        Some(r) if t.claimed_by_me => format!("{r} (claimed by you)"),
        Some(r) => format!("{r} (will claim)"),
        None => "(no mirror, will create and claim)".to_string(),
    };
    let state = t.branch_state.plan();
    let branch_note = if t.pr_link.is_some() {
        format!("reused from the existing PR; {state}")
    } else {
        state.to_string()
    };
    let title_note = if t.pr.is_some() { " (existing PR)" } else { "" };
    let pr = match (&t.pr, t.base_mismatch(), &t.pr_link) {
        (None, ..) => "(will open a draft)".to_string(),
        (Some(pr), Some(why), _) => format!("{} (reuse; {why})", pr.url),
        (Some(pr), None, Some(how)) => format!("{} (reuse; {})", pr.url, how.describe(t.number)),
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
        format!("{} in the workspace's tmux pane", redact_secrets(claude))
    };
    format!(
        "Dispatch plan #{}: {}\n  kata:     {kata}\n  branch:   {} \u{2190} {} ({branch_note})\n  \
         PR title: {}{title_note}\n  PR:       {pr}\n  worktree: {worktree}\n  launch:   {launch}\n",
        t.number, t.title, t.branch, t.base, t.pr_title
    )
}

fn format_ready_base(t: &DispatchPlan) -> String {
    let url = t.pr.as_ref().map_or("", |pr| pr.url.as_str());
    let mismatch = t
        .base_mismatch()
        .map(|w| format!(" ({w})"))
        .unwrap_or_default();
    format!(
        "plan #{}: {}\n  PR {url}{mismatch} is ready for review \u{2014} only used as a base \
         ({} \u{2190} {}); no claim, worktree or session\n",
        t.number, t.title, t.branch, t.base
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

/// Record missing plan issues and mark nodes in progress (done stays done;
/// unknown IDs are skipped). Returns the IDs that changed, each once.
fn merge_node_progress(nodes: &mut [ObjectiveNode], updates: &[(String, u64)]) -> Vec<String> {
    let mut changed: Vec<String> = Vec::new();
    for (id, plan) in updates {
        let Some(node) = nodes.iter_mut().find(|n| &n.id == id) else {
            continue;
        };
        let mut this = node.plan_issue.is_none();
        node.plan_issue.get_or_insert(*plan);
        if node.status == NodeStatus::Pending {
            node.status = NodeStatus::InProgress;
            this = true;
        }
        if this && !changed.contains(id) {
            changed.push(id.clone());
        }
    }
    changed
}

/// Re-fetch the objective right before writing and apply only node status and
/// plan-issue changes, so edits made since the run started aren't overwritten.
fn record_node_progress(repo: &str, number: u64, updates: &[(String, u64)]) -> Result<()> {
    let body = github::get_issue(repo, number)
        .context("re-fetching objective")?
        .body
        .unwrap_or_default();
    let Some(mut nodes) = objective::parse_nodes_from_comment(&body) else {
        eprintln!("warning: objective #{number} no longer has a node list; not updating it");
        return Ok(());
    };
    let changed = merge_node_progress(&mut nodes, updates);
    if changed.is_empty() {
        return Ok(());
    }
    github::update_issue_body(
        repo,
        number,
        &objective::build_objective_body(&body, &nodes)?,
    )
    .with_context(|| format!("updating objective #{number}"))?;
    println!(
        "Objective #{number}: recorded node(s) {} as in progress",
        changed.join(", ")
    );
    Ok(())
}

/// How planning went: plans to dispatch, how many of those are reused as
/// already ready, and how many failed to plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlanCounts {
    planned: usize,
    ready: usize,
    failed: usize,
}

impl PlanCounts {
    fn of<'a>(plans: impl IntoIterator<Item = &'a DispatchPlan>, failed: usize) -> Self {
        let (mut planned, mut ready) = (0, 0);
        for p in plans {
            planned += 1;
            ready += usize::from(p.ready());
        }
        PlanCounts {
            planned,
            ready,
            failed,
        }
    }

    /// The line printed once planning is done.
    fn line(self, no_launch: bool) -> String {
        let PlanCounts {
            planned,
            ready,
            failed,
        } = self;
        let launches = if no_launch { 0 } else { planned - ready };
        let failed = if failed > 0 {
            format!("; {failed} failed to plan")
        } else {
            String::new()
        };
        format!(
            "{planned} plan(s) to dispatch ({ready} already ready for review, not relaunched); \
             up to {launches} Claude session(s) will start{failed}"
        )
    }
}

/// How carrying out the plans went.
#[derive(Debug, Default, Clone, Copy)]
struct Tally {
    launched: usize,
    reused: usize,
    failed: usize,
}

impl Tally {
    /// Count one plan's result (`Ok(true)` when it launched a session),
    /// reporting a failure as `error: <what>: ...`.
    fn record(&mut self, result: &Result<bool>, what: &str) {
        match result {
            Ok(true) => self.launched += 1,
            Ok(false) => self.reused += 1,
            Err(e) => {
                eprintln!("error: {what}: {e:#}");
                self.failed += 1;
            }
        }
    }
}

fn plans_failed(n: usize) -> String {
    format!("{n} plan(s) failed — see errors above")
}

/// Fail when anything failed to plan or dispatch.
fn ensure_none_failed(failed: usize) -> Result<()> {
    anyhow::ensure!(
        failed == 0,
        "{failed} dispatch(es) failed — see errors above"
    );
    Ok(())
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
    if !status.eq_ignore_ascii_case(kata::STATUS_OPEN) {
        anyhow::bail!(
            "kata mirror {kata_ref} of #{number} is {status}, not open — \
             run `engram kata sync --issue {number}` to reconcile it"
        );
    }
    Ok(())
}

/// What `find_mirror` learns about a plan's kata side.
#[derive(Debug)]
struct MirrorInfo {
    kata_ref: Option<String>,
    claimed_by_me: bool,
    /// What `{meta[key]}` placeholders see: the mirror's metadata, else what
    /// a new mirror would get.
    template_meta: BTreeMap<String, String>,
    /// Metadata and parent for the mirror `carry_out` creates when there is none.
    create_meta: kata::TrackerMeta,
    parent: Option<String>,
}

/// Per objective, the `(node ID, plan)` pairs to mark in progress.
type NodeProgress = BTreeMap<u64, Vec<(String, u64)>>;

/// Queue `plan`'s objective node, if any, to be marked in progress.
fn queue_node_progress(plan: &DispatchPlan, progress: &mut NodeProgress) {
    if let Some((objective, node_id)) = &plan.objective_node {
        progress
            .entry(*objective)
            .or_default()
            .push((node_id.clone(), plan.number));
    }
}

/// An objective issue's title and body, as fetched for its node plans.
#[derive(Debug, Clone)]
struct ObjectiveText {
    title: String,
    body: String,
}

/// The objective node a plan implements.
#[derive(Debug, Clone)]
struct ObjectiveRef {
    number: u64,
    node_id: String,
    text: ObjectiveText,
}

/// What launching Claude in a plan's workspace came to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Launch {
    /// A new session took over the pane.
    Launched,
    /// The pane already runs something else (most likely Claude).
    AlreadyRunning(String),
    /// `--no-launch`.
    Skipped,
    /// The PR is ready for review, so its session isn't relaunched.
    Ready,
}

impl std::fmt::Display for Launch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Launch::Launched => f.write_str("launched"),
            Launch::AlreadyRunning(cmd) => write!(f, "already running ({cmd})"),
            Launch::Skipped => f.write_str("not launched (--no-launch)"),
            Launch::Ready => f.write_str(READY_NO_RELAUNCH),
        }
    }
}

/// How carrying out one plan went. `pr_url` is set once the branch and PR
/// exist, even when a later step failed; `launched` is `Ok(true)` when a new
/// Claude session started.
#[derive(Debug)]
struct Outcome {
    pr_url: Option<String>,
    launched: Result<bool>,
}

/// A plan's existing branch, PR and worktree, as `inspect_branch` finds them.
#[derive(Debug)]
struct BranchInfo {
    state: BranchState,
    on_origin: bool,
    pr: Option<OpenPr>,
    worktree: Option<PathBuf>,
}

struct Run<'a> {
    root: &'a Path,
    repo: &'a str,
    cfg: &'a Config,
    tools: &'a Toolchain,
    opts: DispatchOptions,
    open_prs: OnceCell<Vec<PrWithBody>>,
    default: OnceCell<String>,
    actor: OnceCell<String>,
    mirrors: kata::Mirrors,
    /// Objective issues, fetched at most once per run.
    objectives: RefCell<BTreeMap<u64, ObjectiveText>>,
}

fn cached<T>(cell: &OnceCell<T>, init: impl FnOnce() -> Result<T>) -> Result<&T> {
    if let Some(v) = cell.get() {
        return Ok(v);
    }
    let v = init()?;
    Ok(cell.get_or_init(|| v))
}

/// Plan, confirm and execute `engram dispatch` for any mix of plan and
/// objective issues, or (`--stack`) one objective's dependency chains. A
/// failure is reported and the rest continue (a failed stack node skips its
/// dependents); the command fails at the end if any did.
pub fn dispatch(
    repo_root: &Path,
    repo: &str,
    cfg: &Config,
    tools: &Toolchain,
    issues: &[u64],
    opts: DispatchOptions,
) -> Result<()> {
    let run = Run {
        root: repo_root,
        repo,
        cfg,
        tools,
        opts,
        open_prs: OnceCell::new(),
        default: OnceCell::new(),
        actor: OnceCell::new(),
        mirrors: kata::Mirrors::default(),
        objectives: RefCell::new(BTreeMap::new()),
    };
    if opts.stack {
        return run.dispatch_stack(issues);
    }
    let numbers = run.collect_work(issues)?;
    if numbers.is_empty() {
        println!("Nothing to dispatch.");
        return Ok(());
    }
    let (plans, plan_failed) = run.plan_all(&numbers);
    if plans.is_empty() {
        anyhow::bail!("nothing to dispatch \u{2014} see errors above");
    }
    if !run.confirmed(plan_failed)? {
        return Ok(());
    }
    ensure_none_failed(plan_failed + run.run_all(plans))
}

impl Run<'_> {
    /// After planning: on `--dry-run` stop (failing if any plan failed),
    /// else ask for confirmation unless `--yes`. True when dispatch goes ahead.
    fn confirmed(&self, plan_failed: usize) -> Result<bool> {
        if self.opts.dry_run {
            anyhow::ensure!(plan_failed == 0, plans_failed(plan_failed));
            return Ok(false);
        }
        if !self.opts.yes && !confirm()? {
            anyhow::bail!("dispatch cancelled");
        }
        Ok(true)
    }

    /// Open PRs with bodies, listed once per run.
    fn open_prs(&self) -> Result<&[PrWithBody]> {
        cached(&self.open_prs, || {
            github::list_prs_with_body(self.repo, PrState::Open).context("listing open PRs")
        })
        .map(Vec::as_slice)
    }

    fn default_branch(&self) -> Result<&str> {
        cached(&self.default, || {
            let head = run_checked(git(self.root).args([
                "symbolic-ref",
                "--short",
                "refs/remotes/origin/HEAD",
            ]));
            // A stale origin/HEAD can name a branch origin no longer has.
            if let Ok(Some(name)) = head.as_deref().map(|h| h.strip_prefix("origin/")) {
                if !name.is_empty() && self.remote_has_branch(name)? {
                    return Ok(name.to_string());
                }
                eprintln!(
                    "warning: origin/HEAD names {name}, which origin doesn't have; \
                     asking GitHub for the default branch (git remote set-head origin --auto)"
                );
            }
            github::default_branch(self.repo).context("asking GitHub for the default branch")
        })
        .map(String::as_str)
    }

    fn actor(&self) -> Result<&str> {
        cached(&self.actor, || {
            let env = ACTOR_ENV_VARS.map(|v| std::env::var(v).ok());
            pick_actor(env, || {
                run_checked(git(self.root).args(["config", "user.name"])).ok()
            })
            .context(
                "can't tell which kata actor you are: set KATA_AUTHOR (or USER, or git user.name)",
            )
        })
        .map(String::as_str)
    }

    /// Cache an objective issue fetched while collecting work.
    fn remember_objective(&self, number: u64, issue: github::Issue) -> ObjectiveText {
        let text = ObjectiveText {
            title: issue.title,
            body: issue.body.unwrap_or_default(),
        };
        self.objectives.borrow_mut().insert(number, text.clone());
        text
    }

    fn fetch_argument_issue(
        &self,
        number: u64,
    ) -> Result<(github::Issue, Option<Vec<ObjectiveNode>>)> {
        let issue = github::get_issue(self.repo, number)
            .with_context(|| format!("fetching issue #{number}"))?;
        let nodes = objective::parse_nodes_from_comment(issue.body_text());
        require_plan_or_objective(number, &issue, nodes.is_some())?;
        Ok((issue, nodes))
    }

    /// The plan issues to dispatch, each once: plan arguments as given, and
    /// for an objective its ready nodes' plans. Blocked nodes and ready nodes
    /// without a plan issue are listed and skipped.
    fn collect_work(&self, issues: &[u64]) -> Result<Vec<u64>> {
        let (mut plans, mut seen) = (Vec::new(), BTreeSet::new());
        for &number in issues {
            if !seen.insert(number) {
                continue;
            }
            let (issue, nodes) = self.fetch_argument_issue(number)?;
            let Some(nodes) = nodes else {
                plans.push(number);
                continue;
            };
            let blocked = blocked_nodes(&nodes);
            if !blocked.is_empty() {
                println!("Objective #{number}: blocked nodes");
                for (id, blockers) in &blocked {
                    println!("  {id}  waits on {}", blockers.join(", "));
                }
            }
            let ready = ready_nodes(&nodes);
            if ready.is_empty() {
                println!("Objective #{number}: no unblocked nodes to dispatch");
            }
            for node in ready {
                match node.plan_issue {
                    Some(plan) => {
                        if seen.insert(plan) {
                            plans.push(plan);
                        }
                    }
                    None => println!(
                        "Objective #{number}: skip node {}: no plan issue yet \u{2014} run \
                         `engram objective plan {number} --node {}` (or --all-unblocked)",
                        node.id, node.id
                    ),
                }
            }
            self.remember_objective(number, issue);
        }
        Ok(plans)
    }

    /// Plan each issue without mutating anything, printing each plan and then
    /// the count line. Returns the plans and how many failed.
    fn plan_all(&self, numbers: &[u64]) -> (Vec<DispatchPlan>, usize) {
        let mut plans = Vec::new();
        let mut failed = 0;
        for &number in numbers {
            match self.plan_dispatch(number, false) {
                Ok(plan) => {
                    print!(
                        "{}",
                        format_plan(&plan, self.opts.no_launch, &self.claude())
                    );
                    plans.push(plan);
                }
                Err(e) => {
                    eprintln!("error: #{number}: {e:#}");
                    failed += 1;
                }
            }
        }
        println!(
            "{}",
            PlanCounts::of(&plans, failed).line(self.opts.no_launch)
        );
        (plans, failed)
    }

    /// Carry out each plan, then mark dispatched objective nodes in progress
    /// and print the tally. Returns how many failed.
    fn run_all(&self, plans: Vec<DispatchPlan>) -> usize {
        let mut progress = NodeProgress::new();
        let mut tally = Tally::default();
        for plan in plans {
            let outcome = self.dispatch_one(&plan, &mut progress);
            tally.record(&outcome.launched, &format!("#{}", plan.number));
        }
        self.finish(&progress, tally)
    }

    /// Mark dispatched objective nodes in progress and print the tally.
    /// Returns how many dispatches and objective updates failed.
    fn finish(&self, progress: &NodeProgress, tally: Tally) -> usize {
        let mut objective_failed = 0;
        for (number, nodes) in progress {
            if let Err(e) = record_node_progress(self.repo, *number, nodes) {
                eprintln!("error: updating objective #{number}: {e:#}");
                objective_failed += 1;
            }
        }
        let Tally {
            launched,
            reused,
            failed,
        } = tally;
        println!(
            "Dispatched {} plan(s): {launched} newly launched, {reused} reused, {failed} failed",
            launched + reused
        );
        failed + objective_failed
    }

    /// `dispatch` for `--stack`: the same steps, with each node based on its
    /// dependency's branch and linear chains linked with `gh stack link`.
    fn dispatch_stack(&self, issues: &[u64]) -> Result<()> {
        let nodes = self.collect_stack_work(issues)?;
        let main = self.default_branch()?.to_string();
        let (planned, plan_failed) = self.plan_stack(nodes, &main);
        if planned.is_empty() && plan_failed > 0 {
            anyhow::bail!("nothing to dispatch \u{2014} see errors above");
        }
        // Done or only reused as a base: nothing to confirm or link.
        if planned.iter().all(|(_, t)| t.ready()) {
            println!("Nothing to dispatch.");
            anyhow::ensure!(plan_failed == 0, plans_failed(plan_failed));
            return Ok(());
        }
        let mut chains = self.chains_to_link(&planned, &main);
        if !chains.is_empty() && !self.gh_stack_available() {
            chains.clear();
        }
        let counts = PlanCounts::of(planned.iter().map(|(_, t)| t), plan_failed);
        println!("{}", counts.line(self.opts.no_launch));
        if !self.confirmed(plan_failed)? {
            return Ok(());
        }
        ensure_none_failed(plan_failed + self.run_stack(planned, &chains, &main))
    }

    /// The objective's unfinished nodes in stack order, with their plan
    /// issues. Diamonds, nodes without a plan issue and (when planned) their
    /// dependents are listed and skipped.
    fn collect_stack_work(&self, issues: &[u64]) -> Result<Vec<(StackedNode, u64)>> {
        let &[number] = issues else {
            anyhow::bail!(
                "--stack takes exactly one engram-objective issue, got {}",
                issues.len()
            );
        };
        let (issue, nodes) = self.fetch_argument_issue(number)?;
        let Some(nodes) = nodes else {
            anyhow::bail!(
                "--stack needs an engram-objective issue; #{number} is a plan \u{2014} \
                 dispatch it without --stack"
            );
        };
        let StackOrder { stacked, skipped } = order_for_stack(&nodes)?;
        println!(
            "Objective #{number}: stacked dispatch onto {}",
            self.default_branch()?
        );
        for (id, why) in &skipped {
            println!("  skip node {id}: {why}");
        }
        self.remember_objective(number, issue);
        let mut out = Vec::new();
        for s in stacked {
            let id = &s.node_id;
            match nodes
                .iter()
                .find(|n| &n.id == id)
                .and_then(|n| n.plan_issue)
            {
                Some(plan) => out.push((s, plan)),
                None => println!(
                    "  skip node {id}: no plan issue yet \u{2014} run \
                     `engram objective plan {number} --node {id}`"
                ),
            }
        }
        Ok(out)
    }

    /// Plan each stack node on its dependency's branch, changing nothing. A
    /// node whose dependency wasn't planned is skipped, since its base branch
    /// won't exist. Returns the plans and how many failed.
    fn plan_stack(
        &self,
        nodes: Vec<(StackedNode, u64)>,
        main: &str,
    ) -> (Vec<(StackedNode, DispatchPlan)>, usize) {
        let mut planned = Vec::new();
        let mut failed = 0;
        let mut placed: BTreeMap<String, PlacedNode> = BTreeMap::new();
        for (s, number) in nodes {
            let id = &s.node_id;
            let Some(base) = base_branch(&s.base, main, &placed) else {
                println!("  skip node {id}: its dependency was not planned");
                continue;
            };
            let mut t = match self.plan_dispatch(number, true) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("error: node {id} (#{number}): {e:#}");
                    failed += 1;
                    continue;
                }
            };
            t.set_stack_base(base, main);
            let text = if t.ready() {
                format_ready_base(&t)
            } else {
                format_plan(&t, self.opts.no_launch, &self.claude())
            };
            print!("Node {id}: {text}");
            placed.insert(id.clone(), PlacedNode::of(&t));
            planned.push((s, t));
        }
        (planned, failed)
    }

    /// The chains the run will `gh stack link`, each printed as the command
    /// it will try.
    fn chains_to_link(
        &self,
        planned: &[(StackedNode, DispatchPlan)],
        main: &str,
    ) -> Vec<(Base, Vec<String>)> {
        let nodes: Vec<StackedNode> = planned.iter().map(|(s, _)| s.clone()).collect();
        let placed: BTreeMap<String, PlacedNode> = planned
            .iter()
            .map(|(s, t)| (s.node_id.clone(), PlacedNode::of(t)))
            .collect();
        // Nodes that will be dispatched, not just reused as a base.
        let fresh: BTreeSet<String> = planned
            .iter()
            .filter(|(_, t)| !t.ready())
            .map(|(s, _)| s.node_id.clone())
            .collect();
        let chains = chains_to_link(&nodes, &fresh);
        for (base, chain) in &chains {
            let base = base_branch(base, main, &placed).unwrap_or_default();
            let prs: Vec<&str> = chain
                .iter()
                .filter_map(|id| placed.get(id).map(|p| p.pr.as_str()))
                .collect();
            println!(
                "Would link: GH_REPO={} gh stack link --base {base} {}",
                self.repo,
                prs.join(" ")
            );
        }
        chains
    }

    /// Whether `gh stack` works; warns that linking is skipped when it doesn't.
    fn gh_stack_available(&self) -> bool {
        match github::gh_stack(self.root, self.repo, &["--help"]) {
            Ok(()) => true,
            Err(e) => {
                eprintln!(
                    "warning: gh-stack is unavailable ({e:#}), so `gh stack link` will be skipped; \
                     the PRs are still chained by their base branches"
                );
                false
            }
        }
    }

    /// Carry out each stack node, skipping nodes whose dependency got no
    /// branch and PR; a ready PR is only used as a base (no claim, worktree
    /// or session). Then link `chains`, mark nodes in progress and print the
    /// tally. Returns how many failed.
    fn run_stack(
        &self,
        planned: Vec<(StackedNode, DispatchPlan)>,
        chains: &[(Base, Vec<String>)],
        main: &str,
    ) -> usize {
        let mut progress = NodeProgress::new();
        // Nodes whose branch and PR exist, even if their session then failed.
        let mut placed: BTreeMap<String, PlacedNode> = BTreeMap::new();
        let mut tally = Tally::default();
        for (s, plan) in planned {
            let id = s.node_id;
            if base_branch(&s.base, main, &placed).is_none() {
                eprintln!("skip: node {id}: its dependency was not dispatched");
                continue;
            }
            let outcome = if plan.ready() {
                use_as_base(&plan, &mut progress)
            } else {
                self.dispatch_one(&plan, &mut progress)
            };
            tally.record(&outcome.launched, &format!("node {id} (#{})", plan.number));
            if let Some(pr) = outcome.pr_url {
                if outcome.launched.is_err() {
                    eprintln!("  its branch and PR exist; dependents still stack on it");
                }
                placed.insert(
                    id,
                    PlacedNode {
                        branch: plan.branch,
                        pr,
                    },
                );
            }
        }
        for (base, chain) in chains {
            self.link_chain(base, chain, main, &placed);
        }
        self.finish(&progress, tally)
    }

    /// `gh stack link` the placed bottom of `chain` (warnings only).
    fn link_chain(
        &self,
        base: &Base,
        chain: &[String],
        main: &str,
        placed: &BTreeMap<String, PlacedNode>,
    ) {
        // Only the dispatched bottom of a chain can be linked.
        let prs: Vec<&str> = chain
            .iter()
            .map_while(|id| placed.get(id).map(|p| p.pr.as_str()))
            .collect();
        let Some(base) = base_branch(base, main, placed).filter(|_| prs.len() >= 2) else {
            return;
        };
        let mut args = vec!["link", "--base", &base];
        args.extend(&prs);
        match github::gh_stack(self.root, self.repo, &args) {
            Ok(()) => println!("Linked stack on {base}: {}", prs.join(" ")),
            Err(e) => eprintln!(
                "warning: `gh stack link` failed: {e:#}\n  \
                 the PRs are still chained by their base branches"
            ),
        }
    }

    /// For a plan with an `Objective: #N (node X)` marker: that objective and
    /// node. Fails when the objective or node can't be found, or (unless
    /// `stacked`) node X still waits on others.
    fn objective_of(
        &self,
        number: u64,
        plan_body: &str,
        stacked: bool,
    ) -> Result<Option<ObjectiveRef>> {
        let Some((obj, node_id)) = objective::parse_objective_marker(plan_body) else {
            return Ok(None);
        };
        let cached = self.objectives.borrow().get(&obj).cloned();
        let text = if let Some(text) = cached {
            text
        } else {
            let issue = github::get_issue(self.repo, obj).with_context(|| {
                format!("reading objective #{obj} of plan #{number} (node {node_id})")
            })?;
            self.remember_objective(obj, issue)
        };
        let nodes = objective::parse_nodes_from_comment(&text.body).unwrap_or_default();
        let node = nodes.iter().find(|n| n.id == node_id).with_context(|| {
            format!(
                "plan #{number} names node {node_id} of objective #{obj}, which has no such node"
            )
        })?;
        let blockers = objective::blocked_by(node, &nodes);
        if !blockers.is_empty() && !stacked {
            anyhow::bail!(
                "plan #{number} is node {node_id} of objective #{obj}, which is blocked \
                 by node(s) {} that are not done yet",
                blockers.join(", ")
            );
        }
        Ok(Some(ObjectiveRef {
            number: obj,
            node_id,
            text,
        }))
    }

    /// Whether the mirror still needs claiming by this actor (see [`needs_claim`]).
    fn needs_claim(&self, mirror: &kata::KataIssue, kata_ref: &str) -> Result<bool> {
        needs_claim(mirror.owner.as_deref(), self.actor()?, kata_ref)
    }

    /// Work out everything dispatch will do for plan `number`, changing
    /// nothing. `stacked` lets a node with unfinished dependencies through.
    fn plan_dispatch(&self, number: u64, stacked: bool) -> Result<DispatchPlan> {
        let issue = github::get_issue(self.repo, number)
            .with_context(|| format!("fetching issue #{number}"))?;
        require_plan(number, &issue)?;
        let body = issue.body.unwrap_or_default();
        let objective = self.objective_of(number, &body, stacked)?;
        let mirror = self.find_mirror(number, &issue.title, &body, objective.as_ref())?;
        let fields = PlanFields {
            number,
            kata_ref: mirror.kata_ref.clone(),
            title: issue.title.clone(),
            meta: mirror.template_meta,
        };
        // The templates only matter for a plan without a PR yet, so a broken
        // one can't block re-running a plan that is already under way.
        let template_branch = fill_template(&self.cfg.dispatch.branch, &fields);
        let found = self.find_existing_pr(number, template_branch.as_deref().ok())?;
        let (branch, base, pr_link) = match &found {
            Some((pr, how)) => (pr.head.clone(), pr.base.clone(), Some(*how)),
            None => (template_branch?, self.default_branch()?.to_string(), None),
        };
        let info = self.inspect_branch(number, &branch, found.map(|(pr, _)| pr))?;
        let pr_title = match &info.pr {
            Some(pr) => pr.title.clone(),
            None => fill_pr_title(&self.cfg.dispatch.pr_title, &fields)?,
        };
        Ok(DispatchPlan {
            number,
            title: issue.title,
            body,
            kata_ref: mirror.kata_ref,
            claimed_by_me: mirror.claimed_by_me,
            meta: mirror.create_meta,
            parent: mirror.parent,
            branch,
            branch_state: info.state,
            on_origin: info.on_origin,
            pr_title,
            base,
            pr: info.pr,
            pr_link,
            worktree: info.worktree,
            objective_node: objective.map(|o| (o.number, o.node_id)),
            stacked_on: None,
        })
    }

    /// The plan's kata mirror and the metadata its templates see. Without a
    /// mirror, metadata comes from the plan body and title (plus its
    /// objective's, for a node plan), as `plan new` would set it.
    fn find_mirror(
        &self,
        number: u64,
        title: &str,
        body: &str,
        objective: Option<&ObjectiveRef>,
    ) -> Result<MirrorInfo> {
        if let Some(r) = self
            .mirrors
            .find(self.repo, number, body)
            .with_context(|| format!("looking up the kata mirror of #{number}"))?
        {
            let mirror =
                kata::show(&r).with_context(|| format!("reading kata mirror {r} of #{number}"))?;
            check_mirror_open(number, &r, &mirror.status)?;
            let claimed_by_me = !self.needs_claim(&mirror, &r)?;
            return Ok(MirrorInfo {
                kata_ref: Some(r),
                claimed_by_me,
                template_meta: kata::metadata_map(&mirror.metadata),
                create_meta: kata::TrackerMeta::default(),
                parent: None,
            });
        }
        let (create_meta, parent) = match objective {
            Some(o) => {
                let parent = kata::parse_ref(&o.text.body);
                let meta = objective::node_tracker_meta(
                    &kata::TrackerMeta::default(),
                    title,
                    body,
                    &o.text.title,
                    &o.text.body,
                    parent,
                );
                (meta, parent.map(str::to_string))
            }
            None => (kata::TrackerMeta::resolve(None, None, title, body), None),
        };
        Ok(MirrorInfo {
            kata_ref: None,
            claimed_by_me: false,
            template_meta: create_meta.to_map(),
            create_meta,
            parent,
        })
    }

    /// The open PR that closes plan `number`, and how it was found. Work
    /// started under any branch name is found this way, so a retitled plan or
    /// changed template can't start a second branch, PR and session: GitHub's
    /// closing link first, else (for stacked PRs, which GitHub never links)
    /// `Closes #N` in a PR body, preferring the PR on `branch`.
    fn find_existing_pr(
        &self,
        number: u64,
        branch: Option<&str>,
    ) -> Result<Option<(OpenPr, PrLink)>> {
        let linked = github::open_closing_pr(self.repo, number)
            .with_context(|| format!("looking up open PRs linked to #{number}"))?;
        if let Some(pr) = linked {
            return Ok(Some((pr, PrLink::Linked)));
        }
        Ok(
            github::select_body_closing_pr(self.open_prs()?, self.repo, number, branch)
                .cloned()
                .map(|pr| (pr, PrLink::BodyKeyword)),
        )
    }

    fn remote_has_branch(&self, b: &str) -> Result<bool> {
        let mut cmd = git_remote(self.root)?;
        cmd.args([
            "ls-remote",
            "--exit-code",
            "--heads",
            "origin",
            &format!("refs/heads/{b}"),
        ]);
        let output = process::spawn(&mut cmd)?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(LS_REMOTE_NO_MATCH) => Ok(false),
            _ => Err(process::failure(&cmd, &output)),
        }
    }

    /// Where plan `number`'s `branch` already exists, its open PR (`pr` when
    /// one was already found) and its worktree. Fails on an invalid branch
    /// name, or a branch whose PR already merged.
    fn inspect_branch(&self, number: u64, branch: &str, pr: Option<OpenPr>) -> Result<BranchInfo> {
        if run_checked(git(self.root).args(["check-ref-format", "--branch", branch])).is_err() {
            anyhow::bail!(
                "`{branch}` is not a valid branch name — check the [dispatch] branch template"
            );
        }
        let local = run_checked(git(self.root).args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ]))
        .is_ok();
        let on_origin = self.remote_has_branch(branch)?;
        let pr = match pr {
            Some(pr) => Some(pr),
            None => github::pr_for_branch(self.repo, branch, PrState::Open)
                .with_context(|| format!("looking up the open PR from {branch}"))?
                .map(|p| p.pr),
        };
        // A branch whose PR already merged (e.g. into a stacked base, which
        // leaves the plan open) must not get a second PR from the merged head.
        if pr.is_none() && (local || on_origin) {
            let merged = github::pr_for_branch(self.repo, branch, PrState::Merged)
                .with_context(|| format!("looking up a merged PR from {branch}"))?;
            if let Some(merged) = merged {
                anyhow::bail!(
                    "{branch} was already merged into {} ({}); not opening another PR from it \
                     \u{2014} mark the plan's node done or run `engram plan land {number}`",
                    merged.pr.base,
                    merged.pr.url
                );
            }
        }
        Ok(BranchInfo {
            state: BranchState::of(local, on_origin),
            on_origin,
            pr,
            worktree: self.kwt_worktree(branch)?,
        })
    }

    /// Carry out one plan; once its branch and PR exist, its objective node is
    /// queued to be marked in progress, even if a later step failed.
    fn dispatch_one(&self, plan: &DispatchPlan, progress: &mut NodeProgress) -> Outcome {
        let outcome = self.carry_out(plan);
        if outcome.pr_url.is_some() {
            queue_node_progress(plan, progress);
        }
        outcome
    }

    /// Carry out a confirmed plan and print its report. Every step looks up
    /// existing state first, so a re-run after a partial failure resumes.
    fn carry_out(&self, t: &DispatchPlan) -> Outcome {
        let placed = self
            .ensure_mirror_claimed(t)
            .and_then(|kata_ref| Ok((self.place_branch_and_pr(t, &kata_ref)?, kata_ref)));
        let (pr_url, kata_ref) = match placed {
            Ok(v) => v,
            Err(e) => {
                return Outcome {
                    pr_url: None,
                    launched: Err(e),
                }
            }
        };
        let launched = self
            .open_workspace(t, &kata_ref, &pr_url)
            .map(|launch| launch == Launch::Launched);
        Outcome {
            pr_url: Some(pr_url),
            launched,
        }
    }

    /// The plan's kata mirror, created (as `plan new` would) when it has none,
    /// and claimed by this actor. Only once the claim holds does a new
    /// mirror's `Kata:` line go onto the GitHub issue.
    fn ensure_mirror_claimed(&self, t: &DispatchPlan) -> Result<String> {
        let (kata_ref, claimed_by_me, created) = match &t.kata_ref {
            Some(r) => (r.clone(), t.claimed_by_me, false),
            // Same idempotency key as `plan new`, so the two paths can't
            // duplicate the mirror; it may return one someone already claimed.
            None => {
                let url = github::issue_url(self.repo, t.number);
                let kata_ref = kata::create(
                    &t.title,
                    &kata::mirror_body(&url, &t.body),
                    &kata::plan_idempotency_key(t.number),
                    t.parent.as_deref(),
                    &t.meta,
                )
                .context("creating kata mirror")?;
                eprintln!("kata: created mirror {kata_ref}");
                let mirror = kata::show(&kata_ref)
                    .with_context(|| format!("reading kata mirror {kata_ref}"))?;
                let claimed = !self.needs_claim(&mirror, &kata_ref)?;
                (kata_ref, claimed, true)
            }
        };
        if claimed_by_me {
            eprintln!("kata: {kata_ref} already claimed by you");
        } else {
            kata::claim(&kata_ref).with_context(|| format!("claiming kata {kata_ref}"))?;
            eprintln!("kata: claimed {kata_ref}");
        }
        if created {
            let body = kata::with_ref_line(&t.body, &kata_ref);
            if let Err(e) = github::update_issue_body(self.repo, t.number, &body) {
                eprintln!("warning: could not append Kata ref to GitHub issue: {e:#}");
            }
        }
        Ok(kata_ref)
    }

    /// Create and push the plan's branch and open its draft PR (or reuse
    /// them). Returns the PR URL.
    fn place_branch_and_pr(&self, t: &DispatchPlan, kata_ref: &str) -> Result<String> {
        run_checked(git_remote(self.root)?.args(["fetch", "origin", &t.base]))?;
        self.create_branch_if_missing(t)?;
        if t.on_origin {
            eprintln!("push: skipped, origin already has {}", t.branch);
        } else {
            run_checked(git_remote(self.root)?.args(["push", "-u", "origin", &t.branch]))?;
            eprintln!("push: pushed {} to origin", t.branch);
        }

        if let Some(pr) = &t.pr {
            if let Some(warning) = t.base_mismatch_warning() {
                eprintln!("{warning}");
            }
            eprintln!("pr: reusing {}", pr.url);
            return Ok(pr.url.clone());
        }
        let body = pr_body(t.number, kata_ref, t.stacked_on.as_deref());
        let url = github::create_draft_pr(self.repo, &t.branch, &t.base, &t.pr_title, &body)
            .with_context(|| format!("opening a draft PR from {}", t.branch))?;
        eprintln!("pr: opened draft {url}");
        Ok(url)
    }

    /// Create or reuse the plan's worktree and tmux session, launch Claude in
    /// it (unless `--no-launch` or the PR is ready), and print the report.
    fn open_workspace(&self, t: &DispatchPlan, kata_ref: &str, pr_url: &str) -> Result<Launch> {
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

    /// Create the branch without checking anything out, so the user's checkout
    /// and HEAD stay untouched.
    fn create_branch_if_missing(&self, t: &DispatchPlan) -> Result<()> {
        let b = &t.branch;
        match t.branch_state {
            BranchState::Local => eprintln!("branch: {b} exists, reusing it"),
            BranchState::Remote => {
                // Dispatched from another machine: track the remote branch rather
                // than creating a divergent wip commit.
                let refspec = format!("refs/heads/{b}:refs/remotes/origin/{b}");
                run_checked(git_remote(self.root)?.args(["fetch", "origin", &refspec]))?;
                run_checked(git(self.root).args(["branch", "--track", b, &format!("origin/{b}")]))?;
                eprintln!("branch: {b} tracks origin/{b}");
            }
            BranchState::Missing => {
                // Base the wip commit on the freshly fetched remote ref, not local
                // <default>, which can hold unpushed or stale commits.
                let base_ref = format!("origin/{}", t.base);
                let tree = run_checked(
                    git(self.root).args(["rev-parse", &format!("{base_ref}^{{tree}}")]),
                )?;
                let msg = format!("wip: #{} {}", t.number, t.title);
                let commit = run_checked(git(self.root).args([
                    "commit-tree",
                    &tree,
                    "-p",
                    &base_ref,
                    "-m",
                    &msg,
                ]))?;
                run_checked(git(self.root).args(["branch", b, &commit]))?;
                eprintln!("branch: created {b} on {base_ref}");
            }
        }
        Ok(())
    }

    fn kwt(&self) -> Command {
        let mut cmd = Command::new(&self.tools.kwt);
        cmd.current_dir(self.root);
        cmd
    }

    fn kwt_worktree(&self, branch: &str) -> Result<Option<PathBuf>> {
        let out = run_checked(self.kwt().args(["list", "--json"]))?;
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
    fn claude(&self) -> String {
        claude_command(
            std::env::var(CLAUDE_OVERRIDE_ENV).ok(),
            self.cfg.dispatch.claude.as_deref(),
            &self.tools.claude,
        )
    }

    /// Run `tmux` against kwt's server for `endpoint`. kwt (and Ghosthub) run
    /// tmux with `TMUX_TMPDIR` unset; honouring it here would address a
    /// different server than the one holding the workspace.
    fn tmux(&self, endpoint: &Endpoint, args: &[&str]) -> Result<String> {
        run_checked(
            Command::new(&self.tools.tmux)
                .env_remove("TMUX_TMPDIR")
                .args(["-L", &endpoint.tmux_socket_name])
                .args(args),
        )
    }

    /// Type the claude command into the session's pane, then wait until
    /// Claude has taken the pane over and is still there a moment later.
    fn launch_claude(
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

/// A ready PR in a stack: nothing to dispatch, only its branch to stack on.
fn use_as_base(plan: &DispatchPlan, progress: &mut NodeProgress) -> Outcome {
    if let Some(warning) = plan.base_mismatch_warning() {
        eprintln!("{warning}");
    }
    queue_node_progress(plan, progress);
    println!(
        "#{}  {READY_NO_RELAUNCH}; used as a base ({} \u{2190} {})",
        plan.number, plan.branch, plan.base
    );
    Outcome {
        pr_url: plan.pr.as_ref().map(|pr| pr.url.clone()),
        launched: Ok(false),
    }
}

fn pr_body(number: u64, kata_ref: &str, stacked_on: Option<&str>) -> String {
    let stacked = stacked_on
        .map(|base| format!(" Stacked on `{base}`."))
        .unwrap_or_default();
    format!(
        "Work in progress for engram plan #{number} (kata `{kata_ref}`).{stacked}\n\nCloses #{number}"
    )
}

fn build_prompt(t: &DispatchPlan, kata_ref: &str, pr_url: &str) -> String {
    let n = t.number;
    let mut out = format!(
        "Work on engram plan #{n}: {}\nkata issue: {kata_ref}\n\n{}\n\n---\n\
         A draft pull request is already open for this branch: {pr_url} (base `{}`).\n\
         Do not open a new PR and do not change its base.\n\
         Keep the `Closes #{n}` line in the PR body if you edit it.\n",
        t.title,
        t.body.trim_end(),
        t.pr_base()
    );
    if let Some(base) = &t.stacked_on {
        out += &format!(
            "It is stacked on `{base}` as part of a multi-PR dependency chain \u{2014} \
             do not rebase it onto the default branch.\n"
        );
    }
    out + "When the implementation is complete and tests pass, push your commits to this branch \
           and run `gh pr ready` on the existing PR, adding a summary comment of what changed and why.\n"
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

/// `s` in single quotes for a POSIX shell, whatever it contains.
fn single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r#"'"'"'"#))
}

/// `s` as one shell word: as it is when it needs no quoting, else single-quoted.
fn shell_quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        return s.to_string();
    }
    single_quote(s)
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
            .join(DEFAULT_STATE_HOME),
    };
    Ok(state_home
        .join(PROMPT_SUBDIR)
        .join(format!("{}-{number}.md", repo.replace('/', "-"))))
}

/// `ENGRAM_DISPATCH_CLAUDE`, else `[dispatch] claude` (both verbatim), else
/// the resolved `claude` path.
fn claude_command(
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
fn redact_secrets(cmd: &str) -> String {
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

fn launch_command(
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
    use rstest::rstest;
    use NodeStatus::*;

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
            "plan #142 has no metadata 'team', 'area' \
             (kata meta set a56d team <value>; kata meta set a56d area <value>)"
        )
    )]
    #[case::missing_meta_without_mirror(
        "{meta[team]}",
        None,
        Err("'team' only comes from kata metadata: mirror the plan in kata")
    )]
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

    // Without a mirror, jira and type can only come from the plan body.
    #[rstest]
    #[case::mirror(
        Some("a56d"),
        &["no metadata 'jira', 'type' (kata meta set a56d jira <value>; kata meta set a56d type"],
        &["issue body"]
    )]
    #[case::no_mirror(
        None,
        &["`Jira: KEY-1` line to the plan issue body; add a `Type: feat` line"],
        &["kata meta set"]
    )]
    fn fill_template_hints_how_to_supply_missing_meta(
        #[case] kata_ref: Option<&str>,
        #[case] has: &[&str],
        #[case] lacks: &[&str],
    ) {
        let p = PlanFields {
            meta: BTreeMap::new(),
            ..plan(kata_ref)
        };
        let err = fill_template("{meta[jira]}/{meta[type]}", &p).unwrap_err();
        assert_mentions(&format!("{err:#}"), has, lacks);
    }

    #[rstest]
    #[case::filled("{title}", Ok("Add dispatch preflight!"))]
    #[case::empty("", Err("[dispatch] pr_title ``"))]
    #[case::whitespace("  ", Err("renders an empty PR title for plan #142"))]
    fn fill_pr_title_refuses_blank_titles(#[case] template: &str, #[case] want: Want<&str>) {
        assert_result(fill_pr_title(template, &plan(None)), want.map(Into::into));
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
    fn branch_slug_cases(#[case] title: &str, #[case] slug: &str) {
        assert_eq!(branch_slug(title), slug);
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
        let [bin, other, empty] = [(); 3].map(|()| tempfile::tempdir().unwrap());
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

    fn target() -> DispatchPlan {
        DispatchPlan {
            number: 143,
            title: "[5.2] Add engram dispatch".into(),
            body: "Objective: #141 (node 5.2)\n\n## Why\nUse `$HOME` and \"quotes\".\n".into(),
            kata_ref: Some("mfbq".into()),
            claimed_by_me: false,
            meta: kata::TrackerMeta::default(),
            parent: None,
            branch: "143-add-engram-dispatch".into(),
            branch_state: BranchState::Missing,
            on_origin: false,
            pr_title: "[5.2] Add engram dispatch".into(),
            base: "main".into(),
            pr: None,
            pr_link: None,
            worktree: None,
            objective_node: Some((141, "5.2".into())),
            stacked_on: None,
        }
    }

    fn with_pr(mut t: DispatchPlan, base: &str, draft: bool) -> DispatchPlan {
        t.pr = Some(OpenPr {
            number: 9,
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
    fn reused(base: &str) -> DispatchPlan {
        let mut t = with_pr(target(), "main", false);
        t.pr_link = Some(PrLink::BodyKeyword);
        t.branch_state = BranchState::Local;
        t.base = base.into();
        t
    }

    #[rstest]
    #[case::fresh(false, None, "main", &["(base `main`)"], &["stacked on"])]
    #[case::stack_on_the_default_branch(
        true, None, "main", &["(base `main`)"], &["stacked on"])]
    #[case::stacked(true, None, "a56d-x", &[
        "(base `a56d-x`)", "It is stacked on `a56d-x`", "do not rebase it onto the default branch",
    ], &[])]
    // A reused PR's real base wins over the computed one.
    #[case::reused_pr(true, Some("old-base"), "computed-base",
        &["(base `old-base`)", "stacked on `old-base`"], &["computed-base"])]
    #[case::reused_pr_on_main(true, Some("main"), "computed-base",
        &["(base `main`)"], &["stacked on"])]
    fn prompt_carries_plan_base_and_stacking(
        #[case] in_stack: bool,
        #[case] pr_base: Option<&str>,
        #[case] base: &str,
        #[case] has: &[&str],
        #[case] lacks: &[&str],
    ) {
        let mut t = pr_base.map_or_else(target, |b| with_pr(target(), b, true));
        if in_stack {
            t.set_stack_base(base.into(), "main");
        } else {
            t.base = base.into();
        }
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

    #[rstest]
    #[case::own_base(
        143,
        "mfbq",
        None,
        "Work in progress for engram plan #143 (kata `mfbq`).\n\nCloses #143"
    )]
    #[case::stacked(
        145,
        "bjs1",
        Some("81h7-x"),
        "Work in progress for engram plan #145 (kata `bjs1`). Stacked on `81h7-x`.\n\nCloses #145"
    )]
    fn pr_body_closes_the_plan(
        #[case] number: u64,
        #[case] kata_ref: &str,
        #[case] stacked_on: Option<&str>,
        #[case] want: &str,
    ) {
        assert_eq!(pr_body(number, kata_ref, stacked_on), want);
    }

    #[rstest]
    #[case::fresh(target(), false, &[
        "kata:     mfbq (will claim)",
        "143-add-engram-dispatch \u{2190} main (create from origin with an empty wip commit)",
        "PR:       (will open a draft)\n  worktree: (will create with kwt)\n  launch:   claude in",
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
    #[case::no_mirror(DispatchPlan { kata_ref: None, ..target() }, false, &[
        "kata:     (no mirror, will create and claim)",
    ])]
    fn format_plan_says_what_is_reused_and_how(
        #[case] t: DispatchPlan,
        #[case] no_launch: bool,
        #[case] has: &[&str],
    ) {
        assert_mentions(&format_plan(&t, no_launch, "claude"), has, &[]);
    }

    #[rstest]
    #[case::configured(
        "my-claude --x",
        "launch:   my-claude --x in the workspace's tmux pane"
    )]
    #[case::token_hidden(
        "GH_TOKEN=ghp_secret claude",
        "launch:   GH_TOKEN=\u{2026} claude in the workspace's tmux pane"
    )]
    fn format_plan_shows_the_resolved_claude_command(#[case] claude: &str, #[case] want: &str) {
        let text = format_plan(&target(), false, claude);
        assert_mentions(&text, &[want], &["ghp_secret"]);
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
    #[case::objective("OPEN", &[OBJECTIVE_LABEL], true, Ok(()))]
    #[case::node_list_wins_without_label("OPEN", &[], true, Ok(()))]
    #[case::plan("OPEN", &[PLAN_LABEL], false, Ok(()))]
    #[case::objective_without_nodes("OPEN", &[OBJECTIVE_LABEL], false, Err("without a node list"))]
    #[case::unlabelled("OPEN", &[], false, Err("neither"))]
    #[case::closed_plan(
        "CLOSED", &[PLAN_LABEL], false,
        Err("#1 is not open (state: CLOSED); if its work merged, run `engram plan land 1`")
    )]
    #[case::closed_objective("CLOSED", &[OBJECTIVE_LABEL], true, Err("not open (state: CLOSED)"))]
    fn require_plan_or_objective_refuses_closed_and_unlabelled_issues(
        #[case] state: &str,
        #[case] labels: &[&str],
        #[case] has_nodes: bool,
        #[case] want: Want<()>,
    ) {
        let url = "https://github.com/o/r/issues/1";
        let labels = labels.iter().map(|l| github::Label { name: (*l).into() });
        let issue = github::Issue {
            title: String::new(),
            body: None,
            state: state.into(),
            labels: labels.collect(),
            url: url.into(),
        };
        assert_result(require_plan_or_objective(1, &issue, has_nodes), want);
    }

    fn node(id: &str, status: NodeStatus, deps: &[&str]) -> ObjectiveNode {
        ObjectiveNode {
            id: id.into(),
            description: id.into(),
            status,
            plan_issue: None,
            pr_url: None,
            depends_on: deps.iter().map(std::string::ToString::to_string).collect(),
        }
    }

    fn planned(id: &str, status: NodeStatus, deps: &[&str], plan: u64) -> ObjectiveNode {
        ObjectiveNode {
            plan_issue: Some(plan),
            ..node(id, status, deps)
        }
    }

    fn on(id: &str, base: Option<&str>) -> StackedNode {
        StackedNode {
            node_id: id.into(),
            base: base.map_or(Base::Main, |b| Base::OnNode(b.into())),
        }
    }

    #[rstest]
    #[case::empty(vec![], vec![], &[])]
    #[case::chain(
        vec![node("a", Pending, &[]), node("b", Pending, &["a"]), node("c", Pending, &["b"])],
        vec![on("a", None), on("b", Some("a")), on("c", Some("b"))], &[])]
    #[case::dependencies_first(
        vec![node("c", Pending, &["b"]), node("b", Pending, &["a"]),
             node("x", Pending, &[]), node("a", Pending, &[])],
        vec![on("x", None), on("a", None), on("b", Some("a")), on("c", Some("b"))], &[])]
    #[case::fan_out(
        vec![node("a", Pending, &[]), node("b", Pending, &["a"]),
             node("c", Pending, &["a"]), node("r", Pending, &[])],
        vec![on("a", None), on("b", Some("a")), on("c", Some("a")), on("r", None)], &[])]
    // Objective #141's shape: 5.5 waits on 5.3 and 5.4, and 5.4 on 5.3.
    #[case::transitive_dependency_is_no_diamond(
        vec![node("5.3", InProgress, &[]), node("5.4", Pending, &["5.3"]),
             node("5.5", Pending, &["5.3", "5.4"])],
        vec![on("5.3", None), on("5.4", Some("5.3")), on("5.5", Some("5.4"))], &[])]
    // Done dependencies base on the default branch; unknown ones are ignored.
    #[case::done_and_unknown_dependencies(
        vec![node("a", Done, &[]), node("b", Done, &[]),
             node("c", Pending, &["a", "b"]), node("d", Pending, &["a", "c", "zz"])],
        vec![on("c", None), on("d", Some("c"))], &[])]
    #[case::diamond_and_its_dependents(
        vec![node("a", Pending, &[]), node("b", Pending, &[]),
             node("d", Pending, &["a", "b"]), node("e", Pending, &["d"])],
        vec![on("a", None), on("b", None)],
        &[("d", "diamond: waits on a, b"), ("e", "depends on skipped node d")])]
    // b reaches c only through done x, whose work is on the default branch
    // without c's: b's branch doesn't contain c, so n is a diamond.
    #[case::path_through_a_done_node(
        vec![node("c", Pending, &[]), node("x", Done, &["c"]),
             node("b", Pending, &["x"]), node("n", Pending, &["b", "c"])],
        vec![on("c", None), on("b", None)], &[("n", "diamond: waits on b, c")])]
    #[case::cycle_among_done_nodes(
        vec![node("x", Done, &["y"]), node("y", Done, &["x"])], vec![], &[])]
    fn order_for_stack_bases_each_node_on_its_dependency(
        #[case] nodes: Vec<ObjectiveNode>,
        #[case] want: Vec<StackedNode>,
        #[case] want_skipped: &[(&str, &str)],
    ) {
        let StackOrder { stacked, skipped } = order_for_stack(&nodes).unwrap();
        assert_eq!(stacked, want);
        assert_eq!(skipped.len(), want_skipped.len(), "{skipped:?}");
        for ((id, why), (want_id, want_why)) in skipped.iter().zip(want_skipped) {
            assert!(id == want_id && why.starts_with(want_why), "{id}: {why}");
        }
    }

    #[test]
    fn order_for_stack_reports_cycles() {
        let nodes = [
            node("a", Pending, &[]),
            node("b", Pending, &["c"]),
            node("c", Pending, &["b"]),
        ];
        assert_result(
            order_for_stack(&nodes),
            Err("cycle among objective nodes b, c"),
        );
    }

    #[test]
    fn straight_chains_split_at_forks_and_roots() {
        // a -> b -> {c -> d, e}; r alone.
        let targets = [
            on("a", None),
            on("b", Some("a")),
            on("c", Some("b")),
            on("d", Some("c")),
            on("e", Some("b")),
            on("r", None),
        ];
        let chain = |base: Option<&str>, ids: &[&str]| {
            (
                on("", base).base,
                ids.iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            straight_chains(&targets),
            vec![
                chain(None, &["a", "b"]),
                chain(Some("b"), &["c", "d"]),
                chain(Some("b"), &["e"]),
                chain(None, &["r"])
            ]
        );
    }

    #[rstest]
    #[case::nothing_fresh(&[], &[])]
    #[case::fresh_top(&["d"], &[(Some("b"), &["c", "d"][..])])]
    #[case::fresh_bottom(&["a"], &[(None, &["a", "b"][..])])]
    #[case::fresh_single_never_links(&["e", "r"], &[])]
    fn chains_to_link_needs_a_fresh_node(
        #[case] fresh: &[&str],
        #[case] want: &[(Option<&str>, &[&str])],
    ) {
        // a -> b -> {c -> d, e}; r alone.
        let targets = [
            on("a", None),
            on("b", Some("a")),
            on("c", Some("b")),
            on("d", Some("c")),
            on("e", Some("b")),
            on("r", None),
        ];
        let fresh = fresh.iter().map(std::string::ToString::to_string).collect();
        let want: Vec<_> = want
            .iter()
            .map(|(base, ids)| {
                (
                    on("", *base).base,
                    ids.iter().map(std::string::ToString::to_string).collect(),
                )
            })
            .collect();
        assert_eq!(chains_to_link(&targets, &fresh), want);
    }

    #[test]
    fn objective_node_sets() {
        let nodes = vec![
            node("1", Done, &[]),
            planned("2", InProgress, &["1"], 20),
            node("3", Pending, &["1", "2"]),
            node("4", Pending, &["2", "3", "9.9"]),
            node("5", Pending, &["9.9"]),
            planned("6", Done, &["3"], 60),
        ];
        // In-progress 2 counts; 5's unknown dependency doesn't block; done never qualifies.
        let picked: Vec<&str> = ready_nodes(&nodes).iter().map(|n| n.id.as_str()).collect();
        assert_eq!(picked, ["2", "5"]);
        assert_eq!(
            blocked_nodes(&nodes),
            vec![
                ("3".to_string(), vec!["2"]),
                ("4".to_string(), vec!["2", "3"])
            ]
        );
    }

    #[test]
    fn merge_node_progress_records_plan_and_progress_only() {
        let mut nodes = vec![
            node("1", Pending, &[]),
            planned("2", InProgress, &[], 20),
            planned("3", Done, &[], 30),
        ];
        let up = |id: &str, plan: u64| (id.to_string(), plan);
        let updates =
            [("1", 10), ("1", 10), ("2", 99), ("3", 30), ("9.9", 1)].map(|(id, n)| up(id, n));
        let changed = merge_node_progress(&mut nodes, &updates);
        assert_eq!(changed, vec!["1"]);
        assert_eq!(nodes[0].status, InProgress);
        assert_eq!(nodes[0].plan_issue, Some(10));
        // An existing plan issue is never replaced; done stays done.
        assert_eq!(nodes[1].plan_issue, Some(20));
        assert_eq!(nodes[2].status, Done);
        assert!(merge_node_progress(&mut nodes, &[up("1", 10), up("2", 20)]).is_empty());
    }

    #[rstest]
    fn require_plan_or_objective_names_pull_requests(#[values(false, true)] has_nodes: bool) {
        let pr = github::Issue {
            title: String::new(),
            body: None,
            state: "OPEN".into(),
            labels: vec![],
            url: "https://github.com/o/r/pull/147".into(),
        };
        assert_result(
            require_plan_or_objective(147, &pr, has_nodes),
            Err("#147 is a pull request, not an"),
        );
    }

    #[rstest]
    #[case::fresh(3, 0, 0, false, "3 plan(s) to dispatch (0 already ready for review, not relaunched); up to 3 Claude session(s) will start")]
    #[case::ready_and_failed(3, 1, 2, false, "3 plan(s) to dispatch (1 already ready for review, not relaunched); up to 2 Claude session(s) will start; 2 failed to plan")]
    #[case::no_launch(2, 0, 0, true, "2 plan(s) to dispatch (0 already ready for review, not relaunched); up to 0 Claude session(s) will start")]
    fn count_line_cases(
        #[case] planned: usize,
        #[case] ready: usize,
        #[case] failed: usize,
        #[case] no_launch: bool,
        #[case] want: &str,
    ) {
        let counts = PlanCounts {
            planned,
            ready,
            failed,
        };
        assert_eq!(counts.line(no_launch), want);
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

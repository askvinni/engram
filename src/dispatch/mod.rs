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
//!
//! `preflight` finds the tools, `inspect` plans each dispatch without
//! changing anything, `render` prints it, `execute` carries it out and
//! `workspace` opens the kwt worktree and tmux session and launches Claude.
//! `stack` and `template` hold the `--stack` ordering and the `[dispatch]`
//! templates.

use anyhow::{Context, Result};
use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::integrations::github::{self, OpenPr, PrState, PrWithBody, OBJECTIVE_LABEL, PLAN_LABEL};
use crate::integrations::kata;
use crate::process::{git, run_checked};
use crate::workflow::objective::{self, NodeStatus, ObjectiveNode};

mod execute;
mod inspect;
mod preflight;
mod render;
mod stack;
mod template;
#[cfg(test)]
mod test_support;
mod workspace;

use execute::Outcome;
use inspect::ObjectiveText;
pub use preflight::{preflight, unavailable_reasons, Toolchain};
use render::{build_prompt, format_plan, format_ready_base, pr_body};
use template::{fill_pr_title, fill_template, PlanFields};
use workspace::{redact_secrets, Launch};

const READY_NO_RELAUNCH: &str = "PR is ready for review \u{2014} not relaunching";
/// Where kata reads the actor from, in its order (then git's `user.name`).
const ACTOR_ENV_VARS: [&str; 2] = ["KATA_AUTHOR", "USER"];
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::test_support::*;
    use rstest::rstest;
    use NodeStatus::*;

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
}

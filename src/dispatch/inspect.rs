//! Planning a dispatch: everything about a plan's mirror, objective node,
//! branch and PR, worked out without changing anything.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::integrations::github::{self, OpenPr, PrState};
use crate::integrations::kata;
use crate::process::{self, git, git_remote, run_checked};
use crate::workflow::objective;

use super::{
    fill_pr_title, fill_template, require_plan, BranchState, DispatchPlan, PlanFields, PrLink, Run,
};

/// `git ls-remote --exit-code` exits 2 when the ref is absent.
pub(super) const LS_REMOTE_NO_MATCH: i32 = 2;

/// Whether the mirror still needs claiming; fails, before anything is
/// mutated, when another actor owns it.
pub(super) fn needs_claim(owner: Option<&str>, me: &str, kata_ref: &str) -> Result<bool> {
    match owner {
        None | Some("") => Ok(true),
        Some(o) if o == me => Ok(false),
        Some(o) => anyhow::bail!(
            "kata {kata_ref} is claimed by {o}; ask them to release it (kata unassign {kata_ref})"
        ),
    }
}

/// Refuse a mirror kata no longer treats as open (closed, or drifted from GitHub).
pub(super) fn check_mirror_open(number: u64, kata_ref: &str, status: &str) -> Result<()> {
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
pub(super) struct MirrorInfo {
    pub(super) kata_ref: Option<String>,
    pub(super) claimed_by_me: bool,
    /// What `{meta[key]}` placeholders see: the mirror's metadata, else what
    /// a new mirror would get.
    pub(super) template_meta: BTreeMap<String, String>,
    /// Metadata and parent for the mirror `carry_out` creates when there is none.
    pub(super) create_meta: kata::TrackerMeta,
    pub(super) parent: Option<String>,
}

/// An objective issue's title and body, as fetched for its node plans.
#[derive(Debug, Clone)]
pub(super) struct ObjectiveText {
    pub(super) title: String,
    pub(super) body: String,
}

/// The objective node a plan implements.
#[derive(Debug, Clone)]
pub(super) struct ObjectiveRef {
    pub(super) number: u64,
    pub(super) node_id: String,
    pub(super) text: ObjectiveText,
}

/// A plan's existing branch, PR and worktree, as `inspect_branch` finds them.
#[derive(Debug)]
pub(super) struct BranchInfo {
    pub(super) state: BranchState,
    pub(super) on_origin: bool,
    pub(super) pr: Option<OpenPr>,
    pub(super) worktree: Option<PathBuf>,
}

impl Run<'_> {
    /// Cache an objective issue fetched while collecting work.
    pub(super) fn remember_objective(&self, number: u64, issue: github::Issue) -> ObjectiveText {
        let text = ObjectiveText {
            title: issue.title,
            body: issue.body.unwrap_or_default(),
        };
        self.objectives.borrow_mut().insert(number, text.clone());
        text
    }

    /// For a plan with an `Objective: #N (node X)` marker: that objective and
    /// node. Fails when the objective or node can't be found, or (unless
    /// `stacked`) node X still waits on others.
    pub(super) fn objective_of(
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
    pub(super) fn needs_claim(&self, mirror: &kata::KataIssue, kata_ref: &str) -> Result<bool> {
        needs_claim(mirror.owner.as_deref(), self.actor()?, kata_ref)
    }

    /// Work out everything dispatch will do for plan `number`, changing
    /// nothing. `stacked` lets a node with unfinished dependencies through.
    pub(super) fn plan_dispatch(&self, number: u64, stacked: bool) -> Result<DispatchPlan> {
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
    pub(super) fn find_mirror(
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
    pub(super) fn find_existing_pr(
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

    pub(super) fn remote_has_branch(&self, b: &str) -> Result<bool> {
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
    pub(super) fn inspect_branch(
        &self,
        number: u64,
        branch: &str,
        pr: Option<OpenPr>,
    ) -> Result<BranchInfo> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::test_support::*;
    use rstest::rstest;

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
}

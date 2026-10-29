//! Carrying out a confirmed plan: claim the kata mirror, push the branch,
//! open the draft PR, then hand over to the workspace.

use anyhow::{Context, Result};

use crate::integrations::{github, kata};
use crate::process::{git, git_remote, run_checked};

use super::{pr_body, queue_node_progress, BranchState, DispatchPlan, Launch, NodeProgress, Run};

/// How carrying out one plan went. `pr_url` is set once the branch and PR
/// exist, even when a later step failed; `launched` is `Ok(true)` when a new
/// Claude session started.
#[derive(Debug)]
pub(super) struct Outcome {
    pub(super) pr_url: Option<String>,
    pub(super) launched: Result<bool>,
}

impl Run<'_> {
    /// Carry out one plan; once its branch and PR exist, its objective node is
    /// queued to be marked in progress, even if a later step failed.
    pub(super) fn dispatch_one(&self, plan: &DispatchPlan, progress: &mut NodeProgress) -> Outcome {
        let outcome = self.carry_out(plan);
        if outcome.pr_url.is_some() {
            queue_node_progress(plan, progress);
        }
        outcome
    }

    /// Carry out a confirmed plan and print its report. Every step looks up
    /// existing state first, so a re-run after a partial failure resumes.
    pub(super) fn carry_out(&self, t: &DispatchPlan) -> Outcome {
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
    pub(super) fn ensure_mirror_claimed(&self, t: &DispatchPlan) -> Result<String> {
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
    pub(super) fn place_branch_and_pr(&self, t: &DispatchPlan, kata_ref: &str) -> Result<String> {
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

    /// Create the branch without checking anything out, so the user's checkout
    /// and HEAD stay untouched.
    pub(super) fn create_branch_if_missing(&self, t: &DispatchPlan) -> Result<()> {
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
}

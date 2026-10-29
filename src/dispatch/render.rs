//! What dispatch prints and writes for a plan: the plan summary, the draft
//! PR body and the prompt Claude starts with.

use super::{redact_secrets, DispatchPlan, READY_NO_RELAUNCH};

pub(super) fn format_plan(t: &DispatchPlan, no_launch: bool, claude: &str) -> String {
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

pub(super) fn format_ready_base(t: &DispatchPlan) -> String {
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

pub(super) fn pr_body(number: u64, kata_ref: &str, stacked_on: Option<&str>) -> String {
    let stacked = stacked_on
        .map(|base| format!(" Stacked on `{base}`."))
        .unwrap_or_default();
    format!(
        "Work in progress for engram plan #{number} (kata `{kata_ref}`).{stacked}\n\nCloses #{number}"
    )
}

pub(super) fn build_prompt(t: &DispatchPlan, kata_ref: &str, pr_url: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::test_support::*;
    use rstest::rstest;

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
}

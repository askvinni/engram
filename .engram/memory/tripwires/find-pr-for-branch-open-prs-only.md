---
title: "find_pr_for_branch returns None for merged or closed PRs — gh pr list defaults to open only"
read_when:
  - "calling find_pr_for_branch to look up whether a branch has an associated PR"
  - "implementing or extending cmd_status or any command that shows PR state for a branch"
  - "debugging why engram status shows 'PR: none' on a branch that had a merged PR"
tripwires:
  - action: "Calling find_pr_for_branch expecting it to surface merged or closed PRs"
    warning: "gh pr list defaults to --state open; find_pr_for_branch passes no --state flag, so it returns None for any branch whose PR has already been merged or closed — pass --state all to include those"
last_updated: "2026-05-18"
source_issues: [12]
---

find_pr_for_branch in src/github.rs calls gh pr list with no --state flag, which the gh CLI silently defaults to open. A branch in the 'merged-but-not-yet-landed' state — where the PR has merged but engram land has not run — returns None rather than the merged PR, causing engram status to print 'PR: none' even though a PR clearly exists. Any future function built on top of find_pr_for_branch inherits this gap. To surface merged or closed PRs, add --state all (or --state merged) to the argument list in src/github.rs:find_pr_for_branch.

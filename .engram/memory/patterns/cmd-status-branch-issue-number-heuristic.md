---
title: "cmd_land uses merged PR headRefName for branch cleanup; cmd_status scans PR body for closes #N before integer fallback"
read_when:
  - "implementing or changing the branch-deletion step in plan::land()"
  - "implementing or changing the branch-to-issue linking logic in plan::status()"
  - "debugging why engram land or status behaves unexpectedly on a non-standard branch name"
tripwires:
  - action: "Expecting cmd_land to delete a working branch by matching its name against fix/issue-N, feat/issue-N, or issue-N patterns"
    warning: "Those three patterns were removed in issue #73 — cmd_land now calls find_linked_pr() after learn::run() to retrieve headRefName from the merged PR and deletes that exact branch; a missing headRefName emits a non-fatal warning and skips deletion rather than erroring"
  - action: "Relying on cmd_status always deriving the linked issue from the first integer in the branch name"
    warning: "Integer scan is now only the fallback — when find_pr_for_branch returns an open PR, cmd_status first scans that PR body for closes/fixes/resolves #N; the integer scan only fires when no PR exists for the branch"
last_updated: "2026-05-21"
source_issues: [12, 73]
---

After issue #73, the branch-name heuristics in cmd_land and cmd_status were replaced with authoritative GitHub data. cmd_land calls find_linked_pr() a second time after learn::run() — safe because the CLOSED_EVENT is already on GitHub and find_linked_pr() is idempotent — to retrieve the merged PR's headRefName, then runs git branch -d <headRefName>; a missing headRefName (fork PR or no linked PR found) emits a non-fatal stderr warning and skips deletion rather than aborting. cmd_status now checks whether find_pr_for_branch returns an open PR and, if so, scans that PR body for closes/fixes/resolves #N (case-insensitive) as the primary path; the first-integer branch-name split survives only as the fallback for branches with no open PR. The old three-candidate array (fix/issue-N, feat/issue-N, issue-N) in plan::land and the unconditional integer scan in plan::status are both gone. See src/plan.rs:land and src/plan.rs:status.

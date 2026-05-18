---
title: "engram land only auto-deletes branches named fix/issue-N, feat/issue-N, or issue-N"
read_when:
  - "creating a local branch to work on an engram-tracked issue"
  - "implementing or changing the branch-deletion step in cmd_land"
tripwires:
  - action: "Creating a branch for an engram issue with a name outside the fix/issue-N, feat/issue-N, issue-N patterns"
    warning: "cmd_land scans only those three patterns and stops at the first match — branches named anything else are silently left behind after landing"
last_updated: "2026-05-18"
source_issues: [11]
---

cmd_land in src/main.rs cleans up local branches by iterating exactly three candidate names — fix/issue-{N}, feat/issue-{N}, issue-{N} — and deletes the first one that exists via git branch -d (safe delete, not -D). Branches named outside this set, or branches with unmerged commits, are silently left behind even when cmd_land otherwise succeeds. The naming convention must be established at branch-creation time; there is no fallback and no warning when no candidate matches. This constraint is invisible at issue-open time and only discoverable by reading cmd_land. See src/main.rs:cmd_land for the candidate list.

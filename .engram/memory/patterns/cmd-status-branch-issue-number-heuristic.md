---
title: "cmd_status extracts issue numbers from branches via first-integer scan, unlike cmd_land's fixed patterns"
read_when:
  - "creating a branch for an engram issue that should appear in engram status output"
  - "implementing or changing the branch-to-issue linking logic in cmd_status"
  - "debugging why engram status shows the wrong issue for a branch"
tripwires:
  - action: "Naming a branch with multiple numeric segments (e.g., fix/42-backport-v2) expecting cmd_status to link to the correct issue"
    warning: "cmd_status takes the first parseable integer found anywhere in the branch name — fix/42-backport-v2 links to issue 42, but upgrade-node18 links to issue 18; cmd_land uses a different, stricter pattern list"
last_updated: "2026-05-18"
source_issues: [12]
---

cmd_status and cmd_land use entirely different branch-name parsing strategies for the same domain (branch → issue number). cmd_land tries three exact prefix patterns (fix/issue-N, feat/issue-N, issue-N) in order. cmd_status instead splits the branch name on any non-digit character and takes the first segment parseable as a u64 — so any branch containing a number anywhere will link to the issue with that number, and a branch like hotfix-v2 silently links to issue 2. The two commands will disagree on ambiguous names, and neither emits a warning when their heuristic produces a surprising result. See src/main.rs:cmd_status for the split logic and src/main.rs:cmd_land for the pattern list.

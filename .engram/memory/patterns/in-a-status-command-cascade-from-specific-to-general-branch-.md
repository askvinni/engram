---
title: "In status commands, cascade from specific to general context"
read_when:
  - "implementing a status or info command"
  - "deciding what to show when a branch has no linked issue or PR"
  - "improving user feedback when context is ambiguous or missing"
tripwires: []
last_updated: "2026-05-18"
source_issues: [12]
---

A status command should show the most specific available context and fall back gracefully when information is missing. The cascade for engram is: branch name → linked PR (by branch head) → linked issue (parsed from branch name) → all open engram-plan issues. This way users on an untracked branch still see something useful (the open plan list), and users on a tracked branch see exactly their PR and issue. Avoid bailing early with "no linked issue found" — fall through to the next level instead. See src/main.rs:cmd_status for the implementation.

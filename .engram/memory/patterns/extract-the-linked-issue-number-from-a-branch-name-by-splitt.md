---
title: "Extract issue number from a branch name by splitting on non-digit characters"
read_when:
  - "parsing a branch name to find the linked issue number"
  - "implementing status, land, or any command that derives context from branch name"
  - "adding branch-name parsing to engram"
tripwires: []
last_updated: "2026-05-18"
source_issues: [12]
---

Branch names follow no strict convention — common patterns include `feat/issue-12`, `fix/12-foo`, `issue-12-short-title`. Splitting on non-digit characters (`split(|c: char| !c.is_ascii_digit())`) and taking the first parseable integer handles all of these without a regex. This is more robust than string prefix matching or regex patterns that would need updating each time a new naming convention appears. See src/main.rs:cmd_status for the implementation.

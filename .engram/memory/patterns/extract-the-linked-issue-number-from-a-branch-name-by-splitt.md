---
title: "Extract the linked issue number from a branch name by split…"
read_when:
  - "(migrated — add read_when conditions)"
tripwires: []
last_updated: "2026-05-18"
source_issues: [12]
---

Extract the linked issue number from a branch name by splitting on non-digit characters (`split(|c: char| !c.is_ascii_digit())`) and finding the first parseable integer — works for common patterns like `feat/issue-12`, `fix/12-foo`, etc.

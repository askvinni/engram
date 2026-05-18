---
title: "Fetch structured GitHub issue data with `gh issue list --js…"
read_when:
  - "(migrated — add read_when conditions)"
tripwires: []
last_updated: "2026-05-18"
source_issues: [10]
---

Fetch structured GitHub issue data with `gh issue list --json number,title,createdAt` and parse via `serde_json::from_str` into typed structs — keeps all GitHub I/O consistent with the rest of the codebase.

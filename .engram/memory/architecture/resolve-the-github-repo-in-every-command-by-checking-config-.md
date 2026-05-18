---
title: "Resolve the GitHub repo in every command by checking config…"
read_when:
  - "(migrated — add read_when conditions)"
tripwires: []
last_updated: "2026-05-18"
source_issues: [10]
---

Resolve the GitHub repo in every command by checking config first then falling back to `infer_repo()` — never assume the repo is available without this two-step lookup.

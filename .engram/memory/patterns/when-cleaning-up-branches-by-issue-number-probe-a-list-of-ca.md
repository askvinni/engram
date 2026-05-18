---
title: "When cleaning up branches by issue number, probe a list of …"
read_when:
  - "(migrated — add read_when conditions)"
tripwires: []
last_updated: "2026-05-18"
source_issues: [11]
---

When cleaning up branches by issue number, probe a list of candidate name patterns (`fix/issue-{N}`, `feat/issue-{N}`, `issue-{N}`) and break on first match — accommodates real-world naming variance without requiring a strict convention.

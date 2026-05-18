---
title: "Check resource state before mutating it (e.g., check issue …"
read_when:
  - "(migrated — add read_when conditions)"
tripwires: []
last_updated: "2026-05-18"
source_issues: [11]
---

Check resource state before mutating it (e.g., check issue state before closing) since external systems like GitHub may auto-close issues when a linked PR merges — skip redundant operations and report the observed state instead.

---
title: "Define health checks as a slice of (&str, Box<dyn Fn() -> b…"
read_when:
  - "(migrated — add read_when conditions)"
tripwires: []
last_updated: "2026-05-18"
source_issues: [9]
---

Define health checks as a slice of (&str, Box<dyn Fn() -> bool>) tuples so new checks can be added declaratively without touching the display/exit logic.

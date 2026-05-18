---
title: "Guard injected prompt sections with an emptiness check befo…"
read_when:
  - "(migrated — add read_when conditions)"
tripwires: []
last_updated: "2026-05-18"
source_issues: [13]
---

Guard injected prompt sections with an emptiness check before interpolating them — prevents blank `## Project-Specific Rules` headers from appearing in prompts when no hooks are defined. _(from #13)_

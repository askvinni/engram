---
title: "Inlining memory content into CLAUDE.md is unsafe: if any le…"
read_when:
  - "(migrated — add read_when conditions)"
tripwires: []
last_updated: "2026-05-18"
source_issues: [8]
---

Inlining memory content into CLAUDE.md is unsafe: if any learning contains the engram end-marker string, it corrupts the section boundary — this already happened once.

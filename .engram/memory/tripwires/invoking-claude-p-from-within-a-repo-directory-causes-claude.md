---
title: "Invoking `claude -p` from within a repo directory causes CL…"
read_when:
  - "(migrated — add read_when conditions)"
tripwires: []
last_updated: "2026-05-18"
source_issues: [26]
---

Invoking `claude -p` from within a repo directory causes CLAUDE.md to be loaded, turning a simple synthesis call into an action-taking agent — always set `current_dir(temp_dir())` for programmatic Claude calls that should only return structured output.

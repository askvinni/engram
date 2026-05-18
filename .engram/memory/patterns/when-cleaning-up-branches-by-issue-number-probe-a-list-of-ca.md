---
title: "When cleaning up branches by issue number, probe candidate name patterns"
read_when:
  - "deleting a branch associated with a closed issue"
  - "implementing or changing the land command's branch cleanup step"
  - "adding a new naming convention for engram feature branches"
tripwires: []
last_updated: "2026-05-18"
source_issues: [11]
---

There is no enforced branch naming convention in engram — developers use `feat/issue-N`, `fix/issue-N`, `issue-N`, or other variations. When cleaning up after land, probe a list of candidate patterns (`fix/issue-{N}`, `feat/issue-{N}`, `issue-{N}`) and break on first match rather than requiring a specific format or failing silently. Use `git branch --list <name>` to check existence before attempting deletion; an empty output means the branch doesn't exist locally. See src/main.rs:cmd_land for the candidate probing loop.

---
title: "Check resource state before mutating it — external systems may have already acted"
read_when:
  - "calling a mutation on a GitHub resource (close issue, delete branch, merge PR)"
  - "implementing a workflow step that may already have been done by a prior step"
  - "adding a new operation to the land command"
tripwires: []
last_updated: "2026-05-18"
source_issues: [11]
---

GitHub may auto-close an issue when a linked PR merges, auto-delete a branch after merge, or otherwise perform state changes before engram gets a chance to. Always fetch current state before mutating — if the issue is already closed, skip the close call and print "already closed" rather than failing or silently double-acting. This applies to any operation that might have already happened via GitHub's automation. See src/main.rs:cmd_land for the issue-close guard pattern.

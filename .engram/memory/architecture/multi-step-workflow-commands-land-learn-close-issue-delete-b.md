---
title: "Multi-step workflow commands should print a status line after each step"
read_when:
  - "implementing a new engram command that calls multiple sub-operations"
  - "adding a step to an existing workflow command like land"
  - "deciding how to report progress in a command that may fail partway through"
tripwires: []
last_updated: "2026-05-18"
source_issues: [11]
---

Commands that orchestrate multiple operations (e.g. land = learn → close issue → delete branch) should print a status line after each step completes. When a later step fails, the user can see exactly which steps already ran and which didn't — preventing confusion about whether to retry the whole command or only the remainder. Printing "Closed issue #N." or "Deleted local branch feat/issue-N." after each action also makes the command's behavior self-documenting during first use. See src/main.rs:cmd_land for the pattern.

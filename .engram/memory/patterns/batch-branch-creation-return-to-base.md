---
title: "Capture and return to the base branch between iterations in batch branch workflows"
read_when:
  - "implementing a batch command that creates one branch per issue"
  - "adding a new --all variant to any engram command that calls cmd_learn internally"
  - "debugging unexpectedly stacked engram/learn-N branches"
tripwires: []
last_updated: "2026-05-18"
source_issues: [42]
---

cmd_learn_all() captures the current branch name before the loop and checks out that base branch after each iteration. Without this reset, each engram/learn-N branch is created from the previous engram/learn-(N-1) branch rather than from the shared base — producing a stack of dependent branches where merging them in the wrong order rewrites history. Any future batch command that calls cmd_learn (or any command that creates a branch) inside a loop must apply the same capture-and-return pattern. See src/main.rs:cmd_learn_all.

---
title: "Use learn::write_memory (not learn::run) inside batch loops — run creates one branch+PR per call"
read_when:
  - "implementing a batch command that synthesizes learnings from multiple plan issues"
  - "adding an --all or objective-scoped variant of engram land"
  - "calling any learn function inside a loop over issues or nodes"
tripwires:
  - action: "Calling learn::run() inside a loop over multiple plan issues"
    warning: "learn::run() executes the full pipeline — branch, commit, push, PR, close, label — once per call; use learn::write_memory() in the loop instead and do one branch+commit+push+PR after the loop completes"
last_updated: "2026-05-19"
source_issues: [65]
---

learn::run() is the complete single-issue pipeline: it writes memory files, creates a branch, commits, pushes, opens a PR, closes the issue, and adds the engram-learned label — producing one branch and one PR per invocation. learn::write_memory() only writes the memory files to disk and returns a bool indicating whether anything changed, leaving all git workflow to the caller. Batch commands like cmd_learn_all and objective::land must call write_memory() inside the loop, then execute a single branch+commit+push+PR sequence once after the loop using only the issues where write_memory returned true. Using learn::run() in a batch loop creates N separate branches and PRs — one per issue — which is both noisy and defeats the purpose of batching. The canonical reference is src/objective.rs:land for the batch pattern and src/main.rs:cmd_learn_all for the original implementation.

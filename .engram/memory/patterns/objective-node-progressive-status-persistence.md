---
title: "Persist node status to GitHub after each successful write in a batch loop, not at the end"
read_when:
  - "implementing a loop that processes multiple objective nodes and updates their status"
  - "adding batch processing to any command that iterates engram-tracked nodes or issues"
  - "designing a multi-step command where partial progress must survive a mid-run failure"
tripwires: []
last_updated: "2026-05-19"
source_issues: [65]
---

In objective::land, each node is marked Done and the objective issue body is updated on GitHub immediately after its write_memory call succeeds — not accumulated and written once at the end of the loop. This makes the command idempotent under partial failure: if the process crashes or a later node's write_memory fails, already-processed nodes are persisted as Done in the objective body, and re-running the command skips them via the status != Done filter. Batching all status updates into a single gh issue edit at the end would mean any mid-run crash loses all progress tracking and forces a full re-run — including re-synthesizing and overwriting memory that was already correctly written. The cost of one extra GitHub API call per node is negligible compared to the safety of not losing progress. Any future command that iterates objective nodes and mutates their status must apply the same per-iteration persist pattern. See src/objective.rs:land.

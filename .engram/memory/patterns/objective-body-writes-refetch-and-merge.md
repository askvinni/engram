---
type: pattern
title: Update an objective's node list by re-fetching its body just before the write and merging only your fields
read_when:
- writing an objective issue body from any engram command (dispatch, objective plan, land, stacking)
- updating a node's status or plan_issue in the hidden engram:nodes comment
- debugging lost edits or reverted node statuses on an objective issue
tripwires:
- action: Writing build_objective_body from an objective body fetched at the start of a long run
  warning: Plan creation, other dispatches and humans edit the same body during the run, so a stale copy silently reverts their changes. Re-fetch right before the write, parse the nodes, merge only the fields you own and write once per objective. See dispatch::record_node_progress.
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '144'
  resource: https://github.com/askvinni/engram/issues/144
---

An objective's node graph lives in a hidden comment inside a GitHub issue body that several commands and people change concurrently, and GitHub has no compare-and-swap for issue bodies. dispatch keeps the race window small. It collects per-objective (node ID, plan) updates during the run (the NodeProgress map), then record_node_progress re-fetches the body, applies merge_node_progress and calls update_issue_body once, only when something changed. merge_node_progress is a pure function. It fills plan_issue only when it is missing, moves only Pending to InProgress (Done stays Done), skips unknown node IDs and reports each changed ID once, which keeps it unit-testable. If the re-fetched body no longer has a node list, warn and skip; don't fail. Record a node's plan_issue even when the rest of its dispatch fails, so a re-run doesn't create a second plan. Any new command that changes node state should follow the same pattern.

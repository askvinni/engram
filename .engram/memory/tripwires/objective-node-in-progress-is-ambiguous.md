---
type: tripwire
title: An objective node's in_progress status means either planned or dispatched, so don't use it to decide what to launch
read_when:
- selecting which objective nodes to dispatch, stack or relaunch
- deciding whether an objective node has already been started
- changing how objective::create_plan_for_node or dispatch sets node status
tripwires:
- action: Filtering dispatch or relaunch candidates to Pending nodes only (e.g. via unblocked_nodes)
  warning: create_plan_for_node sets a node to in_progress as soon as its plan issue exists. Pending-only selection would skip every planned but undispatched node. Include in_progress nodes and rely on dispatch's live reuse checks (open PR, worktree, running Claude, ready PR) to avoid launching twice.
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '144'
  resource: https://github.com/askvinni/engram/issues/144
---

The node model has no separate 'dispatched' state. A node becomes in_progress both when `engram objective plan` creates its plan issue and when dispatch opens its PR, so the status can't tell you whether work is running. ready_nodes in src/dispatch.rs therefore takes every node that is not Done and has an empty blocked_by, and leaves idempotency to the live lookups described in dispatch-idempotency-from-lookups-not-state. Dispatch's output says whether a session was launched or reused, so a re-run doesn't look like new work. Any future feature that reads node status (stacked PRs in 5.4, status views, relaunch) has to treat in_progress as 'possibly not started'. If it needs the real state, it must check the PR, worktree or session.

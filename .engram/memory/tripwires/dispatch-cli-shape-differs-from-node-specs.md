---
type: tripwire
title: 'engram dispatch takes `issues: Vec<u64>` and does not create plan issues or show a picker, despite the 5.3 spec'
read_when:
- 'implementing stacked dispatch (node 5.4) or other objective dispatch work from the objective #141 issue text'
- changing the Dispatch command arguments in src/cli.rs or the multi-issue loop in src/dispatch.rs
- debugging why engram dispatch on an objective skips a ready node instead of creating its plan
tripwires:
- action: Looking for dispatch_candidates, blocked_report, a multi-select picker or plan creation inside engram dispatch
  warning: None of these shipped. The functions are ready_nodes and blocked_nodes in src/dispatch.rs. A ready node with no plan issue is skipped (not counted as a failure) with a hint to run `engram objective plan --node`. Only the --yes confirmation exists. Don't build on the spec's API.
- action: Adding a mutation to the multi-issue dispatch path before plan_all has finished for every issue
  warning: Dispatch plans every issue first (plan_all, one count_line), then runs each one through dispatch_one (run_all). Each plan fails on its own and the rest continue, and the exit status is non-zero if any plan or run failed. Early mutations break --dry-run and the single confirmation.
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '143'
  resource: https://github.com/askvinni/engram/issues/143
- id: '144'
  resource: https://github.com/askvinni/engram/issues/144
---

Issue #144 (node 5.3) asked dispatch to create missing plan issues through create_plan_for_node, to pick candidates with dispatch_candidates and blocked_report, and to offer a TTY multi-select picker. The merged PR left plan creation in `engram objective plan`, because it needs Claude, is slow and fails per node. It renamed the selectors ready_nodes and blocked_nodes and dropped the picker. Commands::Dispatch is now `{ issues: Vec<u64> (required), opts: DispatchOptions }`. collect_work removes duplicate arguments and plans, so an objective passed together with one of its own plans dispatches that plan once. require_plan_or_objective decides what an issue is from its node-list comment, not from the objective label alone. An objective label with no node list is an error. An agent working from the issue text will reach for functions and behaviour that don't exist.

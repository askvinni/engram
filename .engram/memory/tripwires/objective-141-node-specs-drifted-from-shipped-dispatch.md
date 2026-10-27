---
type: tripwire
title: 'Every dispatch node in objective #141 shipped narrower than its issue spec, so write docs and follow-up work from src/dispatch.rs, not the issue text'
read_when:
- writing or updating engram dispatch documentation in README.md or CLAUDE.md
- 'starting follow-up dispatch work from the objective #141 issue text or any of its node issues (#142-#146)'
- copying a feature list or template field list out of an engram objective/plan issue body into docs
tripwires:
- action: Documenting dispatch features, flags or template fields by copying them from the objective or node issue bodies
  warning: Nodes 5.1-5.4 each dropped specced features ({kata_ref}, {{ }} escapes, the interactive picker, plan creation during dispatch, the merged-dependency pre-pass). Check each claim against `engram dispatch --help` and src/dispatch.rs, or the docs will promise behaviour that does not exist.
- action: Treating an acceptance criterion in a later node as authoritative when an earlier node already shipped
  warning: Later node issues were written before earlier nodes merged, so their criteria assume the earlier spec. The 5.5 docs PR had to list {kata_ref}, plan creation and the closed/merged pre-pass as 'not included'. Reconcile against merged code first and say what was dropped in the PR body.
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '146'
  resource: https://github.com/askvinni/engram/issues/146
---

Objective #141 was planned as a whole up front, but each node's PR cut or renamed parts of its spec: fill_template instead of render, ready_nodes instead of dispatch_candidates, order_for_stack instead of stack_order, and no picker or plan creation. Later node issues, including the docs node 5.5, still asked for the original features, so an agent working from the issue text writes docs and code against an API that does not exist. The merged code is the source of truth. The README Dispatch section and the CLAUDE.md Dispatch developer section were written from the code and describe the real flow (collect_work/collect_stack_work → plan_dispatch → confirm → carry_out), so extend those sections rather than the issue text. When a spec item is intentionally absent, list it as not included in the PR description so the gap is a decision on record and not a silent omission. The same applies to any future multi-node objective: before starting node N, re-read what nodes 1..N-1 actually merged.

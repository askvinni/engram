---
type: tripwire
title: Create plan kata mirrors only through kata.rs helpers so plan new, objective plan and dispatch can't duplicate them
read_when:
- creating a kata mirror for a GitHub plan issue from any engram command
- 'changing the plan idempotency key, mirror body or Kata: ref line format'
- rendering dispatch templates for a plan that has no kata mirror yet
tripwires:
- action: Building a kata create call for a plan issue by hand outside kata.rs
  warning: Use kata::plan_idempotency_key (engram-plan-<N>), mirror_body and with_ref_line. A different key or body lets plan new and dispatch each create a mirror for the same plan.
- action: 'Appending the Kata: <ref> line to the GitHub issue before claiming the new mirror'
  warning: Claim first and append the line only after the claim succeeds. Otherwise a failed claim leaves the issue pointing at a mirror that dispatch then refuses on every rerun.
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '143'
  resource: https://github.com/askvinni/engram/issues/143
---

Three commands can create the kata mirror for a plan issue: plan new, objective plan and dispatch (when the mirror is missing). The mirror body, idempotency key, Kata: ref line and lookup (find_mirror) all live in src/kata.rs so the three paths cannot drift apart. Because kata's idempotency key collapses retries, the paths only converge if the key is identical. When no mirror exists yet, dispatch templates cannot use kata metadata. dispatch::missing_meta_hint therefore tells the user to add a `Jira:` or `Type:` line to the plan body (the only keys TrackerMeta::resolve reads from the body). For any other {meta[key]}, the plan must be mirrored first and the key set with `kata meta set`. A blank rendered PR title is rejected during planning (fill_pr_title), because gh pr create would otherwise reject it only after the branch was pushed.

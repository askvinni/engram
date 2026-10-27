---
type: pattern
title: Make dispatch rerunnable by looking up each artifact before creating it, never from stored state
read_when:
- adding a new mutating step to engram dispatch (5.3 objectives, 5.4 stacked PRs)
- finding the existing PR for a plan issue from code
- debugging dispatch creating a second branch, PR, worktree or kata mirror on a rerun
tripwires:
- action: Using find_pr_for_branch with the templated branch name to decide whether a plan already has a PR
  warning: 'The PR may sit on a different branch (dispatched from another machine, or after a template change). Use dispatch''s find_existing_pr: GitHub closing links first, then open PRs whose body says Closes #N, skipping forks and preferring the templated branch. When a PR is found, adopt its branch, title and base.'
- action: Relaunching Claude or recreating a branch whenever an existing PR is found
  warning: Refuse a PR whose head was merged, and never relaunch on a ready-for-review (non-draft) PR. Keep an existing PR's base even when it differs from the default branch (warn instead of retargeting).
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '143'
  resource: https://github.com/askvinni/engram/issues/143
---

Dispatch can fail partway: the PR is created and then kwt fails. A rerun must resume without duplicating anything. Nothing records progress, so each step looks up live state first. plan_dispatch does every lookup without side effects, so --dry-run is exactly the planning half and carry_out reuses its result. The lookups are: find_mirror for the kata issue, find_existing_branch for the local or remote branch, find_existing_pr, and select_worktree, which matches the exact branch in `kwt list --json`. Creation is ordered so that a conflict stops dispatch before anything is published. The kata claim comes before any git or GitHub mutation, and another actor's claim, or a mirror that is not open, is refused rather than taken over. A branch that exists only on the remote gets a local tracking branch instead of a divergent wip commit. Push is skipped when origin already has the ref. Note that `git ls-remote --exit-code` exits 2 for 'ref absent', which must be told apart from a real failure (LS_REMOTE_NO_MATCH). Every new dispatch step should follow the same pattern: a pure lookup in planning, and creation only in carry_out.

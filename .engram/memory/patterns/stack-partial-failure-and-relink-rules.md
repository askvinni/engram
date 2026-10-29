---
type: pattern
title: In a stack, a node counts as a base once its branch and PR exist, and only chains containing a new PR get relinked
read_when:
- changing run_stack or how stacked dispatch handles a failure in the middle of a stack
- deciding which stacked PR chains to pass to gh stack link on a rerun
- adding a new step to stacked dispatch that could fail after the PR is opened
tripwires:
- action: Skipping a node's dependents whenever any step of that node's dispatch fails
  warning: Skip the dependents only when the failure happened before the node's PR existed. A node that has its branch and PR is a valid base even if kwt or the Claude launch failed; skipping its dependents would stall the stack for no reason.
- action: Passing every chain, or a branched set of PRs, to gh stack link
  warning: gh stack link expects one linear list. Split the stack with straight_chains and link only chains of two or more PRs that contain at least one newly dispatched node (chains_to_link). Otherwise reruns relink untouched stacks, or a fan-out gets mislinked.
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '145'
  resource: https://github.com/askvinni/engram/issues/145
---

Stacked dispatch keeps a map from node to PlacedNode (branch and PR URL). Dependents look up their base branch there through base_branch, which returns None until the dependency is placed. During planning the branch name stands in for a PR that hasn't been opened, so --dry-run can print the bases and the link commands without mutating anything. What makes a node usable as a base is that it has a branch and a PR. Whether its session launched doesn't matter. This keeps a partly failed run resumable: the rerun reuses the placed nodes through the usual live lookups and continues with the independent chains. A chain starts at a node on the default branch, or under a parent with several stacked children, and follows single children upward. A chain made only of reused PRs was already linked by an earlier run and is left alone.

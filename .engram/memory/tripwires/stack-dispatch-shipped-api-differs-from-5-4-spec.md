---
type: tripwire
title: Stacked dispatch shipped as order_for_stack with a Base enum, not the spec's stack_order/StackTarget, and has no merged-dependency pre-pass
read_when:
- 'documenting `engram dispatch --stack` or removing kdispatch (node 5.5) from the objective #141 issue text'
- changing how stacked dispatch orders nodes, chooses bases or handles closed or merged dependencies
- debugging why --stack skips a node whose dependency's PR already merged
tripwires:
- action: Looking for stack_order, StackTarget or a merged-PR detection pass in src/dispatch.rs
  warning: 'The shipped functions are order_for_stack (returning StackOrder with Base::Main or Base::OnNode), straight_chains, chains_to_link and base_branch. The ancestry check is objective::unfinished_ancestors. Merged-but-not-done dependencies are not detected: a closed plan just fails its plan check with an `engram plan land <n>` hint, and its dependents are skipped.'
generated:
  by: engram/0.1.0
  at: 2026-10-27
sources:
- id: '145'
  resource: https://github.com/askvinni/engram/issues/145
---

Issue #145 described a stack_order function, a pass that detects a dependency whose PR merged and rebases its dependents onto the default branch, and plan creation during dispatch. The merged PR left all of that out. Dispatch still never creates plan issues: a node without one is skipped with the `engram objective plan` hint, and so are its dependents. A node with several unfinished dependencies is placed on the one whose unfinished ancestors include all the others. Otherwise it is skipped as a diamond, and anything that depends on a skipped node is skipped too. A node whose PR is already ready for review is only used as a base: it is not claimed, launched or relaunched. Agents working from the issue text will look for functions and recovery behaviour that don't exist. See src/dispatch.rs:order_for_stack and plan_stack.

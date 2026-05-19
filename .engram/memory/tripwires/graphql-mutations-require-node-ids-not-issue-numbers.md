---
title: "GitHub GraphQL mutations accept opaque node IDs, not integer issue numbers"
read_when:
  - "implementing a GitHub GraphQL mutation in github.rs that references issues or PRs"
  - "adding a new function that calls addSubIssue, addLabelsToLabelable, or any GitHub mutation with an ID! parameter"
tripwires:
  - action: "Passing an integer issue number directly to a GitHub GraphQL mutation that declares an ID! parameter"
    warning: "GitHub GraphQL mutations take opaque global node IDs (e.g. MDU_kwDO...), not integers; resolve numbers to node IDs first with a separate query, then call the mutation — see src/github.rs:add_sub_issue for the two-round-trip pattern"
last_updated: "2026-05-19"
source_issues: [59]
---

GitHub's REST API uses integer issue numbers everywhere, but its GraphQL mutation inputs typed as `ID!` require opaque global node IDs — the `MDU_kwDO...` strings returned by querying `issue(number: N) { id }`. add_sub_issue in src/github.rs demonstrates the mandatory two-step pattern: one query resolves both the parent and child issue numbers to their node IDs, then a second call executes the addSubIssue mutation using those IDs as `parentId`/`childId`. Passing the raw integer will produce a GraphQL type error at runtime with no compile-time warning. Any future mutation that operates on issues, PRs, labels, or projects will require the same resolve-then-mutate structure. See src/github.rs:add_sub_issue.

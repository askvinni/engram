---
title: "Apply a completion label to each processed issue to make batch --all reruns idempotent"
read_when:
  - "adding a new --all batch subcommand to engram"
  - "implementing a workflow that processes a set of GitHub issues and must be safely re-runnable"
  - "deciding where to apply the engram-learned label in the learn workflow"
tripwires: []
last_updated: "2026-05-18"
source_issues: [42]
---

After engram learn creates the learning PR, it immediately applies the engram-learned label to the original issue. This label is the sole mechanism that makes --all idempotent: list_unlearned_plans() excludes labeled issues, so re-running --all after a partial failure or interruption skips already-processed issues and resumes from the first unlabeled one. The label must be applied after the PR is created — not before — so a crash mid-flight leaves the issue unlabeled and eligible for retry. Any future engram batch subcommand should follow the same label-as-progress-marker pattern. See src/learn.rs for the label call placement.

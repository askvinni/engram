---
title: "GitHub API has no native 'does not have label' filter — client-side post-filtering required"
read_when:
  - "querying GitHub issues or PRs that lack a specific label"
  - "implementing or changing list_unlearned_plans()"
  - "adding a new GitHub query that needs to exclude issues by label"
tripwires:
  - action: "Passing a negative label predicate to the GitHub REST or GraphQL API"
    warning: "Neither API supports 'issues WITHOUT label X'; fetch the broad set and filter client-side to exclude issues whose labels vec contains the target label"
last_updated: "2026-05-18"
source_issues: [42]
---

The GitHub REST API and GraphQL do not expose a negative label predicate — there is no equivalent of 'issues that do NOT have label engram-learned'. list_unlearned_plans() works around this by fetching all closed issues with the engram-plan label, then filtering in Rust to exclude any whose labels array contains engram-learned. Any future function that needs issues or PRs lacking a given label must do the same: over-fetch with a positive constraint and filter client-side. See src/github.rs:list_unlearned_plans.

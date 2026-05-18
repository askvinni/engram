---
title: "Use GraphQL CLOSED_EVENT to find the PR that closed an issue — text search has post-merge lag"
read_when:
  - "implementing or changing find_linked_pr() in github.rs"
  - "looking up which PR closed a given GitHub issue from code"
tripwires:
  - action: "Using gh pr list --search 'closes #N is:merged' to find the PR that closed an issue"
    warning: "GitHub's search index lags after a merge — the query returns nothing for recently merged PRs; use GraphQL timelineItems(itemTypes: [CLOSED_EVENT]) instead, which reads authoritative event data with no lag"
last_updated: "2026-05-18"
source_issues: [5]
---

GitHub's REST/search API indexes PR bodies asynchronously, so a query for 'closes #N is:merged' immediately after merge reliably returns an empty list even when the PR exists and is merged. The correct approach is to query the issue's timeline directly via GraphQL: `timelineItems(itemTypes: [CLOSED_EVENT], last: 1)` returns a `ClosedEvent` node whose `closer` field is the exact PR that triggered the close — one round-trip, no indexing lag, and no false negatives. The filter `pr.state == "MERGED"` guards against issues closed by hand after a PR was abandoned. See src/github.rs:find_linked_pr for the GraphQL query and the local deserialization structs.

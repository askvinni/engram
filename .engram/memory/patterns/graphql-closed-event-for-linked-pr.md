---
type: pattern
title: Find an issue's PR via its GraphQL timeline (closer, then cross-references) — text search has post-merge lag
read_when:
- implementing or changing find_linked_pr() in github.rs
- looking up which PR closed a given GitHub issue from code
- debugging 'no merged PR found that closes
tripwires:
- action: 'Using gh pr list --search ''closes #N is:merged'' to find the PR that closed an issue'
  warning: 'GitHub''s search index lags after a merge — the query returns nothing for recently merged PRs; use GraphQL timelineItems(itemTypes: [CLOSED_EVENT, CROSS_REFERENCED_EVENT]) instead, which reads authoritative event data with no lag'
- action: Relying only on the ClosedEvent closer to find an issue's PR
  warning: Issues closed by hand have a null closer — stacked PRs merged into a non-default base never fire closing keywords — so also fall back to merged same-repo PRs that cross-reference the issue
generated:
  by: engram/0.1.0
  at: 2026-10-19
sources:
- id: '5'
  resource: https://github.com/askvinni/engram/issues/5
- id: '138'
  resource: https://github.com/askvinni/engram/pull/138
---

GitHub's REST/search API indexes PR bodies asynchronously, so a query for 'closes #N is:merged' immediately after merge reliably returns an empty list even when the PR exists and is merged. Query the issue's timeline via GraphQL instead: `timelineItems(itemTypes: [CLOSED_EVENT, CROSS_REFERENCED_EVENT], last: 100)` reads authoritative event data with no indexing lag.

The latest `ClosedEvent`'s `closer` is the exact PR that triggered the close; use it when it is a merged PR. It is null when the issue was closed by hand, which is routine for stacked PRs: closing keywords only fire on merges into the default branch, so a PR merged into another stack branch never closes its issue (this blocked learning from objective #82 until #138). In that case fall back to a merged PR in the same repo whose `CrossReferencedEvent` points at the issue, preferring `willCloseTarget` and then the most recent. Editing a merged PR's body to add `Closes #N` creates that cross-reference after the fact, which is how to repair an old PR. Closer and source are unions (PullRequest, Commit, Issue, …), so they deserialize as `__typename`-tagged enums. See src/github.rs:find_linked_pr and pick_linked_pr.
